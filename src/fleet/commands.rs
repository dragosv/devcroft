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
pub fn up(provider: &dyn ProviderEntry, req: &UpRequest) -> Result<Vec<Started>, FleetError> {
    if req.agents == 0 {
        return Err(FleetError::Config("--agents must be at least 1".into()));
    }
    require_git_repository(req.project_root)?;

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
        let mut clone_root = None;
        let mut workspace = None;
        let mut refused = None;
        let result = sup.start_with(|id| {
            let dir = req
                .project_root
                .join(crate::services::ARTIFACT_DIR)
                .join("fleet")
                .join(id);
            clone_root = Some(dir.clone());
            let ws = clone_workspace(req.project_root, &dir)?;
            workspace = Some(ws.clone());
            prepare(
                provider,
                req.manifest,
                &ws,
                req.exe,
                req.authorized_key_pem,
                Limits::default(),
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
                if let Some(dir) = clone_root {
                    let _ = std::fs::remove_dir_all(dir);
                }
                return Err(match refused {
                    Some(prepare_error) => prepare_error.into(),
                    None => backend(e),
                });
            }
        }
    }
    Ok(started)
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
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => FleetError::Config(e.to_string()),
            _ => backend(e),
        })
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
