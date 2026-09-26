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
use devcroft::fleet::init::{self, Agent, AgentSpec, Stdio};
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

fn spec(script: &str) -> AgentSpec {
    AgentSpec {
        command: vec!["sh".into(), "-c".into(), script.into()],
        env: vec![("PATH".into(), std::env::var("PATH").unwrap())],
        cwd: None,
        hostname: "agent".into(),
    }
}

/// Start an agent with its stdout on a pipe.
fn start(leaf: &Leaf, spec: &AgentSpec) -> (Agent, std::io::PipeReader) {
    let (r, w) = std::io::pipe().unwrap();
    let stdio = Stdio {
        stdout: Some(w.into()),
        stderr: None,
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
    let (code, out) = run(
        &leaf,
        &spec(
            "echo pid=$$; \
             cat /proc/self/cgroup; \
             cat /proc/sys/kernel/hostname; \
             echo procs=$(ls /proc | grep -c '^[0-9]'); \
             grep CapEff /proc/self/status; \
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
    // No cgroup fd, config pipe or status pipe survived into the agent.
    // (The glob also lists the fd sh read the directory through, which is
    // closed by the time readlink runs, hence the `|| :`.)
    for fd in &lines[5..] {
        assert!(
            !fd.contains("cgroup"),
            "a cgroup fd leaked into the agent: {out}"
        );
    }
    assert!(lines.len() <= 5 + 4, "unexpected fds in the agent: {out}");
    assert_eq!(leaf.kill().unwrap(), Teardown::Removed);
}

#[test]
fn agents_cannot_see_or_signal_each_other() {
    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "pair");
    let (b_leaf, a_leaf) = (node.leaf("b"), node.leaf("a"));
    let (b, mut b_out) = start(&b_leaf, &spec("echo up; exec sleep 30"));
    let mut line = String::new();
    BufReader::new(&mut b_out).read_line(&mut line).unwrap();

    let (code, out) = run(
        &a_leaf,
        &spec(&format!(
            "cat /proc/[0-9]*/comm; if kill -0 {} 2>/dev/null; then echo SIGNALLED; fi",
            b.pid()
        )),
    );
    assert_eq!(code, 0, "{out}");
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
    let (code, _) = run(&node.leaf("a"), &spec("exit 7"));
    assert_eq!(code, 7);
}

#[test]
fn sigterm_reaches_the_command_through_pid_1() {
    let Some(root) = capable_host() else { return };
    let node = Scratch::new(&root, "term");
    let leaf = node.leaf("a");
    let (agent, out) = start(
        &leaf,
        &spec("trap 'exit 42' TERM; echo ready; while :; do sleep 0.05; done"),
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
    // Two background processes outlive the command, then the command exits.
    let (code, _) = run(&leaf, &spec("sleep 300 & sleep 300 & exit 0"));
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
    let mut bad = spec("true");
    bad.hostname = "x".repeat(100); // over HOST_NAME_MAX
    let err = init::spawn(
        Path::new(env!("CARGO_BIN_EXE_devcroft")),
        &leaf,
        &bad,
        Stdio::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("sethostname"), "{err}");
    wait_until("the leaf is empty", || !leaf.populated().unwrap());

    bad = spec("true");
    bad.command = vec!["/nonexistent/agent".into()];
    let err = init::spawn(
        Path::new(env!("CARGO_BIN_EXE_devcroft")),
        &leaf,
        &bad,
        Stdio::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("start \"/nonexistent/agent\""), "{err}");
}
