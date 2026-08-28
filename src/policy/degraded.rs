//! Degraded-capability detection: compares requested policy aspects
//! against what the current host's backend can actually enforce, so `up`
//! can warn instead of silently granting a weaker guarantee than the
//! manifest asked for.
//!
//! MVP checks exactly one aspect — domain-level network filtering. Per
//! design.md's risk note: Linux Landlock (ABI 4+) can mandatorily deny
//! raw `connect()` to everything but the interception proxy, forcing
//! traffic through it; macOS Seatbelt has no equivalent mandatory
//! redirection, so a process that ignores the proxy env vars can reach a
//! host directly. Domain filtering is therefore enforced on Linux and
//! only cooperative (unenforced) on macOS.
//!
//! **Unverified on macOS as of `add-egress-proxy`, flagged rather than
//! silently trusted either way.** `NetworkMode::ProxyOnly`'s own doc
//! comment in the pinned library describes the macOS output as `(allow
//! network-outbound (remote tcp "localhost:PORT"))` — a *scoped* allow
//! rule, which under Seatbelt's own default-deny model would seem to
//! block other outbound the same way Landlock's `NetPort` does, not
//! merely fail to redirect it. That would make this module's claim
//! stale. This devcontainer is Linux-only, so that reading could not be
//! measured live the way this project measures everything else it
//! claims (CLAUDE.md's gVisor/Landlock findings, the two-phase execution
//! table) — the degraded-on-macOS behavior below is kept as the
//! conservative default specifically because it is unverified, not
//! because it is confirmed. Revisit with an actual macOS host before
//! either keeping or removing this warning on the strength of an
//! argument rather than a measurement.

use super::CompiledPolicy;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DegradedCapability {
    pub aspect: &'static str,
    pub reason: &'static str,
    pub fallback: &'static str,
}

impl fmt::Display for DegradedCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is degraded on this host: {} (fallback: {})",
            self.aspect, self.reason, self.fallback
        )
    }
}

/// Aspects of the compiled policy the current host's backend cannot fully
/// enforce. Empty when nothing is degraded.
pub fn detect(compiled: &CompiledPolicy) -> Vec<DegradedCapability> {
    detect_for_host(compiled, HostCapabilities::current())
}

/// Whether the process tier can actually enforce anything on this host,
/// and what it found — `doctor`'s backend line, and the honest gate for
/// any test that brings up a real process-tier sandbox.
///
/// This is a genuine kernel probe, not a version string or a binary
/// lookup. `nono::Sandbox::support_info` walks the Landlock ABI
/// candidates from V6 down to V1, building a real ruleset at each with
/// `CompatLevel::HardRequirement` and calling `create()` — the highest
/// one the running kernel accepts is the answer. It never calls
/// `restrict_self`, so probing restricts nothing and is safe to call
/// from anywhere, including a test process that goes on to do other
/// work. The result is cached in the library for the process's lifetime.
///
/// Worth stating because the obvious-looking alternative was wrong for
/// years: devcroft's tests used to gate on `nono --version` succeeding,
/// which only ever proved a binary was on `PATH`. It said nothing about
/// whether the kernel had Landlock at all — a container with it
/// compiled out, or an old kernel, passed that gate and then failed the
/// actual `up`.
pub fn backend_support() -> nono::SupportInfo {
    nono::Sandbox::support_info()
}

/// Shorthand for [`backend_support`]'s verdict.
pub fn backend_supported() -> bool {
    backend_support().is_supported
}

struct HostCapabilities {
    domain_filtering: bool,
}

impl HostCapabilities {
    #[cfg(target_os = "macos")]
    fn current() -> Self {
        HostCapabilities {
            domain_filtering: false,
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn current() -> Self {
        HostCapabilities {
            domain_filtering: true,
        }
    }
}

fn detect_for_host(compiled: &CompiledPolicy, host: HostCapabilities) -> Vec<DegradedCapability> {
    let mut degraded = Vec::new();

    let requests_domain_filtering =
        compiled.network_block && !compiled.network_allow_domain.is_empty();
    if requests_domain_filtering && !host.domain_filtering {
        degraded.push(DegradedCapability {
            aspect: "network.allow (domain-level filtering)",
            reason: "macOS Seatbelt cannot enforce domain-level network allowlists without a cooperative proxy",
            fallback: "network access is not filtered by domain on this host; a process that bypasses the proxy can reach any host",
        });
    }

    degraded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    fn blocked_with_allowlist() -> CompiledPolicy {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            allow = ["github.com"]
            "#,
        )
        .unwrap();
        super::super::compile(&manifest)
    }

    #[test]
    fn domain_filtering_degraded_when_host_cannot_enforce_it() {
        let compiled = blocked_with_allowlist();
        let degraded = detect_for_host(
            &compiled,
            HostCapabilities {
                domain_filtering: false,
            },
        );

        assert_eq!(degraded.len(), 1);
        assert_eq!(degraded[0].aspect, "network.allow (domain-level filtering)");
    }

    #[test]
    fn domain_filtering_not_degraded_when_host_can_enforce_it() {
        let compiled = blocked_with_allowlist();
        let degraded = detect_for_host(
            &compiled,
            HostCapabilities {
                domain_filtering: true,
            },
        );

        assert!(degraded.is_empty());
    }

    #[test]
    fn no_domain_allowlist_requested_is_never_degraded() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let compiled = super::super::compile(&manifest);

        let degraded = detect_for_host(
            &compiled,
            HostCapabilities {
                domain_filtering: false,
            },
        );
        assert!(degraded.is_empty());
    }

    #[test]
    fn network_default_allow_with_extra_domains_is_never_degraded() {
        // default = "allow" means network_block is false: nothing is
        // being filtered by domain, so there's nothing to degrade.
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            default = "allow"
            allow = ["github.com"]
            "#,
        )
        .unwrap();
        let compiled = super::super::compile(&manifest);

        let degraded = detect_for_host(
            &compiled,
            HostCapabilities {
                domain_filtering: false,
            },
        );
        assert!(degraded.is_empty());
    }

    #[test]
    fn warning_message_names_aspect_reason_and_fallback() {
        let cap = DegradedCapability {
            aspect: "network.allow (domain-level filtering)",
            reason: "test reason",
            fallback: "test fallback",
        };
        let msg = cap.to_string();
        assert!(msg.contains("network.allow (domain-level filtering)"));
        assert!(msg.contains("test reason"));
        assert!(msg.contains("test fallback"));
    }

    #[test]
    fn detect_does_not_panic_on_current_host() {
        let compiled = blocked_with_allowlist();
        let _ = detect(&compiled);
    }
}
