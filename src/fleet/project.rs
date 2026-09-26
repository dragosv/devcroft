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
//! **What fleet does not carry yet is refused by name**, never dropped:
//! egress (agents have no route out until group 3's per-agent proxy) and
//! services (group 5). A manifest asking for either would otherwise start
//! agents that silently lack what it declared.

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
pub fn prepare(
    provider: &dyn ProviderEntry,
    manifest: &Manifest,
    workspace: &Path,
    exe: &Path,
    authorized_key_pem: &str,
    limits: Limits,
    view: bool,
) -> Result<AgentLaunch, PrepareError> {
    refuse_what_fleet_cannot_carry(manifest)?;

    let resolution = provider
        .resolve(workspace)
        .map_err(PrepareError::Provider)?;
    if !resolution.services.declared().is_empty() {
        let names: Vec<_> = resolution
            .services
            .declared()
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        return Err(PrepareError::Config(format!(
            "this environment declares services ({}), and fleet agents cannot run \
             services yet (add-linux-agent-fleet group 5); use `devcroft up` for a \
             single sandbox with services",
            names.join(", ")
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
    let plan = crate::policy::compile(manifest)
        .with_keeper_exe_grant(exe_dir.to_string_lossy().into_owned())
        .with_provider_grants(provider.static_name(), &provider_grants)
        .to_capability_plan();

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
    })
}

fn refuse_what_fleet_cannot_carry(manifest: &Manifest) -> Result<(), PrepareError> {
    let no_egress = "fleet agents have no route out of their network namespace until \
                     each has its own egress proxy (add-linux-agent-fleet group 3)";
    if manifest.network.default == NetworkDefault::Allow {
        return Err(PrepareError::Config(format!(
            "network.default = \"allow\": {no_egress}"
        )));
    }
    if !manifest.network.allow.is_empty() {
        return Err(PrepareError::Config(format!(
            "network.allow = {:?}: {no_egress}",
            manifest.network.allow
        )));
    }
    Ok(())
}
