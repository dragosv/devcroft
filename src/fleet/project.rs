//! From a project to an [`AgentLaunch`]: what `up` resolves for a sandbox,
//! resolved for one agent's workspace.
//!
//! **Per workspace, not per project.** A provider's environment names
//! paths inside the project it was resolved for (flox puts
//! `<project>/.flox/run/…` on `PATH`), and an agent's view contains its own
//! workspace, not the project it was cloned from. So each agent's
//! environment is resolved against its own workspace, once, host-side,
//! before anything is restricted. That is `up`'s two-phase rule applied per
//! agent.
//!
//! **Egress is `network.allow`, through the agent's own proxy**, and
//! nothing else. `network.default = "allow"` is refused: an agent's network
//! namespace has no route out, and that is the egress boundary (D9's
//! re-derivation), so unfiltered egress would mean giving it one.
//!
//! **Services run per agent** (`service-ports`): the provider's declared
//! services get a supervisor config in the agent's own workspace, named by
//! its ID, and the agent's keeper starts them inside its leaf and its
//! network namespace. So N agents each bind the same declared port.

use std::path::Path;

use super::cgroup::Limits;
use super::supervisor::AgentLaunch;
use crate::config::{Manifest, NetworkDefault};
use crate::provider::{ProviderEntry, ProviderError};

/// Why an agent could not be prepared, by the layer the error contract
/// names.
#[derive(Debug)]
pub enum PrepareError {
    /// The manifest asks for something fleet cannot provide yet.
    Config(String),
    Provider(ProviderError),
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrepareError::Config(msg) => write!(f, "config: {msg}"),
            PrepareError::Provider(e) => write!(f, "provider: {e}"),
        }
    }
}

impl std::error::Error for PrepareError {}

/// Resolve `manifest`'s environment for `workspace` and compile the agent's
/// policy, as `up` does for a sandbox. `exe` is the devcroft binary the
/// agent runs as its keeper; its directory is granted, as `up` grants it.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    provider: &dyn ProviderEntry,
    manifest: &Manifest,
    workspace: &Path,
    agent_id: &str,
    exe: &Path,
    authorized_key_pem: &str,
    limits: Limits,
    view: bool,
) -> Result<AgentLaunch, PrepareError> {
    if matches!(provider.static_name(), "mise" | "pixi") {
        return Err(PrepareError::Provider(ProviderError::ResolutionFailed(
            format!(
                "{} fleet runtime has not been qualified; use a native single sandbox",
                provider.static_name()
            ),
        )));
    }
    refuse_what_fleet_cannot_carry(manifest)?;

    let mut resolution = provider
        .resolve(workspace)
        .map_err(PrepareError::Provider)?;
    crate::services::check_declared_ports(
        &manifest.network.services,
        resolution.services.declared(),
    )
    .map_err(PrepareError::Config)?;
    // Exactly `up`'s rule: the shell must come from inside something the
    // sandbox is granted, and its grant is folded into the provider's.
    let shell =
        crate::shell::resolve(&resolution.env, &resolution.read_only_grants).ok_or_else(|| {
            PrepareError::Provider(ProviderError::ResolutionFailed(
                "no POSIX shell found in this environment or its closure, and the \
                 agent's keeper needs one; add one to the environment manifest \
                 (e.g. `flox install bash`)"
                    .to_string(),
            ))
        })?;
    let mut provider_grants = resolution.read_only_grants.clone();
    if let Some(grant) = &shell.grant
        && !provider_grants.contains(grant)
    {
        provider_grants.push(grant.clone());
    }
    // The closure, not the whole store (`fleet::closure`): what the
    // environment names, through its links, and the shell, with their
    // requisites. Degraded rather than refused when it cannot be computed,
    // since what it narrows is visibility, and said once rather than never.
    let mut roots = super::closure::roots(resolution.env.values());
    if let Some(root) = super::closure::root_of(&shell.path) {
        roots.insert(root);
    }
    let store_roots: Vec<String> = roots.iter().cloned().collect();
    let provider_grants = match super::closure::narrow(&provider_grants, &roots) {
        Ok(narrowed) => narrowed,
        Err(why) => {
            eprintln!(
                "devcroft fleet: warning: this agent's store view is not narrowed to its \
                 closure ({why}) (fallback: it sees all of /nix/store, read-only)"
            );
            provider_grants
        }
    };

    // `up`'s CA bundle rule (`crate::trust_store`), against the narrowed
    // grants: a bundle in the store is in them already, since the
    // environment names it and its roots are what the closure is built from.
    let trust_store = crate::trust_store::resolve(&resolution.env, &provider_grants);
    for line in &trust_store.refused {
        eprintln!("devcroft fleet: warning: {line}");
    }
    trust_store.rewrite(&mut resolution.env);

    // An agent has its own namespace, so nothing is allocated: a `var`-only
    // request's port is the one the provider declares, unchanged.
    let mut compiled = crate::policy::compile(manifest)
        .with_keeper_exe_grant(exe.to_string_lossy().into_owned())
        .with_provider_grants(provider.static_name(), &provider_grants)
        .with_trust_store(&trust_store.grants)
        .with_own_procfs()
        .with_service_ports(crate::lifecycle::ports::declared_ports(
            &manifest.network.services,
            resolution.services.declared(),
        ));

    // The agent's own service stack, as `up` prepares a sandbox's: the
    // config in the agent's workspace under its ID, and the supervisor's
    // socket granted (macOS needs it to bind; `up` grants it the same way).
    let declared = resolution.services.declared();
    if !declared.is_empty() {
        crate::services::write_config(
            workspace,
            agent_id,
            provider.static_name(),
            declared,
            &resolution.env,
            &shell.path,
        )
        .map_err(|e| match e {
            crate::services::ConfigError::SocketPathTooLong(m) => PrepareError::Config(m),
            crate::services::ConfigError::NoSupervisor(m) => {
                PrepareError::Provider(ProviderError::ResolutionFailed(m))
            }
            crate::services::ConfigError::Io(e) => {
                PrepareError::Provider(ProviderError::ResolutionFailed(e.to_string()))
            }
        })?;
        compiled = compiled.with_unix_socket_bind(
            crate::services::socket_path(workspace, agent_id)
                .to_string_lossy()
                .into_owned(),
            crate::policy::Origin::Baseline,
        );
    }
    let services: Vec<String> = declared.iter().map(|s| s.name.clone()).collect();

    // Before anything else is written for the agent.
    refuse_package_manager_authority(&compiled.to_capability_plan(), workspace)?;

    // devcroft's runtime and the namespace, as `up` builds them
    // (`add-own-userland` 4.3), in the agent's own workspace under its ID.
    let io =
        |e: std::io::Error| PrepareError::Provider(ProviderError::ResolutionFailed(e.to_string()));
    let artifact_dir = crate::services::artifact_dir(workspace, agent_id);
    let runtime_dir = artifact_dir.join(crate::runtime::DIR);
    crate::runtime::materialize(&artifact_dir, exe).map_err(io)?;
    crate::runtime::owned_dir::OwnedDir::open(&artifact_dir, crate::runtime::namespace::DIR)
        .map_err(io)?;
    for dir in ["home", crate::services::RUN_DIR] {
        crate::runtime::owned_dir::OwnedDir::open(&artifact_dir, dir).map_err(io)?;
    }
    compiled = compiled.with_runtime_dirs(&artifact_dir, crate::runtime::GENERATION, false);
    if !view {
        // Read-only is a mount of the view's; without one, nothing to claim.
        compiled.filesystem_write_deny.clear();
        compiled.filesystem_write_except.clear();
    }
    let plan = compiled.to_capability_plan();
    let grants = plan
        .resolved_grants(workspace)
        .map_err(|e| PrepareError::Config(e.to_string()))?;
    let roots: Vec<std::path::PathBuf> = grants.iter().map(|g| g.path.clone()).collect();
    let path = crate::lifecycle::up::sandbox_path(
        crate::lifecycle::up::project_path(&resolution.env, &resolution.unset, None, manifest)
            .as_deref(),
        shell.path.parent(),
        Some(&runtime_dir),
        &roots,
    );
    let namespace =
        crate::runtime::namespace::compute(&path, &shell.path, Some(&runtime_dir), &roots);
    crate::runtime::owned_dir::sync_links(
        &artifact_dir,
        crate::runtime::namespace::DIR,
        &namespace,
    )
    .map_err(io)?;
    let entries = crate::runtime::namespace::view_entries(&namespace, &roots);
    let runtime = crate::runtime::namespace::RuntimeView::new(&entries, &runtime_dir, exe, &plan);
    // Where shell and userland resolve for this agent
    // (`add-runtime-provenance` 1.4), with `up`'s function, from the
    // agent's own grants.
    let base_runtime = crate::lifecycle::up::base_runtime_record(
        &path,
        &grants,
        provider.static_name(),
        view.then_some(entries.as_slice()),
        &runtime_dir,
        provider.native(),
        // The redirect is macOS's and fleet is Linux's.
        None,
    );
    if let Some(warning) = base_runtime.host_warning() {
        eprintln!("devcroft fleet: warning: {warning}");
    }
    let policy = compiled.with_base_runtime(Some(base_runtime));

    // `up`'s order, which is load-bearing: the provider's activation script
    // prepares the environment that `post_create` usually depends on. Every
    // agent is new, so `post_create` always runs.
    let mut hooks: Vec<(&'static str, String)> = Vec::new();
    if let Some(script) = resolution.activation_script {
        hooks.push(("activation", script));
    }
    if let Some(cmd) = &manifest.hooks.post_create {
        hooks.push(("post_create", cmd.clone()));
    }
    if let Some(cmd) = &manifest.hooks.post_start {
        hooks.push(("post_start", cmd.clone()));
    }

    // `[env.vars]` and `[env] forward`, exactly as `up` applies them, over
    // the provider's environment. The services config above keeps the
    // provider's alone, as `up`'s does.
    let mut env = resolution.env;
    crate::lifecycle::apply_manifest_env(manifest, &mut env);
    // Last, as in `up`: the `PATH` the record and the namespace came from.
    env.insert("PATH".to_owned(), path);
    let mut unset = resolution.unset;
    unset.retain(|k| k != "PATH");

    Ok(AgentLaunch {
        workspace: workspace.to_path_buf(),
        plan,
        provider_env: env,
        unset,
        shell: shell.path,
        hooks,
        authorized_key_pem: authorized_key_pem.to_owned(),
        limits,
        view,
        egress_allow: manifest.network.allow.clone(),
        services,
        expose: manifest
            .network
            .services
            .iter()
            .filter(|(_, s)| s.expose)
            .filter_map(|(name, s)| Some((name.clone(), s.port?)))
            .collect(),
        // A property of the run, not of the manifest: `fleet up --timeout`.
        timeout: None,
        // Where the workspace sits in its repository is the caller's to say.
        repository: None,
        store_roots,
        provisioning_allow: manifest.provisioning.network.allow.clone(),
        policy: Some(policy),
        runtime,
    })
}

/// What a manifest can ask of `up` that fleet does not carry, refused by
/// name: an agent started without it would differ from the sandbox the same
/// manifest describes, and nothing would say so.
fn refuse_what_fleet_cannot_carry(manifest: &Manifest) -> Result<(), PrepareError> {
    if manifest.network.default == NetworkDefault::Allow {
        return Err(PrepareError::Config(
            "network.default = \"allow\": a fleet agent's egress goes through its own \
             proxy, to the hosts `network.allow` names, and nowhere else; its network \
             namespace has no route out, which is what makes that the boundary"
                .to_string(),
        ));
    }
    // Not carried rather than dropped. `up` forwards the agent socket with a
    // grant and the variable; in a fleet that is one person's SSH keys lent
    // to N unattended agents, each needing the socket inside its mount view,
    // and `up`'s own Linux half of it is unverified. A decision to make on
    // purpose, not a default to inherit.
    if manifest.ssh.forward_agent {
        return Err(PrepareError::Config(
            "ssh.forward_agent = true: fleet does not lend the SSH agent to its agents; \
             remove the key to start a fleet from this manifest"
                .to_string(),
        ));
    }
    Ok(())
}

/// `workspace-isolation`: an agent SHALL NOT receive a package-manager
/// daemon socket or a writable host-global store. The view mounts exactly
/// the plan's grants, so a grant is the only way either reaches an agent
/// with one, and a manifest asking for `/nix/var` (or `/`) or for `/nix/store`
/// read-write would hand every agent authority over the store they all share.
/// Checked on the resolved grants, so a symlink in the project naming
/// `/nix/var` is caught as well as the literal path.
///
/// What `--host-root` exposes without any grant is `fleet up`'s to refuse
/// (`commands::refuse_host_root_with_a_daemon`): there the socket is visible
/// whatever the plan says.
fn refuse_package_manager_authority(
    plan: &crate::policy::CapabilityPlan,
    workspace: &Path,
) -> Result<(), PrepareError> {
    let grants = plan
        .resolved_grants(workspace)
        .map_err(|e| PrepareError::Config(e.to_string()))?;
    use crate::policy::PackageManagerAuthority as Authority;
    match crate::policy::package_manager_authority(&grants) {
        Some(Authority::DaemonSocket { grant }) => Err(PrepareError::Config(format!(
            "a grant of {} would put the Nix daemon socket ({}) in every agent: \
             authority over the store all of them share, which an agent is never \
             given. Install what the agent needs through the environment's own \
             manifest instead; it is materialized before the agent starts",
            grant.display(),
            crate::provider::NIX_DAEMON_SOCKET
        ))),
        Some(Authority::WritableStore { grant }) => Err(PrepareError::Config(format!(
            "{} is granted read-write, which makes the Nix store writable from every \
             agent; agents get their toolchain read-only, so grant it read-only or \
             not at all",
            grant.display()
        ))),
        None => Ok(()),
    }
}
