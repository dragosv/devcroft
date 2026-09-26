//! `add-linux-agent-fleet` D6: per-agent cgroup leaves, against a real
//! delegated cgroup v2 subtree.
//!
//! Each test asserts what the kernel *did* (a process killed, a fork
//! refused, a leaf emptied), not that a limit file holds a value. A limit
//! that is written and never enforced is exactly what D6 exists to rule
//! out, and on a host with swap, `memory.max` alone is that case.
//!
//! **Opt-in, because the tests write into a cgroup tree.** They run only
//! when `DEVCROFT_TEST_CGROUP_ROOT` names a delegated cgroup and this
//! process already sits below it: moving a process into a leaf needs write
//! access to the common ancestor's `cgroup.procs`. Deriving the root
//! instead ("my cgroup's parent") would, on a systemd desktop, create
//! cgroups inside the user manager's `app.slice` behind its back. In the
//! devcontainer the variable is set, and the missing step is:
//!
//! ```sh
//! sudo cgroup-delegate enter $$    # once per shell, then cargo test
//! ```

#![cfg(target_os = "linux")]

use devcroft::fleet::cgroup::{self, Degraded, FleetNode, Limits, Teardown};
use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MIB: u64 = 1 << 20;

/// Set in the environment of the re-exec'd test binary that plays the
/// memory hog; see [`memory_hog_helper`].
const HOG_ENV: &str = "DEVCROFT_TEST_CGROUP_HOG";

/// The delegated root, or a printed reason to skip. The gate asks less
/// than the tests assert: it checks only that the root is named and that
/// this process is inside it, never that `FleetNode::create` succeeds, so
/// a broken preflight fails the tests rather than skipping them.
fn delegated_root() -> Option<PathBuf> {
    let Some(root) = std::env::var_os("DEVCROFT_TEST_CGROUP_ROOT").map(PathBuf::from) else {
        eprintln!(
            "skipping: DEVCROFT_TEST_CGROUP_ROOT is not set (no delegated cgroup to test in)"
        );
        return None;
    };
    let own = cgroup::own_cgroup().ok()?;
    if !own.starts_with(&root) {
        eprintln!(
            "skipping: this process is in {}, outside {}; run `sudo cgroup-delegate enter $$` \
             in the shell that runs cargo test",
            own.display(),
            root.display()
        );
        return None;
    }
    Some(root)
}

/// A per-test fleet node, emptied and removed even when an assertion
/// fails, so a red run does not leave cgroups for the next one to trip on.
struct Scratch(FleetNode);

impl Scratch {
    fn new(root: &Path, tag: &str) -> Scratch {
        let name = format!("devcroft-test-{}-{tag}", std::process::id());
        Scratch(FleetNode::create(root, &name).expect("preflight on the delegated root"))
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

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap().trim().to_owned()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Not a test: the process that allocates past its leaf's limit. Runs
/// only when re-exec'd with [`HOG_ENV`] set, and is a no-op otherwise.
#[test]
fn memory_hog_helper() {
    let Some(mib) = std::env::var(HOG_ENV)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    else {
        return;
    };
    let mut blocks = Vec::new();
    for _ in 0..mib {
        // Written, not just reserved: untouched pages are never charged.
        blocks.push(vec![1u8; MIB as usize]);
    }
    std::hint::black_box(&blocks);
}

#[test]
fn a_leaf_carries_its_limits_and_starts_with_its_processes_inside() {
    let Some(root) = delegated_root() else { return };
    let node = Scratch::new(&root, "limits");
    let limits = Limits {
        memory_max: Some(64 * MIB),
        cpu_weight: Some(50),
        io_weight: Some(50),
        pids_max: Some(32),
    };
    let leaf = node.0.create_leaf("agent", &limits).unwrap();

    assert_eq!(
        read(&leaf.path().join("memory.max")),
        (64 * MIB).to_string()
    );
    assert_eq!(read(&leaf.path().join("memory.swap.max")), "0");
    assert_eq!(read(&leaf.path().join("memory.oom.group")), "1");
    assert_eq!(read(&leaf.path().join("cpu.weight")), "50");
    assert_eq!(read(&leaf.path().join("pids.max")), "32");
    assert_eq!(read(&leaf.path().join("cgroup.type")), "domain");

    // io.weight is reported degraded exactly when the leaf lacks the file,
    // whatever the `io` controller says.
    let has_io_weight = leaf.path().join("io.weight").exists();
    assert_eq!(
        !has_io_weight,
        node.0.degraded().contains(&Degraded::IoWeight)
    );
    if has_io_weight {
        assert_eq!(read(&leaf.path().join("io.weight")), "default 50");
    }

    // The program itself, not just its parent, reports the leaf.
    let mut cmd = Command::new("cat");
    cmd.arg("/proc/self/cgroup").stdout(Stdio::piped());
    leaf.attach_on_spawn(&mut cmd).unwrap();
    let out = cmd.output().unwrap();
    let rel = leaf.path().strip_prefix("/sys/fs/cgroup").unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("0::/{}", rel.display())
    );

    assert!(leaf.exit_evidence().unwrap().is_quiet());
    assert_eq!(leaf.kill().unwrap(), Teardown::Removed);
}

#[test]
fn the_leaf_fd_does_not_survive_exec() {
    let Some(root) = delegated_root() else { return };
    let node = Scratch::new(&root, "cloexec");
    let leaf = node.0.create_leaf("agent", &Limits::default()).unwrap();
    let fd = leaf.open_dir().unwrap();
    // SAFETY: F_GETFD on an fd this test owns.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
    assert!(
        flags & libc::FD_CLOEXEC != 0,
        "a leaked leaf fd reaches siblings through `..`"
    );
    drop(fd);
    assert_eq!(leaf.kill().unwrap(), Teardown::Removed);
}

#[test]
fn over_its_memory_limit_the_whole_leaf_dies_and_says_why() {
    let Some(root) = delegated_root() else { return };
    let node = Scratch::new(&root, "oom");
    let limits = Limits {
        memory_max: Some(64 * MIB),
        ..Limits::default()
    };
    let leaf = node.0.create_leaf("agent", &limits).unwrap();

    let mut sibling = Command::new("sleep");
    sibling.arg("60");
    leaf.attach_on_spawn(&mut sibling).unwrap();
    let mut sibling = sibling.spawn().unwrap();

    // 256 MiB into a 64 MiB leaf. On a host with swap, this only fails
    // because memory.swap.max is 0.
    let mut hog = Command::new(std::env::current_exe().unwrap());
    hog.args([
        "--exact",
        "memory_hog_helper",
        "--test-threads=1",
        "--quiet",
    ])
    .env(HOG_ENV, "256")
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    leaf.attach_on_spawn(&mut hog).unwrap();
    let status = hog.status().unwrap();
    assert_eq!(
        status.signal(),
        Some(libc::SIGKILL),
        "the hog was not killed: {status:?}"
    );

    // memory.oom.group: the bystander goes with it.
    let sibling_status = sibling.wait().unwrap();
    assert_eq!(sibling_status.signal(), Some(libc::SIGKILL));

    let evidence = leaf.exit_evidence().unwrap();
    assert!(evidence.oom_group_kills >= 1, "{evidence:?}");
    assert_eq!(leaf.kill().unwrap(), Teardown::Removed);
}

#[test]
fn pids_max_refuses_forks_and_says_so() {
    let Some(root) = delegated_root() else { return };
    let node = Scratch::new(&root, "pids");
    let limits = Limits {
        pids_max: Some(8),
        ..Limits::default()
    };
    let leaf = node.0.create_leaf("agent", &limits).unwrap();

    let mut cmd = Command::new("sh");
    cmd.args(["-c", "for i in $(seq 20); do sleep 30 & done; exit 0"])
        .stderr(Stdio::null());
    leaf.attach_on_spawn(&mut cmd).unwrap();
    cmd.status().unwrap();

    let evidence = leaf.exit_evidence().unwrap();
    assert!(evidence.forks_denied > 0, "{evidence:?}");
    assert_eq!(evidence.oom_kills, 0);
    assert_eq!(leaf.kill().unwrap(), Teardown::Removed);
}

#[test]
fn kill_reaches_detached_orphans_that_ignore_sigterm() {
    let Some(root) = delegated_root() else { return };
    let node = Scratch::new(&root, "kill");
    let leaf = node.0.create_leaf("agent", &Limits::default()).unwrap();

    // Five daemons in their own sessions, reparented away from anything
    // this test can wait on, each ignoring SIGTERM. A process-tree kill
    // finds none of them.
    let mut cmd = Command::new("sh");
    cmd.args([
        "-c",
        "for i in 1 2 3 4 5; do setsid sh -c \"trap '' TERM; exec sleep 300\" & done; exit 0",
    ]);
    leaf.attach_on_spawn(&mut cmd).unwrap();
    assert!(cmd.status().unwrap().success());

    let procs = leaf.path().join("cgroup.procs");
    wait_until("five orphans are in the leaf", || {
        read(&procs).lines().count() == 5
    });
    let pids: Vec<String> = read(&procs).lines().map(str::to_owned).collect();

    assert_eq!(leaf.kill().unwrap(), Teardown::Removed);
    for pid in pids {
        // Gone, or a zombie waiting on a reaper outside this test.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let state = stat.rsplit_once(") ").map(|(_, rest)| &rest[..1]);
        assert!(matches!(state, None | Some("Z")), "{pid} survived: {stat}");
    }
}
