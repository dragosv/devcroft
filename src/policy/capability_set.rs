//! Projects [`CompiledPolicy`] into the `nono` library's `CapabilitySet` —
//! the process tier's self-restriction target (`use-nono-library` task
//! group 2) — via an intermediate [`CapabilityPlan`], because of two real
//! differences from own-policy-baseline's now-removed `to_nono_profile`
//! JSON projection, both forced by the library's own API rather than
//! chosen:
//!
//! - `allow_path`/`allow_file` require an absolute, *existing*,
//!   canonicalizable path — nono-cli's profile reader tolerated a
//!   `~`/project-relative form and a nonexistent path (silently dropping
//!   the grant, confirmed live via `why.rs`'s old `expand_home` module
//!   doc). This module resolves against `project_root`/`$HOME` and skips
//!   (never errors on) a grant that doesn't exist on this host — the same
//!   leniency `KEEPER_SYSTEM_READ`'s multiarch triplets already depend on
//!   (only one of `/lib/x86_64-linux-gnu` / `/lib/aarch64-linux-gnu`
//!   exists on a given host).
//! - There is no deny primitive at this layer at all. Landlock is purely
//!   additive; verified live that `nono-cli` itself refuses to start
//!   rather than attempt a deny nested inside a broader allow ("Landlock
//!   deny-overlap is not enforceable on Linux"). So `filesystem_deny`
//!   entries are never passed to the library — anything not explicitly
//!   granted is denied by Landlock's own default — and this module's one
//!   real job on the deny side is catching the overlap case at compile
//!   time instead of producing a sandbox with a silent hole in it.
//!
//! [`CapabilityPlan`] exists as its own type, separate from
//! [`CompiledPolicy`], because `up` computes the plan host-side (to
//! validate it — deny-overlap detection, task 2 above — before spawning
//! anything) but the *keeper* is the process that must actually build the
//! `CapabilitySet` and apply it to itself, across an exec boundary
//! (lifecycle spec: "The keeper restricts itself with no intermediate
//! process"). `CompiledPolicy` can't cross that boundary directly — its
//! `Origin` values carry `&'static str`, which has no `Deserialize` impl,
//! and origins exist for `policy --render`/`why` (host-side, run fresh
//! from the manifest each time) rather than for enforcement. `CapabilityPlan`
//! is the plain-value subset the keeper actually needs, serialized once
//! by `up` and handed down as an environment variable the same way the SSH
//! key material already crosses this boundary.

use super::CompiledPolicy;
use crate::paths::is_within;
use nono::{AccessMode, CapabilitySet, NetworkMode, NonoError, SignalMode};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The plain-value subset of [`CompiledPolicy`] the keeper needs to
/// restrict itself — see the module doc for why this exists separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPlan {
    pub filesystem_allow: Vec<String>,
    pub filesystem_read: Vec<String>,
    pub filesystem_deny: Vec<String>,
    pub network_block: bool,
    pub network_ports: Vec<u16>,
    pub signal_mode: String,
}

impl CompiledPolicy {
    /// Drop this compiled policy's origin annotations, keeping only what
    /// the keeper needs to actually restrict itself.
    pub fn to_capability_plan(&self) -> CapabilityPlan {
        CapabilityPlan {
            filesystem_allow: self
                .filesystem_allow
                .iter()
                .map(|a| a.value.clone())
                .collect(),
            filesystem_read: self
                .filesystem_read
                .iter()
                .map(|a| a.value.clone())
                .collect(),
            filesystem_deny: self
                .filesystem_deny
                .iter()
                .map(|a| a.value.clone())
                .collect(),
            network_block: self.network_block,
            network_ports: self.network_ports.iter().map(|p| p.value).collect(),
            signal_mode: self.signal_mode.to_string(),
        }
    }
}

#[derive(Debug)]
pub enum CapabilitySetError {
    /// A `filesystem.deny` rule (devcroft's own or the manifest's) falls
    /// inside a broader allow/read grant. Landlock cannot express this —
    /// see the module doc — so it is a compile-time error rather than a
    /// silently-ineffective deny.
    DenyOverlapsAllow { deny: String, allow: String },
    /// Building a capability for a granted path failed for a reason other
    /// than "it doesn't exist" (e.g. it resolved to a file where a
    /// directory was expected).
    Backend(NonoError),
}

impl std::fmt::Display for CapabilitySetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilitySetError::DenyOverlapsAllow { deny, allow } => write!(
                f,
                "'{deny}' is denied but falls inside the broader grant '{allow}' — Landlock \
                 cannot enforce a deny nested inside an allow; narrow the allow, remove the \
                 deny, or restructure the manifest so they don't overlap"
            ),
            CapabilitySetError::Backend(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CapabilitySetError {}

impl From<NonoError> for CapabilitySetError {
    fn from(e: NonoError) -> Self {
        CapabilitySetError::Backend(e)
    }
}

impl CapabilityPlan {
    /// Build the `nono::CapabilitySet` this plan describes, resolving
    /// `~`/project-relative path forms against `project_root` — see the
    /// module doc for why this differs from the (now-removed) JSON
    /// profile projection.
    pub fn to_capability_set(
        &self,
        project_root: &Path,
    ) -> Result<CapabilitySet, CapabilitySetError> {
        check_no_deny_overlaps_allow(self)?;

        let mut caps = CapabilitySet::new();
        for value in &self.filesystem_allow {
            caps = grant(caps, value, project_root, AccessMode::ReadWrite)?;
        }
        for value in &self.filesystem_read {
            caps = grant(caps, value, project_root, AccessMode::Read)?;
        }

        caps = if self.network_block {
            caps.block_network()
        } else {
            caps.set_network_mode(NetworkMode::AllowAll)
        };
        for port in &self.network_ports {
            caps = caps.allow_localhost_port(*port);
        }

        caps = caps.set_signal_mode(match self.signal_mode.as_str() {
            "isolated" => SignalMode::Isolated,
            other => unreachable!("CapabilityPlan::signal_mode has only one value: {other:?}"),
        });

        Ok(caps)
    }
}

/// Every `filesystem_deny` entry against every `filesystem_allow`/
/// `filesystem_read` entry — same `is_within` containment model `why.rs`
/// already uses for attribution, applied here to reject a conflict devcroft
/// cannot enforce rather than compile a sandbox that silently doesn't.
fn check_no_deny_overlaps_allow(plan: &CapabilityPlan) -> Result<(), CapabilitySetError> {
    for deny in &plan.filesystem_deny {
        for allow in plan.filesystem_allow.iter().chain(&plan.filesystem_read) {
            if deny != allow && is_within(deny, allow) {
                return Err(CapabilitySetError::DenyOverlapsAllow {
                    deny: deny.clone(),
                    allow: allow.clone(),
                });
            }
        }
    }
    Ok(())
}

fn grant(
    caps: CapabilitySet,
    value: &str,
    project_root: &Path,
    mode: AccessMode,
) -> Result<CapabilitySet, CapabilitySetError> {
    let resolved = resolve(value, project_root);
    if !resolved.exists() {
        // Tolerated, not an error — see the module doc's multiarch
        // KEEPER_SYSTEM_READ example. A grant for a path this host simply
        // doesn't have is a no-op, exactly like nono-cli's own profile
        // reader treats it.
        return Ok(caps);
    }
    Ok(if resolved.is_dir() {
        caps.allow_path(resolved, mode)?
    } else {
        caps.allow_file(resolved, mode)?
    })
}

/// `~`, `~/rest`, an absolute path, or a project-relative path (`.`,
/// `src`, ...) — the same four forms `paths::is_within` already
/// distinguishes, resolved here into a real filesystem path because
/// (unlike the JSON profile nono-cli used to read) the library's
/// `allow_path`/`allow_file` need one directly.
fn resolve(value: &str, project_root: &Path) -> PathBuf {
    let home = || PathBuf::from(std::env::var("HOME").unwrap_or_default());
    match value {
        "~" => home(),
        v => match v.strip_prefix("~/") {
            Some(rest) => home().join(rest),
            None if v.starts_with('/') => PathBuf::from(v),
            None => project_root.join(v),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;
    use crate::policy::compile;

    fn project_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "devcroft-capset-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn grants_the_project_root_read_write() {
        let root = project_dir();
        std::fs::create_dir_all(&root).unwrap();
        let (manifest, _) = parse("[sandbox]\nname = \"capsettest\"\n").unwrap();

        let caps = compile(&manifest)
            .to_capability_plan()
            .to_capability_set(&root)
            .unwrap();

        assert!(
            caps.fs_capabilities()
                .iter()
                .any(|c| c.resolved == root && c.access == AccessMode::ReadWrite)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nonexistent_grant_is_skipped_not_an_error() {
        let root = project_dir();
        std::fs::create_dir_all(&root).unwrap();
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "capsettest"
            [filesystem]
            allow = ["."]
            read = ["/this/path/does/not/exist/anywhere"]
            "#,
        )
        .unwrap();

        // Must not error: a nonexistent read grant is tolerated, matching
        // KEEPER_SYSTEM_READ's multiarch entries.
        let caps = compile(&manifest)
            .to_capability_plan()
            .to_capability_set(&root)
            .unwrap();
        assert!(
            !caps
                .fs_capabilities()
                .iter()
                .any(|c| c.original == std::path::Path::new("/this/path/does/not/exist/anywhere"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `is_within`'s containment model treats each root namespace (`~`,
    /// absolute, project-relative) separately — the same model `why.rs`
    /// already relies on for attribution, applied here unchanged so this
    /// check agrees with what `devcroft why` would say. Both sides must
    /// therefore be in the *same* namespace to register as overlapping,
    /// which is also the realistic case: devcroft's own baseline denials
    /// (`DEVCROFT_DATA_DIR`, `SENSITIVE_PATHS`) are `~`-relative, so the
    /// scenario this guards against is a manifest granting `~` broadly.
    #[test]
    fn deny_nested_inside_a_broader_allow_is_a_compile_error() {
        let root = project_dir();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let (manifest, _) = parse(
            "[sandbox]\nname = \"capsettest\"\n[filesystem]\nallow = [\".\"]\ndeny = [\"nested\"]\n",
        )
        .unwrap();

        let err = compile(&manifest)
            .to_capability_plan()
            .to_capability_set(&root)
            .unwrap_err();
        assert!(matches!(err, CapabilitySetError::DenyOverlapsAllow { .. }));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_round_trips_through_json() {
        let (manifest, _) = parse("[sandbox]\nname = \"capsettest\"\n").unwrap();
        let plan = compile(&manifest).to_capability_plan();
        let json = serde_json::to_string(&plan).unwrap();
        let back: CapabilityPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(plan, back);
    }
}
