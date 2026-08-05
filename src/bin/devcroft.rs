//! The `devcroft` binary. Its only real entrypoint today is the hidden
//! `__keeper` mode `lifecycle::up` re-execs into via `nono wrap` (see
//! `up.rs`'s module docs) — the user-facing command surface the `cli`
//! spec describes (`init`, `up`, `down`, ...) is built incrementally as
//! each capability lands; task group 7 is where it gets polished with
//! real argument parsing and the error contract's exit codes.

use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::net::UnixListener;
use std::sync::Arc;

use devcroft::keeper::{Keeper, Registry};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("__keeper") => {
            let fd: RawFd = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .expect("__keeper requires a listener fd argument");
            keeper_main(fd);
        }
        other => {
            eprintln!(
                "devcroft: {} is not yet implemented (task group 7 wires up the CLI surface)",
                other.unwrap_or("(no command)")
            );
            std::process::exit(2);
        }
    }
}

/// Runs post-restriction, inside the boundary `nono wrap` just applied
/// (CLAUDE.md's listener-before-restriction ordering: the fd was created
/// by `up`, before restriction, and inherited across nono's exec — this
/// is the load-bearing trick the task 1.1/1.2 spike proved out). Never
/// returns under normal operation.
fn keeper_main(fd: RawFd) -> ! {
    // SAFETY: `up` created this listener before restriction, cleared its
    // FD_CLOEXEC, and passed the fd number as this process's argv — it is
    // ours alone to take ownership of.
    let listener = unsafe { UnixListener::from_raw_fd(fd) };
    let keeper = Keeper::new(listener);
    install_shutdown_handler(Arc::clone(keeper.registry()));
    let _ = keeper.serve();
    std::process::exit(0);
}

/// `down`/`rm` (lifecycle::terminate) signal this process directly, then
/// wait for it to actually exit before touching its state files — so the
/// contract here is: on SIGTERM/SIGINT/SIGHUP, terminate every live
/// session's process group (grace period, then SIGKILL) and exit. The
/// supervisor's own outer grace period (`lifecycle::terminate::GRACE_PERIOD`,
/// 5s) is sized to leave this inner one room to finish first.
fn install_shutdown_handler(registry: Arc<Registry>) {
    const SESSION_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

    // Signal handlers can only safely do async-signal-safe work (no
    // locks, no allocation) — Registry::snapshot needs both. So instead
    // of a handler, block these signals on every thread (a mask set here,
    // before `keeper.serve()` spawns any per-connection threads, is
    // inherited by all of them) and have one dedicated thread block in
    // `sigwait`, which delivers the signal through an ordinary blocking
    // call on a normal thread — safe to do anything from.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGHUP);
        if libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) != 0 {
            panic!(
                "blocking shutdown signals: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    std::thread::spawn(move || {
        let mut received: libc::c_int = 0;
        if unsafe { libc::sigwait(&set, &mut received) } != 0 {
            return;
        }
        for (_, info) in registry.snapshot() {
            unsafe {
                libc::kill(-info.pgid, libc::SIGTERM);
            }
        }
        std::thread::sleep(SESSION_GRACE);
        for (_, info) in registry.snapshot() {
            unsafe {
                libc::kill(-info.pgid, libc::SIGKILL);
            }
        }
        std::process::exit(0);
    });
}
