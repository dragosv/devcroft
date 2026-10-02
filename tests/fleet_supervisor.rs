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
            egress_allow: Vec::new(),
            services: Vec::new(),
            expose: Vec::new(),
            timeout: None,
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
        "a1",
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

/// A host-side HTTP server answering `200 ok` on `addr`, until the test
/// ends. `None` where the address cannot be bound (no 127/8 alias).
fn serve_ok(addr: &str) -> Option<u16> {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind((addr, 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });
    Some(port)
}

/// Group 3: each agent's egress goes through its own proxy, to its own
/// allowlist, and nowhere else. The destinations are on 127.0.0.3/.4, which
/// `NO_PROXY` does not exempt, so the test needs no internet.
#[test]
fn each_agent_reaches_only_its_own_allowlist_through_its_own_proxy() {
    let Some(root) = capable_host() else { return };
    if !Path::new("/usr/bin/curl").exists() {
        eprintln!("skipping: no /usr/bin/curl");
        return;
    }
    let (Some(p3), Some(p4)) = (serve_ok("127.0.0.3"), serve_ok("127.0.0.4")) else {
        eprintln!("skipping: no 127.0.0.3/127.0.0.4 loopback aliases");
        return;
    };
    let fleet = Fleet::new(&root, "egress");
    let (mut la, mut lb) = (fleet.launch("a"), fleet.launch("b"));
    la.egress_allow = vec!["127.0.0.3".into()];
    lb.egress_allow = vec!["127.0.0.4".into()];
    let mut sup = fleet.open();
    let a = sup.start(&la).unwrap();
    let b = sup.start(&lb).unwrap();

    let (sock_a, sock_b) = (sup.control_socket(&a), sup.control_socket(&b));
    let code = |socket: &Path, launch: &AgentLaunch, extra: &str, url: String| {
        let (_, out) = session(
            socket,
            &launch.workspace,
            &format!("curl -s -o /dev/null --max-time 5 -w '%{{http_code}}' {extra} {url}"),
        );
        out.trim().to_owned()
    };
    let (to3, to4) = (
        format!("http://127.0.0.3:{p3}/"),
        format!("http://127.0.0.4:{p4}/"),
    );

    // A session inherits nothing of the keeper's: not its control or SSH
    // listener (which would let project code accept a later `devcroft exec`
    // meant for the keeper), not the relay, not the relay's fd number.
    let (_, fds) = session(
        &sock_a,
        &la.workspace,
        "for f in /proc/$$/fd/*; do readlink $f || :; done; \
         echo relay-var=$(env | grep -c DEVCROFT_PROXY_RELAY_FD)",
    );
    assert!(
        !fds.contains("socket:"),
        "a session inherited a socket: {fds}"
    );
    assert!(fds.contains("relay-var=0"), "{fds}");
    // Each agent reaches its own allowed host, through its proxy.
    assert_eq!(code(&sock_a, &la, "", to3.clone()), "200");
    assert_eq!(code(&sock_b, &lb, "", to4.clone()), "200");
    // And not the other's: B is refused what only A allows, and vice versa.
    assert_ne!(code(&sock_b, &lb, "", to3.clone()), "200");
    assert_ne!(code(&sock_a, &la, "", to4.clone()), "200");
    // Around the proxy there is nothing: the namespace has no route out,
    // and 127.0.0.3 inside it is the agent's own loopback.
    assert_eq!(code(&sock_a, &la, "--noproxy '*'", to3.clone()), "000");

    // Each proxy logged its own agent's requests.
    let log = |id: &str| {
        std::fs::read_to_string(fleet.state().join("agents").join(id).join("egress.log")).unwrap()
    };
    assert!(log(&a).contains(&format!("port={p4}")), "{}", log(&a));
    assert!(log(&b).contains(&format!("port={p3}")), "{}", log(&b));
    // And every record names its agent, so it attributes the request
    // wherever the line ends up.
    for id in [&a, &b] {
        let text = log(id);
        assert!(
            text.lines().all(|l| l.starts_with(&format!("agent={id} "))),
            "{text}"
        );
    }

    // Stopping an agent takes its proxy with it.
    sup.stop(&a).unwrap();
    assert!(!root.join(&fleet.name).join(format!("{a}-host")).exists());
    assert_eq!(
        code(&sock_b, &lb, "", to4),
        "200",
        "B's proxy is independent of A's"
    );
    assert_eq!(sup.inspect(&b).unwrap().record.egress, ["127.0.0.4"]);
    sup.stop(&b).unwrap();
}

/// Whether `leaf` holds any process: `cgroup.events` says `populated 1`.
fn populated(leaf: &Path) -> bool {
    std::fs::read_to_string(leaf.join("cgroup.events"))
        .map(|e| e.lines().any(|l| l == "populated 1"))
        .unwrap_or(false)
}

/// Poll `done` for up to five seconds.
fn eventually(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..500 {
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

/// `service-ports`: "a mapping SHALL NOT outlive the agent it belongs to",
/// for an agent that crashes rather than being stopped. **No command runs
/// after the crash**: the helpers used to die only when a later `ls` or
/// `stop` reconciled, so until then the host port stayed allocated to
/// nothing. The crash is a `cgroup.kill` of the agent's leaf from outside,
/// which the supervisor does not see.
#[test]
fn a_crashed_agents_host_port_and_proxy_go_with_it_before_any_command() {
    let Some(root) = capable_host() else { return };
    let fleet = Fleet::new(&root, "lifeline");
    let mut launch = fleet.launch("a");
    launch.egress_allow = vec!["127.0.0.3".into()];
    launch.expose = vec![("web".into(), 8000)];
    let mut sup = fleet.open();
    let id = sup.start(&launch).unwrap();

    let record = sup.inspect(&id).unwrap().record;
    let [mapping] = &record.port_mappings[..] else {
        panic!("{:?}", record.port_mappings)
    };
    let node = root.join(&fleet.name);
    let host_leaf = node.join(format!("{id}-host"));
    let proxy = fleet.state().join("agents").join(&id).join("proxy.sock");
    // The control: while the agent runs, its helpers do.
    assert!(populated(&host_leaf));
    assert!(std::net::TcpListener::bind(("127.0.0.1", mapping.host)).is_err());
    assert!(UnixStream::connect(&proxy).is_ok());

    std::fs::write(node.join(&id).join("cgroup.kill"), "1").unwrap();

    assert!(
        eventually(|| !populated(&host_leaf)),
        "the agent's helpers outlived it"
    );
    // Released means bindable, not merely refusing: a forwarder left
    // running with nothing behind it refuses too.
    assert!(
        eventually(|| std::net::TcpListener::bind(("127.0.0.1", mapping.host)).is_ok()),
        "a crashed agent's host port is still held"
    );
    assert!(UnixStream::connect(&proxy).is_err());
    // And the next command still reconciles the record.
    assert_eq!(sup.inspect(&id).unwrap().record.state, AgentState::Stopped);
}

/// The other half of the lifeline: a supervisor that dies after starting an
/// agent's helpers and before starting the agent (so before any record
/// exists) holds the only write end, and its death must take the helpers
/// with it. Dropping the write end is what the kernel does for a process
/// that dies, so this is that crash without killing the test.
#[test]
fn a_helper_whose_last_writer_is_gone_exits() {
    let dir = std::env::temp_dir().join(format!("devcroft-lifeline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("proxy.sock");
    let line = devcroft::fleet::lifeline::Lifeline::new().unwrap();
    let (pid, _, _) = devcroft::proxy::spawn_at(
        Path::new(env!("CARGO_BIN_EXE_devcroft")),
        &socket,
        &dir.join("egress.log"),
        &["127.0.0.3".to_owned()],
        |cmd| {
            devcroft::fleet::lifeline::attach(cmd, &line.read);
            Ok(())
        },
    )
    .unwrap();
    let reaped = || {
        let mut status = 0;
        // SAFETY: our own child; a valid out-pointer.
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) == pid }
    };

    // The control: with a writer alive, the helper stays up.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!reaped(), "the helper exited with its lifeline still held");
    assert!(UnixStream::connect(&socket).is_ok());

    drop(line);
    assert!(eventually(reaped), "the helper outlived its last writer");
    let _ = std::fs::remove_dir_all(&dir);
}

/// IDs are never reused, including the highest one after it is removed:
/// the first version scanned the agent directories, so removing `a2` and
/// starting another agent handed `a2` out again, and an old `a2` address
/// named a different workspace.
#[test]
fn a_removed_agents_id_is_not_handed_out_again() {
    let Some(root) = capable_host() else { return };
    let fleet = Fleet::new(&root, "ids");
    let mut sup = fleet.open();
    assert_eq!(sup.start(&fleet.launch("1")).unwrap(), "a1");
    assert_eq!(sup.start(&fleet.launch("2")).unwrap(), "a2");
    sup.stop("a2").unwrap();
    sup.remove("a2").unwrap();
    assert_eq!(sup.start(&fleet.launch("3")).unwrap(), "a3");

    // And across a supervisor restart, which has only what is on disk.
    sup.stop("a3").unwrap();
    sup.remove("a3").unwrap();
    drop(sup);
    let mut sup = fleet.open();
    assert_eq!(sup.start(&fleet.launch("4")).unwrap(), "a4");
    for id in ["a1", "a4"] {
        sup.stop(id).unwrap();
    }
}

/// Reconcile sweeps every leaf no running agent owns, not only those with an
/// agent directory: a supervisor that crashed between creating a leaf and
/// its directory leaves one with none, and a stop whose `cgroup.kill` did
/// not drain leaves one behind a Stopped record. Each stray here holds a
/// live process, which is what makes leaving it a leak.
#[test]
fn leaves_no_running_agent_owns_are_swept_and_their_processes_killed() {
    let Some(root) = capable_host() else { return };
    let fleet = Fleet::new(&root, "sweep");
    let mut sup = fleet.open();
    let stopped = sup.start(&fleet.launch("1")).unwrap();
    let running = sup.start(&fleet.launch("2")).unwrap();
    sup.stop(&stopped).unwrap();
    drop(sup);

    let node = devcroft::fleet::cgroup::FleetNode::create(&root, &fleet.name).unwrap();
    let mut strays = Vec::new();
    for name in ["a9", "a9-host", stopped.as_str()] {
        let leaf = node.create_leaf(name, &Limits::default()).unwrap();
        let mut sleeper = Command::new("sleep");
        sleeper.arg("60");
        leaf.attach_on_spawn(&mut sleeper).unwrap();
        strays.push((name.to_owned(), sleeper.spawn().unwrap()));
    }

    let mut sup = fleet.open();
    for (name, mut child) in strays {
        assert!(
            !root.join(&fleet.name).join(&name).exists(),
            "leaf {name} survived reconcile"
        );
        assert!(
            eventually(|| matches!(child.try_wait(), Ok(Some(_)))),
            "the process in {name} survived"
        );
    }
    // The running agent and its leaf are untouched.
    assert!(root.join(&fleet.name).join(&running).is_dir());
    assert_eq!(
        sup.inspect(&running).unwrap().record.state,
        AgentState::Running
    );
    sup.stop(&running).unwrap();
}

/// Section 1: a runaway build in one agent leaves the others schedulable.
/// A runs a CPU hog per core and then tries to fork 200 processes, under
/// `pids.max = 32` and 64 MiB; B, with no limits, must still be able to
/// fork 50 processes of its own and answer within seconds. The cap is A's
/// leaf's alone: a limit that landed on the fleet node, or nowhere, fails
/// one side or the other. CPU share is deliberately not asserted here
/// (`cpu.weight` is written and read back by `fleet_cgroup`): a timing
/// ratio under a parallel test run is a flaky test, not a measurement.
#[test]
fn a_runaway_agent_leaves_its_sibling_schedulable() {
    let Some(root) = capable_host() else { return };
    let fleet = Fleet::new(&root, "runaway");
    let mut la = fleet.launch("a");
    la.limits.pids_max = Some(32);
    la.limits.memory_max = Some(64 << 20);
    let lb = fleet.launch("b");
    let mut sup = fleet.open();
    let a = sup.start(&la).unwrap();
    let b = sup.start(&lb).unwrap();

    let (code, out) = session(
        &sup.control_socket(&a),
        &la.workspace,
        "nohup sh -c 'for c in $(seq $(nproc)); do (while :; do :; done) & done; \
                      i=0; while [ $i -lt 200 ]; do sleep 120 & i=$((i+1)); done; wait' \
            >/dev/null 2>&1 & echo started",
    );
    assert_eq!((code, out.trim()), (Some(0), "started"), "{out}");
    let events = root.join(&fleet.name).join(&a).join("pids.events");
    assert!(
        eventually(|| std::fs::read_to_string(&events)
            .is_ok_and(|e| e.lines().any(|l| l.starts_with("max ") && l != "max 0"))),
        "A was never refused a fork, so it was not the runaway this test needs"
    );

    let begun = std::time::Instant::now();
    let (code, out) = session(
        &sup.control_socket(&b),
        &lb.workspace,
        // Redirected, or the session would wait for them to close its
        // output. A fork that fails aborts `sh`, so `forked=50` prints only
        // if all fifty succeeded.
        "i=0; while [ $i -lt 50 ]; do sleep 120 >/dev/null 2>&1 & i=$((i+1)); done; \
         echo forked=$i",
    );
    let took = begun.elapsed();
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(
        out.trim(),
        "forked=50",
        "B could not fork its own processes: {out}"
    );
    assert!(
        took < std::time::Duration::from_secs(10),
        "B took {took:?} to fork 50 processes beside a runaway"
    );

    sup.stop(&a).unwrap();
    sup.stop(&b).unwrap();
}

/// `fleet up --timeout`: the agent ends on its own at its deadline, with no
/// command running, and the next command records why from PID 1's report.
/// A `stop` records no exit at all, so the two stay distinguishable.
#[test]
fn an_agent_past_its_deadline_ends_and_is_recorded_as_timed_out() {
    let Some(root) = capable_host() else { return };
    let fleet = Fleet::new(&root, "deadline");
    let mut timed = fleet.launch("t");
    timed.timeout = Some(std::time::Duration::from_secs(1));
    let mut sup = fleet.open();
    let t = sup.start(&timed).unwrap();
    let s = sup.start(&fleet.launch("s")).unwrap();
    assert_eq!(sup.inspect(&t).unwrap().record.timeout_secs, Some(1));

    let leaf = root.join(&fleet.name).join(&t);
    assert!(
        eventually(|| !populated(&leaf)),
        "the agent outlived its deadline"
    );
    let record = sup.inspect(&t).unwrap().record;
    assert_eq!(record.state, AgentState::Stopped);
    assert_eq!(record.exit.map(|e| e.timed_out), Some(true), "{record:?}");

    // The other agent has no deadline and is still running; stopped, it
    // records no exit, because PID 1 was killed rather than seeing an end.
    assert_eq!(sup.inspect(&s).unwrap().record.state, AgentState::Running);
    sup.stop(&s).unwrap();
    assert_eq!(sup.inspect(&s).unwrap().record.exit, None);
}
