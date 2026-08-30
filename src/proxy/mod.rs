//! The egress proxy (`add-egress-proxy`): a resident, permanently
//! unsandboxed process that terminates a sandbox's outbound HTTP/HTTPS
//! traffic and decides per hostname via `nono::HostFilter`. Spawned by
//! `up` only when `CompiledPolicy::wants_egress_proxy()` is true
//! (`network.default = "deny"` and `network.allow` non-empty) — a
//! sandbox with no domain filtering never pays for a proxy it has
//! nothing to filter.
//!
//! It cannot live inside the keeper: the keeper self-restricts to the
//! *same* `NetworkMode::ProxyOnly` its own sessions get
//! (`devcroft.rs::self_restrict`'s `apply_auto` call), so a proxy
//! process needing genuine outbound reach to arbitrary allowlisted hosts
//! would be restricted right alongside the code it exists to filter for.
//! It is spawned the same way the keeper is — a re-exec of this binary
//! under a hidden subcommand (`__egress-proxy`), detached via `setsid()`
//! so it outlives `up`'s own process — but it is never handed a
//! `CapabilityPlan` and never calls `apply_auto`.
//!
//! design.md's Open Questions (add-egress-proxy) record why this is the
//! right shape: `NetworkMode::ProxyOnly` (a plain Landlock `NetPort`/
//! Seatbelt rule, not the seccomp-notify path `apply_auto` reserves for
//! pre-V4 Landlock kernels) only ever permits a literal `connect()` to
//! this process's own port — nothing rewrites or redirects other
//! destinations — so this process is the only place a per-hostname
//! decision can take effect at all.

pub mod server;

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::lifecycle::StatePaths;

/// Binds the proxy's listening socket host-side on an OS-chosen loopback
/// port (never a fixed one — concurrent sandboxes must never collide on
/// it, the same reasoning `add-port-allocation`'s proposal already
/// applies to services), then spawns the resident proxy process with
/// that listener inherited across exec — the same listener-before-
/// restriction shape `up_process` uses for the control and SSH sockets,
/// though this process is never restricted at all.
///
/// Length of the generated session token, in random bytes before hex
/// encoding — 128 bits, the same order of magnitude as the ephemeral SSH
/// host key this project already regenerates per `up` (`ssh::keys`), and
/// far past what a local guessing attempt over a proxy connection could
/// exhaust before an operator would notice the sandbox behaving oddly.
const TOKEN_BYTES: usize = 16;

/// A random per-session token, hex-encoded. Generated fresh by every
/// `spawn` call — never derived from anything predictable, since its
/// only job is to be something a process that isn't this sandbox cannot
/// already know.
fn generate_token() -> String {
    let bytes: [u8; TOKEN_BYTES] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Returns the bound port (folded into the compiled policy via
/// `CompiledPolicy::with_proxy_port`, so the kernel gate matches the
/// process that will actually enforce it), the per-session token every
/// request must present (`add-egress-proxy`'s authentication
/// requirement — binding to loopback alone is reachability, not
/// authorisation), and the spawned pid (recorded in `paths.proxy_pidfile`
/// by the caller so `down`/`rm` can tear it down independently of the
/// keeper).
pub fn spawn(
    exe: &Path,
    paths: &StatePaths,
    allow: &[String],
) -> io::Result<(libc::pid_t, u16, String)> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    crate::lifecycle::clear_cloexec(listener.as_raw_fd())?;
    let token = generate_token();

    // The unix listener a network-isolated sandbox reaches the proxy
    // through — see `StatePaths::proxy_socket`. Bound here, host-side and
    // before exec, for the same listener-before-restriction reason the
    // control and SSH sockets are: the proxy process never gets to widen
    // anything for itself. Mode 0600 alongside the 0700 state dir, the
    // same belt-and-suspenders the control socket has, and it matters
    // more here than usual: reaching this socket *is* egress through this
    // sandbox's allowlist, and Landlock does not mediate unix-socket
    // connect at all (`tests/unix_socket_not_mediated.rs`), so filesystem
    // permissions are the only thing standing between another local user
    // and this sandbox's network reach.
    let _ = std::fs::remove_file(&paths.proxy_socket);
    let unix_listener = std::os::unix::net::UnixListener::bind(&paths.proxy_socket)?;
    std::fs::set_permissions(&paths.proxy_socket, std::fs::Permissions::from_mode(0o600))?;
    crate::lifecycle::clear_cloexec(unix_listener.as_raw_fd())?;

    std::fs::File::create(&paths.proxy_log)?;
    let log = std::fs::OpenOptions::new()
        .append(true)
        .open(&paths.proxy_log)?;

    let mut cmd = Command::new(exe);
    cmd.arg("__egress_proxy")
        .arg(listener.as_raw_fd().to_string())
        .arg(unix_listener.as_raw_fd().to_string())
        .env(
            "DEVCROFT_EGRESS_ALLOW",
            serde_json::to_string(allow).expect("Vec<String> serialization is infallible"),
        )
        .env("DEVCROFT_EGRESS_TOKEN", &token)
        // `server::run` opens this itself for structured allow/refuse
        // records (one write per record, same discipline
        // `keeper::connection::log_record` established) — a separate fd
        // to the same path this process's own stdout/stderr already
        // point at below, which is deliberate belt-and-suspenders: an
        // unexpected panic or a dependency's own stray `eprintln!` still
        // lands in the file even though it never goes through `run`'s
        // structured logging.
        .env("DEVCROFT_EGRESS_LOG", &paths.proxy_log)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: setsid() only touches this (freshly forked, single-
    // threaded) child's own session/process-group state — the same call,
    // same safety argument, as `up.rs::spawn_keeper`'s.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn()?;
    // Both listeners' fds must outlive this function for the child to
    // inherit them across exec; ownership passes to the proxy process.
    std::mem::forget(listener);
    std::mem::forget(unix_listener);
    Ok((child.id() as libc::pid_t, port, token))
}
