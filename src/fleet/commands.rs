//! What `devcroft fleet up | ls | stop` do, as library calls the CLI stays
//! thin over, and that tests can drive with an injected provider.
//!
//! **Each agent works on its own clone of the project** (the
//! `workspace-isolation` spec), at `<project>/.devcroft/fleet/<id>`: inside
//! the project's artifact directory, which `init` already ignores. Not in
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
}

#[derive(Serialize, Deserialize)]
struct FleetFile {
    cgroup_root: PathBuf,
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

    std::fs::create_dir_all(req.state_dir).map_err(backend)?;
    let fleet_file = serde_json::to_vec_pretty(&FleetFile {
        cgroup_root: req.cgroup_root.to_path_buf(),
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
            let dir = clone_root(req.project_root, id);
            made_clone = Some(dir.clone());
            let ws = clone_workspace(req.project_root, &dir)?;
            workspace = Some(ws.clone());
            prepare(
                provider,
                req.manifest,
                &ws,
                req.exe,
                req.authorized_key_pem,
                req.limits.clone(),
                req.view,
            )
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
    // Only what was asked for: a host without io.weight is worth saying
    // so to someone who set an IO weight, and noise to anyone else.
    let degraded = sup
        .degraded()
        .into_iter()
        .filter(|d| match d {
            super::cgroup::Degraded::IoWeight => req.limits.io_weight.is_some(),
        })
        .collect();
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
        };
        let devnull: std::os::fd::OwnedFd = std::fs::File::create("/dev/null")
            .map_err(|e| e.to_string())?
            .into();
        let stdio = Stdio {
            stdout: Some(devnull),
            stderr: None,
            inherit: Vec::new(),
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

/// Every agent the fleet has recorded, reconciled against the kernel.
pub fn ls(
    state_dir: &Path,
    manifest: &Manifest,
    exe: &Path,
) -> Result<Vec<AgentStatus>, FleetError> {
    open_existing(state_dir, manifest, exe)?
        .list()
        .map_err(backend)
}

/// Stop one agent. Its workspace stays.
pub fn stop(state_dir: &Path, manifest: &Manifest, exe: &Path, id: &str) -> Result<(), FleetError> {
    open_existing(state_dir, manifest, exe)?
        .stop(id)
        .map_err(not_found_is_config)
}

/// One agent, reconciled.
pub fn inspect(
    state_dir: &Path,
    manifest: &Manifest,
    exe: &Path,
    id: &str,
) -> Result<AgentStatus, FleetError> {
    open_existing(state_dir, manifest, exe)?
        .inspect(id)
        .map_err(not_found_is_config)
}

/// Remove a stopped agent: its record and its clone. The clone is removed
/// only where `up` makes clones, so a record naming any other path cannot
/// make this delete it.
pub fn rm(
    state_dir: &Path,
    manifest: &Manifest,
    exe: &Path,
    project_root: &Path,
    id: &str,
) -> Result<(), FleetError> {
    let mut sup = open_existing(state_dir, manifest, exe)?;
    let record = sup.remove(id).map_err(not_found_is_config)?;
    remove_clone(project_root, &record.id, &record.workspace)
}

/// Stop and remove every agent, their clones, the fleet's cgroup node and
/// its state.
pub fn rm_all(
    state_dir: &Path,
    manifest: &Manifest,
    exe: &Path,
    project_root: &Path,
) -> Result<Vec<String>, FleetError> {
    let mut sup = open_existing(state_dir, manifest, exe)?;
    let mut removed = Vec::new();
    for status in sup.list().map_err(backend)? {
        let id = status.record.id;
        if status.record.state == super::supervisor::AgentState::Running {
            sup.stop(&id).map_err(backend)?;
        }
        let record = sup.remove(&id).map_err(backend)?;
        remove_clone(project_root, &record.id, &record.workspace)?;
        removed.push(id);
    }
    sup.remove_node().map_err(backend)?;
    std::fs::remove_dir_all(state_dir).map_err(backend)?;
    // The clones' parent, and the artifact dir above it, only if nothing
    // else is in them: `up` keeps its own artifacts in `.devcroft/` too.
    let fleet_dir = clone_root(project_root, "x");
    if let Some(fleet_dir) = fleet_dir.parent() {
        let _ = std::fs::remove_dir(fleet_dir);
        if let Some(artifacts) = fleet_dir.parent() {
            let _ = std::fs::remove_dir(artifacts);
        }
    }
    Ok(removed)
}

fn clone_root(project_root: &Path, id: &str) -> PathBuf {
    project_root
        .join(crate::services::ARTIFACT_DIR)
        .join("fleet")
        .join(id)
}

fn remove_clone(project_root: &Path, id: &str, workspace: &Path) -> Result<(), FleetError> {
    let root = clone_root(project_root, id);
    if !workspace.starts_with(&root) {
        return Err(FleetError::Config(format!(
            "agent {id}'s workspace {} is not the clone fleet made at {}; left in place",
            workspace.display(),
            root.display()
        )));
    }
    match std::fs::remove_dir_all(&root) {
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

fn open_existing(
    state_dir: &Path,
    manifest: &Manifest,
    exe: &Path,
) -> Result<Supervisor, FleetError> {
    let bytes = match std::fs::read(state_dir.join("fleet.json")) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(FleetError::Config(format!(
                "no fleet for sandbox '{}'; start one with `devcroft fleet up --agents N`",
                manifest.sandbox.name
            )));
        }
        Err(e) => return Err(backend(e)),
    };
    let file: FleetFile =
        serde_json::from_slice(&bytes).map_err(|e| FleetError::Backend(e.to_string()))?;
    Supervisor::open(state_dir, &file.cgroup_root, &node_name(manifest), exe).map_err(backend)
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
    let top = PathBuf::from(String::from_utf8_lossy(&top.stdout).trim());
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
