//! `up` (task 4.2): design.md decision 1's supervisor sequence — create
//! the listener, resolve the environment, compile the policy, spawn the
//! keeper under nono with the listener fd inherited, wait for it to come
//! up. Idempotent by default; `--recreate` forces a full teardown and
//! re-resolution.

use std::fmt;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::Manifest;
use crate::policy;
use crate::provider::{FloxProvider, Provider, ProviderError};

use super::state::{self, Health, StatePaths};
use super::terminate::GRACE_PERIOD as TERMINATE_GRACE_PERIOD;

const KEEPER_START_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpOutcome {
    /// A healthy keeper already existed; nothing was done (spec: "Up on a
    /// healthy sandbox").
    AlreadyUp,
    /// State existed but the keeper was dead/unresponsive; it was cleared
    /// and a fresh keeper started (spec: "Recovery after host reboot").
    Recovered,
    /// No prior state; started clean.
    Started,
    /// `--recreate`: any existing keeper was torn down and everything was
    /// re-resolved and recompiled from scratch.
    Recreated,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UpOptions {
    pub recreate: bool,
}

#[derive(Debug)]
pub enum UpError {
    State(io::Error),
    Provider(ProviderError),
    Keeper(String),
    /// CLAUDE.md's error contract names `ssh` as its own layer; key
    /// generation/resolution failures (task 6.1) land here rather than
    /// `Keeper`, even though both currently surface through the same
    /// exit code (keeper/connection, 5) until task group 7 wires up a
    /// dedicated CLI.
    Ssh(String),
}

impl From<io::Error> for UpError {
    fn from(e: io::Error) -> Self {
        UpError::State(e)
    }
}

impl fmt::Display for UpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpError::State(e) => write!(f, "state: {e}"),
            UpError::Provider(e) => write!(f, "provider: {e}"),
            UpError::Keeper(msg) => write!(f, "keeper: {msg}"),
            UpError::Ssh(msg) => write!(f, "ssh: {msg}"),
        }
    }
}

impl std::error::Error for UpError {}

pub fn up(
    manifest: &Manifest,
    project_root: &Path,
    opts: &UpOptions,
) -> Result<UpOutcome, UpError> {
    let paths = StatePaths::new(&manifest.sandbox.name)?;

    let outcome = if opts.recreate {
        if let Health::Healthy(pid) | Health::Stale(pid) = state::health(&paths)? {
            state::terminate_and_wait(pid, TERMINATE_GRACE_PERIOD);
        }
        state::clear_runtime_state(&paths)?;
        UpOutcome::Recreated
    } else {
        match state::health(&paths)? {
            Health::Healthy(_) => return Ok(UpOutcome::AlreadyUp),
            Health::Stale(_) => {
                state::clear_runtime_state(&paths)?;
                UpOutcome::Recovered
            }
            Health::None => UpOutcome::Started,
        }
    };

    // ssh spec: "0700 state dir" — set on creation (mode only applies to
    // dirs `create_dir_all`/`DirBuilder` actually create, so an
    // already-existing root from before this existed is left alone, same
    // as `create_dir_all`'s own idempotency).
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&paths.root)?;

    // Host-side, before any restriction applies (design.md decision 2):
    // the resolved environment and its store grants are captured now,
    // once, and folded into the profile the keeper will be confined to.
    let resolution = FloxProvider
        .resolve(project_root)
        .map_err(UpError::Provider)?;

    // Recorded now so `status` (task 4.3) can later tell whether the
    // environment has drifted since this `up`, without needing the
    // manifest or project root passed back in — the keeper itself is
    // never told its own state dir, so it can't answer this either.
    let env_fingerprint =
        crate::provider::manifest_fingerprint(project_root).map_err(UpError::Provider)?;
    state::write_meta(
        &paths.meta,
        &state::Meta {
            project_root: project_root.to_string_lossy().into_owned(),
            env_fingerprint,
        },
    )?;

    let mut profile = policy::compile(manifest).to_nono_profile();
    for grant in &resolution.read_only_grants {
        profile.filesystem.read.push(grant.clone());
    }
    // The keeper binary itself must be readable+executable inside the
    // boundary it's about to apply to itself — the "default" baseline
    // (NONO_BASELINE_PROFILE) covers system paths but has no way to know
    // where *this build* of devcroft lives. Same requirement the task
    // 1.1/1.2 spike hit and solved the same way (`exe.parent()`).
    let exe = keeper_exe()?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| io::Error::other("devcroft executable path has no parent directory"))?;
    profile
        .filesystem
        .read
        .push(exe_dir.to_string_lossy().into_owned());
    std::fs::write(&paths.profile, profile.to_json())?;

    // Listener created BEFORE restriction (CLAUDE.md's listener-before-
    // restriction ordering, proven by the task 1.1/1.2 spike): its fd
    // survives the exec below, which is the only reason the socket stays
    // reachable once the keeper can no longer widen its own boundary.
    let _ = std::fs::remove_file(&paths.socket); // a stale file would fail bind()
    let listener = UnixListener::bind(&paths.socket)?;
    clear_cloexec(listener.as_raw_fd())?;

    // The ssh server's socket (ssh spec, task 6.1): same listener-before-
    // restriction reasoning as the control socket above, plus its own
    // mode-0600 requirement (belt-and-suspenders alongside the 0700 root
    // — either alone already blocks every other user).
    let _ = std::fs::remove_file(&paths.ssh_socket);
    let ssh_listener = UnixListener::bind(&paths.ssh_socket)?;
    std::fs::set_permissions(&paths.ssh_socket, std::fs::Permissions::from_mode(0o600))?;
    clear_cloexec(ssh_listener.as_raw_fd())?;

    // Both key materials are generated/resolved host-side because the
    // keeper cannot read either back off disk itself once sandboxed —
    // everything under `policy::DEVCROFT_DATA_DIR` (this whole state
    // dir included) is baseline-denied, even to the keeper's own
    // process. `spawn_keeper` hands both down as env vars instead (see
    // `ssh::start_from_env`).
    let (client_private_path, client_public_path) = state::client_key_paths()?;
    let client_key = crate::ssh::ensure_client_keypair(&client_private_path, &client_public_path)
        .map_err(|e| UpError::Ssh(e.to_string()))?;
    let host_key = crate::ssh::generate_host_key(&paths.ssh_host_key)
        .map_err(|e| UpError::Ssh(e.to_string()))?;
    let host_key_pem = host_key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .map_err(|e| UpError::Ssh(e.to_string()))?;
    let authorized_key_pem = client_key
        .public_key()
        .to_openssh()
        .map_err(|e| UpError::Ssh(e.to_string()))?;

    let keeper_pid = spawn_keeper(
        &exe,
        &listener,
        &paths,
        project_root,
        &resolution.env,
        SshHandoff {
            listener: &ssh_listener,
            host_key_pem: &host_key_pem,
            authorized_key_pem: &authorized_key_pem,
        },
    )
    .map_err(|e| UpError::Keeper(e.to_string()))?;
    // Both fds must outlive this function for the child to inherit them
    // across exec; ownership passes to the keeper process from here.
    std::mem::forget(listener);
    std::mem::forget(ssh_listener);

    state::write_pidfile(&paths.pidfile, keeper_pid)?;

    wait_until_responsive(&paths, KEEPER_START_TIMEOUT)
        .map_err(|e| UpError::Keeper(format!("keeper did not become responsive: {e}")))?;

    Ok(outcome)
}

/// `nono wrap -p <profile>` applies the compiled sandbox and execs into
/// the keeper binary — the profile MUST be passed via `-p`/`--profile`
/// (the named-profile schema, which supports `extends: "default"`), never
/// `-c`/`--config` (an unrelated, stricter "capability manifest" schema
/// requiring its own `version` field and providing no baseline system
/// access at all — confirmed against a live nono 0.71.0; see
/// `policy::NONO_BASELINE_PROFILE`).
/// The ssh server's fd and key material `spawn_keeper` hands the keeper,
/// bundled to keep that call under clippy's argument-count lint —  see
/// its call site for why the key material can't just be a file the
/// keeper reads back itself.
struct SshHandoff<'a> {
    listener: &'a UnixListener,
    host_key_pem: &'a str,
    authorized_key_pem: &'a str,
}

fn spawn_keeper(
    exe: &Path,
    listener: &UnixListener,
    paths: &StatePaths,
    project_root: &Path,
    env: &std::collections::BTreeMap<String, String>,
    ssh: SshHandoff,
) -> io::Result<libc::pid_t> {
    let log = std::fs::File::create(&paths.log)?;

    let mut cmd = Command::new("nono");
    cmd.arg("wrap")
        .arg("--silent")
        .arg("-p")
        .arg(&paths.profile)
        .arg("--")
        .arg(exe)
        .arg("__keeper")
        .arg(listener.as_raw_fd().to_string())
        .arg(ssh.listener.as_raw_fd().to_string())
        .current_dir(project_root)
        .envs(env)
        // ssh spec's key handoff (task 6.1): the keeper can't read either
        // key back off disk itself (see the call site's comment), so both
        // travel as env vars, same trust boundary the resolved provider
        // environment above already crosses this way.
        .env("DEVCROFT_SSH_HOST_KEY", ssh.host_key_pem)
        .env("DEVCROFT_SSH_AUTHORIZED_KEY", ssh.authorized_key_pem)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: setsid() only touches this (freshly forked, single-
    // threaded) child's own session/process-group state. Detaching from
    // the supervisor's controlling terminal is what lets the keeper
    // outlive `up`'s own process and the invoking shell.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn()?;
    let pid = child.id() as libc::pid_t;
    // Detach: `up` is a one-shot command, not the keeper's supervisor for
    // its whole lifetime. Not calling `.wait()` lets the keeper outlive
    // this process; once `up` exits, the OS reparents it to init, which
    // reaps it same as any other orphaned daemon.
    std::mem::forget(child);
    Ok(pid)
}

/// The binary to re-exec as the keeper. Normally `current_exe()`, but that
/// resolves to whatever process is *currently running* — inside a `cargo
/// test` unit test that's the libtest harness binary, not `devcroft`,
/// which would otherwise get `__keeper <fd>` handed to it as bogus test
/// filter arguments. `DEVCROFT_KEEPER_EXE` lets the integration test
/// (`tests/lifecycle_up.rs`, via `CARGO_BIN_EXE_devcroft`) point this at
/// the real built binary instead; production code never sets it.
fn keeper_exe() -> io::Result<PathBuf> {
    if let Ok(path) = std::env::var("DEVCROFT_KEEPER_EXE") {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe()
}

fn wait_until_responsive(paths: &StatePaths, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if UnixStream::connect(&paths.socket).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for the keeper's control socket to accept connections",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn clear_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
