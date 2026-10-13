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
//! restriction, 0600 in a 0700 directory), then the host key, the
//! host-side helpers, the cgroup leaf and the agent. A failure at any step
//! removes what the earlier steps made. A crash in the middle leaves a
//! directory without a record, and the next reconcile removes it, so no
//! agent is ever half-started with endpoints nobody serves. The helpers do
//! not wait for that reconcile: they exit when their lifeline's last
//! writer goes (`fleet::lifeline`), which is this process until the
//! agent's PID 1 holds it, and the agent's PID 1 after.
//!
//! **One supervisor per fleet at a time.** Nothing here locks: two
//! supervisors over one state directory would race for IDs and each
//! reconcile the other's half-started agents away. `fleet::commands`
//! holds the fleet's lock around every use.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::cgroup::{Evidence, FleetNode, Limits, Teardown};
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
    /// Hosts the agent may reach, through its own egress proxy. Empty means
    /// no egress at all: the agent's network namespace has no route out.
    pub egress_allow: Vec<String>,
    /// The services its keeper starts, from a supervisor config already
    /// written for this agent (`services::write_config`, named by its ID).
    /// Empty for none.
    pub services: Vec<String>,
    /// Services to reach from the host: each is mapped from an allocated
    /// host port to its declared port inside the agent (`service-ports`).
    pub expose: Vec<(String, u16)>,
    /// A wall-clock limit, enforced by the agent's PID 1 (`AgentSpec::timeout_secs`).
    pub timeout: Option<std::time::Duration>,
    /// The repository the workspace is a subdirectory of, when it is one:
    /// the agent's own clone, which its view mounts at `/workspace` so git
    /// finds `.git`. Set by [`AgentLaunch::in_repository`]; `None` mounts
    /// the workspace itself.
    pub repository: Option<PathBuf>,
    /// The compiled policy `plan` came from, origins included, written for
    /// the agent to read (`why` inside it), as `up` writes a sandbox's.
    /// `None` writes nothing.
    pub policy: Option<crate::policy::CompiledPolicy>,
}

impl AgentLaunch {
    /// Work in `workspace` as a subdirectory of the repository at `root`:
    /// the view mounts `root` at `/workspace`, the agent starts in its
    /// project's directory beneath it, and `root` is granted read-write, so
    /// git finds `.git` and can commit. Without it an agent of a monorepo's
    /// subproject had no repository at all (adversarial review, 3.7). The
    /// clone is the agent's own, so what it gains is its own history.
    /// Nothing changes when the project is the repository's root.
    pub fn in_repository(mut self, root: &Path) -> Self {
        if root == self.workspace || !self.workspace.starts_with(root) {
            return self;
        }
        let value = root.to_string_lossy().into_owned();
        self.plan.filesystem_allow.push(value.clone());
        self.policy = self.policy.take().map(|p| p.with_fleet_clone_grant(value));
        self.repository = Some(root.to_path_buf());
        self
    }
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
    /// The service it reaches.
    #[serde(default)]
    pub service: String,
    /// On the host's loopback, allocated by the OS.
    pub host: u16,
    /// The service's declared port, inside the agent.
    pub agent: u16,
}

/// An agent's durable identity (`record.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: String,
    /// The workspace's path on the host.
    pub workspace: PathBuf,
    /// Its path inside the agent: `/workspace` with a view, the host path
    /// without one. Absent in a record from before it was recorded.
    #[serde(default)]
    pub inside: Option<PathBuf>,
    pub cgroup: PathBuf,
    pub state: AgentState,
    /// FNV-1a of the compiled plan's JSON: whether two agents, or one
    /// agent before and after a change, run under the same policy.
    pub policy_fingerprint: String,
    /// Which mount-view strategy is in force (D2a says it is recorded
    /// and reported, never silently downgraded).
    pub view: bool,
    pub port_mappings: Vec<PortMapping>,
    /// Why the agent's processes died, when the kernel recorded a reason
    /// (an OOM kill, refused forks). Read from the leaf before it is
    /// removed, since its counters go with it; absent on a clean stop.
    #[serde(default)]
    pub evidence: Option<Evidence>,
    /// The hosts its egress proxy allows; empty for no egress.
    #[serde(default)]
    pub egress: Vec<String>,
    /// The limits its leaf was created with.
    #[serde(default)]
    pub limits: Limits,
    /// The services its keeper runs.
    #[serde(default)]
    pub services: Vec<String>,
    /// The services it asked to expose on the host. Kept apart from
    /// `port_mappings`, which is emptied when the agent stops, so "none
    /// declared" and "released" stay distinguishable.
    #[serde(default)]
    pub exposes: Vec<String>,
    /// Its wall-clock limit, in seconds.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// How it ended, when its PID 1 saw the end: its command's exit, or its
    /// deadline. Read at retirement from the report only PID 1 could write;
    /// absent for a `stop` or a kill of the whole leaf.
    #[serde(default)]
    pub exit: Option<init::ExitReport>,
}

/// A record plus what is true of it right now.
#[derive(Debug, Clone)]
pub struct AgentStatus {
    pub record: AgentRecord,
    /// `memory.current`, while running.
    pub memory_bytes: Option<u64>,
    /// `cpu.stat: usage_usec`, while running.
    pub cpu_usec: Option<u64>,
    /// Its most recent task, and what devcroft observed of it.
    pub task: Option<super::task::TaskRecord>,
    /// When that task's log last grew.
    pub last_output_unix: Option<u64>,
    /// What the agent reported (`fleet::report`), or why it is not shown.
    pub report: Result<Option<super::report::Report>, String>,
}

impl AgentStatus {
    /// `record` with what its directories say about its work.
    pub(crate) fn of(
        record: AgentRecord,
        agent_dir: &Path,
        memory_bytes: Option<u64>,
        cpu_usec: Option<u64>,
    ) -> AgentStatus {
        let task = super::task::latest(agent_dir).ok().flatten();
        let last_output_unix = task.as_ref().and_then(|t| {
            std::fs::metadata(super::task::log_path(agent_dir, &t.id))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        });
        let report = super::report::read(&super::report::path(&record.workspace, &record.id));
        AgentStatus {
            record,
            memory_bytes,
            cpu_usec,
            task,
            last_output_unix,
            report,
        }
    }

    /// Whether the agent reported that it needs a human.
    pub fn needs_attention(&self) -> bool {
        matches!(&self.report, Ok(Some(r)) if r.attention.is_some())
    }
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
                for name in [id.clone(), host_leaf_name(&id)] {
                    if let Some(leaf) = self.node.existing_leaf(&name) {
                        let _ = leaf.kill();
                    }
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

        // Egress, when the agent has any: its own proxy, host-side and
        // outside the agent's leaf (D4, D6), reached from inside the
        // route-less namespace through the keeper's relay to the proxy's
        // unix socket, which crosses the namespace. The plan's port gate
        // and the environment point at the same proxy, as in `up`.
        let mut plan = launch.plan.clone();
        let mut provider_env = launch.provider_env.clone();
        let proxy_socket = dir.join("proxy.sock");
        // Everything host-side that serves this agent (its egress proxy, its
        // port forwarders) shares one leaf beside the agent's, and dies with
        // the agent: each holds the read end of a lifeline whose write end
        // only the agent's PID 1 keeps (`fleet::lifeline`), so it exits when
        // the agent does, with no command running to notice. Killing the
        // leaf at `stop` and at reconcile stays, as the backstop.
        let (host_leaf, lifeline) = if launch.egress_allow.is_empty() && launch.expose.is_empty() {
            (None, None)
        } else {
            (
                Some(
                    self.node
                        .create_leaf(&host_leaf_name(id), &host_helper_limits())?,
                ),
                Some(super::lifeline::Lifeline::new()?),
            )
        };
        let relay = if launch.egress_allow.is_empty() {
            None
        } else {
            let proxy_leaf = host_leaf.as_ref().expect("created when there is egress");
            let (_, port, token) = crate::proxy::spawn_at(
                &self.exe,
                &proxy_socket,
                &dir.join("egress.log"),
                &launch.egress_allow,
                |cmd| {
                    // Every record names the agent, so the log attributes
                    // requests on its own, wherever it is read.
                    cmd.env("DEVCROFT_EGRESS_LABEL", id);
                    let line = lifeline.as_ref().expect("made with the host leaf");
                    super::lifeline::attach(cmd, &line.read);
                    proxy_leaf.attach_on_spawn(cmd)
                },
            )?;
            plan.network_proxy_port = Some(port);
            provider_env.extend(crate::proxy::client_env(port, &token));
            Some(port)
        };

        // Host mappings (`service-ports`), the egress relay run backwards:
        // host 127.0.0.1:<allocated> --TCP--> `__ingress` --UDS--> the
        // keeper --TCP--> 127.0.0.1:<declared>, inside the agent. The UDS is
        // bound here, before restriction, and inherited, like the control
        // socket; so the agent gets no route, and the service is neither
        // involved nor told. The host port is the OS's choice.
        let first = init::FIRST_INHERITED_FD;
        let mut ingress_fds: Vec<std::os::fd::OwnedFd> = Vec::new();
        let mut ingress_env: Vec<String> = Vec::new();
        let mut mappings = Vec::new();
        for (k, (service, port)) in launch.expose.iter().enumerate() {
            let socket = dir.join(format!("ingress-{service}.sock"));
            let uds = bind_private(&socket)?;
            let tcp = std::net::TcpListener::bind(("127.0.0.1", 0))?;
            let host_port = tcp.local_addr()?.port();
            // Inheritable in the forwarder alone: clearing CLOEXEC here would
            // let a child another thread forks meanwhile keep the listener,
            // and the host port with it, after this agent is gone (measured).
            let tcp_fd = std::os::fd::AsRawFd::as_raw_fd(&tcp);
            let mut cmd = std::process::Command::new(&self.exe);
            crate::lifecycle::pass_fds(&mut cmd, &[tcp_fd]);
            cmd.arg("__ingress")
                .arg(tcp_fd.to_string())
                .arg(&socket)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                // Its own log, as the egress proxy has one: a forwarder that
                // dies is otherwise a host port that refuses, with no reason.
                .stderr(std::fs::File::create(
                    dir.join(format!("ingress-{service}.log")),
                )?);
            host_leaf
                .as_ref()
                .expect("created when there is a mapping")
                .attach_on_spawn(&mut cmd)?;
            super::lifeline::attach(
                &mut cmd,
                &lifeline.as_ref().expect("made with the host leaf").read,
            );
            // SAFETY: setsid only, in the forked child.
            unsafe {
                std::os::unix::process::CommandExt::pre_exec(&mut cmd, || {
                    libc::setsid();
                    Ok(())
                });
            }
            cmd.spawn()?;
            // The forwarder has its own copy; ours must not reach later agents.
            drop(tcp);
            ingress_env.push(format!("{}:{port}", first + 2 + k as std::os::fd::RawFd));
            ingress_fds.push(uds.into());
            mappings.push(PortMapping {
                service: service.clone(),
                host: host_port,
                agent: *port,
            });
        }

        // With a view, the agent sees its workspace at `/workspace` and
        // nowhere else (D2a), but everything below was resolved against the
        // workspace's host path: the provider's environment (devbox's
        // `PATH` and `DEVBOX_PROJECT_ROOT`, flox's `FLOX_ENV_*`), the
        // services config, the plan's grants. So every path handed to the
        // agent is translated, at the two places they all pass through: the
        // keeper's environment and the helper's plan. Without a view the
        // host root is kept, which has no `/workspace` to offer.
        let mounted = launch.repository.as_ref().unwrap_or(&launch.workspace);
        let at = launch.view.then(|| AtWorkspace::new(mounted));
        let translate = |s: String| match &at {
            Some(at) => at.translate(&s),
            None => s,
        };
        if let Some(at) = &at
            && !launch.services.is_empty()
        {
            let config = crate::services::config_path(&launch.workspace, id);
            let text = std::fs::read_to_string(&config)?;
            std::fs::write(&config, at.translate(&text))?;
        }
        let inside = match &at {
            Some(at) => PathBuf::from(at.translate(&launch.workspace.to_string_lossy())),
            None => launch.workspace.clone(),
        };
        // The agent's compiled policy, where it can read it: what `why` and
        // `policy --render` answer from inside it. Fleet wrote none, so `why`
        // in an agent answered from the manifest alone, DENIED for its own
        // provider's paths (adversarial review). With this agent's proxy
        // port, and its paths as the agent sees them.
        if let Some(policy) = &launch.policy {
            let policy = match relay {
                Some(port) => policy.clone().with_proxy_port(port),
                None => policy.clone(),
            };
            let artifact = crate::services::artifact_dir(&launch.workspace, id).join("policy.json");
            if let Some(parent) = artifact.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(
                &artifact,
                translate(serde_json::to_string_pretty(&policy).map_err(io::Error::other)?),
            )?;
        }

        let env = crate::lifecycle::KeeperEnv {
            env: &provider_env,
            unset: &launch.unset,
            plan: &plan,
            ssh_host_key_pem: &host_key,
            ssh_authorized_key_pem: &launch.authorized_key_pem,
            shell: &launch.shell,
            // The keeper finds the config by this name under the workspace.
            services: (!launch.services.is_empty()).then_some(id),
            hooks: &launch.hooks,
            project_root: &launch.workspace,
            sandbox_home: &home,
            relay: relay.map(|port| (port, proxy_socket.as_path())),
        }
        .vars()
        .into_iter()
        .map(|(k, v)| Ok((lossless(k)?, translate(lossless(v)?))))
        .collect::<io::Result<Vec<_>>>()?;
        let mut env = env;
        if !ingress_env.is_empty() {
            env.push(("DEVCROFT_INGRESS".into(), ingress_env.join(",")));
        }

        let view = if launch.view {
            let root = dir.join("root");
            std::fs::create_dir(&root)?;
            Some(View {
                root,
                proxy_socket: relay.map(|_| proxy_socket.clone()),
                workspace: Some(mounted.clone()),
            })
        } else {
            None
        };

        let spec = AgentSpec {
            command: vec![
                self.exe.to_string_lossy().into_owned(),
                "__keeper".into(),
                first.to_string(),
                (first + 1).to_string(),
            ],
            env,
            cwd: Some(inside.clone()),
            hostname: id.to_owned(),
            // The effective plan, proxy port included: this is what the
            // helper turns into the Landlock ruleset, inside the view.
            plan: serde_json::from_str(&translate(
                serde_json::to_string(&plan).map_err(io::Error::other)?,
            ))
            .map_err(io::Error::other)?,
            project_root: inside.clone(),
            relay_port: relay,
            view,
            timeout_secs: launch.timeout.map(|t| t.as_secs().max(1)),
        };
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("keeper.log"))?;
        let stdio = Stdio {
            stdout: Some(log.try_clone()?.into()),
            stderr: Some(log.into()),
            inherit: [control.into(), ssh.into()]
                .into_iter()
                .chain(ingress_fds)
                .collect(),
            // Every helper has its read end by now. Moved, not copied: once
            // PID 1 has it, this process must hold no writer, or a helper
            // would outlive the agent for as long as this supervisor runs.
            lifeline: lifeline.map(|line| line.write),
            // In the agent's state directory, which the baseline denies to
            // every sandbox, and handed to PID 1 alone.
            report: Some(std::fs::File::create(dir.join("exit.json"))?.into()),
        };

        let leaf = self.node.create_leaf(id, &launch.limits)?;
        let agent = init::spawn(&self.exe, &leaf, &spec, stdio)?;
        self.children.insert(id.to_owned(), agent);

        // The manifest's plan, not the effective one: the proxy port differs
        // per agent, and agents under the same policy should compare equal.
        let plan_json = serde_json::to_vec(&launch.plan).map_err(io::Error::other)?;
        write_record(
            dir,
            &AgentRecord {
                id: id.to_owned(),
                workspace: launch.workspace.clone(),
                inside: Some(inside),
                cgroup: leaf.path().to_path_buf(),
                state: AgentState::Running,
                policy_fingerprint: format!("{:016x}", fnv1a64(&plan_json)),
                view: launch.view,
                port_mappings: mappings,
                evidence: None,
                egress: launch.egress_allow.clone(),
                limits: launch.limits.clone(),
                services: launch.services.clone(),
                exposes: launch.expose.iter().map(|(s, _)| s.clone()).collect(),
                timeout_secs: launch.timeout.map(|t| t.as_secs().max(1)),
                exit: None,
            },
        )
    }

    /// Every recorded agent, reconciled against the kernel first, in ID
    /// order.
    pub fn list(&mut self) -> io::Result<Vec<AgentStatus>> {
        self.reconcile()?;
        let mut out = Vec::new();
        for record in self.records()? {
            let running = record.state == AgentState::Running;
            let dir = self.agent_dir(&record.id);
            // A runner that died with nothing recorded, in a live agent.
            super::task::settle(&dir, running)?;
            let leaf = running
                .then(|| self.node.existing_leaf(&record.id))
                .flatten();
            out.push(AgentStatus::of(
                record,
                &dir,
                leaf.as_ref().and_then(|l| l.memory_current().ok()),
                leaf.as_ref().and_then(|l| l.cpu_usage_usec().ok()),
            ));
        }
        Ok(out)
    }

    /// Stop one agent: everything in its leaf, however detached, and only
    /// that. Its record stays, as Stopped.
    pub fn stop(&mut self, id: &str) -> io::Result<()> {
        let record = read_record(&self.agent_dir(id))?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no agent {id}")))?;
        self.retire(record)?;
        if let Some(agent) = self.children.remove(id) {
            reap(id, &agent)?;
        }
        Ok(())
    }

    /// One agent, reconciled first.
    pub fn inspect(&mut self, id: &str) -> io::Result<AgentStatus> {
        self.list()?
            .into_iter()
            .find(|a| a.record.id == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no agent {id}")))
    }

    /// Forget a stopped agent: its directory and record. Refuses a running
    /// one. Returns the record, so the caller can remove what it made for
    /// the agent (its workspace).
    pub fn remove(&mut self, id: &str) -> io::Result<AgentRecord> {
        let status = self.inspect(id)?;
        if status.record.state == AgentState::Running {
            return Err(io::Error::new(
                io::ErrorKind::ResourceBusy,
                format!("agent {id} is running; stop it first"),
            ));
        }
        std::fs::remove_dir_all(self.agent_dir(id))?;
        Ok(status.record)
    }

    /// The whole fleet's limits, on its node (`FleetLimits`).
    pub fn set_fleet_limits(&self, limits: &super::cgroup::FleetLimits) -> io::Result<()> {
        self.node.set_fleet_limits(limits)
    }

    /// Limits this host cannot apply to any agent, named (`Degraded`).
    pub fn degraded(&self) -> Vec<super::cgroup::Degraded> {
        self.node.degraded()
    }

    /// Remove the fleet's cgroup node. Fails while any agent's leaf exists.
    pub fn remove_node(self) -> io::Result<()> {
        self.node.remove()
    }

    /// Empty the agent's leaf, keep the kernel's reason if it recorded
    /// one, and record the agent Stopped. The evidence is read first: the
    /// counters live in the leaf, and removing it discards them.
    fn retire(&self, mut record: AgentRecord) -> io::Result<()> {
        if let Some(leaf) = self.node.existing_leaf(&record.id) {
            if let Ok(evidence) = leaf.exit_evidence()
                && !evidence.is_quiet()
            {
                record.evidence = Some(evidence);
            }
            if let Teardown::LeftBehind(path) = leaf.kill()? {
                return Err(io::Error::other(format!(
                    "agent {}: processes remain in {} after cgroup.kill; left in place",
                    record.id,
                    path.display()
                )));
            }
        }
        // Its host-side helpers go with it: a proxy outliving its agent
        // would hold an allowlist open for nobody, and a forwarder its host
        // port.
        if let Some(proxy) = self.node.existing_leaf(&host_leaf_name(&record.id)) {
            proxy.kill()?;
        }
        // Written by PID 1 as it ended, so present only when it saw the end;
        // empty (created at start, never written) when it was killed.
        record.exit = std::fs::read(self.agent_dir(&record.id).join("exit.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        record.state = AgentState::Stopped;
        // Its task, if one ran: devcroft killed it, so devcroft records it.
        super::task::settle(&self.agent_dir(&record.id), false)?;
        self.release(&record)
    }

    /// Run `command` in a running agent as its next task, held by a
    /// host-side runner (`fleet::task`). Refused while its last task runs.
    /// The agent's previous reports go: a new task starts with none.
    pub fn run_task(
        &mut self,
        id: &str,
        command: &[String],
    ) -> io::Result<super::task::TaskRecord> {
        if command.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no command to run",
            ));
        }
        let status = self.inspect(id)?;
        if status.record.state != AgentState::Running {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("agent {id} is not running"),
            ));
        }
        if let Some(task) = status
            .task
            .filter(|t| t.outcome == super::task::Outcome::Running)
        {
            return Err(io::Error::new(
                io::ErrorKind::ResourceBusy,
                format!("agent {id} is running task {}; one task at a time", task.id),
            ));
        }
        let dir = self.agent_dir(id);
        let task_id = super::task::next_id(&dir)?;
        let record = &status.record;
        super::task::write(
            &dir,
            &super::task::TaskRecord {
                id: task_id.clone(),
                command: command.to_vec(),
                cwd: record
                    .inside
                    .clone()
                    .unwrap_or_else(|| record.workspace.clone()),
                started_unix: super::report::now_unix(),
                ended_unix: None,
                outcome: super::task::Outcome::Running,
                runner_pid: None,
            },
        )?;
        super::report::clear(&super::report::path(&record.workspace, id))?;

        // In the agent's helpers' leaf, budgeted and removed with it; made
        // now if the agent has no other helper.
        let leaf = match self.node.existing_leaf(&host_leaf_name(id)) {
            Some(leaf) => leaf,
            None => self
                .node
                .create_leaf(&host_leaf_name(id), &host_helper_limits())?,
        };
        let mut cmd = std::process::Command::new(&self.exe);
        cmd.arg("__task")
            .arg(self.control_socket(id))
            .arg(&dir)
            .arg(&task_id)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(
                super::task::tasks_dir(&dir).join(format!("{task_id}.runner.log")),
            )?);
        // Its own session: the command that started it may exit, and its
        // terminal's hangup must not reach the runner.
        // SAFETY: `setsid` is async-signal-safe.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut cmd, || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        leaf.attach_on_spawn(&mut cmd)?;
        let mut child = cmd.spawn()?;
        // Until the runner has recorded itself, so the task this returns
        // is one a later command can tell is alive. Under the fleet's lock,
        // so no other command settles it meanwhile.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let task = super::task::read(&dir, &task_id)?;
            if task.runner_pid.is_some() || task.outcome != super::task::Outcome::Running {
                return Ok(task);
            }
            if let Some(exit) = child.try_wait()? {
                return Err(io::Error::other(format!(
                    "the task runner exited ({exit}) before starting {task_id}; see {}",
                    super::task::tasks_dir(&dir)
                        .join(format!("{task_id}.runner.log"))
                        .display()
                )));
            }
            if std::time::Instant::now() > deadline {
                return Err(io::Error::other(format!(
                    "the task runner did not start {task_id} within 5 s"
                )));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
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
                    // Both leaves: a start that died after launching its
                    // helpers left them in the host leaf, and nothing but
                    // this can name them now. (Its lifeline has already told
                    // them to exit; this is the backstop.)
                    for name in [id.clone(), host_leaf_name(&id)] {
                        if let Some(leaf) = self.node.existing_leaf(&name) {
                            let _ = leaf.kill();
                        }
                    }
                    std::fs::remove_dir_all(&dir)?;
                }
                Some(record) if record.state == AgentState::Running => {
                    let alive = self
                        .node
                        .existing_leaf(&id)
                        .map(|leaf| leaf.populated())
                        .transpose()?
                        .unwrap_or(false);
                    if !alive {
                        self.retire(record)?;
                    }
                }
                Some(_) => {}
            }
        }

        // Leaves no running agent owns. The loop above reaches only leaves
        // that have an agent directory; a supervisor that crashed after
        // creating a leaf and before (or while) removing the directory left
        // one with none, and a stop whose `cgroup.kill` did not drain
        // (`Teardown::LeftBehind`) left one behind a Stopped record. Nothing
        // but this supervisor creates leaves under its node, and every
        // command holds the fleet's lock, so a leaf without a Running owner
        // is never one being started.
        for name in self.node.leaf_names()? {
            let owner = name.strip_suffix("-host").unwrap_or(&name);
            let running = read_record(&self.agent_dir(owner))
                .ok()
                .flatten()
                .is_some_and(|r| r.state == AgentState::Running);
            if !running && let Some(leaf) = self.node.existing_leaf(&name) {
                let _ = leaf.kill();
            }
        }
        Ok(())
    }

    /// Record a Stopped agent and release what only a running one needs:
    /// its sockets and its view root. Port mappings go with them.
    fn release(&self, record: &AgentRecord) -> io::Result<()> {
        release_files(&self.agent_dir(&record.id), record)
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

    /// `a<N>`, one past the highest ever used: an ID is never reused for
    /// the fleet's lifetime, so a stopped agent's record, an old
    /// `a17.<name>.devcroft` in someone's shell history, or a clone path
    /// cannot come to mean a different agent.
    ///
    /// **Recorded, not only scanned.** Scanning the agent directories alone
    /// handed a removed agent's ID straight back out once the highest one
    /// was `rm`'d. The scan stays, for a fleet whose counter predates it.
    /// Written before the agent's directory exists, so a crash between the
    /// two skips an ID rather than repeating one. `rm --all` ends the fleet,
    /// counter included; its next `up` is a new fleet.
    fn next_id(&self) -> io::Result<String> {
        let counter = self.state.join("last-id");
        let mut max = match std::fs::read_to_string(&counter) {
            Ok(text) => text.trim().parse::<u64>().map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: {e}", counter.display()),
                )
            })?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e),
        };
        for entry in std::fs::read_dir(self.agents_dir())? {
            if let Some(n) = entry?.file_name().to_str().and_then(id_number) {
                max = max.max(n);
            }
        }
        let next = max + 1;
        let tmp = self.state.join("last-id.tmp");
        std::fs::write(&tmp, format!("{next}\n"))?;
        std::fs::rename(tmp, counter)?;
        Ok(format!("a{next}"))
    }
}

/// Record an agent Stopped and remove what only a running one needs: its
/// sockets, its view root. Its port mappings go with them.
fn release_files(dir: &Path, record: &AgentRecord) -> io::Result<()> {
    for socket in ["control.sock", "ssh.sock", "proxy.sock"] {
        let _ = std::fs::remove_file(dir.join(socket));
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("ingress-") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir(dir.join("root"));
    let mut record = record.clone();
    record.port_mappings.clear();
    write_record(dir, &record)
}

/// The records of a fleet whose cgroup subtree is gone, every one settled
/// as Stopped, in ID order.
///
/// **No agent of such a fleet can be running**: cgroup v2 cannot remove a
/// populated cgroup, so a subtree that is gone was emptied first, by a
/// reboot or by the end of the systemd unit that delegated it. So nothing
/// here needs, or can have, a `Supervisor`, which opens the fleet's cgroup
/// node and would fail on the missing root; reconciling is retiring every
/// Running record, and a directory without a record (a start interrupted)
/// is removed as reconcile would. The kernel's exit evidence went with the
/// cgroups and is not recorded.
pub fn settle_offline(state: &Path) -> io::Result<Vec<AgentRecord>> {
    let mut records = Vec::new();
    let dir = state.join("agents");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(records),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let path = entry?.path();
        match read_record(&path)? {
            None => std::fs::remove_dir_all(&path)?,
            Some(mut record) => {
                if record.state == AgentState::Running {
                    record.state = AgentState::Stopped;
                    release_files(&path, &record)?;
                    record.port_mappings.clear();
                }
                super::task::settle(&path, false)?;
                records.push(record);
            }
        }
    }
    records.sort_by_key(|r| id_number(&r.id));
    Ok(records)
}

/// Remove a settled agent's record and directory, for a fleet whose
/// cgroup subtree is gone (`settle_offline`).
pub fn forget_offline(state: &Path, id: &str) -> io::Result<AgentRecord> {
    let dir = state.join("agents").join(id);
    let record = read_record(&dir)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no agent {id}")))?;
    std::fs::remove_dir_all(&dir)?;
    Ok(record)
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

/// Rewrites a workspace's host path to [`super::mount::WORKSPACE`] wherever
/// it appears as a whole path or a path's prefix.
///
/// **On path boundaries only.** The host path of agent `a1`'s workspace is
/// a prefix of `a10`'s as a string, and `/x/proj` is inside `/y/x/proj`; so
/// a match must start where a path can (the text's start, or after a
/// character no path contains, such as `:` in `PATH` or `"` in YAML) and
/// end where one can (its end, a `/`, or such a character). Both spellings
/// are rewritten, as given and canonical, since a provider may have used
/// either.
///
/// What this cannot see is a path a provider encoded (hashed, escaped).
/// Every value measured is a plain prefix (devbox, flox, devcroft's own
/// services config), and the symlinks inside a workspace are relative or
/// point into the store, so they resolve the same at `/workspace`.
struct AtWorkspace {
    host: Vec<String>,
}

impl AtWorkspace {
    fn new(workspace: &Path) -> AtWorkspace {
        let mut host = vec![workspace.to_string_lossy().into_owned()];
        if let Ok(canonical) = workspace.canonicalize() {
            let canonical = canonical.to_string_lossy().into_owned();
            if !host.contains(&canonical) {
                host.push(canonical);
            }
        }
        // Longest first, so one spelling never rewrites part of another.
        host.sort_by_key(|h| std::cmp::Reverse(h.len()));
        AtWorkspace { host }
    }

    fn translate(&self, text: &str) -> String {
        self.host.iter().fold(text.to_owned(), |t, h| {
            replace_path(&t, h, super::mount::WORKSPACE)
        })
    }
}

/// A character that can be part of a path as written in an environment
/// variable or a config file.
fn in_path(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | '~' | '@')
}

fn replace_path(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut prev: Option<char> = None;
    while let Some(i) = rest.find(from) {
        let before = rest[..i].chars().next_back().or(prev);
        let after = rest[i + from.len()..].chars().next();
        let starts = before.is_none_or(|c| !in_path(c));
        let ends = after.is_none_or(|c| c == '/' || !in_path(c));
        out.push_str(&rest[..i]);
        if starts && ends {
            out.push_str(to);
        } else {
            out.push_str(from);
        }
        prev = from.chars().next_back();
        rest = &rest[i + from.len()..];
    }
    out.push_str(rest);
    out
}

/// The limits of an agent's helpers' leaf: its proxy, its unix bridge and its
/// port forwarders. They run outside the agent so the agent cannot take them
/// down, and that must not make them outside any budget: measured by
/// adversarial review, 30 connections to an agent's proxy put 93 tasks in an
/// unlimited helpers' leaf beside an agent capped at 64. The proxy now admits
/// at most `proxy::server::MAX_CONNECTIONS` at a time (a few threads each, on
/// the TCP side and on the bridge), which these leave room for; they are the
/// backstop if anything else grows.
fn host_helper_limits() -> Limits {
    Limits {
        memory_max: Some(256 << 20),
        pids_max: Some(1024),
        ..Limits::default()
    }
}

/// The cgroup leaf an agent's host-side helpers (its egress proxy, its port
/// forwarders) run in: beside the agent's, never inside it (D6), so the
/// agent's memory pressure or a `cgroup.kill` of its leaf cannot take down
/// the component filtering it. Killed with the agent, which releases its
/// host ports.
fn host_leaf_name(id: &str) -> String {
    format!("{id}-host")
}

/// Agent `id`'s SSH socket in the fleet whose state is `state`, without
/// opening it: what `devcroft proxy <id>.<name>.devcroft` connects to.
/// `None` for anything that is not an agent ID, so a host name cannot
/// walk out of the agents directory.
pub fn agent_ssh_socket(state: &Path, id: &str) -> Option<PathBuf> {
    id_number(id)?;
    Some(state.join("agents").join(id).join("ssh.sock"))
}

/// `a<N>`'s N, only for the spelling `next_id` makes: `u64::from_str`
/// alone accepts `a+5` and `a05`, two more names for agent 5.
fn id_number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix('a')?;
    if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
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
    fn a_workspace_path_is_translated_only_where_it_is_a_whole_path() {
        let w = "/r/.devcroft/fleet/a1/proj";
        let t = |s: &str| replace_path(s, w, "/workspace");
        assert_eq!(t(w), "/workspace");
        assert_eq!(
            t(&format!("{w}/.devbox/bin:/usr/bin")),
            "/workspace/.devbox/bin:/usr/bin"
        );
        assert_eq!(t(&format!("/usr/bin:{w}/bin")), "/usr/bin:/workspace/bin");
        assert_eq!(t(&format!("\"{w}\"")), "\"/workspace\"");
        assert_eq!(t(&format!("{w} {w}/x")), "/workspace /workspace/x");
        // Not another agent's, not a longer name, not a path containing it.
        assert_eq!(
            t("/r/.devcroft/fleet/a1/proj2"),
            "/r/.devcroft/fleet/a1/proj2"
        );
        assert_eq!(t(&format!("/y{w}")), format!("/y{w}"));
        assert_eq!(t(&format!("{w}.bak")), format!("{w}.bak"));
        let a1 = "/r/.devcroft/fleet/a1";
        assert_eq!(
            replace_path("/r/.devcroft/fleet/a10/proj", a1, "/workspace"),
            "/r/.devcroft/fleet/a10/proj"
        );
    }

    #[test]
    fn ids_are_numbered_and_never_parsed_loosely() {
        assert_eq!(id_number("a17"), Some(17));
        assert_eq!(id_number("a"), None);
        assert_eq!(id_number("b3"), None);
        assert_eq!(id_number("a3x"), None);
        assert_eq!(id_number("a+5"), None);
        assert_eq!(id_number("a05"), None);
        assert_eq!(id_number("a0"), None);
    }
}
