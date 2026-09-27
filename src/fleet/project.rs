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
    refuse_what_fleet_cannot_carry(manifest)?;

    let resolution = provider
        .resolve(workspace)
        .map_err(PrepareError::Provider)?;
    crate::services::check_declared_ports(
        &manifest.network.services,
        resolution.services.declared(),
    )
    .map_err(PrepareError::Config)?;
    if let Some((name, _)) = manifest.network.services.iter().find(|(_, s)| s.expose) {
        return Err(PrepareError::Config(format!(
            "network.services.{name}.expose: host mapping for fleet agents is not \
             built yet (add-linux-agent-fleet 5.3)"
        )));
    }

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

    let exe_dir = exe.parent().ok_or_else(|| {
        PrepareError::Config(format!("{} has no parent directory", exe.display()))
    })?;
    let mut compiled = crate::policy::compile(manifest)
        .with_keeper_exe_grant(exe_dir.to_string_lossy().into_owned())
        .with_provider_grants(provider.static_name(), &provider_grants);

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
    let plan = compiled.to_capability_plan();

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

    Ok(AgentLaunch {
        workspace: workspace.to_path_buf(),
        plan,
        provider_env: resolution.env,
        unset: resolution.unset,
        shell: shell.path,
        hooks,
        authorized_key_pem: authorized_key_pem.to_owned(),
        limits,
        view,
        egress_allow: manifest.network.allow.clone(),
        services,
    })
}

fn refuse_what_fleet_cannot_carry(manifest: &Manifest) -> Result<(), PrepareError> {
    if manifest.network.default == NetworkDefault::Allow {
        return Err(PrepareError::Config(
            "network.default = \"allow\": a fleet agent's egress goes through its own \
             proxy, to the hosts `network.allow` names, and nowhere else; its network \
             namespace has no route out, which is what makes that the boundary"
                .to_string(),
        ));
    }
    Ok(())
}
