//! The fleet supervisor: N agents, each started, listed and stopped by a
//! stable ID (`agent-supervisor` spec).
//!
//! It composes the other fleet modules and adds only what composing them
//! needs: IDs, a state directory per agent, and a durable record.
//!
//! **What is durable and what is not.** Each agent's `record.json` holds
//! its identity: ID, workspace, cgroup, lifecycle state, policy
//! fingerprint, port mappings and whether it needs attention. It holds no
//! pid. Pids are reused, which the single-sandbox lifecycle already had to
//! learn, so a recorded pid is exactly the stale identity the spec rules
//! out. **Liveness is the cgroup leaf instead**: a leaf with processes in
//! it is the agent's, because nothing but this supervisor creates leaves
//! under its fleet node, and membership survives reparenting, `setsid`
//! and a supervisor restart. So reconciling after a crash reads the
//! kernel's answer rather than trusting a record.
//!
//! **Start is all or nothing.** Sockets first (bound here, before any
//! restriction, 0600 in a 0700 directory), then the host key, the cgroup
//! leaf and the agent. A failure at any step removes what the earlier
//! steps made. A crash in the middle leaves a directory without a record,
//! and the next reconcile removes it, so no agent is ever half-started
//! with endpoints nobody serves.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::cgroup::{FleetNode, Limits, Teardown};
use super::init::{self, Agent, AgentSpec, Stdio, View};
use crate::policy::CapabilityPlan;

/// Everything needed to start one agent.
#[derive(Debug, Clone)]
pub struct AgentLaunch {
    /// The directory the agent works in: its project root, writable.
    pub workspace: PathBuf,
    /// The agent's compiled policy, including the keeper binary's grant.
    pub plan: CapabilityPlan,
    /// The provider's resolved environment.
    pub provider_env: BTreeMap<String, String>,
    /// Keys activation removed.
    pub unset: Vec<String>,
    /// The shell resolved from the closure (`crate::shell`).
    pub shell: PathBuf,
    pub hooks: Vec<(&'static str, String)>,
    /// The client public key every agent's SSH server accepts.
    pub authorized_key_pem: String,
    pub limits: Limits,
    /// Build a minimal root from the plan (D2a); `false` keeps the host's
    /// root, still under the same Landlock policy.
    pub view: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Running,
    Stopped,
}

/// A host port forwarded to a port inside one agent (`service-ports`).
/// Recorded from the start so the record's shape does not change when
/// mappings arrive; none are created yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMapping {
    pub host: u16,
    pub agent: u16,
}

/// An agent's durable identity (`record.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: String,
    pub workspace: PathBuf,
    pub cgroup: PathBuf,
    pub state: AgentState,
    /// FNV-1a of the compiled plan's JSON: whether two agents, or one
    /// agent before and after a change, run under the same policy.
    pub policy_fingerprint: String,
    /// Which mount-view strategy is in force (D2a says it is recorded
    /// and reported, never silently downgraded).
    pub view: bool,
    pub port_mappings: Vec<PortMapping>,
    /// Set when the agent is blocked on a question (`add-agent-interaction`).
    pub attention: bool,
}

/// A record plus what is true of it right now.
#[derive(Debug, Clone)]
pub struct AgentStatus {
    pub record: AgentRecord,
    /// `memory.current`, while running.
    pub memory_bytes: Option<u64>,
    /// `cpu.stat: usage_usec`, while running.
    pub cpu_usec: Option<u64>,
}

pub struct Supervisor {
    state: PathBuf,
    node: FleetNode,
    exe: PathBuf,
    /// Agents this process started, for reaping. An agent adopted after a
    /// restart has none: it is not this process's child, and its leaf is
    /// the handle.
    children: HashMap<String, Agent>,
}

impl Supervisor {
    /// Open (or create) a fleet: its state directory, and its node
    /// `fleet_name` under the delegated cgroup `cgroup_root`. `exe` is the
    /// devcroft binary the agents run as their keeper. Reconciles any
    /// agents a previous supervisor left.
    pub fn open(
        state: &Path,
        cgroup_root: &Path,
        fleet_name: &str,
        exe: &Path,
    ) -> io::Result<Supervisor> {
        private_dir(&state.join("agents"))?;
        let node = FleetNode::create(cgroup_root, fleet_name)?;
        let mut sup = Supervisor {
            state: state.to_path_buf(),
            node,
            exe: exe.to_path_buf(),
            children: HashMap::new(),
        };
        sup.reconcile()?;
        Ok(sup)
    }

    fn agents_dir(&self) -> PathBuf {
        self.state.join("agents")
    }

    fn agent_dir(&self, id: &str) -> PathBuf {
        self.agents_dir().join(id)
    }

    /// The agent's control socket, for the keeper protocol.
    pub fn control_socket(&self, id: &str) -> PathBuf {
        self.agent_dir(id).join("control.sock")
    }

    /// The agent's SSH socket (`devcroft proxy`'s target).
    pub fn ssh_socket(&self, id: &str) -> PathBuf {
        self.agent_dir(id).join("ssh.sock")
    }

    /// Start one agent. Returns its ID.
    pub fn start(&mut self, launch: &AgentLaunch) -> io::Result<String> {
        self.start_with(|_| Ok(launch.clone()))
    }

    /// Start one agent whose launch depends on its ID, such as a workspace
    /// named after it. `build` runs once the ID is allocated; if it fails,
    /// nothing is started and what the supervisor made is removed. What
    /// `build` itself made is the caller's to remove.
    pub fn start_with(
        &mut self,
        build: impl FnOnce(&str) -> io::Result<AgentLaunch>,
    ) -> io::Result<String> {
        let id = self.next_id()?;
        let dir = self.agent_dir(&id);
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        match build(&id).and_then(|launch| self.start_in(&id, &dir, &launch)) {
            Ok(()) => Ok(id),
            Err(e) => {
                if let Some(leaf) = self.node.existing_leaf(&id) {
                    let _ = leaf.kill();
                }
                let _ = std::fs::remove_dir_all(&dir);
                Err(io::Error::new(
                    e.kind(),
                    format!("starting agent {id}: {e}"),
                ))
            }
        }
    }

    fn start_in(&mut self, id: &str, dir: &Path, launch: &AgentLaunch) -> io::Result<()> {
        // Before anything is restricted: these stay reachable from outside
        // only because they predate the agent's restriction.
        let control = bind_private(&self.control_socket(id))?;
        let ssh = bind_private(&self.ssh_socket(id))?;

        // Ephemeral, per agent, like `up`'s per-sandbox host key.
        let host_key = crate::ssh::generate_host_key(&dir.join("ssh_host_key"))
            .and_then(|k| Ok(k.to_openssh(russh::keys::ssh_key::LineEnding::LF)?))
            .map_err(|e| io::Error::other(format!("ssh host key: {e}")))?;

        let home = crate::services::artifact_dir(&launch.workspace, id).join("home");
        std::fs::create_dir_all(&home)?;
        let env = crate::lifecycle::KeeperEnv {
            env: &launch.provider_env,
            unset: &launch.unset,
            plan: &launch.plan,
            ssh_host_key_pem: &host_key,
            ssh_authorized_key_pem: &launch.authorized_key_pem,
            shell: &launch.shell,
            services: None,
            hooks: &launch.hooks,
            project_root: &launch.workspace,
            sandbox_home: &home,
            relay: None,
        }
        .vars()
        .into_iter()
        .map(|(k, v)| Ok((lossless(k)?, lossless(v)?)))
        .collect::<io::Result<Vec<_>>>()?;

        let view = if launch.view {
            let root = dir.join("root");
            std::fs::create_dir(&root)?;
            Some(View {
                root,
                proxy_socket: None,
            })
        } else {
            None
        };

        let first = init::FIRST_INHERITED_FD;
        let spec = AgentSpec {
            command: vec![
                self.exe.to_string_lossy().into_owned(),
                "__keeper".into(),
                first.to_string(),
                (first + 1).to_string(),
            ],
            env,
            cwd: Some(launch.workspace.clone()),
            hostname: id.to_owned(),
            plan: launch.plan.clone(),
            project_root: launch.workspace.clone(),
            view,
        };
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("keeper.log"))?;
        let stdio = Stdio {
            stdout: Some(log.try_clone()?.into()),
            stderr: Some(log.into()),
            inherit: vec![control.into(), ssh.into()],
        };

        let leaf = self.node.create_leaf(id, &launch.limits)?;
        let agent = init::spawn(&self.exe, &leaf, &spec, stdio)?;
        self.children.insert(id.to_owned(), agent);

        let plan_json = serde_json::to_vec(&launch.plan).map_err(io::Error::other)?;
        write_record(
            dir,
            &AgentRecord {
                id: id.to_owned(),
                workspace: launch.workspace.clone(),
                cgroup: leaf.path().to_path_buf(),
                state: AgentState::Running,
                policy_fingerprint: format!("{:016x}", fnv1a64(&plan_json)),
                view: launch.view,
                port_mappings: Vec::new(),
                attention: false,
            },
        )
    }

    /// Every recorded agent, reconciled against the kernel first, in ID
    /// order.
    pub fn list(&mut self) -> io::Result<Vec<AgentStatus>> {
        self.reconcile()?;
        let mut out = Vec::new();
        for record in self.records()? {
            let leaf = (record.state == AgentState::Running)
                .then(|| self.node.existing_leaf(&record.id))
                .flatten();
            out.push(AgentStatus {
                memory_bytes: leaf.as_ref().and_then(|l| l.memory_current().ok()),
                cpu_usec: leaf.as_ref().and_then(|l| l.cpu_usage_usec().ok()),
                record,
            });
        }
        Ok(out)
    }

    /// Stop one agent: everything in its leaf, however detached, and only
    /// that. Its record stays, as Stopped.
    pub fn stop(&mut self, id: &str) -> io::Result<()> {
        let mut record = read_record(&self.agent_dir(id))?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no agent {id}")))?;
        if let Some(leaf) = self.node.existing_leaf(id)
            && let Teardown::LeftBehind(path) = leaf.kill()?
        {
            return Err(io::Error::other(format!(
                "agent {id}: processes remain in {} after cgroup.kill; left in place",
                path.display()
            )));
        }
        if let Some(agent) = self.children.remove(id) {
            reap(id, &agent)?;
        }
        record.state = AgentState::Stopped;
        self.release(&record)
    }

    /// Make the records match what is alive. A Running agent whose leaf is
    /// empty or gone is Stopped, and its resources are released. A
    /// directory without a record is a start that never finished.
    fn reconcile(&mut self) -> io::Result<()> {
        // Reap our own children that exited, so they are not left as
        // zombies and their leaves read empty.
        self.children
            .retain(|_, agent| matches!(agent.try_wait(), Ok(None)));

        for entry in std::fs::read_dir(self.agents_dir())? {
            let dir = entry?.path();
            let Some(id) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                continue;
            };
            match read_record(&dir)? {
                None => {
                    if let Some(leaf) = self.node.existing_leaf(&id) {
                        let _ = leaf.kill();
                    }
                    std::fs::remove_dir_all(&dir)?;
                }
                Some(mut record) if record.state == AgentState::Running => {
                    let alive = self
                        .node
                        .existing_leaf(&id)
                        .map(|leaf| leaf.populated())
                        .transpose()?
                        .unwrap_or(false);
                    if !alive {
                        if let Some(leaf) = self.node.existing_leaf(&id) {
                            leaf.kill()?;
                        }
                        record.state = AgentState::Stopped;
                        self.release(&record)?;
                    }
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Record a Stopped agent and release what only a running one needs:
    /// its sockets and its view root. Port mappings go with them.
    fn release(&self, record: &AgentRecord) -> io::Result<()> {
        let dir = self.agent_dir(&record.id);
        let _ = std::fs::remove_file(self.control_socket(&record.id));
        let _ = std::fs::remove_file(self.ssh_socket(&record.id));
        let _ = std::fs::remove_dir(dir.join("root"));
        let mut record = record.clone();
        record.port_mappings.clear();
        write_record(&dir, &record)
    }

    fn records(&self) -> io::Result<Vec<AgentRecord>> {
        let mut records = Vec::new();
        for entry in std::fs::read_dir(self.agents_dir())? {
            if let Some(r) = read_record(&entry?.path())? {
                records.push(r);
            }
        }
        records.sort_by_key(|r| id_number(&r.id));
        Ok(records)
    }

    /// `a<N>`, one past the highest ever used: an ID is never reused, so
    /// a stopped agent's record cannot be mistaken for a new agent's.
    fn next_id(&self) -> io::Result<String> {
        let mut max = 0;
        for entry in std::fs::read_dir(self.agents_dir())? {
            if let Some(n) = entry?.file_name().to_str().and_then(id_number) {
                max = max.max(n);
            }
        }
        Ok(format!("a{}", max + 1))
    }
}

/// Reap a helper whose leaf has just been emptied. Bounded rather than a
/// blocking `wait`: the kernel marks a leaf empty (`cgroup_exit`) a moment
/// before the process becomes reapable (`exit_notify`), so an immediate
/// check would be flaky, but a helper still running a second after its
/// leaf emptied means the leaf was not its, and waiting forever would turn
/// that bug into a hang.
fn reap(id: &str, agent: &Agent) -> io::Result<()> {
    for _ in 0..100 {
        if agent.try_wait()?.is_some() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err(io::Error::other(format!(
        "agent {id}: its init helper (pid {}) outlived its emptied leaf",
        agent.pid()
    )))
}

fn id_number(id: &str) -> Option<u64> {
    id.strip_prefix('a')?.parse().ok()
}

fn private_dir(path: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    // A directory that existed before keeps its old mode otherwise.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

fn bind_private(path: &Path) -> io::Result<UnixListener> {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn write_record(dir: &Path, record: &AgentRecord) -> io::Result<()> {
    let tmp = dir.join("record.json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(record).map_err(io::Error::other)?,
    )?;
    std::fs::rename(tmp, dir.join("record.json"))
}

fn read_record(dir: &Path) -> io::Result<Option<AgentRecord>> {
    match std::fs::read(dir.join("record.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The agent's environment crosses a JSON pipe as UTF-8. A variable that is
/// not UTF-8 is refused rather than mangled on the way.
fn lossless(s: std::ffi::OsString) -> io::Result<String> {
    s.into_string().map_err(|s| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("environment value is not UTF-8: {s:?}"),
        )
    })
}

/// FNV-1a, 64-bit: stable across builds and platforms, which `std`'s hasher
/// is not, and a fingerprint is compared across supervisor restarts.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_matches_its_reference_values() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn ids_are_numbered_and_never_parsed_loosely() {
        assert_eq!(id_number("a17"), Some(17));
        assert_eq!(id_number("a"), None);
        assert_eq!(id_number("b3"), None);
        assert_eq!(id_number("a3x"), None);
    }
}
