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

    /// Where fleet clones: under the repository root, not the project.
    fn clones(&self) -> PathBuf {
        self.repo().join(".devcroft/fleet")
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
                timeout: None,
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
            Path::new(WORKSPACE),
            &format!("echo {} > mine; pwd", a.id),
        );
        assert_eq!(code, Some(0), "{out}");
        // Its workspace is at `/workspace`, whatever its host path.
        assert_eq!(out.trim(), WORKSPACE);
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
            timeout: None,
        },
    )
    .unwrap()
    .started;
    let agent = &started[0];
    let socket = state.join("agents").join(&agent.id).join("control.sock");
    let repo = env!("CARGO_MANIFEST_DIR");
    let (code, out) = session(
        &socket,
        Path::new(WORKSPACE),
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
            Path::new(WORKSPACE),
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
    let (code, out) = session(&socket, Path::new(WORKSPACE), "echo still-up");
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
            timeout: None,
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
            timeout: None,
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
            timeout: None,
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
