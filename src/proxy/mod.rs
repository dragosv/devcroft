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
    spawn_at(
        exe,
        &paths.proxy_socket,
        &paths.proxy_log,
        allow,
        |_| Ok(()),
    )
}

/// [`spawn`], with the unix socket and log at explicit paths and a hook
/// that can adjust the command before it runs. Fleet uses both: each agent
/// has its own proxy in its own directory, placed in a cgroup leaf of its
/// own through the hook.
pub fn spawn_at(
    exe: &Path,
    socket: &Path,
    log_path: &Path,
    allow: &[String],
    configure: impl FnOnce(&mut Command) -> io::Result<()>,
) -> io::Result<(libc::pid_t, u16, String)> {
    let token = generate_token();

    // The unix listener a network-isolated sandbox reaches the proxy
    // through — see `StatePaths::proxy_socket`. Bound here, host-side and
    // before exec, for the same listener-before-restriction reason the
    // control and SSH sockets are: the proxy process never gets to widen
    // anything for itself. Mode 0600 alongside the 0700 state dir: reaching
    // this socket *is* egress through this sandbox's allowlist, so
    // filesystem permissions are what stand between another local user and
    // this sandbox's network reach.
    let _ = std::fs::remove_file(socket);
    let unix_listener = std::os::unix::net::UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;

    std::fs::File::create(log_path)?;
    let log = std::fs::OpenOptions::new().append(true).open(log_path)?;

    // **The TCP port is the child's to bind, and it reports it here.**
    // `nono-proxy` binds its own listener and accepts an extra one only on
    // that same port (`ProxyHandle::add_listener`), so a listener bound
    // before exec cannot be handed to it. The proxy is never restricted, so
    // nothing about listener-before-restriction is lost; what matters is
    // that the port is known before `up` compiles it into the policy, and
    // this call returns only once the child has written it.
    let (port_r, port_w) = pipe()?;

    // Inheritable in the child alone, never made so here: a fleet
    // supervisor has other threads forking, and a fd with CLOEXEC cleared
    // leaks into their children for good (`lifecycle::pass_fds`).
    let mut cmd = Command::new(exe);
    crate::lifecycle::pass_fds(&mut cmd, &[port_w.as_raw_fd(), unix_listener.as_raw_fd()]);
    cmd.arg("__egress_proxy")
        .arg(port_w.as_raw_fd().to_string())
        .arg(unix_listener.as_raw_fd().to_string())
        .env(
            "DEVCROFT_EGRESS_ALLOW",
            serde_json::to_string(allow).expect("Vec<String> serialization is infallible"),
        )
        .env("DEVCROFT_EGRESS_TOKEN", &token)
        // `server::start` opens this itself for the allow/refuse records;
        // stdout/stderr point at it too, so a panic still lands in the file.
        .env("DEVCROFT_EGRESS_LOG", log_path)
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

    configure(&mut cmd)?;
    let mut child = cmd.spawn()?;
    // Ours are closed here, not leaked: with CLOEXEC cleared in the child
    // only, every process started afterwards would otherwise inherit them,
    // and a process in agent B could `accept()` agent A's egress. Closing
    // the write end also makes the read below see EOF if the child dies.
    drop(port_w);
    drop(unix_listener);
    match read_port(port_r) {
        Ok(port) => Ok((child.id() as libc::pid_t, port, token)),
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(io::Error::new(
                e.kind(),
                format!(
                    "the egress proxy did not report its port ({e}); see {}",
                    log_path.display()
                ),
            ))
        }
    }
}

/// How long `spawn_at` waits for the child to report its port. Starting
/// `nono-proxy` takes milliseconds; this only bounds a child that hangs.
const PORT_REPORT_TIMEOUT_MS: i32 = 10_000;

fn pipe() -> io::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::FromRawFd;
    let mut fds = [0; 2];
    // `pipe` + `FD_CLOEXEC` rather than `pipe2`, which macOS lacks. The
    // window between the two is narrow and its worst case is bounded: a
    // fleet supervisor thread forking in it would carry the write end into
    // its child, which can only hold it open, turning "the proxy died
    // before reporting" from an immediate EOF into `read_port`'s timeout.
    // It cannot read the port or write a false one it was never told of.
    // SAFETY: `fds` is a valid two-element array for pipe to fill.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: pipe succeeded, so both are fresh fds owned by nobody else.
    let (r, w) = unsafe {
        (
            std::os::fd::OwnedFd::from_raw_fd(fds[0]),
            std::os::fd::OwnedFd::from_raw_fd(fds[1]),
        )
    };
    for fd in [&r, &w] {
        // SAFETY: a valid fd we own.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok((r, w))
}

/// Reads `<port>\n` from the child, bounded by [`PORT_REPORT_TIMEOUT_MS`].
fn read_port(fd: std::os::fd::OwnedFd) -> io::Result<u16> {
    use std::io::Read;
    let mut file = std::fs::File::from(fd);
    let mut buf = Vec::new();
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(PORT_REPORT_TIMEOUT_MS as u64);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let mut pfd = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let n = unsafe { libc::poll(&mut pfd, 1, left.as_millis() as i32) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "timed out"));
        }
        let mut chunk = [0u8; 16];
        let got = file.read(&mut chunk)?;
        if got == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "it exited first",
            ));
        }
        buf.extend_from_slice(&chunk[..got]);
        if let Some(end) = buf.iter().position(|b| *b == b'\n') {
            return std::str::from_utf8(&buf[..end])
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not a port"));
        }
        if buf.len() > 8 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a port"));
        }
    }
}

/// The proxy variables a sandbox's environment gets: every standard HTTP
/// client turns `token@` in a proxy URL into `Proxy-Authorization`
/// unprompted, and `NO_PROXY` keeps a client's own loopback traffic (a test
/// hitting a dev server) away from a proxy that would refuse it. Shared by
/// `up` and fleet, so the two cannot disagree on what a sandbox sees.
pub fn client_env(port: u16, token: &str) -> Vec<(String, String)> {
    // An empty token is a client that reaches the proxy through its unix
    // socket (a network-isolated sandbox's relay), where the bridge presents
    // the token itself (`server::bridge_unix_to_proxy`): no userinfo, so
    // clients that mishandle it are not handed one.
    let endpoint = if token.is_empty() {
        format!("http://127.0.0.1:{port}")
    } else {
        // `nono:` is the user `nono-proxy` checks a Basic credential for;
        // `<token>@` alone is refused (measured).
        format!("http://nono:{token}@127.0.0.1:{port}")
    };
    let mut vars: Vec<(String, String)> =
        ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"]
            .iter()
            .map(|k| (k.to_string(), endpoint.clone()))
            .collect();
    for key in ["NO_PROXY", "no_proxy"] {
        vars.push((key.to_string(), "localhost,127.0.0.1,::1".to_string()));
    }
    vars
}
