//! Phase-scoped host proxy policy, private/local IP refusal beyond DNS,
//! setup stream revocation + reboot gate, and init-owned setup windows
//! (`add-vm-project-runtime` tasks 3.2, 3.3, and 3.6).

use std::collections::BTreeSet;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::contract::{ErrorLayer, GuestPhase, HandshakeV1, LaunchRecordV1};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedDestination {
    pub host: String,
    pub port: u16,
    pub phase: GuestPhase,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupRevocationReceipt {
    pub closed_setup_streams: usize,
    pub setup_generation: u64,
    pub requires_reboot_before_runtime: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminChannelOrigin {
    ControlInit,
    KeeperSession,
    SshSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupWindowStatus {
    pub active: bool,
    pub window_id: Option<String>,
    pub expires_at_epoch_secs: Option<u64>,
    pub credential_exposure_disclosed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyChannelError {
    ForgedInstanceIdentity {
        expected: String,
        claimed: String,
    },
    HostNotAllowed {
        host: String,
        phase: GuestPhase,
    },
    ForbiddenDestinationIp {
        host: String,
        ip: IpAddr,
    },
    MissingResolvedAddress(String),
    RebootRequiredBeforeRuntime {
        setup_generation: u64,
        found_generation: u64,
    },
    CredentialExposureNotDisclosed,
    KeeperCannotElevate,
    SetupWindowClosed,
    HandshakeInvalid(String),
}

impl ProxyChannelError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            ProxyChannelError::HandshakeInvalid(_) | ProxyChannelError::KeeperCannotElevate => {
                ErrorLayer::Keeper
            }
            _ => ErrorLayer::Config,
        }
    }
}

impl fmt::Display for ProxyChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyChannelError::ForgedInstanceIdentity { expected, claimed } => write!(
                f,
                "forged guest instance identity `{claimed}` (channel bound to `{expected}`)"
            ),
            ProxyChannelError::HostNotAllowed { host, phase } => {
                write!(f, "destination `{host}` refused in phase {phase:?}")
            }
            ProxyChannelError::ForbiddenDestinationIp { host, ip } => write!(
                f,
                "resolved address `{ip}` for `{host}` is a forbidden local/private destination"
            ),
            ProxyChannelError::MissingResolvedAddress(host) => {
                write!(f, "no resolved IP addresses supplied for `{host}`")
            }
            ProxyChannelError::RebootRequiredBeforeRuntime {
                setup_generation,
                found_generation,
            } => write!(
                f,
                "setup completed at generation {setup_generation}; runtime admission requires guest reboot (saw generation {found_generation})"
            ),
            ProxyChannelError::CredentialExposureNotDisclosed => write!(
                f,
                "refusing setup window on credential-holding instance until credential exposure is disclosed"
            ),
            ProxyChannelError::KeeperCannotElevate => {
                write!(
                    f,
                    "keeper or SSH sessions cannot open or execute setup authority"
                )
            }
            ProxyChannelError::SetupWindowClosed => {
                write!(f, "setup window is not active on control channel")
            }
            ProxyChannelError::HandshakeInvalid(msg) => {
                write!(f, "runtime admission refused: {msg}")
            }
        }
    }
}

impl std::error::Error for ProxyChannelError {}

/// Reject loopback, private, link-local/metadata, CGNAT, unspecified, and
/// multicast addresses for both IPv4 and IPv6 (including IPv4-mapped IPv6).
pub fn is_forbidden_private_or_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_forbidden_v4(v4),
        IpAddr::V6(v6) => is_forbidden_v6(v6),
    }
}

fn is_forbidden_v4(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    let is_cgnat = a == 100 && (64..=127).contains(&b);
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || is_cgnat
}

fn is_forbidden_v6(v6: Ipv6Addr) -> bool {
    if let Some(mapped) = v6.to_ipv4_mapped() {
        return is_forbidden_v4(mapped);
    }
    v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        || v6.is_unicast_link_local()
        || v6.is_unique_local()
}

struct ActiveSetupWindow {
    window_id: String,
    deadline: Instant,
    expires_at_epoch_secs: u64,
    credential_exposure_disclosed: bool,
}

struct ProxyState {
    phase: GuestPhase,
    prep_hosts: BTreeSet<String>,
    setup_hosts: BTreeSet<String>,
    runtime_hosts: BTreeSet<String>,
    setup_streams: Vec<UnixStream>,
    refused: Vec<RefusedDestination>,
    requires_reboot_before_runtime: bool,
    last_setup_generation: Option<u64>,
    setup_window: Option<ActiveSetupWindow>,
}

/// Host-held phase state and channel-bound egress policy for one VM instance.
pub struct HostPhaseProxy {
    instance_id: String,
    state: Mutex<ProxyState>,
}

impl HostPhaseProxy {
    pub fn new(
        instance_id: impl Into<String>,
        initial_phase: GuestPhase,
        prep_hosts: &[String],
        setup_hosts: &[String],
        runtime_hosts: &[String],
    ) -> Self {
        let normalize = |list: &[String]| -> BTreeSet<String> {
            list.iter().map(|h| h.to_ascii_lowercase()).collect()
        };
        Self {
            instance_id: instance_id.into(),
            state: Mutex::new(ProxyState {
                phase: initial_phase,
                prep_hosts: normalize(prep_hosts),
                setup_hosts: normalize(setup_hosts),
                runtime_hosts: normalize(runtime_hosts),
                setup_streams: Vec::new(),
                refused: Vec::new(),
                requires_reboot_before_runtime: false,
                last_setup_generation: None,
                setup_window: None,
            }),
        }
    }

    pub fn channel_instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn current_phase(&self) -> GuestPhase {
        let mut st = self.state.lock().expect("proxy lock poisoned");
        Self::expire_window_if_needed(&mut st);
        st.phase
    }

    pub fn refused_destinations(&self) -> Vec<RefusedDestination> {
        let st = self.state.lock().expect("proxy lock poisoned");
        st.refused.clone()
    }

    pub fn register_active_stream(&self, stream: UnixStream) {
        let mut st = self.state.lock().expect("proxy lock poisoned");
        Self::expire_window_if_needed(&mut st);
        if st.phase == GuestPhase::Setup {
            st.setup_streams.push(stream);
        }
    }

    /// Evaluate outbound proxy request against channel identity, active phase
    /// allowlist, and post-DNS private/local destination restrictions.
    pub fn evaluate_egress(
        &self,
        host: &str,
        port: u16,
        resolved_ips: &[IpAddr],
        guest_claimed_instance_id: Option<&str>,
    ) -> Result<(), ProxyChannelError> {
        if let Some(claimed) = guest_claimed_instance_id
            && claimed != self.instance_id
        {
            return Err(ProxyChannelError::ForgedInstanceIdentity {
                expected: self.instance_id.clone(),
                claimed: claimed.to_owned(),
            });
        }
        let mut st = self.state.lock().expect("proxy lock poisoned");
        Self::expire_window_if_needed(&mut st);

        let norm = host.to_ascii_lowercase();
        let active_phase = st.phase;
        let allowed = match active_phase {
            GuestPhase::Preparation => st.prep_hosts.contains(&norm),
            GuestPhase::Setup => st.setup_hosts.contains(&norm),
            GuestPhase::Runtime => st.runtime_hosts.contains(&norm),
        };
        if !allowed {
            st.refused.push(RefusedDestination {
                host: norm.clone(),
                port,
                phase: active_phase,
                reason: "not allowed in active phase".into(),
            });
            return Err(ProxyChannelError::HostNotAllowed {
                host: norm,
                phase: active_phase,
            });
        }
        if resolved_ips.is_empty() {
            return Err(ProxyChannelError::MissingResolvedAddress(norm));
        }
        for ip in resolved_ips {
            if is_forbidden_private_or_local_ip(*ip) {
                st.refused.push(RefusedDestination {
                    host: norm.clone(),
                    port,
                    phase: active_phase,
                    reason: format!("resolved IP `{ip}` is forbidden private/local/link-local"),
                });
                return Err(ProxyChannelError::ForbiddenDestinationIp {
                    host: norm,
                    ip: *ip,
                });
            }
        }
        Ok(())
    }

    /// Close setup streams host-side, switch policy to `Runtime`, and require
    /// a fresh guest reboot generation before runtime admission (`3.3`).
    pub fn end_setup_phase(&self, setup_generation: u64) -> SetupRevocationReceipt {
        let mut st = self.state.lock().expect("proxy lock poisoned");
        let closed = close_streams(&mut st.setup_streams);
        st.phase = GuestPhase::Runtime;
        st.setup_window = None;
        st.last_setup_generation = Some(setup_generation);
        st.requires_reboot_before_runtime = true;
        SetupRevocationReceipt {
            closed_setup_streams: closed,
            setup_generation,
            requires_reboot_before_runtime: true,
        }
    }

    /// Enforce that setup-time processes cannot outlive creation setup: reject
    /// runtime admission unless the verified handshake reflects a reboot
    /// (`handshake.instance_generation > setup_generation`).
    pub fn admit_runtime_after_reboot(
        &self,
        handshake: &HandshakeV1,
        launch: &LaunchRecordV1,
        expected_protocol: u32,
    ) -> Result<(), ProxyChannelError> {
        handshake
            .validate_against(launch, expected_protocol)
            .map_err(|e| ProxyChannelError::HandshakeInvalid(e.to_string()))?;
        let mut st = self.state.lock().expect("proxy lock poisoned");
        if st.requires_reboot_before_runtime
            && let Some(setup_gen) = st.last_setup_generation
            && handshake.instance_generation <= setup_gen
        {
            return Err(ProxyChannelError::RebootRequiredBeforeRuntime {
                setup_generation: setup_gen,
                found_generation: handshake.instance_generation,
            });
        }
        st.requires_reboot_before_runtime = false;
        Ok(())
    }

    /// Explicitly widen a running instance to setup policy for bounded duration
    /// (`3.6`). Discloses direct credential exposure first if any exist.
    pub fn open_setup_window(
        &self,
        window_id: &str,
        duration: Duration,
        instance_holds_direct_credentials: bool,
        exposure_acknowledged: bool,
    ) -> Result<SetupWindowStatus, ProxyChannelError> {
        if instance_holds_direct_credentials && !exposure_acknowledged {
            return Err(ProxyChannelError::CredentialExposureNotDisclosed);
        }
        let mut st = self.state.lock().expect("proxy lock poisoned");
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let expires_at_epoch_secs = now_epoch.saturating_add(duration.as_secs());
        st.phase = GuestPhase::Setup;
        st.setup_window = Some(ActiveSetupWindow {
            window_id: window_id.to_owned(),
            deadline: Instant::now() + duration,
            expires_at_epoch_secs,
            credential_exposure_disclosed: instance_holds_direct_credentials,
        });
        Ok(SetupWindowStatus {
            active: true,
            window_id: Some(window_id.to_owned()),
            expires_at_epoch_secs: Some(expires_at_epoch_secs),
            credential_exposure_disclosed: instance_holds_direct_credentials,
        })
    }

    pub fn close_setup_window(&self) -> usize {
        let mut st = self.state.lock().expect("proxy lock poisoned");
        st.phase = GuestPhase::Runtime;
        st.setup_window = None;
        close_streams(&mut st.setup_streams)
    }

    pub fn window_status(&self) -> SetupWindowStatus {
        let mut st = self.state.lock().expect("proxy lock poisoned");
        Self::expire_window_if_needed(&mut st);
        match &st.setup_window {
            Some(w) => SetupWindowStatus {
                active: true,
                window_id: Some(w.window_id.clone()),
                expires_at_epoch_secs: Some(w.expires_at_epoch_secs),
                credential_exposure_disclosed: w.credential_exposure_disclosed,
            },
            None => SetupWindowStatus {
                active: false,
                window_id: None,
                expires_at_epoch_secs: None,
                credential_exposure_disclosed: false,
            },
        }
    }

    /// Only the init-owned `control.sock` route may perform setup actions, and
    /// only while a non-expired setup window is active.
    pub fn authorize_admin_operation(
        &self,
        origin: AdminChannelOrigin,
    ) -> Result<(), ProxyChannelError> {
        match origin {
            AdminChannelOrigin::KeeperSession | AdminChannelOrigin::SshSession => {
                Err(ProxyChannelError::KeeperCannotElevate)
            }
            AdminChannelOrigin::ControlInit => {
                let mut st = self.state.lock().expect("proxy lock poisoned");
                Self::expire_window_if_needed(&mut st);
                if st.phase == GuestPhase::Setup && st.setup_window.is_some() {
                    Ok(())
                } else {
                    Err(ProxyChannelError::SetupWindowClosed)
                }
            }
        }
    }

    fn expire_window_if_needed(st: &mut ProxyState) {
        if let Some(w) = &st.setup_window
            && Instant::now() >= w.deadline
        {
            st.phase = GuestPhase::Runtime;
            st.setup_window = None;
            let _ = close_streams(&mut st.setup_streams);
        }
    }
}

fn close_streams(streams: &mut Vec<UnixStream>) -> usize {
    let count = streams.len();
    for s in streams.drain(..) {
        let _ = s.shutdown(Shutdown::Both);
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::contract::{
        CONTRACT_SCHEMA_V1, GuestTarget, HostChannels, ResourceBudgets, VmAdapter,
    };
    use std::io::Read;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn sample_launch(gen_id: u64) -> LaunchRecordV1 {
        LaunchRecordV1 {
            schema: CONTRACT_SCHEMA_V1,
            instance_generation: gen_id,
            target: GuestTarget::LinuxArm64,
            adapter: VmAdapter::Libkrun,
            phase: GuestPhase::Runtime,
            boot_artifact_sha256: SHA_A.into(),
            required_capabilities: vec!["nono".into()],
            disks: vec![],
            channels: HostChannels {
                control_socket: "/inst/control.sock".into(),
                keeper_socket: "/inst/keeper.sock".into(),
                ssh_socket: "/inst/ssh.sock".into(),
                proxy_socket: Some("/inst/proxy.sock".into()),
                allow_nic: false,
                allow_tsi: false,
            },
            resources: ResourceBudgets {
                vcpus: 2,
                guest_memory_mib: 512,
                writable_disk_mib: 1024,
                log_bytes_cap: 1024 * 1024,
                readiness_timeout_secs: 60,
            },
        }
    }

    fn sample_hs(gen_id: u64) -> HandshakeV1 {
        HandshakeV1 {
            schema: CONTRACT_SCHEMA_V1,
            instance_generation: gen_id,
            protocol_version: 1,
            target: GuestTarget::LinuxArm64,
            boot_artifact_sha256: SHA_A.into(),
            capabilities: vec!["nono".into()],
        }
    }

    #[test]
    fn dns_private_local_ips_and_forged_instance_ids_are_refused() {
        let proxy = HostPhaseProxy::new(
            "inst-alpha",
            GuestPhase::Runtime,
            &[],
            &[],
            &["allowed.example".into()],
        );

        let public_ip: IpAddr = "93.184.216.34".parse().unwrap();
        proxy
            .evaluate_egress("ALLOWED.EXAMPLE", 443, &[public_ip], Some("inst-alpha"))
            .unwrap();

        assert!(matches!(
            proxy.evaluate_egress("allowed.example", 443, &[public_ip], Some("inst-beta")),
            Err(ProxyChannelError::ForgedInstanceIdentity { .. })
        ));

        for bad in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.5",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            let ip: IpAddr = bad.parse().unwrap();
            assert!(
                matches!(
                    proxy.evaluate_egress("allowed.example", 443, &[ip], None),
                    Err(ProxyChannelError::ForbiddenDestinationIp { .. })
                ),
                "expected refusal for {bad}"
            );
        }

        assert!(matches!(
            proxy.evaluate_egress("ungranted.example", 443, &[public_ip], None),
            Err(ProxyChannelError::HostNotAllowed { .. })
        ));
        assert!(!proxy.refused_destinations().is_empty());
    }

    #[test]
    fn setup_revocation_closes_lingering_streams_and_requires_reboot() {
        let proxy = HostPhaseProxy::new(
            "inst-setup",
            GuestPhase::Setup,
            &[],
            &["dl-cdn.alpinelinux.org".into()],
            &["api.example".into()],
        );
        let (mut guest_end, proxy_end) = UnixStream::pair().unwrap();
        proxy.register_active_stream(proxy_end);

        let receipt = proxy.end_setup_phase(1);
        assert_eq!(receipt.closed_setup_streams, 1);
        assert_eq!(proxy.current_phase(), GuestPhase::Runtime);

        let mut buf = [0u8; 8];
        assert_eq!(guest_end.read(&mut buf).unwrap(), 0);

        let pub_ip: IpAddr = "151.101.2.132".parse().unwrap();
        assert!(matches!(
            proxy.evaluate_egress("dl-cdn.alpinelinux.org", 443, &[pub_ip], None),
            Err(ProxyChannelError::HostNotAllowed { .. })
        ));

        // Same generation (guest claiming cleanup without reboot) must be refused.
        assert!(matches!(
            proxy.admit_runtime_after_reboot(&sample_hs(1), &sample_launch(1), 1),
            Err(ProxyChannelError::RebootRequiredBeforeRuntime {
                setup_generation: 1,
                found_generation: 1
            })
        ));
        proxy
            .admit_runtime_after_reboot(&sample_hs(2), &sample_launch(2), 1)
            .unwrap();
    }

    #[test]
    fn setup_windows_block_keeper_elevation_and_require_credential_disclosure() {
        let proxy = HostPhaseProxy::new(
            "inst-win",
            GuestPhase::Runtime,
            &[],
            &["dl-cdn.alpinelinux.org".into()],
            &[],
        );
        assert!(matches!(
            proxy.authorize_admin_operation(AdminChannelOrigin::KeeperSession),
            Err(ProxyChannelError::KeeperCannotElevate)
        ));
        assert!(matches!(
            proxy.authorize_admin_operation(AdminChannelOrigin::ControlInit),
            Err(ProxyChannelError::SetupWindowClosed)
        ));
        assert!(matches!(
            proxy.open_setup_window("w1", Duration::from_secs(30), true, false),
            Err(ProxyChannelError::CredentialExposureNotDisclosed)
        ));

        let st = proxy
            .open_setup_window("w1", Duration::from_secs(30), true, true)
            .unwrap();
        assert!(st.active);
        assert!(st.credential_exposure_disclosed);
        proxy
            .authorize_admin_operation(AdminChannelOrigin::ControlInit)
            .unwrap();
        assert!(matches!(
            proxy.authorize_admin_operation(AdminChannelOrigin::KeeperSession),
            Err(ProxyChannelError::KeeperCannotElevate)
        ));

        let (mut peer, stream) = UnixStream::pair().unwrap();
        proxy.register_active_stream(stream);
        assert_eq!(proxy.close_setup_window(), 1);
        let mut b = [0u8; 4];
        assert_eq!(peer.read(&mut b).unwrap(), 0);
        assert!(!proxy.window_status().active);
    }
}
