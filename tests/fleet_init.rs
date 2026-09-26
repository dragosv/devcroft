//! `add-linux-agent-fleet` D2: the init helper, as an agent sees it.
//!
//! Each test starts real agents through `fleet::init::spawn` (clone3
//! into fresh namespaces and a cgroup leaf, then `__fleet_init` as PID 1)
//! and asserts from *inside* the agent: what it can see, what it can
//! signal, what capabilities it holds, which fds it holds.
//!
//! Two gates, both asking less than the tests assert. A delegated cgroup
//! root, the same opt-in as `tests/fleet_cgroup.rs`. And a host that can
//! mount a fresh procfs in a user+PID namespace, checked with util-linux's
//! `unshare --mount-proc`, an independent implementation. Docker's
//! default /proc masking refuses that (EPERM), which is why the
//! devcontainer runs with `systempaths=unconfined`.

#![cfg(target_os = "linux")]

use devcroft::fleet::cgroup::{FleetNode, Leaf, Limits, Teardown};
use devcroft::fleet::init::{self, Agent, AgentSpec, Stdio, View};
use devcroft::policy::CapabilityPlan;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

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
        eprintln!(
            "skipping: this host cannot mount a fresh /proc in a user+PID namespace \
             (Docker's /proc masking; the devcontainer needs systempaths=unconfined)"
        );
        return None;
    }
    Some(root)
}

/// A per-test fleet node, emptied and removed even when a test fails.
struct Scratch(FleetNode);

impl Scratch {
    fn new(root: &Path, tag: &str) -> Scratch {
        let name = format!("devcroft-init-test-{}-{tag}", std::process::id());
        Scratch(FleetNode::create(root, &name).expect("preflight on the delegated root"))
    }
    fn leaf(&self, name: &str) -> Leaf {
        self.0.create_leaf(name, &Limits::default()).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let Ok(entries) = std::fs::read_dir(self.0.path()) else {
            return;
        };
        for leaf in entries.flatten().filter(|e| e.path().is_dir()) {
            let _ = std::fs::write(leaf.path().join("cgroup.kill"), "1");
            std::thread::sleep(Duration::from_millis(50));
            let _ = std::fs::remove_dir(leaf.path());
        }
        let _ = std::fs::remove_dir(self.0.path());
    }
}

/// A plan like `up` compiles for a project whose manifest grants `.` and
/// reads `read`, with the host's `/usr` standing in for a provider closure
/// (merged `/usr` makes that `sh`, `cat` and the loader).
fn plan(read: &[&str]) -> CapabilityPlan {
    let read: Vec<String> = read.iter().map(|r| format!("{r:?}")).collect();
    let manifest = format!(
        "[sandbox]\nname = \"fleet-agent\"\n\n[env]\nprovider = \"flox\"\n\n\
         [filesystem]\nallow = [\".\"]\nread = [{}]\n",
        read.join(", ")
    );
    let (manifest, _) = devcroft::config::parse(&manifest).unwrap();
    devcroft::policy::compile(&manifest)
        .with_provider_grants("flox", &["/usr".to_owned()])
        .to_capability_plan()
}

/// A project directory and an empty view root, removed on drop.
struct Dirs(PathBuf, PathBuf);

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let base = std::env::temp_dir().join(format!("devcroft-init-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (project, view) = (base.join("project"), base.join("view"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&view).unwrap();
        Dirs(project, view)
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

/// `sh -c script` in the project root, on the host's root (no view), with
/// `/proc` readable so the tests can look at the agent from inside it.
/// A restricted process reads nothing under `/proc` without that grant.
fn spec(dirs: &Dirs, script: &str) -> AgentSpec {
    AgentSpec {
        command: vec!["sh".into(), "-c".into(), script.into()],
        env: vec![("PATH".into(), std::env::var("PATH").unwrap())],
        cwd: Some(dirs.0.clone()),
        hostname: "agent".into(),
        plan: plan(&["/proc"]),
        project_root: dirs.0.clone(),
        view: None,
    }
}

/// Start an agent with its stdout on a pipe.
fn start(leaf: &Leaf, spec: &AgentSpec) -> (Agent, std::io::PipeReader) {
    let (r, w) = std::io::pipe().unwrap();
    let stdio = Stdio {
        stdout: Some(w.into()),
        stderr: None,
        inherit: Vec::new(),
    };
    let agent = init::spawn(Path::new(env!("CARGO_BIN_EXE_devcroft")), leaf, spec, stdio).unwrap();
    (agent, r)
}

fn run(leaf: &Leaf, spec: &AgentSpec) -> (i32, String) {
    let (agent, mut out) = start(leaf, spec);
    let mut s = String::new();
    out.read_to_string(&mut s).unwrap();
    let status = agent.wait().unwrap();
    (
        status.code().expect("the helper exits, it is not killed"),
        s,
    )
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn an_agent_is_alone_in_its_namespaces_and_holds_nothing() {
    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "alone");
    let leaf = node.leaf("a");
    let dirs = Dirs::new("alone");
    let (code, out) = run(
        &leaf,
        &spec(
            &dirs,
            "echo pid=$$; \
             cat /proc/self/cgroup; \
             cat /proc/sys/kernel/hostname; \
             echo procs=$(ls /proc | grep -c '^[0-9]'); \
             grep CapEff /proc/self/status; \
             grep NoNewPrivs /proc/1/status; \
             for f in /proc/$$/fd/*; do readlink $f || :; done",
        ),
    );
    assert_eq!(code, 0, "{out}");
    let lines: Vec<&str> = out.lines().collect();
    // The helper is 1; the command is the first thing it starts.
    assert_eq!(lines[0], "pid=2", "{out}");
    // Its cgroup namespace is rooted at its leaf.
    assert_eq!(lines[1], "0::/", "{out}");
    assert_eq!(lines[2], "agent", "{out}");
    // The fresh /proc lists the helper, sh and the pipeline's commands,
    // not the host.
    let procs: usize = lines[3].trim_start_matches("procs=").parse().unwrap();
    assert!(procs <= 5, "{procs} processes visible: {out}");
    // uid 0 in its namespace, and no capabilities (SECBIT_NOROOT).
    assert_eq!(
        lines[4].split_whitespace().nth(1),
        Some("0000000000000000"),
        "{out}"
    );
    // PID 1 applied the ruleset to itself, not only to the command: nono
    // sets no_new_privs as part of restricting.
    assert_eq!(lines[5].split_whitespace().nth(1), Some("1"), "{out}");
    // No cgroup fd, config pipe or status pipe survived into the agent.
    // (The glob also lists the fd sh read the directory through, which is
    // closed by the time readlink runs, hence the `|| :`.)
    for fd in &lines[6..] {
        assert!(
            !fd.contains("cgroup"),
            "a cgroup fd leaked into the agent: {out}"
        );
    }
    assert!(lines.len() <= 6 + 4, "unexpected fds in the agent: {out}");
    assert_eq!(leaf.kill().unwrap(), Teardown::Removed);
}

#[test]
fn on_the_host_root_the_plan_still_refuses_what_it_does_not_grant() {
    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "enforced");
    let leaf = node.leaf("a");
    let dirs = Dirs::new("enforced");
    let repo = env!("CARGO_MANIFEST_DIR");
    let mut s = spec(
        &dirs,
        &format!(
            "test -e {repo}/Cargo.toml && echo REPO_VISIBLE; \
             cat {repo}/Cargo.toml >/dev/null 2>&1 && echo REPO_READ; \
             cat /proc/self/status >/dev/null 2>&1 && echo PROC_READ; \
             echo x > written && echo PROJECT_WRITTEN; \
             true"
        ),
    );
    s.plan = plan(&[]);
    let (code, out) = run(&leaf, &s);
    assert_eq!(code, 0, "{out}");
    // D2a's host-root strategy: an ungranted path stays visible...
    assert!(out.contains("REPO_VISIBLE"), "{out}");
    // ...and is refused by Landlock, like /proc without a grant.
    assert!(
        !out.contains("REPO_READ"),
        "an ungranted path was readable: {out}"
    );
    assert!(
        !out.contains("PROC_READ"),
        "an ungranted /proc was readable: {out}"
    );
    assert!(out.contains("PROJECT_WRITTEN"), "{out}");
}

#[test]
fn agents_cannot_see_or_signal_each_other() {
    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "pair");
    let (b_leaf, a_leaf) = (node.leaf("b"), node.leaf("a"));
    let (b_dirs, a_dirs) = (Dirs::new("pair-b"), Dirs::new("pair-a"));
    let (b, mut b_out) = start(&b_leaf, &spec(&b_dirs, "echo up; exec sleep 30"));
    let mut line = String::new();
    BufReader::new(&mut b_out).read_line(&mut line).unwrap();

    let (code, out) = run(
        &a_leaf,
        &spec(
            &a_dirs,
            &format!(
                "cat /proc/[0-9]*/comm; if kill -0 {} 2>/dev/null; then echo SIGNALLED; fi",
                b.pid()
            ),
        ),
    );
    assert_eq!(code, 0, "{out}");
    // Its own processes are listed, so the loop below checks something.
    assert!(out.lines().any(|l| l == "sh"), "{out}");
    for comm in out.lines() {
        assert!(
            ["devcroft", "sh", "cat"].contains(&comm),
            "agent a sees a process that is not its own: {comm:?} in {out}"
        );
    }
    b.signal(libc::SIGKILL).unwrap();
    b.wait().unwrap();
}

#[test]
fn the_commands_exit_status_is_the_agents() {
    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "exit");
    let dirs = Dirs::new("exit");
    let (code, _) = run(&node.leaf("a"), &spec(&dirs, "exit 7"));
    assert_eq!(code, 7);
}

#[test]
fn sigterm_reaches_the_command_through_pid_1() {
    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "term");
    let leaf = node.leaf("a");
    let dirs = Dirs::new("term");
    let (agent, out) = start(
        &leaf,
        &spec(
            &dirs,
            "trap 'exit 42' TERM; echo ready; while :; do sleep 0.05; done",
        ),
    );
    let mut line = String::new();
    BufReader::new(out).read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "ready");
    // PID 1 ignores signals it has no handler for: without forwarding,
    // this would do nothing and the wait would hang.
    agent.signal(libc::SIGTERM).unwrap();
    assert_eq!(agent.wait().unwrap().code(), Some(42));
}

#[test]
fn the_namespace_ends_with_its_command() {
    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "ends");
    let leaf = node.leaf("a");
    let dirs = Dirs::new("ends");
    // Two background processes outlive the command, then the command exits.
    let (code, _) = run(&leaf, &spec(&dirs, "sleep 300 & sleep 300 & exit 0"));
    assert_eq!(code, 0);
    // PID 1 exited, so the kernel killed the rest of the namespace.
    wait_until("the leaf is empty", || !leaf.populated().unwrap());
    assert_eq!(leaf.kill().unwrap(), Teardown::Removed);
}

#[test]
fn a_failing_step_is_reported_by_name() {
    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "fail");
    let leaf = node.leaf("a");
    let dirs = Dirs::new("fail");
    let exe = Path::new(env!("CARGO_BIN_EXE_devcroft"));

    let mut bad = spec(&dirs, "true");
    bad.hostname = "x".repeat(100); // over HOST_NAME_MAX
    let err = init::spawn(exe, &leaf, &bad, Stdio::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("sethostname"), "{err}");
    wait_until("the leaf is empty", || !leaf.populated().unwrap());

    bad = spec(&dirs, "true");
    bad.command = vec!["/nonexistent/agent".into()];
    let err = init::spawn(exe, &leaf, &bad, Stdio::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("start \"/nonexistent/agent\""), "{err}");
}

#[test]
fn with_a_view_an_agent_sees_exactly_its_grants() {
    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "view");
    let leaf = node.leaf("a");
    let dirs = Dirs::new("view");
    let repo = env!("CARGO_MANIFEST_DIR");
    let mut s = spec(
        &dirs,
        &format!(
            "echo mounts=$(cut -d' ' -f5 /proc/self/mountinfo | tr '\\n' ' '); \
             echo procs=$(ls /proc | grep -c '^[0-9]'); \
             pwd; \
             test -e {repo} && echo REPO_VISIBLE; \
             test -e /etc/passwd && echo PASSWD_VISIBLE; \
             echo x > /tmp/scratch && echo TMP_WRITTEN; \
             echo written > marker"
        ),
    );
    s.view = Some(View {
        root: dirs.1.clone(),
        proxy_socket: None,
    });
    let (code, out) = run(&leaf, &s);
    assert_eq!(code, 0, "{out}");
    let lines: Vec<&str> = out.lines().collect();

    // Every mount in the agent's namespace is one of its plan's grants, or
    // one the view makes for itself (its root, /proc, /tmp and /dev). Read from the agent's own mountinfo: listing `/`
    // is refused, since nothing grants it, so the view cannot be
    // enumerated from inside by walking it.
    let grants: Vec<PathBuf> = s
        .plan
        .resolved_grants(&dirs.0)
        .unwrap()
        .into_iter()
        .map(|g| g.path)
        .collect();
    // `setup_dev`'s minimal /dev (its own nodes and devpts) is the view's,
    // covered by that function's tests rather than by the plan.
    let own = |m: &str| m == "/" || m == "/proc" || m == "/tmp" || m.starts_with("/dev");
    let mounts: Vec<&str> = lines[0]
        .trim_start_matches("mounts=")
        .split_whitespace()
        .collect();
    assert!(mounts.contains(&"/usr"), "{out}");
    for m in &mounts {
        assert!(
            own(m) || grants.iter().any(|g| g == Path::new(m)),
            "{m:?} is mounted in the view but is not a grant: {out}"
        );
    }
    assert!(
        !out.contains("REPO_VISIBLE"),
        "an ungranted path is in the view: {out}"
    );
    assert!(
        !out.contains("PASSWD_VISIBLE"),
        "an ungranted path is in the view: {out}"
    );
    // The view's /proc is the fresh one.
    let procs: usize = lines[1].trim_start_matches("procs=").parse().unwrap();
    assert!(procs <= 5, "{procs} processes visible: {out}");
    assert_eq!(Path::new(lines[2]), dirs.0);
    // The view's /tmp is its own tmpfs, which exists only inside the
    // agent's mount namespace. The baseline's /tmp rule covers it only
    // because the ruleset was built after the view.
    assert!(
        out.contains("TMP_WRITTEN"),
        "the view's private /tmp was refused: {out}"
    );

    // The project root is the host's, writable, and what the agent writes
    // is owned by the real user (the identity map).
    let marker = dirs.0.join("marker");
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "written\n");
    use std::os::unix::fs::MetadataExt;
    assert_eq!(std::fs::metadata(&marker).unwrap().uid(), unsafe {
        libc::getuid()
    });
    // Everything was mounted inside the agent's namespace.
    assert_eq!(std::fs::read_dir(&dirs.1).unwrap().count(), 0);
}

/// The keeper, as fleet will run it: `__keeper` as the agent's command,
/// its control and SSH sockets bound here, by the supervisor, before any
/// restriction exists, and handed in as inherited fds. A session then goes
/// through the ordinary keeper protocol and runs inside the agent.
#[test]
fn the_keeper_runs_as_the_agents_command_and_serves_sessions() {
    use devcroft::keeper::protocol::{self, Frame, SpawnRequest};
    use std::os::unix::net::{UnixListener, UnixStream};

    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "keeper");
    let leaf = node.leaf("a");
    let dirs = Dirs::new("keeper");
    let exe = Path::new(env!("CARGO_BIN_EXE_devcroft"));

    // Bound on the host, before the agent exists: the sockets stay
    // reachable from out here only because they predate its restriction.
    let state = dirs.0.parent().unwrap().join("state");
    std::fs::create_dir(&state).unwrap();
    let control_path = state.join("control.sock");
    let control = UnixListener::bind(&control_path).unwrap();
    let ssh = UnixListener::bind(state.join("ssh.sock")).unwrap();

    // What `up` compiles, including the grant that lets the keeper binary
    // be exec'd inside the view.
    let (manifest, _) = devcroft::config::parse(
        "[sandbox]\nname = \"fleet-keeper\"\n\n[env]\nprovider = \"flox\"\n\n\
         [filesystem]\nallow = [\".\"]\nread = [\"/proc\"]\n",
    )
    .unwrap();
    let plan = devcroft::policy::compile(&manifest)
        .with_provider_grants("flox", &["/usr".to_owned()])
        .with_keeper_exe_grant(exe.parent().unwrap().to_string_lossy().into_owned())
        .to_capability_plan();

    let first = init::FIRST_INHERITED_FD;
    let spec = AgentSpec {
        command: vec![
            exe.to_string_lossy().into_owned(),
            "__keeper".into(),
            first.to_string(),
            (first + 1).to_string(),
        ],
        env: vec![
            ("PATH".into(), std::env::var("PATH").unwrap()),
            (
                "DEVCROFT_CAPABILITY_PLAN".into(),
                serde_json::to_string(&plan).unwrap(),
            ),
            ("DEVCROFT_START_SERVICES".into(), "0".into()),
        ],
        cwd: Some(dirs.0.clone()),
        hostname: "agent".into(),
        plan,
        project_root: dirs.0.clone(),
        view: Some(View {
            root: dirs.1.clone(),
            proxy_socket: None,
        }),
    };
    let (log_r, log_w) = std::io::pipe().unwrap();
    let stdio = Stdio {
        stdout: None,
        stderr: Some(log_w.into()),
        inherit: vec![control.into(), ssh.into()],
    };
    let agent = init::spawn(exe, &leaf, &spec, stdio).unwrap();

    let session = |script: &str| -> (Option<i32>, String) {
        let mut stream = UnixStream::connect(&control_path).unwrap();
        protocol::write_frame(
            &mut stream,
            &Frame::Spawn(SpawnRequest {
                cmd: "sh".into(),
                args: vec!["-c".into(), script.into()],
                cwd: dirs.0.to_string_lossy().into_owned(),
                env: Default::default(),
                pty: None,
            }),
        )
        .unwrap();
        let mut out = String::new();
        loop {
            match protocol::read_frame(&mut stream).unwrap() {
                Frame::SpawnOk { .. } => {}
                Frame::SpawnErr { message } => panic!("keeper refused to spawn: {message}"),
                Frame::Stdout(b) | Frame::Stderr(b) => out.push_str(&String::from_utf8_lossy(&b)),
                Frame::Exit(status) => return (status.code, out),
                _ => {}
            }
        }
    };

    let repo = env!("CARGO_MANIFEST_DIR");
    let (code, out) = session(&format!(
        "cat /proc/self/cgroup; \
         cat /proc/sys/kernel/hostname; \
         cat /proc/[0-9]*/comm | sort | uniq -c | tr -s ' ' | tr '\\n' ';'; echo; \
         test -e {repo}/Cargo.toml && echo REPO_VISIBLE; \
         echo from-a-session > marker; \
         exit 3"
    ));
    assert_eq!(code, Some(3), "{out}");
    let lines: Vec<&str> = out.lines().collect();
    // The session runs in the agent: its leaf, its hostname, its view.
    assert_eq!(lines[0], "0::/", "{out}");
    assert_eq!(lines[1], "agent", "{out}");
    // Its PID namespace holds the helper, the keeper and the session
    // (both `devcroft`), and nothing of the host.
    assert!(lines[2].contains("devcroft"), "{out}");
    for entry in lines[2].split(';').filter(|e| !e.trim().is_empty()) {
        let comm = entry.split_whitespace().last().unwrap();
        assert!(
            ["devcroft", "sh", "cat", "sort", "uniq", "tr"].contains(&comm),
            "a process that is not the agent's: {comm:?} in {out}"
        );
    }
    // The keeper binary's grant is `target/debug`, inside this repository,
    // so the repository's path exists in the view as that grant's parent
    // directories. Its contents do not.
    assert!(!out.contains("REPO_VISIBLE"), "{out}");
    assert_eq!(
        std::fs::read_to_string(dirs.0.join("marker")).unwrap(),
        "from-a-session\n"
    );

    // Stopping the agent takes the keeper and its sessions with it.
    agent.signal(libc::SIGKILL).unwrap();
    agent.wait().unwrap();
    wait_until("the leaf is empty", || !leaf.populated().unwrap());
    assert!(UnixStream::connect(&control_path).is_err());
    drop(log_r);
}
