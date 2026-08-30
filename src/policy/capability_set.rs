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
use std::io;
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
    /// See [`CompiledPolicy::network_proxy_port`]'s doc — `Some` only when
    /// `up` actually started the egress proxy for this sandbox.
    pub network_proxy_port: Option<u16>,
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
            network_proxy_port: self.network_proxy_port,
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
    /// A project-relative `filesystem.allow`/`filesystem.read` entry
    /// resolves, once symlinks are followed, to somewhere outside the
    /// project root. `is_within`'s containment model (and everything
    /// built on it — the sensitive-path warning, the baseline-deny-
    /// unless-granted check, `check_no_deny_overlaps_allow` above)
    /// compares the *lexical* manifest string, never the filesystem: a
    /// project-relative entry that is itself a symlink to `~/.ssh`
    /// passes every one of those checks looking like an ordinary
    /// in-project grant, then `nono::allow_path` canonicalizes and grants
    /// the real target. A compile-time error here, not a silent grant of
    /// whatever the symlink happens to point at — the manifest's own
    /// string is what a reviewer reads, and it must not lie about what
    /// gets granted, project-controlled dependency or not.
    SymlinkEscapesProjectRoot {
        value: String,
        canonical_target: PathBuf,
    },
    /// `canonicalize()` failed on a path `resolved.exists()` had just
    /// confirmed exists — a permission error on an intermediate
    /// directory, or a TOCTOU race with something removing it. Rare
    /// enough that a plain `io::Error` is honest about it rather than
    /// forcing a fabricated `NonoError` variant onto a failure that never
    /// reaches the library at all.
    Canonicalize { value: String, source: io::Error },
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
            CapabilitySetError::SymlinkEscapesProjectRoot {
                value,
                canonical_target,
            } => write!(
                f,
                "'{value}' resolves outside the project root once symlinks are followed \
                 (real target: {}) — a project-relative grant must stay inside the project; \
                 if this path outside the project is genuinely intended, grant it directly \
                 with an absolute path or a `~/...` form instead",
                canonical_target.display()
            ),
            CapabilitySetError::Canonicalize { value, source } => {
                write!(f, "resolving the real path of '{value}': {source}")
            }
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

        // `network_proxy_port` takes precedence over the plain block/
        // allow-all binary: it is only ever `Some` when `up` has already
        // started the egress proxy for this manifest's `network.allow`,
        // so the kernel gate here just has to match the process that
        // will actually make per-hostname decisions. See design.md's
        // Open Questions in add-egress-proxy for why `ProxyOnly` (a plain
        // Landlock `NetPort`/Seatbelt rule, confirmed live on this host's
        // ABI V6) is the right primitive rather than the seccomp-notify
        // path `apply_auto` reserves for pre-V4 kernels.
        caps = match self.network_proxy_port {
            Some(port) => caps.proxy_only(port),
            None if self.network_block => caps.block_network(),
            None => caps.set_network_mode(NetworkMode::AllowAll),
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
///
/// Includes an *exact* match between a deny and an allow entry, not just a
/// strictly nested one — `is_within(x, x)` is true by construction, and an
/// earlier version of this function excluded that case with `deny !=
/// allow`, on the reasoning (never written down, reconstructed by git
/// history and by checking which entries can ever produce an exact match
/// at all) that identical strings "aren't really overlapping". They are:
/// `DEVCROFT_DATA_DIR` is the only `filesystem_deny` entry `policy::compile`
/// ever pushes unconditionally (`SENSITIVE_PATHS` entries are *omitted* from
/// deny when granted, so they never reach this function carrying the same
/// string an allow entry does), so the only way `deny == allow` could ever
/// happen was a manifest granting `~/.local/share/devcroft` verbatim — and
/// the exclusion let that compile silently. Confirmed live: `policy
/// --render` showed the grant under `filesystem.allow`, the baseline deny
/// unchanged underneath it, `why` reported the path DENIED, and Landlock got
/// a real read-write grant to the directory holding this sandbox's
/// ephemeral SSH host key — the exact "baseline denials always win,
/// including devcroft's own data dir" invariant this project states as
/// load-bearing, silently violated by two checks each independently
/// assuming the other would catch it. `DEVCROFT_DATA_DIR`'s own doc comment
/// already says "never overridable by the manifest"; this is what makes
/// that literally true rather than aspirational.
fn check_no_deny_overlaps_allow(plan: &CapabilityPlan) -> Result<(), CapabilitySetError> {
    for deny in &plan.filesystem_deny {
        for allow in plan.filesystem_allow.iter().chain(&plan.filesystem_read) {
            if is_within(deny, allow) {
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
    let (resolved, is_project_relative) = resolve(value, project_root);
    if !resolved.exists() {
        // Tolerated, not an error — see the module doc's multiarch
        // KEEPER_SYSTEM_READ example. A grant for a path this host simply
        // doesn't have is a no-op, exactly like nono-cli's own profile
        // reader treats it.
        return Ok(caps);
    }
    // Symlink-escape guard (found by adversarial review, confirmed live:
    // a project-relative `credential-link -> ~/.ssh` granted the real
    // `~/.ssh` while every lexical check — the sensitive-path warning,
    // the baseline-deny-unless-granted rule, `check_no_deny_overlaps_
    // allow` — saw only an innocuous in-project string). Only applies to
    // project-relative entries: an explicit absolute or `~/...` grant
    // already names its target directly, canonicalization or not, so
    // there is no lexical/real divergence for a reviewer to be misled by.
    if is_project_relative {
        let canonical_target =
            resolved
                .canonicalize()
                .map_err(|source| CapabilitySetError::Canonicalize {
                    value: value.to_string(),
                    source,
                })?;
        let canonical_root =
            project_root
                .canonicalize()
                .map_err(|source| CapabilitySetError::Canonicalize {
                    value: ".".to_string(),
                    source,
                })?;
        if !canonical_target.starts_with(&canonical_root) {
            return Err(CapabilitySetError::SymlinkEscapesProjectRoot {
                value: value.to_string(),
                canonical_target,
            });
        }
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
/// `allow_path`/`allow_file` need one directly. The `bool` says whether
/// `value` took the project-relative branch — `grant`'s symlink-escape
/// guard only applies there: an explicit `~/...` or absolute entry
/// already names its target directly, so there is no lexical string for
/// a hidden symlink target to diverge from.
fn resolve(value: &str, project_root: &Path) -> (PathBuf, bool) {
    let home = || PathBuf::from(std::env::var("HOME").unwrap_or_default());
    match value {
        "~" => (home(), false),
        v => match v.strip_prefix("~/") {
            Some(rest) => (home().join(rest), false),
            None if v.starts_with('/') => (PathBuf::from(v), false),
            None => (project_root.join(v), true),
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

        // `resolved` is what `allow_path` canonicalized, so compare
        // against the canonical root: on macOS `temp_dir()` hands back a
        // `/var/...` path and `/var` is a symlink to `/private/var`.
        let canonical_root = root.canonicalize().unwrap();
        assert!(
            caps.fs_capabilities()
                .iter()
                .any(|c| c.resolved == canonical_root && c.access == AccessMode::ReadWrite)
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

    /// The traversal this check exists to close: a project-relative
    /// entry that is a symlink to somewhere outside the project must be
    /// rejected, not silently granted while `policy --render` shows it
    /// as an ordinary in-project string — confirmed live at the CLI
    /// level (a symlink to a scratch "credential" directory, granted
    /// despite reading as `credential-link` in the manifest) and here at
    /// the unit level so the guarantee doesn't depend on remembering to
    /// canonicalize at every caller.
    #[test]
    fn project_relative_symlink_escaping_the_project_root_is_rejected() {
        let root = project_dir();
        std::fs::create_dir_all(&root).unwrap();
        let outside = project_dir(); // a second, sibling scratch dir — not under `root`
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape-link")).unwrap();

        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "capsettest"
            [filesystem]
            allow = [".", "escape-link"]
            "#,
        )
        .unwrap();

        let err = compile(&manifest)
            .to_capability_plan()
            .to_capability_set(&root)
            .unwrap_err();
        assert!(
            matches!(err, CapabilitySetError::SymlinkEscapesProjectRoot { .. }),
            "expected a SymlinkEscapesProjectRoot error, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn project_relative_symlink_staying_inside_the_project_is_allowed() {
        let root = project_dir();
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("inside-link")).unwrap();

        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "capsettest"
            [filesystem]
            allow = [".", "inside-link"]
            "#,
        )
        .unwrap();

        compile(&manifest)
            .to_capability_plan()
            .to_capability_set(&root)
            .unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_explicit_home_grant_is_never_treated_as_an_escape() {
        // `~/.ssh` names its target directly — canonicalization of an
        // explicit `~`/absolute entry is not this check's concern, and
        // must not become one: the sensitive-path warning is what
        // governs those, not this guard.
        let root = project_dir();
        std::fs::create_dir_all(&root).unwrap();
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "capsettest"
            [filesystem]
            allow = [".", "~/.ssh"]
            "#,
        )
        .unwrap();

        // Must not error even though `~/.ssh` is outside `root` by
        // construction — that's the whole point of an explicit grant.
        compile(&manifest)
            .to_capability_plan()
            .to_capability_set(&root)
            .unwrap();

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

    /// The exact-match case this function used to exclude with `deny !=
    /// allow`: `policy::compile` pushes `DEVCROFT_DATA_DIR`
    /// ("~/.local/share/devcroft") into `filesystem_deny`
    /// unconditionally, so a manifest granting that exact path verbatim
    /// used to produce `deny == allow` and slip past this check entirely
    /// — the one entry `compile`'s own doc comment says is "never
    /// overridable by the manifest". Confirmed live before this fix: the
    /// grant compiled, `why --path ~/.local/share/devcroft --op
    /// readwrite` reported it DENIED as baseline, and Landlock got a real
    /// read-write grant to the directory holding the sandbox's ephemeral
    /// SSH host key. Must now fail identically to the nested case above.
    #[test]
    fn exact_match_between_deny_and_allow_is_also_a_compile_error() {
        let root = project_dir();
        std::fs::create_dir_all(&root).unwrap();
        let (manifest, _) = parse(
            "[sandbox]\nname = \"capsettest\"\n[filesystem]\nallow = [\".\", \"~/.local/share/devcroft\"]\n",
        )
        .unwrap();

        let err = compile(&manifest)
            .to_capability_plan()
            .to_capability_set(&root)
            .unwrap_err();
        assert!(
            matches!(err, CapabilitySetError::DenyOverlapsAllow { .. }),
            "expected a DenyOverlapsAllow error, got: {err}"
        );
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
