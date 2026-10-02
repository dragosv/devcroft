//! The agent's init helper (D2): the supervisor's `clone3` and the
//! re-exec'd `__fleet_init` that becomes PID 1 of the agent's namespaces.
//!
//! **Supervisor side ([`spawn`]).** One `clone3` creates the user, mount,
//! PID, network, IPC, UTS and cgroup namespaces, places the child in its
//! cgroup leaf (`CLONE_INTO_CGROUP`, so it never runs outside it), and
//! returns a pidfd. The child does nothing but `dup2` and `execve`: the
//! supervisor may be multi-threaded, and after a `fork`-like clone only
//! async-signal-safe calls are sound. Everything the child needs is built
//! before the clone.
//!
//! **Handshake.** The parent writes the identity map (`0 → ` its own
//! uid/gid, `setgroups` denied first), then releases the child, which has
//! been blocked in `read` on a sync pipe. **The child must not `execve`
//! before the map exists.** An unmapped uid is not 0, and an `execve` by a
//! non-zero uid drops the whole capability set, so the helper would start
//! with nothing and fail at its first mount. The first version exec'd
//! straight away, and that race failed about half the tests with `private
//! propagation: EPERM`. The configuration goes to the helper's fd 3 after
//! that. The helper reports back once on fd 4: a
//! single JSON line, either `{"ok":true}` or an error naming the step that
//! failed. That is the structured error channel section 2 asks for; a
//! helper that dies before reporting shows up as an EOF plus its exit
//! status.
//!
//! **Helper side ([`helper_main`]).** It is PID 1, which is forced rather
//! than chosen: if PID 1 exits, the kernel kills the whole namespace. So it
//! does not `exec` itself away. It starts the agent's command as its child,
//! reaps everything that gets reparented to it, forwards termination
//! signals, and exits with the command's status. That exit then takes
//! every leftover process in the namespace with it.
//!
//! The helper also mounts a **fresh `/proc`**, since without one a PID
//! namespace hides nothing (the inherited procfs still lists the host). And
//! it starts the command with `SECBIT_NOROOT`: the helper is uid 0 in its
//! user namespace, and without that bit anything it execs would hold every
//! capability there.
//!
//! **The mount view is optional per agent** ([`AgentSpec::view`]),
//! which is D2a's choice made explicit. With one, the helper builds
//! `fleet::mount::construct_view` with a fresh `/proc` and pivots into it,
//! so the agent sees only what its plan grants. Without one, the agent
//! keeps the host's root and gets only the fresh `/proc`. The grants are
//! resolved in the helper, before `pivot_root` while host paths still mean
//! what they say, and by the same resolver Landlock's rules come from, so
//! the view and the ruleset cannot disagree.
//!
//! **Then the helper restricts itself**, from the same plan, the way `up`'s
//! keeper does (`to_capability_set` plus `nono::Sandbox::apply_auto`), and
//! the command inherits it. That happens after the view rather than in the
//! supervisor, as D2 first had it, because the view's `/tmp` and
//! `/dev/pts` are new filesystem instances. Landlock rules bind to inodes,
//! so rules built in the parent would attach to the host's and miss the
//! agent's. PID 1 is confined too, so nothing in the namespace runs
//! unrestricted once the command exists.
//!
//! What is not here yet: the keeper, which becomes the command.

use std::ffi::CString;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use serde::{Deserialize, Serialize};

use super::cgroup::Leaf;

/// The hidden subcommand the helper runs as.
pub const SUBCOMMAND: &str = "__fleet_init";

/// The fds the helper finds its configuration and status channel on.
const CONFIG_FD: RawFd = 3;
const STATUS_FD: RawFd = 4;

/// Where [`Stdio::inherit`]'s fds land, in order, in both the helper and
/// the command: the first is fd 5, the next 6, and so on.
pub const FIRST_INHERITED_FD: RawFd = 5;

// Not exported by `libc` for every target; stable kernel ABI.
const CLONE_INTO_CGROUP: u64 = 0x2_0000_0000;
const SECBIT_NOROOT: libc::c_ulong = 1 << 0;
const SECBIT_NOROOT_LOCKED: libc::c_ulong = 1 << 1;

/// `struct clone_args` (`<linux/sched.h>`), up to the `cgroup` field
/// (`CLONE_ARGS_SIZE_VER2`).
#[repr(C)]
#[derive(Default)]
struct CloneArgs {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

/// What the agent runs, sent to the helper over its configuration pipe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSpec {
    /// The program and its arguments. The program is resolved against the
    /// `PATH` in `env`, as `Command` does.
    pub command: Vec<String>,
    /// The command's whole environment; nothing is inherited.
    pub env: Vec<(String, String)>,
    /// The command's working directory.
    pub cwd: Option<PathBuf>,
    /// The agent's hostname, in its own UTS namespace.
    pub hostname: String,
    /// The compiled policy. Always applied: an agent without one would
    /// not be a sandbox, so it is not representable. With a view, its
    /// grants are also exactly what the view contains.
    pub plan: crate::policy::CapabilityPlan,
    /// What relative grants such as `.` resolve against.
    pub project_root: PathBuf,
    /// The port of the keeper's egress relay, when the agent has egress.
    /// The helper binds it on the agent's loopback **before** restricting,
    /// since the policy grants only `connect` to it, and hands it to the
    /// command as `DEVCROFT_PROXY_RELAY_FD` (listener before restriction,
    /// one level down: a keeper that starts restricted cannot bind it).
    #[serde(default)]
    pub relay_port: Option<u16>,
    /// The agent's filesystem view. `None` keeps the host's root (D2a's
    /// host-root strategy), with only `/proc` replaced; Landlock still
    /// refuses everything the plan does not grant, but it stays visible.
    #[serde(default)]
    pub view: Option<View>,
    /// A wall-clock limit, in seconds, enforced by PID 1 (the only process
    /// that lives exactly as long as the agent, since fleet has no daemon):
    /// SIGTERM to the command when it passes, and after
    /// `lifecycle::GRACE_PERIOD` PID 1 exits, which kills everything left.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// How an agent's PID 1 ended, written to [`Stdio::report`] as one JSON
/// object. Absent when PID 1 was killed (a `stop`, an OOM kill of the
/// whole leaf): it reports only an end it saw.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExitReport {
    /// The command's exit code, or 128 + the signal that killed it.
    pub code: i32,
    /// Whether [`AgentSpec::timeout_secs`] ran out first.
    pub timed_out: bool,
}

/// A minimal root for one agent, built from [`AgentSpec::plan`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct View {
    /// An existing, empty directory to build the view on. It stays empty
    /// on the host: everything is mounted inside the agent's namespace.
    pub root: PathBuf,
    /// The agent's egress proxy socket, bound into the view by itself.
    pub proxy_socket: Option<PathBuf>,
}

/// A running agent: its init helper, seen from the supervisor.
#[derive(Debug)]
pub struct Agent {
    pid: libc::pid_t,
    pidfd: OwnedFd,
}

/// The agent's fds. For stdout and stderr, `None` inherits the
/// supervisor's; stdin is always `/dev/null`.
#[derive(Debug, Default)]
pub struct Stdio {
    pub stdout: Option<OwnedFd>,
    pub stderr: Option<OwnedFd>,
    /// Handed to the command at [`FIRST_INHERITED_FD`] onward. This is how
    /// the keeper gets its listening sockets: bound by the supervisor, on
    /// the host, before any restriction exists (the listener-before-
    /// restriction invariant), and reachable from outside only because
    /// they predate it. PID 1 closes its own copies once the command has
    /// them.
    pub inherit: Vec<OwnedFd>,
    /// A lifeline's write end (`fleet::lifeline`), held by PID 1 alone for
    /// as long as it lives, and never by the command: PID 1's exit is the
    /// agent's, so its host-side helpers see EOF exactly then.
    pub lifeline: Option<OwnedFd>,
    /// Where PID 1 writes its [`ExitReport`], held by it alone. A file the
    /// supervisor opened in the agent's state directory, which the command
    /// can neither reach (baseline-denied) nor inherit, so the report
    /// cannot be forged by what it reports on.
    pub report: Option<OwnedFd>,
}

/// What crosses the configuration pipe: the spec plus how many fds were
/// inherited, which only `spawn` knows.
#[derive(Serialize)]
struct ConfigOut<'a> {
    spec: &'a AgentSpec,
    inherited: usize,
    /// Whether a lifeline follows the inherited fds.
    lifeline: bool,
    /// Whether a report fd follows that.
    report: bool,
}

#[derive(Deserialize)]
struct ConfigIn {
    spec: AgentSpec,
    inherited: usize,
    lifeline: bool,
    report: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Status {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Start an agent: its init helper in fresh namespaces, inside `leaf`,
/// running `spec`. Returns once the helper has reported that the command
/// started, or with the helper's own error if it did not.
pub fn spawn(exe: &Path, leaf: &Leaf, spec: &AgentSpec, stdio: Stdio) -> io::Result<Agent> {
    let config = serde_json::to_vec(&ConfigOut {
        spec,
        inherited: stdio.inherit.len(),
        lifeline: stdio.lifeline.is_some(),
        report: stdio.report.is_some(),
    })
    .map_err(io::Error::other)?;
    let (config_r, mut config_w) = pipe()?;
    let (mut status_r, status_w) = pipe()?;
    let (sync_r, sync_w) = pipe()?;
    let devnull: OwnedFd = std::fs::File::open("/dev/null")?.into();

    // Every fd the child ends up with, moved above the targets first so
    // no `dup2` in the child can overwrite a source it has not used yet.
    let mut plan: Vec<(OwnedFd, RawFd)> = vec![
        (high(&devnull)?, 0),
        (high(&config_r)?, CONFIG_FD),
        (high(&status_w)?, STATUS_FD),
    ];
    if let Some(fd) = &stdio.stdout {
        plan.push((high(fd)?, 1));
    }
    if let Some(fd) = &stdio.stderr {
        plan.push((high(fd)?, 2));
    }
    for (n, fd) in stdio.inherit.iter().enumerate() {
        plan.push((high(fd)?, FIRST_INHERITED_FD + n as RawFd));
    }
    // PID 1's own fds, right after the inherited range, in this order.
    let first_kept = FIRST_INHERITED_FD + stdio.inherit.len() as RawFd;
    for (target, fd) in (first_kept..).zip([&stdio.lifeline, &stdio.report].into_iter().flatten()) {
        plan.push((high(fd)?, target));
    }
    drop((config_r, status_w, devnull, stdio));
    let dups: Vec<(RawFd, RawFd)> = plan.iter().map(|(fd, t)| (fd.as_raw_fd(), *t)).collect();
    // Read in place, not dup'd: close-on-exec, the helper never sees it.
    let sync_fd = sync_r.as_raw_fd();

    let exe_c = CString::new(exe.as_os_str().as_bytes())?;
    let sub_c = CString::new(SUBCOMMAND)?;
    let argv = [exe_c.as_ptr(), sub_c.as_ptr(), std::ptr::null()];
    let envp = [std::ptr::null::<libc::c_char>()];

    // The leaf fd is `O_CLOEXEC`: the helper holds it only until it execs.
    let leaf_fd = leaf.open_dir()?;
    let mut pidfd: libc::c_int = -1;
    let args = CloneArgs {
        flags: (libc::CLONE_NEWUSER
            | libc::CLONE_NEWNS
            | libc::CLONE_NEWPID
            | libc::CLONE_NEWNET
            | libc::CLONE_NEWIPC
            | libc::CLONE_NEWUTS
            | libc::CLONE_NEWCGROUP
            | libc::CLONE_PIDFD) as u64
            | CLONE_INTO_CGROUP,
        pidfd: &mut pidfd as *mut libc::c_int as u64,
        exit_signal: libc::SIGCHLD as u64,
        cgroup: leaf_fd.as_raw_fd() as u64,
        ..CloneArgs::default()
    };

    // Read before cloning: from inside the new user namespace these report
    // the overflow id, and the map needs the real ones.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };

    // SAFETY: `args` is a valid `clone_args` of the size passed. Without
    // CLONE_VM the child gets a copy of this address space, like `fork`.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_clone3,
            &args as *const CloneArgs,
            std::mem::size_of::<CloneArgs>(),
        )
    };
    if ret == 0 {
        // Child. Only async-signal-safe calls from here to `execve`:
        // `dups`, `argv` and `envp` were all built before the clone.
        unsafe {
            // Wait for the identity map (see the module doc). EOF means
            // the parent gave up.
            let mut go = 0u8;
            if libc::read(sync_fd, (&mut go as *mut u8).cast(), 1) != 1 {
                libc::_exit(125);
            }
            for &(src, target) in &dups {
                if libc::dup2(src, target) < 0 {
                    libc::_exit(126);
                }
            }
            libc::execve(argv[0], argv.as_ptr(), envp.as_ptr());
            libc::_exit(127);
        }
    }
    if ret < 0 {
        return Err(io::Error::other(format!(
            "clone3 into {}: {}",
            leaf.path().display(),
            io::Error::last_os_error()
        )));
    }
    let pid = ret as libc::pid_t;
    // SAFETY: CLONE_PIDFD filled `pidfd` with a new fd this process owns.
    let agent = Agent {
        pid,
        pidfd: unsafe { OwnedFd::from_raw_fd(pidfd) },
    };
    drop((plan, leaf_fd));

    drop(sync_r);
    let handshake = (|| {
        write_id_maps(pid, uid, gid)?;
        (&sync_w).write_all(b"1")?;
        drop(sync_w);
        config_w.write_all(&config)?;
        drop(config_w);
        let mut line = String::new();
        status_r.read_to_string(&mut line)?;
        Ok::<_, io::Error>(line)
    })();

    let failure = match handshake {
        Ok(line) => match serde_json::from_str::<Status>(line.trim()) {
            Ok(Status { ok: true, .. }) => return Ok(agent),
            Ok(Status { error: Some(e), .. }) => e,
            _ if line.trim().is_empty() => "exited before reporting".to_owned(),
            _ => format!("unreadable status {line:?}"),
        },
        Err(e) => format!("handshake: {e}"),
    };
    let _ = agent.signal(libc::SIGKILL);
    let status = agent.wait()?;
    Err(io::Error::other(format!(
        "fleet init: {failure} ({status})"
    )))
}

impl Agent {
    /// The helper's pid, in the supervisor's PID namespace.
    pub fn pid(&self) -> u32 {
        self.pid as u32
    }

    /// Send `signal` to the helper, which forwards termination signals to
    /// the command. Through the pidfd, so a reused pid is never signalled.
    pub fn signal(&self, signal: libc::c_int) -> io::Result<()> {
        // SAFETY: a pidfd this struct owns and a null siginfo.
        let r = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.pidfd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if r < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// The helper's exit status if it has already exited, reaping it;
    /// `None` while it runs.
    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        let mut status = 0;
        // SAFETY: our own child; `status` is a valid out-pointer.
        match unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) } {
            0 => Ok(None),
            n if n > 0 => Ok(Some(ExitStatus::from_raw(status))),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Wait for the helper, which exits with the command's status.
    pub fn wait(self) -> io::Result<ExitStatus> {
        let mut status = 0;
        loop {
            // SAFETY: our own child; `status` is a valid out-pointer.
            if unsafe { libc::waitpid(self.pid, &mut status, 0) } >= 0 {
                return Ok(ExitStatus::from_raw(status));
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
    }
}

/// `0 → uid` and `0 → gid`, single entries: no `/etc/subuid` range and no
/// `newuidmap`, and files the agent creates stay owned by the real user.
/// `setgroups` must be denied before an unprivileged `gid_map` write.
fn write_id_maps(pid: libc::pid_t, uid: libc::uid_t, gid: libc::gid_t) -> io::Result<()> {
    std::fs::write(format!("/proc/{pid}/setgroups"), "deny")?;
    std::fs::write(format!("/proc/{pid}/uid_map"), format!("0 {uid} 1"))?;
    std::fs::write(format!("/proc/{pid}/gid_map"), format!("0 {gid} 1"))?;
    Ok(())
}

fn pipe() -> io::Result<(std::fs::File, std::fs::File)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` is a valid two-element out-array.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both fds were just created and are owned by nothing else.
    Ok(unsafe {
        (
            std::fs::File::from_raw_fd(fds[0]),
            std::fs::File::from_raw_fd(fds[1]),
        )
    })
}

/// A close-on-exec duplicate at 10 or above, clear of every `dup2` target.
fn high(fd: &impl AsRawFd) -> io::Result<OwnedFd> {
    // SAFETY: F_DUPFD_CLOEXEC on a valid fd returns a new one we own.
    let new = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
    if new < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(new) })
}

/// The helper: PID 1 of the agent's namespaces. Returns the exit code.
pub fn helper_main() -> i32 {
    // The command must not inherit either channel.
    for fd in [CONFIG_FD, STATUS_FD] {
        // SAFETY: setting a flag on an fd; harmless if it is not open.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    // SAFETY: fd 4 was placed by `spawn` and is owned by nothing else here.
    let mut status = unsafe { std::fs::File::from_raw_fd(STATUS_FD) };
    let signals = forwarded_signals();
    match setup(&signals) {
        Ok(started) => {
            let _ = writeln!(status, "{}", serde_json::json!({ "ok": true }));
            drop(status);
            let timeout = started.timeout_secs.map(std::time::Duration::from_secs);
            let (code, timed_out) = supervise(started.main, &signals, timeout);
            if let Some(mut report) = started.report {
                let _ = writeln!(
                    report,
                    "{}",
                    serde_json::to_string(&ExitReport { code, timed_out })
                        .expect("ExitReport serializes")
                );
            }
            code
        }
        Err(e) => {
            let _ = writeln!(status, "{}", serde_json::json!({ "error": e.to_string() }));
            1
        }
    }
}

/// What `setup` hands `supervise`: the command, and PID 1's own business.
struct Started {
    main: libc::pid_t,
    report: Option<std::fs::File>,
    timeout_secs: Option<u64>,
}

fn setup(signals: &libc::sigset_t) -> io::Result<Started> {
    // Off the supervisor's terminal, as `up` detaches its keeper. A clone
    // child is never a group leader, so this cannot fail with EPERM.
    // SAFETY: no arguments; affects only this process.
    unsafe { libc::setsid() };
    // SAFETY: fd 3 was placed by `spawn`.
    let mut config = unsafe { std::fs::File::from_raw_fd(CONFIG_FD) };
    let mut raw = Vec::new();
    config
        .read_to_end(&mut raw)
        .map_err(|e| step("read configuration", e))?;
    let ConfigIn {
        spec,
        inherited,
        lifeline,
        report,
    } = serde_json::from_slice(&raw)
        .map_err(|e| step("parse configuration", io::Error::other(e)))?;
    // Every fd above the ones this agent was given is something the
    // supervisor leaked (an fd with CLOEXEC cleared for some other child),
    // and it would otherwise reach the agent's command. Measured to matter:
    // an egress proxy's listeners, cleared for the proxy to inherit, would
    // let a process in one agent `accept()` another agent's egress.
    // The lifeline sits just past the inherited range. PID 1 keeps it for
    // its whole life, and the command must never get it: a command that
    // forked something outside the agent (it cannot, but a daemonizing
    // child of one would be the way) would keep the helpers alive.
    // The report fd follows it, kept and hidden the same way.
    let lifeline_fd = FIRST_INHERITED_FD + inherited as RawFd;
    let report_fd = lifeline_fd + lifeline as RawFd;
    for (wanted, fd) in [(lifeline, lifeline_fd), (report, report_fd)] {
        if wanted {
            // SAFETY: setting a flag on an fd `spawn` placed.
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
    }
    // SAFETY: closes only descriptors this process does not use; 0-4, the
    // inherited range, the lifeline and the report fd are kept.
    if unsafe {
        libc::syscall(
            libc::SYS_close_range,
            report_fd as u32 + report as u32,
            u32::MAX,
            0u32,
        )
    } != 0
    {
        return Err(step("close leaked descriptors", io::Error::last_os_error()));
    }
    if spec.command.is_empty() {
        return Err(io::Error::other("configuration: empty command"));
    }

    // The map is written before the configuration, so it must be here.
    if unsafe { libc::getuid() } != 0 {
        return Err(io::Error::other(
            "identity map: not uid 0 after the handshake",
        ));
    }
    if std::process::id() != 1 {
        return Err(io::Error::other("not PID 1 of a new PID namespace"));
    }

    super::mount::make_propagation_private().map_err(|e| step("private propagation", e))?;
    match &spec.view {
        Some(view) => {
            let grants = spec
                .plan
                .resolved_grants(&spec.project_root)
                .map_err(|e| step("resolve grants", io::Error::other(e.to_string())))?;
            super::mount::construct_view(
                &view.root,
                &grants,
                view.proxy_socket.as_deref(),
                super::mount::ProcMount::Fresh,
            )
            .map_err(|e| step("construct the view", e))?;
        }
        None => mount_fresh_proc().map_err(|e| step("mount a fresh /proc", e))?,
    }
    // SAFETY: a valid buffer and its length.
    if unsafe { libc::sethostname(spec.hostname.as_ptr().cast(), spec.hostname.len()) } != 0 {
        return Err(step("sethostname", io::Error::last_os_error()));
    }
    super::netns::bring_loopback_up().map_err(|e| step("bring lo up", e))?;

    // Bound now, while nothing is restricted, at the fd after the inherited
    // ones and the lifeline (every fd above those was just closed, so it is
    // free).
    let relay_fd = report_fd + report as RawFd;
    let mut env = spec.env.clone();
    if let Some(port) = spec.relay_port {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| step(&format!("bind the egress relay on 127.0.0.1:{port}"), e))?;
        // Taken out of the listener rather than dup2'd from it: 0-6 are all
        // in use here, so `bind` usually returns the very fd wanted, and
        // `dup2(fd, fd)` is a no-op that leaves CLOEXEC set, after which
        // dropping the listener closes the relay (measured: the keeper
        // found an eventfd at its number and every agent's egress was
        // refused).
        let raw = std::os::fd::IntoRawFd::into_raw_fd(listener);
        // SAFETY: `raw` is ours; `relay_fd` is free (everything above the
        // inherited range was closed) or is `raw` itself.
        unsafe {
            if raw != relay_fd {
                if libc::dup2(raw, relay_fd) < 0 {
                    return Err(step("place the egress relay", io::Error::last_os_error()));
                }
                libc::close(raw);
            }
            // Inherited by the command, so not close-on-exec.
            libc::fcntl(relay_fd, libc::F_SETFD, 0);
        }
        env.push(("DEVCROFT_PROXY_RELAY_FD".into(), relay_fd.to_string()));
    }

    // Resolved now, inside the view: the rules must bind to the inodes the
    // agent will see, its private /tmp and devpts included.
    let caps = spec
        .plan
        .to_capability_set(&spec.project_root)
        .map_err(|e| {
            step(
                "build the Landlock ruleset",
                io::Error::other(e.to_string()),
            )
        })?;
    nono::Sandbox::apply_auto(&caps).map_err(|e| {
        step(
            "apply the Landlock ruleset",
            io::Error::other(e.to_string()),
        )
    })?;

    // Blocked before the command exists, so no SIGCHLD can be missed.
    // std's `Command` clears the mask again in the child.
    // SAFETY: a valid, initialised signal set.
    unsafe { libc::sigprocmask(libc::SIG_BLOCK, signals, std::ptr::null_mut()) };

    let mut cmd = std::process::Command::new(&spec.command[0]);
    cmd.args(&spec.command[1..]).env_clear().envs(env);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    // SAFETY: `prctl` only, in the forked child of this single-threaded
    // process. With NOROOT set, uid 0 gains no capabilities across the
    // exec, so the command starts with none; LOCKED keeps it from undoing
    // that.
    unsafe {
        cmd.pre_exec(|| {
            let bits = SECBIT_NOROOT | SECBIT_NOROOT_LOCKED;
            if libc::prctl(libc::PR_SET_SECUREBITS, bits, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd
        .spawn()
        .map_err(|e| step(&format!("start {:?}", spec.command[0]), e))?;
    // The command holds the inherited fds now. PID 1 keeping a listening
    // socket would leave it open after the command that serves it exits.
    for n in 0..inherited {
        // SAFETY: closing fds `spawn` placed for the command, unused here.
        unsafe { libc::close(FIRST_INHERITED_FD + n as RawFd) };
    }
    if spec.relay_port.is_some() {
        // SAFETY: as above; the relay is the command's now.
        unsafe { libc::close(relay_fd) };
    }
    Ok(Started {
        main: child.id() as libc::pid_t,
        // SAFETY: `spawn` placed this fd for PID 1 alone.
        report: report.then(|| unsafe { std::fs::File::from_raw_fd(report_fd) }),
        timeout_secs: spec.timeout_secs,
    })
}

fn mount_fresh_proc() -> io::Result<()> {
    // SAFETY: static NUL-terminated strings and a null data pointer.
    let r = unsafe {
        libc::mount(
            c"proc".as_ptr(),
            c"/proc".as_ptr(),
            c"proc".as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
            std::ptr::null(),
        )
    };
    if r != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// SIGCHLD plus the termination signals PID 1 forwards to the command. PID
/// 1 ignores any signal it has no handler for, so without forwarding a
/// supervisor's SIGTERM would do nothing.
fn forwarded_signals() -> libc::sigset_t {
    // SAFETY: `sigemptyset` initialises the set before any `sigaddset`.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for s in [
            libc::SIGCHLD,
            libc::SIGTERM,
            libc::SIGINT,
            libc::SIGHUP,
            libc::SIGQUIT,
        ] {
            libc::sigaddset(&mut set, s);
        }
        set
    }
}

/// Reap everything, forward termination signals to `main`, and return its
/// exit code once it exits (128 + the signal, if one killed it), with
/// whether `timeout` ran out first.
///
/// At the deadline `main` gets SIGTERM, as from a `stop`, so the keeper
/// drains its sessions. If it is still there `GRACE_PERIOD` later, PID 1
/// returns anyway: its exit kills every process in the namespace, which is
/// the SIGKILL, and the report says 128 + 9.
fn supervise(
    main: libc::pid_t,
    signals: &libc::sigset_t,
    timeout: Option<std::time::Duration>,
) -> (i32, bool) {
    let began = std::time::Instant::now();
    let mut timed_out = false;
    loop {
        let wait = timeout.map(|t| {
            let until = if timed_out {
                t + crate::lifecycle::GRACE_PERIOD
            } else {
                t
            };
            until.saturating_sub(began.elapsed())
        });
        let sig = match wait {
            // SAFETY: a valid set; the info pointer may be null.
            None => unsafe { libc::sigwaitinfo(signals, std::ptr::null_mut()) },
            Some(left) => {
                let ts = libc::timespec {
                    tv_sec: left.as_secs() as libc::time_t,
                    tv_nsec: left.subsec_nanos() as libc::c_long,
                };
                // SAFETY: a valid set and timespec; the info pointer may be
                // null.
                unsafe { libc::sigtimedwait(signals, std::ptr::null_mut(), &ts) }
            }
        };
        if sig < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::EAGAIN) {
                if timed_out {
                    return (128 + libc::SIGKILL, true);
                }
                timed_out = true;
                // SAFETY: signalling our own child.
                unsafe { libc::kill(main, libc::SIGTERM) };
            }
            continue;
        }
        if sig != libc::SIGCHLD {
            // SAFETY: signalling our own child.
            unsafe { libc::kill(main, sig) };
            continue;
        }
        loop {
            let mut status = 0;
            // SAFETY: reaping any child; `status` is a valid out-pointer.
            let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
            if pid <= 0 {
                break;
            }
            if pid == main {
                let status = ExitStatus::from_raw(status);
                let code = status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
                return (code, timed_out);
            }
        }
    }
}

fn step(what: &str, e: io::Error) -> io::Error {
    io::Error::new(e.kind(), format!("{what}: {e}"))
}
