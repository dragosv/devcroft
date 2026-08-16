//! Compiles a [`Manifest`](crate::config::Manifest) plus baseline denials
//! into a nono profile: deterministic, and with every rule traceable back
//! to the manifest key, provider, or baseline default that produced it.

mod degraded;
mod render;
mod why;

pub use degraded::{DegradedCapability, detect as detect_degraded};
pub use render::render;
pub use why::{Explanation, Op, WhyError, why_host, why_path};

use crate::config::{Manifest, NetworkDefault};
use crate::paths::{SENSITIVE_PATHS, is_within};
use serde::Serialize;

/// devcroft's own data dir (client keypair, host keys). Always denied,
/// regardless of manifest contents — see the policy spec's "Baseline
/// denials" requirement.
const DEVCROFT_DATA_DIR: &str = "~/.local/share/devcroft";

const NONO_SCHEMA_URI: &str = "https://nono.sh/schemas/nono-profile.schema.json";

/// Every compiled profile extends nono's own `default` profile, which is
/// where standard system read access (dynamic linker paths, `/usr`,
/// `/bin`, ...) and its dangerous-command blocklist live. Without it a
/// profile fed to nono can't exec anything at all — confirmed against a
/// live nono 0.71.0: a from-scratch manifest with no `extends` denies
/// even `/usr/bin/cat` (EPERM), and this shape is only accepted via
/// `nono wrap -p <path>` (the named-profile schema, which supports
/// `extends`), not `-c/--config` (a stricter, unrelated "capability
/// manifest" schema requiring its own `version` field). `up` (task 4.2)
/// must invoke nono with `-p`, never `-c`.
const NONO_BASELINE_PROFILE: &str = "default";

/// Where a compiled rule came from, rendered as `manifest:<key>`,
/// `provider:<name>`, or `baseline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Manifest(&'static str),
    Provider(&'static str),
    Baseline,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Manifest(key) => write!(f, "manifest:{key}"),
            Origin::Provider(name) => write!(f, "provider:{name}"),
            Origin::Baseline => write!(f, "baseline"),
        }
    }
}

/// A single compiled value (path or domain) paired with the rule that
/// produced it, for `policy --render` and `why` to trace back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotatedValue {
    pub value: String,
    pub origin: Origin,
}

impl AnnotatedValue {
    fn new(value: impl Into<String>, origin: Origin) -> Self {
        AnnotatedValue {
            value: value.into(),
            origin,
        }
    }
}

/// The manifest compiled into policy rules, still carrying origin
/// annotations. [`CompiledPolicy::to_nono_profile`] projects this down to
/// the plain JSON nono consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPolicy {
    pub sandbox_name: String,
    pub filesystem_allow: Vec<AnnotatedValue>,
    pub filesystem_read: Vec<AnnotatedValue>,
    pub filesystem_deny: Vec<AnnotatedValue>,
    pub network_block: bool,
    pub network_allow_domain: Vec<AnnotatedValue>,
}

/// Compile `manifest` plus baseline denials into a [`CompiledPolicy`].
///
/// Deterministic: identical manifests always produce identically-ordered
/// output, since manifest lists preserve TOML order and baseline entries
/// are appended in the fixed order of [`SENSITIVE_PATHS`].
pub fn compile(manifest: &Manifest) -> CompiledPolicy {
    let filesystem_allow: Vec<AnnotatedValue> = manifest
        .filesystem
        .allow
        .iter()
        .map(|p| AnnotatedValue::new(p.clone(), Origin::Manifest("filesystem.allow")))
        .collect();
    let filesystem_read: Vec<AnnotatedValue> = manifest
        .filesystem
        .read
        .iter()
        .map(|p| AnnotatedValue::new(p.clone(), Origin::Manifest("filesystem.read")))
        .collect();
    let mut filesystem_deny: Vec<AnnotatedValue> = manifest
        .filesystem
        .deny
        .iter()
        .map(|p| AnnotatedValue::new(p.clone(), Origin::Manifest("filesystem.deny")))
        .collect();

    // devcroft's own data dir: never overridable by the manifest.
    filesystem_deny.push(AnnotatedValue::new(DEVCROFT_DATA_DIR, Origin::Baseline));

    // Known credential dirs: baseline-denied unless the manifest already
    // granted them (in which case config::validate has already warned).
    let granted: Vec<&str> = manifest
        .filesystem
        .allow
        .iter()
        .chain(manifest.filesystem.read.iter())
        .map(String::as_str)
        .collect();
    for sensitive in SENSITIVE_PATHS {
        if !granted.iter().any(|g| is_within(sensitive, g)) {
            filesystem_deny.push(AnnotatedValue::new(*sensitive, Origin::Baseline));
        }
    }

    let network_allow_domain: Vec<AnnotatedValue> = manifest
        .network
        .allow
        .iter()
        .map(|d| AnnotatedValue::new(d.clone(), Origin::Manifest("network.allow")))
        .collect();

    CompiledPolicy {
        sandbox_name: manifest.sandbox.name.clone(),
        filesystem_allow,
        filesystem_read,
        filesystem_deny,
        network_block: manifest.network.default == NetworkDefault::Deny,
        network_allow_domain,
    }
}

impl CompiledPolicy {
    /// Fold in the read-only store grants a provider resolved at the last
    /// `up` (recorded in `lifecycle::state::Meta`), tagged with the given
    /// provider name's origin. Deliberately not part of [`compile`]: that
    /// function's own doc comment guarantees it is a pure, deterministic
    /// function of the manifest alone, and a provider's grants can only be
    /// known by actually running the provider (`flox activate`/`nix
    /// develop`) — something `compile` never does and `policy --render`/
    /// `why` still don't do live. This just merges in whatever was
    /// recorded the last time something *did* run it, so those commands
    /// can show grants that are otherwise invisible to them (found while
    /// implementing add-nix-provider: `Origin::Provider` existed but had
    /// no caller — `policy --render` never showed a provider's store
    /// grants for any provider, flox included).
    pub fn with_provider_grants(mut self, provider: &'static str, grants: &[String]) -> Self {
        self.filesystem_read.extend(
            grants
                .iter()
                .map(|g| AnnotatedValue::new(g.clone(), Origin::Provider(provider))),
        );
        self
    }

    /// Project down to the plain nono profile JSON (no origin metadata —
    /// origins are devcroft-internal and surfaced only via
    /// `policy --render`).
    pub fn to_nono_profile(&self) -> NonoProfile {
        NonoProfile {
            schema: NONO_SCHEMA_URI,
            extends: NONO_BASELINE_PROFILE,
            meta: NonoMeta {
                name: self.sandbox_name.clone(),
            },
            filesystem: NonoFilesystem {
                allow: self
                    .filesystem_allow
                    .iter()
                    .map(|a| a.value.clone())
                    .collect(),
                read: self
                    .filesystem_read
                    .iter()
                    .map(|a| a.value.clone())
                    .collect(),
                deny: self
                    .filesystem_deny
                    .iter()
                    .map(|a| a.value.clone())
                    .collect(),
            },
            network: NonoNetwork {
                block: self.network_block,
                allow_domain: self
                    .network_allow_domain
                    .iter()
                    .map(|a| a.value.clone())
                    .collect(),
            },
        }
    }
}

/// The subset of nono's profile schema devcroft emits. Field names and
/// shapes match `nono profile schema` exactly so the output validates
/// against nono's own JSON Schema unmodified.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NonoProfile {
    #[serde(rename = "$schema")]
    pub schema: &'static str,
    pub extends: &'static str,
    pub meta: NonoMeta,
    pub filesystem: NonoFilesystem,
    pub network: NonoNetwork,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NonoMeta {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NonoFilesystem {
    pub allow: Vec<String>,
    pub read: Vec<String>,
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NonoNetwork {
    pub block: bool,
    pub allow_domain: Vec<String>,
}

impl NonoProfile {
    /// Serialize deterministically (fixed struct field order, no map
    /// iteration) for `<state>/<name>/profile.json`.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("NonoProfile serialization is infallible")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    #[test]
    fn minimal_manifest_denies_data_dir_and_credentials() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = compile(&manifest);

        assert!(
            compiled
                .filesystem_deny
                .contains(&AnnotatedValue::new(DEVCROFT_DATA_DIR, Origin::Baseline))
        );
        for sensitive in SENSITIVE_PATHS {
            assert!(
                compiled
                    .filesystem_deny
                    .contains(&AnnotatedValue::new(*sensitive, Origin::Baseline)),
                "expected baseline deny for {sensitive}"
            );
        }
    }

    #[test]
    fn explicitly_granted_credential_dir_is_not_baseline_denied() {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = ["~/.ssh"]
            "#,
        )
        .unwrap();
        let compiled = compile(&manifest);

        assert!(
            !compiled
                .filesystem_deny
                .iter()
                .any(|d| d.value == "~/.ssh" && d.origin == Origin::Baseline)
        );
        // Data dir denial is unconditional regardless of what's granted.
        assert!(
            compiled
                .filesystem_deny
                .contains(&AnnotatedValue::new(DEVCROFT_DATA_DIR, Origin::Baseline))
        );
    }

    #[test]
    fn manifest_rules_carry_manifest_origin() {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = ["src"]
            [network]
            default = "allow"
            allow = ["github.com"]
            "#,
        )
        .unwrap();
        let compiled = compile(&manifest);

        assert!(compiled.filesystem_allow.contains(&AnnotatedValue::new(
            "src",
            Origin::Manifest("filesystem.allow")
        )));
        assert!(compiled.network_allow_domain.contains(&AnnotatedValue::new(
            "github.com",
            Origin::Manifest("network.allow")
        )));
        assert!(!compiled.network_block);
    }

    /// add-hardened-tier design.md decision 4: `compile`/`render`/`why`
    /// never look at `[sandbox].isolation` — they operate on
    /// `CompiledPolicy` before any tier-specific projection exists, so
    /// the same manifest compiles identically regardless of tier. This
    /// pins that down as a regression test rather than trusting it.
    #[test]
    fn compilation_is_identical_regardless_of_isolation_tier() {
        let (process_manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let (hardened_manifest, _) =
            parse("[sandbox]\nname = \"myproj\"\nisolation = \"hardened\"\n").unwrap();

        assert_eq!(
            compile(&process_manifest),
            compile(&hardened_manifest),
            "CompiledPolicy must not depend on the isolation tier"
        );
        assert_eq!(
            render(&compile(&process_manifest)),
            render(&compile(&hardened_manifest))
        );
    }

    #[test]
    fn compilation_is_deterministic() {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = ["src", "docs"]
            "#,
        )
        .unwrap();

        let a = compile(&manifest).to_nono_profile().to_json();
        let b = compile(&manifest).to_nono_profile().to_json();
        assert_eq!(a, b);
    }

    #[test]
    fn with_provider_grants_tags_grants_with_provider_origin() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = compile(&manifest).with_provider_grants("nix", &["/nix/store".to_string()]);

        assert!(
            compiled
                .filesystem_read
                .contains(&AnnotatedValue::new("/nix/store", Origin::Provider("nix")))
        );
    }

    /// Spec: "Provider does not weaken the sandbox" — a provider's grants
    /// can only ever land in `filesystem_read`, never `filesystem_allow`
    /// (write). `Resolution` (provider/mod.rs) has no field through which
    /// a provider implementation could even express a write grant, and
    /// `with_provider_grants` only ever appends to `filesystem_read` — this
    /// pins that structural guarantee down as a test, for both providers
    /// sharing the same merge path (flox and nix alike).
    #[test]
    fn with_provider_grants_never_touches_filesystem_allow() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let before = compile(&manifest);
        let after = before.clone().with_provider_grants(
            "nix",
            &["/nix/store".to_string(), "/some/other".to_string()],
        );

        assert_eq!(before.filesystem_allow, after.filesystem_allow);
    }

    #[test]
    fn with_provider_grants_leaves_manifest_rules_untouched() {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            read = ["docs"]
            "#,
        )
        .unwrap();
        let compiled = compile(&manifest).with_provider_grants("flox", &["/nix/store".to_string()]);

        assert!(compiled.filesystem_read.contains(&AnnotatedValue::new(
            "docs",
            Origin::Manifest("filesystem.read")
        )));
        assert!(
            compiled
                .filesystem_read
                .contains(&AnnotatedValue::new("/nix/store", Origin::Provider("flox")))
        );
    }

    #[test]
    fn nono_profile_json_matches_expected_shape() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let json = compile(&manifest).to_nono_profile().to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["meta"]["name"], "myproj");
        assert_eq!(parsed["extends"], "default");
        assert_eq!(parsed["network"]["block"], true);
        assert!(parsed["filesystem"]["deny"].as_array().unwrap().len() >= 5);
    }

    /// Best-effort: only runs where `nono` is installed (the devcontainer
    /// provides it). Exercises the exact integration gap that motivated
    /// `extends` above — a profile nono's own validator accepts and that
    /// can actually execute something, not just JSON that looks plausible.
    #[test]
    fn compiled_profile_validates_and_executes_under_real_nono() {
        if std::process::Command::new("nono")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("devcroft-policy-nono-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), "hi\n").unwrap();

        let (manifest, _) = parse(&format!(
            "[sandbox]\nname = \"nonocheck\"\n[filesystem]\nallow = [{:?}]\n",
            dir.to_str().unwrap()
        ))
        .unwrap();
        let profile_path = dir.join("profile.json");
        std::fs::write(
            &profile_path,
            compile(&manifest).to_nono_profile().to_json(),
        )
        .unwrap();

        let validate = std::process::Command::new("nono")
            .arg("profile")
            .arg("validate")
            .arg(&profile_path)
            .output()
            .unwrap();
        assert!(
            validate.status.success(),
            "nono profile validate failed: {}",
            String::from_utf8_lossy(&validate.stderr)
        );

        let run = std::process::Command::new("nono")
            .arg("wrap")
            .arg("--silent")
            .arg("-p")
            .arg(&profile_path)
            .arg("--")
            .arg("cat")
            .arg(dir.join("f.txt"))
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "nono wrap -p <profile> failed: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&run.stdout), "hi\n");
    }
}
