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
        // No ignore for `.devcroft/`: fleet's own `.gitignore` in its
        // clones' directory is what must keep them out of `git status`.
        std::fs::write(base.join("repo").join("README"), "test\n").unwrap();
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

    fn repo(&self) -> PathBuf {
        self.base.join("repo")
    }

    /// Where this fleet clones: under the repository root, not the project,
    /// in the fleet's own directory (`commands::clone_root`).
    fn clones(&self) -> PathBuf {
        commands::clone_root(&self.project(), &self.name, "a0")
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    fn state(&self) -> PathBuf {
        self.base.join("state")
    }

    /// Run a command after `up` on this fleet, named as the CLI names it.
    fn cmd<T>(&self, run: impl FnOnce(&commands::FleetRef) -> T) -> T {
        let (state, manifest, project) = (self.state(), self.manifest(), self.project());
        run(&commands::FleetRef {
            state_dir: &state,
            manifest: &manifest,
            project_root: &project,
            exe: exe(),
        })
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
        self.up_via(&HostUsr, agents, limits)
    }

    fn up_via(
        &self,
        provider: &dyn ProviderEntry,
        agents: usize,
        limits: devcroft::fleet::cgroup::Limits,
    ) -> Result<commands::UpOutcome, FleetError> {
        self.up_full(provider, agents, limits, Default::default())
    }

    fn up_capped(
        &self,
        agents: usize,
        fleet_limits: devcroft::fleet::cgroup::FleetLimits,
    ) -> Result<commands::UpOutcome, FleetError> {
        self.up_full(&HostUsr, agents, Default::default(), fleet_limits)
    }

    fn up_task(&self, agents: usize, task: &[&str]) -> Result<commands::UpOutcome, FleetError> {
        self.up_all(
            &HostUsr,
            agents,
            Default::default(),
            Default::default(),
            Some(task.iter().map(|a| a.to_string()).collect()),
        )
    }

    fn up_full(
        &self,
        provider: &dyn ProviderEntry,
        agents: usize,
        limits: devcroft::fleet::cgroup::Limits,
        fleet_limits: devcroft::fleet::cgroup::FleetLimits,
    ) -> Result<commands::UpOutcome, FleetError> {
        self.up_all(provider, agents, limits, fleet_limits, None)
    }

    fn up_all(
        &self,
        provider: &dyn ProviderEntry,
        agents: usize,
        limits: devcroft::fleet::cgroup::Limits,
        fleet_limits: devcroft::fleet::cgroup::FleetLimits,
        task: Option<Vec<String>>,
    ) -> Result<commands::UpOutcome, FleetError> {
        let manifest = self.manifest();
        let keys = self.base.join("keys");
        std::fs::create_dir_all(&keys).unwrap();
        let client =
            devcroft::ssh::ensure_client_keypair(&keys.join("id"), &keys.join("id.pub")).unwrap();
        let authorized = client.public_key().to_openssh().unwrap();
        let fallback = self.base.join("no-cgroup");
        commands::up(
            provider,
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
                fleet_limits,
                timeout: None,
                task,
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

/// Where every agent with a view sees its workspace.
const WORKSPACE: &str = devcroft::fleet::mount::WORKSPACE;
/// Where an agent starts: `Setup`'s project is `proj/` in its repository,
/// and an agent's view mounts its whole clone at `/workspace` (3.7).
const PROJECT: &str = "/workspace/proj";

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
        match protocol::read_frame(&mut stream) {
            Ok(Frame::SpawnErr { message }) => panic!("keeper refused to spawn: {message}"),
            Ok(Frame::Stdout(b) | Frame::Stderr(b)) => out.push_str(&String::from_utf8_lossy(&b)),
            Ok(Frame::Exit(status)) => return (status.code, out),
            Ok(_) => {}
            // The keeper went away mid-session: its leaf was killed whole
            // (`memory.oom.group`). Reported as no exit code, never a pass.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                out.push_str("<the keeper's connection closed>");
                return (None, out);
            }
            Err(e) => panic!("reading from the keeper: {e}"),
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
    // The clones are not untracked files in the repository they came from.
    let status = Command::new("git")
        .arg("-C")
        .arg(s.repo())
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&status.stdout), "");

    for a in &started {
        // The project's own directory inside the agent's clone, not the
        // clone's root.
        let clone_root = s.clones().join(&a.id);
        assert_eq!(a.workspace, clone_root.join("proj"));
        assert!(a.workspace.join("devcroft.toml").is_file());

        let socket = s.state().join("agents").join(&a.id).join("control.sock");
        let (code, out) = session(
            &socket,
            Path::new(PROJECT),
            &format!("echo {} > mine; pwd", a.id),
        );
        assert_eq!(code, Some(0), "{out}");
        // Its project is under `/workspace`, whatever its host path.
        assert_eq!(out.trim(), PROJECT);
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
    let listed = s.cmd(commands::ls).unwrap();
    assert!(listed.iter().all(|a| a.record.state == AgentState::Running));
    s.cmd(|f| commands::stop(f, "a1")).unwrap();
    let listed = s.cmd(commands::ls).unwrap();
    assert_eq!(listed[0].record.state, AgentState::Stopped);
    assert_eq!(listed[1].record.state, AgentState::Running);
    // Stopping an agent does not throw away its work.
    assert!(started[0].workspace.join("mine").is_file());

    let err = s.cmd(|f| commands::stop(f, "a9")).unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    s.cmd(|f| commands::stop(f, "a2")).unwrap();
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
    assert!(!s.clones().join("a1").exists());
    assert!(!s.state().join("agents/a1").exists());
}

#[test]
fn a_project_outside_git_is_refused_before_anything_is_made() {
    let s = Setup::new("nogit", None, "", false);
    let err = s.up(1).unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert!(err.to_string().contains("git repository"), "{err}");
    assert!(!s.state().exists());
    assert!(!s.repo().join(".devcroft").exists());
}

#[test]
fn ls_without_a_fleet_says_how_to_start_one() {
    let s = Setup::new("none", None, "", true);
    let err = s.cmd(commands::ls).unwrap_err();
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
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".devcroft/fleet"),
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
            fleet_limits: Default::default(),
            timeout: None,
            task: None,
        },
    )
    .unwrap()
    .started;
    let agent = &started[0];
    let socket = state.join("agents").join(&agent.id).join("control.sock");
    let repo = env!("CARGO_MANIFEST_DIR");
    // The sample is a subdirectory of this repository, so the agent's view
    // mounts its whole clone and the sample sits beneath `/workspace` (3.7).
    let clone = commands::agent_clone_root(&agent.workspace, &agent.id).unwrap();
    let inside = Path::new(WORKSPACE).join(agent.workspace.strip_prefix(&clone).unwrap());
    assert_eq!(
        inside,
        Path::new("/workspace/samples/devbox-citytime-sample")
    );
    // A store path in no closure, made on the host just now. With the whole
    // store in the view (a bind of `/nix/store` itself) it would be visible;
    // narrowed to the agent's closure (`fleet::closure`) it is not.
    let unrelated = {
        let file = base.join("unrelated-store-probe");
        std::fs::write(&file, format!("{}\n", std::process::id())).unwrap();
        let out = Command::new("nix-store")
            .arg("--add")
            .arg(&file)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let (code, out) = session(
        &socket,
        &inside,
        &format!(
            "cargo build 2>&1 | tail -1; \
             ./target/debug/citytime 2>&1 | head -1; \
             cat {repo}/Cargo.toml >/dev/null 2>&1 && echo REPO_READABLE; \
             test -e {unrelated} && echo UNRELATED_STORE_PATH_VISIBLE; true"
        ),
    );
    let _ = Command::new("nix-store")
        .arg("--delete")
        .arg(&unrelated)
        .output();
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("Finished"), "the build did not finish: {out}");
    assert!(
        out.contains("usage: citytime"),
        "the binary did not run: {out}"
    );
    assert!(!out.contains("REPO_READABLE"), "{out}");
    assert!(
        !out.contains("UNRELATED_STORE_PATH_VISIBLE"),
        "the agent sees store paths outside its closure: {out}"
    );
    commands::stop(
        &commands::FleetRef {
            state_dir: &state,
            manifest: &manifest,
            project_root: &project,
            exe: exe(),
        },
        &agent.id,
    )
    .unwrap();
}

#[test]
fn rm_refuses_a_running_agent_and_removes_a_stopped_one_with_its_clone() {
    let Some(root) = capable_host() else { return };
    let s = Setup::new("rm", Some(&root), "", true);
    let started = s.up(2).unwrap();
    let clone = |id: &str| s.clones().join(id);

    let err = s.cmd(|f| commands::rm(f, "a1")).unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert!(err.to_string().contains("stop it first"), "{err}");
    assert!(clone("a1").is_dir());

    s.cmd(|f| commands::stop(f, "a1")).unwrap();
    s.cmd(|f| commands::rm(f, "a1")).unwrap();
    assert!(!clone("a1").exists());
    assert!(!s.state().join("agents/a1").exists());
    let listed = s.cmd(commands::ls).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].record.id, started[1].id);

    // --all stops what is running and leaves nothing: no clones, no node,
    // no state.
    let removed = s.cmd(commands::rm_all).unwrap();
    assert_eq!(removed, ["a2"]);
    assert!(!clone("a2").exists());
    assert!(!root.join(format!("fleet-{}", s.name)).exists());
    assert!(!s.state().exists());
    // Nor empty directories where the clones were.
    assert!(!s.repo().join(".devcroft").exists());
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
    assert!(!s.repo().join(".devcroft").exists());
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
    for a in &outcome.started {
        let status = s.cmd(|f| commands::inspect(f, &a.id)).unwrap();
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
    let has_io_weight = s
        .cmd(|f| commands::inspect(f, "a1"))
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
    s.cmd(commands::rm_all).unwrap();
}

/// The host's `/usr` plus a process-compose from the Nix store, declaring
/// `services`: the smallest environment that can run a service stack.
struct WithServices {
    supervisor_dir: PathBuf,
    services: Vec<devcroft::provider::ServiceDecl>,
}

impl ProviderEntry for WithServices {
    fn resolve(&self, _: &Path) -> Result<Resolution, ProviderError> {
        Ok(Resolution {
            env: [(
                "PATH".to_owned(),
                format!("{}:/usr/bin", self.supervisor_dir.display()),
            )]
            .into(),
            unset: Vec::new(),
            read_only_grants: vec!["/usr".to_owned(), "/nix/store".to_owned()],
            activation_script: None,
            services: ServiceSupport::Declared(self.services.clone()),
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

fn process_compose_dir() -> Option<PathBuf> {
    std::fs::read_dir("/nix/store")
        .ok()?
        .flatten()
        .map(|e| e.path().join("bin"))
        .find(|bin| {
            bin.join("process-compose").is_file()
                && bin
                    .parent()
                    .unwrap()
                    .to_string_lossy()
                    .contains("process-compose")
        })
}

fn service(name: &str, command: &str, probe: Option<&str>) -> devcroft::provider::ServiceDecl {
    devcroft::provider::ServiceDecl {
        name: name.into(),
        command: command.into(),
        vars: Default::default(),
        is_daemon: false,
        working_dir: None,
        depends_on: Vec::new(),
        restart: devcroft::provider::RestartPolicy::Never,
        shutdown: devcroft::provider::Shutdown::Default,
        readiness: probe.map(|p| devcroft::provider::Readiness {
            probe: devcroft::provider::Probe::Command(p.into()),
            initial_delay: 0,
            period: 1,
            probe_timeout: 2,
            success_threshold: 1,
            failure_threshold: 60,
        }),
    }
}

/// `service-ports`: every agent runs its own instance of the same declared
/// service on the same port, and `up` returns only once each is ready.
#[test]
fn every_agent_runs_its_own_service_on_the_same_port_and_up_waits_for_it() {
    let Some(root) = capable_host() else { return };
    let Some(supervisor_dir) = process_compose_dir() else {
        eprintln!("skipping: no process-compose in /nix/store");
        return;
    };
    if !Path::new("/usr/bin/python3").exists() || !Path::new("/usr/bin/curl").exists() {
        eprintln!("skipping: no /usr/bin/python3 or /usr/bin/curl");
        return;
    }
    let s = Setup::new("svc", Some(&root), "\n[network]\nports = [8000]\n", true);
    // Ready only after it has written `probe-ok`, two seconds in: `up`
    // returning before that would mean it did not wait for readiness.
    let provider = WithServices {
        supervisor_dir,
        services: vec![service(
            "web",
            "sleep 2; touch probe-ok; exec python3 -m http.server 8000 --bind 127.0.0.1",
            Some("test -f probe-ok && curl -sf http://127.0.0.1:8000/ >/dev/null"),
        )],
    };
    let outcome = s
        .up_via(&provider, 3, devcroft::fleet::cgroup::Limits::default())
        .unwrap();
    for a in &outcome.started {
        assert_eq!(a.services, commands::ServicesOutcome::Ready, "{}", a.id);
        assert!(
            a.workspace.join("probe-ok").exists(),
            "up returned before {}'s service was ready",
            a.id
        );
    }
    // Three agents, one port number, three instances: each answers with
    // a file from its own workspace.
    for a in &outcome.started {
        let socket = s.state().join("agents").join(&a.id).join("control.sock");
        let (code, out) = session(
            &socket,
            Path::new(PROJECT),
            &format!(
                "echo {} > whoami; curl -s http://127.0.0.1:8000/whoami",
                a.id
            ),
        );
        assert_eq!(code, Some(0), "{out}");
        assert_eq!(out.trim(), a.id);
    }
    s.cmd(commands::rm_all).unwrap();
}

/// A service that fails is reported, by name, for its agent; the agent
/// stays up and keeps serving sessions.
#[test]
fn a_failing_service_is_reported_for_its_agent_which_stays_up() {
    let Some(root) = capable_host() else { return };
    let Some(supervisor_dir) = process_compose_dir() else {
        eprintln!("skipping: no process-compose in /nix/store");
        return;
    };
    let s = Setup::new("svcfail", Some(&root), "", true);
    let provider = WithServices {
        supervisor_dir,
        services: vec![service("broken", "exit 3", None)],
    };
    let outcome = s
        .up_via(&provider, 1, devcroft::fleet::cgroup::Limits::default())
        .unwrap();
    let a = &outcome.started[0];
    match &a.services {
        commands::ServicesOutcome::Failed(v) => {
            assert_eq!(v[0].0, "broken", "{v:?}");
            assert!(v[0].1.contains("exit 3"), "{v:?}");
        }
        other => panic!("expected the failure to be reported, got {other:?}"),
    }
    let socket = s.state().join("agents").join(&a.id).join("control.sock");
    let (code, out) = session(&socket, Path::new(PROJECT), "echo still-up");
    assert_eq!((code, out.trim()), (Some(0), "still-up"));
    s.cmd(commands::rm_all).unwrap();
}

/// One HTTP GET over a plain TCP connection from the host; the body.
fn get_from_host(port: u16, path: &str) -> std::io::Result<String> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    write!(stream, "GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.trim().to_owned())
        .unwrap_or_default())
}

/// 5.6 and 5.3/5.4: five agents run the same service on the same declared
/// port, each gets its own host port, each host port reaches the right
/// agent, and a stopped agent's port is released.
#[test]
fn five_agents_one_port_each_host_mapping_reaches_its_own_agent() {
    let Some(root) = capable_host() else { return };
    let Some(supervisor_dir) = process_compose_dir() else {
        eprintln!("skipping: no process-compose in /nix/store");
        return;
    };
    if !Path::new("/usr/bin/python3").exists() {
        eprintln!("skipping: no /usr/bin/python3");
        return;
    }
    let s = Setup::new(
        "expose",
        Some(&root),
        "\n[network.services.web]\nport = 8000\nexpose = true\n",
        true,
    );
    let provider = WithServices {
        supervisor_dir,
        // With a probe, "ready" means listening. Without one it only means
        // started, and under load the first GET below beat the server's
        // bind (1 run in 10).
        services: vec![service(
            "web",
            "exec python3 -m http.server 8000 --bind 127.0.0.1",
            Some("curl -sf http://127.0.0.1:8000/ >/dev/null"),
        )],
    };
    let outcome = s
        .up_via(&provider, 5, devcroft::fleet::cgroup::Limits::default())
        .unwrap();

    let mut host_ports = Vec::new();
    for a in &outcome.started {
        assert_eq!(a.services, commands::ServicesOutcome::Ready, "{}", a.id);
        // Each agent's instance serves its own workspace.
        std::fs::write(a.workspace.join("whoami"), &a.id).unwrap();
        let record = s.cmd(|f| commands::inspect(f, &a.id)).unwrap().record;
        assert_eq!(record.exposes, ["web"]);
        let [mapping] = &record.port_mappings[..] else {
            panic!("{}: {:?}", a.id, record.port_mappings)
        };
        assert_eq!((mapping.service.as_str(), mapping.agent), ("web", 8000));
        host_ports.push(mapping.host);
    }
    // Five distinct host ports for one declared port.
    let mut distinct = host_ports.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 5, "{host_ports:?}");

    // From the host, each mapping reaches its own agent and no other.
    for (a, port) in outcome.started.iter().zip(&host_ports) {
        assert_eq!(get_from_host(*port, "/whoami").unwrap(), a.id);
    }

    // Stopping one releases its port and leaves the rest working.
    s.cmd(|f| commands::stop(f, "a1")).unwrap();
    // Released means free: the port can be bound again, which is what lets
    // a later agent be given it. A refused connection alone would not show
    // that (a forwarder left running with nothing behind it refuses too).
    assert!(
        std::net::TcpListener::bind(("127.0.0.1", host_ports[0])).is_ok(),
        "a stopped agent's host port is still held"
    );
    let a1 = s.cmd(|f| commands::inspect(f, "a1")).unwrap().record;
    assert!(a1.port_mappings.is_empty());
    assert_eq!(
        a1.exposes,
        ["web"],
        "declared stays distinguishable from released"
    );
    assert_eq!(get_from_host(host_ports[1], "/whoami").unwrap(), "a2");

    s.cmd(commands::rm_all).unwrap();
}

/// `HostUsr`, slowly: holds `up` between creating an agent's directory and
/// recording it, the window a concurrent command used to fall into.
struct Slow;

impl ProviderEntry for Slow {
    fn resolve(&self, root: &Path) -> Result<Resolution, ProviderError> {
        std::thread::sleep(std::time::Duration::from_secs(2));
        HostUsr.resolve(root)
    }
    fn fingerprint(&self, root: &Path) -> Result<String, ProviderError> {
        HostUsr.fingerprint(root)
    }
    fn tier(&self) -> Tier {
        HostUsr.tier()
    }
    fn static_name(&self) -> &'static str {
        HostUsr.static_name()
    }
}

/// Fleet commands are serialized per fleet. Without that, an `ls` during an
/// `up` found the new agent's directory with no record yet, took it for a
/// crashed start and removed it, and the `up` failed underneath it; and two
/// `up`s computed the same next ID.
#[test]
fn a_command_during_up_waits_for_it_instead_of_removing_its_agent() {
    let Some(root) = capable_host() else { return };
    let s = Setup::new("lock", Some(&root), "", true);
    std::thread::scope(|scope| {
        let up = scope.spawn(|| s.up_via(&Slow, 1, devcroft::fleet::cgroup::Limits::default()));
        let dir = s.state().join("agents/a1");
        for _ in 0..500 {
            if dir.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(dir.exists(), "up never made its agent's directory");
        assert!(!dir.join("record.json").exists(), "the window was missed");
        // Blocks until `up` has recorded a1, then sees it running.
        let listed = s.cmd(commands::ls).unwrap();
        let ids: Vec<_> = listed.iter().map(|a| a.record.id.as_str()).collect();
        assert_eq!(ids, ["a1"]);
        assert_eq!(listed[0].record.state, AgentState::Running);
        let started = up.join().unwrap().unwrap();
        assert_eq!(started.started[0].id, "a1");
    });
    // Two `up`s at once get different IDs, and both succeed.
    std::thread::scope(|scope| {
        let a = scope.spawn(|| s.up_via(&Slow, 1, devcroft::fleet::cgroup::Limits::default()));
        let b = scope.spawn(|| s.up_via(&Slow, 1, devcroft::fleet::cgroup::Limits::default()));
        let mut ids = vec![
            a.join().unwrap().unwrap().started[0].id.clone(),
            b.join().unwrap().unwrap().started[0].id.clone(),
        ];
        ids.sort();
        assert_eq!(ids, ["a2", "a3"]);
    });
    s.cmd(commands::rm_all).unwrap();
}

/// A fleet belongs to the project that started it. State is keyed by
/// sandbox name, which two checkouts of one repository share; the first
/// version let the second's `up` rewrite `fleet.json` and add its clones to
/// the first's agents, and its `ls`/`stop`/`rm` act on them.
#[test]
fn a_second_checkout_with_the_same_name_cannot_take_over_the_fleet() {
    let Some(root) = capable_host() else { return };
    let owner = Setup::new("own", Some(&root), "", true);
    owner.up(1).unwrap();
    let recorded = std::fs::read(owner.state().join("fleet.json")).unwrap();

    // Another repository whose manifest carries the owner's sandbox name.
    let other = Setup::new("oth", Some(&root), "", true);
    std::fs::copy(
        owner.project().join("devcroft.toml"),
        other.project().join("devcroft.toml"),
    )
    .unwrap();
    let manifest = owner.manifest();
    let theirs = commands::FleetRef {
        state_dir: &owner.state(),
        manifest: &manifest,
        project_root: &other.project(),
        exe: exe(),
    };
    let keys = other.base.join("keys");
    std::fs::create_dir_all(&keys).unwrap();
    let client =
        devcroft::ssh::ensure_client_keypair(&keys.join("id"), &keys.join("id.pub")).unwrap();
    let err = commands::up(
        &HostUsr,
        &UpRequest {
            manifest: &manifest,
            project_root: &other.project(),
            cgroup_root: &root,
            agents: 1,
            view: true,
            exe: exe(),
            authorized_key_pem: &client.public_key().to_openssh().unwrap(),
            state_dir: &owner.state(),
            limits: devcroft::fleet::cgroup::Limits::default(),
            fleet_limits: Default::default(),
            timeout: None,
            task: None,
        },
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert!(err.to_string().contains("different project"), "{err}");
    assert_eq!(
        std::fs::read(owner.state().join("fleet.json")).unwrap(),
        recorded,
        "fleet.json was rewritten before the refusal"
    );
    assert!(!other.clones().exists(), "the refused up made a clone");
    for result in [
        commands::ls(&theirs).map(drop),
        commands::stop(&theirs, "a1"),
        commands::rm(&theirs, "a1"),
    ] {
        let err = result.unwrap_err();
        assert!(err.to_string().contains("different project"), "{err}");
    }
    // The owner's agent is untouched, and still the owner's to remove.
    let listed = owner.cmd(commands::ls).unwrap();
    assert_eq!(listed[0].record.state, AgentState::Running);
    owner.cmd(commands::rm_all).unwrap();
}

/// The proposal's access path, `ssh a17.myrepo.devcroft`, through the same
/// `ProxyCommand devcroft proxy %n` the generated SSH config uses. `proxy`
/// used to parse one `<name>.devcroft` and open the ordinary sandbox's
/// socket, so an agent's SSH listener existed and nothing could reach it.
/// Run with its own `HOME`, so the proxy finds the fleet where the CLI puts
/// it and authenticates with that home's client key.
#[test]
fn an_agent_is_reachable_as_id_dot_name_dot_devcroft_over_real_ssh() {
    let Some(root) = capable_host() else { return };
    if !Path::new("/usr/bin/ssh").exists() {
        eprintln!("skipping: no /usr/bin/ssh");
        return;
    }
    let s = Setup::new("ssh", Some(&root), "", true);
    let home = s.base.join("home");
    let data = home.join(".local/share/devcroft");
    std::fs::create_dir_all(&data).unwrap();
    let client = devcroft::ssh::ensure_client_keypair(
        &data.join("id_ed25519"),
        &data.join("id_ed25519.pub"),
    )
    .unwrap();
    let state = data.join("_fleet").join(&s.name);
    let manifest = s.manifest();
    commands::up(
        &HostUsr,
        &UpRequest {
            manifest: &manifest,
            project_root: &s.project(),
            cgroup_root: &root,
            agents: 2,
            view: true,
            exe: exe(),
            authorized_key_pem: &client.public_key().to_openssh().unwrap(),
            state_dir: &state,
            limits: devcroft::fleet::cgroup::Limits::default(),
            fleet_limits: Default::default(),
            timeout: None,
            task: None,
        },
    )
    .unwrap();

    let ssh = |host: &str| {
        Command::new("ssh")
            .env("HOME", &home)
            .args(["-F", "/dev/null", "-o", "BatchMode=yes"])
            .args([
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
            ])
            .arg("-o")
            .arg(format!("ProxyCommand {} proxy %n", exe().display()))
            .arg("-i")
            .arg(data.join("id_ed25519"))
            // The proposal's own form: `ssh a17.myrepo.devcroft 'cd /workspace && …'`.
            .args([host, "cd /workspace && uname -n && pwd"])
            .output()
            .unwrap()
    };
    // Each name reaches its own agent: the hostname is the agent's ID.
    for id in ["a1", "a2"] {
        let out = ssh(&format!("{id}.{}.devcroft", s.name));
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            format!("{id}\n{WORKSPACE}")
        );
    }
    // A stopped agent is named as such, not as a missing sandbox.
    let fleet = commands::FleetRef {
        state_dir: &state,
        manifest: &manifest,
        project_root: &s.project(),
        exe: exe(),
    };
    commands::stop(&fleet, "a1").unwrap();
    let out = ssh(&format!("a1.{}.devcroft", s.name));
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("agent a1 of fleet"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    commands::rm_all(&fleet).unwrap();
}

/// `--timeout` takes a whole number with a unit; anything else is a usage
/// error naming the flag, before anything is cloned or started.
#[test]
fn a_timeout_without_a_unit_is_refused_by_the_cli() {
    let s = Setup::new("tflag", None, "", true);
    for bad in ["30", "5x", "0s", "m"] {
        let out = Command::new(exe())
            .current_dir(s.project())
            .args(["fleet", "up", "--agents", "1", "--timeout", bad])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{bad}: {stderr}");
        assert!(stderr.contains("--timeout"), "{bad}: {stderr}");
    }
    assert!(!s.clones().exists());
}

/// `--fleet-cpus` takes a positive number of CPUs to at most three decimals
/// (`cpu.max` in thousandths); anything else is a usage error naming the
/// flag. Accepted values get past parsing, to the missing cgroup root.
#[test]
fn fleet_cpus_is_a_positive_number_of_cpus() {
    let s = Setup::new("cpus", None, "", true);
    let up = |value: &str| {
        let out = Command::new(exe())
            .current_dir(s.project())
            .env_remove("DEVCROFT_FLEET_CGROUP_ROOT")
            .args(["fleet", "up", "--agents", "1", "--fleet-cpus", value])
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    for bad in ["0", "0.000", "-1", "1.2345", "x", ".", "", "1e3"] {
        let (code, stderr) = up(bad);
        assert_eq!(code, Some(2), "{bad:?}: {stderr}");
        assert!(stderr.contains("--fleet-cpus"), "{bad:?}: {stderr}");
    }
    for good in ["2", "0.5", ".25", "1.125"] {
        let (code, stderr) = up(good);
        assert_eq!(code, Some(4), "{good:?}: {stderr}");
        assert!(stderr.contains("cgroup"), "{good:?}: {stderr}");
    }
    assert!(!s.clones().exists());
}

/// `--host-root` cannot withhold the Nix daemon: Landlock does not mediate
/// connecting to a unix socket, and a host-root agent was measured adding a
/// path to the shared store through it. So on a host running the daemon the
/// strategy is refused, naming the socket, before anything is cloned.
#[test]
fn host_root_is_refused_where_the_nix_daemon_runs() {
    if !Path::new(devcroft::provider::NIX_DAEMON_SOCKET).exists() {
        eprintln!("skipping: no Nix daemon socket on this host");
        return;
    }
    let s = Setup::new("hostroot", None, "", true);
    let manifest = s.manifest();
    let err = commands::up(
        &HostUsr,
        &UpRequest {
            manifest: &manifest,
            project_root: &s.project(),
            cgroup_root: &s.base.join("no-cgroup"),
            agents: 1,
            view: false,
            exe: exe(),
            authorized_key_pem: "unused",
            state_dir: &s.state(),
            limits: devcroft::fleet::cgroup::Limits::default(),
            fleet_limits: Default::default(),
            timeout: None,
            task: None,
        },
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert!(
        err.to_string()
            .contains(devcroft::provider::NIX_DAEMON_SOCKET),
        "{err}"
    );
    assert!(err.to_string().contains("cannot be confined"), "{err}");
    assert!(!s.clones().exists());
    assert!(!s.state().exists());
}

/// `workspace-isolation`, the git half, on the clones `fleet up` makes.
/// Each holds its own copies of the objects (`--no-hardlinks`): a hardlinked
/// clone shared inodes with the source and its siblings, so a write through
/// one corrupted all of them (adversarial review). And it depends on
/// nothing the source can prune.
#[test]
fn agent_clones_are_independent_unpushable_and_fetchable() {
    let Some(root) = capable_host() else { return };
    let s = Setup::new("git", Some(&root), "", true);
    let started = s.up(5).unwrap();
    let git = |dir: &Path, args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=agent", "-c", "user.email=a@a"])
            .args(args)
            .output()
            .unwrap()
    };
    let user_refs_before = git(&s.repo(), &["show-ref"]).stdout;

    // Settled: each clone's objects are its own (no inode shared with the
    // source, so a write through one cannot reach another), and a push is
    // refused with the reason.
    let a1 = s.clones().join("a1");
    {
        use std::os::unix::fs::MetadataExt;
        let inodes = |root: &Path| -> std::collections::BTreeSet<(u64, u64)> {
            let mut out = std::collections::BTreeSet::new();
            let mut stack = vec![root.join(".git/objects")];
            while let Some(dir) = stack.pop() {
                for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    let m = e.metadata().unwrap();
                    if m.is_dir() {
                        stack.push(e.path());
                    } else {
                        out.insert((m.dev(), m.ino()));
                    }
                }
            }
            out
        };
        let source = inodes(&s.repo());
        assert!(!source.is_empty());
        assert!(
            inodes(&a1).is_disjoint(&source),
            "an agent's clone shares object files with the user's repository"
        );
    }
    let push = git(&a1, &["push", "origin", "HEAD"]);
    assert!(!push.status.success());
    assert!(
        String::from_utf8_lossy(&push.stderr).contains("agents do not push"),
        "{}",
        String::from_utf8_lossy(&push.stderr)
    );

    // Five agents commit, branch and rebase at once, each inside itself:
    // no lock failure, since no two share a ref namespace or an index. Run
    // in the agents, not host-side on their clones, because the project is
    // `proj/`, a subdirectory: an agent sees its repository only because
    // its view mounts the whole clone (3.7). The first version of this test
    // ran git on the host and so never noticed an agent had no `.git`.
    std::thread::scope(|scope| {
        for a in &started {
            let socket = s.state().join("agents").join(&a.id).join("control.sock");
            scope.spawn(move || {
                let (code, out) = session(
                    &socket,
                    Path::new(PROJECT),
                    "set -e; g() { git -c user.name=agent -c user.email=a@a \"$@\"; }; \
                     for n in $(seq 15); do echo x > ../work-$n; g add -A; \
                     g commit -qm work; g branch -f topic; g rebase -q topic; done; \
                     git rev-parse --show-toplevel",
                );
                assert_eq!(code, Some(0), "{}: {out}", a.id);
                assert_eq!(out.trim(), WORKSPACE, "{}: {out}", a.id);
            });
        }
    });
    // Each clone's refs are its own, and the user's repository saw nothing.
    assert_eq!(git(&s.repo(), &["show-ref"]).stdout, user_refs_before);

    // Integration is a fetch from the clone, as `fleet inspect` prints it.
    let fetch = git(
        &s.repo(),
        &["fetch", "-q", a1.to_str().unwrap(), "HEAD:refs/agents/a1"],
    );
    assert!(
        fetch.status.success(),
        "{}",
        String::from_utf8_lossy(&fetch.stderr)
    );
    let head = |dir: &Path, r: &str| {
        String::from_utf8_lossy(&git(dir, &["rev-parse", r]).stdout)
            .trim()
            .to_owned()
    };
    assert_eq!(head(&s.repo(), "refs/agents/a1"), head(&a1, "HEAD"));
    // And the path `fleet inspect` prints the fetch from is that clone.
    assert_eq!(
        commands::agent_clone_root(&started[0].workspace, "a1").as_deref(),
        Some(a1.as_path())
    );

    // The source rewrites its history and prunes everything; a clone, which
    // holds its own links to the objects it started with, stays whole.
    for args in [
        &["checkout", "-q", "--orphan", "rewritten"][..],
        &["commit", "-q", "--allow-empty", "-m", "fresh"],
        &["reflog", "expire", "--expire=now", "--all"],
    ] {
        assert!(git(&s.repo(), args).status.success(), "{args:?}");
    }
    for branch in ["master", "main"] {
        let _ = git(&s.repo(), &["branch", "-qD", branch]);
    }
    let _ = git(&s.repo(), &["update-ref", "-d", "refs/agents/a1"]);
    assert!(
        git(&s.repo(), &["gc", "-q", "--prune=now"])
            .status
            .success()
    );
    let fsck = git(&s.clones().join("a2"), &["fsck", "--no-progress"]);
    assert!(
        fsck.status.success(),
        "{}",
        String::from_utf8_lossy(&fsck.stderr)
    );

    s.cmd(commands::rm_all).unwrap();
}

/// Two fleets of one repository (two projects in it, two sandbox names)
/// each have an `a1`, and neither touches the other's clone. The clones
/// were addressed by agent ID alone, and a start that failed because the
/// path already existed then removed the existing directory: another fleet's
/// running agent's workspace (adversarial review, reproduced). Clones are now
/// keyed by fleet, and only a directory the start itself created is removed.
#[test]
fn two_fleets_in_one_repository_never_share_or_remove_each_others_clones() {
    let Some(root) = capable_host() else { return };
    let a = Setup::new("twoa", Some(&root), "", true);
    // A second project in the same repository, with its own name.
    let proj2 = a.repo().join("proj2");
    std::fs::create_dir_all(&proj2).unwrap();
    let name_b = format!("{}b", a.name);
    std::fs::write(
        proj2.join("devcroft.toml"),
        format!("[sandbox]\nname = \"{name_b}\"\n\n[env]\nprovider = \"nix\"\n\n[filesystem]\nread = [\"/proc\"]\n"),
    )
    .unwrap();
    for args in [
        &["add", "."][..],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "b",
        ],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(a.repo())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let started_a = a.up(1).unwrap();
    std::fs::write(started_a[0].workspace.join("work-of-a"), "a").unwrap();

    let manifest_b =
        devcroft::config::parse(&std::fs::read_to_string(proj2.join("devcroft.toml")).unwrap())
            .unwrap()
            .0;
    let state_b = a.base.join("state-b");
    let up_b = |exe: &Path| {
        commands::up(
            &HostUsr,
            &UpRequest {
                manifest: &manifest_b,
                project_root: &proj2,
                cgroup_root: &root,
                agents: 1,
                view: true,
                exe,
                authorized_key_pem: "",
                state_dir: &state_b,
                limits: devcroft::fleet::cgroup::Limits::default(),
                fleet_limits: Default::default(),
                timeout: None,
                task: None,
            },
        )
    };
    // A directory already at B's would-be clone path is not B's to remove:
    // the start refuses, and the directory and its contents survive.
    let b_a1 = commands::clone_root(&proj2, &name_b, "a1").unwrap();
    std::fs::create_dir_all(&b_a1).unwrap();
    std::fs::write(b_a1.join("not-yours"), "x").unwrap();
    let refused = up_b(exe()).unwrap_err();
    assert!(refused.to_string().contains("already exists"), "{refused}");
    assert!(
        b_a1.join("not-yours").is_file(),
        "a failed start removed a directory it did not make"
    );
    std::fs::remove_dir_all(&b_a1).unwrap();

    // B's own a1 is a different directory from A's, and A's work survives.
    let started_b = up_b(exe()).unwrap().started;
    assert_ne!(
        commands::agent_clone_root(&started_a[0].workspace, "a1"),
        commands::agent_clone_root(&started_b[0].workspace, "a1")
    );
    assert!(started_a[0].workspace.join("work-of-a").is_file());

    let fleet_b = commands::FleetRef {
        state_dir: &state_b,
        manifest: &manifest_b,
        project_root: &proj2,
        exe: exe(),
    };
    commands::rm_all(&fleet_b).unwrap();
    assert!(
        started_a[0].workspace.join("work-of-a").is_file(),
        "B's rm removed A's clone"
    );
    let node_b = root.join(format!("fleet-{name_b}"));
    let _ = std::fs::remove_dir(node_b);
    a.cmd(commands::rm_all).unwrap();
}

/// `fleet up` reports an agent as up only once its keeper answers, which it
/// does after its hooks. It returned on the init helper's handshake (the
/// keeper was exec'd), so with `post_create = "exit 7"` it exited 0 and the
/// agent was Stopped a second later (adversarial review, reproduced).
#[test]
fn an_agent_whose_hook_fails_is_reported_as_not_started() {
    let Some(root) = capable_host() else { return };
    let s = Setup::new(
        "hookfail",
        Some(&root),
        "\n[hooks]\npost_create = \"exit 7\"\n",
        true,
    );
    let started = s.up(1).unwrap();
    let why = started[0]
        .failed
        .as_deref()
        .expect("a failing hook is a failed start");
    assert!(why.contains("post_create"), "{why}");

    // The control: an agent with no failing hook is up, and serving.
    let ok = Setup::new(
        "hookok",
        Some(&root),
        "\n[hooks]\npost_create = \"true\"\n",
        true,
    );
    let started = ok.up(1).unwrap();
    assert_eq!(started[0].failed, None);
    let socket = ok.state().join("agents/a1/control.sock");
    let (code, _) = session(&socket, Path::new(PROJECT), "true");
    assert_eq!(code, Some(0));
    ok.cmd(commands::rm_all).unwrap();
    let _ = s.cmd(commands::rm_all);
}

/// A fleet whose cgroup subtree is gone (a reboot, or the end of the
/// systemd unit that delegated it) can still be listed, moved and removed.
/// Every command used to reopen the supervisor at the recorded root and
/// fail there, `rm --all` included, and `up` with the new root refused
/// because the root differed (adversarial review).
#[test]
fn a_fleet_whose_cgroup_subtree_is_gone_can_be_listed_moved_and_removed() {
    let Some(root) = capable_host() else { return };
    // A subtree of the delegated root to stand for the one that goes away.
    let gone = root.join(format!("devcroft-gone-{}", std::process::id()));
    std::fs::create_dir(&gone).unwrap();
    let s = Setup::new("gone", Some(&gone), "", true);
    let started = s.up(1).unwrap();
    assert_eq!(started[0].failed, None);

    // What a reboot or the unit's end does: everything in it killed, then
    // the cgroups removed, while the record still says Running.
    let mut dirs = vec![gone.clone()];
    let mut i = 0;
    while i < dirs.len() {
        for e in std::fs::read_dir(&dirs[i]).unwrap().flatten() {
            if e.path().is_dir() {
                dirs.push(e.path());
            }
        }
        i += 1;
    }
    for d in dirs.iter().rev() {
        let _ = std::fs::write(d.join("cgroup.kill"), "1");
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    for d in dirs.iter().rev() {
        std::fs::remove_dir(d).unwrap_or_else(|e| panic!("removing {}: {e}", d.display()));
    }

    // Listed: settled as Stopped, nothing to reach.
    let listed = s.cmd(commands::ls).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].record.state, AgentState::Stopped);
    assert!(s.cmd(|f| commands::inspect(f, "a1")).is_ok());

    // Moved: `up` under a root that exists, now that the recorded one is gone.
    let manifest = s.manifest();
    let moved = commands::up(
        &HostUsr,
        &UpRequest {
            manifest: &manifest,
            project_root: &s.project(),
            cgroup_root: &root,
            agents: 1,
            view: true,
            exe: exe(),
            authorized_key_pem: "",
            state_dir: &s.state(),
            limits: devcroft::fleet::cgroup::Limits::default(),
            fleet_limits: Default::default(),
            timeout: None,
            task: None,
        },
    )
    .unwrap();
    assert_eq!(moved.started[0].id, "a2", "IDs carry on across the move");

    // Removed, the moved agent and the stranded one alike.
    let removed = s.cmd(commands::rm_all).unwrap();
    assert_eq!(removed, ["a1", "a2"]);
    assert!(!s.state().exists());
    let _ = std::fs::remove_dir(root.join(format!("fleet-{}", s.name)));
}

#[test]
fn why_inside_an_agent_answers_from_that_agents_own_policy() {
    let Some(root) = capable_host() else { return };
    let s = Setup::new("why", Some(&root), "", true);
    let started = s.up(1).unwrap();
    let a = &started[0];
    let socket = s.state().join("agents").join(&a.id).join("control.sock");
    // The provider's grant (`/usr`, origin `provider:nix`) is only in the
    // compiled policy, never in the manifest: answered from the manifest
    // alone, as `why` in an agent was before it had an artifact, it is DENIED.
    let (code, out) = session(
        &socket,
        Path::new(PROJECT),
        &format!("{} why --path /usr/bin/env --op read 2>&1", exe().display()),
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("ALLOWED"), "{out}");
    assert!(out.contains("provider:nix"), "{out}");
    assert!(!out.contains("covers only"), "{out}");
    // The artifact holds the agent's paths as the agent sees them.
    let (code, out) = session(
        &socket,
        Path::new(PROJECT),
        &format!("{} policy --render 2>&1", exe().display()),
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains(WORKSPACE), "{out}");
    // `Setup`'s project is a subdirectory, so the clone root is granted, and
    // under its own origin: the artifact round-trips it (3.7).
    assert!(out.contains("fleet:clone"), "{out}");
    assert!(
        !out.contains(&a.workspace.to_string_lossy().into_owned()),
        "{out}"
    );
    s.cmd(|f| commands::stop(f, &a.id)).unwrap();
}

#[test]
fn the_fleets_own_limits_bound_all_its_agents_together() {
    use devcroft::fleet::cgroup::FleetLimits;
    let Some(root) = capable_host() else { return };
    if !Path::new("/usr/bin/python3").exists() {
        eprintln!("skipping: no /usr/bin/python3 to allocate with");
        return;
    }
    let s = Setup::new("cap", Some(&root), "", true);
    let cap = 384u64 << 20;
    let outcome = s
        .up_capped(
            2,
            FleetLimits {
                memory_max: Some(cap),
                pids_max: Some(10_000),
                cpu_millis: Some(1_500),
            },
        )
        .unwrap();
    let node = root.join(format!("fleet-{}", s.name));
    let read = |f: &str| {
        std::fs::read_to_string(node.join(f))
            .unwrap()
            .trim()
            .to_owned()
    };
    assert_eq!(read("memory.max"), cap.to_string());
    assert_eq!(read("memory.swap.max"), "0");
    assert_eq!(read("pids.max"), "10000");
    assert_eq!(read("cpu.max"), "150000 100000");
    // Every agent's leaf is under the node, so all of them are charged to
    // it (a helpers' leaf, made only for an agent with a proxy, is made by
    // the same `create_leaf`).
    for a in &outcome.started {
        assert!(node.join(&a.id).is_dir(), "{}", a.id);
    }

    // No per-agent limit at all, so only the fleet's can refuse this. a1
    // holds 150 MiB, which fits (the control); a2 then asks for 300 MiB,
    // which no agent's limit stops but the fleet has no room for.
    let alloc = |mib: u64| {
        format!(
            "/usr/bin/python3 -c 'import time; b = bytes([1]) * ({mib} << 20); \
             open(\"held\", \"w\").write(\"x\"); time.sleep(10)'"
        )
    };
    let socket = |id: &str| s.state().join("agents").join(id).join("control.sock");
    let a1 = &outcome.started[0];
    std::thread::scope(|scope| {
        let holder = scope.spawn(|| session(&socket("a1"), Path::new(PROJECT), &alloc(150)));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !a1.workspace.join("held").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the control allocation never completed"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let (code, out) = session(&socket("a2"), Path::new(PROJECT), &alloc(300));
        assert_ne!(
            code,
            Some(0),
            "300 MiB fit beside 150 MiB under a 384 MiB cap: {out}"
        );
        // The fleet's OOM kill can take either leaf: the control may be the
        // victim. What it cannot be is both allocations held at once.
        let _ = holder.join();
    });
    let events = read("memory.events");
    let oom_kills: u64 = events
        .lines()
        .find_map(|l| l.strip_prefix("oom_kill "))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    assert!(oom_kills > 0, "{events}");
    assert!(
        read("memory.peak").parse::<u64>().unwrap() <= cap,
        "{}",
        read("memory.peak")
    );
    let _ = s.cmd(commands::rm_all);

    // Kept for the fleet's next `up`, a limit given there replacing only
    // itself: a node recreated (after a reboot) does not run uncapped.
    let fresh = s
        .up_capped(
            1,
            FleetLimits {
                pids_max: Some(5_000),
                ..Default::default()
            },
        )
        .unwrap();
    // `rm --all` removed the fleet, so this is a new one: nothing kept, and
    // what was not given comes from the host (review §4.2), said to.
    let host = FleetLimits::host_defaults();
    let host_memory = host.memory_max.expect("this host reports MemTotal");
    let host_cpus = host.cpu_millis.expect("this host reports its CPUs");
    assert_eq!(fresh.defaulted, ["memory", "cpus"]);
    assert_eq!(read("memory.max"), host_memory.to_string());
    assert_eq!(read("cpu.max"), format!("{} 100000", host_cpus * 100));
    // Each agent's process budget has its own default, a fork-bomb guard.
    let a1_pids = std::fs::read_to_string(node.join("a1/pids.max")).unwrap();
    assert_eq!(a1_pids.trim(), "4096");
    // The next `up` keeps all three, the defaults included, and says none
    // is a default now: they are the fleet's.
    let again = s.up_capped(1, FleetLimits::default()).unwrap();
    assert!(again.defaulted.is_empty(), "{:?}", again.defaulted);
    assert_eq!(read("pids.max"), "5000");
    assert_eq!(read("memory.max"), host_memory.to_string());
    s.cmd(commands::rm_all).unwrap();
}

/// Review §4.2: the control plane shares an agent's process budget with its
/// workload. Below `PIDS_FLOOR` it cannot be counted on, so it is refused,
/// before anything is cloned; and the control plane's own footprint no
/// longer grows with the host's CPU count (the SSH server's runtime was one
/// worker per CPU: 16 pids idle on 12 CPUs, 19 with a session, measured).
#[test]
fn the_control_plane_fits_its_floor_and_a_budget_below_it_is_refused() {
    use devcroft::fleet::cgroup::{Limits, PIDS_FLOOR};
    let Some(root) = capable_host() else { return };
    let s = Setup::new("floor", Some(&root), "", true);
    let err = s
        .up_with(
            1,
            Limits {
                pids_max: Some(32),
                ..Limits::default()
            },
        )
        .unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert!(err.to_string().contains(&PIDS_FLOOR.to_string()), "{err}");
    assert!(!s.clones().exists());

    s.up(1).unwrap();
    let leaf = root.join(format!("fleet-{}", s.name)).join("a1");
    let socket = s.state().join("agents/a1/control.sock");
    let (code, out) = session(
        &socket,
        Path::new(PROJECT),
        "cat /sys/fs/cgroup/pids.current 2>/dev/null; true",
    );
    assert_eq!(code, Some(0), "{out}");
    let with_session: u64 = std::fs::read_to_string(leaf.join("pids.peak"))
        .or_else(|_| std::fs::read_to_string(leaf.join("pids.current")))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        with_session <= PIDS_FLOOR / 4,
        "an agent with one session open peaked at {with_session} pids"
    );
    s.cmd(commands::rm_all).unwrap();
}

/// Review §4.2: what happens when a workload consumes its budget. Its forks
/// are refused, which `inspect` reports while it runs; and the host side,
/// acting from outside the agent, still stops it.
#[test]
fn a_workload_at_its_budget_is_reported_and_still_stopped() {
    use devcroft::fleet::cgroup::Limits;
    let Some(root) = capable_host() else { return };
    let s = Setup::new("budget", Some(&root), "", true);
    s.up_with(
        1,
        Limits {
            pids_max: Some(64),
            ..Limits::default()
        },
    )
    .unwrap();
    s.cmd(|f| {
        commands::run(
            f,
            "a1",
            &[
                "sh".into(),
                "-c".into(),
                "for i in $(seq 200); do sleep 60 & done; wait".into(),
            ],
        )
    })
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let pressure = loop {
        let status = s.cmd(|f| commands::inspect(f, "a1")).unwrap();
        if let Some(p) = status.pressure.filter(|p| p.forks_denied > 0) {
            break p;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no refused fork was reported"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    assert!(pressure.forks_denied > 0);
    s.cmd(|f| commands::stop(f, "a1")).unwrap();
    let status = s.cmd(|f| commands::inspect(f, "a1")).unwrap();
    assert_eq!(status.record.state, AgentState::Stopped);
    assert_eq!(
        status.pressure, None,
        "a stopped agent has no live pressure"
    );
    s.cmd(commands::rm_all).unwrap();
}

/// The agent's latest task once it has ended, within `secs`.
fn task_end(s: &Setup, id: &str, secs: u64) -> devcroft::fleet::task::TaskRecord {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let status = s.cmd(|f| commands::inspect(f, id)).unwrap();
        let task = status.task.expect("a task was started");
        if task.outcome != devcroft::fleet::task::Outcome::Running {
            return task;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{} still running after {secs}s",
            task.id
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[test]
fn tasks_have_ids_observed_outcomes_and_logs() {
    use devcroft::fleet::task::Outcome;
    let Some(root) = capable_host() else { return };
    let s = Setup::new("task", Some(&root), "", true);
    let started = s
        .up_task(
            1,
            &["sh", "-c", "echo first; echo \"$DEVCROFT_TASK\"; exit 3"],
        )
        .unwrap()
        .started;
    assert_eq!(started[0].failed, None);
    assert_eq!(started[0].task.as_deref(), Some("t1"));

    // Observed: the code the keeper reported, and the output in the log.
    let t1 = task_end(&s, "a1", 15);
    assert_eq!(t1.outcome, Outcome::Exited { code: 3 });
    assert!(t1.ended_unix.is_some());
    let agent_dir = s.state().join("agents/a1");
    let log = std::fs::read_to_string(devcroft::fleet::task::log_path(&agent_dir, "t1")).unwrap();
    assert_eq!(log, "first\nt1\n");

    // The next task is t2; a second while it runs is refused, naming it.
    let long = ["sleep".to_string(), "30".to_string()];
    let t2 = s.cmd(|f| commands::run(f, "a1", &long)).unwrap();
    assert_eq!(t2.id, "t2");
    let err = s.cmd(|f| commands::run(f, "a1", &long)).unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert!(err.to_string().contains("t2"), "{err}");

    // Stopped mid-task: the task ended with its agent, and t1 is as it was.
    s.cmd(|f| commands::stop(f, "a1")).unwrap();
    let status = s.cmd(|f| commands::inspect(f, "a1")).unwrap();
    assert_eq!(status.task.unwrap().outcome, Outcome::EndedWithAgent);
    assert_eq!(
        devcroft::fleet::task::read(&agent_dir, "t1")
            .unwrap()
            .outcome,
        Outcome::Exited { code: 3 }
    );
    // And a stopped agent runs nothing.
    let err = s.cmd(|f| commands::run(f, "a1", &long)).unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");

    // devcroft's files in the clone are not the agent's to commit.
    let status = Command::new("git")
        .arg("-C")
        .arg(&started[0].workspace)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(
        started[0].workspace.join(".devcroft/a1").is_dir(),
        "the check needs devcroft's files to exist"
    );
    assert!(!status.contains(".devcroft"), "{status}");
    s.cmd(commands::rm_all).unwrap();
}

#[test]
fn an_agents_reports_are_shown_beside_what_was_observed() {
    use devcroft::fleet::report::{ClaimOutcome, escape};
    use devcroft::fleet::task::Outcome;
    let Some(root) = capable_host() else { return };
    let s = Setup::new("report", Some(&root), "", true);
    let agent = exe().display().to_string();
    // Reports from inside, under the agent's ordinary deny-by-default policy:
    // progress, attention (with an escape sequence in it), and a claim of
    // `done` from a task that then exits 1.
    let script = format!(
        "{agent} agent progress step 2 of 3 && \
         {agent} agent attention \"$(printf 'need a call\\033[2J')\" && \
         {agent} agent done all green && exit 1"
    );
    s.up_task(1, &["sh", "-c", &script]).unwrap();
    let t1 = task_end(&s, "a1", 15);
    let status = s.cmd(|f| commands::inspect(f, "a1")).unwrap();

    // Observed and reported, side by side: the exit is 1, the claim `done`.
    assert_eq!(t1.outcome, Outcome::Exited { code: 1 });
    let report = status.report.clone().unwrap().expect("the agent reported");
    assert_eq!(report.task.as_deref(), Some("t1"));
    assert_eq!(report.claim.as_ref().unwrap().outcome, ClaimOutcome::Done);
    assert_eq!(report.progress.as_ref().unwrap().message, "step 2 of 3");
    // The process that raised attention has exited; the flag has not.
    assert!(status.needs_attention());
    let message = &report.attention.as_ref().unwrap().message;
    assert_eq!(message, "need a call\x1b[2J", "kept verbatim");
    assert_eq!(escape(message), "need a call\\u{1b}[2J", "shown escaped");

    // A new task starts with no reports: t2 says nothing, so any attention
    // seen while it runs could only be t1's, carried over.
    let quiet = ["sleep".to_string(), "2".to_string()];
    let t2 = s.cmd(|f| commands::run(f, "a1", &quiet)).unwrap();
    let status = s.cmd(|f| commands::inspect(f, "a1")).unwrap();
    assert_eq!(status.task.as_ref().unwrap().id, t2.id);
    assert!(
        !status.needs_attention(),
        "t1's attention carried over to t2"
    );
    assert_eq!(status.report, Ok(None));
    task_end(&s, "a1", 15);

    // One that raises attention and clears it leaves none.
    s.cmd(|f| {
        commands::run(
            f,
            "a1",
            &[
                "sh".into(),
                "-c".into(),
                format!("{agent} agent attention again && {agent} agent attention --clear"),
            ],
        )
    })
    .unwrap();
    assert_eq!(task_end(&s, "a1", 15).outcome, Outcome::Exited { code: 0 });
    let status = s.cmd(|f| commands::inspect(f, "a1")).unwrap();
    assert!(!status.needs_attention());
    assert_eq!(status.report.unwrap().unwrap().task.as_deref(), Some("t3"));

    // Outside any sandbox there is nothing to report into.
    let out = Command::new(exe())
        .args(["agent", "attention", "from the host"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    s.cmd(commands::rm_all).unwrap();
}

/// The host's `/usr` as the closure, and one store path the environment
/// names: what the agent's lease must keep.
struct NamesAStorePath(String);

impl ProviderEntry for NamesAStorePath {
    fn resolve(&self, _: &Path) -> Result<Resolution, ProviderError> {
        Ok(Resolution {
            env: [
                ("PATH".to_owned(), "/usr/bin".to_owned()),
                ("AGENT_DATA".to_owned(), self.0.clone()),
            ]
            .into(),
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

/// Review §4.1: nothing kept a running agent's closure from the garbage
/// collector. **Named through a link, deliberately.** Nix already treats a
/// store path written in a live process's environment as a root (measured:
/// `--print-roots` lists it as `{censored}`), so a path the keeper's
/// environment names literally is kept while it runs, lease or none, and a
/// first version of this test passed with the lease removed. A path the
/// environment reaches only through a link, the way devbox's and flox's
/// profiles are reached, is kept by nothing but the lease.
#[test]
fn a_running_agents_closure_is_kept_from_the_garbage_collector() {
    let Some(root) = capable_host() else { return };
    if !devcroft::provider::host_can_build_nix_closures()
        || Command::new("nix-store").arg("--version").output().is_err()
    {
        eprintln!("skipping: no usable Nix store");
        return;
    }
    let s = Setup::new("gc", Some(&root), "", true);
    let file = s.base.join(format!("agent-data-{}", std::process::id()));
    std::fs::write(&file, "kept while the agent runs\n").unwrap();
    let out = Command::new("nix-store")
        .arg("--add")
        .arg(&file)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let link = s.base.join("profile");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let delete = || {
        let _ = Command::new("nix-store").args(["--delete", &path]).output();
        Path::new(&path).exists()
    };

    s.up_via(
        &NamesAStorePath(link.to_string_lossy().into_owned()),
        1,
        Default::default(),
    )
    .unwrap();
    assert!(
        delete(),
        "the collector removed a running agent's store path"
    );

    // Stopped, it runs nothing from the store: the lease ends.
    s.cmd(|f| commands::stop(f, "a1")).unwrap();
    assert!(
        !delete(),
        "a stopped agent still keeps its store path from the collector"
    );
    s.cmd(commands::rm_all).unwrap();
}
