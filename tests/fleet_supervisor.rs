//! `add-linux-agent-fleet`'s `agent-supervisor` spec, against real agents:
//! N agents by ID, each serving sessions as itself, stopped one at a time,
//! and reconciled by a supervisor that restarts while they run.
//!
//! Same gates as `tests/fleet_init.rs`: a delegated cgroup root this
//! process sits inside, and a host that can mount a fresh procfs in a
//! user+PID namespace.

#![cfg(target_os = "linux")]

use devcroft::fleet::cgroup::Limits;
use devcroft::fleet::supervisor::{AgentLaunch, AgentState, Supervisor};
use devcroft::keeper::protocol::{self, Frame, SpawnRequest};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;

fn capable_host() -> Option<PathBuf> {
    let Some(root) = std::env::var_os("DEVCROFT_TEST_CGROUP_ROOT").map(PathBuf::from) else {
        eprintln!(
            "skipping: DEVCROFT_TEST_CGROUP_ROOT is not set (no delegated cgroup to test in)"
        );
        return None;
    };
    let own = devcroft::fleet::cgroup::own_cgroup().ok()?;
    if !own.starts_with(&root) {
        eprintln!(
            "skipping: this process is in {}, outside {}; run `sudo cgroup-delegate enter $$` first",
            own.display(),
            root.display()
        );
        return None;
    }
    let proc_ok = Command::new("unshare")
        .args([
            "--user",
            "--map-root-user",
            "--pid",
            "--fork",
            "--mount",
            "--mount-proc",
            "true",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !proc_ok {
        eprintln!("skipping: this host cannot mount a fresh /proc in a user+PID namespace");
        return None;
    }
    Some(root)
}

/// A scratch fleet: a state directory, workspaces, and a fleet node, all
/// removed on drop, killing whatever is still running first.
struct Fleet {
    base: PathBuf,
    cgroup_root: PathBuf,
    name: String,
}

impl Fleet {
    fn new(cgroup_root: &Path, tag: &str) -> Fleet {
        let name = format!("devcroft-sup-test-{}-{tag}", std::process::id());
        let base = std::env::temp_dir().join(&name);
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        Fleet {
            base,
            cgroup_root: cgroup_root.to_path_buf(),
            name,
        }
    }

    fn state(&self) -> PathBuf {
        self.base.join("state")
    }

    fn open(&self) -> Supervisor {
        Supervisor::open(
            &self.state(),
            &self.cgroup_root,
            &self.name,
            Path::new(env!("CARGO_BIN_EXE_devcroft")),
        )
        .unwrap()
    }

    /// A launch for a fresh workspace, compiled the way `up` compiles a
    /// manifest granting `.` and reading `/proc`, with `/usr` as the
    /// closure and the keeper binary's directory granted.
    fn launch(&self, workspace: &str) -> AgentLaunch {
        let workspace = self.base.join("work").join(workspace);
        std::fs::create_dir_all(&workspace).unwrap();
        let exe = Path::new(env!("CARGO_BIN_EXE_devcroft"));
        let (manifest, _) = devcroft::config::parse(
            "[sandbox]\nname = \"fleet-sup\"\n\n[env]\nprovider = \"flox\"\n\n\
             [filesystem]\nallow = [\".\"]\nread = [\"/proc\"]\n",
        )
        .unwrap();
        let plan = devcroft::policy::compile(&manifest)
            .with_provider_grants("flox", &["/usr".to_owned()])
            .with_keeper_exe_grant(exe.parent().unwrap().to_string_lossy().into_owned())
            .to_capability_plan();
        let keys = self.base.join("keys");
        std::fs::create_dir_all(&keys).unwrap();
        let client =
            devcroft::ssh::ensure_client_keypair(&keys.join("id"), &keys.join("id.pub")).unwrap();
        AgentLaunch {
            workspace,
            plan,
            provider_env: [("PATH".to_owned(), std::env::var("PATH").unwrap())].into(),
            unset: Vec::new(),
            shell: PathBuf::from("/usr/bin/sh"),
            hooks: Vec::new(),
            authorized_key_pem: client.public_key().to_openssh().unwrap(),
            limits: Limits::default(),
            view: true,
        }
    }
}

impl Drop for Fleet {
    fn drop(&mut self) {
        let node = self.cgroup_root.join(&self.name);
        if let Ok(entries) = std::fs::read_dir(&node) {
            for leaf in entries.flatten().filter(|e| e.path().is_dir()) {
                let _ = std::fs::write(leaf.path().join("cgroup.kill"), "1");
                std::thread::sleep(std::time::Duration::from_millis(50));
                let _ = std::fs::remove_dir(leaf.path());
            }
        }
        let _ = std::fs::remove_dir(&node);
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// Run `script` in a session on the agent behind `socket`; (exit code,
/// output).
fn session(socket: &Path, cwd: &Path, script: &str) -> (Option<i32>, String) {
    let mut stream = UnixStream::connect(socket).unwrap();
    protocol::write_frame(
        &mut stream,
        &Frame::Spawn(SpawnRequest {
            cmd: "sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: cwd.to_string_lossy().into_owned(),
            env: Default::default(),
            pty: None,
        }),
    )
    .unwrap();
    let mut out = String::new();
    loop {
        match protocol::read_frame(&mut stream).unwrap() {
            Frame::SpawnErr { message } => panic!("keeper refused to spawn: {message}"),
            Frame::Stdout(b) | Frame::Stderr(b) => out.push_str(&String::from_utf8_lossy(&b)),
            Frame::Exit(status) => return (status.code, out),
            _ => {}
        }
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn agents_start_by_id_serve_as_themselves_and_stop_one_at_a_time() {
    let Some(root) = capable_host() else { return };
    let fleet = Fleet::new(&root, "two");
    let mut sup = fleet.open();
    let (la, lb) = (fleet.launch("a"), fleet.launch("b"));
    let a = sup.start(&la).unwrap();
    let b = sup.start(&lb).unwrap();
    assert_eq!((a.as_str(), b.as_str()), ("a1", "a2"));

    // Each agent's endpoint reaches that agent: its hostname is its ID,
    // and what it writes lands in its own workspace.
    for (id, launch) in [(&a, &la), (&b, &lb)] {
        let (code, out) = session(
            &sup.control_socket(id),
            &launch.workspace,
            &format!("cat /proc/sys/kernel/hostname; echo {id} > whoami"),
        );
        assert_eq!(code, Some(0), "{out}");
        assert_eq!(out.trim(), id.as_str());
        assert_eq!(
            std::fs::read_to_string(launch.workspace.join("whoami"))
                .unwrap()
                .trim(),
            id.as_str()
        );
    }

    // Permissions, not location, are the access boundary.
    assert_eq!(mode(&fleet.state().join("agents")), 0o700);
    for id in [&a, &b] {
        assert_eq!(mode(&fleet.state().join("agents").join(id)), 0o700);
        assert_eq!(mode(&sup.control_socket(id)), 0o600);
        assert_eq!(mode(&sup.ssh_socket(id)), 0o600);
    }

    let listed = sup.list().unwrap();
    assert_eq!(listed.len(), 2);
    for status in &listed {
        assert_eq!(status.record.state, AgentState::Running);
        assert!(status.record.view);
        assert!(status.memory_bytes.unwrap() > 0, "{status:?}");
        assert!(status.cpu_usec.is_some(), "{status:?}");
    }
    // Same plan, same fingerprint.
    assert_eq!(
        listed[0].record.policy_fingerprint,
        listed[1].record.policy_fingerprint
    );

    // Stopping one leaves the other running and reachable.
    sup.stop(&a).unwrap();
    let (code, out) = session(&sup.control_socket(&b), &lb.workspace, "echo still-here");
    assert_eq!((code, out.trim()), (Some(0), "still-here"));
    let listed = sup.list().unwrap();
    assert_eq!(listed[0].record.state, AgentState::Stopped);
    assert_eq!(listed[0].memory_bytes, None);
    assert_eq!(listed[1].record.state, AgentState::Running);
    assert!(!sup.control_socket(&a).exists());
    assert!(!Path::new(&root).join(&fleet.name).join(&a).exists());

    // IDs are never reused: the stopped a1 keeps its record.
    let c = sup.start(&fleet.launch("c")).unwrap();
    assert_eq!(c, "a3");
    sup.stop(&b).unwrap();
    sup.stop(&c).unwrap();
}

#[test]
fn a_restarted_supervisor_adopts_live_agents_and_retires_dead_ones() {
    let Some(root) = capable_host() else { return };
    let fleet = Fleet::new(&root, "restart");
    let (la, lb) = (fleet.launch("a"), fleet.launch("b"));
    let (a, b) = {
        let mut sup = fleet.open();
        (sup.start(&la).unwrap(), sup.start(&lb).unwrap())
        // Dropped without stopping anything: the supervisor "crashes".
    };

    // While it is down, one agent dies, and a start is left half-done.
    // cgroup.kill is asynchronous: the agent has died only once its leaf
    // reads empty, and reopening before that would find it (correctly)
    // still running.
    let b_leaf = root.join(&fleet.name).join(&b);
    std::fs::write(b_leaf.join("cgroup.kill"), "1").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::fs::read_to_string(b_leaf.join("cgroup.events"))
        .unwrap()
        .contains("populated 1")
    {
        assert!(std::time::Instant::now() < deadline, "agent b never died");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let interrupted = fleet.state().join("agents").join("a9");
    std::fs::create_dir(&interrupted).unwrap();

    let mut sup = fleet.open();
    let listed = sup.list().unwrap();
    let state = |id: &str| {
        listed
            .iter()
            .find(|s| s.record.id == id)
            .map(|s| s.record.state)
    };
    // The live agent is adopted: still Running, still serving.
    assert_eq!(state(&a), Some(AgentState::Running));
    let (code, out) = session(&sup.control_socket(&a), &la.workspace, "echo adopted");
    assert_eq!((code, out.trim()), (Some(0), "adopted"));
    // The dead one is reconciled, not reported as running, and its
    // endpoints are released.
    assert_eq!(state(&b), Some(AgentState::Stopped));
    assert!(!sup.ssh_socket(&b).exists());
    // The interrupted start is gone rather than listed.
    assert_eq!(state("a9"), None);
    assert!(!interrupted.exists());

    // An adopted agent is not this process's child; its leaf is the handle.
    sup.stop(&a).unwrap();
    assert!(!root.join(&fleet.name).join(&a).exists());
}

#[test]
fn a_failed_start_leaves_no_agent_behind() {
    let Some(root) = capable_host() else { return };
    let fleet = Fleet::new(&root, "fail");
    let mut sup = fleet.open();
    let mut launch = fleet.launch("a");
    // Without the keeper binary's grant, the view does not contain it, so
    // the helper cannot start the keeper.
    launch.plan = {
        let (manifest, _) = devcroft::config::parse(
            "[sandbox]\nname = \"fleet-sup\"\n\n[env]\nprovider = \"flox\"\n",
        )
        .unwrap();
        devcroft::policy::compile(&manifest)
            .with_provider_grants("flox", &["/usr".to_owned()])
            .to_capability_plan()
    };
    let err = sup.start(&launch).unwrap_err().to_string();
    assert!(err.contains("starting agent a1"), "{err}");
    assert!(err.contains("start "), "the helper's step is named: {err}");

    // Checked before anything reconciles: a leftover directory without a
    // record would otherwise be removed by `list` and hide a start that
    // did not clean up after itself.
    assert!(!fleet.state().join("agents").join("a1").exists());
    assert!(!root.join(&fleet.name).join("a1").exists());
    assert!(sup.list().unwrap().is_empty());
}

/// `fleet::project::prepare` into `Supervisor::start`: an agent whose
/// launch came from a manifest and a provider, the way `devcroft fleet up`
/// builds one. The provider's activation script and the manifest's hooks
/// run inside the agent, in its own workspace, in `up`'s order.
#[test]
fn a_prepared_agent_runs_its_hooks_inside_itself() {
    use devcroft::provider::{ProviderEntry, ProviderError, Resolution, ServiceSupport, Tier};

    struct HostUsr;
    impl ProviderEntry for HostUsr {
        fn resolve(&self, _: &Path) -> Result<Resolution, ProviderError> {
            Ok(Resolution {
                env: [("PATH".to_owned(), "/usr/bin".to_owned())].into(),
                unset: Vec::new(),
                read_only_grants: vec!["/usr".to_owned()],
                activation_script: Some("echo activation >> hooks.log".to_owned()),
                services: ServiceSupport::Unsupported,
                ran_activation_hook: false,
            })
        }
        fn fingerprint(&self, _: &Path) -> Result<String, ProviderError> {
            Ok(String::new())
        }
        fn tier(&self) -> Tier {
            Tier::Closure
        }
        fn static_name(&self) -> &'static str {
            "nix"
        }
    }

    let Some(root) = capable_host() else { return };
    let fleet = Fleet::new(&root, "prepared");
    let template = fleet.launch("a");
    let (manifest, _) = devcroft::config::parse(
        "[sandbox]\nname = \"fleet-sup\"\n\n[env]\nprovider = \"nix\"\n\n\
         [hooks]\npost_create = \"echo post_create $(cat /proc/sys/kernel/hostname) >> hooks.log\"\n\
         post_start = \"echo post_start >> hooks.log\"\n\n\
         [filesystem]\nread = [\"/proc\"]\n",
    )
    .unwrap();
    let launch = devcroft::fleet::project::prepare(
        &HostUsr,
        &manifest,
        &template.workspace,
        Path::new(env!("CARGO_BIN_EXE_devcroft")),
        &template.authorized_key_pem,
        Limits::default(),
        true,
    )
    .unwrap();
    let mut sup = fleet.open();
    let id = sup.start(&launch).unwrap();

    // The keeper runs the hooks before it accepts sessions, so once a
    // session answers they have run.
    let (code, _) = session(&sup.control_socket(&id), &launch.workspace, "true");
    assert_eq!(code, Some(0));
    let log = std::fs::read_to_string(launch.workspace.join("hooks.log")).unwrap();
    assert_eq!(
        log,
        format!("activation\npost_create {id}\npost_start\n"),
        "hooks ran out of order, or outside the agent"
    );
    sup.stop(&id).unwrap();
}

/// Section 1's "why something died": an agent over its memory limit is
/// OOM-killed whole (`memory.oom.group`), and its record says so after the
/// leaf, and the counters in it, are gone.
#[test]
fn an_oom_killed_agent_is_retired_with_the_reason() {
    let Some(root) = capable_host() else { return };
    if !Path::new("/usr/bin/python3").exists() {
        eprintln!("skipping: no /usr/bin/python3 to allocate with");
        return;
    }
    let fleet = Fleet::new(&root, "oom");
    let mut launch = fleet.launch("a");
    launch.limits.memory_max = Some(64 << 20);
    let mut sup = fleet.open();
    let id = sup.start(&launch).unwrap();

    // 256 MiB in a 64 MiB agent. The session dies with the agent, so its
    // connection just closes; nothing here waits for an Exit frame.
    let mut stream = UnixStream::connect(sup.control_socket(&id)).unwrap();
    protocol::write_frame(
        &mut stream,
        &Frame::Spawn(SpawnRequest {
            cmd: "/usr/bin/python3".into(),
            args: vec!["-c".into(), "b = bytearray(256 << 20)".into()],
            cwd: launch.workspace.to_string_lossy().into_owned(),
            env: Default::default(),
            pty: None,
        }),
    )
    .unwrap();
    while protocol::read_frame(&mut stream).is_ok() {}

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        let s = sup.inspect(&id).unwrap();
        if s.record.state == AgentState::Stopped {
            break s;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the agent was never retired"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let evidence = status.record.evidence.expect("an OOM kill leaves evidence");
    assert!(evidence.oom_group_kills >= 1, "{evidence:?}");
    // And a clean stop leaves none.
    let other = sup.start(&fleet.launch("b")).unwrap();
    sup.stop(&other).unwrap();
    assert_eq!(sup.inspect(&other).unwrap().record.evidence, None);
}
