//! `devcroft why`: explain a single ALLOWED/DENIED decision.
//!
//! Under own-policy-baseline this delegated the filesystem verdict to
//! `nono why`, since enforcement went through an external backend devcroft
//! didn't fully control. `use-nono-library` removed that: self-restriction
//! is now a pure `CapabilitySet` application of exactly what
//! `CompiledPolicy` records, with no group injection and no backend
//! process to ask. `why` is therefore a pure function again — devcroft's
//! own compiled rules *are* the verdict, not just the origin attribution
//! for someone else's verdict.

use super::{AnnotatedValue, CompiledPolicy, Origin};
use crate::paths::is_within;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub allowed: bool,
    /// The devcroft rule responsible. `None` only when denied and nothing
    /// grants the path at all — distinct from a rule actively denying it
    /// (policy spec's "Ungranted path is distinguished from a denied
    /// one" scenario). Under the library-based process tier this is the
    /// only way to be denied with no origin: there is no backend group
    /// left that could grant something outside devcroft's own rules.
    pub origin: Option<Origin>,
    pub detail: String,
}

/// Explain whether `op` on `path` would be allowed. Deny rules always win
/// (`filesystem_deny`, matching the "baseline denials always win"
/// invariant); among allow-shaped rules, `filesystem.allow` grants both
/// read and write, `filesystem.read` grants read only.
pub fn why_path(compiled: &CompiledPolicy, path: &str, op: Op) -> Explanation {
    if let Some(rule) = matching(&compiled.filesystem_deny, path) {
        return Explanation {
            allowed: false,
            detail: format!("denied by rule {}", rule.origin),
            origin: Some(rule.origin.clone()),
        };
    }
    let allow = if matches!(op, Op::Write | Op::ReadWrite) {
        matching(&compiled.filesystem_allow, path)
    } else {
        matching(&compiled.filesystem_allow, path)
            .or_else(|| matching(&compiled.filesystem_read, path))
    };
    match allow {
        Some(rule) => Explanation {
            allowed: true,
            detail: format!("allowed by rule {}", rule.origin),
            origin: Some(rule.origin.clone()),
        },
        None => Explanation {
            allowed: false,
            origin: None,
            detail: "denied: not granted by any rule".to_string(),
        },
    }
}

/// Explain whether outbound access to `host` would be allowed. See the
/// module doc's `network.allow` note: it compiles to a rule here (so
/// `why --host` still explains it), but — same as before this change,
/// design.md's own recorded Non-Goal — is not actually a working domain
/// filter under either the old exec-based process tier or this one.
pub fn why_host(compiled: &CompiledPolicy, host: &str) -> Explanation {
    if !compiled.network_block {
        return Explanation {
            allowed: true,
            origin: Some(Origin::Manifest("network.default")),
            detail: "allowed by rule manifest:network.default".to_string(),
        };
    }
    if let Some(rule) = compiled
        .network_allow_domain
        .iter()
        .find(|d| d.value == host)
    {
        return Explanation {
            allowed: true,
            origin: Some(rule.origin.clone()),
            detail: format!("allowed by rule {}", rule.origin),
        };
    }
    Explanation {
        allowed: false,
        origin: Some(Origin::Manifest("network.default")),
        detail: "denied by rule manifest:network.default (host not in network.allow)".to_string(),
    }
}

fn matching<'a>(rules: &'a [AnnotatedValue], path: &str) -> Option<&'a AnnotatedValue> {
    rules.iter().find(|r| is_within(path, &r.value))
}

#[cfg(test)]
mod tests {
    use super::super::compile;
    use super::*;
    use crate::config::parse;

    fn manifest_with_src_allow() -> crate::config::Manifest {
        parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = ["src"]
            "#,
        )
        .unwrap()
        .0
    }

    #[test]
    fn why_path_denies_credential_dir_as_baseline() {
        let compiled = compile(&manifest_with_src_allow());
        let explanation = why_path(&compiled, "~/.aws/credentials", Op::Read);

        assert!(!explanation.allowed);
        assert_eq!(explanation.origin, Some(Origin::Baseline));
    }

    /// Under the library-based process tier, a path nothing grants is
    /// simply denied by Landlock's own default — there is no backend
    /// group left that could grant it outside devcroft's own rules, so
    /// `~/.bashrc` (a nono-cli `deny_shell_configs` concern the process
    /// tier no longer enforces at all, design.md Decision 5) is denied
    /// the same way any other ungranted path is: no rule, no origin.
    #[test]
    fn why_path_ungranted_path_has_no_origin() {
        let compiled = compile(&manifest_with_src_allow());
        let explanation = why_path(&compiled, "~/.bashrc", Op::Read);

        assert!(!explanation.allowed);
        assert_eq!(explanation.origin, None);
        assert_eq!(explanation.detail, "denied: not granted by any rule");
    }

    #[test]
    fn why_path_ungranted_host_binary_has_no_origin() {
        let compiled = compile(&manifest_with_src_allow());
        let explanation = why_path(&compiled, "/usr/bin/gcc", Op::Read);

        assert!(!explanation.allowed);
        assert_eq!(explanation.origin, None);
        assert_eq!(explanation.detail, "denied: not granted by any rule");
    }

    #[test]
    fn why_path_allows_granted_dir_as_manifest() {
        let compiled = compile(&manifest_with_src_allow());
        let explanation = why_path(&compiled, "src/main.rs", Op::Write);

        assert!(explanation.allowed);
        assert_eq!(
            explanation.origin,
            Some(Origin::Manifest("filesystem.allow"))
        );
    }

    #[test]
    fn why_path_write_is_denied_for_read_only_grant() {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = []
            read = ["docs"]
            "#,
        )
        .unwrap();
        let compiled = compile(&manifest);
        let explanation = why_path(&compiled, "docs/readme.md", Op::Write);

        // devcroft attributes no rule: `filesystem.read` never grants write.
        assert_eq!(explanation.origin, None);
    }

    #[test]
    fn why_host_allowed_when_default_allow() {
        let (manifest, _) =
            parse("[sandbox]\nname = \"myproj\"\n[network]\ndefault = \"allow\"\n").unwrap();
        let explanation = why_host(&compile(&manifest), "example.com");
        assert!(explanation.allowed);
        assert_eq!(
            explanation.origin,
            Some(Origin::Manifest("network.default"))
        );
    }

    #[test]
    fn why_host_allowed_when_in_allowlist() {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            allow = ["github.com"]
            "#,
        )
        .unwrap();
        let explanation = why_host(&compile(&manifest), "github.com");
        assert!(explanation.allowed);
        assert_eq!(explanation.origin, Some(Origin::Manifest("network.allow")));
    }

    #[test]
    fn why_host_denied_when_not_in_allowlist() {
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            allow = ["github.com"]
            "#,
        )
        .unwrap();
        let explanation = why_host(&compile(&manifest), "evil.example");
        assert!(!explanation.allowed);
        assert_eq!(
            explanation.origin,
            Some(Origin::Manifest("network.default"))
        );
    }
}
