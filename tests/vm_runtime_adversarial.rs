//! End-to-end adversarial host/access controls, positive controls, and
//! lifecycle integration tests for `devcroft::vm` (`add-vm-project-runtime`
//! task 4.4).

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::IpAddr;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use devcroft::keeper::{ExitStatus, Keeper, LocalSessionBackend, SpawnRequest};
use devcroft::vm::{
    AdminChannelOrigin, ArtifactCacheStore, ComponentHealth, CrossProjectEligibility,
    DirectCredential, DirectCredentialDelivery, ErrorLayer, ExplorationProfileV1, GuestPhase,
    GuestTarget, HostChannels, HostPhaseProxy, HostPlatform, HostProcessIdentity,
    InstanceCredentialState, LaunchRecordV1, ProjectAuthorityRequest, ProjectIdentity,
    ResourceBudgets, RetentionLeaseV1, RuntimeGrantStore, RuntimeKind, RuntimePolicyMode,
    RuntimeRequest, RuntimeSelectionSource, StopClassification, TrustedArtifactManifestV1,
    VmAdapter, VmHealthBreakdown, VmInstancePaths, VmKeeperClient, VmRollbackPolicy,
    VmSshConfigOutcome, VmStateRecordV1, compile_effective_authority,
    deliver_post_boot_credentials, export_retained_vm_disk, graceful_down, guard_new_vm_creation,
    guard_shared_base_promotion, is_owned_helper_alive, prepare_cold_restart, qualification_for,
    remove_owned_instance, reset_credentials_for_new_instance_or_refresh, resolve_runtime,
    save_vm_state, ssh_config_for_vm_instance, wait_for_verified_handshake,
};

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SIG_HEX: &str = "22222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222";

#[test]
fn runtime_dispatch_never_allows_repo_escalation_or_silent_fallback() {
    let repo = ProjectIdentity::from_root(Path::new("/work/unreviewed-project")).unwrap();
    let mut grants = RuntimeGrantStore::default();

    // Unqualified platforms fail closed without native fallback.
    for platform in [
        HostPlatform::LinuxArm64,
        HostPlatform::LinuxX86_64,
        HostPlatform::MacosX86_64,
    ] {
        assert!(!qualification_for(platform).is_qualified());
        let err = resolve_runtime(&RuntimeRequest {
            project: &repo,
            provider: "nix",
            existing_instance: None,
            explicit_vm_opt_in: false,
            policy_mode: RuntimePolicyMode::RequireNativeGrant,
            grants: &grants,
            host: platform,
            stdin_is_terminal: true,
        })
        .unwrap_err();
        assert_eq!(err.layer(), ErrorLayer::Backend);
    }

    // Darwin-only `swift` refuses without a user grant whether or not a TTY exists.
    let err = resolve_runtime(&RuntimeRequest {
        project: &repo,
        provider: "swift",
        existing_instance: None,
        explicit_vm_opt_in: false,
        policy_mode: RuntimePolicyMode::RequireNativeGrant,
        grants: &grants,
        host: HostPlatform::MacosArm64,
        stdin_is_terminal: true,
    })
    .unwrap_err();
    assert_eq!(err.layer(), ErrorLayer::Config);

    // Non-Darwin project selects qualified VM default on macos-arm64.
    let vm = resolve_runtime(&RuntimeRequest {
        project: &repo,
        provider: "nix",
        existing_instance: None,
        explicit_vm_opt_in: false,
        policy_mode: RuntimePolicyMode::RequireNativeGrant,
        grants: &grants,
        host: HostPlatform::MacosArm64,
        stdin_is_terminal: false,
    })
    .unwrap();
    assert_eq!(
        vm.runtime,
        RuntimeKind::Vm {
            target: GuestTarget::LinuxArm64,
            adapter: VmAdapter::Libkrun,
        }
    );

    // Granting trusted-editor must NOT grant native runtime execution, and ssh-config
    // gates Remote-SSH on that separate grant.
    grants.grant_trusted_editor(&repo, "2026-11-26T19:40:00Z");
    assert!(!grants.has_native_grant(&repo));
    assert!(matches!(
        ssh_config_for_vm_instance(
            "demo",
            &repo,
            &grants,
            "devcroft vm-proxy demo",
            Path::new("/ctrl/id_ed25519"),
            Path::new("/ctrl/known_hosts"),
        ),
        VmSshConfigOutcome::Emitted(_)
    ));
}

#[test]
fn trusted_bootstrap_and_authority_boundaries_reject_bypass_attempts() {
    let manifest_json = format!(
        r#"{{
          "schema": 1,
          "manifest_version": "2026.11.1",
          "min_compatible_controller": "0.0.1",
          "max_compatible_controller": "0.0.5",
          "publisher_key_id": "release-1",
          "signature_ed25519": "{SIG_HEX}",
          "target": "linux-arm64",
          "artifacts": {{
            "helper": {{"relative_path": "bin/helper", "sha256": "{SHA_A}", "size_bytes": 100, "license_id": "Apache-2.0", "source_url": "https://example.invalid/h"}},
            "kernel": {{"relative_path": "boot/vmlinux", "sha256": "{SHA_A}", "size_bytes": 100, "license_id": "GPL-2.0-only", "source_url": "https://example.invalid/k"}},
            "initramfs": {{"relative_path": "boot/init", "sha256": "{SHA_A}", "size_bytes": 100, "license_id": "Apache-2.0", "source_url": "https://example.invalid/i"}},
            "runtime_image": {{"relative_path": "img/rt.img", "sha256": "{SHA_A}", "size_bytes": 100, "license_id": "Apache-2.0", "source_url": "https://example.invalid/r"}},
            "preparation_image": {{"relative_path": "img/prep.img", "sha256": "{SHA_A}", "size_bytes": 100, "license_id": "Apache-2.0", "source_url": "https://example.invalid/p"}},
            "license_inventory": {{"relative_path": "lic.json", "sha256": "{SHA_A}", "size_bytes": 100, "license_id": "Apache-2.0", "source_url": "https://example.invalid/l"}}
          }}
        }}"#
    );
    let manifest = TrustedArtifactManifestV1::from_json_str(&manifest_json).unwrap();
    // Bootstrap stored inside the project checkout is refused.
    assert!(
        manifest
            .packaged_layout(
                Path::new("/work/evil-repo/.bootstrap"),
                Path::new("/work/evil-repo"),
            )
            .is_err()
    );
    manifest
        .packaged_layout(
            Path::new("/trusted/vm-artifacts"),
            Path::new("/work/evil-repo"),
        )
        .unwrap();

    // First-open project gets exploration egress for prep/setup, empty runtime egress,
    // and host mount requests are refused.
    let mut bad_req = ProjectAuthorityRequest {
        requested_host_mounts: vec!["/Users/op/.ssh".into()],
        ..Default::default()
    };
    let budgets = ResourceBudgets {
        vcpus: 2,
        guest_memory_mib: 512,
        writable_disk_mib: 1024,
        log_bytes_cap: 1024 * 1024,
        readiness_timeout_secs: 60,
    };
    assert!(
        compile_effective_authority(
            &bad_req,
            None,
            &ExplorationProfileV1::default_profile(),
            &budgets,
            false,
            HostPlatform::MacosArm64,
        )
        .is_err()
    );
    bad_req.requested_host_mounts.clear();
    let auth = compile_effective_authority(
        &bad_req,
        None,
        &ExplorationProfileV1::default_profile(),
        &budgets,
        false,
        HostPlatform::MacosArm64,
    )
    .unwrap();
    assert!(auth.runtime_egress.is_empty());

    // Host proxy blocks forged instance ID and DNS rebinding to metadata/loopback/IPv6-mapped IPv4.
    let proxy = HostPhaseProxy::new(
        "inst-real",
        GuestPhase::Setup,
        &[],
        &["dl-cdn.alpinelinux.org".into()],
        &["api.github.com".into()],
    );
    assert!(
        proxy
            .evaluate_egress(
                "dl-cdn.alpinelinux.org",
                443,
                &["169.254.169.254".parse::<IpAddr>().unwrap()],
                None,
            )
            .is_err()
    );
    assert!(
        proxy
            .evaluate_egress(
                "dl-cdn.alpinelinux.org",
                443,
                &["::ffff:127.0.0.1".parse::<IpAddr>().unwrap()],
                None,
            )
            .is_err()
    );
    assert!(
        proxy
            .evaluate_egress(
                "dl-cdn.alpinelinux.org",
                443,
                &["151.101.2.132".parse::<IpAddr>().unwrap()],
                Some("inst-other"),
            )
            .is_err()
    );

    // Setup completion revokes lingering setup streams host-side and blocks keeper elevation.
    let (mut peer, stream) = UnixStream::pair().unwrap();
    proxy.register_active_stream(stream);
    assert_eq!(proxy.end_setup_phase(1).closed_setup_streams, 1);
    let mut buf = [0u8; 4];
    assert_eq!(peer.read(&mut buf).unwrap(), 0);
    assert!(
        proxy
            .authorize_admin_operation(AdminChannelOrigin::KeeperSession)
            .is_err()
    );
    proxy
        .open_setup_window("w1", Duration::from_secs(30), false, true)
        .unwrap();
    proxy
        .authorize_admin_operation(AdminChannelOrigin::ControlInit)
        .unwrap();
    proxy.close_setup_window();
}

#[test]
fn end_to_end_handshake_keeper_credentials_cache_lifecycle_and_rollback() {
    let base = std::env::temp_dir().join(format!("dc-vm-e2e-adv-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    // 1. Handshake + credential + STOP control server and real Keeper on sockets.
    let ctrl_dir = base.join("inst-a-ctrl");
    let inst_dir = base.join("inst-a-work");
    fs::create_dir_all(&ctrl_dir).unwrap();
    fs::create_dir_all(&inst_dir).unwrap();
    let paths_a = VmInstancePaths {
        state_file: ctrl_dir.join("vm-state.json"),
        private_disk: inst_dir.join("workspace.img"),
        control_socket: inst_dir.join("control.sock"),
        keeper_socket: inst_dir.join("keeper.sock"),
        controller_dir: ctrl_dir,
        instance_dir: inst_dir,
    };
    fs::write(&paths_a.private_disk, b"adv-test-private-ext4-image").unwrap();

    let control_listener = UnixListener::bind(&paths_a.control_socket).unwrap();
    let keeper_listener = UnixListener::bind(&paths_a.keeper_socket).unwrap();

    let ready_hs = format!(
        r#"READY {{"schema":1,"instance_generation":2,"protocol_version":1,"target":"linux-arm64","boot_artifact_sha256":"{SHA_A}","capabilities":["nono","ssh"]}}"#
    );
    thread::spawn(move || {
        for conn in control_listener.incoming().take(3).flatten() {
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_ok() {
                let mut w = conn;
                let trimmed = line.trim();
                if trimmed == "HANDSHAKE" {
                    let _ = w.write_all(ready_hs.as_bytes());
                    let _ = w.write_all(b"\n");
                } else if trimmed.starts_with("CRED-SET 2 ") {
                    let _ = w.write_all(b"CRED-OK 2\n");
                } else if trimmed == "SYNC-STOP 2 stop-nonce-1" {
                    let _ = w.write_all(b"SYNCED 2 stop-nonce-1\n");
                } else {
                    let _ = w.write_all(b"ERR unknown\n");
                }
            }
        }
    });
    thread::spawn(move || {
        let _ = Keeper::new(keeper_listener, Arc::new(LocalSessionBackend)).serve();
    });

    let launch = LaunchRecordV1 {
        schema: 1,
        instance_generation: 2,
        target: GuestTarget::LinuxArm64,
        adapter: VmAdapter::Libkrun,
        phase: GuestPhase::Runtime,
        boot_artifact_sha256: SHA_A.into(),
        required_capabilities: vec!["nono".into(), "ssh".into()],
        disks: vec![],
        channels: HostChannels {
            control_socket: paths_a.control_socket.display().to_string(),
            keeper_socket: paths_a.keeper_socket.display().to_string(),
            ssh_socket: "/inst/ssh.sock".into(),
            proxy_socket: None,
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
    };
    let hs =
        wait_for_verified_handshake(&paths_a.control_socket, &launch, 1, Duration::from_secs(2))
            .unwrap();
    assert_eq!(hs.instance_generation, 2);

    // Positive control through real guest keeper transport.
    let out = VmKeeperClient::new(&paths_a.keeper_socket, Duration::from_secs(5))
        .run_command(
            SpawnRequest {
                cmd: "/bin/sh".into(),
                args: vec!["-c".into(), "cat; exit 0".into()],
                cwd: "/tmp".into(),
                env: BTreeMap::new(),
                pty: None,
            },
            b"adversarial-positive-control",
        )
        .unwrap();
    assert_eq!(out.stdout, b"adversarial-positive-control");
    assert_eq!(
        out.exit,
        ExitStatus {
            code: Some(0),
            signal: None
        }
    );

    // Post-boot credential delivery redacts secret values and blocks shared-base promotion.
    let delivery = DirectCredentialDelivery::new(vec![DirectCredential {
        name: "OPENAI_API_KEY".into(),
        value: "sk-secret-e2e-test".into(),
    }])
    .unwrap();
    let mut cred_state = InstanceCredentialState {
        instance_id: "inst-a".into(),
        source_revision_sha256: SHA_A.into(),
        delivered_names: Vec::new(),
        ever_held_direct_credentials: false,
    };
    deliver_post_boot_credentials(
        &paths_a.control_socket,
        2,
        &delivery,
        &mut cred_state,
        Duration::from_secs(2),
    )
    .unwrap();
    assert!(guard_shared_base_promotion(&cred_state).is_err());
    let refreshed = reset_credentials_for_new_instance_or_refresh(
        &cred_state,
        "inst-a",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .unwrap();
    assert!(refreshed.delivered_names.is_empty());

    // Save state with stale reused PID (must NOT match), shutdown cleanly, prepare cold restart,
    // verify publication isolation, rollback export, and owned removal + GC.
    let mut st = VmStateRecordV1 {
        schema: 1,
        runtime_kind: RuntimeKind::Vm {
            target: GuestTarget::LinuxArm64,
            adapter: VmAdapter::Libkrun,
        },
        selection_source: RuntimeSelectionSource::QualifiedVmDefault,
        instance_generation: 2,
        host_process: Some(HostProcessIdentity {
            pid: std::process::id(),
            start_time_ns: 1,
        }),
        private_disk_id: "disk-a".into(),
        boot_artifact_sha256: SHA_A.into(),
        health: VmHealthBreakdown {
            helper: ComponentHealth::Stopped,
            transport: ComponentHealth::Stopped,
            keeper: ComponentHealth::Stopped,
            environment_fresh: true,
            services: ComponentHealth::Stopped,
        },
        last_stop: None,
    };
    assert!(!is_owned_helper_alive(st.host_process.as_ref().unwrap()));
    save_vm_state(&paths_a.state_file, &st).unwrap();
    let down_out = graceful_down(
        &paths_a,
        &mut st,
        "stop-nonce-1",
        Duration::from_secs(2),
        Duration::from_millis(20),
    )
    .unwrap();
    assert_eq!(down_out.classification, StopClassification::Orderly);

    let restart = prepare_cold_restart(
        &paths_a,
        GuestTarget::LinuxArm64,
        VmAdapter::Libkrun,
        "disk-a",
    )
    .unwrap();
    assert_eq!(restart.next_generation, 3);
    assert!(!restart.recovery_check_required);

    assert!(
        guard_new_vm_creation(
            VmRollbackPolicy {
                creation_disabled: true
            },
            None
        )
        .is_err()
    );
    let exported = base.join("rollback-export.img");
    export_retained_vm_disk(&paths_a, &exported).unwrap();

    // Namespace cache + lease GC on removal.
    let store = ArtifactCacheStore::new(base.join("cache"), 1024 * 1024);
    let proj_a = ProjectIdentity::from_root(Path::new("/work/proj-a")).unwrap();
    let proj_b = ProjectIdentity::from_root(Path::new("/work/proj-b")).unwrap();
    let blob_src = base.join("blob.img");
    fs::write(&blob_src, b"sealed-blob").unwrap();
    let staged = store
        .stage_guest_output(
            "tx-e2e",
            br#"{"schema":1,"format":"ext4-raw","size_bytes":11,"guest_notes":null}"#,
            &blob_src,
        )
        .unwrap();
    let rec = store
        .commit_publication(
            staged,
            &proj_a,
            SHA_A,
            CrossProjectEligibility::ProjectPrivateOnly,
            "2026-11-26T19:42:00Z",
        )
        .unwrap();
    assert!(store.lookup_for_project(&proj_b, SHA_A).unwrap().is_none());
    store
        .acquire_lease(
            &proj_a,
            &rec.content_sha256,
            &RetentionLeaseV1 {
                instance_id: "inst-a".into(),
                instance_generation: 2,
                acquired_at: "t1".into(),
            },
        )
        .unwrap();

    let rm_rep = remove_owned_instance(
        &paths_a,
        "inst-a",
        &proj_a,
        Some((&store, &rec.content_sha256)),
    )
    .unwrap();
    assert!(rm_rep.removed_instance_dir);
    assert_eq!(rm_rep.reclaimed_cache_artifacts, vec![rec.content_sha256]);
    assert!(exported.is_file());

    let _ = fs::remove_dir_all(&base);
}
