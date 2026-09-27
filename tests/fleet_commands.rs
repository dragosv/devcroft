//! `devcroft fleet up | ls | stop`: the library calls behind the CLI, with
//! an injected provider, and the CLI's own error paths.
//!
//! The project deliberately sits in a subdirectory of its repository, as
//! every sample in this one does: an agent's workspace is that same
//! subdirectory inside its clone, not the clone's root.

#![cfg(target_os = "linux")]

use devcroft::fleet::commands::{self, FleetError, UpRequest};
use devcroft::fleet::supervisor::AgentState;
use devcroft::keeper::protocol::{self, Frame, SpawnRequest};
use devcroft::provider::{ProviderEntry, ProviderError, Resolution, ServiceSupport, Tier};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;

fn capable_host() -> Option<PathBuf> {
    let Some(root) = std::env::var_os("DEVCROFT_TEST_CGROUP_ROOT").map(PathBuf::from) else {
        eprintln!(
            "skipping: DEVCROFT_TEST_CGROUP_ROOT is not set (no delegated cgroup to test in)"
        );
        return None;
    };
    let own = devcroft::fleet::cgroup::own_cgroup().ok()?;
    if !own.starts_with(&root) {
        eprintln!("skipping: run `sudo cgroup-delegate enter $$` first");
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
        eprintln!("skipping: this host cannot mount a fresh /proc in a user+PID namespace");
        return None;
    }
    Some(root)
}

/// The host's `/usr` as the closure.
struct HostUsr;

impl ProviderEntry for HostUsr {
    fn resolve(&self, _: &Path) -> Result<Resolution, ProviderError> {
        Ok(Resolution {
            env: [("PATH".to_owned(), "/usr/bin".to_owned())].into(),
            unset: Vec::new(),
            read_only_grants: vec!["/usr".to_owned()],
            activation_script: None,
            services: ServiceSupport::Unsupported,
            ran_activation_hook: false,
        })
    }
    fn fingerprint(&self, _: &Path) -> Result<String, ProviderError> {
        Ok(String::new())
    }
    fn tier(&self) -> Tier {
        Tier::Closure
    }
    fn static_name(&self) -> &'static str {
        "nix"
    }
}

/// A repository with the project in `proj/`, a state dir, and a unique
/// sandbox name; everything removed on drop, running agents first.
struct Setup {
    base: PathBuf,
    name: String,
    cgroup_root: Option<PathBuf>,
}

impl Setup {
    fn new(tag: &str, cgroup_root: Option<&Path>, manifest_extra: &str, git: bool) -> Setup {
        let name = format!("fc{tag}{}", std::process::id());
        let base = std::env::temp_dir().join(format!("devcroft-fleet-cmd-{name}"));
        let _ = std::fs::remove_dir_all(&base);
        let proj = base.join("repo").join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("devcroft.toml"),
            format!(
                "[sandbox]\nname = \"{name}\"\n\n[env]\nprovider = \"nix\"\n\n\
                 [filesystem]\nread = [\"/proc\"]\n{manifest_extra}"
            ),
        )
        .unwrap();
        std::fs::write(base.join("repo").join(".gitignore"), ".devcroft/\n").unwrap();
        if git {
            let repo = base.join("repo");
            for args in [
                &["init", "--quiet"][..],
                &["add", "."],
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "commit",
                    "--quiet",
                    "-m",
                    "init",
                ],
            ] {
                assert!(
                    Command::new("git")
                        .arg("-C")
                        .arg(&repo)
                        .args(args)
                        .status()
                        .unwrap()
                        .success()
                );
            }
        }
        Setup {
            base,
            name,
            cgroup_root: cgroup_root.map(Path::to_path_buf),
        }
    }

    fn project(&self) -> PathBuf {
        self.base.join("repo").join("proj")
    }

    fn state(&self) -> PathBuf {
        self.base.join("state")
    }

    fn manifest(&self) -> devcroft::config::Manifest {
        let text = std::fs::read_to_string(self.project().join("devcroft.toml")).unwrap();
        devcroft::config::parse(&text).unwrap().0
    }

    fn up(&self, agents: usize) -> Result<Vec<commands::Started>, FleetError> {
        self.up_with(agents, devcroft::fleet::cgroup::Limits::default())
            .map(|o| o.started)
    }

    fn up_with(
        &self,
        agents: usize,
        limits: devcroft::fleet::cgroup::Limits,
    ) -> Result<commands::UpOutcome, FleetError> {
        let manifest = self.manifest();
        let keys = self.base.join("keys");
        std::fs::create_dir_all(&keys).unwrap();
        let client =
            devcroft::ssh::ensure_client_keypair(&keys.join("id"), &keys.join("id.pub")).unwrap();
        let authorized = client.public_key().to_openssh().unwrap();
        let fallback = self.base.join("no-cgroup");
        commands::up(
            &HostUsr,
            &UpRequest {
                manifest: &manifest,
                project_root: &self.project(),
                cgroup_root: self.cgroup_root.as_deref().unwrap_or(&fallback),
                agents,
                view: true,
                exe: exe(),
                authorized_key_pem: &authorized,
                state_dir: &self.state(),
                limits,
            },
        )
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        if let Some(root) = &self.cgroup_root {
            let node = root.join(format!("fleet-{}", self.name));
            if let Ok(entries) = std::fs::read_dir(&node) {
                for leaf in entries.flatten().filter(|e| e.path().is_dir()) {
                    let _ = std::fs::write(leaf.path().join("cgroup.kill"), "1");
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let _ = std::fs::remove_dir(leaf.path());
                }
            }
            let _ = std::fs::remove_dir(&node);
        }
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn exe() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_devcroft"))
}

fn session(socket: &Path, cwd: &Path, script: &str) -> (Option<i32>, String) {
    let mut stream = UnixStream::connect(socket).unwrap();
    protocol::write_frame(
        &mut stream,
        &Frame::Spawn(SpawnRequest {
            cmd: "sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: cwd.to_string_lossy().into_owned(),
            env: Default::default(),
            pty: None,
        }),
    )
    .unwrap();
    let mut out = String::new();
    loop {
        match protocol::read_frame(&mut stream).unwrap() {
            Frame::SpawnErr { message } => panic!("keeper refused to spawn: {message}"),
            Frame::Stdout(b) | Frame::Stderr(b) => out.push_str(&String::from_utf8_lossy(&b)),
            Frame::Exit(status) => return (status.code, out),
            _ => {}
        }
    }
}

#[test]
fn each_agent_works_on_its_own_clone_and_keeps_it_after_stopping() {
    let Some(root) = capable_host() else { return };
    let s = Setup::new("two", Some(&root), "", true);
    let started = s.up(2).unwrap();
    let ids: Vec<_> = started.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["a1", "a2"]);

    for a in &started {
        // The project's own directory inside the agent's clone, not the
        // clone's root.
        let clone_root = s.project().join(".devcroft/fleet").join(&a.id);
        assert_eq!(a.workspace, clone_root.join("proj"));
        assert!(a.workspace.join("devcroft.toml").is_file());

        let socket = s.state().join("agents").join(&a.id).join("control.sock");
        let (code, out) = session(&socket, &a.workspace, &format!("echo {} > mine; pwd", a.id));
        assert_eq!(code, Some(0), "{out}");
        assert_eq!(Path::new(out.trim()), a.workspace);
    }
    // What one agent writes is in its clone only.
    for a in &started {
        assert_eq!(
            std::fs::read_to_string(a.workspace.join("mine"))
                .unwrap()
                .trim(),
            a.id
        );
    }
    assert!(!s.project().join("mine").exists());

    // `ls` and `stop` find the fleet from its state alone.
    let m = s.manifest();
    let listed = commands::ls(&s.state(), &m, exe()).unwrap();
    assert!(listed.iter().all(|a| a.record.state == AgentState::Running));
    commands::stop(&s.state(), &m, exe(), "a1").unwrap();
    let listed = commands::ls(&s.state(), &m, exe()).unwrap();
    assert_eq!(listed[0].record.state, AgentState::Stopped);
    assert_eq!(listed[1].record.state, AgentState::Running);
    // Stopping an agent does not throw away its work.
    assert!(started[0].workspace.join("mine").is_file());

    let err = commands::stop(&s.state(), &m, exe(), "a9").unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    commands::stop(&s.state(), &m, exe(), "a2").unwrap();
}

#[test]
fn a_refused_manifest_leaves_no_clone_and_no_agent() {
    let Some(root) = capable_host() else { return };
    let s = Setup::new(
        "egress",
        Some(&root),
        "\n[network]\ndefault = \"allow\"\n",
        true,
    );
    let err = s.up(1).unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert!(err.to_string().contains("network.default"), "{err}");
    assert!(!s.project().join(".devcroft/fleet/a1").exists());
    assert!(!s.state().join("agents/a1").exists());
}

#[test]
fn a_project_outside_git_is_refused_before_anything_is_made() {
    let s = Setup::new("nogit", None, "", false);
    let err = s.up(1).unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert!(err.to_string().contains("git repository"), "{err}");
    assert!(!s.state().exists());
    assert!(!s.project().join(".devcroft").exists());
}

#[test]
fn ls_without_a_fleet_says_how_to_start_one() {
    let s = Setup::new("none", None, "", true);
    let err = commands::ls(&s.state(), &s.manifest(), exe()).unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert!(err.to_string().contains("fleet up"), "{err}");
}

/// The CLI's own paths that need no delegated cgroup.
#[test]
fn the_cli_names_what_is_missing_with_the_contracts_exit_codes() {
    let s = Setup::new("cli", None, "", true);
    let home = s.base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(exe())
            .arg("fleet")
            .args(args)
            .current_dir(s.project())
            .env("HOME", &home)
            .env_remove("DEVCROFT_FLEET_CGROUP_ROOT")
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    let (code, err) = run(&["up", "--agents", "1"]);
    assert_eq!(code, Some(4), "{err}");
    assert!(err.contains("--cgroup-root"), "{err}");

    let (code, err) = run(&["up"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("usage"), "{err}");

    let (code, err) = run(&["ls"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("fleet up"), "{err}");

    let (code, _) = run(&["bogus"]);
    assert_eq!(code, Some(2));

    // Limits are validated before anything starts.
    for bad in [
        &["up", "--agents", "1", "--memory", "lots"][..],
        &["up", "--agents", "1", "--cpu-weight", "0"],
        &["up", "--agents", "1", "--io-weight", "20000"],
        &["up", "--agents", "1", "--pids", "-1"],
    ] {
        let (code, err) = run(bad);
        assert_eq!(code, Some(2), "{bad:?}: {err}");
        assert!(err.contains("config"), "{bad:?}: {err}");
    }

    // Removing agents deletes their clones: never without --yes when
    // nobody is at a terminal to have meant it.
    let (code, err) = run(&["rm", "--all"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("--yes"), "{err}");

    // Fleet state lives under `_fleet` in the data dir, which `ps` must not
    // list as a sandbox.
    std::fs::create_dir_all(home.join(".local/share/devcroft/_fleet/x/agents")).unwrap();
    let out = Command::new(exe())
        .arg("ps")
        .env("HOME", &home)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "no sandboxes");
}

/// The whole path with a real provider: devbox resolves the sample's
/// closure for the agent's own clone, and a real `cargo build` succeeds
/// inside the agent, with the repository it was cloned from unreadable.
/// Gated like `tests/mount_view_e2e.rs`, on devbox and a usable Nix store.
#[test]
fn a_real_devbox_agent_builds_its_project_in_its_clone() {
    let Some(root) = capable_host() else { return };
    if Command::new("devbox").arg("version").output().is_err()
        || !devcroft::provider::host_can_build_nix_closures()
    {
        eprintln!("skipping: no devbox, or no usable Nix store");
        return;
    }
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/devbox-citytime-sample");
    let text = std::fs::read_to_string(project.join("devcroft.toml")).unwrap();
    let manifest = devcroft::config::parse(&text).unwrap().0;
    let provider = devcroft::provider::ProviderKind::from_name(&manifest.env.provider).unwrap();

    let base = std::env::temp_dir().join(format!("devcroft-fleet-devbox-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let client =
        devcroft::ssh::ensure_client_keypair(&base.join("id"), &base.join("id.pub")).unwrap();
    let authorized = client.public_key().to_openssh().unwrap();
    let state = base.join("state");

    // Removes the clones this test makes in the sample, its fleet node and
    // its state, whatever happens below.
    struct Cleanup(PathBuf, PathBuf, PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Ok(entries) = std::fs::read_dir(&self.1) {
                for leaf in entries.flatten().filter(|e| e.path().is_dir()) {
                    let _ = std::fs::write(leaf.path().join("cgroup.kill"), "1");
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let _ = std::fs::remove_dir(leaf.path());
                }
            }
            let _ = std::fs::remove_dir(&self.1);
            let _ = std::fs::remove_dir_all(&self.0);
            // `.devcroft` itself, only if this test left it empty.
            if let Some(parent) = self.0.parent() {
                let _ = std::fs::remove_dir(parent);
            }
            let _ = std::fs::remove_dir_all(&self.2);
        }
    }
    let _cleanup = Cleanup(
        project.join(".devcroft/fleet"),
        root.join(format!("fleet-{}", manifest.sandbox.name)),
        base.clone(),
    );

    let started = commands::up(
        &provider,
        &UpRequest {
            manifest: &manifest,
            project_root: &project,
            cgroup_root: &root,
            agents: 1,
            view: true,
            exe: exe(),
            authorized_key_pem: &authorized,
            state_dir: &state,
            limits: devcroft::fleet::cgroup::Limits::default(),
        },
    )
    .unwrap()
    .started;
    let agent = &started[0];
    let socket = state.join("agents").join(&agent.id).join("control.sock");
    let repo = env!("CARGO_MANIFEST_DIR");
    let (code, out) = session(
        &socket,
        &agent.workspace,
        &format!(
            "cargo build 2>&1 | tail -1; \
             ./target/debug/citytime 2>&1 | head -1; \
             cat {repo}/Cargo.toml >/dev/null 2>&1 && echo REPO_READABLE; true"
        ),
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("Finished"), "the build did not finish: {out}");
    assert!(
        out.contains("usage: citytime"),
        "the binary did not run: {out}"
    );
    assert!(!out.contains("REPO_READABLE"), "{out}");
    commands::stop(&state, &manifest, exe(), &agent.id).unwrap();
}

#[test]
fn rm_refuses_a_running_agent_and_removes_a_stopped_one_with_its_clone() {
    let Some(root) = capable_host() else { return };
    let s = Setup::new("rm", Some(&root), "", true);
    let started = s.up(2).unwrap();
    let m = s.manifest();
    let clone = |id: &str| s.project().join(".devcroft/fleet").join(id);

    let err = commands::rm(&s.state(), &m, exe(), &s.project(), "a1").unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert!(err.to_string().contains("stop it first"), "{err}");
    assert!(clone("a1").is_dir());

    commands::stop(&s.state(), &m, exe(), "a1").unwrap();
    commands::rm(&s.state(), &m, exe(), &s.project(), "a1").unwrap();
    assert!(!clone("a1").exists());
    assert!(!s.state().join("agents/a1").exists());
    let listed = commands::ls(&s.state(), &m, exe()).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].record.id, started[1].id);

    // --all stops what is running and leaves nothing: no clones, no node,
    // no state.
    let removed = commands::rm_all(&s.state(), &m, exe(), &s.project()).unwrap();
    assert_eq!(removed, ["a2"]);
    assert!(!clone("a2").exists());
    assert!(!root.join(format!("fleet-{}", s.name)).exists());
    assert!(!s.state().exists());
    // Nor empty directories where the clones were.
    assert!(!s.project().join(".devcroft").exists());
}

/// Preflight runs before anything is cloned, and names what failed.
#[test]
fn preflight_refuses_an_undelegated_cgroup_before_cloning() {
    if !Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
        eprintln!("skipping: no cgroup v2 hierarchy here");
        return;
    }
    // The hierarchy's own root: never delegated to an unprivileged user.
    let s = Setup::new("undelegated", Some(Path::new("/sys/fs/cgroup")), "", true);
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipping: root can write the hierarchy's root");
        return;
    }
    let err = s.up(1).unwrap_err();
    assert_eq!(err.exit_code(), 4, "{err}");
    assert!(
        err.to_string().contains("preflight: cgroup delegation"),
        "{err}"
    );
    assert!(!s.project().join(".devcroft").exists());
}

#[test]
fn limits_given_to_up_reach_every_agents_leaf_and_record() {
    let Some(root) = capable_host() else { return };
    let s = Setup::new("limits", Some(&root), "", true);
    let limits = devcroft::fleet::cgroup::Limits {
        memory_max: Some(64 << 20),
        cpu_weight: Some(50),
        io_weight: Some(50),
        pids_max: Some(64),
    };
    let outcome = s.up_with(2, limits.clone()).unwrap();
    let m = s.manifest();
    for a in &outcome.started {
        let status = commands::inspect(&s.state(), &m, exe(), &a.id).unwrap();
        assert_eq!(status.record.limits, limits);
        let leaf = &status.record.cgroup;
        let read = |f: &str| {
            std::fs::read_to_string(leaf.join(f))
                .unwrap()
                .trim()
                .to_owned()
        };
        assert_eq!(read("memory.max"), (64u64 << 20).to_string());
        assert_eq!(read("memory.swap.max"), "0");
        assert_eq!(read("pids.max"), "64");
        assert_eq!(read("cpu.weight"), "50");
    }
    // An IO weight this host cannot apply is reported, by name.
    let has_io_weight = commands::inspect(&s.state(), &m, exe(), "a1")
        .unwrap()
        .record
        .cgroup
        .join("io.weight")
        .exists();
    assert_eq!(
        outcome
            .degraded
            .contains(&devcroft::fleet::cgroup::Degraded::IoWeight),
        !has_io_weight
    );
    commands::rm_all(&s.state(), &m, exe(), &s.project()).unwrap();
}
