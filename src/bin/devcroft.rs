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
        Some("exec") => std::process::exit(cli_exec(&args[2..])),
        Some("shell") => std::process::exit(cli_shell(&args[2..])),
        other => {
            eprintln!(
                "devcroft: {} is not yet implemented (task group 7 wires up the CLI surface)",
                other.unwrap_or("(no command)")
            );
            std::process::exit(2);
        }
    }
}

/// `devcroft exec [name] -- <cmd> [args...]` (task 5.1). No auto-up yet
/// (task 5.3): if the sandbox isn't up, this reports it and exits rather
/// than starting one. Returns the process exit code directly, matching
/// the exec spec's "returns the command's exit code as its own" — the
/// child's exit status is passed through as-is and deliberately does not
/// follow CLAUDE.md's 0-5 layered contract, which is for devcroft's own
/// failures, not what it's asked to run.
fn cli_exec(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft exec: usage: devcroft exec [name] -- <cmd> [args...]";

    let Some(sep) = args.iter().position(|a| a == "--") else {
        eprintln!("{USAGE}");
        return 2;
    };
    let (name_args, rest) = args.split_at(sep);
    let command_args = &rest[1..];
    if name_args.len() > 1 {
        eprintln!("{USAGE}");
        return 2;
    }
    let Some((cmd, cmd_rest)) = command_args.split_first() else {
        eprintln!("devcroft exec: no command given after `--`");
        return 2;
    };

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft exec: cannot determine current directory: {e}");
            return 1;
        }
    };

    let sandbox_name = match name_args.first() {
        Some(name) => name.clone(),
        None => match resolve_sandbox_name(&cwd) {
            Ok(name) => name,
            Err(msg) => {
                eprintln!("devcroft exec: {msg}");
                return 2;
            }
        },
    };

    // No path remapping between host and sandbox (design.md decision 5:
    // MVP is access-restricted, not namespace-isolated) — the real host
    // cwd is already the right path inside the sandbox too, which is
    // exactly the exec spec's "Working directory mapping" scenario.
    let req = devcroft::exec::ExecRequest {
        cmd: cmd.clone(),
        args: cmd_rest.to_vec(),
        cwd: cwd.to_string_lossy().into_owned(),
    };

    match devcroft::exec::exec(&sandbox_name, &req) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("devcroft exec: {e}");
            5 // keeper/connection layer, per CLAUDE.md's error contract
        }
    }
}

/// `devcroft shell [name]` (task 5.2). Same no-auto-up posture as
/// `cli_exec` (task 5.3 adds it for both).
fn cli_shell(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft shell: usage: devcroft shell [name]";
    if args.len() > 1 {
        eprintln!("{USAGE}");
        return 2;
    }

    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("devcroft shell: cannot determine current directory: {e}");
            return 1;
        }
    };

    let sandbox_name = match args.first() {
        Some(name) => name.clone(),
        None => match resolve_sandbox_name(&cwd) {
            Ok(name) => name,
            Err(msg) => {
                eprintln!("devcroft shell: {msg}");
                return 2;
            }
        },
    };

    let req = devcroft::exec::ShellRequest {
        cwd: cwd.to_string_lossy().into_owned(),
    };

    match devcroft::exec::shell(&sandbox_name, &req) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("devcroft shell: {e}");
            5 // keeper/connection layer, per CLAUDE.md's error contract
        }
    }
}

/// Ancestor-walks from `start` for `devcroft.toml` (config::discover) and
/// returns the sandbox name it declares. The `cli` spec's full name
/// resolution (disambiguation, listing known sandboxes on failure) is
/// task group 7; this is the minimum `exec` needs to work without an
/// explicit name.
fn resolve_sandbox_name(start: &std::path::Path) -> Result<String, String> {
    let manifest_path = devcroft::config::discover(start).map_err(|_| {
        "no devcroft.toml found in this directory or its ancestors; pass a sandbox name explicitly"
            .to_string()
    })?;
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("reading {}: {e}", manifest_path.display()))?;
    let (manifest, _warnings) =
        devcroft::config::parse(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    Ok(manifest.sandbox.name)
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
