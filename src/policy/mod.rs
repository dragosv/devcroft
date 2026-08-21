//! Compiles a [`Manifest`](crate::config::Manifest) plus baseline denials
//! into a nono profile: deterministic, and with every rule traceable back
//! to the manifest key, provider, or baseline default that produced it.

mod degraded;
mod render;
mod why;

pub use degraded::{DegradedCapability, detect as detect_degraded};
pub use render::{render, render_backend_enforced};
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

/// Backend policy groups devcroft declines via `groups.exclude` (own-
/// policy-baseline Decisions 2–3). nono injects its full 18-group set into
/// any profile that doesn't say otherwise — `extends` contributes only
/// `signal_mode`, confirmed with `nono profile diff` — so declining a
/// group is the only lever that removes rules, and it must be named
/// explicitly rather than achieved by omission.
///
/// `system_read_linux_core` / `system_read_macos`: devcroft is closure-
/// tier by design (`docs/decisions.md` §1) — a project's toolchain comes
/// from the provider's store, never the host, so host `/usr/bin`, `/lib`,
/// `/usr/share` read access is exactly the passthrough that qualification
/// test rejects. [`KEEPER_SYSTEM_READ`] grants back only what devcroft's
/// own host-linked keeper needs, with `Origin::Baseline`.
///
/// `dangerous_commands*`: verified inert under `nono wrap` (`rm`, `cp`
/// both succeeded with the group active) — `deny.commands` needs nono's
/// resident supervisor, which `wrap` does not provide. Emitting it would
/// claim a protection that isn't enforced, which the cli spec's "sandbox
/// does not claim protections it does not apply" requirement forbids.
const GROUPS_EXCLUDE: &[&str] = &[
    "system_read_linux_core",
    "system_read_macos",
    "dangerous_commands",
    "dangerous_commands_linux",
    "dangerous_commands_macos",
];

/// The 18 groups nono injects into every profile, minus [`GROUPS_EXCLUDE`]
/// — what actually reaches the backend that devcroft neither compiles nor
/// can remove, rendered by `policy --render` with `Origin::BackendEnforced`
/// (own-policy-baseline Decision 5). The first eight are the required deny
/// groups (`nono profile validate` refuses to exclude them); the remaining
/// five are optional groups this change leaves alone on the merits stated
/// in design.md ("Decision 2" scopes the exclusion to system-read access
/// specifically) — narrow, host-specific conveniences (`/tmp`, `/dev`
/// device writes, a handful of `~/.local`/Homebrew paths), not the broad
/// `/usr/bin`-shaped passthrough `GROUPS_EXCLUDE` targets.
pub(crate) const BACKEND_ENFORCED_GROUPS: &[&str] = &[
    "deny_credentials",
    "deny_keychains_macos",
    "deny_keychains_linux",
    "deny_browser_data_macos",
    "deny_browser_data_linux",
    "deny_macos_private",
    "deny_shell_history",
    "deny_shell_configs",
    "system_write_macos",
    "system_write_linux",
    "user_tools",
    "homebrew_macos",
    "homebrew_linux",
];

/// Explicit read grants replacing what `system_read_linux_core` /
/// `system_read_macos` used to supply implicitly — sized for the
/// host-linked keeper binary's own needs (dynamic linker, libc, entropy),
/// not for project code, which gets its toolchain from the provider's
/// closure and is granted nothing here.
///
/// Linux: mirrors the multiarch triplets nono's own group grants
/// unconditionally regardless of host arch (`/lib/x86_64-linux-gnu` and
/// `/lib/aarch64-linux-gnu` both, verified via `nono profile groups
/// system_read_linux_core --json`) rather than detecting the host triplet,
/// since a wrong guess would silently fail to load the keeper on whichever
/// arch it guessed wrong for. `/dev/urandom` and `/dev/null`: exercised
/// continuously, not just at startup — every inbound SSH key exchange the
/// keeper's embedded russh server performs after restriction draws from
/// it.
#[cfg(not(target_os = "macos"))]
const KEEPER_SYSTEM_READ: &[&str] = &[
    "/lib",
    "/lib64",
    "/lib/x86_64-linux-gnu",
    "/lib/aarch64-linux-gnu",
    "/usr/lib",
    "/usr/lib64",
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
    "/etc/ld.so.cache",
    "/dev/urandom",
    "/dev/null",
    // `devcroft shell`/pty sessions (`keeper::pty::open_pty`, `libc::openpty`)
    // need this: glibc's `openpty` opens `/dev/ptmx`, which on this host (and
    // every Linux system checked) is itself a symlink to `/dev/pts/ptmx` —
    // Landlock evaluates the resolved target, so granting the directory
    // covers it, the same way the group this replaces did (it granted
    // `/dev/pts` but no standalone `/dev/ptmx` entry either). Without this,
    // `open_pty` fails before `Command::new(&req.cmd)` ever runs, surfacing
    // as an opaque "keeper refused to spawn" with nothing shell-related in
    // it — found via a live `devcroft shell` session, not by inspection.
    "/dev/pts",
];

/// macOS equivalent of [`KEEPER_SYSTEM_READ`]. Not live-verified against a
/// running Seatbelt keeper the way the Linux list is (this repo's
/// devcontainer is Linux-only) — derived from `nono profile groups
/// system_read_macos --json`'s own path list, narrowed to what a
/// dynamically-linked binary and its runtime crypto need rather than the
/// full 35-entry group. Revisit against a real macOS run before relying on
/// it.
#[cfg(target_os = "macos")]
const KEEPER_SYSTEM_READ: &[&str] = &[
    "/usr/lib",
    "/System/Library",
    "/private/var/db/dyld",
    "/var/db/dyld",
    "/dev/urandom",
    "/dev/null",
    // `devcroft shell`'s pty allocation — macOS's `/dev/ptmx` equivalent.
    // Not live-verified (see this const's own doc comment).
    "/dev/ptmx",
];

/// The signal isolation `extends: "default"` currently supplies as its
/// *only* effective contribution (own-policy-baseline design.md Decision
/// 4) — declared explicitly so it survives independently of `extends`
/// rather than being the single most easily lost property in the policy.
const SIGNAL_MODE: &str = "isolated";

/// Where a compiled rule came from, rendered as `manifest:<key>`,
/// `provider:<name>`, `baseline`, or `backend:<group>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Manifest(&'static str),
    Provider(&'static str),
    Baseline,
    /// A rule the backend enforces unconditionally — one of the eight
    /// `required` deny groups nono refuses to exclude. Distinct from
    /// `Baseline`: devcroft neither chose these nor can remove them, so
    /// attributing them to devcroft's own baseline would misstate who is
    /// responsible (own-policy-baseline design.md Decision 5).
    BackendEnforced(String),
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Manifest(key) => write!(f, "manifest:{key}"),
            Origin::Provider(name) => write!(f, "provider:{name}"),
            Origin::Baseline => write!(f, "baseline"),
            Origin::BackendEnforced(group) => write!(f, "backend:{group}"),
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

/// A compiled loopback port paired with the rule that produced it — the
/// same role [`AnnotatedValue`] plays for paths and domains. A separate
/// type rather than making that one generic: ports are the only
/// non-string rule devcroft compiles, and a type parameter would ripple
/// through every existing call site to buy nothing here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotatedPort {
    pub value: u16,
    pub origin: Origin,
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
    /// Loopback TCP ports the sandbox may bind and connect on. Governed
    /// by `network.ports`, independently of `network_block` — the two
    /// answer different questions (outbound egress vs local listeners),
    /// which is why a deny-all sandbox can still run a dev server.
    pub network_ports: Vec<AnnotatedPort>,
    /// Backend policy groups this profile declines — see
    /// [`GROUPS_EXCLUDE`]. Fixed, not manifest-driven, but carried on the
    /// compiled value (rather than read from the constant at each call
    /// site) so `policy --render` and `to_nono_profile` see the same list.
    pub groups_exclude: Vec<&'static str>,
    /// The backend setting `extends: "default"` used to supply implicitly
    /// — see [`SIGNAL_MODE`].
    pub signal_mode: &'static str,
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
    let mut filesystem_read: Vec<AnnotatedValue> = manifest
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

    let network_ports: Vec<AnnotatedPort> = manifest
        .network
        .ports
        .iter()
        .map(|p| AnnotatedPort {
            value: *p,
            origin: Origin::Manifest("network.ports"),
        })
        .collect();

    // What system_read_linux_core/macos used to grant implicitly, replaced
    // with only what the keeper itself needs — see [`KEEPER_SYSTEM_READ`].
    // Project code gets none of this: its toolchain comes from the
    // provider's closure, never the host.
    filesystem_read.extend(
        KEEPER_SYSTEM_READ
            .iter()
            .map(|p| AnnotatedValue::new(*p, Origin::Baseline)),
    );

    CompiledPolicy {
        sandbox_name: manifest.sandbox.name.clone(),
        filesystem_allow,
        filesystem_read,
        filesystem_deny,
        network_block: manifest.network.default == NetworkDefault::Deny,
        network_allow_domain,
        network_ports,
        groups_exclude: GROUPS_EXCLUDE.to_vec(),
        signal_mode: SIGNAL_MODE,
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

    /// Grant read access to the directory holding *this build* of the
    /// devcroft binary, with `Origin::Baseline` — the keeper must be able
    /// to read+exec itself inside the boundary it applies to itself, and
    /// no baseline group can know where a given build lives (own-policy-
    /// baseline task 6.1). Previously appended to the projected
    /// [`NonoProfile`] directly after compilation, which meant this grant
    /// existed in `profile.json` but never in `CompiledPolicy` — the
    /// exact kind of unrendered rule `policy --render` is supposed to
    /// catch, and would have gone on catching, since nothing called
    /// `render` or `why` with it applied.
    pub fn with_keeper_exe_grant(mut self, exe_dir: impl Into<String>) -> Self {
        self.filesystem_read
            .push(AnnotatedValue::new(exe_dir.into(), Origin::Baseline));
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
            security: NonoSecurity {
                signal_mode: self.signal_mode,
            },
            groups: NonoGroups {
                exclude: self.groups_exclude.iter().map(|g| g.to_string()).collect(),
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
                open_port: self.network_ports.iter().map(|a| a.value).collect(),
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
    pub security: NonoSecurity,
    pub groups: NonoGroups,
    pub filesystem: NonoFilesystem,
    pub network: NonoNetwork,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NonoMeta {
    pub name: String,
}

/// `security.signal_mode` — see [`SIGNAL_MODE`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NonoSecurity {
    pub signal_mode: &'static str,
}

/// `groups.exclude` — see [`GROUPS_EXCLUDE`]. `include` is deliberately
/// absent: devcroft never names groups to include, only ones to decline,
/// and nono's schema treats a missing `include` as "the full injected
/// set", which is exactly the behavior devcroft relies on here.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NonoGroups {
    pub exclude: Vec<String>,
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
    /// Loopback TCP ports the sandboxed process may bind and connect on
    /// (`manifest:network.ports`). Omitted entirely when empty, so a
    /// manifest that declares no ports produces byte-identical
    /// `profile.json` to one written before this field existed.
    ///
    /// This is nono's own field name, and picking it over the adjacent
    /// `listen_port` was determined empirically, not from the schema
    /// descriptions: against nono 0.71.0 on Linux, `open_port` grants a
    /// real `127.0.0.1` bind under `block: true`, while `listen_port`
    /// granted neither a loopback nor a `0.0.0.0` bind.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub open_port: Vec<u16>,
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
        assert!(
            parsed["network"].get("open_port").is_none(),
            "a manifest declaring no ports must produce byte-identical \
             profile.json to one written before the field existed"
        );
    }

    #[test]
    fn network_ports_compile_to_open_port_alongside_a_deny_default() {
        let (manifest, _) =
            parse("[sandbox]\nname = \"myproj\"\n[network]\ndefault = \"deny\"\nports = [5432]\n")
                .unwrap();
        let compiled = compile(&manifest);

        assert!(compiled.network_block, "egress stays denied");
        assert_eq!(compiled.network_ports.len(), 1);
        assert_eq!(compiled.network_ports[0].value, 5432);
        assert_eq!(
            compiled.network_ports[0].origin,
            Origin::Manifest("network.ports")
        );

        // `open_port`, not `listen_port`: determined empirically against
        // nono 0.71.0 — see `NonoNetwork::open_port`'s doc comment. A
        // rename here silently stops granting anything, since nono
        // ignores unknown profile fields rather than rejecting them.
        let parsed: serde_json::Value =
            serde_json::from_str(&compiled.to_nono_profile().to_json()).unwrap();
        assert_eq!(parsed["network"]["block"], true);
        assert_eq!(parsed["network"]["open_port"][0], 5432);
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

        // `cat` is no longer covered by the compiled policy on its own:
        // GROUPS_EXCLUDE declines system_read_linux_core/macos, so a host
        // binary not supplied by a provider's closure is exactly what the
        // policy spec's "Host binary not supplied by the closure" scenario
        // says should be denied. Granting the host `cat`'s own directory
        // stands in for what a real provider's closure would supply, so
        // this test still exercises "profile validates and executes under
        // real nono" rather than becoming a same-host-binary-denied test
        // (that behavior has its own coverage below).
        let cat_dir = std::path::Path::new(
            std::process::Command::new("which")
                .arg("cat")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|| "/bin/cat".to_string())
                .as_str(),
        )
        .parent()
        .unwrap()
        .to_string_lossy()
        .into_owned();
        let (manifest, _) = parse(&format!(
            "[sandbox]\nname = \"nonocheck\"\n[filesystem]\nallow = [{:?}]\nread = [{cat_dir:?}]\n",
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

    /// Task 1.1's completeness proof, plus task 1.3's regression guard for
    /// the group-injection behavior this whole change depends on, in one
    /// test: compiles a real profile, resolves it through the actual
    /// backend (`nono profile show --json`, not the file devcroft wrote),
    /// and asserts every included group is one `policy --render` already
    /// accounts for — [`GROUPS_EXCLUDE`] (declined) or
    /// [`BACKEND_ENFORCED_GROUPS`] (rendered via
    /// `render_backend_enforced`). If a future nono stops injecting the
    /// full 18-group set the way design.md's "measurement that reframes
    /// everything" found, or starts injecting a nineteenth group neither
    /// constant knows about, this fails instead of silently under-
    /// rendering. Self-skips when nono is absent.
    #[test]
    fn every_resolved_group_is_accounted_for_by_render() {
        if std::process::Command::new("nono")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let (manifest, _) = parse("[sandbox]\nname = \"completeness\"\n").unwrap();
        let dir = std::env::temp_dir().join(format!(
            "devcroft-policy-completeness-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let profile_path = dir.join("profile.json");
        std::fs::write(
            &profile_path,
            compile(&manifest).to_nono_profile().to_json(),
        )
        .unwrap();

        let show = std::process::Command::new("nono")
            .arg("profile")
            .arg("show")
            .arg(&profile_path)
            .arg("--json")
            .output()
            .unwrap();
        assert!(show.status.success(), "nono profile show failed: {show:?}");
        let resolved: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
        let included: Vec<&str> = resolved["groups"]["include"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        for group in &included {
            assert!(
                BACKEND_ENFORCED_GROUPS.contains(group),
                "resolved group {group:?} is neither excluded nor in \
                 BACKEND_ENFORCED_GROUPS — policy --render would silently omit it"
            );
        }
        for excluded in GROUPS_EXCLUDE {
            assert!(
                !included.contains(excluded),
                "{excluded:?} is in GROUPS_EXCLUDE but nono still resolved it as included"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
