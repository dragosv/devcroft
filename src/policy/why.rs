//! `devcroft why`: explain a single ALLOWED/DENIED decision.
//!
//! Filesystem queries delegate the verdict itself to `nono why` (ad-hoc
//! query context built from the compiled policy), per design.md decision
//! 4 — the backend is authoritative on enforcement, devcroft is
//! authoritative on which of its own rules is responsible. Network host
//! queries are answered entirely from the compiled policy: `nono why`'s
//! ad-hoc query context has no flag for domain allowlists (only
//! `--block-net`), so delegating would silently misreport any host
//! covered by `network.allow` — see the "Degraded capability surfacing"
//! invariant in CLAUDE.md.

use super::{AnnotatedValue, CompiledPolicy, Origin};
use crate::paths::is_within;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Read,
    Write,
    ReadWrite,
}

impl Op {
    fn nono_flag(self) -> &'static str {
        match self {
            Op::Read => "read",
            Op::Write => "write",
            Op::ReadWrite => "readwrite",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub allowed: bool,
    /// The devcroft rule responsible, if the decision matched one of
    /// devcroft's own compiled rules rather than a backend-only default.
    pub origin: Option<Origin>,
    pub detail: String,
}

#[derive(Debug)]
pub enum WhyError {
    /// `nono why` could not be run or returned something devcroft cannot
    /// interpret — a backend-layer failure per CLAUDE.md's error contract.
    Backend(String),
}

impl fmt::Display for WhyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WhyError::Backend(msg) => write!(f, "backend: {msg}"),
        }
    }
}

impl std::error::Error for WhyError {}

/// Explain whether `op` on `path` would be allowed, delegating the verdict
/// to `nono why` and attributing it to devcroft's own compiled rule.
pub fn why_path(compiled: &CompiledPolicy, path: &str, op: Op) -> Result<Explanation, WhyError> {
    let allowed = nono_why_path(compiled, path, op)?;
    let origin = origin_for_path(compiled, path, op);
    let detail = match &origin {
        Some(o) => format!("{} by rule {o}", verdict_word(allowed)),
        None => format!(
            "{} by backend default policy (no matching devcroft rule)",
            verdict_word(allowed)
        ),
    };
    Ok(Explanation {
        allowed,
        origin,
        detail,
    })
}

/// Explain whether outbound access to `host` would be allowed. Answered
/// entirely from the compiled policy — see the module doc for why this
/// does not delegate to `nono why --host`.
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

fn verdict_word(allowed: bool) -> &'static str {
    if allowed { "allowed" } else { "denied" }
}

/// Find the devcroft rule responsible for `path` under `op`, if any.
/// Deny rules always win, matching the "baseline denials always win"
/// invariant; among allow-shaped rules, `filesystem.allow` grants both
/// read and write, `filesystem.read` grants read only.
fn origin_for_path(compiled: &CompiledPolicy, path: &str, op: Op) -> Option<Origin> {
    if let Some(rule) = matching(&compiled.filesystem_deny, path) {
        return Some(rule.origin.clone());
    }
    if matches!(op, Op::Write | Op::ReadWrite) {
        return matching(&compiled.filesystem_allow, path).map(|r| r.origin.clone());
    }
    matching(&compiled.filesystem_allow, path)
        .or_else(|| matching(&compiled.filesystem_read, path))
        .map(|r| r.origin.clone())
}

fn matching<'a>(rules: &'a [AnnotatedValue], path: &str) -> Option<&'a AnnotatedValue> {
    rules.iter().find(|r| is_within(path, &r.value))
}

fn nono_why_path(compiled: &CompiledPolicy, path: &str, op: Op) -> Result<bool, WhyError> {
    let mut cmd = std::process::Command::new("nono");
    cmd.arg("why")
        .arg("--json")
        .arg("--path")
        .arg(path)
        .arg("--op")
        .arg(op.nono_flag());
    for allow in &compiled.filesystem_allow {
        cmd.arg("--allow").arg(&allow.value);
    }
    for read in &compiled.filesystem_read {
        cmd.arg("--read").arg(&read.value);
    }
    if compiled.network_block {
        cmd.arg("--block-net");
    }

    let output = cmd
        .output()
        .map_err(|e| WhyError::Backend(format!("running `nono why`: {e}")))?;
    if !output.status.success() {
        return Err(WhyError::Backend(format!(
            "`nono why` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| WhyError::Backend(format!("parsing `nono why` output: {e}")))?;
    match json["status"].as_str() {
        Some("allowed") => Ok(true),
        Some("denied") => Ok(false),
        other => Err(WhyError::Backend(format!(
            "unexpected `nono why` status: {other:?}"
        ))),
    }
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
        let explanation = why_path(&compiled, "~/.aws/credentials", Op::Read).unwrap();

        assert!(!explanation.allowed);
        assert_eq!(explanation.origin, Some(Origin::Baseline));
    }

    #[test]
    fn why_path_allows_granted_dir_as_manifest() {
        let compiled = compile(&manifest_with_src_allow());
        let explanation = why_path(&compiled, "src/main.rs", Op::Write).unwrap();

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
        let explanation = why_path(&compiled, "docs/readme.md", Op::Write).unwrap();

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
