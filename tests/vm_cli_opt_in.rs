use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use devcroft::vm::{
    ArtifactEntry, ComponentHealth, GuestTarget, RuntimeGrantStore, RuntimeKind,
    RuntimeSelectionSource, TrustedArtifactManifestV1, TrustedArtifactsV1, VmAdapter,
    VmCliMetadata, VmHealthBreakdown, VmSshKeyMaterial, VmStateRecordV1,
    contract::CONTRACT_SCHEMA_V1, save_vm_state, sha256_file_hex,
};

fn devcroft_bin() -> &'static str {
    env!("CARGO_BIN_EXE_devcroft")
}

struct TempHarness {
    root: PathBuf,
    home: PathBuf,
    xdg_config: PathBuf,
    sandbox_to_cleanup: Option<String>,
}

impl TempHarness {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root =
            std::env::temp_dir().join(format!("dc-vm-cli-{tag}-{}-{nanos}", std::process::id()));
        let home = root.join("home");
        let xdg_config = root.join("xdg-config");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&xdg_config).unwrap();
        Self {
            root,
            home,
            xdg_config,
            sandbox_to_cleanup: None,
        }
    }

    fn run_in(&self, cwd: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(devcroft_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.xdg_config)
            .env_remove("DEVCROFT_VM_MANIFEST")
            .env_remove("DEVCROFT_VM_ROLLBACK");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd.output().expect("execute devcroft binary")
    }
}

impl Drop for TempHarness {
    fn drop(&mut self) {
        if let Some(name) = &self.sandbox_to_cleanup {
            let _ = Command::new(devcroft_bin())
                .args(["rm", name, "--yes"])
                .env("HOME", &self.home)
                .env("XDG_CONFIG_HOME", &self.xdg_config)
                .output();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_project_manifest(dir: &Path, name: &str, provider: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("devcroft.toml"),
        format!("[sandbox]\nname = {name:?}\n\n[env]\nprovider = {provider:?}\n"),
    )
    .unwrap();
}

#[test]
fn runtime_grants_cli_roundtrip() {
    let h = TempHarness::new("grants");
    let proj = h.root.join("myproj");
    write_project_manifest(&proj, "myproj", "flox");

    let st = h.run_in(&proj, &["runtime", "status"], &[]);
    assert_eq!(
        st.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&st.stderr)
    );
    let st_out = String::from_utf8_lossy(&st.stdout);
    assert!(st_out.contains("default-runtime: process"), "{st_out}");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    assert!(
        st_out.contains("qualified-opt-in (libkrun -> linux-arm64"),
        "{st_out}"
    );

    let ls_empty = h.run_in(&proj, &["runtime", "list"], &[]);
    assert_eq!(ls_empty.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&ls_empty.stdout).contains("no runtime grants"));

    let grant_te = h.run_in(
        &proj,
        &[
            "runtime",
            "grant-trusted-editor",
            "--project",
            proj.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(
        grant_te.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&grant_te.stderr)
    );

    let grants_path = h.xdg_config.join("devcroft/runtime-grants.toml");
    assert!(grants_path.is_file());
    assert_eq!(
        fs::metadata(&grants_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // Granting trusted-editor must NOT elevate runtime to native.
    let store = RuntimeGrantStore::load_from(&grants_path).unwrap();
    let id = devcroft::vm::ProjectIdentity::from_root(&proj).unwrap();
    assert!(store.has_trusted_editor_grant(&id));
    assert!(!store.has_native_grant(&id));

    let grant_native = h.run_in(
        &proj,
        &[
            "runtime",
            "grant-native",
            "--project",
            proj.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(grant_native.status.code(), Some(0));

    let ls = h.run_in(&proj, &["runtime", "list"], &[]);
    assert_eq!(ls.status.code(), Some(0));
    let ls_out = String::from_utf8_lossy(&ls.stdout);
    assert!(ls_out.contains("runtime=native"), "{ls_out}");
    assert!(ls_out.contains("trusted_editor=yes"), "{ls_out}");
}

#[test]
fn up_vm_refuses_swift_repo_manifest_and_rollback() {
    let h = TempHarness::new("refusals");

    // 1. Darwin-only `swift` provider fails closed under `--vm` (provider layer -> exit 3).
    let swift_proj = h.root.join("swiftproj");
    write_project_manifest(&swift_proj, "swiftproj", "swift");
    let res_swift = h.run_in(&swift_proj, &["up", "--vm"], &[]);
    assert_eq!(
        res_swift.status.code(),
        Some(3),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&res_swift.stdout),
        String::from_utf8_lossy(&res_swift.stderr)
    );
    assert!(
        !h.home
            .join(".local/share/devcroft/swiftproj/meta.json")
            .exists(),
        "swift VM refusal must never fall back to native state"
    );

    // 2. Rollback policy blocks new VM creation fail-closed (backend layer -> exit 4).
    let proj = h.root.join("floxproj");
    write_project_manifest(&proj, "floxproj", "flox");
    let res_rollback = h.run_in(&proj, &["up", "--vm"], &[("DEVCROFT_VM_ROLLBACK", "1")]);
    assert_eq!(
        res_rollback.status.code(),
        Some(4),
        "stderr: {}",
        String::from_utf8_lossy(&res_rollback.stderr)
    );

    // 3. Repo-local bootstrap manifest is refused by packaged_layout (on qualified host -> exit 4).
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let repo_bundle = proj.join("evil-bundle/2026.11.1");
        fs::create_dir_all(&repo_bundle).unwrap();
        let sha = "1111111111111111111111111111111111111111111111111111111111111111";
        let sig = "22222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222";
        let art = |rel: &str| ArtifactEntry {
            relative_path: rel.into(),
            sha256: sha.into(),
            size_bytes: 64,
            license_id: "Apache-2.0".into(),
            source_url: "https://example.invalid/art".into(),
        };
        let manifest = TrustedArtifactManifestV1 {
            schema: CONTRACT_SCHEMA_V1,
            manifest_version: "2026.11.1".into(),
            min_compatible_controller: "0.0.1".into(),
            max_compatible_controller: "9.9.9".into(),
            publisher_key_id: "k1".into(),
            signature_ed25519: sig.into(),
            target: GuestTarget::LinuxArm64,
            artifacts: TrustedArtifactsV1 {
                helper: art("helper"),
                kernel: art("Image"),
                initramfs: art("initramfs.cpio"),
                runtime_image: art("runtime.img"),
                preparation_image: art("prep.img"),
                license_inventory: art("LICENSES.txt"),
                workload: None,
                data_seed: None,
            },
        };
        let manifest_path = proj.join("evil-bundle/manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let res_repo = h.run_in(
            &proj,
            &[
                "up",
                "--vm",
                "--vm-manifest",
                manifest_path.to_str().unwrap(),
            ],
            &[],
        );
        assert_eq!(
            res_repo.status.code(),
            Some(4),
            "stderr: {}",
            String::from_utf8_lossy(&res_repo.stderr)
        );
        let stderr = String::from_utf8_lossy(&res_repo.stderr);
        assert!(stderr.contains("inside repository root"), "{stderr}");
    }
}

#[test]
fn vm_ssh_config_gates_on_trusted_editor_grant_and_preserves_native_default() {
    let h = TempHarness::new("ssh-gate");
    let proj = h.root.join("vm-editor-proj");
    fs::create_dir_all(&proj).unwrap();

    // Native `ssh-config` remains unaffected and emits `Host *.devcroft`.
    let native_cfg = h.run_in(&proj, &["ssh-config"], &[]);
    assert_eq!(native_cfg.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&native_cfg.stdout).contains("Host *.devcroft"),
        "{}",
        String::from_utf8_lossy(&native_cfg.stdout)
    );

    // Seed synthetic VM state + keys inside isolated HOME.
    let ctrl = h.home.join(".local/share/devcroft/eddemo/vm-ctrl");
    let inst = h.home.join(".local/share/devcroft/eddemo/vm-inst");
    fs::create_dir_all(&ctrl).unwrap();
    fs::create_dir_all(&inst).unwrap();
    let keys = VmSshKeyMaterial::generate(&ctrl, "eddemo.devcroft", &"a".repeat(32)).unwrap();

    let state = VmStateRecordV1 {
        schema: CONTRACT_SCHEMA_V1,
        runtime_kind: RuntimeKind::Vm {
            target: GuestTarget::LinuxArm64,
            adapter: VmAdapter::Libkrun,
        },
        selection_source: RuntimeSelectionSource::ExplicitOptIn,
        instance_generation: 1,
        host_process: None,
        private_disk_id: "vm-state-eddemo".into(),
        boot_artifact_sha256: "b".repeat(64),
        health: VmHealthBreakdown {
            helper: ComponentHealth::Stopped,
            transport: ComponentHealth::Stopped,
            keeper: ComponentHealth::Stopped,
            environment_fresh: true,
            services: ComponentHealth::Stopped,
        },
        last_stop: None,
    };
    save_vm_state(&ctrl.join("vm-state.json"), &state).unwrap();

    let meta = VmCliMetadata {
        schema: 1,
        sandbox_name: "eddemo".into(),
        project_root: fs::canonicalize(&proj).unwrap(),
        controller_dir: fs::canonicalize(&ctrl).unwrap(),
        instance_dir: fs::canonicalize(&inst).unwrap(),
        private_disk: inst.join("state.img"),
        helper_log: ctrl.join("helper.log"),
        ssh_identity_file: keys.client_private_key,
        ssh_known_hosts: keys.known_hosts,
        guest_boot_state: 1,
        manifest_path: h.root.join("unused/manifest.json"),
        vcpus: 2,
        memory_mib: 1024,
        egress_allow: Vec::new(),
        proxy_process: None,
        proxy_log: None,
        broker_sessions: Vec::new(),
        direct_env: BTreeMap::new(),
        disclosed_credentials: Vec::new(),
        workspace_disk: None,
        workload_image: None,
    };
    fs::write(
        ctrl.join("vm-cli.json"),
        serde_json::to_vec_pretty(&meta).unwrap(),
    )
    .unwrap();

    // Without trusted-editor grant: stdout is empty, stderr explains guest-served + grant hint.
    let omitted = h.run_in(&proj, &["ssh-config", "eddemo"], &[]);
    assert_eq!(omitted.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&omitted.stdout).is_empty());
    let omitted_err = String::from_utf8_lossy(&omitted.stderr);
    assert!(omitted_err.contains("guest-served"), "{omitted_err}");
    assert!(
        omitted_err.contains("grant-trusted-editor"),
        "{omitted_err}"
    );

    let omitted_write = h.run_in(&proj, &["ssh-config", "--write", "eddemo"], &[]);
    assert_eq!(omitted_write.status.code(), Some(2));

    // After granting trusted-editor: stdout/write emits hardened Host eddemo.devcroft block.
    let grant = h.run_in(
        &proj,
        &[
            "runtime",
            "grant-trusted-editor",
            "--project",
            proj.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(grant.status.code(), Some(0));

    let emitted = h.run_in(&proj, &["ssh-config", "eddemo"], &[]);
    assert_eq!(emitted.status.code(), Some(0));
    let block = String::from_utf8_lossy(&emitted.stdout);
    assert!(block.contains("Host eddemo.devcroft"), "{block}");
    assert!(block.contains("ForwardAgent no"), "{block}");
    assert!(block.contains("ForwardX11 no"), "{block}");
    assert!(block.contains("ClearAllForwardings yes"), "{block}");
    assert!(block.contains("StrictHostKeyChecking yes"), "{block}");

    let written = h.run_in(&proj, &["ssh-config", "--write", "eddemo"], &[]);
    assert_eq!(written.status.code(), Some(0));
    let ssh_cfg = fs::read_to_string(h.home.join(".ssh/config")).unwrap();
    assert!(ssh_cfg.contains("Host eddemo.devcroft"), "{ssh_cfg}");
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn real_libkrun_vm_cli_end_to_end_smoke() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let helper_src = repo_root.join("experiments/microvm/libkrun/target/debug/devcroft-krun-spike");
    let kernel_src = repo_root.join("experiments/microvm/artifacts/kernel-landlock/Image");
    let initramfs_src = repo_root
        .join("experiments/microvm/artifacts/stage-b-protocol-backpressure/initramfs.cpio");
    let runtime_src =
        repo_root.join("experiments/microvm/artifacts/stage-b-protocol-backpressure/runtime.img");
    if !helper_src.is_file()
        || !kernel_src.is_file()
        || !initramfs_src.is_file()
        || !runtime_src.is_file()
    {
        eprintln!("skipping real_libkrun_vm_cli_end_to_end_smoke: local microVM artifacts absent");
        return;
    }

    let mut h = TempHarness::new("e2e");
    h.sandbox_to_cleanup = Some("vmcliproj".into());

    // Build trusted bundle directory outside the project root.
    let bundle_root = h.root.join("trusted-bundle");
    let ver_dir = bundle_root.join("2026.11.1");
    fs::create_dir_all(&ver_dir).unwrap();
    std::os::unix::fs::symlink(&helper_src, ver_dir.join("devcroft-vm-helper")).unwrap();
    std::os::unix::fs::symlink(&kernel_src, ver_dir.join("Image")).unwrap();
    std::os::unix::fs::symlink(&initramfs_src, ver_dir.join("initramfs.cpio")).unwrap();
    std::os::unix::fs::symlink(&runtime_src, ver_dir.join("runtime.img")).unwrap();
    let prep_dummy = ver_dir.join("prep.img");
    let lic_dummy = ver_dir.join("LICENSES.txt");
    fs::write(&prep_dummy, b"prep-placeholder").unwrap();
    fs::write(&lic_dummy, b"Apache-2.0 GPL-2.0-only\n").unwrap();

    let mk_entry = |rel: &str, src: &Path| ArtifactEntry {
        relative_path: rel.into(),
        sha256: sha256_file_hex(src).unwrap(),
        size_bytes: fs::metadata(src).unwrap().len(),
        license_id: "Apache-2.0".into(),
        source_url: "https://releases.devcroft.invalid/local".into(),
    };
    let manifest = TrustedArtifactManifestV1 {
        schema: CONTRACT_SCHEMA_V1,
        manifest_version: "2026.11.1".into(),
        min_compatible_controller: "0.0.1".into(),
        max_compatible_controller: "9.9.9".into(),
        publisher_key_id: "local-dev".into(),
        signature_ed25519: "a".repeat(128),
        target: GuestTarget::LinuxArm64,
        artifacts: TrustedArtifactsV1 {
            helper: mk_entry("devcroft-vm-helper", &helper_src),
            kernel: mk_entry("Image", &kernel_src),
            initramfs: mk_entry("initramfs.cpio", &initramfs_src),
            runtime_image: mk_entry("runtime.img", &runtime_src),
            preparation_image: mk_entry("prep.img", &prep_dummy),
            license_inventory: mk_entry("LICENSES.txt", &lic_dummy),
            workload: None,
            data_seed: None,
        },
    };
    let manifest_path = bundle_root.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let proj = h.root.join("vmcliproj");
    write_project_manifest(&proj, "vmcliproj", "flox");

    // 1. Boot opt-in VM sandbox via `devcroft up --vm`.
    let up = h.run_in(
        &proj,
        &[
            "up",
            "--vm",
            "--vm-manifest",
            manifest_path.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(
        up.status.code(),
        Some(0),
        "up --vm failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&up.stdout),
        String::from_utf8_lossy(&up.stderr)
    );
    let up_out = String::from_utf8_lossy(&up.stdout);
    assert!(up_out.contains("vm (libkrun / linux-arm64)"), "{up_out}");

    // Wait-free subsequent `status` check.
    let status = h.run_in(&proj, &["status"], &[]);
    assert_eq!(status.status.code(), Some(0));
    let st_out = String::from_utf8_lossy(&status.stdout);
    assert!(
        st_out.contains("runtime: vm (libkrun / linux-arm64)"),
        "{st_out}"
    );
    assert!(st_out.contains("generation: 1"), "{st_out}");
    assert!(st_out.contains("helper: healthy"), "{st_out}");
    assert!(st_out.contains("transport: healthy"), "{st_out}");
    assert!(st_out.contains("keeper: healthy"), "{st_out}");

    // `ps` includes the live VM instance.
    let ps = h.run_in(&proj, &["ps"], &[]);
    assert_eq!(ps.status.code(), Some(0));
    let ps_out = String::from_utf8_lossy(&ps.stdout);
    assert!(ps_out.contains("vmcliproj\t[vm]\tvm healthy"), "{ps_out}");

    // Plain `devcroft up` (without `--vm`) refuses switching existing VM instance.
    let native_up = h.run_in(&proj, &["up"], &[]);
    assert_eq!(native_up.status.code(), Some(2));

    // `exec` runs inside Linux guest over vsock keeper and forwards exit codes verbatim.
    let exec_ok = h.run_in(
        &proj,
        &["exec", "--", "/bin/sh", "-c", "uname -s; echo hello-vm"],
        &[],
    );
    assert_eq!(
        exec_ok.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&exec_ok.stderr)
    );
    let exec_out = String::from_utf8_lossy(&exec_ok.stdout);
    assert!(exec_out.contains("Linux"), "{exec_out}");
    assert!(exec_out.contains("hello-vm"), "{exec_out}");

    let exec_nz = h.run_in(&proj, &["exec", "--", "/bin/sh", "-c", "exit 7"], &[]);
    assert_eq!(exec_nz.status.code(), Some(7));

    // `logs` exposes helper/guest log output.
    let logs = h.run_in(&proj, &["logs", "--tail", "20"], &[]);
    assert_eq!(logs.status.code(), Some(0));
    assert!(!String::from_utf8_lossy(&logs.stdout).trim().is_empty());

    // `down` performs orderly shutdown and preserves private disk.
    let down = h.run_in(&proj, &["down"], &[]);
    assert_eq!(
        down.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&down.stderr)
    );
    let down_out = String::from_utf8_lossy(&down.stdout);
    assert!(down_out.contains("orderly"), "{down_out}");
    assert!(down_out.contains("private disk retained"), "{down_out}");

    // Export retained private disk with SHA-256 verification.
    let exported = h.root.join("exported-state.img");
    let exp = h.run_in(
        &proj,
        &[
            "runtime",
            "export-disk",
            "vmcliproj",
            "--out",
            exported.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(
        exp.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&exp.stderr)
    );
    assert_eq!(fs::metadata(&exported).unwrap().len(), 1024 * 1024);

    // Cold restart increments generation while reusing private disk.
    let up2 = h.run_in(
        &proj,
        &[
            "up",
            "--vm",
            "--vm-manifest",
            manifest_path.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(
        up2.status.code(),
        Some(0),
        "cold restart failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&up2.stdout),
        String::from_utf8_lossy(&up2.stderr)
    );
    let st2 = h.run_in(&proj, &["status", "vmcliproj"], &[]);
    assert_eq!(st2.status.code(), Some(0));
    let st2_out = String::from_utf8_lossy(&st2.stdout);
    assert!(st2_out.contains("generation: 2"), "{st2_out}");

    let exec2 = h.run_in(
        &proj,
        &["exec", "--", "/bin/sh", "-c", "echo restarted"],
        &[],
    );
    assert_eq!(exec2.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&exec2.stdout).contains("restarted"));

    // Clean shutdown + `rm --yes` deletes all owned state.
    let rm = h.run_in(&proj, &["rm", "--yes"], &[]);
    assert_eq!(
        rm.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&rm.stderr)
    );
    assert!(!h.home.join(".local/share/devcroft/vmcliproj").exists());
    h.sandbox_to_cleanup = None;
}

#[test]
fn up_vm_refuses_unrestricted_network_default_allow() {
    let h = TempHarness::new("net-allow-refuse");
    let proj = h.root.join("vmopenproj");
    fs::create_dir_all(&proj).unwrap();
    fs::write(
        proj.join("devcroft.toml"),
        "[sandbox]\nname = \"vmopenproj\"\n\n[env]\nprovider = \"flox\"\n\n[network]\ndefault = \"allow\"\n",
    )
    .unwrap();

    let out = h.run_in(&proj, &["up", "--vm"], &[]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("network.default = \"allow\""),
        "expected network.default refusal message, got: {stderr}"
    );
}

#[test]
fn real_libkrun_vm_egress_proxy_and_brokered_credentials_e2e() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let helper_src = repo_root.join("experiments/microvm/libkrun/target/debug/devcroft-krun-spike");
    let kernel_src = repo_root.join("experiments/microvm/artifacts/kernel-landlock/Image");
    let initramfs_src =
        repo_root.join("experiments/microvm/artifacts/stage-b-egress-proxy/initramfs.cpio");
    let runtime_src =
        repo_root.join("experiments/microvm/artifacts/stage-b-egress-proxy/runtime.img");
    if !helper_src.is_file()
        || !kernel_src.is_file()
        || !initramfs_src.is_file()
        || !runtime_src.is_file()
    {
        eprintln!(
            "skipping real_libkrun_vm_egress_proxy_and_brokered_credentials_e2e: local stage-b-egress-proxy artifacts absent"
        );
        return;
    }

    let mut h = TempHarness::new("egress-e2e");
    h.sandbox_to_cleanup = Some("vmegressproj".into());

    let bundle_root = h.root.join("trusted-bundle");
    let ver_dir = bundle_root.join("2026.11.1");
    fs::create_dir_all(&ver_dir).unwrap();
    std::os::unix::fs::symlink(&helper_src, ver_dir.join("devcroft-vm-helper")).unwrap();
    std::os::unix::fs::symlink(&kernel_src, ver_dir.join("Image")).unwrap();
    std::os::unix::fs::symlink(&initramfs_src, ver_dir.join("initramfs.cpio")).unwrap();
    std::os::unix::fs::symlink(&runtime_src, ver_dir.join("runtime.img")).unwrap();
    let prep_dummy = ver_dir.join("prep.img");
    let lic_dummy = ver_dir.join("LICENSES.txt");
    fs::write(&prep_dummy, b"prep-placeholder").unwrap();
    fs::write(&lic_dummy, b"Apache-2.0 GPL-2.0-only\n").unwrap();

    let mk_entry = |rel: &str, src: &Path| ArtifactEntry {
        relative_path: rel.into(),
        sha256: sha256_file_hex(src).unwrap(),
        size_bytes: fs::metadata(src).unwrap().len(),
        license_id: "Apache-2.0".into(),
        source_url: "https://releases.devcroft.invalid/local".into(),
    };
    let manifest = TrustedArtifactManifestV1 {
        schema: CONTRACT_SCHEMA_V1,
        manifest_version: "2026.11.1".into(),
        min_compatible_controller: "0.0.1".into(),
        max_compatible_controller: "9.9.9".into(),
        publisher_key_id: "local-dev".into(),
        signature_ed25519: "a".repeat(128),
        target: GuestTarget::LinuxArm64,
        artifacts: TrustedArtifactsV1 {
            helper: mk_entry("devcroft-vm-helper", &helper_src),
            kernel: mk_entry("Image", &kernel_src),
            initramfs: mk_entry("initramfs.cpio", &initramfs_src),
            runtime_image: mk_entry("runtime.img", &runtime_src),
            preparation_image: mk_entry("prep.img", &prep_dummy),
            license_inventory: mk_entry("LICENSES.txt", &lic_dummy),
            workload: None,
            data_seed: None,
        },
    };
    let manifest_path = bundle_root.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let proj = h.root.join("vmegressproj");
    fs::create_dir_all(&proj).unwrap();
    fs::write(
        proj.join("devcroft.toml"),
        "[sandbox]\nname = \"vmegressproj\"\n\n[env]\nprovider = \"flox\"\n\n[env.vars]\nAPP_MODE = \"vm-e2e\"\n\n[network]\ndefault = \"deny\"\nallow = [\"api.openai.com\", \"api.anthropic.com\"]\n",
    )
    .unwrap();

    let real_secret = "sk-ant-real-host-secret-9999";
    let user_cfg_dir = h.xdg_config.join("devcroft");
    fs::create_dir_all(&user_cfg_dir).unwrap();
    fs::write(
        user_cfg_dir.join("config.toml"),
        "[[credentials.broker]]\nhost = \"api.anthropic.com\"\nheader = \"x-api-key\"\nenv = \"ANTHROPIC_API_KEY\"\nendpoint_env = \"ANTHROPIC_BASE_URL\"\nplaceholder = \"sk-ant-proxy-{}\"\n",
    )
    .unwrap();

    let up = h.run_in(
        &proj,
        &[
            "up",
            "--vm",
            "--vm-manifest",
            manifest_path.to_str().unwrap(),
        ],
        &[("ANTHROPIC_API_KEY", real_secret)],
    );
    assert_eq!(
        up.status.code(),
        Some(0),
        "up --vm failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&up.stdout),
        String::from_utf8_lossy(&up.stderr)
    );
    let up_out = String::from_utf8_lossy(&up.stdout);
    assert!(
        up_out.contains("egress-proxy: host relay active"),
        "{up_out}"
    );
    assert!(
        up_out.contains("broker: ANTHROPIC_API_KEY -> http://127.0.0.1:18080/broker0"),
        "{up_out}"
    );

    // `status` reports healthy egress-proxy and broker line.
    let status = h.run_in(&proj, &["status"], &[]);
    assert_eq!(status.status.code(), Some(0));
    let st_out = String::from_utf8_lossy(&status.stdout);
    assert!(st_out.contains("egress-proxy: healthy"), "{st_out}");
    assert!(
        st_out.contains("broker: ANTHROPIC_API_KEY -> http://127.0.0.1:18080/broker0"),
        "{st_out}"
    );

    // Zero-secret invariant: neither vm controller files nor guest env hold `real_secret`.
    let ctrl_root = h.home.join(".local/share/devcroft/vmegressproj");
    let cli_meta_json: serde_json::Value = serde_json::from_slice(
        &fs::read(ctrl_root.join("vm-ctrl/vm-cli.json")).expect("read vm-cli.json"),
    )
    .expect("parse vm-cli.json");
    let proxy_pid = cli_meta_json["proxy_process"]["pid"]
        .as_i64()
        .expect("proxy_process.pid") as libc::pid_t;
    for entry in fs::read_dir(ctrl_root.join("vm-ctrl")).unwrap() {
        let p = entry.unwrap().path();
        if p.is_file()
            && let Ok(bytes) = fs::read(&p)
        {
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                !text.contains(real_secret),
                "controller file {} leaked real broker secret!",
                p.display()
            );
        }
    }

    let env_probe = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "printf 'HTTPS_PROXY=%s\\nNO_PROXY=%s\\nAPP_MODE=%s\\nANTHROPIC_BASE_URL=%s\\nANTHROPIC_API_KEY=%s\\n' \"$HTTPS_PROXY\" \"$NO_PROXY\" \"$APP_MODE\" \"$ANTHROPIC_BASE_URL\" \"$ANTHROPIC_API_KEY\"",
        ],
        &[],
    );
    assert_eq!(
        env_probe.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&env_probe.stderr)
    );
    let env_out = String::from_utf8_lossy(&env_probe.stdout);
    assert!(
        env_out.contains("HTTPS_PROXY=http://127.0.0.1:18080"),
        "{env_out}"
    );
    assert!(
        env_out.contains("NO_PROXY=localhost,127.0.0.1,::1"),
        "{env_out}"
    );
    assert!(env_out.contains("APP_MODE=vm-e2e"), "{env_out}");
    assert!(
        env_out.contains("ANTHROPIC_BASE_URL=http://127.0.0.1:18080/broker0"),
        "{env_out}"
    );
    assert!(
        env_out.contains("ANTHROPIC_API_KEY=sk-ant-proxy-"),
        "{env_out}"
    );
    assert!(
        !env_out.contains(real_secret),
        "guest env leaked real secret: {env_out}"
    );

    // Denied host via proxy (`HTTP_PROXY=http://127.0.0.1:18080`) -> 403.
    let denied_host = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "/bin/busybox wget -Y on -T 5 -S -O - http://example.com/ 2>&1 || true",
        ],
        &[],
    );
    assert_eq!(
        denied_host.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&denied_host.stdout),
        String::from_utf8_lossy(&denied_host.stderr)
    );
    let denied_out = String::from_utf8_lossy(&denied_host.stdout);
    assert!(
        denied_out.contains("403"),
        "expected 403 for example.com, got: {denied_out}"
    );

    // Brokered request with wrong placeholder -> refused before reaching upstream.
    let wrong_broker = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "/bin/busybox wget -Y off -T 5 -S -O - --header=\"x-api-key: sk-ant-wrong\" \"$ANTHROPIC_BASE_URL/v1/models\" 2>&1 || true",
        ],
        &[],
    );
    assert_eq!(
        wrong_broker.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&wrong_broker.stdout),
        String::from_utf8_lossy(&wrong_broker.stderr)
    );
    let wrong_broker_out = String::from_utf8_lossy(&wrong_broker.stdout).to_ascii_lowercase();
    assert!(
        wrong_broker_out.contains("401") || wrong_broker_out.contains("403"),
        "expected 401/403 for wrong broker placeholder, got: {wrong_broker_out}"
    );
    assert!(
        !wrong_broker_out.contains("anthropic-") && !wrong_broker_out.contains("cf-ray"),
        "wrong placeholder unexpectedly reached upstream Anthropic API: {wrong_broker_out}"
    );

    // Brokered request with valid placeholder -> host proxy injects real host key
    // and completes live HTTPS call to https://api.anthropic.com/v1/models.
    let valid_broker = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "/bin/busybox wget -Y off -T 10 -S -O - --header=\"x-api-key: $ANTHROPIC_API_KEY\" \"$ANTHROPIC_BASE_URL/v1/models\" 2>&1 || true",
        ],
        &[],
    );
    assert_eq!(
        valid_broker.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&valid_broker.stdout),
        String::from_utf8_lossy(&valid_broker.stderr)
    );
    let valid_broker_raw = String::from_utf8_lossy(&valid_broker.stdout);
    let valid_broker_lower = valid_broker_raw.to_ascii_lowercase();
    assert!(
        !valid_broker_raw.contains(real_secret),
        "response leaked real host secret: {valid_broker_raw}"
    );
    assert!(
        !valid_broker_lower.contains("403"),
        "brokered request with valid placeholder was refused locally: {valid_broker_raw}"
    );
    assert!(
        valid_broker_lower.contains("http/1.1 401 unauthorized")
            || valid_broker_lower.contains("http/1.1 200"),
        "expected live upstream Anthropic HTTPS response (401 for synthetic key or 200), got: {valid_broker_raw}"
    );

    // Allowlisted non-brokered LLM host via proxy -> reaches api.openai.com.
    let allow_http = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "/bin/busybox wget -Y on -T 10 -S -O - http://api.openai.com/ 2>&1 || true",
        ],
        &[],
    );
    assert_eq!(
        allow_http.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&allow_http.stdout),
        String::from_utf8_lossy(&allow_http.stderr)
    );
    let allow_out = String::from_utf8_lossy(&allow_http.stdout);
    let allow_lower = allow_out.to_ascii_lowercase();
    assert!(
        allow_lower.contains("http/1.1") && !allow_lower.contains("devcroft: egress denied"),
        "expected live upstream HTTP response from api.openai.com, got: {allow_out}"
    );

    // Direct outbound bypass attempt fails (no guest NIC + Landlock blocks non-proxy ports).
    let direct_bypass = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "/bin/busybox wget -Y off -T 2 -O - http://1.1.1.1/",
        ],
        &[],
    );
    assert_ne!(
        direct_bypass.status.code(),
        Some(0),
        "direct bypass 1.1.1.1 unexpectedly succeeded!"
    );

    // `logs` surfaces both helper logs and host egress proxy logs without secrets.
    let logs = h.run_in(&proj, &["logs"], &[]);
    assert_eq!(logs.status.code(), Some(0));
    let logs_out = String::from_utf8_lossy(&logs.stdout);
    assert!(logs_out.contains("--- egress proxy ---"), "{logs_out}");
    assert!(logs_out.contains("example.com"), "{logs_out}");
    assert!(logs_out.contains("api.openai.com"), "{logs_out}");
    assert!(!logs_out.contains(real_secret), "logs leaked real secret!");

    // `down` stops both VM helper and host egress proxy cleanly.
    let down = h.run_in(&proj, &["down"], &[]);
    assert_eq!(
        down.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&down.stderr)
    );
    // Give the kernel a brief moment to reap the terminated proxy process if needed.
    std::thread::sleep(std::time::Duration::from_millis(100));
    let kill_rc = unsafe { libc::kill(proxy_pid, 0) };
    assert_eq!(
        kill_rc, -1,
        "egress proxy pid {proxy_pid} remained alive after down"
    );

    let rm = h.run_in(&proj, &["rm", "--yes"], &[]);
    assert_eq!(
        rm.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&rm.stderr)
    );
    assert!(!h.home.join(".local/share/devcroft/vmegressproj").exists());
    h.sandbox_to_cleanup = None;
}

#[test]
fn vm_policy_render_and_doctor_surface_vm_authority_without_secrets() {
    let h = TempHarness::new("vm-policy-doctor");
    let proj = h.root.join("vmpolproj");
    fs::create_dir_all(&proj).unwrap();
    fs::write(
        proj.join("devcroft.toml"),
        "[sandbox]\nname = \"vmpolproj\"\n\n[env]\nprovider = \"flox\"\n\n[network]\ndefault = \"deny\"\nallow = [\"api.anthropic.com\"]\n",
    )
    .unwrap();

    let real_secret = "sk-ant-policy-secret-never-render";
    let user_cfg_dir = h.xdg_config.join("devcroft");
    fs::create_dir_all(&user_cfg_dir).unwrap();
    fs::write(
        user_cfg_dir.join("config.toml"),
        "[[credentials.broker]]\nhost = \"api.anthropic.com\"\nheader = \"x-api-key\"\nenv = \"ANTHROPIC_API_KEY\"\nendpoint_env = \"ANTHROPIC_BASE_URL\"\nplaceholder = \"sk-ant-proxy-{}\"\n",
    )
    .unwrap();

    let pol = h.run_in(
        &proj,
        &["policy", "--render", "--vm"],
        &[("ANTHROPIC_API_KEY", real_secret)],
    );
    assert_eq!(
        pol.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&pol.stderr)
    );
    let pol_out = String::from_utf8_lossy(&pol.stdout);
    assert!(pol_out.contains("vm-plan:"), "{pol_out}");
    assert!(pol_out.contains("nic=false tsi=false"), "{pol_out}");
    assert!(pol_out.contains("api.anthropic.com"), "{pol_out}");
    assert!(
        pol_out.contains("broker: ANTHROPIC_API_KEY -> http://127.0.0.1:18080/broker0"),
        "{pol_out}"
    );
    assert!(
        !pol_out.contains(real_secret),
        "policy render leaked real host secret: {pol_out}"
    );

    // Refuses network.default = "allow" in VM mode.
    fs::write(
        proj.join("devcroft.toml"),
        "[sandbox]\nname = \"vmpolproj\"\n\n[env]\nprovider = \"flox\"\n\n[network]\ndefault = \"allow\"\n",
    )
    .unwrap();
    let bad_pol = h.run_in(&proj, &["policy", "--render", "--vm"], &[]);
    assert_eq!(bad_pol.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad_pol.stderr).contains("network.default = \"allow\""));

    // `doctor` surfaces opt-in VM qualification and artifact status without failing.
    let doc = h.run_in(&proj, &["doctor"], &[]);
    let doc_out = String::from_utf8_lossy(&doc.stdout);
    assert!(doc_out.contains("[INFO] vm-runtime:"), "{doc_out}");
    assert!(doc_out.contains("[INFO] vm-artifacts:"), "{doc_out}");
}

#[test]
fn real_libkrun_vm_install_local_sync_and_ssh_e2e() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let helper_src = repo_root.join("experiments/microvm/libkrun/target/debug/devcroft-krun-spike");
    let kernel_src = repo_root.join("experiments/microvm/artifacts/kernel-landlock/Image");
    let initramfs_src =
        repo_root.join("experiments/microvm/artifacts/stage-b-egress-proxy/initramfs.cpio");
    let runtime_src =
        repo_root.join("experiments/microvm/artifacts/stage-b-egress-proxy/runtime.img");
    if !helper_src.is_file()
        || !kernel_src.is_file()
        || !initramfs_src.is_file()
        || !runtime_src.is_file()
    {
        eprintln!(
            "skipping real_libkrun_vm_install_local_sync_and_ssh_e2e: local stage-b-egress-proxy artifacts absent"
        );
        return;
    }

    let mut h = TempHarness::new("sync-ssh-e2e");
    h.sandbox_to_cleanup = Some("vmsyncproj".into());

    let proj = h.root.join("vmsyncproj");
    write_project_manifest(&proj, "vmsyncproj", "flox");
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::write(proj.join("src/hello.txt"), b"hello-from-host\n").unwrap();
    fs::create_dir_all(proj.join(".git")).unwrap();
    fs::write(proj.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    std::os::unix::fs::symlink("/etc/hosts", proj.join("escape-link")).unwrap();

    // 1. Install local bundle into default user store (`~/.local/share/devcroft/vm-artifacts`).
    let inst = h.run_in(
        &proj,
        &[
            "runtime",
            "install-local",
            "--helper",
            helper_src.to_str().unwrap(),
            "--kernel",
            kernel_src.to_str().unwrap(),
            "--initramfs",
            initramfs_src.to_str().unwrap(),
            "--runtime",
            runtime_src.to_str().unwrap(),
            "--version",
            "local-e2e-1",
        ],
        &[],
    );
    assert_eq!(
        inst.status.code(),
        Some(0),
        "install-local failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&inst.stdout),
        String::from_utf8_lossy(&inst.stderr)
    );

    let rstat = h.run_in(&proj, &["runtime", "status"], &[]);
    assert_eq!(rstat.status.code(), Some(0));
    let rstat_out = String::from_utf8_lossy(&rstat.stdout);
    assert!(
        rstat_out.contains("vm-artifacts: installed (local-e2e-1,"),
        "{rstat_out}"
    );

    // 2. Boot opt-in VM WITHOUT `--vm-manifest` and sync workspace in one command (`--sync`).
    let up = h.run_in(&proj, &["up", "--vm", "--sync"], &[]);
    assert_eq!(
        up.status.code(),
        Some(0),
        "up --vm --sync failed without --vm-manifest\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&up.stdout),
        String::from_utf8_lossy(&up.stderr)
    );
    let up_out = String::from_utf8_lossy(&up.stdout);
    assert!(up_out.contains("synced 2 file(s)"), "{up_out}");

    // 3. Re-sync is idempotent and skipped symlinks / `.git` stay absent in `/workspace`.
    let check_sync = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "cat /workspace/src/hello.txt; test ! -e /workspace/escape-link; test ! -e /workspace/.git/HEAD; echo SYNC_OK",
        ],
        &[],
    );
    assert_eq!(
        check_sync.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&check_sync.stderr)
    );
    let check_out = String::from_utf8_lossy(&check_sync.stdout);
    assert!(check_out.contains("hello-from-host"), "{check_out}");
    assert!(check_out.contains("SYNC_OK"), "{check_out}");

    // 4. Real OpenSSH exec over per-instance keys + `devcroft proxy --no-up` -> vsock 1026.
    let ssh_exec = h.run_in(
        &proj,
        &[
            "ssh",
            "vmsyncproj",
            "--",
            "sh",
            "-c",
            "uname -s; cat /workspace/src/hello.txt",
        ],
        &[],
    );
    assert_eq!(
        ssh_exec.status.code(),
        Some(0),
        "devcroft ssh failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&ssh_exec.stdout),
        String::from_utf8_lossy(&ssh_exec.stderr)
    );
    let ssh_out = String::from_utf8_lossy(&ssh_exec.stdout);
    assert!(ssh_out.contains("Linux"), "{ssh_out}");
    assert!(ssh_out.contains("hello-from-host"), "{ssh_out}");

    // 5. Real SSH local port-forward (`-N -L`) to the guest's allowed loopback service (:8080).
    let free_port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let forward_spec = format!("127.0.0.1:{free_port}:127.0.0.1:8080");
    let mut tunnel = Command::new(devcroft_bin())
        .args(["ssh", "-N", "-L", &forward_spec, "vmsyncproj"])
        .current_dir(&proj)
        .env("HOME", &h.home)
        .env("XDG_CONFIG_HOME", &h.xdg_config)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn devcroft ssh -N -L tunnel");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut response = String::new();
    while Instant::now() < deadline {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", free_port)) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ =
                stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf);
            let text = String::from_utf8_lossy(&buf).into_owned();
            if !text.is_empty() {
                response = text;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = tunnel.kill();
    let _ = tunnel.wait();
    assert!(
        response.contains("200 OK") && response.contains("instance="),
        "expected forwarded HTTP fixture response from guest :8080, got: {response}"
    );

    // 6. Guest edits & safe `sync-out`: write file + hostile symlink in guest, verify
    //    `sync-out --out` transfers only the regular file and skips sentinels/symlinks,
    //    `sync-out` without `--out`/`--yes` refuses exit 2, and `--yes` updates host project.
    let mutate = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "printf 'edited-by-agent\\n' > /workspace/src/from_guest.txt && ln -s /etc/passwd /workspace/evil_link",
        ],
        &[],
    );
    assert_eq!(mutate.status.code(), Some(0));

    let export_dir = h.root.join("exported-workspace");
    let sync_out_dir = h.run_in(
        &proj,
        &[
            "runtime",
            "sync-out",
            "vmsyncproj",
            "--out",
            export_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(
        sync_out_dir.status.code(),
        Some(0),
        "sync-out --out failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&sync_out_dir.stdout),
        String::from_utf8_lossy(&sync_out_dir.stderr)
    );
    assert_eq!(
        fs::read_to_string(export_dir.join("src/from_guest.txt")).unwrap(),
        "edited-by-agent\n"
    );
    assert!(!export_dir.join("evil_link").exists());
    assert!(!export_dir.join("instance-id").exists());
    assert!(!export_dir.join("allowed").exists());

    let sync_out_no_yes = h.run_in(&proj, &["runtime", "sync-out", "vmsyncproj"], &[]);
    assert_eq!(sync_out_no_yes.status.code(), Some(2));

    let sync_out_yes = h.run_in(&proj, &["runtime", "sync-out", "vmsyncproj", "--yes"], &[]);
    assert_eq!(
        sync_out_yes.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&sync_out_yes.stderr)
    );
    assert_eq!(
        fs::read_to_string(proj.join("src/from_guest.txt")).unwrap(),
        "edited-by-agent\n"
    );
    assert!(!proj.join("evil_link").exists());

    let rm = h.run_in(&proj, &["rm", "--yes"], &[]);
    assert_eq!(rm.status.code(), Some(0));
    assert!(!h.home.join(".local/share/devcroft/vmsyncproj").exists());
    h.sandbox_to_cleanup = None;
}

#[test]
fn vm_why_explains_egress_broker_env_and_no_host_mounts() {
    let h = TempHarness::new("vm-why");
    let proj = h.root.join("vmwhyproj");
    fs::create_dir_all(&proj).unwrap();
    fs::write(
        proj.join("devcroft.toml"),
        "[sandbox]\nname = \"vmwhyproj\"\n\n[env]\nprovider = \"flox\"\n\n[env.vars]\nPUBLIC_VAR = \"dev\"\n\n[network]\ndefault = \"deny\"\nallow = [\"api.anthropic.com\"]\n",
    )
    .unwrap();

    let real_secret = "sk-ant-why-secret-value";
    let user_cfg_dir = h.xdg_config.join("devcroft");
    fs::create_dir_all(&user_cfg_dir).unwrap();
    fs::write(
        user_cfg_dir.join("config.toml"),
        "[[credentials.broker]]\nhost = \"api.anthropic.com\"\nheader = \"x-api-key\"\nenv = \"ANTHROPIC_API_KEY\"\nendpoint_env = \"ANTHROPIC_BASE_URL\"\n",
    )
    .unwrap();

    let allowed_host = h.run_in(
        &proj,
        &["why", "--vm", "--host", "api.anthropic.com"],
        &[("ANTHROPIC_API_KEY", real_secret)],
    );
    assert_eq!(allowed_host.status.code(), Some(0));
    let allowed_out = String::from_utf8_lossy(&allowed_host.stdout);
    assert!(allowed_out.contains("ALLOWED"), "{allowed_out}");
    assert!(allowed_out.contains("broker0"), "{allowed_out}");

    let denied_host = h.run_in(&proj, &["why", "--vm", "--host", "example.com"], &[]);
    assert_eq!(denied_host.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&denied_host.stdout).contains("DENIED"));

    let private_host = h.run_in(&proj, &["why", "--vm", "--host", "127.0.0.1"], &[]);
    assert_eq!(private_host.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&private_host.stdout).contains("DENIED"));

    let env_broker = h.run_in(
        &proj,
        &["why", "--vm", "--env", "ANTHROPIC_API_KEY"],
        &[("ANTHROPIC_API_KEY", real_secret)],
    );
    assert_eq!(env_broker.status.code(), Some(0));
    let env_out = String::from_utf8_lossy(&env_broker.stdout);
    assert!(env_out.contains("ALLOWED"), "{env_out}");
    assert!(env_out.contains("placeholder"), "{env_out}");
    assert!(!env_out.contains(real_secret), "leaked secret: {env_out}");

    let env_denied = h.run_in(&proj, &["why", "--vm", "--env", "UNDECLARED_SECRET"], &[]);
    assert_eq!(env_denied.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&env_denied.stdout).contains("DENIED"));

    let path_denied = h.run_in(
        &proj,
        &["why", "--vm", "--path", "/etc/passwd", "--op", "read"],
        &[],
    );
    assert_eq!(path_denied.status.code(), Some(0));
    let path_out = String::from_utf8_lossy(&path_denied.stdout);
    assert!(path_out.contains("DENIED"), "{path_out}");
    assert!(path_out.contains("virtio-fs"), "{path_out}");
}

#[test]
fn real_libkrun_vm_workload_toolchain_and_persistent_workspace_e2e() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let helper_src = repo_root.join("experiments/microvm/libkrun/target/debug/devcroft-krun-spike");
    let kernel_src = repo_root.join("experiments/microvm/artifacts/kernel-landlock/Image");
    let initramfs_src =
        repo_root.join("experiments/microvm/artifacts/stage-b-workload-egress/initramfs.cpio");
    let runtime_src =
        repo_root.join("experiments/microvm/artifacts/stage-b-workload-egress/runtime.img");
    let workload_src = repo_root.join("experiments/microvm/artifacts/workload/toolchain.img");
    let data_seed_src = repo_root.join("experiments/microvm/artifacts/workload/data.seed.img");
    if !helper_src.is_file()
        || !kernel_src.is_file()
        || !initramfs_src.is_file()
        || !runtime_src.is_file()
        || !workload_src.is_file()
        || !data_seed_src.is_file()
    {
        eprintln!(
            "skipping real_libkrun_vm_workload_toolchain_and_persistent_workspace_e2e: workload artifacts absent"
        );
        return;
    }

    let mut h = TempHarness::new("workload-e2e");
    h.sandbox_to_cleanup = Some("vmworkproj".into());

    let proj = h.root.join("vmworkproj");
    write_project_manifest(&proj, "vmworkproj", "flox");
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::write(
        proj.join("src/hello.c"),
        b"#include <stdio.h>\nint main(void) { puts(\"hello-from-vm-gcc\"); return 0; }\n",
    )
    .unwrap();

    // 1. Install workload-enabled bundle (`toolchain.img` + `data.seed.img`).
    let inst = h.run_in(
        &proj,
        &[
            "runtime",
            "install-local",
            "--helper",
            helper_src.to_str().unwrap(),
            "--kernel",
            kernel_src.to_str().unwrap(),
            "--initramfs",
            initramfs_src.to_str().unwrap(),
            "--runtime",
            runtime_src.to_str().unwrap(),
            "--workload",
            workload_src.to_str().unwrap(),
            "--data-seed",
            data_seed_src.to_str().unwrap(),
            "--version",
            "workload-e2e-1",
        ],
        &[],
    );
    assert_eq!(
        inst.status.code(),
        Some(0),
        "install-local +workload failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&inst.stdout),
        String::from_utf8_lossy(&inst.stderr)
    );
    assert!(
        String::from_utf8_lossy(&inst.stdout).contains("+workload"),
        "{}",
        String::from_utf8_lossy(&inst.stdout)
    );

    // 2. Boot VM + sync workspace in one command (`up --vm --sync`).
    let up = h.run_in(&proj, &["up", "--vm", "--sync"], &[]);
    assert_eq!(
        up.status.code(),
        Some(0),
        "up --vm --sync failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&up.stdout),
        String::from_utf8_lossy(&up.stderr)
    );

    let st = h.run_in(&proj, &["status", "vmworkproj"], &[]);
    assert_eq!(st.status.code(), Some(0));
    let st_out = String::from_utf8_lossy(&st.stdout);
    assert!(st_out.contains("workspace-disk:"), "{st_out}");
    assert!(st_out.contains("workload-image:"), "{st_out}");

    // 3. Compile and execute C code inside the Landlock-confined guest using mounted `/toolchain` gcc.
    let build_and_run = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "PATH=/usr/bin:/bin TMPDIR=/tmp /usr/bin/gcc -O2 /workspace/src/hello.c -o /workspace/hello_bin && /workspace/hello_bin",
        ],
        &[],
    );
    assert_eq!(
        build_and_run.status.code(),
        Some(0),
        "gcc compile/run failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build_and_run.stdout),
        String::from_utf8_lossy(&build_and_run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&build_and_run.stdout).contains("hello-from-vm-gcc"),
        "{}",
        String::from_utf8_lossy(&build_and_run.stdout)
    );

    // 4. Cold shutdown & restart WITHOUT `--sync` -> persistent `/workspace` ext4 disk survives!
    let down = h.run_in(&proj, &["down", "vmworkproj"], &[]);
    assert_eq!(down.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&down.stdout).contains("orderly"));

    let up2 = h.run_in(&proj, &["up", "--vm"], &[]);
    assert_eq!(
        up2.status.code(),
        Some(0),
        "cold restart failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&up2.stdout),
        String::from_utf8_lossy(&up2.stderr)
    );
    let st2 = h.run_in(&proj, &["status", "vmworkproj"], &[]);
    assert_eq!(st2.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&st2.stdout).contains("generation: 2"));

    let rerun = h.run_in(&proj, &["exec", "--", "/workspace/hello_bin"], &[]);
    assert_eq!(
        rerun.status.code(),
        Some(0),
        "persisted binary run failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&rerun.stdout),
        String::from_utf8_lossy(&rerun.stderr)
    );
    assert!(
        String::from_utf8_lossy(&rerun.stdout).contains("hello-from-vm-gcc"),
        "{}",
        String::from_utf8_lossy(&rerun.stdout)
    );

    // 5. Safe `sync-out` exports compiled artifact and source out of persistent `/workspace`.
    let export_dir = h.root.join("exported-work");
    let sync_out = h.run_in(
        &proj,
        &[
            "runtime",
            "sync-out",
            "vmworkproj",
            "--out",
            export_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(
        sync_out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&sync_out.stderr)
    );
    assert!(export_dir.join("src/hello.c").is_file());
    assert!(export_dir.join("hello_bin").is_file());

    let rm = h.run_in(&proj, &["rm", "--yes"], &[]);
    assert_eq!(rm.status.code(), Some(0));
    assert!(!h.home.join(".local/share/devcroft/vmworkproj").exists());
    h.sandbox_to_cleanup = None;
}

#[test]
fn real_libkrun_vm_coding_agents_tls_and_broker_e2e() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let helper_src = repo_root.join("experiments/microvm/libkrun/target/debug/devcroft-krun-spike");
    let kernel_src = repo_root.join("experiments/microvm/artifacts/kernel-landlock/Image");
    let initramfs_src =
        repo_root.join("experiments/microvm/artifacts/stage-b-nix-egress/initramfs.cpio");
    let runtime_src =
        repo_root.join("experiments/microvm/artifacts/stage-b-nix-egress/runtime.img");
    let workload_src = repo_root.join("experiments/microvm/artifacts/agents/toolchain.img");
    let data_seed_src = repo_root.join("experiments/microvm/artifacts/agents/data.seed.img");
    if !helper_src.is_file()
        || !kernel_src.is_file()
        || !initramfs_src.is_file()
        || !runtime_src.is_file()
        || !workload_src.is_file()
        || !data_seed_src.is_file()
    {
        eprintln!(
            "skipping real_libkrun_vm_coding_agents_tls_and_broker_e2e: Nix agents artifacts absent"
        );
        return;
    }

    let mut h = TempHarness::new("agents-e2e");
    h.sandbox_to_cleanup = Some("vmagentproj".into());

    let proj = h.root.join("vmagentproj");
    fs::create_dir_all(&proj).unwrap();
    fs::write(
        proj.join("devcroft.toml"),
        "[sandbox]\nname = \"vmagentproj\"\n\n[env]\nprovider = \"flox\"\n\n[env.vars]\nCLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC = \"1\"\nCLAUDE_CODE_MAX_RETRIES = \"0\"\n\n[network]\ndefault = \"deny\"\nallow = [\"api.anthropic.com\", \"api.openai.com\"]\n",
    )
    .unwrap();

    let real_secret = "sk-ant-real-host-agent-secret-4242";
    let user_cfg_dir = h.xdg_config.join("devcroft");
    fs::create_dir_all(&user_cfg_dir).unwrap();
    fs::write(
        user_cfg_dir.join("config.toml"),
        "[[credentials.broker]]\nhost = \"api.anthropic.com\"\nheader = \"x-api-key\"\nenv = \"ANTHROPIC_API_KEY\"\nendpoint_env = \"ANTHROPIC_BASE_URL\"\nplaceholder = \"sk-ant-proxy-{}\"\n",
    )
    .unwrap();

    // 1. Install Nix coding-agent bundle (`claude`, `codex`, `curl`, `ca-bundle.crt`).
    let inst = h.run_in(
        &proj,
        &[
            "runtime",
            "install-local",
            "--helper",
            helper_src.to_str().unwrap(),
            "--kernel",
            kernel_src.to_str().unwrap(),
            "--initramfs",
            initramfs_src.to_str().unwrap(),
            "--runtime",
            runtime_src.to_str().unwrap(),
            "--workload",
            workload_src.to_str().unwrap(),
            "--data-seed",
            data_seed_src.to_str().unwrap(),
            "--version",
            "agents-e2e-1",
        ],
        &[],
    );
    assert_eq!(
        inst.status.code(),
        Some(0),
        "install-local agents failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&inst.stdout),
        String::from_utf8_lossy(&inst.stderr)
    );

    // 2. Boot VM with host `nono-proxy` + zero-secret broker for Anthropic.
    let up = h.run_in(
        &proj,
        &["up", "--vm"],
        &[("ANTHROPIC_API_KEY", real_secret)],
    );
    assert_eq!(
        up.status.code(),
        Some(0),
        "up --vm agents failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&up.stdout),
        String::from_utf8_lossy(&up.stderr)
    );

    // 3. Verify guest env (`SSL_CERT_FILE`, `HTTPS_PROXY`, broker placeholder without real secret).
    let env_probe = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "printf 'SSL_CERT_FILE=%s\\nHTTPS_PROXY=%s\\nANTHROPIC_BASE_URL=%s\\nANTHROPIC_API_KEY=%s\\n' \"$SSL_CERT_FILE\" \"$HTTPS_PROXY\" \"$ANTHROPIC_BASE_URL\" \"$ANTHROPIC_API_KEY\"",
        ],
        &[],
    );
    assert_eq!(env_probe.status.code(), Some(0));
    let env_out = String::from_utf8_lossy(&env_probe.stdout);
    assert!(
        env_out.contains("SSL_CERT_FILE=/usr/etc/ssl/certs/ca-bundle.crt"),
        "{env_out}"
    );
    assert!(
        env_out.contains("HTTPS_PROXY=http://127.0.0.1:18080"),
        "{env_out}"
    );
    assert!(
        env_out.contains("ANTHROPIC_BASE_URL=http://127.0.0.1:18080/broker0"),
        "{env_out}"
    );
    assert!(
        env_out.contains("ANTHROPIC_API_KEY=sk-ant-proxy-"),
        "{env_out}"
    );
    assert!(
        !env_out.contains(real_secret),
        "real secret leaked into guest env: {env_out}"
    );

    // 4. Real end-to-end TLS over HTTPS_PROXY via `curl`:
    //    - allowlisted `https://api.openai.com/v1/models` succeeds TLS handshake -> live HTTP 401
    //    - denied `https://example.com/` refused 403 by proxy
    //    - direct bypass `https://1.1.1.1/` refused by Landlock + no guest NIC
    let tls_allow = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "curl -sS -o /dev/null -w '%{http_code}' --max-time 15 https://api.openai.com/v1/models",
        ],
        &[],
    );
    assert_eq!(
        tls_allow.status.code(),
        Some(0),
        "curl https://api.openai.com failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&tls_allow.stdout),
        String::from_utf8_lossy(&tls_allow.stderr)
    );
    let tls_code = String::from_utf8_lossy(&tls_allow.stdout);
    assert!(
        tls_code.trim() == "401" || tls_code.trim() == "200",
        "expected live OpenAI HTTPS 401/200, got: {tls_code}"
    );

    let tls_deny = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "curl -sS -i --max-time 10 https://example.com/ 2>&1 || true",
        ],
        &[],
    );
    let tls_deny_out = String::from_utf8_lossy(&tls_deny.stdout);
    assert!(
        tls_deny_out.contains("403"),
        "expected proxy 403 for https://example.com/, got: {tls_deny_out}"
    );

    let tls_bypass = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "curl --noproxy '*' --max-time 3 https://1.1.1.1/",
        ],
        &[],
    );
    assert_ne!(
        tls_bypass.status.code(),
        Some(0),
        "direct HTTPS bypass unexpectedly succeeded"
    );

    // 5. Real Claude Code (`claude --bare -p`) inside the Landlock-confined VM talking to
    //    live `https://api.anthropic.com` via `/broker0` with zero host API key in the guest.
    let claude_run = h.run_in(
        &proj,
        &[
            "exec",
            "--",
            "/bin/sh",
            "-c",
            "claude --bare -p --output-format stream-json --verbose --no-session-persistence --tools '' --model claude-sonnet-4-5 'Reply only ok.' 2>&1 || true",
        ],
        &[],
    );
    assert_eq!(claude_run.status.code(), Some(0));
    let claude_out = String::from_utf8_lossy(&claude_run.stdout);
    assert!(
        !claude_out.contains(real_secret),
        "claude output leaked real host secret: {claude_out}"
    );
    let claude_lower = claude_out.to_ascii_lowercase();
    assert!(
        claude_lower.contains("\"subtype\":\"init\"")
            || claude_lower.contains("authentication_error")
            || claude_lower.contains("invalid x-api-key")
            || claude_lower.contains("401"),
        "expected Claude Code init + live Anthropic response via broker0, got:\n{claude_out}"
    );

    // 6. Real Codex binary starts cleanly inside the Landlock-confined VM.
    let codex_ver = h.run_in(&proj, &["exec", "--", "/usr/bin/codex", "--version"], &[]);
    assert_eq!(
        codex_ver.status.code(),
        Some(0),
        "codex --version failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&codex_ver.stdout),
        String::from_utf8_lossy(&codex_ver.stderr)
    );
    assert!(
        String::from_utf8_lossy(&codex_ver.stdout).contains("codex"),
        "{}",
        String::from_utf8_lossy(&codex_ver.stdout)
    );

    // Logs must never leak the real host secret.
    let logs = h.run_in(&proj, &["logs"], &[]);
    assert_eq!(logs.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&logs.stdout).contains(real_secret),
        "logs leaked real host secret!"
    );

    let rm = h.run_in(&proj, &["rm", "--yes"], &[]);
    assert_eq!(rm.status.code(), Some(0));
    assert!(!h.home.join(".local/share/devcroft/vmagentproj").exists());
    h.sandbox_to_cleanup = None;
}
