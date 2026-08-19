//! Builds the OCI runtime `config.json` `runsc run` consumes, from the
//! same [`CompiledPolicy`] the process tier compiles into (design.md
//! decision 3) plus the provider's resolved environment. Deliberately a
//! typed subset — process, root, mounts, `linux.namespaces` — matching
//! what mxc's own `oci_spec.rs` found sufficient for the same runtime,
//! not a dependency on an external `oci-spec` crate for a schema this
//! small and stable. Pure JSON generation: no host dependency, so this
//! compiles and is fully unit-tested on every platform, same posture as
//! [`super::runsc_command`].
//!
//! One assumption worth naming, and now corrected rather than merely
//! reasoned about: [`INIT_COMMAND`] resolves `sh` through the resolved
//! environment's own `PATH` rather than bundling a static init binary.
//! The claim that every qualified provider's closure transitively
//! depends on a POSIX shell and coreutils holds for a nix flake
//! devShell (nixpkgs' stdenv bootstrap pulls bash+coreutils in
//! automatically) but **does not hold for a bare `flox init` with
//! nothing installed** — found live (add-flox-services task 6.5) against
//! a real `runsc`: such an environment's own `PATH` prefix has no shell
//! at all, and the mount list's deny-by-default means the host's
//! `/usr/bin` fallback flox's own `PATH` construction relies on isn't
//! reachable inside the sandbox either. Every *real* project this tier
//! targets installs something, so this is a real gap for a genuinely
//! empty flox project, not a hypothetical one; the fix, if it's ever
//! worth it, is a small statically-linked init bundled the same way
//! `landlock-abi.c` is already compiled into the devcontainer image, not
//! a redesign of the mount model.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::policy::CompiledPolicy;

/// The OCI runtime spec version this subset targets — matches what
/// current `runsc` releases accept.
const OCI_VERSION: &str = "1.0.2-dev";

/// Kept alive as the sandbox's PID 1 so `runsc exec` has a live
/// container to attach sessions to (design.md decision 2: persistent,
/// not one-shot). `sh` is resolved via the injected `PATH`, not an
/// absolute path — see the module doc for why that's safe here.
const INIT_COMMAND: &[&str] = &["sh", "-c", "while true; do sleep 86400 || true; done"];

/// Which way `[network]` resolved for this sandbox (design.md decision
/// 1: no per-sandbox netstack, since `runsc` rejects `--network=sandbox`
/// together with `--rootless`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMode {
    /// `network.default = "deny"` with no egress grant: no connectivity
    /// of any kind, inbound or outbound.
    None,
    /// The manifest's `[network]` section grants egress: hostinet
    /// passthrough, unfiltered. Unlike the process tier's `nono` profile,
    /// nothing here enforces `[network]`'s domain allowlist at this
    /// tier — an earlier version of this comment claimed Landlock on the
    /// Sentry process did, but that was never actually wired (no code
    /// path threads `network_allow_domain` into anything gVisor-facing),
    /// and Landlock cannot express domain-based filtering in any current
    /// ABI regardless. A domain allowlist at the hardened tier is
    /// currently a known gap, not a silently-dropped enforcement — see
    /// `docs/decisions.md`.
    Host,
}

impl NetworkMode {
    /// The corrected network-policy requirement: block only when nothing
    /// in the manifest grants egress.
    pub fn from_compiled_policy(compiled: &CompiledPolicy) -> Self {
        if compiled.network_block && compiled.network_allow_domain.is_empty() {
            NetworkMode::None
        } else {
            NetworkMode::Host
        }
    }

    pub(crate) fn runsc_flag(self) -> &'static str {
        match self {
            NetworkMode::None => "none",
            NetworkMode::Host => "host",
        }
    }
}

/// What [`build`] needs beyond the compiled policy: the paths the OCI
/// bundle itself lives at, and the provider's resolved activation.
pub struct BundleInputs<'a> {
    /// The sandbox's project root, bind-mounted read-write.
    pub project_root: &'a std::path::Path,
    /// Where the bundle directory lives on the host — `root.path` in the
    /// generated spec is `bundle_dir/rootfs`, **absolute**. Found live,
    /// not reasoned about: a relative `"rootfs"` (the OCI spec's own
    /// documented convention, and what an earlier version of this
    /// function emitted) makes gVisor's rootless gofer fail every single
    /// mount with "failed to safely mount: expected to open rootfs/<path>,
    /// but found <absolute path>" — its `SafeMount` check opens the
    /// literal `root.path`-joined destination and compares it against
    /// `/proc/self/fd/<n>`'s always-absolute readlink target, which a
    /// relative `root.path` can never equal. Confirmed by hand: the
    /// identical bundle boots once `root.path` is made absolute, and
    /// fails this way whenever it isn't.
    pub bundle_dir: &'a std::path::Path,
    /// Also used to resolve read-only store grants that must be
    /// representable as mounts (add-gvisor-backend's "Provider grants map
    /// onto mounts or fail loudly" requirement — checked by the caller,
    /// not here).
    pub read_only_grants: &'a [String],
    /// The provider's captured activation env diff, injected into the
    /// init process and inherited by every `runsc exec` session.
    pub env: &'a BTreeMap<String, String>,
}

/// Builds the OCI spec for `compiled`'s sandbox. Deny-by-default by
/// construction: only `project_root`, `read_only_grants`, and the fixed
/// baseline skeleton mounts below are ever reachable inside the sandbox —
/// there is no host-root passthrough to omit.
pub fn build(compiled: &CompiledPolicy, inputs: &BundleInputs<'_>) -> OciSpec {
    let mut mounts = vec![
        Mount {
            destination: "/proc".to_string(),
            typ: "proc".to_string(),
            source: "proc".to_string(),
            options: vec![],
        },
        Mount {
            destination: "/dev".to_string(),
            typ: "tmpfs".to_string(),
            source: "tmpfs".to_string(),
            options: vec!["nosuid".to_string(), "strictatime".to_string()],
        },
        Mount {
            destination: "/tmp".to_string(),
            typ: "tmpfs".to_string(),
            source: "tmpfs".to_string(),
            options: vec![],
        },
    ];

    // Provider store grants: read-only, exactly what the resolved
    // closure needs — never widened, per the "provider resolution must
    // not widen the policy" invariant, projected onto mounts.
    for grant in inputs.read_only_grants {
        mounts.push(Mount {
            destination: grant.clone(),
            typ: "bind".to_string(),
            source: grant.clone(),
            options: vec!["bind".to_string(), "ro".to_string()],
        });
    }

    // The project root: read-write, always granted (mirrors the process
    // tier's implicit `filesystem.allow = ["."]` default).
    let project_root_str = inputs.project_root.to_string_lossy().into_owned();
    mounts.push(Mount {
        destination: project_root_str.clone(),
        typ: "bind".to_string(),
        source: project_root_str.clone(),
        options: vec!["bind".to_string(), "rw".to_string()],
    });

    let mut env: Vec<String> = inputs.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    env.sort(); // deterministic output, same rationale as CompiledPolicy::compile

    let network_mode = NetworkMode::from_compiled_policy(compiled);
    let mut namespaces = vec![
        Namespace {
            typ: "pid".to_string(),
        },
        Namespace {
            typ: "ipc".to_string(),
        },
        Namespace {
            typ: "uts".to_string(),
        },
        Namespace {
            typ: "mount".to_string(),
        },
    ];
    if network_mode == NetworkMode::None {
        // An entry with no `path` requests a fresh, isolated network
        // namespace — omitted entirely for `Host`, which asks runsc to
        // share the host's, per design.md decision 1's corrected model.
        namespaces.push(Namespace {
            typ: "network".to_string(),
        });
    }

    OciSpec {
        oci_version: OCI_VERSION,
        process: Process {
            terminal: false,
            user: User { uid: 0, gid: 0 },
            args: INIT_COMMAND.iter().map(|s| s.to_string()).collect(),
            env,
            cwd: project_root_str,
            capabilities: Capabilities::empty(),
        },
        root: Root {
            path: inputs
                .bundle_dir
                .join("rootfs")
                .to_string_lossy()
                .into_owned(),
            readonly: true,
        },
        hostname: compiled.sandbox_name.clone(),
        mounts,
        linux: Linux { namespaces },
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OciSpec {
    #[serde(rename = "ociVersion")]
    pub oci_version: &'static str,
    pub process: Process,
    pub root: Root,
    pub hostname: String,
    pub mounts: Vec<Mount>,
    pub linux: Linux,
}

impl OciSpec {
    /// Serialized deterministically (fixed field order, sorted env) so
    /// the bundle is reproducible from the same manifest and lockfile —
    /// `up --recreate` rebuilds it byte-identically given unchanged
    /// inputs.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("OciSpec serialization is infallible")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Process {
    pub terminal: bool,
    pub user: User,
    pub args: Vec<String>,
    pub env: Vec<String>,
    pub cwd: String,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
}

/// Every capability set left empty: the sandbox's confinement comes from
/// the mount model and Sentry's own syscall mediation, not from Linux
/// capabilities, which gVisor's rootless mode does not have anyway.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Capabilities {
    pub bounding: Vec<String>,
    pub effective: Vec<String>,
    pub inheritable: Vec<String>,
    pub permitted: Vec<String>,
    pub ambient: Vec<String>,
}

impl Capabilities {
    fn empty() -> Self {
        Capabilities {
            bounding: vec![],
            effective: vec![],
            inheritable: vec![],
            permitted: vec![],
            ambient: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Root {
    pub path: String,
    pub readonly: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mount {
    pub destination: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub source: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Linux {
    pub namespaces: Vec<Namespace>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Namespace {
    #[serde(rename = "type")]
    pub typ: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;
    use crate::policy;
    use std::path::Path;

    fn inputs<'a>(
        project_root: &'a Path,
        read_only_grants: &'a [String],
        env: &'a BTreeMap<String, String>,
    ) -> BundleInputs<'a> {
        BundleInputs {
            project_root,
            bundle_dir: Path::new("/bundle"),
            read_only_grants,
            env,
        }
    }

    #[test]
    fn deny_all_policy_produces_network_none_and_a_fresh_netns() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = Vec::new();
        let env = BTreeMap::new();
        let spec = build(&compiled, &inputs(project_root, &grants, &env));

        assert_eq!(
            NetworkMode::from_compiled_policy(&compiled),
            NetworkMode::None
        );
        assert!(
            spec.linux.namespaces.iter().any(|n| n.typ == "network"),
            "deny-all must request a fresh network namespace"
        );
    }

    #[test]
    fn egress_allowlist_produces_network_host_and_no_fresh_netns() {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            default = "deny"
            allow = ["github.com"]
            "#,
        )
        .unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = Vec::new();
        let env = BTreeMap::new();
        let spec = build(&compiled, &inputs(project_root, &grants, &env));

        assert_eq!(
            NetworkMode::from_compiled_policy(&compiled),
            NetworkMode::Host
        );
        assert!(
            !spec.linux.namespaces.iter().any(|n| n.typ == "network"),
            "granted egress must share the host network namespace, not request a fresh one"
        );
    }

    /// Backs a concrete claim `tests/gvisor_hardened_tier_pid_isolation.rs`
    /// (and the process-tier exploit `tests/process_tier_pid_namespace_exploit.rs`
    /// documents the *absence* of) depends on: every hardened-tier bundle,
    /// regardless of policy shape, requests its own `pid`/`ipc`/`uts`/`mount`
    /// namespaces — never omitted, never conditional on the manifest the
    /// way the `network` namespace is. This is what makes a signal-based
    /// attack across the sandbox boundary structurally impossible at this
    /// tier rather than merely denied by a runtime check: inside a fresh
    /// PID namespace, a host PID has no referent to `kill()` in the first
    /// place. Pure JSON generation — no `runsc` required, unlike the live
    /// exploit test this exists to substantiate.
    #[test]
    fn hardened_tier_bundle_always_requests_pid_ipc_uts_and_mount_namespaces() {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            default = "deny"
            allow = ["github.com"]
            "#,
        )
        .unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = Vec::new();
        let env = BTreeMap::new();
        let spec = build(&compiled, &inputs(project_root, &grants, &env));

        let types: Vec<&str> = spec
            .linux
            .namespaces
            .iter()
            .map(|n| n.typ.as_str())
            .collect();
        for required in ["pid", "ipc", "uts", "mount"] {
            assert!(
                types.contains(&required),
                "hardened-tier bundle must always request a {required} namespace, got {types:?}"
            );
        }
    }

    #[test]
    fn read_only_grants_become_ro_bind_mounts() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = vec!["/nix/store".to_string()];
        let env = BTreeMap::new();
        let spec = build(&compiled, &inputs(project_root, &grants, &env));

        let store_mount = spec
            .mounts
            .iter()
            .find(|m| m.destination == "/nix/store")
            .expect("store grant must be mounted");
        assert_eq!(store_mount.typ, "bind");
        assert!(store_mount.options.contains(&"ro".to_string()));
    }

    #[test]
    fn project_root_is_a_rw_bind_mount() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = Vec::new();
        let env = BTreeMap::new();
        let spec = build(&compiled, &inputs(project_root, &grants, &env));

        let root_mount = spec
            .mounts
            .iter()
            .find(|m| m.destination == "/proj")
            .expect("project root must be mounted");
        assert!(root_mount.options.contains(&"rw".to_string()));
    }

    #[test]
    fn unmounted_paths_are_simply_absent() {
        // The deny-by-default requirement's own scenario: nothing beyond
        // the fixed skeleton (proc/dev/tmp), store grants, and the
        // project root is ever mounted — there is no host `/`, `/home`,
        // or `/etc` passthrough to accidentally include.
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = Vec::new();
        let env = BTreeMap::new();
        let spec = build(&compiled, &inputs(project_root, &grants, &env));

        let destinations: Vec<&str> = spec.mounts.iter().map(|m| m.destination.as_str()).collect();
        assert_eq!(destinations, vec!["/proc", "/dev", "/tmp", "/proj"]);
    }

    #[test]
    fn env_is_sorted_for_deterministic_output() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = Vec::new();
        let mut env = BTreeMap::new();
        env.insert("ZVAR".to_string(), "1".to_string());
        env.insert("AVAR".to_string(), "2".to_string());
        let spec = build(&compiled, &inputs(project_root, &grants, &env));

        assert_eq!(spec.process.env, vec!["AVAR=2", "ZVAR=1"]);
    }

    #[test]
    fn output_is_deterministic() {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = ["src"]
            "#,
        )
        .unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = vec!["/nix/store".to_string()];
        let env = BTreeMap::new();

        let a = build(&compiled, &inputs(project_root, &grants, &env)).to_json();
        let b = build(&compiled, &inputs(project_root, &grants, &env)).to_json();
        assert_eq!(a, b);
    }

    #[test]
    fn capabilities_are_always_empty() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = Vec::new();
        let env = BTreeMap::new();
        let spec = build(&compiled, &inputs(project_root, &grants, &env));

        assert!(spec.process.capabilities.bounding.is_empty());
        assert!(spec.process.capabilities.effective.is_empty());
        assert!(spec.process.capabilities.permitted.is_empty());
    }

    #[test]
    fn json_shape_matches_oci_runtime_spec_field_names() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = policy::compile(&manifest);
        let project_root = Path::new("/proj");
        let grants = Vec::new();
        let env = BTreeMap::new();
        let json = build(&compiled, &inputs(project_root, &grants, &env)).to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["ociVersion"], OCI_VERSION);
        assert_eq!(parsed["root"]["path"], "/bundle/rootfs");
        assert_eq!(parsed["root"]["readonly"], true);
        assert_eq!(parsed["hostname"], "myproj");
        assert!(parsed["mounts"].as_array().unwrap().len() >= 3);
    }
}
