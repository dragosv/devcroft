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
                .expect("__keeper requires a control-socket fd argument");
            let ssh_fd: RawFd = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .expect("__keeper requires an ssh-socket fd argument");
            keeper_main(fd, ssh_fd);
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

/// `devcroft exec [--no-up] [name] -- <cmd> [args...]` (task 5.1, plus
/// auto-up from task 5.3). Returns the process exit code directly,
/// matching the exec spec's "returns the command's exit code as its
/// own" — the child's exit status is passed through as-is and
/// deliberately does not follow CLAUDE.md's 0-5 layered contract, which
/// is for devcroft's own failures, not what it's asked to run.
fn cli_exec(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft exec: usage: devcroft exec [--no-up] [name] -- <cmd> [args...]";

    let Some(sep) = args.iter().position(|a| a == "--") else {
        eprintln!("{USAGE}");
        return 2;
    };
    let (name_args, rest) = args.split_at(sep);
    let command_args = &rest[1..];
    // `--no-up` only counts ahead of `--`: the exec spec's own command
    // could legitimately want to pass that literal string to whatever
    // it's running (`exec -- mytool --no-up`), so only the devcroft-level
    // arguments before the separator are ever inspected for it.
    let no_up = name_args.iter().any(|a| a == "--no-up");
    let name_args: Vec<&String> = name_args.iter().filter(|a| *a != "--no-up").collect();
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
        Some(name) => (*name).clone(),
        None => match resolve_sandbox_name(&cwd) {
            Ok(name) => name,
            Err(msg) => {
                eprintln!("devcroft exec: {msg}");
                return 2;
            }
        },
    };

    if !no_up && let Err(msg) = maybe_auto_up(&sandbox_name, &cwd) {
        eprintln!("devcroft exec: {msg}");
        return 3; // environment/provider layer, per CLAUDE.md's error contract
    }

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

/// `devcroft shell [--no-up] [name]` (task 5.2, plus auto-up from task
/// 5.3).
fn cli_shell(args: &[String]) -> i32 {
    const USAGE: &str = "devcroft shell: usage: devcroft shell [--no-up] [name]";
    let no_up = args.iter().any(|a| a == "--no-up");
    let name_args: Vec<&String> = args.iter().filter(|a| *a != "--no-up").collect();
    if name_args.len() > 1 {
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

    let sandbox_name = match name_args.first() {
        Some(name) => (*name).clone(),
        None => match resolve_sandbox_name(&cwd) {
            Ok(name) => name,
            Err(msg) => {
                eprintln!("devcroft shell: {msg}");
                return 2;
            }
        },
    };

    if !no_up && let Err(msg) = maybe_auto_up(&sandbox_name, &cwd) {
        eprintln!("devcroft shell: {msg}");
        return 3; // environment/provider layer, per CLAUDE.md's error contract
    }

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
/// task group 7; this is the minimum `exec`/`shell` need to work without
/// an explicit name.
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

/// Auto-up convenience (exec spec, task 5.3): if `sandbox_name` isn't
/// healthy, brings it up before the caller proceeds, printing a line
/// first so its output precedes whatever `exec`/`shell` streams next
/// (spec scenario: "the `up` output preceding the prompt"). Silent, and
/// `Ok`, when the sandbox is already healthy *or* no manifest matching
/// `sandbox_name` can be discovered from `cwd` — the latter just leaves
/// `exec`/`shell` to fail with their own "not running" error afterward,
/// same as before this existed. Only an attempted-and-failed `up` is
/// reported as an error here.
fn maybe_auto_up(sandbox_name: &str, cwd: &std::path::Path) -> Result<(), String> {
    let paths = devcroft::lifecycle::StatePaths::new(sandbox_name)
        .map_err(|e| format!("resolving state paths for '{sandbox_name}': {e}"))?;
    let healthy = matches!(
        devcroft::lifecycle::health(&paths)
            .map_err(|e| format!("checking '{sandbox_name}' health: {e}"))?,
        devcroft::lifecycle::Health::Healthy(_)
    );
    if healthy {
        return Ok(());
    }

    let Ok(manifest_path) = devcroft::config::discover(cwd) else {
        return Ok(());
    };
    let Ok(text) = std::fs::read_to_string(&manifest_path) else {
        return Ok(());
    };
    let Ok((manifest, _warnings)) = devcroft::config::parse(&text) else {
        return Ok(());
    };
    // The manifest found by walking up from `cwd` might not even be the
    // one for the sandbox the user actually named — don't `up` an
    // unrelated project just because its devcroft.toml happened to be
    // the nearest one.
    if manifest.sandbox.name != sandbox_name {
        return Ok(());
    }
    let project_root = manifest_path.parent().unwrap_or(cwd);

    eprintln!("devcroft: sandbox '{sandbox_name}' is not up; starting it...");
    devcroft::lifecycle::up(
        &manifest,
        project_root,
        &devcroft::lifecycle::UpOptions::default(),
    )
    .map(|_| ())
    .map_err(|e| format!("starting sandbox '{sandbox_name}': {e}"))
}

/// Runs post-restriction, inside the boundary `nono wrap` just applied
/// (CLAUDE.md's listener-before-restriction ordering: the fd was created
/// by `up`, before restriction, and inherited across nono's exec — this
/// is the load-bearing trick the task 1.1/1.2 spike proved out). Never
/// returns under normal operation.
fn keeper_main(fd: RawFd, ssh_fd: RawFd) -> ! {
    // SAFETY: `up` created both listeners before restriction, cleared
    // their FD_CLOEXEC, and passed the fd numbers as this process's argv
    // — they are ours alone to take ownership of.
    let listener = unsafe { UnixListener::from_raw_fd(fd) };
    let ssh_listener = unsafe { UnixListener::from_raw_fd(ssh_fd) };

    let keeper = Keeper::new(listener);
    // Must run before anything else spawns a thread — including the ssh
    // server below, whose tokio runtime spawns its own worker-thread pool
    // immediately. `install_shutdown_handler` blocks SIGTERM/SIGINT/
    // SIGHUP on *this* thread specifically so every thread created after
    // it inherits that block; a thread created before it would inherit
    // the signals unblocked instead, and a `SIGTERM` sent to the process
    // could then land on that thread and kill it via the kernel's default
    // disposition before the graceful-shutdown logic below ever runs —
    // exactly what broke `down`'s "terminate live sessions" guarantee the
    // first time ssh startup was ordered ahead of this.
    install_shutdown_handler(Arc::clone(keeper.registry()));

    // Best-effort (task 6.1): a broken ssh handoff logs to this process's
    // own stderr (redirected by `up` to `<state>/<name>/keeper.log`) and
    // leaves ssh unavailable for this sandbox rather than taking the
    // whole keeper down — exec/shell must keep working regardless.
    devcroft::ssh::start_from_env(ssh_listener);

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
