//! `fleet::project::prepare`: what `up` resolves for a sandbox, resolved
//! for one agent's workspace, through the provider injection seam.

#![cfg(target_os = "linux")]

use devcroft::fleet::cgroup::Limits;
use devcroft::fleet::project::{PrepareError, prepare};
use devcroft::provider::{
    ProviderEntry, ProviderError, Resolution, RestartPolicy, ServiceDecl, ServiceSupport, Shutdown,
    Tier,
};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A closure-tier stand-in whose closure is the host's `/usr` (merged `/usr`
/// makes that `sh` and the loader). Records the directory it was asked to
/// resolve for.
struct HostUsr {
    services: Vec<ServiceDecl>,
    resolved_for: Mutex<Option<PathBuf>>,
}

impl HostUsr {
    fn new() -> HostUsr {
        HostUsr {
            services: Vec::new(),
            resolved_for: Mutex::new(None),
        }
    }
}

impl ProviderEntry for HostUsr {
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError> {
        *self.resolved_for.lock().unwrap() = Some(project_root.to_path_buf());
        Ok(Resolution {
            env: [("PATH".to_owned(), "/usr/bin".to_owned())].into(),
            unset: Vec::new(),
            read_only_grants: vec!["/usr".to_owned()],
            activation_script: Some("echo activated > activation-ran".to_owned()),
            services: if self.services.is_empty() {
                ServiceSupport::Unsupported
            } else {
                ServiceSupport::Declared(self.services.clone())
            },
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

fn manifest(extra: &str) -> devcroft::config::Manifest {
    let text = format!(
        "[sandbox]\nname = \"fleet-project\"\n\n[env]\nprovider = \"nix\"\n\n\
         [hooks]\npost_create = \"echo created\"\npost_start = \"echo started\"\n{extra}"
    );
    devcroft::config::parse(&text).unwrap().0
}

fn exe() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_devcroft"))
}

#[test]
fn an_agent_gets_ups_plan_shell_and_hooks_for_its_own_workspace() {
    let provider = HostUsr::new();
    let workspace = Path::new("/tmp/some-agent-workspace");
    let launch = prepare(
        &provider,
        &manifest(""),
        workspace,
        "a1",
        exe(),
        "key",
        Limits::default(),
        true,
    )
    .unwrap();

    // Resolved against the agent's workspace, not some other root.
    assert_eq!(
        provider.resolved_for.lock().unwrap().as_deref(),
        Some(workspace)
    );
    // The provider's grant and the keeper binary's directory are both in
    // the plan, as `up` compiles them.
    assert!(launch.plan.filesystem_read.iter().any(|g| g == "/usr"));
    let exe_dir = exe().parent().unwrap().to_string_lossy().into_owned();
    assert!(
        launch.plan.filesystem_read.contains(&exe_dir),
        "{:?}",
        launch.plan.filesystem_read
    );
    // The shell comes from inside the provider's grant.
    assert!(
        launch.shell.starts_with("/usr"),
        "{}",
        launch.shell.display()
    );
    // `up`'s hook order: activation prepares what post_create relies on.
    let names: Vec<_> = launch.hooks.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["activation", "post_create", "post_start"]);
    assert_eq!(launch.workspace, workspace);
}

#[test]
fn egress_is_network_allow_through_a_proxy_and_never_unfiltered() {
    // `network.allow` becomes the agent's proxy allowlist.
    let launch = prepare(
        &HostUsr::new(),
        &manifest("[network]\nallow = [\"crates.io\"]\n"),
        Path::new("/tmp/w"),
        "a1",
        exe(),
        "key",
        Limits::default(),
        true,
    )
    .unwrap();
    assert_eq!(launch.egress_allow, ["crates.io"]);

    // Unfiltered egress would need a route out, and the route-less
    // namespace is the boundary: refused by name.
    let err = prepare(
        &HostUsr::new(),
        &manifest("[network]\ndefault = \"allow\"\n"),
        Path::new("/tmp/w"),
        "a1",
        exe(),
        "key",
        Limits::default(),
        true,
    )
    .unwrap_err();
    assert!(matches!(err, PrepareError::Config(_)), "{err}");
    assert!(err.to_string().contains("network.default"), "{err}");

    // Listener ports are inside the agent's own namespace, so they are fine.
    prepare(
        &HostUsr::new(),
        &manifest("[network]\nports = [5432]\n"),
        Path::new("/tmp/w"),
        "a1",
        exe(),
        "key",
        Limits::default(),
        true,
    )
    .unwrap();
}

#[test]
fn services_without_a_supervisor_in_the_environment_are_refused_by_name() {
    let mut provider = HostUsr::new();
    provider.services = vec![ServiceDecl {
        name: "postgres".into(),
        command: "postgres".into(),
        vars: Default::default(),
        is_daemon: false,
        working_dir: None,
        depends_on: Vec::new(),
        restart: RestartPolicy::Never,
        shutdown: Shutdown::Default,
        readiness: None,
    }];
    let workspace = std::env::temp_dir().join(format!("fleet-project-svc-{}", std::process::id()));
    std::fs::create_dir_all(&workspace).unwrap();
    let err = prepare(
        &provider,
        &manifest(""),
        &workspace,
        "a1",
        exe(),
        "key",
        Limits::default(),
        true,
    )
    .unwrap_err();
    let _ = std::fs::remove_dir_all(&workspace);
    // Services now run per agent, so what is refused is an environment that
    // cannot run them: at layer provider, with the fix named.
    assert!(matches!(err, PrepareError::Provider(_)), "{err}");
    assert!(err.to_string().contains("process-compose"), "{err}");
}

/// 5.8: a port declared for a service the environment does not declare is a
/// typo, and fails as one: at layer config, naming it and what exists,
/// before anything starts. A service that exists and then fails is a
/// different outcome (`ServicesOutcome::Failed`, in `tests/fleet_commands.rs`).
#[test]
fn a_port_for_a_service_that_does_not_exist_fails_naming_it() {
    let mut provider = HostUsr::new();
    provider.services = vec![ServiceDecl {
        name: "api".into(),
        command: "true".into(),
        vars: Default::default(),
        is_daemon: false,
        working_dir: None,
        depends_on: Vec::new(),
        restart: RestartPolicy::Never,
        shutdown: Shutdown::Default,
        readiness: None,
    }];
    let err = prepare(
        &provider,
        &manifest("[network.services.apl]\nport = 8710\n"),
        Path::new("/tmp/w"),
        "a1",
        exe(),
        "key",
        Limits::default(),
        true,
    )
    .unwrap_err();
    assert!(matches!(err, PrepareError::Config(_)), "{err}");
    let msg = err.to_string();
    assert!(
        msg.contains("'apl'") && msg.contains("declares: api"),
        "{msg}"
    );
}
