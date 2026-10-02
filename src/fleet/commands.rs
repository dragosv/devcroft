//! What `devcroft fleet up | ls | stop` do, as library calls the CLI stays
//! thin over, and that tests can drive with an injected provider.
//!
//! **Each agent works on its own clone of the project** (the
//! `workspace-isolation` spec), at `<repo>/.devcroft/fleet/<id>`: in the
//! repository's artifact directory, made self-ignoring. Not in
//! devcroft's data dir, where the fleet's state lives, because the baseline
//! denies that dir to every sandbox. A `git clone --local` hardlinks the
//! object store, so a clone costs its checkout, not the history. Only
//! committed content is carried, which is what makes each agent's work a
//! reviewable diff against a known commit. Workspaces outlive their agents:
//! stopping an agent must not throw away what it did.
//!
//! This is the minimal form of D7. The shared bare mirror, GC control and
//! upstream-remote handling are still group 4's.
//!
//! **The cgroup root is given, not discovered.** Finding the delegated
//! subtree is systemd's half (D6), which needs a VM. `up` records the root
//! in `fleet.json`, so `ls` and `stop` do not need it again.
//!
//! **A fleet belongs to one project, and one command at a time.** State is
//! keyed by sandbox name, which two checkouts of one repository share, so
//! `fleet.json` records the project too, and every command refuses a fleet
//! recorded for another one, as `up` refuses a sandbox recorded for another
//! root. And every command holds the fleet's lock (`FleetLock`) for as
//! long as it reads or changes agent state: without it, two `up`s computed
//! the same next ID, and an `ls` during an `up` took the half-started
//! agent's directory for a crashed start and removed it.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use super::cgroup::Limits;
use super::project::{PrepareError, prepare};
use super::supervisor::{AgentStatus, Supervisor};
use crate::config::Manifest;
use crate::provider::ProviderEntry;

/// A fleet command's failure, by the error contract's layers.
#[derive(Debug)]
pub enum FleetError {
    /// Usage or manifest: exit 2.
    Config(String),
    /// The environment could not be resolved: exit 3.
    Provider(String),
    /// Namespaces, cgroups or the sandbox itself: exit 4.
    Backend(String),
}

impl FleetError {
    /// The exit code the error contract assigns this layer.
    pub fn exit_code(&self) -> i32 {
        match self {
            FleetError::Config(_) => 2,
            FleetError::Provider(_) => 3,
            FleetError::Backend(_) => 4,
        }
    }
}

impl std::fmt::Display for FleetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FleetError::Config(m) => write!(f, "config: {m}"),
            FleetError::Provider(m) => write!(f, "provider: {m}"),
            FleetError::Backend(m) => write!(f, "backend: {m}"),
        }
    }
}

impl std::error::Error for FleetError {}

impl From<PrepareError> for FleetError {
    fn from(e: PrepareError) -> Self {
        match e {
            PrepareError::Config(m) => FleetError::Config(m),
            PrepareError::Provider(e) => FleetError::Provider(e.to_string()),
        }
    }
}

fn backend(e: io::Error) -> FleetError {
    FleetError::Backend(e.to_string())
}

/// What `fleet up` needs besides the provider.
pub struct UpRequest<'a> {
    pub manifest: &'a Manifest,
    pub project_root: &'a Path,
    /// The delegated cgroup v2 subtree the fleet's node goes under.
    pub cgroup_root: &'a Path,
    pub agents: usize,
    /// `false` keeps the host root (D2a's other strategy).
    pub view: bool,
    /// The devcroft binary each agent runs as its keeper.
    pub exe: &'a Path,
    /// The client public key every agent's SSH server accepts.
    pub authorized_key_pem: &'a str,
    /// Where the fleet's state lives; `lifecycle::fleet_state_dir` in the
    /// CLI.
    pub state_dir: &'a Path,
    /// Every agent's cgroup limits.
    pub limits: Limits,
    /// Every agent's wall-clock limit, from its start; `None` for none.
    pub timeout: Option<std::time::Duration>,
}

/// What `up` did: the agents it started, and the limits it was asked for
/// that this host cannot apply (reported, never dropped quietly).
#[derive(Debug)]
pub struct UpOutcome {
    pub started: Vec<Started>,
    pub degraded: Vec<super::cgroup::Degraded>,
}

/// One agent `up` started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub id: String,
    pub workspace: PathBuf,
    /// How its services came up; `up` returns only once each agent's are
    /// ready, have failed, or have run out of time.
    pub services: ServicesOutcome,
}

/// Where an agent's services stood when `up` stopped waiting for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServicesOutcome {
    /// It declares none.
    None,
    /// Every service is running and past its readiness probe, or is a
    /// one-shot job that finished cleanly.
    Ready,
    /// These failed (or were skipped because a dependency failed). The
    /// agent stays up, as a sandbox does: a failed service must not take
    /// the agent's sessions with it.
    Failed(Vec<(String, String)>),
    /// Not ready within [`READY_BUDGET`]; each service's state.
    NotReady(Vec<(String, String)>),
}

/// How long `up` waits for an agent's services to be ready. Generous: a
/// database initialising its data directory on first start is the case
/// this is for.
pub const READY_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

/// How long a service without a readiness probe must stay up before `up`
/// calls it ready: long enough to catch one that dies at start, which is
/// the most common way a service fails.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Serialize, Deserialize)]
struct FleetFile {
    cgroup_root: PathBuf,
    /// The project the fleet was started for, canonical. Absent in a
    /// `fleet.json` written before it was recorded, which the next `up`
    /// fills in.
    #[serde(default)]
    project_root: Option<PathBuf>,
}

/// A fleet's lock, held for a command's whole critical section. `flock`,
/// like `lifecycle::acquire_lifecycle_lock` and for its reason: the kernel
/// releases it however the holder dies, so a crashed command never leaves
/// a fleet locked. **Beside the state directory, not in it**, because
/// `rm --all` removes that directory while holding the lock.
struct FleetLock(#[allow(dead_code)] std::fs::File);

fn lock(state_dir: &Path) -> Result<FleetLock, FleetError> {
    use std::os::fd::AsRawFd;
    let name = state_dir
        .file_name()
        .ok_or_else(|| FleetError::Backend(format!("{} has no name", state_dir.display())))?;
    let path = state_dir.with_file_name(format!("{}.lock", name.to_string_lossy()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(backend)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(backend)?;
    let flock = |op| {
        // SAFETY: `file` owns a valid fd for the duration of this call.
        if unsafe { libc::flock(file.as_raw_fd(), op) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    };
    match flock(libc::LOCK_EX | libc::LOCK_NB) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
            // Said, because the wait can be long: an `up` holds the lock
            // while it resolves each agent's environment.
            eprintln!("devcroft fleet: waiting for another fleet command on this fleet to finish");
            flock(libc::LOCK_EX).map_err(backend)?;
        }
        Err(e) => return Err(backend(e)),
    }
    Ok(FleetLock(file))
}

/// `project_root`, canonical where it can be, as the fleet records it.
fn canonical(project_root: &Path) -> PathBuf {
    project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf())
}

/// Refuse a fleet recorded for another project: acting on it would stop,
/// list or remove another checkout's agents, and `up` would add clones of
/// this one to them.
fn ensure_same_project(
    file: &FleetFile,
    manifest: &Manifest,
    project_root: &Path,
) -> Result<(), FleetError> {
    match &file.project_root {
        Some(recorded) if *recorded != canonical(project_root) => Err(FleetError::Config(format!(
            "the fleet for sandbox '{name}' belongs to a different project\n  \
             recorded: {recorded}\n   current: {current}\n\
             two checkouts sharing a `[sandbox].name` share a fleet; remove it from the \
             recorded project with `devcroft fleet rm --all`, or give this one its own \
             `[sandbox].name`",
            name = manifest.sandbox.name,
            recorded = recorded.display(),
            current = canonical(project_root).display(),
        ))),
        _ => Ok(()),
    }
}

fn read_fleet_file(state_dir: &Path) -> Result<Option<FleetFile>, FleetError> {
    match std::fs::read(state_dir.join("fleet.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| FleetError::Backend(format!("fleet.json: {e}"))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(backend(e)),
    }
}

/// The fleet node's name under the cgroup root, one per sandbox name.
fn node_name(manifest: &Manifest) -> String {
    format!("fleet-{}", manifest.sandbox.name)
}

/// Start `agents` agents on fresh clones of the project. Stops at the first
/// failure; agents already started keep running and are listed in it.
pub fn up(provider: &dyn ProviderEntry, req: &UpRequest) -> Result<UpOutcome, FleetError> {
    if req.agents == 0 {
        return Err(FleetError::Config("--agents must be at least 1".into()));
    }
    require_git_repository(req.project_root)?;
    preflight(req.cgroup_root, req.exe)?;

    let lock = lock(req.state_dir)?;
    // Checked before anything is written: the first version overwrote
    // `fleet.json` first, so a second checkout's `up` moved the fleet to
    // its own cgroup root and added its clones to the first one's agents.
    if let Some(existing) = read_fleet_file(req.state_dir)? {
        ensure_same_project(&existing, req.manifest, req.project_root)?;
        if existing.cgroup_root != req.cgroup_root {
            return Err(FleetError::Config(format!(
                "the fleet for sandbox '{}' runs under cgroup root {}, not {}; its agents' \
                 leaves are there. Pass that root, or remove the fleet with \
                 `devcroft fleet rm --all` first",
                req.manifest.sandbox.name,
                existing.cgroup_root.display(),
                req.cgroup_root.display(),
            )));
        }
    }
    std::fs::create_dir_all(req.state_dir).map_err(backend)?;
    let fleet_file = serde_json::to_vec_pretty(&FleetFile {
        cgroup_root: req.cgroup_root.to_path_buf(),
        project_root: Some(canonical(req.project_root)),
    })
    .map_err(|e| FleetError::Backend(e.to_string()))?;
    std::fs::write(req.state_dir.join("fleet.json"), fleet_file).map_err(backend)?;

    let mut sup = Supervisor::open(
        req.state_dir,
        req.cgroup_root,
        &node_name(req.manifest),
        req.exe,
    )
    .map_err(backend)?;

    let mut started = Vec::new();
    for _ in 0..req.agents {
        let mut made_clone = None;
        let mut workspace = None;
        let mut refused = None;
        let result = sup.start_with(|id| {
            let dir = clone_root(req.project_root, id)?;
            ignore_clones(req.project_root)?;
            made_clone = Some(dir.clone());
            let ws = clone_workspace(req.project_root, &dir)?;
            workspace = Some(ws.clone());
            prepare(
                provider,
                req.manifest,
                &ws,
                id,
                req.exe,
                req.authorized_key_pem,
                req.limits.clone(),
                req.view,
            )
            // `prepare` resolves what the manifest says; the deadline is the run's.
            .map(|mut launch| {
                launch.timeout = req.timeout;
                launch
            })
            .map_err(|e| {
                let msg = e.to_string();
                refused = Some(e);
                io::Error::other(msg)
            })
        });
        match result {
            Ok(id) => started.push(Started {
                id,
                workspace: workspace.expect("set before a successful start"),
                services: ServicesOutcome::None,
            }),
            Err(e) => {
                // The clone was ours; the supervisor removed its own part.
                if let Some(dir) = made_clone {
                    let _ = std::fs::remove_dir_all(dir);
                }
                return Err(match refused {
                    Some(prepare_error) => prepare_error.into(),
                    None => backend(e),
                });
            }
        }
    }
    let mut declared = Vec::new();
    for agent in &started {
        declared.push(sup.inspect(&agent.id).map_err(backend)?.record.services);
    }
    // Only what was asked for: a host without io.weight is worth saying
    // so to someone who set an IO weight, and noise to anyone else.
    let degraded = sup
        .degraded()
        .into_iter()
        .filter(|d| match d {
            super::cgroup::Degraded::IoWeight => req.limits.io_weight.is_some(),
        })
        .collect();
    // Every agent is recorded, so the fleet is consistent again: waiting for
    // services reads no fleet state, and must not keep `ls` waiting for up
    // to `READY_BUDGET`.
    drop(sup);
    drop(lock);

    // Readiness is waited for after every agent has started, so N agents'
    // databases initialise at once rather than one after another.
    let deadline = std::time::Instant::now() + READY_BUDGET;
    for (agent, declared) in started.iter_mut().zip(&declared) {
        agent.services = wait_ready(&agent.workspace, &agent.id, declared, deadline);
    }
    Ok(UpOutcome { started, degraded })
}

/// Check the host can run an agent at all, before cloning anything
/// (`agent-supervisor`: *Preflight environment validation*).
///
/// **By running one**, not by reading sysctls or a kernel version: a
/// container's seccomp profile, an AppArmor rule on unprivileged user
/// namespaces, `max_user_namespaces`, Docker's `/proc` masking and a
/// missing Landlock each refuse independently, and no readable value
/// predicts all of them (`fleet::netns::probe` follows the same rule). The
/// probe is a real agent running `devcroft --version` on the host root, in
/// its own throwaway node. The helper already names the step that failed;
/// this adds what to do about it.
pub fn preflight(cgroup_root: &Path, exe: &Path) -> Result<(), FleetError> {
    use super::cgroup::FleetNode;
    use super::init::{self, AgentSpec, Stdio};

    // Unique per call, not just per process: two `up`s in one process (a
    // library caller, or parallel tests) would otherwise share the probe
    // leaf and fail each other with EEXIST.
    static PROBES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = PROBES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let node = FleetNode::create(
        cgroup_root,
        &format!("fleet-preflight-{}-{n}", std::process::id()),
    )
    .map_err(|e| FleetError::Backend(format!("preflight: cgroup delegation: {e}")))?;
    let result = (|| {
        let leaf = node
            .create_leaf("probe", &Limits::default())
            .map_err(|e| format!("cgroup delegation: {e}"))?;
        let (manifest, _) =
            crate::config::parse("[sandbox]\nname = \"preflight\"\n\n[env]\nprovider = \"nix\"\n")
                .map_err(|e| e.to_string())?;
        let exe_dir = exe.parent().unwrap_or(exe).to_string_lossy().into_owned();
        let plan = crate::policy::compile(&manifest)
            .with_keeper_exe_grant(exe_dir)
            .to_capability_plan();
        let spec = AgentSpec {
            command: vec![exe.to_string_lossy().into_owned(), "--version".into()],
            env: Vec::new(),
            cwd: None,
            hostname: "preflight".into(),
            plan,
            project_root: std::env::temp_dir(),
            relay_port: None,
            view: None,
            timeout_secs: None,
        };
        let devnull: std::os::fd::OwnedFd = std::fs::File::create("/dev/null")
            .map_err(|e| e.to_string())?
            .into();
        let stdio = Stdio {
            stdout: Some(devnull),
            stderr: None,
            inherit: Vec::new(),
            lifeline: None,
            report: None,
        };
        let outcome = init::spawn(exe, &leaf, &spec, stdio)
            .map_err(|e| remedy(&e.to_string()))
            .and_then(|agent| match agent.wait() {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(format!("the probe agent exited with {status}")),
                Err(e) => Err(e.to_string()),
            });
        let _ = leaf.kill();
        outcome
    })();
    let _ = node.remove();
    result.map_err(|e| FleetError::Backend(format!("preflight: {e}")))
}

/// What to do about a failed agent step, keyed on the step the helper
/// names (or on `clone3` itself, which fails before there is a helper).
fn remedy(error: &str) -> String {
    let fix = if error.contains("clone3") {
        "unprivileged user namespaces are unavailable: check \
         kernel.unprivileged_userns_clone, user.max_user_namespaces, an AppArmor \
         restriction on unprivileged user namespaces (Ubuntu 23.10+), or a \
         container seccomp profile"
    } else if error.contains("mount a fresh /proc") || error.contains("mounting /proc") {
        "this host masks /proc (Docker's default), so an agent's PID namespace \
         cannot have its own; run the container with \
         `--security-opt systempaths=unconfined`, or run on a host or VM"
    } else if error.contains("Landlock") {
        "Landlock is unavailable: it needs Linux 5.13+ with `landlock` in the \
         LSM list (`cat /sys/kernel/security/lsm`)"
    } else {
        return error.to_owned();
    };
    format!("{error}\n  {fix}")
}

/// Poll an agent's service supervisor until its services are ready, one has
/// failed, or `deadline` passes (`service-ports`: *Agent readiness*).
fn wait_ready(
    workspace: &Path,
    id: &str,
    declared: &[String],
    deadline: std::time::Instant,
) -> ServicesOutcome {
    use crate::services::ServiceHealth;
    if declared.is_empty() {
        return ServicesOutcome::None;
    }
    let socket = crate::services::socket_path(workspace, id);
    let mut settled_since: Option<std::time::Instant> = None;
    loop {
        let report = crate::services::reconcile(declared, crate::services::query(&socket));
        let label = |states: &[&crate::services::ServiceState]| {
            states
                .iter()
                .map(|s| (s.name.clone(), s.health.label()))
                .collect::<Vec<_>>()
        };
        let failed: Vec<_> = report
            .states
            .iter()
            .filter(|s| s.health.is_failure() || s.health == ServiceHealth::Skipped)
            .collect();
        if !failed.is_empty() {
            return ServicesOutcome::Failed(label(&failed));
        }
        let done =
            |s: &crate::services::ServiceState| s.is_ready() || s.health == ServiceHealth::Exited;
        if report.supervisor_error.is_none() && report.states.iter().all(done) {
            // A probe that passed is evidence. "Running" alone is not: a
            // service that dies at start is caught running first (measured,
            // `exit 3` read as `Running` on process-compose 1.116.0), and
            // was reported Ready. So a service without a probe must stay up
            // for `SETTLE` first, and a failure meanwhile is reported.
            if report.states.iter().all(|s| s.ready.is_some()) {
                return ServicesOutcome::Ready;
            }
            let now = std::time::Instant::now();
            if now.duration_since(*settled_since.get_or_insert(now)) >= SETTLE {
                return ServicesOutcome::Ready;
            }
        } else {
            settled_since = None;
        }
        if std::time::Instant::now() >= deadline {
            let waiting: Vec<_> = report.states.iter().filter(|s| !done(s)).collect();
            return ServicesOutcome::NotReady(label(&waiting));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Where a fleet is and whose it is: what every command after `up` needs.
pub struct FleetRef<'a> {
    /// `lifecycle::fleet_state_dir` in the CLI.
    pub state_dir: &'a Path,
    pub manifest: &'a Manifest,
    /// The project the command runs in, which must be the fleet's.
    pub project_root: &'a Path,
    /// The devcroft binary.
    pub exe: &'a Path,
}

/// Every agent the fleet has recorded, reconciled against the kernel.
pub fn ls(fleet: &FleetRef) -> Result<Vec<AgentStatus>, FleetError> {
    let (_lock, mut sup) = open_existing(fleet)?;
    sup.list().map_err(backend)
}

/// Stop one agent. Its workspace stays.
pub fn stop(fleet: &FleetRef, id: &str) -> Result<(), FleetError> {
    let (_lock, mut sup) = open_existing(fleet)?;
    sup.stop(id).map_err(not_found_is_config)
}

/// One agent, reconciled.
pub fn inspect(fleet: &FleetRef, id: &str) -> Result<AgentStatus, FleetError> {
    let (_lock, mut sup) = open_existing(fleet)?;
    sup.inspect(id).map_err(not_found_is_config)
}

/// Remove a stopped agent: its record and its clone. The clone is removed
/// only where `up` makes clones, so a record naming any other path cannot
/// make this delete it, and that is checked **before** the record goes: a
/// record removed first leaves a clone nothing names any more.
pub fn rm(fleet: &FleetRef, id: &str) -> Result<(), FleetError> {
    let (_lock, mut sup) = open_existing(fleet)?;
    let status = sup.inspect(id).map_err(not_found_is_config)?;
    let clone = own_clone(fleet.project_root, id, &status.record.workspace)?;
    sup.remove(id).map_err(not_found_is_config)?;
    remove_clone(&clone)
}

/// Stop and remove every agent, their clones, the fleet's cgroup node and
/// its state.
pub fn rm_all(fleet: &FleetRef) -> Result<Vec<String>, FleetError> {
    let (lock, mut sup) = open_existing(fleet)?;
    let project_root = fleet.project_root;
    let mut removed = Vec::new();
    for status in sup.list().map_err(backend)? {
        let id = status.record.id;
        let clone = own_clone(project_root, &id, &status.record.workspace)?;
        if status.record.state == super::supervisor::AgentState::Running {
            sup.stop(&id).map_err(backend)?;
        }
        sup.remove(&id).map_err(backend)?;
        remove_clone(&clone)?;
        removed.push(id);
    }
    sup.remove_node().map_err(backend)?;
    std::fs::remove_dir_all(fleet.state_dir).map_err(backend)?;
    // Released only now: a fleet removed while another command waits must
    // be gone, not half gone, when that command reads it.
    drop(lock);
    // The clones' parent, and the artifact dir above it, only if nothing
    // else is in them: `up` keeps its own artifacts in `.devcroft/` too.
    if let Ok(dir) = fleet_dir(project_root) {
        let only_ignore = std::fs::read_dir(&dir)
            .map(|entries| entries.flatten().all(|e| e.file_name() == ".gitignore"))
            .unwrap_or(false);
        if only_ignore {
            let _ = std::fs::remove_file(dir.join(".gitignore"));
        }
        let _ = std::fs::remove_dir(&dir);
        if let Some(artifacts) = dir.parent() {
            let _ = std::fs::remove_dir(artifacts);
        }
    }
    Ok(removed)
}

/// The repository holding `project_root`.
fn repo_top(project_root: &Path) -> io::Result<PathBuf> {
    let top = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !top.status.success() {
        return Err(io::Error::other(format!(
            "finding the repository of {}: {}",
            project_root.display(),
            String::from_utf8_lossy(&top.stderr).trim()
        )));
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&top.stdout).trim()))
}

/// Where the clones live: `<repo>/.devcroft/fleet`, at the **repository**
/// root rather than under the project. A project in a subdirectory (every
/// sample here) would otherwise have its relative path twice in each
/// workspace path, and a service supervisor's socket inside it came to 123
/// bytes against the OS's 103 (measured on `flox-services-sample`).
fn fleet_dir(project_root: &Path) -> io::Result<PathBuf> {
    Ok(repo_top(project_root)?
        .join(crate::services::ARTIFACT_DIR)
        .join("fleet"))
}

fn clone_root(project_root: &Path, id: &str) -> io::Result<PathBuf> {
    Ok(fleet_dir(project_root)?.join(id))
}

/// A `.gitignore` of `*` in the clones' directory: it ignores everything
/// there, itself included, so a repository whose own ignores do not cover
/// `.devcroft/` at its root does not list agents' clones as untracked.
fn ignore_clones(project_root: &Path) -> io::Result<()> {
    let dir = fleet_dir(project_root)?;
    std::fs::create_dir_all(&dir)?;
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(ignore, "*\n")?;
    }
    Ok(())
}

/// The clone `up` made for agent `id`, if `workspace` is inside it.
fn own_clone(project_root: &Path, id: &str, workspace: &Path) -> Result<PathBuf, FleetError> {
    let root = clone_root(project_root, id).map_err(backend)?;
    if !workspace.starts_with(&root) {
        return Err(FleetError::Config(format!(
            "agent {id}'s workspace {} is not the clone fleet made at {}; left in place",
            workspace.display(),
            root.display()
        )));
    }
    Ok(root)
}

fn remove_clone(root: &Path) -> Result<(), FleetError> {
    match std::fs::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(backend(e)),
    }
}

fn not_found_is_config(e: io::Error) -> FleetError {
    match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ResourceBusy => FleetError::Config(e.to_string()),
        _ => backend(e),
    }
}

/// The fleet's lock, then its supervisor, for a fleet that must exist and
/// be this project's. The lock is first in the tuple so it is the last
/// thing a caller's `let (_lock, sup)` drops.
fn open_existing(fleet: &FleetRef) -> Result<(FleetLock, Supervisor), FleetError> {
    let lock = lock(fleet.state_dir)?;
    let Some(file) = read_fleet_file(fleet.state_dir)? else {
        return Err(FleetError::Config(format!(
            "no fleet for sandbox '{}'; start one with `devcroft fleet up --agents N`",
            fleet.manifest.sandbox.name
        )));
    };
    ensure_same_project(&file, fleet.manifest, fleet.project_root)?;
    let sup = Supervisor::open(
        fleet.state_dir,
        &file.cgroup_root,
        &node_name(fleet.manifest),
        fleet.exe,
    )
    .map_err(backend)?;
    Ok((lock, sup))
}

fn require_git_repository(project_root: &Path) -> Result<(), FleetError> {
    let ok = Command::new("git")
        .args(["-C"])
        .arg(project_root)
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(FleetError::Config(format!(
            "{} is not a git repository with a commit; each agent works on its own \
             clone of the project, so fleet needs one",
            project_root.display()
        )))
    }
}

/// Clone the repository holding `project_root` into `dest`, and return
/// the project's own directory inside the clone.
///
/// **The project can be a subdirectory of its repository** (every sample
/// here is one). `git clone` always clones the whole repository, so the
/// agent's workspace is the same relative path inside the clone, not the
/// clone's root. Cloning the subdirectory's path and using `dest` as the
/// workspace would put the agent at the repository root, with the wrong
/// manifest or none.
fn clone_workspace(project_root: &Path, dest: &Path) -> io::Result<PathBuf> {
    let top = repo_top(project_root)?;
    let relative = project_root
        .canonicalize()?
        .strip_prefix(&top)
        .map(Path::to_path_buf)
        .map_err(|_| {
            io::Error::other(format!(
                "{} is not inside its repository {}",
                project_root.display(),
                top.display()
            ))
        })?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let out = Command::new("git")
        .args(["clone", "--local", "--quiet"])
        .arg(&top)
        .arg(dest)
        .output()?;
    if out.status.success() {
        Ok(dest.join(relative))
    } else {
        Err(io::Error::other(format!(
            "git clone into {}: {}",
            dest.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::remedy;

    #[test]
    fn each_failing_step_gets_its_own_remedy() {
        assert!(remedy("clone3 into /x: Operation not permitted").contains("user namespaces"));
        assert!(
            remedy("fleet init: mount a fresh /proc: EPERM").contains("systempaths=unconfined")
        );
        assert!(remedy("fleet init: apply the Landlock ruleset: x").contains("landlock"));
        // An error nothing recognises is passed through, not dressed up.
        assert_eq!(remedy("something else"), "something else");
    }
}
