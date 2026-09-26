//! Per-agent cgroup v2 leaves inside a delegated subtree (D6).
//!
//! Fleet's resource control: one process-free internal node, one domain
//! leaf per agent, limits written into each leaf, and `cgroup.kill` for
//! teardown. What is here is the cgroup half of D6 alone. Finding the
//! delegated subtree is the caller's job: it is systemd's `Delegate=yes`
//! half, which this module deliberately does not guess at. So is choosing
//! which process goes where.
//!
//! Every rule below was measured before it was written down, in the
//! devcontainer's delegated subtree (design.md D6, "Measured"). Two of them
//! are not obvious from the cgroup v2 documentation:
//!
//! - **An available `io` controller does not mean `io.weight` exists.**
//!   Weights need BFQ or iocost; under the `none` scheduler the controller
//!   enables cleanly and the leaf has only `io.max` and `io.stat`. So
//!   availability is decided by the file, and its absence is reported as
//!   [`Degraded::IoWeight`] rather than skipped quietly.
//! - **On a host with swap, `memory.max` alone is not a cap.** With swap
//!   left at `max`, a 512 MB allocation succeeded in a 128 MB leaf: the
//!   excess went to zram and nothing was killed. `memory.swap.max=0` is
//!   what makes the limit real, so a swap host without that file (swap
//!   accounting off) is refused, not degraded.
//!
//! Agents run as the uid that owns every file in the delegated subtree, so
//! file permissions separate nothing between them. What keeps one agent
//! off another's limits is that the other leaf is unreachable: its own
//! cgroup namespace, a mount view without `/sys`, and no leaked cgroup fd.
//! The last was measured to defeat the other two through
//! `openat(fd, "../sibling/memory.max")`, which is why every fd opened here
//! is `O_CLOEXEC`.

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Controllers fleet cannot run without. A host that cannot delegate all
/// three is refused: D6 has no fallback that runs agents unlimited.
pub const REQUIRED_CONTROLLERS: [&str; 3] = ["cpu", "memory", "pids"];

/// `CGROUP2_SUPER_MAGIC` from `<linux/magic.h>`, stable kernel ABI.
const CGROUP2_SUPER_MAGIC: i64 = 0x6367_7270;

/// How long [`Leaf::kill`] waits for the kernel to empty a leaf: 50 polls
/// of 10 ms, the reference's budget. Measured, 20 orphaned daemons that
/// ignored SIGTERM were gone before the first 1 ms poll, so this is
/// generous rather than tight.
const KILL_POLLS: u32 = 50;
const KILL_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Limits for one agent's leaf. `None` leaves the kernel's default, which
/// for every one of these is "unlimited" or the default weight.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Limits {
    /// `memory.max`, in bytes. Setting it also sets `memory.swap.max=0`,
    /// without which it is not a cap on a host with swap.
    pub memory_max: Option<u64>,
    /// `cpu.weight`, 1–10000; the kernel default is 100.
    pub cpu_weight: Option<u32>,
    /// `io.weight`, 1–10000. Written only where the leaf has the file;
    /// see [`Degraded::IoWeight`].
    pub io_weight: Option<u32>,
    /// `pids.max`: processes and threads together, the launcher included.
    pub pids_max: Option<u64>,
}

/// A limit this host cannot apply, named rather than dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degraded {
    /// The `io` controller may be enabled, but leaves have no `io.weight`
    /// file, because the block devices use neither BFQ nor iocost.
    IoWeight,
}

impl std::fmt::Display for Degraded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Degraded::IoWeight => f.write_str(
                "io.weight: this host's block devices use neither BFQ nor iocost, \
                 so leaves have no io.weight file; IO is not weighted between agents",
            ),
        }
    }
}

/// The unified-hierarchy path in `/proc/<pid>/cgroup` content.
///
/// A pure cgroup v2 host has exactly one line, `0::<path>`. Any other line
/// means a v1 or hybrid hierarchy, where the `0::` entry (if present) is
/// not where the controllers are. That is refused rather than half-used.
pub fn parse_unified_path(proc_cgroup: &str) -> io::Result<&str> {
    let mut unified = None;
    for line in proc_cgroup.lines().filter(|l| !l.is_empty()) {
        match line.strip_prefix("0::") {
            Some(path) if unified.is_none() && path.starts_with('/') => unified = Some(path),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!(
                        "not a pure cgroup v2 hierarchy (/proc/self/cgroup has {line:?}); \
                         fleet needs the unified hierarchy alone"
                    ),
                ));
            }
        }
    }
    unified.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "no cgroup v2 entry in /proc/self/cgroup",
        )
    })
}

/// This process's own cgroup directory, under the cgroup2 mount at
/// `/sys/fs/cgroup`.
pub fn own_cgroup() -> io::Result<PathBuf> {
    let content = fs::read_to_string("/proc/self/cgroup")?;
    let rel = parse_unified_path(&content)?;
    Ok(Path::new("/sys/fs/cgroup").join(rel.trim_start_matches('/')))
}

/// Whether `/proc/swaps` lists any active swap device.
pub fn swap_active(proc_swaps: &str) -> bool {
    // First line is the column header.
    proc_swaps.lines().skip(1).any(|l| !l.trim().is_empty())
}

/// The internal `fleet` node: process-free, with the required controllers
/// enabled for its leaves.
#[derive(Debug)]
pub struct FleetNode {
    dir: PathBuf,
    io_weight: bool,
}

impl FleetNode {
    /// Create (or reuse) `<root>/<name>` and enable controllers top-down
    /// through `root` and the node. This is the preflight: every way this
    /// can fail is a host that cannot enforce D6's limits.
    ///
    /// `root` must be a delegated cgroup the caller can write, holding no
    /// processes itself. cgroup v2's no-internal-process rule refuses
    /// domain controllers on a populated cgroup (`EBUSY`, measured), so a
    /// supervisor started in its delegated cgroup moves itself into a
    /// leaf before calling this.
    pub fn create(root: &Path, name: &str) -> io::Result<FleetNode> {
        validate_name(name)?;
        require_cgroup2(root)?;

        let available = read_words(&root.join("cgroup.controllers"))?;
        let missing: Vec<_> = REQUIRED_CONTROLLERS
            .iter()
            .filter(|c| !available.iter().any(|a| a == *c))
            .collect();
        if !missing.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "{} does not have the {missing:?} controller(s) delegated to it \
                     (available: {available:?}); fleet refuses to run agents without limits",
                    root.display()
                ),
            ));
        }
        let mut wanted: Vec<&str> = REQUIRED_CONTROLLERS.to_vec();
        if available.iter().any(|a| a == "io") {
            wanted.push("io");
        }

        let dir = root.join(name);
        match fs::create_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(ctx(e, "create fleet node", &dir)),
        }
        enable_controllers(root, &wanted)?;
        enable_controllers(&dir, &wanted)?;

        // Checked on the node, which has these files exactly when its
        // leaves will: both depend on what the parent enabled.
        let active = swap_active(&fs::read_to_string("/proc/swaps").unwrap_or_default());
        if active && !dir.join("memory.swap.max").exists() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "swap is active but {} has no memory.swap.max (swap accounting is off), \
                     so memory.max cannot stop an agent from swapping past its limit",
                    dir.display()
                ),
            ));
        }

        let io_weight = dir.join("io.weight").exists();
        Ok(FleetNode { dir, io_weight })
    }

    /// The node's directory.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Limits this host cannot apply to any leaf of this node.
    pub fn degraded(&self) -> Vec<Degraded> {
        if self.io_weight {
            Vec::new()
        } else {
            vec![Degraded::IoWeight]
        }
    }

    /// Create a domain leaf for one agent and write its limits. Fails if
    /// the leaf already exists: a leftover belongs to a crashed supervisor
    /// and has to be swept deliberately, not silently adopted.
    pub fn create_leaf(&self, name: &str, limits: &Limits) -> io::Result<Leaf> {
        validate_name(name)?;
        let dir = self.dir.join(name);
        fs::create_dir(&dir).map_err(|e| ctx(e, "create leaf", &dir))?;
        let leaf = Leaf { dir };
        if let Err(e) = leaf.apply(limits, self.io_weight) {
            let _ = fs::remove_dir(&leaf.dir);
            return Err(e);
        }
        Ok(leaf)
    }

    /// A handle to a leaf that already exists, such as one a previous
    /// supervisor created. `None` if there is no such leaf.
    pub fn existing_leaf(&self, name: &str) -> Option<Leaf> {
        validate_name(name).ok()?;
        let dir = self.dir.join(name);
        dir.is_dir().then_some(Leaf { dir })
    }

    /// Remove the node. Fails while any leaf remains.
    pub fn remove(self) -> io::Result<()> {
        fs::remove_dir(&self.dir).map_err(|e| ctx(e, "remove fleet node", &self.dir))
    }
}

/// One agent's leaf.
#[derive(Debug)]
pub struct Leaf {
    dir: PathBuf,
}

/// How [`Leaf::kill`] ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Teardown {
    /// Every process was gone and the directory was removed.
    Removed,
    /// Processes remained after the wait. The directory is left in place
    /// for a later sweep rather than blocking forever.
    LeftBehind(PathBuf),
}

/// The kernel's record of why a leaf's processes died. All zero on a
/// clean run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Evidence {
    /// `memory.events: oom_group_kill`: the whole leaf was OOM-killed.
    pub oom_group_kills: u64,
    /// `memory.events: oom_kill`: processes the OOM killer took.
    pub oom_kills: u64,
    /// `pids.events: max`: forks refused by `pids.max`.
    pub forks_denied: u64,
}

impl Evidence {
    /// True when there is nothing to report.
    pub fn is_quiet(&self) -> bool {
        *self == Evidence::default()
    }
}

impl Leaf {
    /// The leaf's directory.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    fn apply(&self, limits: &Limits, io_weight: bool) -> io::Result<()> {
        if let Some(bytes) = limits.memory_max {
            self.write("memory.swap.max", "0")?;
            self.write("memory.max", &bytes.to_string())?;
        }
        // The whole leaf dies together, including on a host-wide OOM. A
        // half-killed agent is harder to reason about than a dead one.
        self.write("memory.oom.group", "1")?;
        if let Some(w) = limits.cpu_weight {
            self.write("cpu.weight", &w.to_string())?;
        }
        if let (Some(w), true) = (limits.io_weight, io_weight) {
            self.write("io.weight", &w.to_string())?;
        }
        if let Some(n) = limits.pids_max {
            self.write("pids.max", &n.to_string())?;
        }
        Ok(())
    }

    fn write(&self, file: &str, value: &str) -> io::Result<()> {
        let path = self.dir.join(file);
        fs::write(&path, value).map_err(|e| ctx(e, &format!("write {value:?} to"), &path))
    }

    /// An `O_CLOEXEC` directory fd for the leaf, for
    /// `clone3(CLONE_INTO_CGROUP)`. Close-on-exec is the point: a leaf fd
    /// that survives into an agent lets it reach a sibling's files through
    /// `..`, whatever its mount view and cgroup namespace (measured).
    pub fn open_dir(&self) -> io::Result<OwnedFd> {
        let path = cstring(&self.dir)?;
        // SAFETY: `path` is a valid NUL-terminated string; the fd returned
        // is owned by nothing else.
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(ctx(io::Error::last_os_error(), "open", &self.dir));
        }
        // SAFETY: `fd` was just returned by `open` and is not shared.
        Ok(unsafe { std::os::fd::FromRawFd::from_raw_fd(fd) })
    }

    /// Move an existing process into the leaf. The caller needs write
    /// access to `cgroup.procs` of the common ancestor of the process's
    /// current cgroup and this leaf, as well as the leaf's own.
    pub fn attach(&self, pid: u32) -> io::Result<()> {
        self.write("cgroup.procs", &pid.to_string())
    }

    /// Make `cmd`'s child move itself into the leaf before it `exec`s, so
    /// the program never runs outside it.
    ///
    /// This is the fallback route. D2's `clone3(CLONE_INTO_CGROUP)` starts
    /// the child inside the leaf with no window at all. With this route,
    /// the child spends the time between `fork` and `exec` in the parent's
    /// cgroup, running only std's post-fork code and one `write`.
    ///
    /// The fd is opened here, in the parent, `O_CLOEXEC`, so it is gone
    /// once the child execs. The child's `write("0")` moves the writer
    /// itself and is async-signal-safe.
    pub fn attach_on_spawn(&self, cmd: &mut std::process::Command) -> io::Result<()> {
        let path = self.dir.join("cgroup.procs");
        let procs: OwnedFd = fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .map_err(|e| ctx(e, "open", &path))?
            .into();
        // SAFETY: the closure calls only `write(2)` on an fd it owns.
        unsafe {
            cmd.pre_exec(move || {
                let fd = procs.as_raw_fd();
                if libc::write(fd, b"0".as_ptr().cast(), 1) != 1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(())
    }

    /// Whether any process is still in the leaf.
    pub fn populated(&self) -> io::Result<bool> {
        let events = fs::read_to_string(self.dir.join("cgroup.events"))?;
        Ok(field(&events, "populated") != 0)
    }

    /// `memory.current`: bytes charged to the leaf right now.
    pub fn memory_current(&self) -> io::Result<u64> {
        let s = fs::read_to_string(self.dir.join("memory.current"))?;
        s.trim()
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// `cpu.stat: usage_usec`: CPU time the leaf has used, in microseconds,
    /// across every process that has ever been in it.
    pub fn cpu_usage_usec(&self) -> io::Result<u64> {
        let stat = fs::read_to_string(self.dir.join("cpu.stat"))?;
        Ok(field(&stat, "usage_usec"))
    }

    /// The kernel's counters for why processes in this leaf died.
    pub fn exit_evidence(&self) -> io::Result<Evidence> {
        let memory = fs::read_to_string(self.dir.join("memory.events"))?;
        let pids = fs::read_to_string(self.dir.join("pids.events"))?;
        Ok(Evidence {
            oom_group_kills: field(&memory, "oom_group_kill"),
            oom_kills: field(&memory, "oom_kill"),
            forks_denied: field(&pids, "max"),
        })
    }

    /// Kill every process in the leaf, wait for the kernel to reap them,
    /// then remove the directory.
    ///
    /// `cgroup.kill` (Linux 5.14+) reaches every member, however far it
    /// has been reparented or detached with `setsid`: membership does not
    /// follow the process tree. A leaf that has not drained after the wait
    /// is left in place for a later sweep rather than blocking teardown.
    pub fn kill(self) -> io::Result<Teardown> {
        self.write("cgroup.kill", "1")?;
        for _ in 0..KILL_POLLS {
            if !self.populated()? {
                fs::remove_dir(&self.dir).map_err(|e| ctx(e, "remove leaf", &self.dir))?;
                return Ok(Teardown::Removed);
            }
            std::thread::sleep(KILL_POLL_INTERVAL);
        }
        Ok(Teardown::LeftBehind(self.dir))
    }
}

/// One path segment for a node or leaf name. No `.` at all, which rules
/// out `..` and every interface-file name (`memory.max`, `cgroup.procs`).
fn validate_name(name: &str) -> io::Result<()> {
    let ok = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name:?} is not a valid cgroup name for fleet (letters, digits, '-', '_')"),
        ))
    }
}

fn require_cgroup2(dir: &Path) -> io::Result<()> {
    let path = cstring(dir)?;
    // SAFETY: `statfs` is plain-old-data, fully written by the call.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is NUL-terminated and `st` is a valid out-pointer.
    if unsafe { libc::statfs(path.as_ptr(), &mut st) } != 0 {
        return Err(ctx(io::Error::last_os_error(), "statfs", dir));
    }
    #[allow(clippy::unnecessary_cast)] // f_type's width differs by target
    if st.f_type as i64 != CGROUP2_SUPER_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{} is not on a cgroup v2 filesystem", dir.display()),
        ));
    }
    Ok(())
}

/// Enable `controllers` in `dir`'s `cgroup.subtree_control`, then read it
/// back and fail if any is not listed. A controller that silently did not
/// enable is a limit that is configured and absent.
fn enable_controllers(dir: &Path, controllers: &[&str]) -> io::Result<()> {
    let path = dir.join("cgroup.subtree_control");
    for c in controllers {
        if let Err(e) = fs::write(&path, format!("+{c}")) {
            let hint = if e.raw_os_error() == Some(libc::EBUSY) {
                " (the cgroup holds processes; cgroup v2 allows domain controllers only \
                 on a cgroup whose processes are all in its children)"
            } else {
                ""
            };
            return Err(io::Error::new(
                e.kind(),
                format!("enable {c} in {}: {e}{hint}", path.display()),
            ));
        }
    }
    let enabled = read_words(&path)?;
    if let Some(c) = controllers
        .iter()
        .find(|c| !enabled.iter().any(|e| e == *c))
    {
        return Err(io::Error::other(format!(
            "{c} is not listed in {} after enabling it (listed: {enabled:?})",
            path.display()
        )));
    }
    Ok(())
}

fn read_words(path: &Path) -> io::Result<Vec<String>> {
    let s = fs::read_to_string(path).map_err(|e| ctx(e, "read", path))?;
    Ok(s.split_whitespace().map(str::to_owned).collect())
}

/// A counter in a flat-keyed cgroup file (`key value` per line), matched
/// on the whole key: `oom_kill` must not match `oom_group_kill`.
fn field(content: &str, key: &str) -> u64 {
    content
        .lines()
        .filter_map(|l| l.split_once(' '))
        .find(|(k, _)| *k == key)
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0)
}

fn cstring(path: &Path) -> io::Result<std::ffi::CString> {
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

fn ctx(e: io::Error, step: &str, path: &Path) -> io::Error {
    io::Error::new(e.kind(), format!("{step} {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pure_v2_hierarchy_yields_its_path() {
        assert_eq!(
            parse_unified_path("0::/user.slice/user-1000.slice/x.service\n").unwrap(),
            "/user.slice/user-1000.slice/x.service"
        );
    }

    #[test]
    fn a_hybrid_or_v1_hierarchy_is_refused() {
        let hybrid = "1:name=systemd:/user.slice\n0::/user.slice\n";
        assert!(parse_unified_path(hybrid).is_err());
        assert!(parse_unified_path("4:memory:/x\n").is_err());
        assert!(parse_unified_path("").is_err());
        assert!(parse_unified_path("0::/a\n0::/b\n").is_err());
    }

    #[test]
    fn counters_match_whole_keys() {
        let events = "low 0\nhigh 0\nmax 27\noom 1\noom_kill 3\noom_group_kill 1\n";
        assert_eq!(field(events, "oom_kill"), 3);
        assert_eq!(field(events, "oom_group_kill"), 1);
        assert_eq!(field(events, "max"), 27);
        assert_eq!(field(events, "missing"), 0);
    }

    #[test]
    fn swap_is_read_from_the_rows_not_the_header() {
        let header = "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n";
        assert!(!swap_active(header));
        assert!(swap_active(&format!(
            "{header}/dev/zram0 partition\t12304532\t0\t32767\n"
        )));
    }

    #[test]
    fn names_are_single_plain_segments() {
        for ok in ["fleet", "agent-1", "a_b"] {
            assert!(validate_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "..", ".x", "a/b", "../b", "cgroup.procs", "a b"] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
    }
}
