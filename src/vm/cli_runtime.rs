//! Host-side opt-in VM CLI orchestration (`up --vm`, `down`, `rm`, `status`,
//! `logs`, `ps`, `exec`, `shell`, `proxy`, `ssh-config`, and `runtime` grants).
//!
//! Coordinates the Phase 1–4 `crate::vm` primitives against the real `devcroft`
//! CLI without altering native `process`-tier defaults (`meta.json` stays untouched).

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fmt::{self, Write as _};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::IpAddr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::config::{BrokerDecl, is_valid_name};
use crate::keeper::protocol::{Frame, SpawnRequest};
use crate::lifecycle::{StatePaths, acquire_lifecycle_lock};

use super::authority::{
    ExplorationProfileV1, ProjectAuthorityRequest, UserAuthorityGrants, compile_effective_authority,
};
use super::contract::{
    CONTRACT_SCHEMA_V1, ComponentHealth, DiskAttachment, ErrorLayer, GuestPhase, GuestTarget,
    HostChannels, HostProcessIdentity, LaunchRecordV1, ResourceBudgets, RuntimeKind,
    StopClassification, VmAdapter, VmHealthBreakdown, VmStateRecordV1,
};
use super::grants::{
    ExistingRuntime, ProjectIdentity, RuntimeGrantStore, RuntimePolicyMode, RuntimeRequest,
    resolve_runtime,
};
use super::launcher::{
    HelperArtifactInput, HelperLaunchFileV1, MAX_SOCKET_PATH_BYTES, capture_start_time_ns,
    prepare_instance_layout, spawn_detached_helper, write_private_launch_record,
};
use super::lifecycle::{
    RemovalReport, ShutdownOutcome, VmInstancePaths, VmLifecycleError, VmRollbackPolicy,
    export_retained_vm_disk, guard_new_vm_creation, is_owned_helper_alive, load_vm_state,
    prepare_cold_restart, save_vm_state,
};
use super::manifest::{ArtifactEntry, TrustedArtifactManifestV1, TrustedArtifactsV1};
use super::preparation::CANONICAL_GUEST_WORKSPACE;
use super::proxy_channel::is_forbidden_private_or_local_ip;
use super::qualification::current_host_platform;
use super::sha256::sha256_file_hex;
use super::ssh::VmSshKeyMaterial;
use super::transport::{MAX_CONTROL_LINE, TransportError, VmKeeperClient};

pub const VM_CLI_META_SCHEMA_V1: u32 = 1;
pub const DEFAULT_VM_VCPUS: u8 = 2;
pub const DEFAULT_VM_MEMORY_MIB: u32 = 1024;
pub const PRIVATE_STATE_DISK_BYTES: u64 = 1024 * 1024;
pub const VM_GUEST_EGRESS_PROXY_PORT: u16 = 18080;
const READY_POLL_TIMEOUT: Duration = Duration::from_secs(20);
const CONTROL_IO_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmBrokerSessionEntry {
    pub host: String,
    pub env: String,
    pub endpoint_env: String,
    pub placeholder: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmCliMetadata {
    pub schema: u32,
    pub sandbox_name: String,
    pub project_root: PathBuf,
    pub controller_dir: PathBuf,
    pub instance_dir: PathBuf,
    pub private_disk: PathBuf,
    pub helper_log: PathBuf,
    pub ssh_identity_file: PathBuf,
    pub ssh_known_hosts: PathBuf,
    pub guest_boot_state: u64,
    pub manifest_path: PathBuf,
    pub vcpus: u8,
    pub memory_mib: u32,
    #[serde(default)]
    pub egress_allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_process: Option<HostProcessIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_log: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub broker_sessions: Vec<VmBrokerSessionEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub direct_env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disclosed_credentials: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_disk: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_image: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmCliInstance {
    pub meta: VmCliMetadata,
    pub state: VmStateRecordV1,
}

impl VmCliInstance {
    pub fn control_socket(&self) -> PathBuf {
        self.meta.instance_dir.join("control.sock")
    }

    pub fn keeper_socket(&self) -> PathBuf {
        self.meta.instance_dir.join("keeper.sock")
    }

    pub fn ssh_socket(&self) -> PathBuf {
        self.meta.instance_dir.join("ssh.sock")
    }

    pub fn proxy_socket(&self) -> PathBuf {
        self.meta.instance_dir.join("proxy.sock")
    }

    pub fn state_file(&self) -> PathBuf {
        self.meta.controller_dir.join("vm-state.json")
    }

    pub fn instance_paths(&self) -> VmInstancePaths {
        VmInstancePaths {
            controller_dir: self.meta.controller_dir.clone(),
            instance_dir: self.meta.instance_dir.clone(),
            state_file: self.state_file(),
            private_disk: self.meta.private_disk.clone(),
            control_socket: self.control_socket(),
            keeper_socket: self.keeper_socket(),
        }
    }
}

/// Returns the per-session environment variables injected into VM `exec` /
/// `shell` sessions (`[env.vars]`, `[env.forward]`, `[credentials] env`, plus
/// the loopback proxy URLs and brokered credential placeholders/endpoints when
/// the host egress proxy is active).
pub fn vm_session_env(instance: &VmCliInstance) -> BTreeMap<String, String> {
    let mut env = instance.meta.direct_env.clone();
    if !instance.meta.egress_allow.is_empty() && instance.meta.proxy_process.is_some() {
        for (k, v) in crate::proxy::client_env(VM_GUEST_EGRESS_PROXY_PORT, "") {
            env.insert(k, v);
        }
        for b in &instance.meta.broker_sessions {
            env.insert(b.endpoint_env.clone(), b.endpoint.clone());
            env.insert(b.env.clone(), b.placeholder.clone());
        }
    }
    env
}

#[derive(Debug, Clone)]
pub struct VmUpOptions {
    pub manifest_path: Option<PathBuf>,
    pub vcpus: u8,
    pub memory_mib: u32,
    pub recreate: bool,
    pub rollback_disabled: bool,
    pub network_default_allow: bool,
    pub egress_allow: Vec<String>,
    pub brokers: Vec<BrokerDecl>,
    pub direct_env_vars: BTreeMap<String, String>,
    pub env_forward: Vec<String>,
    pub direct_credentials: Vec<String>,
}

impl Default for VmUpOptions {
    fn default() -> Self {
        Self {
            manifest_path: None,
            vcpus: DEFAULT_VM_VCPUS,
            memory_mib: DEFAULT_VM_MEMORY_MIB,
            recreate: false,
            rollback_disabled: false,
            network_default_allow: false,
            egress_allow: Vec::new(),
            brokers: Vec::new(),
            direct_env_vars: BTreeMap::new(),
            env_forward: Vec::new(),
            direct_credentials: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmUpOutcome {
    pub instance: VmCliInstance,
    pub already_up: bool,
    pub restarted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmLiveStatus {
    pub instance: VmCliInstance,
    pub health: VmHealthBreakdown,
    pub uptime_secs: Option<u64>,
    pub session_count: Option<usize>,
    pub guest_state: Option<u64>,
    pub proxy_alive: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveControlReply {
    pub guest_state: u64,
    pub greeting: String,
}

#[derive(Debug)]
pub enum VmCliError {
    StateIo(String),
    Layered { layer: ErrorLayer, message: String },
}

impl VmCliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            VmCliError::StateIo(_) => 1,
            VmCliError::Layered { layer, .. } => i32::from(layer.exit_code()),
        }
    }

    fn layered(layer: ErrorLayer, message: impl Into<String>) -> Self {
        Self::Layered {
            layer,
            message: message.into(),
        }
    }
}

impl fmt::Display for VmCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmCliError::StateIo(msg) => write!(f, "state: {msg}"),
            VmCliError::Layered { layer, message } => {
                write!(f, "{}: {message}", layer.layer_name())
            }
        }
    }
}

impl std::error::Error for VmCliError {}

impl From<VmLifecycleError> for VmCliError {
    fn from(value: VmLifecycleError) -> Self {
        Self::layered(value.layer(), value.to_string())
    }
}

fn data_dir() -> Result<PathBuf, VmCliError> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| VmCliError::StateIo("HOME is not set".to_string()))?;
    Ok(PathBuf::from(home).join(".local/share/devcroft"))
}

pub fn vm_controller_dir(sandbox_name: &str) -> Result<PathBuf, VmCliError> {
    if !is_valid_name(sandbox_name) {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            format!("'{sandbox_name}' is not a valid sandbox name ([a-z0-9][a-z0-9-]{{0,31}})"),
        ));
    }
    Ok(data_dir()?.join(sandbox_name).join("vm-ctrl"))
}

fn cli_meta_path(controller_dir: &Path) -> PathBuf {
    controller_dir.join("vm-cli.json")
}

fn vm_state_path(controller_dir: &Path) -> PathBuf {
    controller_dir.join("vm-state.json")
}

fn load_cli_metadata(path: &Path) -> Result<Option<VmCliMetadata>, VmCliError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(VmCliError::StateIo(format!("{}: {e}", path.display()))),
    };
    let parsed: VmCliMetadata = serde_json::from_slice(&bytes).map_err(|e| {
        VmCliError::layered(ErrorLayer::Config, format!("invalid vm-cli.json: {e}"))
    })?;
    if parsed.schema != VM_CLI_META_SCHEMA_V1 {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            format!("unsupported vm-cli.json schema {}", parsed.schema),
        ));
    }
    Ok(Some(parsed))
}

fn save_cli_metadata(path: &Path, meta: &VmCliMetadata) -> Result<(), VmCliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| VmCliError::StateIo(e.to_string()))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|e| VmCliError::StateIo(e.to_string()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(meta).map_err(|e| VmCliError::StateIo(e.to_string()))?;
    {
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| VmCliError::StateIo(e.to_string()))?;
        f.write_all(&body)
            .map_err(|e| VmCliError::StateIo(e.to_string()))?;
        f.sync_data()
            .map_err(|e| VmCliError::StateIo(e.to_string()))?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
        .map_err(|e| VmCliError::StateIo(e.to_string()))?;
    fs::rename(&tmp, path).map_err(|e| VmCliError::StateIo(e.to_string()))?;
    Ok(())
}

/// Loads VM state + CLI metadata for `sandbox_name` if a VM instance exists.
pub fn load_vm_instance(sandbox_name: &str) -> Result<Option<VmCliInstance>, VmCliError> {
    let ctrl = vm_controller_dir(sandbox_name)?;
    let Some(state) = load_vm_state(&vm_state_path(&ctrl))? else {
        return Ok(None);
    };
    let Some(meta) = load_cli_metadata(&cli_meta_path(&ctrl))? else {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            format!(
                "missing {} alongside {}",
                cli_meta_path(&ctrl).display(),
                vm_state_path(&ctrl).display()
            ),
        ));
    };
    Ok(Some(VmCliInstance { meta, state }))
}

/// Lists all VM instances stored under `~/.local/share/devcroft`.
pub fn list_vm_instances() -> Result<Vec<VmCliInstance>, VmCliError> {
    let root = data_dir()?;
    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(VmCliError::StateIo(format!("{}: {e}", root.display()))),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_valid_name(&name) {
            continue;
        }
        if let Some(inst) = load_vm_instance(&name)? {
            out.push(inst);
        }
    }
    out.sort_by(|a, b| a.meta.sandbox_name.cmp(&b.meta.sandbox_name));
    Ok(out)
}

/// Resolves `(manifest_file, trusted_root)` from `--vm-manifest`,
/// `DEVCROFT_VM_MANIFEST`, or `~/.local/share/devcroft/vm-artifacts/manifest.json`.
pub fn resolve_vm_manifest_path(flag: Option<&Path>) -> Result<(PathBuf, PathBuf), VmCliError> {
    let raw = if let Some(p) = flag {
        p.to_path_buf()
    } else if let Some(env_p) = std::env::var_os("DEVCROFT_VM_MANIFEST").filter(|v| !v.is_empty()) {
        PathBuf::from(env_p)
    } else {
        data_dir()?.join("vm-artifacts").join("manifest.json")
    };

    let cwd = std::env::current_dir().map_err(|e| VmCliError::StateIo(e.to_string()))?;
    let abs = if raw.is_absolute() {
        raw
    } else {
        cwd.join(raw)
    };
    let manifest_file = if abs.is_dir() {
        abs.join("manifest.json")
    } else {
        abs
    };
    if !manifest_file.is_file() {
        return Err(VmCliError::layered(
            ErrorLayer::Backend,
            format!(
                "trusted VM artifact manifest {} not found; pass `--vm-manifest <path>` or set DEVCROFT_VM_MANIFEST",
                manifest_file.display()
            ),
        ));
    }
    let trusted_root = manifest_file
        .parent()
        .ok_or_else(|| {
            VmCliError::layered(ErrorLayer::Config, "VM manifest path has no parent dir")
        })?
        .to_path_buf();
    Ok((manifest_file, trusted_root))
}

fn choose_instance_dir(controller_dir: &Path) -> Result<PathBuf, VmCliError> {
    if let Some(parent) = controller_dir.parent() {
        fs::create_dir_all(parent).map_err(|e| VmCliError::StateIo(e.to_string()))?;
        let canon_parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        let candidate = canon_parent.join("vm-inst");
        if candidate.join("control.sock").as_os_str().len() <= MAX_SOCKET_PATH_BYTES {
            return Ok(candidate);
        }
    }

    // macOS `HOME` or test temp roots can make `<data_dir>/<name>/vm-inst/control.sock`
    // exceed `sun_path` (104 bytes on Darwin). Fall back to a per-uid private directory
    // under `/tmp` whose canonical path (`/private/tmp/dc-vm-<uid>-<hash12>/control.sock`)
    // is always ~54 bytes (`<= 100`).
    let digest = super::sha256::sha256_hex(controller_dir.to_string_lossy().as_bytes());
    let uid = unsafe { libc::geteuid() };
    let tmp_base = fs::canonicalize(Path::new("/tmp")).unwrap_or_else(|_| PathBuf::from("/tmp"));
    let fallback = tmp_base.join(format!("dc-vm-{uid}-{}", &digest[..12]));
    if fallback.join("control.sock").as_os_str().len() > MAX_SOCKET_PATH_BYTES {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            format!(
                "instance socket path {} exceeds {MAX_SOCKET_PATH_BYTES} bytes",
                fallback.join("control.sock").display()
            ),
        ));
    }
    Ok(fallback)
}

fn random_hex(bytes: usize) -> String {
    let mut raw = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut raw);
    let mut out = String::with_capacity(bytes * 2);
    for b in raw {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn parse_guest_state_from_ready(greeting: &str) -> Option<u64> {
    let rest = greeting.strip_prefix("READY ")?;
    for tok in rest.split_whitespace() {
        if let Some(val) = tok.strip_prefix("state=") {
            return val.parse::<u64>().ok();
        }
    }
    None
}

/// Connects to `guest/init.c`'s vsock control channel, reads the initial `READY 1 state=<N>`
/// greeting, and verifies a `PING <nonce>` / `PONG <nonce>` round-trip.
pub fn probe_live_control(
    control_socket: &Path,
    timeout: Duration,
) -> Result<LiveControlReply, TransportError> {
    let mut stream = UnixStream::connect(control_socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut greeting = String::new();
    let n = (&mut reader)
        .take((MAX_CONTROL_LINE + 2) as u64)
        .read_line(&mut greeting)?;
    if n == 0 || !greeting.ends_with('\n') {
        return Err(TransportError::ControlProtocol(
            "control socket closed before READY greeting".into(),
        ));
    }
    let greeting_trimmed = greeting.trim_end_matches(['\r', '\n']).to_owned();
    let guest_state = parse_guest_state_from_ready(&greeting_trimmed).ok_or_else(|| {
        TransportError::ControlProtocol(format!("unexpected control greeting `{greeting_trimmed}`"))
    })?;

    let token = random_hex(8);
    let req = format!("PING {token}\n");
    stream.write_all(req.as_bytes())?;
    stream.flush()?;

    let mut reply = String::new();
    let rn = reader
        .take((MAX_CONTROL_LINE + 2) as u64)
        .read_line(&mut reply)?;
    if rn == 0 || !reply.ends_with('\n') {
        return Err(TransportError::ControlProtocol(
            "control socket closed before PONG reply".into(),
        ));
    }
    let expected = format!("PONG {token}");
    let reply_trimmed = reply.trim_end_matches(['\r', '\n']);
    if reply_trimmed != expected {
        return Err(TransportError::ControlProtocol(format!(
            "unexpected PING reply `{reply_trimmed}` (expected `{expected}`)"
        )));
    }

    Ok(LiveControlReply {
        guest_state,
        greeting: greeting_trimmed,
    })
}

/// Sends `SHUTDOWN <state> <64-hex-nonce>` to `guest/init.c` and verifies `SYNCED <state> <nonce>`.
pub fn send_live_shutdown(
    control_socket: &Path,
    nonce_64_hex: &str,
    timeout: Duration,
) -> Result<(u64, String), TransportError> {
    let mut stream = UnixStream::connect(control_socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut greeting = String::new();
    let n = (&mut reader)
        .take((MAX_CONTROL_LINE + 2) as u64)
        .read_line(&mut greeting)?;
    if n == 0 || !greeting.ends_with('\n') {
        return Err(TransportError::ControlProtocol(
            "control socket closed before READY greeting".into(),
        ));
    }
    let greeting_trimmed = greeting.trim_end_matches(['\r', '\n']);
    let guest_state = parse_guest_state_from_ready(greeting_trimmed).ok_or_else(|| {
        TransportError::ControlProtocol(format!("unexpected control greeting `{greeting_trimmed}`"))
    })?;

    let req = format!("SHUTDOWN {guest_state} {nonce_64_hex}\n");
    stream.write_all(req.as_bytes())?;
    stream.flush()?;

    let mut reply = String::new();
    let rn = reader
        .take((MAX_CONTROL_LINE + 2) as u64)
        .read_line(&mut reply)?;
    if rn == 0 || !reply.ends_with('\n') {
        return Err(TransportError::ControlProtocol(
            "control socket closed before SYNCED acknowledgement".into(),
        ));
    }
    let reply_trimmed = reply.trim_end_matches(['\r', '\n']).to_owned();
    let expected = format!("SYNCED {guest_state} {nonce_64_hex}");
    if reply_trimmed != expected {
        return Err(TransportError::ControlProtocol(format!(
            "shutdown rejected or mismatched: got `{reply_trimmed}`, expected `{expected}`"
        )));
    }
    Ok((guest_state, reply_trimmed))
}

fn log_tail(path: &Path, max_lines: usize) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn verify_artifact_file(
    label: &str,
    path: &Path,
    expected_sha256: &str,
) -> Result<(PathBuf, String), VmCliError> {
    if !path.is_file() {
        return Err(VmCliError::layered(
            ErrorLayer::Backend,
            format!("trusted VM {label} artifact {} not found", path.display()),
        ));
    }
    let canonical = fs::canonicalize(path).map_err(|e| {
        VmCliError::layered(ErrorLayer::Backend, format!("{}: {e}", path.display()))
    })?;
    let actual = sha256_file_hex(&canonical).map_err(|e| {
        VmCliError::layered(ErrorLayer::Backend, format!("{}: {e}", canonical.display()))
    })?;
    if actual != expected_sha256.to_ascii_lowercase() {
        return Err(VmCliError::layered(
            ErrorLayer::Backend,
            format!(
                "trusted VM {label} artifact {} SHA-256 mismatch (expected {}, got {actual})",
                canonical.display(),
                expected_sha256
            ),
        ));
    }
    Ok((canonical, actual))
}

fn create_zeroed_private_disk(path: &Path, size_bytes: u64) -> Result<(), VmCliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| VmCliError::StateIo(e.to_string()))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|e| VmCliError::StateIo(e.to_string()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", path.display())))?;
    file.set_len(size_bytes)
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", path.display())))?;
    file.sync_data()
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", path.display())))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", path.display())))?;
    Ok(())
}

fn cleanup_stale_sockets(instance_dir: &Path) {
    for sock in ["control.sock", "keeper.sock", "ssh.sock", "proxy.sock"] {
        let p = instance_dir.join(sock);
        if p.symlink_metadata().is_ok() {
            let _ = fs::remove_file(p);
        }
    }
}

fn validate_vm_egress_allow(allow: &[String]) -> Result<(), VmCliError> {
    for entry in allow {
        let trimmed = entry.trim();
        let host = trimmed.trim_end_matches('.');
        if host.eq_ignore_ascii_case("localhost")
            || host.to_ascii_lowercase().ends_with(".localhost")
        {
            return Err(VmCliError::layered(
                ErrorLayer::Config,
                format!("network.allow entry `{entry}` targets localhost, which VM egress refuses"),
            ));
        }
        if let Ok(ip) = host.parse::<IpAddr>()
            && is_forbidden_private_or_local_ip(ip)
        {
            return Err(VmCliError::layered(
                ErrorLayer::Config,
                format!(
                    "network.allow entry `{entry}` targets a private or local IP (`{ip}`), which VM egress refuses"
                ),
            ));
        }
    }
    Ok(())
}

fn check_vm_brokers(opts: &VmUpOptions) -> Result<(), VmCliError> {
    if opts.brokers.is_empty() {
        return Ok(());
    }
    let filter = nono::HostFilter::new_strict(&opts.egress_allow);
    for b in &opts.brokers {
        let key = format!("credentials.broker (`{}`)", b.env);
        if !filter.check_host(&b.host, &[]).is_allowed() {
            return Err(VmCliError::layered(
                ErrorLayer::Config,
                format!(
                    "{key}: `{}` is not in this project's `network.allow`; brokering grants no egress of its own, so add the host there",
                    b.host
                ),
            ));
        }
        for name in [&b.env, &b.endpoint_env] {
            if opts.direct_env_vars.contains_key(name) {
                return Err(VmCliError::layered(
                    ErrorLayer::Config,
                    format!(
                        "{key}: `{name}` is also set by this project's `[env.vars]`; remove it from whichever you did not mean"
                    ),
                ));
            }
        }
        if env::var_os(&b.env).is_none() {
            return Err(VmCliError::layered(
                ErrorLayer::Config,
                format!(
                    "{key}: `{}` is not set in this shell, and the proxy reads the real value from it; set it, or remove the entry from your user configuration",
                    b.env
                ),
            ));
        }
    }
    Ok(())
}

fn validate_and_resolve_direct_env(
    opts: &VmUpOptions,
) -> Result<(BTreeMap<String, String>, Vec<String>), VmCliError> {
    for name in &opts.direct_credentials {
        if opts.direct_env_vars.contains_key(name) {
            return Err(VmCliError::layered(
                ErrorLayer::Config,
                format!(
                    "`{name}` is requested by your `[credentials] env` and also set by this project's `[env.vars]`"
                ),
            ));
        }
        if env::var_os(name).is_none() {
            return Err(VmCliError::layered(
                ErrorLayer::Config,
                format!(
                    "`{name}` is requested by your `[credentials] env` but is not set in this shell; set it, or remove it from your user configuration"
                ),
            ));
        }
    }
    let mut resolved = opts.direct_env_vars.clone();
    for name in &opts.env_forward {
        if let Ok(val) = env::var(name) {
            resolved.insert(name.clone(), val);
        }
    }
    for name in &opts.direct_credentials {
        resolved.insert(name.clone(), env::var(name).unwrap_or_default());
    }
    Ok((resolved, opts.direct_credentials.clone()))
}

fn brokers_match_session(recorded: &[VmBrokerSessionEntry], configured: &[BrokerDecl]) -> bool {
    recorded.len() == configured.len()
        && recorded
            .iter()
            .zip(configured.iter())
            .all(|(r, c)| r.host == c.host && r.env == c.env && r.endpoint_env == c.endpoint_env)
}

fn terminate_owned_helper_group(id: &HostProcessIdentity, grace: Duration) {
    if !is_owned_helper_alive(id) {
        return;
    }
    let pid = id.pid as libc::pid_t;
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + grace;
    while is_owned_helper_alive(id) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    if is_owned_helper_alive(id) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let kill_deadline = Instant::now() + Duration::from_millis(500);
        while is_owned_helper_alive(id) && Instant::now() < kill_deadline {
            thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Brings up or cold-restarts an opt-in VM sandbox on a qualified host.
pub fn up_vm_instance(
    sandbox_name: &str,
    project_root: &Path,
    provider: &str,
    opts: &VmUpOptions,
) -> Result<VmUpOutcome, VmCliError> {
    if !(1..=8).contains(&opts.vcpus) || !(128..=4096).contains(&opts.memory_mib) {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            "VM resources require --vm-vcpus in 1..=8 and --vm-memory in 128..=4096 MiB",
        ));
    }
    if opts.network_default_allow {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            "VM sandboxes do not allow unrestricted `network.default = \"allow\"`; use `network.default = \"deny\"` with explicit `network.allow` hosts via the host egress proxy",
        ));
    }
    validate_vm_egress_allow(&opts.egress_allow)?;
    let (resolved_direct_env, disclosed_credentials) = validate_and_resolve_direct_env(opts)?;
    check_vm_brokers(opts)?;

    let state_paths = StatePaths::new(sandbox_name)
        .map_err(|e| VmCliError::layered(ErrorLayer::Config, e.to_string()))?;
    let project = ProjectIdentity::from_root(project_root)
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
    let grants =
        RuntimeGrantStore::load().map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;

    let ctrl_dir = vm_controller_dir(sandbox_name)?;
    let existing_vm = load_vm_instance(sandbox_name)?;
    let native_meta_exists = state_paths.meta.is_file() && existing_vm.is_none();
    if native_meta_exists {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            format!(
                "sandbox '{sandbox_name}' already exists as a native process-tier instance; run `devcroft rm {sandbox_name}` or use `--name` before starting a VM instance"
            ),
        ));
    }

    let existing_runtime = existing_vm
        .as_ref()
        .map(|inst| match inst.state.runtime_kind {
            RuntimeKind::Native => ExistingRuntime::Native,
            RuntimeKind::Vm { target, adapter } => ExistingRuntime::Vm { target, adapter },
        });

    guard_new_vm_creation(
        VmRollbackPolicy {
            creation_disabled: opts.rollback_disabled,
        },
        existing_runtime,
    )?;

    let decision = resolve_runtime(&RuntimeRequest {
        project: &project,
        provider,
        existing_instance: existing_runtime,
        explicit_vm_opt_in: true,
        policy_mode: RuntimePolicyMode::OptInVm,
        grants: &grants,
        host: current_host_platform(),
        stdin_is_terminal: false,
    })
    .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;

    let (guest_target, vm_adapter) = match decision.runtime {
        RuntimeKind::Vm { target, adapter } => (target, adapter),
        RuntimeKind::Native => {
            return Err(VmCliError::layered(
                ErrorLayer::Config,
                format!(
                    "sandbox '{sandbox_name}' is already bound to the native runtime; cannot switch to VM"
                ),
            ));
        }
    };

    let _lock = acquire_lifecycle_lock(&state_paths.lifecycle_lock)
        .map_err(|e| VmCliError::StateIo(format!("acquiring lifecycle lock: {e}")))?;

    // Check if an existing VM instance is already healthy with matching egress/env config.
    if let Some(existing) = existing_vm.clone() {
        let helper_alive = existing
            .state
            .host_process
            .as_ref()
            .is_some_and(is_owned_helper_alive);
        let proxy_ok = if opts.egress_allow.is_empty() {
            existing.meta.egress_allow.is_empty()
        } else {
            existing.meta.egress_allow == opts.egress_allow
                && existing
                    .meta
                    .proxy_process
                    .as_ref()
                    .is_some_and(is_owned_helper_alive)
        };
        let same_env = brokers_match_session(&existing.meta.broker_sessions, &opts.brokers)
            && existing.meta.direct_env == resolved_direct_env
            && existing.meta.disclosed_credentials == disclosed_credentials;
        if helper_alive && proxy_ok && same_env && !opts.recreate {
            let live_ctrl = probe_live_control(&existing.control_socket(), CONTROL_IO_TIMEOUT);
            let live_keeper =
                VmKeeperClient::new(existing.keeper_socket(), CONTROL_IO_TIMEOUT).query();
            if live_ctrl.is_ok() && live_keeper.is_ok() {
                return Ok(VmUpOutcome {
                    instance: existing,
                    already_up: true,
                    restarted: false,
                });
            }
        }
    }

    // Resolve and strictly verify trusted artifacts BEFORE creating/restarting instance disks.
    let (manifest_path, trusted_root) = resolve_vm_manifest_path(opts.manifest_path.as_deref())?;
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|e| {
        VmCliError::layered(
            ErrorLayer::Backend,
            format!("{}: {e}", manifest_path.display()),
        )
    })?;
    let artifact_manifest = TrustedArtifactManifestV1::from_json_str(&manifest_text)
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
    artifact_manifest
        .check_controller_compatible(env!("CARGO_PKG_VERSION"))
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
    let packaged = artifact_manifest
        .packaged_layout(&trusted_root, project.as_path())
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;

    let (helper_path, _) = verify_artifact_file(
        "helper",
        &packaged.helper,
        &artifact_manifest.artifacts.helper.sha256,
    )?;
    let (kernel_path, kernel_sha) = verify_artifact_file(
        "kernel",
        &packaged.kernel,
        &artifact_manifest.artifacts.kernel.sha256,
    )?;
    let (initramfs_path, initramfs_sha) = verify_artifact_file(
        "initramfs",
        &packaged.initramfs,
        &artifact_manifest.artifacts.initramfs.sha256,
    )?;
    let (runtime_path, runtime_sha) = verify_artifact_file(
        "runtime_image",
        &packaged.runtime_image,
        &artifact_manifest.artifacts.runtime_image.sha256,
    )?;
    let (workload_path, workload_sha, data_seed_path) = match (
        &artifact_manifest.artifacts.workload,
        &packaged.workload,
        &artifact_manifest.artifacts.data_seed,
        &packaged.data_seed,
    ) {
        (Some(w_entry), Some(w_path), Some(d_entry), Some(d_path)) => {
            let (w_canon, w_sha) = verify_artifact_file("workload", w_path, &w_entry.sha256)?;
            let (d_canon, _) = verify_artifact_file("data_seed", d_path, &d_entry.sha256)?;
            (Some(w_canon), Some(w_sha), Some(d_canon))
        }
        _ => (None, None, None),
    };

    let boot_sha_input = match &workload_sha {
        Some(w_sha) => format!("{kernel_sha}:{initramfs_sha}:{runtime_sha}:{w_sha}"),
        None => format!("{kernel_sha}:{initramfs_sha}:{runtime_sha}"),
    };
    let boot_artifact_sha256 = super::sha256::sha256_hex(boot_sha_input.as_bytes());
    let private_disk_id = format!("vm-state-{sandbox_name}");

    if opts.recreate
        && let Some(existing) = existing_vm.as_ref()
    {
        if existing
            .state
            .host_process
            .as_ref()
            .is_some_and(is_owned_helper_alive)
            || existing
                .meta
                .proxy_process
                .as_ref()
                .is_some_and(is_owned_helper_alive)
        {
            down_vm_instance_inner(existing)?;
        }
        let _ = fs::remove_file(&existing.meta.private_disk);
        if let Some(ws) = &existing.meta.workspace_disk {
            let _ = fs::remove_file(ws);
        }
        let _ = fs::remove_file(existing.state_file());
        let _ = fs::remove_file(cli_meta_path(&existing.meta.controller_dir));
    }

    let (
        instance_dir,
        private_disk,
        workspace_disk,
        next_generation,
        ssh_identity_file,
        ssh_known_hosts,
        restarted,
    ) = if !opts.recreate
        && let Some(existing) = load_vm_instance(sandbox_name)?
    {
        // Clean dead/stale helper or proxy sockets before cold restart.
        if existing
            .state
            .host_process
            .as_ref()
            .is_some_and(is_owned_helper_alive)
            || existing
                .meta
                .proxy_process
                .as_ref()
                .is_some_and(is_owned_helper_alive)
        {
            down_vm_instance_inner(&existing)?;
        }
        let plan = prepare_cold_restart(
            &existing.instance_paths(),
            guest_target,
            vm_adapter,
            &private_disk_id,
        )?;
        cleanup_stale_sockets(&existing.meta.instance_dir);
        let canonical_disk = fs::canonicalize(&plan.preserved_private_disk)
            .map_err(|e| VmCliError::StateIo(e.to_string()))?;
        let preserved_workspace = match (
            data_seed_path.as_ref(),
            existing.meta.workspace_disk.as_ref(),
        ) {
            (Some(_), Some(ws)) => {
                if !ws.is_file() {
                    return Err(VmCliError::layered(
                        ErrorLayer::Config,
                        format!("missing persistent VM workspace disk {}", ws.display()),
                    ));
                }
                Some(fs::canonicalize(ws).map_err(|e| VmCliError::StateIo(e.to_string()))?)
            }
            (Some(seed), None) => {
                let ws_path = existing.meta.instance_dir.join("workspace.img");
                copy_regular_artifact("workspace_disk", seed, &ws_path, 0o600)?;
                Some(fs::canonicalize(&ws_path).map_err(|e| VmCliError::StateIo(e.to_string()))?)
            }
            (None, _) => None,
        };
        (
            existing.meta.instance_dir,
            canonical_disk,
            preserved_workspace,
            plan.next_generation,
            existing.meta.ssh_identity_file,
            existing.meta.ssh_known_hosts,
            true,
        )
    } else {
        let inst_dir = choose_instance_dir(&ctrl_dir)?;
        if inst_dir.exists() {
            cleanup_stale_sockets(&inst_dir);
        }
        let endpoints = prepare_instance_layout(&ctrl_dir, &inst_dir)
            .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
        let disk_path = endpoints.instance_dir.join("state.img");
        create_zeroed_private_disk(&disk_path, PRIVATE_STATE_DISK_BYTES)?;
        let instance_id_hex = random_hex(16);
        let ssh_keys = VmSshKeyMaterial::generate(
            &endpoints.controller_dir,
            &format!("{sandbox_name}.devcroft"),
            &instance_id_hex,
        )
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
        ssh_keys
            .bootstrap_seed
            .write_to_disk(&disk_path)
            .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
        let canonical_disk =
            fs::canonicalize(&disk_path).map_err(|e| VmCliError::StateIo(e.to_string()))?;
        let seeded_workspace = match data_seed_path.as_ref() {
            Some(seed) => {
                let ws_path = endpoints.instance_dir.join("workspace.img");
                copy_regular_artifact("workspace_disk", seed, &ws_path, 0o600)?;
                Some(fs::canonicalize(&ws_path).map_err(|e| VmCliError::StateIo(e.to_string()))?)
            }
            None => None,
        };
        (
            endpoints.instance_dir,
            canonical_disk,
            seeded_workspace,
            1u64,
            ssh_keys.client_private_key,
            ssh_keys.known_hosts,
            false,
        )
    };

    cleanup_stale_sockets(&instance_dir);
    let endpoints = prepare_instance_layout(&ctrl_dir, &instance_dir)
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;

    let (proxy_proc, proxy_log, broker_sessions, launch_proxy_socket) =
        if opts.egress_allow.is_empty() {
            (None, None, Vec::new(), None)
        } else {
            let exe = env::current_exe().map_err(|e| {
                VmCliError::layered(
                    ErrorLayer::Backend,
                    format!("resolving current devcroft binary for egress proxy: {e}"),
                )
            })?;
            let proxy_socket = endpoints.instance_dir.join("proxy.sock");
            let proxy_log_path = endpoints.controller_dir.join("proxy.log");
            let routes: Vec<crate::proxy::BrokerRoute> = opts
                .brokers
                .iter()
                .enumerate()
                .map(|(i, b)| crate::proxy::BrokerRoute {
                    prefix: crate::proxy::broker_prefix(i),
                    host: b.host.clone(),
                    header: b.header.clone(),
                    env: b.env.clone(),
                    placeholder: b.placeholder.clone(),
                    tls_ca: None,
                    value: env::var(&b.env).unwrap_or_default(),
                })
                .collect();
            let spawned = crate::proxy::spawn_at(
                &exe,
                &proxy_socket,
                &proxy_log_path,
                &opts.egress_allow,
                &routes,
                |_| Ok(()),
            );
            drop(routes);
            let (proxy_pid, _tcp_port, token) = spawned.map_err(|e| {
                VmCliError::layered(
                    ErrorLayer::Backend,
                    format!("starting host egress proxy: {e}"),
                )
            })?;
            let token = zeroize::Zeroizing::new(token);
            let start_time_ns = capture_start_time_ns(proxy_pid as u32).unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0)
            });
            let proxy_id = HostProcessIdentity {
                pid: proxy_pid as u32,
                start_time_ns,
            };
            let mut sessions = Vec::with_capacity(opts.brokers.len());
            for (i, b) in opts.brokers.iter().enumerate() {
                let prefix = crate::proxy::broker_prefix(i);
                let endpoint = crate::proxy::broker_endpoint(VM_GUEST_EGRESS_PROXY_PORT, &prefix);
                let Some(placeholder) =
                    crate::proxy::broker_phantom(&token, b.placeholder.as_deref())
                else {
                    terminate_owned_helper_group(&proxy_id, Duration::from_secs(1));
                    cleanup_stale_sockets(&endpoints.instance_dir);
                    return Err(VmCliError::layered(
                        ErrorLayer::Config,
                        format!(
                            "credentials.broker: `{}`'s placeholder does not render",
                            b.env
                        ),
                    ));
                };
                sessions.push(VmBrokerSessionEntry {
                    host: b.host.clone(),
                    env: b.env.clone(),
                    endpoint_env: b.endpoint_env.clone(),
                    placeholder,
                    endpoint,
                });
            }
            (
                Some(proxy_id),
                Some(proxy_log_path),
                sessions,
                Some(proxy_socket),
            )
        };

    let launch = HelperLaunchFileV1 {
        schema: 1,
        kernel: HelperArtifactInput {
            path: kernel_path,
            sha256: kernel_sha,
        },
        initramfs: HelperArtifactInput {
            path: initramfs_path,
            sha256: initramfs_sha,
        },
        runtime: HelperArtifactInput {
            path: runtime_path,
            sha256: runtime_sha,
        },
        workload: match (&workload_path, &workload_sha) {
            (Some(path), Some(sha)) => Some(HelperArtifactInput {
                path: path.clone(),
                sha256: sha.clone(),
            }),
            _ => None,
        },
        data: workspace_disk.clone(),
        state: private_disk.clone(),
        instance_dir: endpoints.instance_dir.clone(),
        proxy_socket: launch_proxy_socket,
        vcpus: opts.vcpus,
        memory_mib: opts.memory_mib,
    };
    let launch_path = match write_private_launch_record(
        &endpoints.controller_dir,
        &endpoints.instance_dir,
        "launch.json",
        &launch,
    ) {
        Ok(p) => p,
        Err(e) => {
            if let Some(proxy_id) = proxy_proc.as_ref() {
                terminate_owned_helper_group(proxy_id, Duration::from_secs(1));
            }
            cleanup_stale_sockets(&endpoints.instance_dir);
            return Err(VmCliError::layered(e.layer(), e.to_string()));
        }
    };

    let helper_log = endpoints.controller_dir.join("helper.log");
    let host_proc = match spawn_detached_helper(
        &helper_path,
        &[OsStr::new("run"), launch_path.as_os_str()],
        &helper_log,
    ) {
        Ok(proc_id) => proc_id,
        Err(e) => {
            if let Some(proxy_id) = proxy_proc.as_ref() {
                terminate_owned_helper_group(proxy_id, Duration::from_secs(1));
            }
            cleanup_stale_sockets(&endpoints.instance_dir);
            return Err(VmCliError::layered(e.layer(), e.to_string()));
        }
    };

    let deadline = Instant::now() + READY_POLL_TIMEOUT;
    let observed_guest_state = loop {
        if !is_owned_helper_alive(&host_proc) {
            if let Some(proxy_id) = proxy_proc.as_ref() {
                terminate_owned_helper_group(proxy_id, Duration::from_secs(1));
            }
            cleanup_stale_sockets(&endpoints.instance_dir);
            let tail = log_tail(&helper_log, 20);
            return Err(VmCliError::layered(
                ErrorLayer::Backend,
                if tail.is_empty() {
                    "VM helper exited before readiness handshake completed".to_string()
                } else {
                    format!("VM helper exited before readiness:\n{tail}")
                },
            ));
        }
        if let Ok(ctrl_reply) = probe_live_control(&endpoints.control_socket, CONTROL_IO_TIMEOUT)
            && VmKeeperClient::new(&endpoints.keeper_socket, CONTROL_IO_TIMEOUT)
                .query()
                .is_ok()
        {
            break ctrl_reply.guest_state;
        }
        if Instant::now() >= deadline {
            terminate_owned_helper_group(&host_proc, Duration::from_secs(1));
            if let Some(proxy_id) = proxy_proc.as_ref() {
                terminate_owned_helper_group(proxy_id, Duration::from_secs(1));
            }
            cleanup_stale_sockets(&endpoints.instance_dir);
            let tail = log_tail(&helper_log, 20);
            return Err(VmCliError::layered(
                ErrorLayer::Keeper,
                if tail.is_empty() {
                    format!(
                        "timed out waiting for VM readiness on {}",
                        endpoints.control_socket.display()
                    )
                } else {
                    format!("timed out waiting for VM readiness:\n{tail}")
                },
            ));
        }
        thread::sleep(Duration::from_millis(50));
    };

    let state = VmStateRecordV1 {
        schema: CONTRACT_SCHEMA_V1,
        runtime_kind: RuntimeKind::Vm {
            target: guest_target,
            adapter: vm_adapter,
        },
        selection_source: decision.selected_by,
        instance_generation: next_generation,
        host_process: Some(host_proc),
        private_disk_id,
        boot_artifact_sha256,
        health: VmHealthBreakdown {
            helper: ComponentHealth::Healthy,
            transport: ComponentHealth::Healthy,
            keeper: ComponentHealth::Healthy,
            environment_fresh: true,
            services: ComponentHealth::Stopped,
        },
        last_stop: None,
    };

    let cli_meta = VmCliMetadata {
        schema: VM_CLI_META_SCHEMA_V1,
        sandbox_name: sandbox_name.to_owned(),
        project_root: project.as_path().to_path_buf(),
        controller_dir: endpoints.controller_dir.clone(),
        instance_dir: endpoints.instance_dir,
        private_disk,
        helper_log,
        ssh_identity_file,
        ssh_known_hosts,
        guest_boot_state: observed_guest_state,
        manifest_path: fs::canonicalize(&manifest_path).unwrap_or(manifest_path),
        vcpus: opts.vcpus,
        memory_mib: opts.memory_mib,
        egress_allow: opts.egress_allow.clone(),
        proxy_process: proxy_proc,
        proxy_log,
        broker_sessions,
        direct_env: resolved_direct_env,
        disclosed_credentials,
        workspace_disk,
        workload_image: workload_path,
    };

    save_vm_state(&vm_state_path(&endpoints.controller_dir), &state)?;
    save_cli_metadata(&cli_meta_path(&endpoints.controller_dir), &cli_meta)?;

    Ok(VmUpOutcome {
        instance: VmCliInstance {
            meta: cli_meta,
            state,
        },
        already_up: false,
        restarted,
    })
}

/// Probes live helper, control channel, and guest keeper independently.
pub fn status_vm_instance(sandbox_name: &str) -> Result<VmLiveStatus, VmCliError> {
    let instance = load_vm_instance(sandbox_name)?.ok_or_else(|| {
        VmCliError::layered(
            ErrorLayer::Config,
            format!("VM sandbox '{sandbox_name}' not found"),
        )
    })?;

    let helper = match &instance.state.host_process {
        Some(id) if is_owned_helper_alive(id) => ComponentHealth::Healthy,
        _ => ComponentHealth::Stopped,
    };

    let (transport, guest_state) =
        match probe_live_control(&instance.control_socket(), CONTROL_IO_TIMEOUT) {
            Ok(reply) => (ComponentHealth::Healthy, Some(reply.guest_state)),
            Err(TransportError::ControlProtocol(_)) if helper == ComponentHealth::Healthy => {
                (ComponentHealth::Unhealthy, None)
            }
            Err(_) => (ComponentHealth::Stopped, None),
        };

    let (keeper, uptime_secs, session_count) =
        match VmKeeperClient::new(instance.keeper_socket(), CONTROL_IO_TIMEOUT).query() {
            Ok(res) => (
                ComponentHealth::Healthy,
                Some(res.uptime_secs),
                Some(res.sessions.len()),
            ),
            Err(_) if helper == ComponentHealth::Healthy => {
                (ComponentHealth::Unhealthy, None, None)
            }
            Err(_) => (ComponentHealth::Stopped, None, None),
        };

    let proxy_alive = if instance.meta.egress_allow.is_empty() {
        None
    } else {
        Some(
            instance
                .meta
                .proxy_process
                .as_ref()
                .is_some_and(is_owned_helper_alive),
        )
    };

    let health = VmHealthBreakdown {
        helper,
        transport,
        keeper,
        environment_fresh: instance.state.health.environment_fresh,
        services: instance.state.health.services,
    };

    Ok(VmLiveStatus {
        instance,
        health,
        uptime_secs,
        session_count,
        guest_state,
        proxy_alive,
    })
}

fn down_vm_instance_inner(instance: &VmCliInstance) -> Result<ShutdownOutcome, VmCliError> {
    let paths = instance.instance_paths();
    let mut state = instance.state.clone();
    let helper_alive = state
        .host_process
        .as_ref()
        .is_some_and(is_owned_helper_alive);

    let mut orderly = false;
    let mut control_reply = None;

    if helper_alive || paths.control_socket.exists() {
        let nonce = random_hex(32);
        if let Ok((_guest_state, reply)) =
            send_live_shutdown(&paths.control_socket, &nonce, SHUTDOWN_WAIT_TIMEOUT)
        {
            orderly = true;
            control_reply = Some(reply);
        }
    }

    if let Some(id) = state.host_process.clone()
        && is_owned_helper_alive(&id)
    {
        if orderly {
            let deadline = Instant::now() + SHUTDOWN_WAIT_TIMEOUT;
            while is_owned_helper_alive(&id) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            if is_owned_helper_alive(&id) {
                orderly = false;
                terminate_owned_helper_group(&id, Duration::from_secs(2));
            }
        } else {
            terminate_owned_helper_group(&id, Duration::from_secs(2));
        }
    }

    if let Some(proxy_id) = instance.meta.proxy_process.as_ref()
        && is_owned_helper_alive(proxy_id)
    {
        terminate_owned_helper_group(proxy_id, Duration::from_secs(2));
    }

    let classification = if orderly {
        StopClassification::Orderly
    } else if state.host_process.is_some() {
        StopClassification::ForcedStop
    } else {
        state.last_stop.unwrap_or(StopClassification::Orderly)
    };

    cleanup_stale_sockets(&paths.instance_dir);
    state.last_stop = Some(classification);
    state.host_process = None;
    state.health.helper = ComponentHealth::Stopped;
    state.health.transport = ComponentHealth::Stopped;
    state.health.keeper = ComponentHealth::Stopped;
    state.health.services = ComponentHealth::Stopped;
    save_vm_state(&paths.state_file, &state)?;
    let meta_path = cli_meta_path(&paths.controller_dir);
    if let Some(mut meta) = load_cli_metadata(&meta_path)?
        && meta.proxy_process.is_some()
    {
        meta.proxy_process = None;
        save_cli_metadata(&meta_path, &meta)?;
    }

    Ok(ShutdownOutcome {
        classification,
        private_disk_retained: paths.private_disk.is_file(),
        control_reply,
    })
}

/// Gracefully quiesces and powers off a running VM instance while retaining its
/// private disk for cold restart or rollback export.
pub fn down_vm_instance(sandbox_name: &str) -> Result<ShutdownOutcome, VmCliError> {
    let state_paths = StatePaths::new(sandbox_name)
        .map_err(|e| VmCliError::layered(ErrorLayer::Config, e.to_string()))?;
    let _lock = acquire_lifecycle_lock(&state_paths.lifecycle_lock)
        .map_err(|e| VmCliError::StateIo(format!("acquiring lifecycle lock: {e}")))?;

    let instance = load_vm_instance(sandbox_name)?.ok_or_else(|| {
        VmCliError::layered(
            ErrorLayer::Config,
            format!("VM sandbox '{sandbox_name}' not found"),
        )
    })?;
    down_vm_instance_inner(&instance)
}

/// Stops (if still running) and removes only the owned directories/files of a VM instance.
pub fn rm_vm_instance(sandbox_name: &str) -> Result<RemovalReport, VmCliError> {
    let state_paths = StatePaths::new(sandbox_name)
        .map_err(|e| VmCliError::layered(ErrorLayer::Config, e.to_string()))?;
    let _lock = acquire_lifecycle_lock(&state_paths.lifecycle_lock)
        .map_err(|e| VmCliError::StateIo(format!("acquiring lifecycle lock: {e}")))?;

    let instance = load_vm_instance(sandbox_name)?.ok_or_else(|| {
        VmCliError::layered(
            ErrorLayer::Config,
            format!("VM sandbox '{sandbox_name}' not found"),
        )
    })?;
    if instance
        .state
        .host_process
        .as_ref()
        .is_some_and(is_owned_helper_alive)
        || instance
            .meta
            .proxy_process
            .as_ref()
            .is_some_and(is_owned_helper_alive)
    {
        down_vm_instance_inner(&instance)?;
    }

    let project = ProjectIdentity::from_root(&instance.meta.project_root)
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
    let report = super::lifecycle::remove_owned_instance(
        &instance.instance_paths(),
        sandbox_name,
        &project,
        None,
    )?;

    if state_paths.root.is_dir() {
        let _ = fs::remove_dir(&state_paths.root);
    }
    Ok(report)
}

/// Returns captured helper/guest logs for a VM instance.
pub fn logs_vm_instance(
    sandbox_name: &str,
    tail_lines: Option<usize>,
) -> Result<String, VmCliError> {
    let instance = load_vm_instance(sandbox_name)?.ok_or_else(|| {
        VmCliError::layered(
            ErrorLayer::Config,
            format!("VM sandbox '{sandbox_name}' not found"),
        )
    })?;
    let mut contents = fs::read_to_string(&instance.meta.helper_log)
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", instance.meta.helper_log.display())))?;
    if let Some(proxy_log) = &instance.meta.proxy_log
        && let Ok(proxy_contents) = fs::read_to_string(proxy_log)
    {
        contents.push_str("\n--- egress proxy ---\n");
        contents.push_str(&proxy_contents);
    }
    match tail_lines {
        None => Ok(contents),
        Some(n) => {
            let lines: Vec<&str> = contents.lines().collect();
            let start = lines.len().saturating_sub(n);
            let mut out = lines[start..].join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            Ok(out)
        }
    }
}

/// Exports the retained private disk of a stopped VM instance with SHA-256 verification.
pub fn export_vm_disk(sandbox_name: &str, destination: &Path) -> Result<u64, VmCliError> {
    let instance = load_vm_instance(sandbox_name)?.ok_or_else(|| {
        VmCliError::layered(
            ErrorLayer::Config,
            format!("VM sandbox '{sandbox_name}' not found"),
        )
    })?;
    let abs_dest = if destination.is_absolute() {
        destination.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| VmCliError::StateIo(e.to_string()))?
            .join(destination)
    };
    Ok(export_retained_vm_disk(
        &instance.instance_paths(),
        &abs_dest,
    )?)
}

/// Upserts a per-VM managed SSH block in `~/.ssh/config`, preserving user rules
/// and the native `Host *.devcroft` wildcard block.
pub fn write_vm_ssh_config_section(
    config_path: &Path,
    sandbox_name: &str,
    block: &str,
) -> io::Result<()> {
    let begin = format!("# >>> devcroft-vm {sandbox_name} — do not edit >>>");
    let end = format!("# <<< devcroft-vm {sandbox_name} — do not edit <<<");
    let existing = fs::read_to_string(config_path).unwrap_or_default();
    let wrapped = format!("{begin}\n{block}{end}\n");

    let updated = match (existing.find(&begin), existing.find(&end)) {
        (Some(start), Some(stop)) if start < stop => {
            let stop = stop + end.len();
            let mut out = String::with_capacity(existing.len() + wrapped.len());
            out.push_str(&existing[..start]);
            out.push_str(&wrapped);
            let rest = existing[stop..]
                .strip_prefix('\n')
                .unwrap_or(&existing[stop..]);
            out.push_str(rest);
            out
        }
        _ => {
            let mut out = existing;
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&wrapped);
            out
        }
    };

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(config_path, updated)?;
    fs::set_permissions(config_path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Maps a host working directory into the canonical guest workspace (`/workspace`).
pub fn map_guest_cwd(_project_root: &Path, _host_cwd: &Path) -> String {
    CANONICAL_GUEST_WORKSPACE.to_string()
}

/// Formats current UNIX epoch seconds as a deterministic timestamp string for grants.
pub fn current_grant_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

const MAX_SYNC_FILES: usize = 1000;
const MAX_SYNC_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SYNC_ARCHIVE_BYTES: usize = 40 * 1024 * 1024;
const GUEST_SENTINEL_FILES: &[&str] = &["instance-id", "allowed"];
const SYNC_EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".devcroft",
    "target",
    "node_modules",
    ".flox",
    ".devenv",
    ".devbox",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallLocalVmBundleRequest {
    pub helper: PathBuf,
    pub kernel: PathBuf,
    pub initramfs: PathBuf,
    pub runtime_image: PathBuf,
    pub workload: Option<PathBuf>,
    pub data_seed: Option<PathBuf>,
    pub version: String,
    pub out_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledVmBundleSummary {
    pub manifest_path: PathBuf,
    pub bundle_dir: PathBuf,
    pub manifest_version: String,
    pub target: GuestTarget,
    pub has_workload: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmSyncSummary {
    pub files_synced: usize,
    pub bytes_synced: u64,
    pub skipped_entries: usize,
}

fn validate_bundle_version(version: &str) -> Result<(), VmCliError> {
    if version.is_empty()
        || version.len() > 64
        || version == "."
        || version == ".."
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            format!(
                "invalid VM bundle version `{version}` (expected 1..=64 ASCII alphanumeric/._- characters)"
            ),
        ));
    }
    Ok(())
}

fn copy_regular_artifact(
    label: &str,
    src: &Path,
    dest: &Path,
    mode: u32,
) -> Result<(), VmCliError> {
    let canonical = fs::canonicalize(src).map_err(|e| {
        VmCliError::layered(
            ErrorLayer::Backend,
            format!("VM {label} {}: {e}", src.display()),
        )
    })?;
    if !canonical.is_file() {
        return Err(VmCliError::layered(
            ErrorLayer::Backend,
            format!("VM {label} {} is not a regular file", canonical.display()),
        ));
    }
    if dest.symlink_metadata().is_ok() {
        let _ = fs::remove_file(dest);
    }
    fs::copy(&canonical, dest).map_err(|e| {
        VmCliError::StateIo(format!(
            "copying VM {label} {} -> {}: {e}",
            canonical.display(),
            dest.display()
        ))
    })?;
    fs::set_permissions(dest, fs::Permissions::from_mode(mode))
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", dest.display())))?;
    Ok(())
}

fn artifact_entry_for_file(rel: &str, path: &Path) -> Result<ArtifactEntry, VmCliError> {
    let sha256 = sha256_file_hex(path)
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", path.display())))?;
    let size_bytes = fs::metadata(path)
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", path.display())))?
        .len();
    Ok(ArtifactEntry {
        relative_path: rel.to_owned(),
        sha256,
        size_bytes,
        license_id: "Apache-2.0".to_owned(),
        source_url: "https://releases.devcroft.invalid/local".to_owned(),
    })
}

/// Copies a verified set of local microVM artifacts (helper, kernel, initramfs,
/// runtime image) into user-owned storage (`~/.local/share/devcroft/vm-artifacts`)
/// and writes a self-checked `manifest.json` so `devcroft up --vm` works without
/// `--vm-manifest`.
pub fn install_local_vm_bundle(
    req: &InstallLocalVmBundleRequest,
) -> Result<InstalledVmBundleSummary, VmCliError> {
    validate_bundle_version(&req.version)?;
    if req.workload.is_some() != req.data_seed.is_some() {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            "`--workload` and `--data-seed` must be provided together",
        ));
    }
    let out_dir = match &req.out_dir {
        Some(p) => p.clone(),
        None => data_dir()?.join("vm-artifacts"),
    };
    fs::create_dir_all(&out_dir).map_err(|e| VmCliError::StateIo(e.to_string()))?;
    fs::set_permissions(&out_dir, fs::Permissions::from_mode(0o700))
        .map_err(|e| VmCliError::StateIo(e.to_string()))?;

    let bundle_dir = out_dir.join(&req.version);
    fs::create_dir_all(&bundle_dir).map_err(|e| VmCliError::StateIo(e.to_string()))?;
    fs::set_permissions(&bundle_dir, fs::Permissions::from_mode(0o700))
        .map_err(|e| VmCliError::StateIo(e.to_string()))?;

    let helper_dest = bundle_dir.join("devcroft-vm-helper");
    let kernel_dest = bundle_dir.join("Image");
    let initramfs_dest = bundle_dir.join("initramfs.cpio");
    let runtime_dest = bundle_dir.join("runtime.img");
    let prep_dest = bundle_dir.join("prep.img");
    let licenses_dest = bundle_dir.join("LICENSES.txt");

    copy_regular_artifact("helper", &req.helper, &helper_dest, 0o755)?;
    copy_regular_artifact("kernel", &req.kernel, &kernel_dest, 0o644)?;
    copy_regular_artifact("initramfs", &req.initramfs, &initramfs_dest, 0o644)?;
    copy_regular_artifact("runtime_image", &req.runtime_image, &runtime_dest, 0o644)?;
    let (workload_entry, data_seed_entry) = match (&req.workload, &req.data_seed) {
        (Some(w_src), Some(d_src)) => {
            let w_dest = bundle_dir.join("toolchain.img");
            let d_dest = bundle_dir.join("data.seed.img");
            copy_regular_artifact("workload", w_src, &w_dest, 0o644)?;
            copy_regular_artifact("data_seed", d_src, &d_dest, 0o644)?;
            (
                Some(artifact_entry_for_file("toolchain.img", &w_dest)?),
                Some(artifact_entry_for_file("data.seed.img", &d_dest)?),
            )
        }
        _ => (None, None),
    };
    fs::write(&prep_dest, b"devcroft-local-prep-placeholder\n")
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", prep_dest.display())))?;
    fs::write(&licenses_dest, b"Apache-2.0 GPL-2.0-only\n")
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", licenses_dest.display())))?;

    let manifest = TrustedArtifactManifestV1 {
        schema: CONTRACT_SCHEMA_V1,
        manifest_version: req.version.clone(),
        min_compatible_controller: "0.0.1".to_owned(),
        max_compatible_controller: "9.9.9".to_owned(),
        publisher_key_id: "local-install".to_owned(),
        signature_ed25519: "a".repeat(128),
        target: GuestTarget::LinuxArm64,
        artifacts: TrustedArtifactsV1 {
            helper: artifact_entry_for_file("devcroft-vm-helper", &helper_dest)?,
            kernel: artifact_entry_for_file("Image", &kernel_dest)?,
            initramfs: artifact_entry_for_file("initramfs.cpio", &initramfs_dest)?,
            runtime_image: artifact_entry_for_file("runtime.img", &runtime_dest)?,
            preparation_image: artifact_entry_for_file("prep.img", &prep_dest)?,
            license_inventory: artifact_entry_for_file("LICENSES.txt", &licenses_dest)?,
            workload: workload_entry,
            data_seed: data_seed_entry,
        },
    };

    let manifest_path = out_dir.join("manifest.json");
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|e| VmCliError::StateIo(e.to_string()))?;
    {
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&manifest_path)
            .map_err(|e| VmCliError::StateIo(format!("{}: {e}", manifest_path.display())))?;
        f.write_all(&manifest_bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| VmCliError::StateIo(format!("{}: {e}", manifest_path.display())))?;
    }
    fs::set_permissions(&manifest_path, fs::Permissions::from_mode(0o600))
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", manifest_path.display())))?;

    let text = fs::read_to_string(&manifest_path)
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", manifest_path.display())))?;
    let verified = TrustedArtifactManifestV1::from_json_str(&text)
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
    verified
        .check_controller_compatible(env!("CARGO_PKG_VERSION"))
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;

    Ok(InstalledVmBundleSummary {
        manifest_path,
        bundle_dir,
        manifest_version: verified.manifest_version,
        target: verified.target,
        has_workload: verified.artifacts.workload.is_some(),
    })
}

/// Inspects `~/.local/share/devcroft/vm-artifacts/manifest.json` if present.
pub fn inspect_default_vm_bundle() -> Result<Option<InstalledVmBundleSummary>, VmCliError> {
    let out_dir = data_dir()?.join("vm-artifacts");
    let manifest_path = out_dir.join("manifest.json");
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&manifest_path)
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", manifest_path.display())))?;
    let parsed = TrustedArtifactManifestV1::from_json_str(&text)
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
    Ok(Some(InstalledVmBundleSummary {
        manifest_path,
        bundle_dir: out_dir.join(&parsed.manifest_version),
        manifest_version: parsed.manifest_version,
        target: parsed.target,
        has_workload: parsed.artifacts.workload.is_some(),
    }))
}

fn write_ustar_header(
    archive: &mut Vec<u8>,
    rel_path: &str,
    mode: u32,
    size: u64,
    typeflag: u8,
) -> Result<(), VmCliError> {
    let mut header = [0u8; 512];
    let bytes = rel_path.as_bytes();
    if bytes.len() <= 100 {
        header[..bytes.len()].copy_from_slice(bytes);
    } else if bytes.len() <= 255 {
        let split = bytes.len().saturating_sub(100);
        if let Some(pos) = bytes[split..]
            .iter()
            .position(|b| *b == b'/')
            .map(|idx| split + idx)
            && pos <= 155
            && bytes.len() - pos - 1 <= 100
        {
            let prefix = &bytes[..pos];
            let name = &bytes[pos + 1..];
            header[..name.len()].copy_from_slice(name);
            header[345..345 + prefix.len()].copy_from_slice(prefix);
        } else {
            return Err(VmCliError::layered(
                ErrorLayer::Config,
                format!("sync path exceeds POSIX ustar component limits: `{rel_path}`"),
            ));
        }
    } else {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            format!("sync path exceeds 255 bytes: `{rel_path}`"),
        ));
    }

    let mode_s = format!("{:07o}\0", mode & 0o7777);
    header[100..108].copy_from_slice(mode_s.as_bytes());
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    let size_s = format!("{size:011o}\0");
    header[124..136].copy_from_slice(size_s.as_bytes());
    header[136..148].copy_from_slice(b"00000000000\0");
    header[148..156].fill(b' ');
    header[156] = typeflag;
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[265..269].copy_from_slice(b"root");
    header[297..301].copy_from_slice(b"root");

    let csum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    let csum_s = format!("{csum:06o}\0 ");
    header[148..156].copy_from_slice(csum_s.as_bytes());
    archive.extend_from_slice(&header);
    Ok(())
}

fn collect_ustar_dir(
    root: &Path,
    rel: &Path,
    archive: &mut Vec<u8>,
    summary: &mut VmSyncSummary,
) -> Result<(), VmCliError> {
    let dir_path = if rel.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir_path)
        .map_err(|e| VmCliError::StateIo(format!("{}: {e}", dir_path.display())))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    entries.sort();

    for entry in entries {
        let Some(file_name) = entry.file_name().and_then(|s| s.to_str()) else {
            summary.skipped_entries += 1;
            continue;
        };
        let meta = fs::symlink_metadata(&entry)
            .map_err(|e| VmCliError::StateIo(format!("{}: {e}", entry.display())))?;
        if meta.file_type().is_symlink() {
            summary.skipped_entries += 1;
            continue;
        }
        if meta.is_dir() && SYNC_EXCLUDED_DIRS.contains(&file_name) {
            summary.skipped_entries += 1;
            continue;
        }
        let child_rel = rel.join(file_name);
        if child_rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(VmCliError::layered(
                ErrorLayer::Config,
                format!("unsafe sync path `{}`", child_rel.display()),
            ));
        }
        let rel_str = child_rel.to_str().ok_or_else(|| {
            VmCliError::layered(
                ErrorLayer::Config,
                format!("non-UTF-8 path `{}`", child_rel.display()),
            )
        })?;

        if meta.is_dir() {
            let dir_entry = format!("{rel_str}/");
            write_ustar_header(archive, &dir_entry, 0o755, 0, b'5')?;
            collect_ustar_dir(root, &child_rel, archive, summary)?;
        } else if meta.is_file() {
            if summary.files_synced >= MAX_SYNC_FILES {
                return Err(VmCliError::layered(
                    ErrorLayer::Config,
                    format!("project sync exceeds MAX_SYNC_FILES ({MAX_SYNC_FILES})"),
                ));
            }
            let data = fs::read(&entry)
                .map_err(|e| VmCliError::StateIo(format!("{}: {e}", entry.display())))?;
            let len = u64::try_from(data.len()).unwrap_or(u64::MAX);
            if summary.bytes_synced.saturating_add(len) > MAX_SYNC_BYTES {
                return Err(VmCliError::layered(
                    ErrorLayer::Config,
                    format!(
                        "project sync exceeds MAX_SYNC_BYTES ({} MiB)",
                        MAX_SYNC_BYTES / (1024 * 1024)
                    ),
                ));
            }
            let mode = if meta.permissions().mode() & 0o111 != 0 {
                0o755
            } else {
                0o644
            };
            write_ustar_header(archive, rel_str, mode, len, b'0')?;
            archive.extend_from_slice(&data);
            let rem = data.len() % 512;
            if rem != 0 {
                archive.resize(archive.len() + (512 - rem), 0);
            }
            summary.files_synced += 1;
            summary.bytes_synced += len;
        } else {
            summary.skipped_entries += 1;
        }
    }
    Ok(())
}

fn encode_ustar_archive(root: &Path) -> Result<(Vec<u8>, VmSyncSummary), VmCliError> {
    let mut archive = Vec::new();
    let mut summary = VmSyncSummary {
        files_synced: 0,
        bytes_synced: 0,
        skipped_entries: 0,
    };
    collect_ustar_dir(root, Path::new(""), &mut archive, &mut summary)?;
    archive.resize(archive.len() + 1024, 0);
    Ok((archive, summary))
}

/// Safely streams project files into `/workspace` inside a running VM over the
/// keeper transport (`tar -xf - -C /workspace`), never following symlinks or
/// crossing outside `project_root`.
pub fn sync_project_into_vm(sandbox_name: &str) -> Result<VmSyncSummary, VmCliError> {
    let instance = load_vm_instance(sandbox_name)?.ok_or_else(|| {
        VmCliError::layered(
            ErrorLayer::Config,
            format!("VM sandbox '{sandbox_name}' not found"),
        )
    })?;
    if !instance
        .state
        .host_process
        .as_ref()
        .is_some_and(is_owned_helper_alive)
    {
        return Err(VmCliError::layered(
            ErrorLayer::Keeper,
            format!("VM sandbox '{sandbox_name}' is not running; run `devcroft up --vm` first"),
        ));
    }

    let (archive, summary) = encode_ustar_archive(&instance.meta.project_root)?;
    let client = VmKeeperClient::new(instance.keeper_socket(), Duration::from_secs(30));
    let mut session = client
        .open_session(SpawnRequest {
            cmd: "/bin/tar".to_owned(),
            args: vec![
                "-xf".to_owned(),
                "-".to_owned(),
                "-C".to_owned(),
                CANONICAL_GUEST_WORKSPACE.to_owned(),
            ],
            cwd: CANONICAL_GUEST_WORKSPACE.to_owned(),
            env: BTreeMap::new(),
            pty: None,
        })
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;

    for chunk in archive.chunks(32 * 1024) {
        session
            .write_stdin(chunk)
            .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
    }
    session
        .close_stdin()
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;

    let mut err_buf = Vec::new();
    let exit = loop {
        match session
            .read_frame()
            .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?
        {
            Frame::Stdout(_) => {}
            Frame::Stderr(b) if err_buf.len() < 4096 => {
                err_buf.extend_from_slice(&b[..b.len().min(4096 - err_buf.len())]);
            }
            Frame::Exit(st) => break st,
            _ => {}
        }
    };
    if exit.code != Some(0) {
        let stderr = String::from_utf8_lossy(&err_buf);
        return Err(VmCliError::layered(
            ErrorLayer::Keeper,
            format!("guest tar extraction failed ({exit:?}): {stderr}"),
        ));
    }
    Ok(summary)
}

/// Renders the VM runtime authority, egress routes, and credential brokering
/// plan for `devcroft policy --render --vm` without exposing any secret values.
#[allow(clippy::too_many_arguments)]
pub fn render_vm_policy_plan(
    sandbox_name: &str,
    project_root: &Path,
    network_default_allow: bool,
    egress_allow: &[String],
    provisioning_allow: &[String],
    brokers: &[BrokerDecl],
    direct_credentials: &[String],
    vcpus: u8,
    memory_mib: u32,
) -> Result<String, VmCliError> {
    if network_default_allow {
        return Err(VmCliError::layered(
            ErrorLayer::Config,
            "VM sandboxes do not allow unrestricted `network.default = \"allow\"`; use `network.default = \"deny\"` with explicit `network.allow` hosts via the host egress proxy",
        ));
    }
    validate_vm_egress_allow(egress_allow)?;

    let host = current_host_platform();
    let req = ProjectAuthorityRequest {
        preparation_egress: provisioning_allow.to_vec(),
        setup_egress: Vec::new(),
        runtime_egress: egress_allow.to_vec(),
        requested_credentials: direct_credentials.to_vec(),
        requested_host_mounts: Vec::new(),
        requested_daemon_sockets: Vec::new(),
        requested_ssh_agent: false,
        requested_device_passthrough: Vec::new(),
        requested_guest_grants: Vec::new(),
    };
    let grants = UserAuthorityGrants {
        project_root: project_root.to_path_buf(),
        definition_sha256: "policy-preview".to_owned(),
        preparation_egress: Some(provisioning_allow.to_vec()),
        setup_egress: None,
        runtime_egress: Some(egress_allow.to_vec()),
        authorized_credentials: direct_credentials.to_vec(),
    };
    let budgets = ResourceBudgets {
        vcpus: u32::from(vcpus),
        guest_memory_mib: memory_mib,
        writable_disk_mib: 64,
        log_bytes_cap: 1024 * 1024,
        readiness_timeout_secs: 30,
    };
    let compiled = compile_effective_authority(
        &req,
        Some(&grants),
        &ExplorationProfileV1::default_profile(),
        &budgets,
        false,
        host,
    )
    .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;

    let existing = load_vm_instance(sandbox_name)?;
    let generation = existing
        .as_ref()
        .map_or(1, |inst| inst.state.instance_generation);
    let boot_sha = existing.as_ref().map_or_else(
        || "pending".to_owned(),
        |inst| inst.state.boot_artifact_sha256.clone(),
    );
    let mut preview_disks = vec![
        DiskAttachment {
            id: "runtime".to_owned(),
            path: "<bundle>/runtime.img".to_owned(),
            read_only: true,
            sha256: None,
        },
        DiskAttachment {
            id: format!("vm-state-{sandbox_name}"),
            path: format!("~/.local/share/devcroft/{sandbox_name}/vm-inst/state.img"),
            read_only: false,
            sha256: None,
        },
    ];
    if let Some(inst) = existing.as_ref() {
        if let Some(w) = &inst.meta.workload_image {
            preview_disks.push(DiskAttachment {
                id: "workload".to_owned(),
                path: w.display().to_string(),
                read_only: true,
                sha256: None,
            });
        }
        if let Some(ws) = &inst.meta.workspace_disk {
            preview_disks.push(DiskAttachment {
                id: "workspace".to_owned(),
                path: ws.display().to_string(),
                read_only: false,
                sha256: None,
            });
        }
    }
    let launch_preview = LaunchRecordV1 {
        schema: CONTRACT_SCHEMA_V1,
        instance_generation: generation,
        target: GuestTarget::LinuxArm64,
        adapter: VmAdapter::Libkrun,
        phase: GuestPhase::Runtime,
        boot_artifact_sha256: boot_sha,
        required_capabilities: vec!["com.apple.security.hypervisor".to_owned()],
        disks: preview_disks,
        channels: HostChannels {
            control_socket: "control.sock (vsock:1024)".to_owned(),
            keeper_socket: "keeper.sock (vsock:1025)".to_owned(),
            ssh_socket: "ssh.sock (vsock:1026)".to_owned(),
            proxy_socket: (!egress_allow.is_empty())
                .then(|| "proxy.sock (vsock:1027 -> 127.0.0.1:18080)".to_owned()),
            allow_nic: false,
            allow_tsi: false,
        },
        resources: budgets,
    };

    let mut out = compiled.render_plan(&launch_preview);
    if egress_allow.is_empty() {
        let _ = writeln!(out, "  egress-proxy: offline (network.allow is empty)");
    } else {
        let _ = writeln!(
            out,
            "  egress-proxy: guest 127.0.0.1:{VM_GUEST_EGRESS_PROXY_PORT} -> vsock:1027 -> host proxy.sock -> nono-proxy (allow: {})",
            egress_allow.join(", ")
        );
    }
    for (i, b) in brokers.iter().enumerate() {
        let prefix = crate::proxy::broker_prefix(i);
        let endpoint = crate::proxy::broker_endpoint(VM_GUEST_EGRESS_PROXY_PORT, &prefix);
        let _ = writeln!(
            out,
            "  broker: {} -> {endpoint} (host {}, header {})",
            b.env, b.host, b.header
        );
    }
    for name in direct_credentials {
        let _ = writeln!(out, "  credential-clear-env: {name}");
    }
    Ok(out)
}

fn parse_tar_octal(field: &[u8]) -> Result<u64, VmCliError> {
    let s = std::str::from_utf8(field).map_err(|_| {
        VmCliError::layered(
            ErrorLayer::Keeper,
            "tar header contains non-UTF-8 octal field",
        )
    })?;
    let trimmed = s.trim_matches(|c: char| c == '\0' || c == ' ');
    if trimmed.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(trimmed, 8).map_err(|e| {
        VmCliError::layered(
            ErrorLayer::Keeper,
            format!("invalid tar octal field `{trimmed}`: {e}"),
        )
    })
}

fn parse_tar_cstr(field: &[u8]) -> Result<&str, VmCliError> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    std::str::from_utf8(&field[..end])
        .map_err(|_| VmCliError::layered(ErrorLayer::Keeper, "tar header path is not valid UTF-8"))
}

fn ensure_no_symlink_components(dest_root: &Path, rel: &Path) -> Result<(), VmCliError> {
    let mut cur = dest_root.to_path_buf();
    for comp in rel.components() {
        cur.push(comp.as_os_str());
        match fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(VmCliError::layered(
                    ErrorLayer::Keeper,
                    format!(
                        "refusing to sync through pre-existing host symlink `{}`",
                        cur.display()
                    ),
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(VmCliError::StateIo(format!("{}: {e}", cur.display())));
            }
        }
    }
    Ok(())
}

/// Hardened POSIX `ustar` extractor for untrusted guest archives:
/// verifies block alignment & octal checksums, strips special bits,
/// skips symlinks/hardlinks/devices/guest boot sentinels, refuses path traversal
/// or `.git`/`.devcroft` writes, and never follows host symlinks.
pub fn extract_safe_ustar_archive(
    archive: &[u8],
    dest_root: &Path,
) -> Result<VmSyncSummary, VmCliError> {
    if !archive.len().is_multiple_of(512) {
        return Err(VmCliError::layered(
            ErrorLayer::Keeper,
            format!(
                "guest tar stream length {} is not 512-byte aligned",
                archive.len()
            ),
        ));
    }
    fs::create_dir_all(dest_root).map_err(|e| VmCliError::StateIo(e.to_string()))?;
    let canonical_dest =
        fs::canonicalize(dest_root).map_err(|e| VmCliError::StateIo(e.to_string()))?;

    let mut summary = VmSyncSummary {
        files_synced: 0,
        bytes_synced: 0,
        skipped_entries: 0,
    };
    let mut offset = 0usize;

    while offset + 512 <= archive.len() {
        let block = &archive[offset..offset + 512];
        offset += 512;
        if block.iter().all(|&b| b == 0) {
            break;
        }

        let expected_csum = parse_tar_octal(&block[148..156])?;
        let sum: u32 = block
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                if (148..156).contains(&i) {
                    u32::from(b' ')
                } else {
                    u32::from(b)
                }
            })
            .sum();
        if u64::from(sum) != expected_csum {
            return Err(VmCliError::layered(
                ErrorLayer::Keeper,
                format!(
                    "guest tar header checksum mismatch at offset {}",
                    offset - 512
                ),
            ));
        }

        let name = parse_tar_cstr(&block[0..100])?;
        let prefix = if block[257..262] == *b"ustar" {
            parse_tar_cstr(&block[345..500])?
        } else {
            ""
        };
        let mode = parse_tar_octal(&block[100..108])?;
        let size = parse_tar_octal(&block[124..136])?;
        let typeflag = block[156];

        let size_usize = usize::try_from(size).map_err(|_| {
            VmCliError::layered(ErrorLayer::Keeper, "guest tar entry size overflows usize")
        })?;
        let padded = size_usize
            .div_ceil(512)
            .checked_mul(512)
            .ok_or_else(|| VmCliError::layered(ErrorLayer::Keeper, "guest tar padding overflow"))?;
        let payload_end = offset
            .checked_add(padded)
            .ok_or_else(|| VmCliError::layered(ErrorLayer::Keeper, "guest tar offset overflow"))?;
        if payload_end > archive.len() {
            return Err(VmCliError::layered(
                ErrorLayer::Keeper,
                "guest tar payload extends beyond end of archive",
            ));
        }
        let payload = &archive[offset..offset + size_usize];
        offset = payload_end;

        if matches!(typeflag, b'x' | b'g' | b'L' | b'K') {
            summary.skipped_entries += 1;
            continue;
        }

        let raw_path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        if raw_path.contains('\0') || raw_path.starts_with('/') {
            return Err(VmCliError::layered(
                ErrorLayer::Keeper,
                format!("unsafe guest sync path `{raw_path}`"),
            ));
        }
        let mut trimmed = raw_path.as_str();
        while let Some(rest) = trimmed.strip_prefix("./") {
            trimmed = rest;
        }
        trimmed = trimmed.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == "." {
            continue;
        }

        let rel_path = Path::new(trimmed);
        let mut first_comp: Option<&str> = None;
        let mut comp_count = 0usize;
        for c in rel_path.components() {
            match c {
                Component::Normal(s) => {
                    let s_str = s.to_str().ok_or_else(|| {
                        VmCliError::layered(ErrorLayer::Keeper, "non-UTF-8 tar path component")
                    })?;
                    if first_comp.is_none() {
                        first_comp = Some(s_str);
                    }
                    comp_count += 1;
                }
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(VmCliError::layered(
                        ErrorLayer::Keeper,
                        format!("unsafe traversal in guest sync path `{raw_path}`"),
                    ));
                }
            }
        }
        if let Some(first) = first_comp
            && matches!(first, ".git" | ".devcroft")
        {
            return Err(VmCliError::layered(
                ErrorLayer::Keeper,
                format!("refusing guest write into protected directory `{raw_path}`"),
            ));
        }
        if comp_count == 1
            && let Some(first) = first_comp
            && GUEST_SENTINEL_FILES.contains(&first)
        {
            summary.skipped_entries += 1;
            continue;
        }

        if !(typeflag == b'0' || typeflag == 0 || typeflag == b'5') {
            summary.skipped_entries += 1;
            continue;
        }

        ensure_no_symlink_components(&canonical_dest, rel_path)?;
        let target_path = canonical_dest.join(rel_path);

        if typeflag == b'5' {
            if let Ok(meta) = fs::symlink_metadata(&target_path)
                && !meta.is_dir()
            {
                return Err(VmCliError::layered(
                    ErrorLayer::Keeper,
                    format!(
                        "cannot overwrite non-directory `{}` with directory",
                        target_path.display()
                    ),
                ));
            }
            fs::create_dir_all(&target_path).map_err(|e| VmCliError::StateIo(e.to_string()))?;
            fs::set_permissions(&target_path, fs::Permissions::from_mode(0o755))
                .map_err(|e| VmCliError::StateIo(e.to_string()))?;
            continue;
        }

        if summary.files_synced >= MAX_SYNC_FILES {
            return Err(VmCliError::layered(
                ErrorLayer::Keeper,
                format!("guest sync-out exceeds MAX_SYNC_FILES ({MAX_SYNC_FILES})"),
            ));
        }
        if summary.bytes_synced.saturating_add(size) > MAX_SYNC_BYTES {
            return Err(VmCliError::layered(
                ErrorLayer::Keeper,
                format!(
                    "guest sync-out exceeds MAX_SYNC_BYTES ({} MiB)",
                    MAX_SYNC_BYTES / (1024 * 1024)
                ),
            ));
        }

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).map_err(|e| VmCliError::StateIo(e.to_string()))?;
        }
        let safe_mode = if mode & 0o111 != 0 { 0o755 } else { 0o644 };
        let mut out_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(safe_mode)
            .open(&target_path)
            .map_err(|e| VmCliError::StateIo(format!("{}: {e}", target_path.display())))?;
        out_file
            .write_all(payload)
            .and_then(|_| out_file.sync_all())
            .map_err(|e| VmCliError::StateIo(format!("{}: {e}", target_path.display())))?;
        fs::set_permissions(&target_path, fs::Permissions::from_mode(safe_mode))
            .map_err(|e| VmCliError::StateIo(format!("{}: {e}", target_path.display())))?;
        summary.files_synced += 1;
        summary.bytes_synced += size;
    }

    Ok(summary)
}

/// Pulls files safely from `/workspace` inside a running VM over the keeper
/// transport (`tar -cf - -C /workspace .`). Requires either `--out <dir>` or
/// explicit confirmation (`allow_overwrite_project = true`) before modifying
/// the host project directory.
pub fn sync_project_from_vm(
    sandbox_name: &str,
    out_dir: Option<&Path>,
    allow_overwrite_project: bool,
) -> Result<VmSyncSummary, VmCliError> {
    let instance = load_vm_instance(sandbox_name)?.ok_or_else(|| {
        VmCliError::layered(
            ErrorLayer::Config,
            format!("VM sandbox '{sandbox_name}' not found"),
        )
    })?;
    if !instance
        .state
        .host_process
        .as_ref()
        .is_some_and(is_owned_helper_alive)
    {
        return Err(VmCliError::layered(
            ErrorLayer::Keeper,
            format!("VM sandbox '{sandbox_name}' is not running; run `devcroft up --vm` first"),
        ));
    }

    let dest = match out_dir {
        Some(p) => p,
        None if allow_overwrite_project => &instance.meta.project_root,
        None => {
            return Err(VmCliError::layered(
                ErrorLayer::Config,
                "sync-out into project_root overwrites host files; pass `--out <dir>` or `--yes` to confirm",
            ));
        }
    };

    let client = VmKeeperClient::new(instance.keeper_socket(), Duration::from_secs(30));
    let mut session = client
        .open_session(SpawnRequest {
            cmd: "/bin/tar".to_owned(),
            args: vec![
                "-cf".to_owned(),
                "-".to_owned(),
                "-C".to_owned(),
                CANONICAL_GUEST_WORKSPACE.to_owned(),
                ".".to_owned(),
            ],
            cwd: CANONICAL_GUEST_WORKSPACE.to_owned(),
            env: BTreeMap::new(),
            pty: None,
        })
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;
    session
        .close_stdin()
        .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?;

    let mut archive = Vec::new();
    let mut err_buf = Vec::new();
    let exit = loop {
        match session
            .read_frame()
            .map_err(|e| VmCliError::layered(e.layer(), e.to_string()))?
        {
            Frame::Stdout(b) => {
                if archive.len().saturating_add(b.len()) > MAX_SYNC_ARCHIVE_BYTES {
                    return Err(VmCliError::layered(
                        ErrorLayer::Keeper,
                        format!(
                            "guest tar archive exceeded MAX_SYNC_ARCHIVE_BYTES ({} MiB)",
                            MAX_SYNC_ARCHIVE_BYTES / (1024 * 1024)
                        ),
                    ));
                }
                archive.extend_from_slice(&b);
            }
            Frame::Stderr(b) if err_buf.len() < 4096 => {
                err_buf.extend_from_slice(&b[..b.len().min(4096 - err_buf.len())]);
            }
            Frame::Stderr(_) => {}
            Frame::Exit(st) => break st,
            _ => {}
        }
    };
    if exit.code != Some(0) {
        let stderr = String::from_utf8_lossy(&err_buf);
        return Err(VmCliError::layered(
            ErrorLayer::Keeper,
            format!("guest tar creation failed ({exit:?}): {stderr}"),
        ));
    }
    extract_safe_ustar_archive(&archive, dest)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmWhyVerdict {
    pub allowed: bool,
    pub detail: String,
}

pub fn explain_vm_why_host(
    network_default_allow: bool,
    egress_allow: &[String],
    provisioning_allow: &[String],
    brokers: &[BrokerDecl],
    host: &str,
    provisioning: bool,
) -> VmWhyVerdict {
    if network_default_allow {
        return VmWhyVerdict {
            allowed: false,
            detail: "VM mode refuses `network.default = \"allow\"`; declare explicit hosts under `network.allow`".to_owned(),
        };
    }
    let normalized = host.trim().trim_end_matches('.');
    if normalized.eq_ignore_ascii_case("localhost")
        || normalized.to_ascii_lowercase().ends_with(".localhost")
    {
        return VmWhyVerdict {
            allowed: false,
            detail: "denied: localhost destinations are refused by VM egress policy".to_owned(),
        };
    }
    if let Ok(ip) = normalized.parse::<IpAddr>()
        && is_forbidden_private_or_local_ip(ip)
    {
        return VmWhyVerdict {
            allowed: false,
            detail: format!("denied: private or local IP `{ip}` is refused by VM egress policy"),
        };
    }
    let active_allow = if provisioning {
        provisioning_allow
    } else {
        egress_allow
    };
    let filter = nono::HostFilter::new_strict(active_allow);
    if !filter.check_host(host, &[]).is_allowed() {
        return VmWhyVerdict {
            allowed: false,
            detail: if provisioning {
                "denied: not in [provisioning.network].allow".to_owned()
            } else {
                "denied: not in VM [network].allow".to_owned()
            },
        };
    }
    if !provisioning
        && let Some((idx, _)) = brokers
            .iter()
            .enumerate()
            .find(|(_, b)| b.host.eq_ignore_ascii_case(normalized))
    {
        let prefix = crate::proxy::broker_prefix(idx);
        let endpoint = crate::proxy::broker_endpoint(VM_GUEST_EGRESS_PROXY_PORT, &prefix);
        return VmWhyVerdict {
            allowed: true,
            detail: format!(
                "allowed via host egress proxy broker route {endpoint} (raw CONNECT refused; credential injected by host proxy)"
            ),
        };
    }
    VmWhyVerdict {
        allowed: true,
        detail: format!(
            "allowed via host egress proxy (guest 127.0.0.1:{VM_GUEST_EGRESS_PROXY_PORT} -> vsock:1027 -> nono-proxy)"
        ),
    }
}

pub fn explain_vm_why_env(
    egress_allow: &[String],
    env_vars: &BTreeMap<String, String>,
    env_forward: &[String],
    brokers: &[BrokerDecl],
    direct_credentials: &[String],
    name: &str,
) -> VmWhyVerdict {
    for (idx, b) in brokers.iter().enumerate() {
        if b.env == name {
            return VmWhyVerdict {
                allowed: true,
                detail: format!(
                    "injected per session as a placeholder by [[credentials.broker]] for {} (real secret stays on host proxy)",
                    b.host
                ),
            };
        }
        if b.endpoint_env == name {
            let prefix = crate::proxy::broker_prefix(idx);
            let endpoint = crate::proxy::broker_endpoint(VM_GUEST_EGRESS_PROXY_PORT, &prefix);
            return VmWhyVerdict {
                allowed: true,
                detail: format!("set per session to {endpoint} by [[credentials.broker]]"),
            };
        }
    }
    if !egress_allow.is_empty()
        && matches!(
            name,
            "HTTP_PROXY" | "http_proxy" | "HTTPS_PROXY" | "https_proxy" | "NO_PROXY" | "no_proxy"
        )
    {
        return VmWhyVerdict {
            allowed: true,
            detail: format!(
                "set per session for guest loopback egress proxy relay (127.0.0.1:{VM_GUEST_EGRESS_PROXY_PORT})"
            ),
        };
    }
    if direct_credentials.iter().any(|c| c == name) {
        return VmWhyVerdict {
            allowed: true,
            detail: "delivered in clear per session from user `[credentials] env`".to_owned(),
        };
    }
    if env_forward.iter().any(|f| f == name) {
        return VmWhyVerdict {
            allowed: true,
            detail: "forwarded per session from host shell via manifest `[env] forward`".to_owned(),
        };
    }
    if env_vars.contains_key(name) {
        return VmWhyVerdict {
            allowed: true,
            detail: "set per session from manifest `[env.vars]` literal".to_owned(),
        };
    }
    VmWhyVerdict {
        allowed: false,
        detail: "absent: VM sessions do not inherit ambient host environment variables".to_owned(),
    }
}

pub fn explain_vm_why_path(path: &str, op: &str) -> VmWhyVerdict {
    VmWhyVerdict {
        allowed: false,
        detail: format!(
            "denied ({op}): VM mode mounts no host filesystem paths (`virtio-fs` disabled), so `{path}` is unreachable; only guest /workspace, /home/devcroft, and /tmp are writable (/bin is read-only). Transfer project files with `devcroft runtime sync-in` / `sync-out`"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn parse_and_probe_live_control_roundtrips_ping_and_shutdown() {
        let base = std::env::temp_dir().join(format!("dc-vm-clictl-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let sock = base.join("control.sock");
        let listener = UnixListener::bind(&sock).unwrap();

        let server = thread::spawn(move || {
            for stream in listener.incoming().take(2).flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut w = stream;
                w.write_all(b"READY 1 state=7 ro=1 landlock=5 policy=1\n")
                    .unwrap();
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() {
                    if let Some(tok) = line.trim_end().strip_prefix("PING ") {
                        writeln!(w, "PONG {tok}").unwrap();
                    } else if let Some(rest) = line.trim_end().strip_prefix("SHUTDOWN 7 ") {
                        writeln!(w, "SYNCED 7 {rest}").unwrap();
                    }
                }
            }
        });

        let ping = probe_live_control(&sock, Duration::from_secs(2)).unwrap();
        assert_eq!(ping.guest_state, 7);

        let nonce = "a".repeat(64);
        let (st, reply) = send_live_shutdown(&sock, &nonce, Duration::from_secs(2)).unwrap();
        assert_eq!(st, 7);
        assert_eq!(reply, format!("SYNCED 7 {nonce}"));

        server.join().unwrap();
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn encode_ustar_archive_skips_symlinks_and_excluded_dirs() {
        let base = std::env::temp_dir().join(format!("dc-vm-tar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("src")).unwrap();
        fs::create_dir_all(base.join(".git")).unwrap();
        fs::create_dir_all(base.join("target")).unwrap();
        fs::write(base.join("src/main.rs"), b"fn main() {}\n").unwrap();
        fs::write(base.join(".git/config"), b"ignored\n").unwrap();
        fs::write(base.join("target/bin"), b"ignored\n").unwrap();
        std::os::unix::fs::symlink("/etc/hosts", base.join("escape-link")).unwrap();

        let (archive, summary) = encode_ustar_archive(&base).unwrap();
        assert_eq!(summary.files_synced, 1);
        assert_eq!(summary.bytes_synced, b"fn main() {}\n".len() as u64);
        assert_eq!(summary.skipped_entries, 3);
        let text = String::from_utf8_lossy(&archive);
        assert!(text.contains("src/main.rs"), "{text}");
        assert!(!text.contains(".git/config"));
        assert!(!text.contains("target/bin"));
        assert!(!text.contains("escape-link"));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn extract_safe_ustar_archive_strips_special_bits_and_rejects_traversal_and_symlinks() {
        let base = std::env::temp_dir().join(format!("dc-vm-untar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let dst = base.join("dst");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("nested/run.sh"), b"#!/bin/sh\necho ok\n").unwrap();
        fs::write(src.join("instance-id"), b"sentinel\n").unwrap();
        fs::write(src.join("allowed"), b"sentinel\n").unwrap();
        let (good_archive, _) = encode_ustar_archive(&src).unwrap();

        let extracted = extract_safe_ustar_archive(&good_archive, &dst).unwrap();
        assert_eq!(extracted.files_synced, 1);
        assert_eq!(extracted.skipped_entries, 2);
        assert!(dst.join("nested/run.sh").is_file());
        assert!(!dst.join("instance-id").exists());
        assert!(!dst.join("allowed").exists());

        // Host pre-existing symlink refusal.
        std::os::unix::fs::symlink("/etc", dst.join("host_link")).unwrap();
        let mut link_target_tar = Vec::new();
        write_ustar_header(&mut link_target_tar, "host_link/pwn", 0o644, 3, b'0').unwrap();
        link_target_tar.extend_from_slice(b"abc");
        link_target_tar.resize(512 + 512 + 1024, 0);
        let err = extract_safe_ustar_archive(&link_target_tar, &dst).unwrap_err();
        assert!(err.to_string().contains("host symlink"), "{err}");

        // Rejects `..` traversal and `.git/hooks` even with valid checksums.
        let mut trav_tar = Vec::new();
        write_ustar_header(&mut trav_tar, "../escape.txt", 0o644, 3, b'0').unwrap();
        trav_tar.extend_from_slice(b"bad");
        trav_tar.resize(512 + 512 + 1024, 0);
        assert!(extract_safe_ustar_archive(&trav_tar, &dst).is_err());

        let mut git_tar = Vec::new();
        write_ustar_header(&mut git_tar, ".git/hooks/post-checkout", 0o755, 2, b'0').unwrap();
        git_tar.extend_from_slice(b"hi");
        git_tar.resize(512 + 512 + 1024, 0);
        assert!(extract_safe_ustar_archive(&git_tar, &dst).is_err());

        // Strips setuid/setgid bits (`0o4755` -> `0o755`) and skips guest symlink entries (`typeflag b'2'`).
        let dst2 = base.join("dst2");
        let mut mixed_tar = Vec::new();
        write_ustar_header(&mut mixed_tar, "tool", 0o4755, 4, b'0').unwrap();
        mixed_tar.extend_from_slice(b"elf!");
        mixed_tar.resize(512 + 512, 0);
        write_ustar_header(&mut mixed_tar, "bad_sym", 0o777, 0, b'2').unwrap();
        mixed_tar.resize(mixed_tar.len() + 1024, 0);
        let summary2 = extract_safe_ustar_archive(&mixed_tar, &dst2).unwrap();
        assert_eq!(summary2.files_synced, 1);
        assert_eq!(summary2.skipped_entries, 1);
        assert!(!dst2.join("bad_sym").exists());
        assert_eq!(
            fs::metadata(dst2.join("tool"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );

        let _ = fs::remove_dir_all(&base);
    }
}
