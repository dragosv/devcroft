//! Controller-side VMM helper launch, descriptor minimization, bounded log
//! capture, and owned endpoint/process cleanup (`add-vm-project-runtime` task 2.1).

use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, DirBuilder};
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::contract::{ErrorLayer, HostProcessIdentity, is_valid_hex};

/// Conservative bound well below `sockaddr_un.sun_path` limits on macOS (104)
/// and Linux (108), matching `experiments/microvm/libkrun/src/config.rs`.
pub const MAX_SOCKET_PATH_BYTES: usize = 100;

/// Single artifact reference matching the isolated helper JSON schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperArtifactInput {
    pub path: PathBuf,
    pub sha256: String,
}

/// Private helper launch record matching `devcroft-krun-spike` /
/// `devcroft-vm-helper` (`Launch`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperLaunchFileV1 {
    pub schema: u32,
    pub kernel: HelperArtifactInput,
    pub initramfs: HelperArtifactInput,
    pub runtime: HelperArtifactInput,
    pub workload: Option<HelperArtifactInput>,
    pub data: Option<PathBuf>,
    pub state: PathBuf,
    pub instance_dir: PathBuf,
    pub vcpus: u8,
    pub memory_mib: u32,
}

/// Bounded per-instance socket paths directly inside `instance_dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmEndpointPaths {
    pub controller_dir: PathBuf,
    pub instance_dir: PathBuf,
    pub control_socket: PathBuf,
    pub keeper_socket: PathBuf,
    pub ssh_socket: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    OverlappingDirectories {
        controller_dir: PathBuf,
        instance_dir: PathBuf,
    },
    EndpointTooLong {
        path: PathBuf,
        len: usize,
    },
    EndpointOccupied(PathBuf),
    InvalidLaunchRecord(String),
    Spawn(String),
    Io(String),
}

impl LaunchError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            LaunchError::Spawn(_) => ErrorLayer::Backend,
            _ => ErrorLayer::Config,
        }
    }
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchError::OverlappingDirectories {
                controller_dir,
                instance_dir,
            } => write!(
                f,
                "controller directory {} and writable instance directory {} must be disjoint",
                controller_dir.display(),
                instance_dir.display()
            ),
            LaunchError::EndpointTooLong { path, len } => write!(
                f,
                "socket path {} ({len} bytes) exceeds maximum {MAX_SOCKET_PATH_BYTES} bytes",
                path.display()
            ),
            LaunchError::EndpointOccupied(path) => write!(
                f,
                "refusing launch because socket endpoint {} already exists",
                path.display()
            ),
            LaunchError::InvalidLaunchRecord(msg) => {
                write!(f, "invalid helper launch record: {msg}")
            }
            LaunchError::Spawn(msg) => write!(f, "spawning VM helper failed: {msg}"),
            LaunchError::Io(msg) => write!(f, "VM launch I/O error: {msg}"),
        }
    }
}

impl std::error::Error for LaunchError {}

fn create_private_dir(path: &Path) -> Result<PathBuf, LaunchError> {
    if !path.is_absolute() {
        return Err(LaunchError::InvalidLaunchRecord(format!(
            "directory {} must be absolute",
            path.display()
        )));
    }
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| LaunchError::Io(format!("{}: {e}", path.display())))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| LaunchError::Io(format!("{}: {e}", path.display())))?;
    fs::canonicalize(path).map_err(|e| LaunchError::Io(format!("{}: {e}", path.display())))
}

/// Create owner-only (`0700`) disjoint controller and instance directories and
/// validate clean, length-bounded socket endpoints inside `instance_dir`.
pub fn prepare_instance_layout(
    controller_dir: &Path,
    instance_dir: &Path,
) -> Result<VmEndpointPaths, LaunchError> {
    let controller = create_private_dir(controller_dir)?;
    let instance = create_private_dir(instance_dir)?;
    if controller.starts_with(&instance) || instance.starts_with(&controller) {
        return Err(LaunchError::OverlappingDirectories {
            controller_dir: controller,
            instance_dir: instance,
        });
    }
    let control_socket = instance.join("control.sock");
    let keeper_socket = instance.join("keeper.sock");
    let ssh_socket = instance.join("ssh.sock");

    for sock in [&control_socket, &keeper_socket, &ssh_socket] {
        let len = sock.as_os_str().len();
        if len > MAX_SOCKET_PATH_BYTES {
            return Err(LaunchError::EndpointTooLong {
                path: sock.clone(),
                len,
            });
        }
        match sock.symlink_metadata() {
            Ok(_) => return Err(LaunchError::EndpointOccupied(sock.clone())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(LaunchError::Io(format!("{}: {e}", sock.display()))),
        }
    }

    Ok(VmEndpointPaths {
        controller_dir: controller,
        instance_dir: instance,
        control_socket,
        keeper_socket,
        ssh_socket,
    })
}

/// Persist `record` with mode `0600` inside `controller_dir` (refusing any
/// target inside `instance_dir`).
pub fn write_private_launch_record(
    controller_dir: &Path,
    instance_dir: &Path,
    filename: &str,
    record: &HelperLaunchFileV1,
) -> Result<PathBuf, LaunchError> {
    if record.schema != 1
        || !(1..=8).contains(&record.vcpus)
        || !(128..=4096).contains(&record.memory_mib)
    {
        return Err(LaunchError::InvalidLaunchRecord(
            "require schema=1, vcpus in 1..=8, memory_mib in 128..=4096".into(),
        ));
    }
    for (name, art) in [
        ("kernel", &record.kernel),
        ("initramfs", &record.initramfs),
        ("runtime", &record.runtime),
    ]
    .into_iter()
    .chain(record.workload.iter().map(|w| ("workload", w)))
    {
        if !art.path.is_absolute() || !is_valid_hex(&art.sha256.to_ascii_lowercase(), 64) {
            return Err(LaunchError::InvalidLaunchRecord(format!(
                "artifact `{name}` requires an absolute path and 64-hex SHA-256"
            )));
        }
    }
    if filename.is_empty() || filename.contains('/') || filename.contains('\\') || filename == ".."
    {
        return Err(LaunchError::InvalidLaunchRecord(format!(
            "invalid filename `{filename}`"
        )));
    }
    let controller = create_private_dir(controller_dir)?;
    let instance = create_private_dir(instance_dir)?;
    if controller.starts_with(&instance) || instance.starts_with(&controller) {
        return Err(LaunchError::OverlappingDirectories {
            controller_dir: controller,
            instance_dir: instance,
        });
    }
    let target = controller.join(filename);
    let payload = serde_json::to_vec_pretty(record)
        .map_err(|e| LaunchError::InvalidLaunchRecord(e.to_string()))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&target)
        .map_err(|e| LaunchError::Io(format!("{}: {e}", target.display())))?;
    io::Write::write_all(&mut file, &payload)
        .map_err(|e| LaunchError::Io(format!("{}: {e}", target.display())))?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
        .map_err(|e| LaunchError::Io(format!("{}: {e}", target.display())))?;
    fs::canonicalize(&target).map_err(|e| LaunchError::Io(format!("{}: {e}", target.display())))
}

/// Snapshot of a helper's captured stdout or stderr stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogSnapshot {
    pub bytes: Vec<u8>,
    pub truncated: bool,
    pub total_bytes_seen: u64,
}

struct LogState {
    bytes: Vec<u8>,
    truncated: bool,
    total_bytes_seen: u64,
}

/// Drains a helper pipe continuously while retaining at most `cap` bytes so a
/// verbose guest never blocks on stdout/stderr or exhausts host memory.
struct BoundedLogCapture {
    state: Arc<Mutex<LogState>>,
    thread: Option<JoinHandle<()>>,
}

impl BoundedLogCapture {
    fn spawn<R: Read + Send + 'static>(mut reader: R, cap: usize) -> Self {
        let state = Arc::new(Mutex::new(LogState {
            bytes: Vec::with_capacity(cap.min(8192)),
            truncated: false,
            total_bytes_seen: 0,
        }));
        let shared = Arc::clone(&state);
        let thread = thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut guard = shared.lock().expect("log lock poisoned");
                        guard.total_bytes_seen += n as u64;
                        let rem = cap.saturating_sub(guard.bytes.len());
                        if rem > 0 {
                            let take = rem.min(n);
                            guard.bytes.extend_from_slice(&buf[..take]);
                            if take < n {
                                guard.truncated = true;
                            }
                        } else {
                            guard.truncated = true;
                        }
                    }
                }
            }
        });
        Self {
            state,
            thread: Some(thread),
        }
    }

    fn snapshot(&self) -> LogSnapshot {
        let guard = self.state.lock().expect("log lock poisoned");
        LogSnapshot {
            bytes: guard.bytes.clone(),
            truncated: guard.truncated,
            total_bytes_seen: guard.total_bytes_seen,
        }
    }

    fn join(&mut self) {
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// Running or recently reaped VMM helper process owned by the controller.
pub struct HelperProcess {
    child: Child,
    identity: HostProcessIdentity,
    stdout: BoundedLogCapture,
    stderr: BoundedLogCapture,
    reaped: Option<ExitStatus>,
}

impl HelperProcess {
    pub fn pid(&self) -> u32 {
        self.identity.pid
    }

    pub fn identity(&self) -> &HostProcessIdentity {
        &self.identity
    }

    pub fn stdout_snapshot(&self) -> LogSnapshot {
        self.stdout.snapshot()
    }

    pub fn stderr_snapshot(&self) -> LogSnapshot {
        self.stderr.snapshot()
    }

    pub fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<ExitStatus>, LaunchError> {
        if let Some(status) = self.reaped {
            return Ok(Some(status));
        }
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.reaped = Some(status);
                    self.stdout.join();
                    self.stderr.join();
                    return Ok(Some(status));
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        return Ok(None);
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(LaunchError::Io(e.to_string())),
            }
        }
    }

    /// Terminate the helper's process group cleanly (`SIGTERM` then `SIGKILL`)
    /// only after confirming process start identity still matches.
    pub fn terminate_and_reap(&mut self, grace: Duration) -> Result<ExitStatus, LaunchError> {
        if let Some(status) = self.reaped {
            return Ok(status);
        }
        if let Some(status) = self.wait_timeout(Duration::from_millis(1))? {
            return Ok(status);
        }
        if process_matches_identity(&self.identity) {
            let pgid = self.identity.pid as libc::pid_t;
            // Signal negative pgid so child threads/descendants created by the
            // helper are reaped together; helper holds its own process group
            // via setpgid(0, 0) in pre_exec.
            unsafe {
                libc::kill(-pgid, libc::SIGTERM);
            }
        }
        if let Some(status) = self.wait_timeout(grace)? {
            return Ok(status);
        }
        if process_matches_identity(&self.identity) {
            let pgid = self.identity.pid as libc::pid_t;
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
        let status = self
            .child
            .wait()
            .map_err(|e| LaunchError::Io(e.to_string()))?;
        self.reaped = Some(status);
        self.stdout.join();
        self.stderr.join();
        Ok(status)
    }
}

impl Drop for HelperProcess {
    fn drop(&mut self) {
        if self.reaped.is_none() {
            let _ = self.terminate_and_reap(Duration::from_millis(50));
        }
    }
}

/// Spawn `helper_bin` with stdin closed (`/dev/null`), bounded stdout/stderr
/// pipes, strict umask `0077`, dedicated process group, and all ambient file
/// descriptors `>= 3` closed before `exec`.
pub fn spawn_helper(
    helper_bin: &Path,
    args: &[&OsStr],
    log_cap_bytes: usize,
) -> Result<HelperProcess, LaunchError> {
    if !helper_bin.is_file() {
        return Err(LaunchError::Spawn(format!(
            "helper binary {} not found",
            helper_bin.display()
        )));
    }
    let mut cmd = Command::new(helper_bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SAFETY: only async-signal-safe libc calls (`umask`, `setpgid`,
    // `getrlimit`, `close`) execute between `fork` and `exec`.
    unsafe {
        cmd.pre_exec(|| {
            libc::umask(0o077);
            libc::setpgid(0, 0);
            let mut rl = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            let max_fd = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) == 0 {
                rl.rlim_cur.min(8192) as libc::c_int
            } else {
                4096
            };
            for fd in 3..=max_fd {
                libc::close(fd);
            }
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| LaunchError::Spawn(format!("{}: {e}", helper_bin.display())))?;
    let pid = child.id();
    let start_time_ns = capture_start_time_ns(pid).unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    });
    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| LaunchError::Spawn("missing stdout pipe".into()))?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| LaunchError::Spawn("missing stderr pipe".into()))?;

    Ok(HelperProcess {
        child,
        identity: HostProcessIdentity { pid, start_time_ns },
        stdout: BoundedLogCapture::spawn(stdout_pipe, log_cap_bytes),
        stderr: BoundedLogCapture::spawn(stderr_pipe, log_cap_bytes),
        reaped: None,
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn capture_start_time_ns(pid: u32) -> Option<u64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::pid_t,
            libc::PROC_PIDTBSDINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if n == size {
        let ns = (info.pbi_start_tvsec)
            .saturating_mul(1_000_000_000)
            .saturating_add((info.pbi_start_tvusec).saturating_mul(1_000));
        (ns > 0).then_some(ns)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn capture_start_time_ns(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rfind(')')?;
    // Field 22 (1-indexed) in /proc/<pid>/stat is index 19 after ') '.
    stat[after_comm + 1..]
        .split_whitespace()
        .nth(19)
        .and_then(|v| v.parse::<u64>().ok())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn capture_start_time_ns(_pid: u32) -> Option<u64> {
    None
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn process_matches_identity(identity: &HostProcessIdentity) -> bool {
    capture_start_time_ns(identity.pid).is_some_and(|observed| observed == identity.start_time_ns)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn process_matches_identity(identity: &HostProcessIdentity) -> bool {
    unsafe { libc::kill(identity.pid as libc::pid_t, 0) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn prepare_instance_layout_enforces_modes_disjointness_and_endpoint_bounds() {
        let base = std::env::temp_dir().join(format!("dc-vm-launch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let ctrl = base.join("ctrl");
        let inst = base.join("inst");

        let paths = prepare_instance_layout(&ctrl, &inst).unwrap();
        assert_eq!(
            fs::metadata(&paths.controller_dir)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&paths.instance_dir)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        // Nested controller inside instance is refused.
        assert!(matches!(
            prepare_instance_layout(&inst.join("nested"), &inst),
            Err(LaunchError::OverlappingDirectories { .. })
        ));

        // Occupied endpoint or dangling symlink is refused.
        fs::write(&paths.control_socket, b"").unwrap();
        assert!(matches!(
            prepare_instance_layout(&ctrl, &inst),
            Err(LaunchError::EndpointOccupied(_))
        ));
        let _ = fs::remove_file(&paths.control_socket);

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/nonexistent/target", &paths.keeper_socket).unwrap();
            assert!(matches!(
                prepare_instance_layout(&ctrl, &inst),
                Err(LaunchError::EndpointOccupied(_))
            ));
            let _ = fs::remove_file(&paths.keeper_socket);
        }

        // Socket path exceeding MAX_SOCKET_PATH_BYTES is refused.
        let deep = base.join("a".repeat(95));
        assert!(matches!(
            prepare_instance_layout(&ctrl, &deep),
            Err(LaunchError::EndpointTooLong { .. })
        ));

        let launch = HelperLaunchFileV1 {
            schema: 1,
            kernel: HelperArtifactInput {
                path: PathBuf::from("/trusted/vmlinux"),
                sha256: SHA_A.into(),
            },
            initramfs: HelperArtifactInput {
                path: PathBuf::from("/trusted/initramfs"),
                sha256: SHA_A.into(),
            },
            runtime: HelperArtifactInput {
                path: PathBuf::from("/trusted/runtime.img"),
                sha256: SHA_A.into(),
            },
            workload: None,
            data: None,
            state: inst.join("state.img"),
            instance_dir: inst.clone(),
            vcpus: 2,
            memory_mib: 512,
        };
        let rec_path = write_private_launch_record(&ctrl, &inst, "launch.json", &launch).unwrap();
        assert_eq!(
            fs::metadata(&rec_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn spawn_helper_closes_inherited_fds_and_caps_logs() {
        let base = std::env::temp_dir().join(format!("dc-vm-spawn-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let secret = base.join("secret.txt");
        fs::write(&secret, b"top-secret").unwrap();

        // Open file without CLOEXEC explicitly to verify pre_exec fd scrub.
        let file = fs::File::open(&secret).unwrap();
        let fd = file.as_raw_fd();
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
        }

        let probe_cmd =
            format!("if cat <&{fd} 2>/dev/null; then echo LEAKED; else echo CLOSED; fi");
        let mut proc_fd = spawn_helper(
            Path::new("/bin/sh"),
            &[OsStr::new("-c"), OsStr::new(&probe_cmd)],
            1024,
        )
        .unwrap();
        let status = proc_fd
            .wait_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(status.success());
        let out = String::from_utf8_lossy(&proc_fd.stdout_snapshot().bytes).into_owned();
        assert_eq!(out.trim(), "CLOSED");
        drop(file);

        // Verify log capture bounds large output while draining cleanly.
        let flood_cmd = "i=0; while [ $i -lt 3000 ]; do printf '0123456789abcdef0123456789abcdef\\n'; i=$((i+1)); done";
        let mut proc_log = spawn_helper(
            Path::new("/bin/sh"),
            &[OsStr::new("-c"), OsStr::new(flood_cmd)],
            2048,
        )
        .unwrap();
        let status = proc_log
            .wait_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(status.success());
        let snap = proc_log.stdout_snapshot();
        assert_eq!(snap.bytes.len(), 2048);
        assert!(snap.truncated);
        assert!(snap.total_bytes_seen > 64 * 1024);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn spawn_helper_queries_experimental_libkrun_version_when_built() {
        let candidate = Path::new("experiments/microvm/libkrun/target/debug/devcroft-krun-spike");
        if !candidate.is_file() {
            return;
        }
        let mut proc_ver = spawn_helper(candidate, &[OsStr::new("version")], 4096).unwrap();
        let status = proc_ver
            .wait_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(status.success());
        let out = String::from_utf8_lossy(&proc_ver.stdout_snapshot().bytes).into_owned();
        assert!(out.contains("libkrun"), "{out}");
    }
}
