//! Compiles a [`Manifest`] plus baseline denials
//! into a [`CompiledPolicy`], projected into the `nono` library's
//! `CapabilitySet` for the process tier's self-restriction
//! (`use-nono-library`) — deterministic, and with every rule traceable
//! back to the manifest key, provider, or baseline default that produced
//! it.

mod capability_set;
mod degraded;
mod render;
mod why;

pub use capability_set::{CapabilityPlan, CapabilitySetError, ResolvedGrant};
pub use degraded::{
    DegradedCapability, backend_support, backend_supported, detect as detect_degraded,
};
pub use render::render;
pub use why::{Explanation, Op, why_host, why_path};

use crate::config::{Manifest, NetworkDefault};
use crate::paths::{SENSITIVE_PATHS, is_within};

/// devcroft's own data dir (client keypair, host keys). Always denied,
/// regardless of manifest contents — see the policy spec's "Baseline
/// denials" requirement.
const DEVCROFT_DATA_DIR: &str = "~/.local/share/devcroft";

/// Explicit read grants replacing what nono-cli's `system_read_linux_core`
/// / `system_read_macos` groups used to supply implicitly under the
/// exec-based process tier (own-policy-baseline) — sized for the
/// host-linked keeper binary's own needs (dynamic linker, libc, entropy),
/// not for project code, which gets its toolchain from the provider's
/// closure and is granted nothing here. Unlike own-policy-baseline's
/// `groups.exclude`, there is no group mechanism left to exclude *from*
/// under `use-nono-library`'s raw `CapabilitySet` — this is simply the
/// full explicit grant, still needed for the same reason.
///
/// Linux: mirrors the multiarch triplets nono's own group grants
/// unconditionally regardless of host arch (`/lib/x86_64-linux-gnu` and
/// `/lib/aarch64-linux-gnu` both, verified via `nono profile groups
/// system_read_linux_core --json`) rather than detecting the host triplet,
/// since a wrong guess would silently fail to load the keeper on whichever
/// arch it guessed wrong for. `/dev/urandom`: exercised continuously, not
/// just at startup — every inbound SSH key exchange the keeper's embedded
/// russh server performs after restriction draws from it. (`/dev/null` is
/// *not* here — see [`KEEPER_SYSTEM_READWRITE`].)
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
];

/// Read+write baseline grants — separate from [`KEEPER_SYSTEM_READ`]
/// because these actually need both, discovered the same way both times:
/// own-policy-baseline granted them read-only and it worked anyway,
/// because nono-cli's still-active `system_write_linux` group separately
/// granted write access underneath, unnoticed. `use-nono-library` dropped
/// that group entirely (design.md Decision 5), which surfaced both gaps
/// live as an opaque "keeper refused to spawn" with nothing pointing at
/// the real cause.
///
/// - `/dev/pts` (Linux): `devcroft shell`/pty sessions
///   (`keeper::pty::open_pty`, `libc::openpty`) — glibc's `openpty` opens
///   `/dev/ptmx`, itself a symlink to `/dev/pts/ptmx` on this host (and
///   every Linux system checked), and Landlock evaluates the resolved
///   target. macOS's `/dev/ptmx` equivalent is included on the same
///   reasoning, not independently live-verified (this devcontainer is
///   Linux-only).
/// - `/dev/null`: every session's `Stdio::null()` redirection
///   (`keeper::session::spawn_piped`/`spawn_pty`) opens it for *writing*
///   (stdout/stderr), not just reading — confirmed with a standalone
///   `Sandbox::apply_auto` + `std::process::Command` reproduction outside
///   this crate: `cmd.spawn()` itself failed with `Permission denied`
///   until this was `ReadWrite`, with the pty mechanics (`openpty`,
///   `setsid`, `TIOCSCTTY`, raw `dup2`+`execv`) all verified to work fine
///   in isolation first — the failure was specifically in
///   `std::process::Command::spawn()`'s own `Stdio::null()` handling.
#[cfg(not(target_os = "macos"))]
const KEEPER_SYSTEM_READWRITE: &[&str] = &["/dev/pts", "/dev/null"];

#[cfg(target_os = "macos")]
const KEEPER_SYSTEM_READWRITE: &[&str] = &["/dev/ptmx", "/dev/null"];

/// The signal isolation `extends: "default"` currently supplies as its
/// *only* effective contribution (own-policy-baseline design.md Decision
/// 4) — declared explicitly so it survives independently of `extends`
/// rather than being the single most easily lost property in the policy.
const SIGNAL_MODE: &str = "isolated";

/// Where a compiled rule came from, rendered as `manifest:<key>`,
/// `provider:<name>`, or `baseline`.
///
/// own-policy-baseline added a fourth variant, `BackendEnforced`, for
/// nono-cli's ~13 non-excluded policy groups reaching the backend outside
/// devcroft's own compiled rules. `use-nono-library` removed it: those
/// groups are a pure nono-cli concept (its own `policy.json` catalog),
/// invisible to the raw `nono` library the process tier now links
/// directly — there is nothing left for that origin to attribute
/// (design.md Decision 5, confirmed with the project owner).
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
/// annotations. [`CapabilityPlan::to_capability_set`] projects this down
/// to the `nono` library's `CapabilitySet` the process tier applies to
/// itself.
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
    /// The egress proxy's bound port, when domain filtering is active.
    /// Folded in post-hoc by `with_proxy_port` — like a provider's store
    /// grants, this can only be known by actually binding a socket at
    /// `up`, never from the manifest alone, so it has no place in
    /// [`compile`]'s pure projection. `Some` if and only if `up` started
    /// the proxy for this sandbox (`network.default = "deny"` and
    /// `network.allow` non-empty); `None` leaves the existing binary
    /// block/allow-all path in [`CapabilityPlan::to_capability_set`]
    /// untouched, so a sandbox with no domain filtering never spins up a
    /// proxy it doesn't need.
    pub network_proxy_port: Option<u16>,
    /// The backend setting `extends: "default"` used to supply implicitly
    /// under the exec-based process tier — see `SIGNAL_MODE`. Still
    /// meaningful under `use-nono-library`: `CapabilitySet::set_signal_mode`
    /// takes the same value directly.
    pub signal_mode: &'static str,
}

/// Compile `manifest` plus baseline denials into a [`CompiledPolicy`].
///
/// Deterministic: identical manifests always produce identically-ordered
/// output, since manifest lists preserve TOML order and baseline entries
/// are appended in the fixed order of `SENSITIVE_PATHS`.
pub fn compile(manifest: &Manifest) -> CompiledPolicy {
    let mut filesystem_allow: Vec<AnnotatedValue> = manifest
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
    // Pty allocation needs read+write, not read-only — see
    // [`KEEPER_SYSTEM_READWRITE`]'s doc comment for why this is separate.
    filesystem_allow.extend(
        KEEPER_SYSTEM_READWRITE
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
        network_proxy_port: None,
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

    /// Whether this policy calls for the egress proxy at all: `up` checks
    /// this *before* binding a socket, so a sandbox with domain filtering
    /// off never pays for a proxy process it has nothing to filter.
    pub fn wants_egress_proxy(&self) -> bool {
        self.network_block && !self.network_allow_domain.is_empty()
    }

    /// Whether this sandbox should get its own network namespace, giving
    /// its declared ports a private table instead of the host's shared
    /// one — the fix for two sandboxes both binding 5432 (README's own
    /// "Why").
    ///
    /// Deliberately narrow: `true` only when the manifest asks for zero
    /// outbound network at all (`network.default = "deny"` and no
    /// `network.allow` entries) *and* there is something to isolate
    /// (`network.ports` non-empty, or the caller reports services are
    /// declared). Both halves are load-bearing, not incidental:
    ///
    /// - **Zero egress, not "filtered egress".** An isolated namespace
    ///   starts with loopback only — nothing routes it to the real
    ///   network at all, filtered or not. `add-egress-proxy`'s proxy
    ///   binds on the *host's* loopback; a sandbox in its own namespace
    ///   cannot reach it without a forwarding helper (pasta/slirp4netns),
    ///   which `add-linux-agent-fleet`'s D5 has not resolved. Reusing
    ///   this for a sandbox with any `network.allow` entry — i.e. when
    ///   [`Self::wants_egress_proxy`] is also true — would silently
    ///   break that sandbox's egress instead of isolating its ports, so
    ///   the two are mutually exclusive by construction: this returns
    ///   `false` whenever `wants_egress_proxy` would return `true`.
    ///   `network.default = "allow"` (unfiltered, and today's default
    ///   for a bare manifest) is refused for the identical reason: an
    ///   isolated namespace cannot reach *anything* external without
    ///   that same missing helper, so "allow" cannot be honoured inside
    ///   one either.
    /// - **Only when there is something to isolate.** A sandbox binding
    ///   nothing has nothing that can collide; entering a namespace for
    ///   it would cost a syscall for no observable benefit.
    ///
    /// A sandbox that wants both isolation and any outbound network gets
    /// neither degraded nor upgraded by this method — it simply returns
    /// `false`, leaving that sandbox exactly where it was before this
    /// existed (shared host ports, `add-port-allocation`'s open gap),
    /// which is a known limitation, not a regression this introduces.
    pub fn wants_network_isolation(&self, _services_declared: bool) -> bool {
        // Every deny-network sandbox, not only those with something to
        // isolate. That second condition used to be here and was removed
        // for a security reason, not a tidiness one.
        //
        // **Landlock's network rules are TCP-only.** `NetPort` gates
        // `connect`/`bind` for AF_INET stream sockets and says nothing
        // about UDP, so a sandbox with `network.default = "deny"` — with
        // or without an allowlist — could open a UDP socket and complete
        // a full DNS round-trip to 8.8.8.8, measured. nono does have a
        // seccomp filter that denies UDP, but it is `apply_auto`'s
        // fallback for pre-V4 Landlock kernels and is never installed on
        // a V6 host. Same shape as the `install_seccomp_proxy_filter`
        // finding in `add-egress-proxy` task 0: the library has two
        // paths, and the modern one does not cover what the fallback did.
        //
        // A network namespace closes it without needing either filter —
        // an isolated sandbox has no route out at all, so UDP fails with
        // `ENETUNREACH` regardless of protocol coverage. Egress that
        // *is* wanted still works, because it goes through the relay
        // (`add-egress-proxy` E7), which is TCP to the proxy's own port.
        //
        // The cost of widening this is a namespace for sandboxes that
        // declare no ports — which is a probe and an `unshare`, and
        // nothing observable, since a sandbox with no declared ports has
        // no host-visible ports to lose.
        self.network_block
    }

    /// Whether isolating this sandbox would collide with a port it
    /// declared for itself.
    ///
    /// The relay binds the host proxy's own port number *inside* the
    /// namespace, which is what keeps `HTTP_PROXY` and the compiled
    /// `proxy_only` gate identical isolated or not. A fresh namespace has
    /// every port free, so the only possible clash is this sandbox's own
    /// `network.ports` — and the proxy port is OS-assigned from the
    /// ephemeral range, so a manifest naming 5432 or 3000 never hits it.
    /// Checked rather than assumed, because the consequence would be the
    /// relay silently failing to bind and egress disappearing with it.
    pub fn proxy_port_collides_with_declared_ports(&self, proxy_port: u16) -> bool {
        self.network_ports.iter().any(|p| p.value == proxy_port)
    }

    /// Fold in the egress proxy's bound port, once `up` has actually
    /// started it (see [`Self::wants_egress_proxy`]). See
    /// `network_proxy_port`'s doc for why this isn't part of [`compile`].
    pub fn with_proxy_port(mut self, port: u16) -> Self {
        self.network_proxy_port = Some(port);
        self
    }

    /// Grant read access to the directory holding *this build* of the
    /// devcroft binary, with `Origin::Baseline` — the keeper must be able
    /// to read+exec itself inside the boundary it applies to itself, and
    /// no baseline group can know where a given build lives (own-policy-
    /// baseline task 6.1). `to_capability_set` folds this in like any
    /// other `filesystem_read` grant, so it's rendered and explainable
    /// like every other rule.
    pub fn with_keeper_exe_grant(mut self, exe_dir: impl Into<String>) -> Self {
        self.filesystem_read
            .push(AnnotatedValue::new(exe_dir.into(), Origin::Baseline));
        self
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

        let a = compile(&manifest).to_capability_plan();
        let b = compile(&manifest).to_capability_plan();
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

    /// The JSON-profile-shape test this replaced (`nono_profile_json_matches_expected_shape`)
    /// and the real-nono functional test (`compiled_profile_validates_and_executes_under_real_nono`)
    /// are both gone, not replaced in kind: `capability_set.rs`'s own tests
    /// cover the shape (a `CapabilitySet` actually contains the right
    /// grants), and a unit test cannot call `Sandbox::apply_auto` at all —
    /// it's irreversible and process-wide, so applying it inside a `cargo
    /// test` process would restrict every other test sharing that process.
    /// The real functional proof ("self-restriction actually works") lives
    /// in the integration suite (`tests/*.rs`), which spawns the real
    /// binary as its own process — see `use-nono-library` task group 4.
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

        let plan = compiled.to_capability_plan();
        assert!(plan.network_block);
        assert_eq!(plan.network_ports, vec![5432]);
    }

    #[test]
    fn every_deny_network_sandbox_is_isolated_regardless_of_ports_or_egress() {
        let deny_with_ports = compile(
            &parse("[sandbox]\nname = \"p\"\n[network]\ndefault = \"deny\"\nports = [5432]\n")
                .unwrap()
                .0,
        );
        assert!(deny_with_ports.wants_network_isolation(false));
        assert!(deny_with_ports.wants_network_isolation(true));

        // A bare deny sandbox is isolated too, and this assertion was
        // inverted to make it so. It used to read "nothing to isolate:
        // entering a namespace would cost a syscall for no observable
        // benefit" — which was true about *ports* and wrong about
        // egress. Landlock's network rules are TCP-only, so such a
        // sandbox could send UDP freely; the namespace is what denies
        // it (`tests/udp_egress_denied.rs`).
        let deny_bare = compile(
            &parse("[sandbox]\nname = \"p\"\n[network]\ndefault = \"deny\"\n")
                .unwrap()
                .0,
        );
        assert!(deny_bare.wants_network_isolation(false));
        assert!(deny_bare.wants_network_isolation(true));

        // **An allowlist no longer disqualifies isolation.** This
        // assertion was inverted when written this morning, on the
        // reasoning that an isolated namespace could not reach the
        // host-bound proxy. It can: the proxy listens on a unix socket,
        // which crosses a network namespace, and the keeper relays to it
        // from inside (add-egress-proxy design.md E7). Both properties
        // now hold together, which is the combination an agent needs —
        // asserted end to end in `tests/isolated_egress_e2e.rs`.
        let deny_with_allow = compile(
            &parse(
                "[sandbox]\nname = \"p\"\n\
                 [network]\ndefault = \"deny\"\nallow = [\"example.com\"]\nports = [5432]\n",
            )
            .unwrap()
            .0,
        );
        assert!(deny_with_allow.wants_network_isolation(true));
        assert!(deny_with_allow.wants_egress_proxy());
    }

    #[test]
    fn an_unfiltered_network_default_still_gets_no_isolation() {
        // `default = "allow"` means "reach the real network directly",
        // and there is no proxy in that case to relay through — so an
        // isolated namespace would have no route out at all. This is the
        // one case the relay does not rescue, and it stays excluded by
        // `network_block` being false.
        let allow_default = compile(
            &parse("[sandbox]\nname = \"p\"\n[network]\ndefault = \"allow\"\nports = [5432]\n")
                .unwrap()
                .0,
        );
        assert!(!allow_default.wants_network_isolation(true));
        assert!(!allow_default.wants_egress_proxy());
    }

    #[test]
    fn the_proxy_port_collision_guard_only_fires_on_a_declared_port() {
        let compiled = compile(
            &parse(
                "[sandbox]\nname = \"p\"\n\
                 [network]\ndefault = \"deny\"\nallow = [\"example.com\"]\nports = [5432]\n",
            )
            .unwrap()
            .0,
        );
        // The relay binds the proxy's own number inside the namespace, so
        // this is the one collision a fresh namespace does not rule out.
        assert!(compiled.proxy_port_collides_with_declared_ports(5432));
        assert!(!compiled.proxy_port_collides_with_declared_ports(41234));
    }
}
