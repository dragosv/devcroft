//! `add-backend-capabilities`: one declared, machine-readable answer to
//! "what does devcroft actually enforce?" — replacing the same claim
//! maintained by hand in five prose locations (README,
//! `docs/threat-model.md`, `docs/decisions.md`, `policy::degraded`'s
//! doc comments, `doctor`'s output), which drifted every time the code
//! moved and nothing forced the prose to follow.
//!
//! **Compiled in, not a data file** (design.md open question 1,
//! resolved): the requirement that a change updating a capability must
//! update its declaration *in the same change* (spec: "A change that
//! alters a capability updates the declaration") argues for the
//! declaration living where the code does, where `cargo build` and
//! review both see it, rather than a TOML file nothing forces anyone to
//! open.
//!
//! **Granularity is per user-visible capability** (open question 2,
//! resolved), not per enforcement mechanism: one entry for "domain
//! filtering", not a separate one for the Landlock `NetPort` rule and
//! the Seatbelt equivalent underneath it. That is what a reader of
//! `doctor` or the README wants to know; the mechanism is what a given
//! entry's evidence cites.

use std::fmt;

/// A capability's status on one platform — a closed vocabulary
/// (spec: "Status uses a closed vocabulary that distinguishes unmeasured
/// from unsupported"). Deliberately not `String`: free text is exactly
/// how the five prose locations this replaces drifted, each phrasing the
/// same fact slightly differently until the facts themselves diverged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Enforced, uniformly, with no platform-specific weakening.
    Enforced,
    /// Enforced, but weaker than the same capability elsewhere — the
    /// entry carrying this status MUST explain how in its `evidence`
    /// (design.md C4 / spec: "must say so where the claim is made, not
    /// in a footnote elsewhere").
    EnforcedWithNamedDegradation,
    /// This platform cannot provide it. A constraint, not a choice —
    /// distinct from `NotAdopted`, which is devcroft's own choice not to
    /// use something the platform *can* provide.
    Unsupported,
    /// The backend library offers this; devcroft does not configure or
    /// use it. Recorded rather than omitted, because the gap between
    /// offered and adopted is the whole point of this matrix once there
    /// is a single backend (design.md C1).
    NotAdopted,
    /// Believed to hold because the mechanism suggests it should, but
    /// nothing has measured it. Not a synonym for `Enforced` — this
    /// project has shipped that substitution before and been wrong
    /// (design.md C2: macOS domain filtering, raw-socket allowlist
    /// bypass, gVisor port isolation — three claims that were reasonable,
    /// unmeasured, and wrong).
    Unverified,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Status::Enforced => "enforced",
            Status::EnforcedWithNamedDegradation => "enforced (degraded)",
            Status::Unsupported => "unsupported",
            Status::NotAdopted => "not-adopted",
            Status::Unverified => "unverified",
        })
    }
}

/// One platform's entry for one capability: its status and the evidence
/// behind it (spec: "Every claim names its evidence" — required on
/// every variant, `NotAdopted` included, since "nobody has tried" is
/// itself worth being able to point at).
#[derive(Debug, Clone, Copy)]
pub struct PlatformStatus {
    pub status: Status,
    /// A test path, a live measurement, or an upstream guarantee — never
    /// "it seemed to work" (spec: "A capability whose status rests on
    /// inference SHALL be `unverified` regardless of how reasonable the
    /// inference is"). For `EnforcedWithNamedDegradation`, this is also
    /// where the *how* it's weaker lives.
    pub evidence: &'static str,
}

/// A live probe of whether *this host* can actually provide a capability
/// devcroft declares `Enforced`/`EnforcedWithNamedDegradation` on its
/// platform — `doctor`'s declared-versus-available distinction (spec:
/// "`doctor` SHALL report the declared capabilities alongside what this
/// host can provide"). `None` where no host-specific gate exists beyond
/// the backend being supported at all (filesystem policy, e.g. — if
/// Landlock/Seatbelt works, this works, and `doctor` already reports
/// that once).
pub type HostProbe = fn() -> bool;

/// One user-visible capability: what it is, its status on each platform,
/// and — only when devcroft's status differs from what the platform can
/// give — how to check this specific host.
pub struct Capability {
    pub name: &'static str,
    pub description: &'static str,
    pub linux: PlatformStatus,
    pub macos: PlatformStatus,
    /// Only meaningful when `linux`/`macos` claim `Enforced` or
    /// `EnforcedWithNamedDegradation` — `doctor` skips the probe for
    /// `NotAdopted` entries entirely (spec: "`doctor` SHALL NOT probe
    /// the host... not reported as a host deficiency, since the host is
    /// not the reason it is absent").
    pub linux_probe: Option<HostProbe>,
    pub macos_probe: Option<HostProbe>,
}

impl Capability {
    /// The declared status on the platform this binary was compiled for.
    pub fn status_here(&self) -> PlatformStatus {
        #[cfg(target_os = "macos")]
        {
            self.macos
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.linux
        }
    }

    /// This host's live answer, if a probe exists for the platform this
    /// binary was compiled for — `None` when the declared status is
    /// `NotAdopted` (nothing to probe) or no probe was written.
    pub fn probe_here(&self) -> Option<HostProbe> {
        #[cfg(target_os = "macos")]
        {
            self.macos_probe
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.linux_probe
        }
    }
}

/// `landlock_scoping_available`: whether this kernel's Landlock ABI
/// supports the V6 scoping features (`Scope::Signal`,
/// `Scope::AbstractUnixSocket`) — shared by the signal-isolation and
/// abstract-unix-socket entries below, both of which rest on the same
/// ABI level. Mirrors `tests/abstract_socket_not_reachable.rs`'s own
/// gate: `nono::Sandbox::support_info().details` names every feature the
/// detected ABI has, and V6 scoping is reported there by name.
#[cfg(target_os = "linux")]
fn landlock_scoping_available() -> bool {
    crate::policy::backend_support()
        .details
        .contains("abstract UNIX socket scoping")
}

#[cfg(not(target_os = "linux"))]
fn landlock_scoping_available() -> bool {
    false
}

/// Wraps `fleet::mount::probe`, which needs the current binary's own
/// path to re-exec as `__mount_probe` — `doctor`'s existing namespace
/// report resolves it the identical way (`std::env::current_exe`).
fn mount_namespace_available() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| crate::fleet::mount::probe(&exe).ok())
        .unwrap_or(false)
}

/// Wraps `fleet::netns::probe`, same reasoning as
/// [`mount_namespace_available`].
fn network_namespace_available() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| crate::fleet::netns::probe(&exe).ok())
        .unwrap_or(false)
}

/// The declared capabilities. Order matches the spec's own task list
/// (`add-backend-capabilities` tasks.md task group 1), not alphabetical —
/// a reader comparing this file against that list should be able to walk
/// both in lockstep.
pub fn capabilities() -> &'static [Capability] {
    &[
        Capability {
            name: "filesystem-policy",
            description: "Path-level read/read-write grants, everything \
                not explicitly granted denied by default.",
            linux: PlatformStatus {
                status: Status::Enforced,
                evidence: "Landlock LSM; process_tier_landlock_boundaries.rs",
            },
            macos: PlatformStatus {
                status: Status::Enforced,
                evidence: "Seatbelt; process_tier_landlock_boundaries.rs \
                    covers the shared CapabilitySet construction, not a \
                    macOS host directly \u{2014} see the mount-isolation \
                    macOS caveat below for what remains unmeasured there",
            },
            linux_probe: Some(crate::policy::backend_supported),
            macos_probe: Some(crate::policy::backend_supported),
        },
        Capability {
            name: "network-block-and-ports",
            description: "Deny-by-default outbound TCP, with an explicit \
                per-port loopback allowlist for services and dev servers \
                (`network.ports`).",
            linux: PlatformStatus {
                status: Status::Enforced,
                evidence: "Landlock NetPort; udp_egress_denied.rs \
                    (the namespace half) and process_tier_landlock_boundaries.rs \
                    (the TCP block/port-allow half)",
            },
            macos: PlatformStatus {
                status: Status::EnforcedWithNamedDegradation,
                evidence: "Seatbelt network deny + scoped allow rules. \
                    Now measured on macOS 15.7.4, and the previous \
                    `enforced` was half right: outbound deny IS enforced \
                    (an ungranted destination is refused), but the \
                    per-port half is NOT. DEGRADATION: Seatbelt has no \
                    per-port form of `network-bind`, so granting any \
                    `network.ports` entry emits a blanket \
                    `(allow network-bind)`/`(allow network-inbound)` and \
                    the sandbox can listen on ports the manifest never \
                    granted \u{2014} measured, an ungranted port bound \
                    successfully. Landlock's NetPort does scope per port, \
                    so this is a platform difference. Surfaced at `up` by \
                    policy::degraded's `network.ports (per-port listen \
                    scoping)` warning; tests/network_ports_listen.rs \
                    asserts the Linux half.",
            },
            linux_probe: Some(crate::policy::backend_supported),
            macos_probe: Some(crate::policy::backend_supported),
        },
        Capability {
            name: "domain-filtering",
            description: "Restricting outbound connections to an \
                allowlist of domains, via the egress proxy's TLS SNI \
                inspection and a kernel-enforced proxy-only network mode.",
            linux: PlatformStatus {
                status: Status::Enforced,
                evidence: "NetworkMode::ProxyOnly is a Landlock NetPort \
                    rule permitting only the proxy's own port \u{2014} \
                    raw connect() to anything else fails at the kernel; \
                    tests/egress_proxy_e2e.rs",
            },
            macos: PlatformStatus {
                status: Status::Unverified,
                evidence: "policy::degraded currently asserts this is \
                    cooperative (unenforced) on macOS, but the pinned \
                    library's own ProxyOnly doc comment describes a \
                    *scoped* Seatbelt allow rule, which reads as \
                    enforced, not cooperative. Neither claim has been \
                    run on a macOS host \u{2014} this project has none. \
                    See design.md C2; do not resolve this from argument.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "signal-isolation",
            description: "A sandboxed process cannot signal (kill, stop, \
                ...) processes outside its own sandbox, and nothing \
                outside can signal into it \u{2014} the one library \
                capability knob devcroft actually sets.",
            linux: PlatformStatus {
                status: Status::EnforcedWithNamedDegradation,
                evidence: "SignalMode::Isolated maps to Landlock's \
                    Scope::Signal, requiring ABI V6+; on an older kernel \
                    apply_auto falls back below V6 and signal scoping is \
                    not enforced, degrading silently to whatever the \
                    kernel's own process/session boundaries provide. \
                    capability_set.rs's set_signal_mode call; \
                    keeper::connection tests exercise the isolated case",
            },
            macos: PlatformStatus {
                status: Status::Unverified,
                evidence: "Seatbelt signal scoping is configured \
                    identically via the same set_signal_mode call, but \
                    has not been run on a macOS host.",
            },
            linux_probe: Some(landlock_scoping_available),
            macos_probe: None,
        },
        Capability {
            name: "abstract-unix-sockets",
            description: "A sandboxed process cannot connect() to an \
                abstract (`@`-prefixed, no filesystem path) unix socket \
                outside its own sandbox \u{2014} dbus, X11, PipeWire, \
                systemd-journald on a typical desktop.",
            linux: PlatformStatus {
                status: Status::EnforcedWithNamedDegradation,
                evidence: "IpcMode::SharedMemoryOnly is nono's own \
                    #[default], never overridden by \
                    policy::capability_set.rs, and requests Landlock's \
                    Scope::AbstractUnixSocket whenever the ABI supports \
                    scoping (V6+) \u{2014} true today with zero code \
                    change, false on an older kernel, where this \
                    degrades to unenforced with no warning surfaced yet. \
                    tests/abstract_socket_not_reachable.rs, live: \
                    devcroft's real CapabilitySet against a real \
                    abstract socket gets EPERM. See docs/known-gaps.md \
                    for the corrected history \u{2014} this was believed \
                    open until this change traced it.",
            },
            macos: PlatformStatus {
                status: Status::Unverified,
                evidence: "Seatbelt has no scoping-ABI equivalent \
                    examined; unmeasured on a macOS host.",
            },
            linux_probe: Some(landlock_scoping_available),
            macos_probe: None,
        },
        Capability {
            name: "pathname-unix-sockets",
            description: "A sandboxed process cannot connect() to a \
                pathname unix socket outside its filesystem view \u{2014} \
                the nix daemon socket being the measured instance that \
                mattered (sandbox-provisioning P2a/P2b).",
            linux: PlatformStatus {
                status: Status::Enforced,
                evidence: "add-mount-isolation's per-sandbox mount \
                    namespace and filesystem view \u{2014} an ungranted \
                    socket's path does not resolve at all. \
                    tests/unix_socket_not_mediated.rs, inverted to \
                    assert refusal; verified live against a real nix \
                    daemon via the real up/status/exec/down CLI.",
            },
            macos: PlatformStatus {
                status: Status::EnforcedWithNamedDegradation,
                evidence: "Seatbelt classifies unix-socket connect() as \
                    network-outbound, so `network.default = \"deny\"` \
                    mediates it with no mount view needed \u{2014} measured \
                    live on macOS 15.7.4 (arm64) against a real nix daemon \
                    socket: refused EPERM, and reachable again only via an \
                    explicit unix-socket grant \
                    (add-macos-unix-socket-scoping task 0; \
                    tests/unix_socket_not_mediated.rs, macOS half). \
                    DEGRADATION, two named parts. (1) The guarantee is \
                    scoped to deny-default sandboxes: an `allow`-default \
                    macOS sandbox still reaches any world-accessible \
                    socket, where a Linux one does not, because a mount \
                    view removes the path regardless of network mode. \
                    (2) It is reachability only, not a filesystem view \u{2014} \
                    macOS has no user/mount namespace, so nothing narrows \
                    what the sandbox can see, only what it can dial.",
            },
            linux_probe: Some(mount_namespace_available),
            macos_probe: Some(crate::policy::backend_supported),
        },
        Capability {
            name: "process-info-isolation",
            description: "Whether a sandboxed process can see other \
                processes' existence, command lines, and metadata \
                (ProcessInfoMode) beyond its own sandbox.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "policy::capability_set.rs never calls \
                    set_process_info_mode; devcroft inherits the \
                    library's own default (Isolated), which already \
                    denies rather than grants, so nothing is silently \
                    widened \u{2014} but nothing records that the \
                    project depends on this default holding either.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same: never configured on either platform.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "resource-limits",
            description: "CPU/memory/process-count ceilings on a \
                sandbox, preventing one runaway build from starving \
                every other sandbox on the host.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "nono's ResourceLimits is a declaration only; \
                    rendering it to cgroups lives in nono-cli, which \
                    use-nono-library stopped depending on \
                    (confirmed in add-linux-agent-fleet task 0). A \
                    working reference (nono-cli's resource_cgroup.rs, \
                    Apache-2.0) exists to adapt when this is adopted.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same: no cgroup equivalent wired on either \
                    platform.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "snapshot-and-undo",
            description: "Filesystem snapshot/rollback of a sandbox's \
                writable paths.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "No consumer; the library's snapshot/undo \
                    module is unused.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "keystore",
            description: "The library's own credential storage \
                primitive.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "No consumer; devcroft's proxy handles \
                    credentials with its own per-session token instead \
                    (add-egress-proxy task group 4a).",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "audit-log",
            description: "An append-only, tamper-evident record of \
                sandbox activity.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "No consumer today, with one named: \
                    add-agent-interaction's durable record should be \
                    nono's append-only NDJSON with a rolling chain hash \
                    and Merkle commit, rather than a second log format \
                    devcroft would maintain itself.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "credential-brokering",
            description: "Phantom/broker tokens (nono-proxy's \
                jwt_phantom) so a sandbox never holds a real credential, \
                only one the proxy exchanges on its behalf.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "nono-proxy is not a dependency \
                    (docs/roadmap.md: 116 crates, not yet taken). The \
                    auth gap that first motivated looking at it \u{2014} \
                    an unauthenticated loopback proxy \u{2014} was \
                    closed directly in devcroft's own proxy instead \
                    (add-egress-proxy task group 4a). This capability \
                    alone is what would justify adopting the crate now, \
                    not the thing that made it urgent.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same: not a dependency on either platform.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "l7-endpoint-policy",
            description: "Method- and path-scoped egress rules \
                (`SERVICE:METHOD:PATH`), narrower than a domain \
                allowlist.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "No consumer yet; devcroft's own proxy filters \
                    by domain only. Recorded because \"allow github.com, \
                    GET only\" is a real want no current change \
                    expresses.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "tls-interception",
            description: "Decrypting and re-encrypting a sandbox's own \
                outbound TLS to inspect it beyond SNI.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "By decision, not omission: add-egress-proxy \
                    explicitly refuses this as a non-goal. If \
                    nono-proxy is ever adopted, this capability arrives \
                    with it and must stay refused deliberately, not by \
                    default.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same decision, both platforms.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "spiffe-identity",
            description: "SPIFFE/SPIRE workload identity issuance for a \
                sandbox.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Out of scope; would arrive only as a side \
                    effect of adopting nono-proxy, and is not itself a \
                    devcroft want.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "aws-request-routing",
            description: "Routing/signing AWS API requests through the \
                proxy layer.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Out of scope, same reasoning as \
                    spiffe-identity.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "runtime-capability-approval",
            description: "Turning a policy denial into a request an \
                operator can answer live, rather than a hard refusal \
                (the library's `supervisor` mechanism).",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Shipped in the library, never wired to \
                    anything \u{2014} devcroft always fails closed \
                    today. add-agent-interaction is the named consumer: \
                    it adopts this to turn a denial into an operator \
                    prompt.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same.",
            },
            linux_probe: None,
            macos_probe: None,
        },
        Capability {
            name: "per-agent-network-namespace",
            description: "Each fleet agent gets its own network \
                namespace and port table, so N agents can each bind the \
                same declared port without colliding.",
            linux: PlatformStatus {
                status: Status::Enforced,
                evidence: "fleet::netns::enter_network_namespace + \
                    bring_loopback_up; tests/fleet_netns.rs",
            },
            macos: PlatformStatus {
                status: Status::Unsupported,
                evidence: "Linux network namespaces (unshare(CLONE_NEWNET)) \
                    have no macOS equivalent.",
            },
            linux_probe: Some(network_namespace_available),
            macos_probe: None,
        },
        Capability {
            name: "inter-sandbox-process-visibility",
            description: "Whether one sandbox can see or signal another \
                sandbox's processes on the same host.",
            linux: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "No PID namespace is taken (add-mount-isolation \
                    design.md Open Question 2, deliberately left to \
                    fleet's own D2) \u{2014} every sandbox on a host \
                    currently shares the host's PID namespace and can \
                    enumerate every other sandbox's processes. Published \
                    as a known limitation, not hidden: docs/known-gaps.md, \
                    README.",
            },
            macos: PlatformStatus {
                status: Status::NotAdopted,
                evidence: "Same: no per-sandbox process isolation on \
                    either platform outside fleet.",
            },
            linux_probe: None,
            macos_probe: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: "Every claim names its evidence." A blank or placeholder
    /// evidence string is exactly the failure mode this whole change
    /// exists to prevent, so it is enforced structurally rather than
    /// left to review.
    #[test]
    fn every_entry_has_nonempty_evidence_on_both_platforms() {
        for cap in capabilities() {
            assert!(
                !cap.linux.evidence.trim().is_empty(),
                "{}: linux evidence is empty",
                cap.name
            );
            assert!(
                !cap.macos.evidence.trim().is_empty(),
                "{}: macos evidence is empty",
                cap.name
            );
        }
    }

    /// Spec: "`enforced-with-named-degradation` versus `enforced`... must
    /// say so where the claim is made" — a degraded entry whose evidence
    /// does not actually explain the degradation is indistinguishable
    /// from plain `Enforced` to a reader who only sees the status word.
    #[test]
    fn degraded_entries_explain_the_degradation_in_their_own_evidence() {
        for cap in capabilities() {
            for (platform, status) in [("linux", cap.linux), ("macos", cap.macos)] {
                if status.status == Status::EnforcedWithNamedDegradation {
                    assert!(
                        status.evidence.len() > 40,
                        "{} ({platform}): EnforcedWithNamedDegradation needs a real \
                         explanation, not a one-line evidence citation",
                        cap.name
                    );
                }
            }
        }
    }

    /// Spec: "`doctor` SHALL NOT probe the host" for `not-adopted`
    /// capabilities. Enforced structurally: a `NotAdopted` entry that
    /// carries a probe would make `doctor` do exactly the probing the
    /// spec forbids.
    #[test]
    fn not_adopted_entries_carry_no_host_probe() {
        for cap in capabilities() {
            if cap.linux.status == Status::NotAdopted {
                assert!(
                    cap.linux_probe.is_none(),
                    "{}: not-adopted on Linux but carries a linux_probe",
                    cap.name
                );
            }
            if cap.macos.status == Status::NotAdopted {
                assert!(
                    cap.macos_probe.is_none(),
                    "{}: not-adopted on macOS but carries a macos_probe",
                    cap.name
                );
            }
        }
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = capabilities().iter().map(|c| c.name).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate capability name");
    }
}
