//! Compiles a [`Manifest`](crate::config::Manifest) plus baseline denials
//! into a nono profile: deterministic, and with every rule traceable back
//! to the manifest key, provider, or baseline default that produced it.

use crate::config::{Manifest, NetworkDefault};
use crate::paths::{SENSITIVE_PATHS, is_within};
use serde::Serialize;

/// devcroft's own data dir (client keypair, host keys). Always denied,
/// regardless of manifest contents — see the policy spec's "Baseline
/// denials" requirement.
const DEVCROFT_DATA_DIR: &str = "~/.local/share/devcroft";

const NONO_SCHEMA_URI: &str = "https://nono.sh/schemas/nono-profile.schema.json";

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
    /// Project down to the plain nono profile JSON (no origin metadata —
    /// origins are devcroft-internal and surfaced only via
    /// `policy --render`).
    pub fn to_nono_profile(&self) -> NonoProfile {
        NonoProfile {
            schema: NONO_SCHEMA_URI,
            meta: NonoMeta {
                name: self.sandbox_name.clone(),
            },
            filesystem: NonoFilesystem {
                allow: self.filesystem_allow.iter().map(|a| a.value.clone()).collect(),
                read: self.filesystem_read.iter().map(|a| a.value.clone()).collect(),
                deny: self.filesystem_deny.iter().map(|a| a.value.clone()).collect(),
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

        assert!(compiled.filesystem_deny.contains(&AnnotatedValue::new(
            DEVCROFT_DATA_DIR,
            Origin::Baseline
        )));
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
        assert!(compiled.filesystem_deny.contains(&AnnotatedValue::new(
            DEVCROFT_DATA_DIR,
            Origin::Baseline
        )));
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
    fn nono_profile_json_matches_expected_shape() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let json = compile(&manifest).to_nono_profile().to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["meta"]["name"], "myproj");
        assert_eq!(parsed["network"]["block"], true);
        assert!(parsed["filesystem"]["deny"].as_array().unwrap().len() >= 5);
    }
}
