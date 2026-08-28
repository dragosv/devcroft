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
/// Returns the bound port (folded into the compiled policy via
/// `CompiledPolicy::with_proxy_port`, so the kernel gate matches the
/// process that will actually enforce it) and the spawned pid (recorded
/// in `paths.proxy_pidfile` by the caller so `down`/`rm` can tear it
/// down independently of the keeper).
pub fn spawn(exe: &Path, paths: &StatePaths, allow: &[String]) -> io::Result<(libc::pid_t, u16)> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    crate::lifecycle::clear_cloexec(listener.as_raw_fd())?;

    std::fs::File::create(&paths.proxy_log)?;
    let log = std::fs::OpenOptions::new()
        .append(true)
        .open(&paths.proxy_log)?;

    let mut cmd = Command::new(exe);
    cmd.arg("__egress_proxy")
        .arg(listener.as_raw_fd().to_string())
        .env(
            "DEVCROFT_EGRESS_ALLOW",
            serde_json::to_string(allow).expect("Vec<String> serialization is infallible"),
        )
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
    // The listener's fd must outlive this function for the child to
    // inherit it across exec; ownership passes to the proxy process.
    std::mem::forget(listener);
    Ok((child.id() as libc::pid_t, port))
}
