use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

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
