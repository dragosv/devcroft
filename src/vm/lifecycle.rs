//! VM persistent state, independent health checks, graceful/forced shutdown,
//! cold restart recovery, PID-reuse-safe owned removal, leased-cache GC, and
//! rollback disk export (`add-vm-project-runtime` tasks 4.1, 4.2, 4.3, 4.6).

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::contract::{
    ComponentHealth, ErrorLayer, GuestTarget, HostProcessIdentity, RuntimeKind, StopClassification,
    VmAdapter, VmHealthBreakdown, VmStateRecordV1,
};
use super::grants::{ExistingRuntime, ProjectIdentity};
use super::guest_init::{format_stop_request, verify_stop_acknowledgement};
use super::launcher::process_matches_identity;
use super::publication::ArtifactCacheStore;
use super::sha256::sha256_file_hex;
use super::transport::{VmControlClient, VmKeeperClient};

/// Host-side file layout for one VM instance's lifecycle operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmInstancePaths {
    pub controller_dir: PathBuf,
    pub instance_dir: PathBuf,
    pub state_file: PathBuf,
    pub private_disk: PathBuf,
    pub control_socket: PathBuf,
    pub keeper_socket: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownOutcome {
    pub classification: StopClassification,
    pub private_disk_retained: bool,
    pub control_reply: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartPlan {
    pub next_generation: u64,
    pub recovery_check_required: bool,
    pub preserved_private_disk: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalReport {
    pub removed_instance_dir: bool,
    pub removed_controller_dir: bool,
    pub reclaimed_cache_artifacts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VmRollbackPolicy {
    pub creation_disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmLifecycleError {
    InstanceStillRunning(u32),
    MissingState(PathBuf),
    IncompatibleState(String),
    RollbackCreationDisabled,
    ExportTargetExists(PathBuf),
    ExportVerificationFailed(String),
    Io(String),
}

impl VmLifecycleError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            VmLifecycleError::RollbackCreationDisabled => ErrorLayer::Backend,
            VmLifecycleError::ExportVerificationFailed(_) => ErrorLayer::Backend,
            _ => ErrorLayer::Config,
        }
    }
}

impl fmt::Display for VmLifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmLifecycleError::InstanceStillRunning(pid) => write!(
                f,
                "VM helper PID {pid} is still running; stop it cleanly before restarting, removing, or exporting"
            ),
            VmLifecycleError::MissingState(path) => {
                write!(f, "VM state file {} not found", path.display())
            }
            VmLifecycleError::IncompatibleState(msg) => {
                write!(
                    f,
                    "incompatible VM state for restart (disk retained): {msg}"
                )
            }
            VmLifecycleError::RollbackCreationDisabled => write!(
                f,
                "new VM creation is disabled by rollback policy (existing native behavior unchanged)"
            ),
            VmLifecycleError::ExportTargetExists(path) => write!(
                f,
                "refusing disk export: destination {} already exists",
                path.display()
            ),
            VmLifecycleError::ExportVerificationFailed(msg) => {
                write!(f, "retained disk export verification failed: {msg}")
            }
            VmLifecycleError::Io(msg) => write!(f, "VM lifecycle I/O error: {msg}"),
        }
    }
}

impl std::error::Error for VmLifecycleError {}

/// Persist `VmStateRecordV1` atomically inside `controller_dir` with mode `0600`.
pub fn save_vm_state(path: &Path, state: &VmStateRecordV1) -> Result<(), VmLifecycleError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| VmLifecycleError::Io(e.to_string()))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|e| VmLifecycleError::Io(e.to_string()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(state).map_err(|e| VmLifecycleError::Io(e.to_string()))?;
    {
        let mut f = File::create(&tmp).map_err(|e| VmLifecycleError::Io(e.to_string()))?;
        f.write_all(&body)
            .map_err(|e| VmLifecycleError::Io(e.to_string()))?;
        f.sync_data()
            .map_err(|e| VmLifecycleError::Io(e.to_string()))?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
        .map_err(|e| VmLifecycleError::Io(e.to_string()))?;
    fs::rename(&tmp, path).map_err(|e| VmLifecycleError::Io(e.to_string()))?;
    Ok(())
}

/// Strictly decode `VmStateRecordV1`, returning `Ok(None)` only when absent.
pub fn load_vm_state(path: &Path) -> Result<Option<VmStateRecordV1>, VmLifecycleError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(VmLifecycleError::Io(e.to_string())),
    };
    let parsed: VmStateRecordV1 = serde_json::from_slice(&bytes)
        .map_err(|e| VmLifecycleError::IncompatibleState(e.to_string()))?;
    Ok(Some(parsed))
}

/// Verify whether `identity` refers to the exact still-live helper process
/// using both PID and process start time so reused PIDs never match.
pub fn is_owned_helper_alive(identity: &HostProcessIdentity) -> bool {
    process_matches_identity(identity)
}

/// Independently probe helper identity liveness, guest control channel
/// (`PING` -> `PONG`), and keeper readiness (`Frame::Query`). Helper liveness
/// alone never reports `is_project_ready() == true`.
pub fn probe_vm_health(
    state: &VmStateRecordV1,
    control_socket: &Path,
    keeper_socket: &Path,
    timeout: Duration,
) -> VmHealthBreakdown {
    let helper = match &state.host_process {
        Some(id) if is_owned_helper_alive(id) => ComponentHealth::Healthy,
        Some(_) => ComponentHealth::Stopped,
        None => ComponentHealth::Stopped,
    };
    let transport = match VmControlClient::new(control_socket, timeout).send_command("PING") {
        Ok(reply) if reply.trim() == "PONG" => ComponentHealth::Healthy,
        Ok(_) => ComponentHealth::Unhealthy,
        Err(_) => ComponentHealth::Stopped,
    };
    let keeper = match VmKeeperClient::new(keeper_socket, timeout).query() {
        Ok(_) => ComponentHealth::Healthy,
        Err(_) if helper == ComponentHealth::Healthy => ComponentHealth::Unhealthy,
        Err(_) => ComponentHealth::Stopped,
    };
    VmHealthBreakdown {
        helper,
        transport,
        keeper,
        environment_fresh: state.health.environment_fresh,
        services: state.health.services,
    }
}

/// Request cooperative quiesced stop (`SYNC-STOP`); escalate to process-group
/// termination only after verifying `HostProcessIdentity` when control misses
/// or refuses (`ERR shutdown-busy`), preserving `private_disk` for recovery.
pub fn graceful_down(
    paths: &VmInstancePaths,
    state: &mut VmStateRecordV1,
    nonce: &str,
    control_timeout: Duration,
    force_grace: Duration,
) -> Result<ShutdownOutcome, VmLifecycleError> {
    let req = format_stop_request(state.instance_generation, nonce)
        .map_err(|e| VmLifecycleError::Io(e.to_string()))?;
    let reply = VmControlClient::new(&paths.control_socket, control_timeout)
        .send_command(&req)
        .ok();

    let orderly = reply
        .as_deref()
        .is_some_and(|r| verify_stop_acknowledgement(r, state.instance_generation, nonce).is_ok());

    if let Some(id) = state.host_process.clone()
        && is_owned_helper_alive(&id)
    {
        if orderly {
            wait_helper_exit(&id, force_grace);
        } else {
            terminate_owned_helper(&id, force_grace);
        }
    }

    let classification = if orderly {
        StopClassification::Orderly
    } else {
        StopClassification::ForcedStop
    };
    state.last_stop = Some(classification);
    state.host_process = None;
    state.health.helper = ComponentHealth::Stopped;
    state.health.transport = ComponentHealth::Stopped;
    state.health.keeper = ComponentHealth::Stopped;
    state.health.services = ComponentHealth::Stopped;

    let _ = fs::remove_file(&paths.control_socket);
    let _ = fs::remove_file(&paths.keeper_socket);
    save_vm_state(&paths.state_file, state)?;

    Ok(ShutdownOutcome {
        classification,
        private_disk_retained: paths.private_disk.is_file(),
        control_reply: reply,
    })
}

fn wait_helper_exit(id: &HostProcessIdentity, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while is_owned_helper_alive(id) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn terminate_owned_helper(id: &HostProcessIdentity, grace: Duration) {
    if !is_owned_helper_alive(id) {
        return;
    }
    let pid = id.pid as libc::pid_t;
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    wait_helper_exit(id, grace);
    if is_owned_helper_alive(id) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        wait_helper_exit(id, Duration::from_millis(200));
    }
}

/// Validate compatibility and private disk presence before restarting a stopped
/// VM instance (`4.2`), refusing incompatible state without deleting data.
pub fn prepare_cold_restart(
    paths: &VmInstancePaths,
    expected_target: GuestTarget,
    expected_adapter: VmAdapter,
    expected_disk_id: &str,
) -> Result<RestartPlan, VmLifecycleError> {
    let state = load_vm_state(&paths.state_file)?
        .ok_or_else(|| VmLifecycleError::MissingState(paths.state_file.clone()))?;
    if let Some(id) = &state.host_process
        && is_owned_helper_alive(id)
    {
        return Err(VmLifecycleError::InstanceStillRunning(id.pid));
    }
    if state.runtime_kind
        != (RuntimeKind::Vm {
            target: expected_target,
            adapter: expected_adapter,
        })
    {
        return Err(VmLifecycleError::IncompatibleState(
            "recorded runtime target/adapter does not match requested VM configuration".into(),
        ));
    }
    if state.private_disk_id != expected_disk_id {
        return Err(VmLifecycleError::IncompatibleState(format!(
            "private disk `{}` does not match expected `{expected_disk_id}`",
            state.private_disk_id
        )));
    }
    if !paths.private_disk.is_file() {
        return Err(VmLifecycleError::IncompatibleState(format!(
            "missing private disk {}",
            paths.private_disk.display()
        )));
    }
    let recovery_check_required = state.last_stop != Some(StopClassification::Orderly);
    Ok(RestartPlan {
        next_generation: state.instance_generation.saturating_add(1),
        recovery_check_required,
        preserved_private_disk: paths.private_disk.clone(),
    })
}

/// Remove only stopped resources owned by this instance (`4.3`), verifying PID
/// identity before refusing/reaping and releasing any associated cache lease.
pub fn remove_owned_instance(
    paths: &VmInstancePaths,
    instance_id: &str,
    project: &ProjectIdentity,
    cache: Option<(&ArtifactCacheStore, &str)>,
) -> Result<RemovalReport, VmLifecycleError> {
    if let Some(state) = load_vm_state(&paths.state_file)?
        && let Some(id) = &state.host_process
        && is_owned_helper_alive(id)
    {
        return Err(VmLifecycleError::InstanceStillRunning(id.pid));
    }

    let removed_instance_dir = if paths.instance_dir.exists() {
        fs::remove_dir_all(&paths.instance_dir).map_err(|e| VmLifecycleError::Io(e.to_string()))?;
        true
    } else {
        false
    };
    let removed_controller_dir = if paths.controller_dir.exists() {
        fs::remove_dir_all(&paths.controller_dir)
            .map_err(|e| VmLifecycleError::Io(e.to_string()))?;
        true
    } else {
        false
    };

    let reclaimed_cache_artifacts = if let Some((store, content_sha256)) = cache {
        store
            .release_lease(project, content_sha256, instance_id)
            .map_err(|e| VmLifecycleError::Io(e.to_string()))?;
        store
            .collect_unleased(project)
            .map_err(|e| VmLifecycleError::Io(e.to_string()))?
    } else {
        Vec::new()
    };

    Ok(RemovalReport {
        removed_instance_dir,
        removed_controller_dir,
        reclaimed_cache_artifacts,
    })
}

/// Enforce rollback policy (`4.6`): disabling new VM creation fails closed
/// without downgrading an existing VM or silently diverting to native.
pub fn guard_new_vm_creation(
    policy: VmRollbackPolicy,
    existing: Option<ExistingRuntime>,
) -> Result<(), VmLifecycleError> {
    if policy.creation_disabled && existing.is_none() {
        return Err(VmLifecycleError::RollbackCreationDisabled);
    }
    Ok(())
}

/// Export a stopped VM instance's retained private disk (`4.6`) with exact
/// byte-length and SHA-256 verification, keeping the retained original intact.
pub fn export_retained_vm_disk(
    paths: &VmInstancePaths,
    destination: &Path,
) -> Result<u64, VmLifecycleError> {
    if let Some(state) = load_vm_state(&paths.state_file)?
        && let Some(id) = &state.host_process
        && is_owned_helper_alive(id)
    {
        return Err(VmLifecycleError::InstanceStillRunning(id.pid));
    }
    if !paths.private_disk.is_file() {
        return Err(VmLifecycleError::IncompatibleState(format!(
            "retained private disk {} missing",
            paths.private_disk.display()
        )));
    }
    if destination.exists() {
        return Err(VmLifecycleError::ExportTargetExists(
            destination.to_path_buf(),
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|e| VmLifecycleError::Io(e.to_string()))?;
    }
    let copied = fs::copy(&paths.private_disk, destination)
        .map_err(|e| VmLifecycleError::Io(e.to_string()))?;
    let src_sha =
        sha256_file_hex(&paths.private_disk).map_err(|e| VmLifecycleError::Io(e.to_string()))?;
    let dst_sha = sha256_file_hex(destination).map_err(|e| VmLifecycleError::Io(e.to_string()))?;
    if src_sha != dst_sha {
        let _ = fs::remove_file(destination);
        return Err(VmLifecycleError::ExportVerificationFailed(format!(
            "SHA-256 mismatch after export ({src_sha} != {dst_sha})"
        )));
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::contract::{CONTRACT_SCHEMA_V1, RuntimeSelectionSource};
    use crate::vm::publication::{CrossProjectEligibility, RetentionLeaseV1};
    use crate::vm::sha256::sha256_hex;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::thread;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn sample_state(gen_id: u64, host_process: Option<HostProcessIdentity>) -> VmStateRecordV1 {
        VmStateRecordV1 {
            schema: CONTRACT_SCHEMA_V1,
            runtime_kind: RuntimeKind::Vm {
                target: GuestTarget::LinuxArm64,
                adapter: VmAdapter::Libkrun,
            },
            selection_source: RuntimeSelectionSource::QualifiedVmDefault,
            instance_generation: gen_id,
            host_process,
            private_disk_id: "disk-private-1".into(),
            boot_artifact_sha256: SHA_A.into(),
            health: VmHealthBreakdown {
                helper: ComponentHealth::Healthy,
                transport: ComponentHealth::Stopped,
                keeper: ComponentHealth::Stopped,
                environment_fresh: true,
                services: ComponentHealth::Stopped,
            },
            last_stop: None,
        }
    }

    fn make_paths(base: &Path, name: &str) -> VmInstancePaths {
        let ctrl = base.join(format!("{name}-ctrl"));
        let inst = base.join(format!("{name}-inst"));
        fs::create_dir_all(&ctrl).unwrap();
        fs::create_dir_all(&inst).unwrap();
        VmInstancePaths {
            state_file: ctrl.join("vm-state.json"),
            private_disk: inst.join("workspace.img"),
            control_socket: inst.join("control.sock"),
            keeper_socket: inst.join("keeper.sock"),
            controller_dir: ctrl,
            instance_dir: inst,
        }
    }

    #[test]
    fn shutdown_cold_restart_and_incompatible_disk_preservation() {
        let base = std::env::temp_dir().join(format!("dc-vm-life-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let paths = make_paths(&base, "alpha");
        fs::write(&paths.private_disk, b"private-workspace-data").unwrap();

        // 1. Orderly SYNC-STOP -> SYNCED
        let listener = UnixListener::bind(&paths.control_socket).unwrap();
        thread::spawn(move || {
            for conn in listener.incoming().take(2).flatten() {
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut req = String::new();
                if reader.read_line(&mut req).is_ok() {
                    let mut w = conn;
                    if req.starts_with("SYNC-STOP 2 ") {
                        let _ = w.write_all(b"SYNCED 2 n1\n");
                    } else {
                        let _ = w.write_all(b"ERR shutdown-busy\n");
                    }
                }
            }
        });

        let mut st = sample_state(2, None);
        let out = graceful_down(
            &paths,
            &mut st,
            "n1",
            Duration::from_secs(2),
            Duration::from_millis(20),
        )
        .unwrap();
        assert_eq!(out.classification, StopClassification::Orderly);
        assert!(out.private_disk_retained);
        let plan = prepare_cold_restart(
            &paths,
            GuestTarget::LinuxArm64,
            VmAdapter::Libkrun,
            "disk-private-1",
        )
        .unwrap();
        assert_eq!(plan.next_generation, 3);
        assert!(!plan.recovery_check_required);

        // 2. Forced stop (no listener -> ForcedStop -> recovery check required)
        let mut st2 = sample_state(3, None);
        let forced = graceful_down(
            &paths,
            &mut st2,
            "n2",
            Duration::from_millis(100),
            Duration::from_millis(20),
        )
        .unwrap();
        assert_eq!(forced.classification, StopClassification::ForcedStop);
        assert!(forced.private_disk_retained);
        let plan2 = prepare_cold_restart(
            &paths,
            GuestTarget::LinuxArm64,
            VmAdapter::Libkrun,
            "disk-private-1",
        )
        .unwrap();
        assert!(plan2.recovery_check_required);

        // 3. Incompatible target refuses without deleting disk.
        assert!(matches!(
            prepare_cold_restart(
                &paths,
                GuestTarget::LinuxX86_64,
                VmAdapter::Libkrun,
                "disk-private-1"
            ),
            Err(VmLifecycleError::IncompatibleState(_))
        ));
        assert!(paths.private_disk.is_file());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn pid_reuse_removal_sibling_survival_cache_gc_and_rollback_export() {
        let base = std::env::temp_dir().join(format!("dc-vm-rm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        // Stale record pointing to our OWN live PID with bogus start time must NOT match!
        let reused_pid = HostProcessIdentity {
            pid: std::process::id(),
            start_time_ns: 1,
        };
        assert!(!is_owned_helper_alive(&reused_pid));

        let paths_a = make_paths(&base, "a");
        let paths_b = make_paths(&base, "b");
        let disk_bytes = b"vm-retained-disk-bytes-v1";
        fs::write(&paths_a.private_disk, disk_bytes).unwrap();
        fs::write(&paths_b.private_disk, disk_bytes).unwrap();

        save_vm_state(&paths_a.state_file, &sample_state(1, Some(reused_pid))).unwrap();
        save_vm_state(&paths_b.state_file, &sample_state(1, None)).unwrap();

        // Rollback blocks new VM creation while export copies byte-identical retained disk.
        assert!(matches!(
            guard_new_vm_creation(
                VmRollbackPolicy {
                    creation_disabled: true
                },
                None
            ),
            Err(VmLifecycleError::RollbackCreationDisabled)
        ));
        let export_path = base.join("exported/workspace-a.img");
        let copied = export_retained_vm_disk(&paths_a, &export_path).unwrap();
        assert_eq!(copied as usize, disk_bytes.len());
        assert_eq!(
            sha256_file_hex(&export_path).unwrap(),
            sha256_hex(disk_bytes)
        );

        // Shared leased cache survival across sibling removal.
        let store = ArtifactCacheStore::new(base.join("cache"), 1024 * 1024);
        let project = ProjectIdentity::from_root(Path::new("/work/shared-proj")).unwrap();
        let raw_art = base.join("raw-art.img");
        fs::write(&raw_art, b"shared-base-ext4").unwrap();
        let staged = store
            .stage_guest_output(
                "tx-1",
                br#"{"schema":1,"format":"ext4-raw","size_bytes":16,"guest_notes":null}"#,
                &raw_art,
            )
            .unwrap();
        let rec = store
            .commit_publication(
                staged,
                &project,
                SHA_A,
                CrossProjectEligibility::ProjectPrivateOnly,
                "2026-11-26T19:35:00Z",
            )
            .unwrap();
        store
            .acquire_lease(
                &project,
                &rec.content_sha256,
                &RetentionLeaseV1 {
                    instance_id: "inst-a".into(),
                    instance_generation: 1,
                    acquired_at: "t1".into(),
                },
            )
            .unwrap();
        store
            .acquire_lease(
                &project,
                &rec.content_sha256,
                &RetentionLeaseV1 {
                    instance_id: "inst-b".into(),
                    instance_generation: 1,
                    acquired_at: "t2".into(),
                },
            )
            .unwrap();

        // Remove A: succeeds despite reused PID in state, B disk + cache artifact survive.
        let rep_a = remove_owned_instance(
            &paths_a,
            "inst-a",
            &project,
            Some((&store, &rec.content_sha256)),
        )
        .unwrap();
        assert!(rep_a.removed_instance_dir);
        assert!(rep_a.reclaimed_cache_artifacts.is_empty());
        assert!(paths_b.private_disk.is_file());
        assert!(store.lookup_for_project(&project, SHA_A).unwrap().is_some());

        // Remove B: now last lease is gone so shared cache artifact is reclaimed.
        let rep_b = remove_owned_instance(
            &paths_b,
            "inst-b",
            &project,
            Some((&store, &rec.content_sha256)),
        )
        .unwrap();
        assert_eq!(rep_b.reclaimed_cache_artifacts, vec![rec.content_sha256]);
        assert!(store.lookup_for_project(&project, SHA_A).unwrap().is_none());

        let _ = fs::remove_dir_all(&base);
    }
}
