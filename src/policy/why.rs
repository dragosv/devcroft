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
    let (allowed, backend_group) = nono_why_path(compiled, path, op)?;
    // devcroft's own rules take priority when one matches — `origin_for_path`
    // already applies "deny wins" for rules devcroft compiled. Only once
    // none of them account for the verdict does a backend-attributed group
    // apply: it's a required deny group devcroft neither chose nor can
    // remove, distinct from `Baseline` for exactly that reason (Origin's
    // own doc comment).
    let origin =
        origin_for_path(compiled, path, op).or_else(|| backend_group.map(Origin::BackendEnforced));
    let detail = match (&origin, allowed) {
        (Some(o), _) => format!("{} by rule {o}", verdict_word(allowed)),
        // Denied with no devcroft rule and no backend group: nothing
        // grants the path at all, distinct from a rule actively denying it
        // (policy spec's "Ungranted path is distinguished from a denied
        // one" scenario).
        (None, false) => "denied: not granted by any rule".to_string(),
        // Allowed with no devcroft rule: one of the backend's non-excluded
        // optional groups (`user_tools`, `homebrew_linux`, `system_write_*`)
        // granted it — narrow, host-specific conveniences own-policy-
        // baseline leaves alone (only the broad system-read groups and the
        // inert command blocklist are excluded).
        (None, true) => "allowed by a backend group outside devcroft's own rules".to_string(),
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

/// `~/…` → `$HOME/…`, for values crossing into `nono why`'s *command-line
/// flags* only. nono expands `~` inside a profile file but not in
/// `--allow`/`--read` arguments, where it reads the literal `~` as a
/// relative path, finds nothing there, and drops the grant — silently as
/// far as the verdict goes, and noisily on **stdout**
/// (`'~/x' does not exist and will be ignored.`), which lands in the middle
/// of the JSON this function parses. Found via task 6.5: without this,
/// `why` on any manifest carrying a `~`-form grant failed outright with
/// `parsing 'nono why' output: expected value at line 1 column 1`, and
/// would have returned a wrong DENIED verdict even if it had parsed.
/// The profile written for `up` is deliberately left alone — nono does
/// expand `~` there, so the compiled policy stays in its own `~`-relative
/// form everywhere it is rendered or stored.
fn expand_home(value: &str) -> String {
    let Ok(home) = std::env::var("HOME") else {
        return value.to_string();
    };
    match value {
        "~" => home,
        v => match v.strip_prefix("~/") {
            Some(rest) => format!("{}/{rest}", home.trim_end_matches('/')),
            None => v.to_string(),
        },
    }
}

/// A scratch profile file for a single `nono why -p <file>` query, removed
/// on drop. Own-policy-baseline replaced the ad-hoc `--allow`/`--read`/
/// `--block-net` flags this used to build with the real compiled profile
/// (`CompiledPolicy::to_nono_profile`), because those flags have no way to
/// express `groups.exclude` — a query built from them silently answered
/// `ALLOWED` for a path like `/usr/bin/gcc` that `GROUPS_EXCLUDE` actually
/// denies, since nono's ad-hoc query context falls back to its own
/// baseline group set when nothing says otherwise. `-p <file>` uses the
/// exact same schema `up` writes to `profile.json`, so the query and the
/// real enforcement can no longer disagree about what's excluded.
struct QueryProfile(std::path::PathBuf);

impl QueryProfile {
    fn write(compiled: &CompiledPolicy) -> Result<Self, WhyError> {
        let path = std::env::temp_dir().join(format!(
            "devcroft-why-query-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::write(&path, compiled.to_nono_profile().to_json())
            .map_err(|e| WhyError::Backend(format!("writing query profile: {e}")))?;
        Ok(QueryProfile(path))
    }
}

impl Drop for QueryProfile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Returns the verdict plus, for a denial none of devcroft's own rules
/// caused, the backend group nono attributes it to (`policy_source:
/// "group:<name>"`).
fn nono_why_path(
    compiled: &CompiledPolicy,
    path: &str,
    op: Op,
) -> Result<(bool, Option<String>), WhyError> {
    let profile = QueryProfile::write(compiled)?;
    let mut cmd = std::process::Command::new("nono");
    cmd.arg("why")
        .arg("--json")
        // Expanded for the same reason `up` never expands the profile's own
        // paths but does expand this one: the query path is `~`-rooted in
        // the cli spec's own example (`why --path ~/.aws/credentials`), and
        // nono expands `~` inside a profile file but not in this flag,
        // where a literal `~` reads as relative-to-cwd and matches nothing.
        // `origin_for_path` keeps the caller's unexpanded form on purpose —
        // `paths::is_within` compares `~`-rooted rules against `~`-rooted
        // paths and treats home and absolute as separate namespaces.
        .arg("--path")
        .arg(expand_home(path))
        .arg("--op")
        .arg(op.nono_flag())
        .arg("-p")
        .arg(&profile.0);

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
    // `policy_source` is nono's own attribution for a denial none of
    // devcroft's compiled rules caused — one of the eight required deny
    // groups (own-policy-baseline policy spec: "A denial is attributed to
    // whoever imposed it"). Passed through rather than inferred, but the
    // exact string is not stable across nono releases: 0.71.0 reports
    // `"group:deny_shell_configs"`, naming the group; 0.74.0 collapsed
    // every required-group denial to the generic `"filesystem.deny"`,
    // with no group name recoverable from `why` at all (verified against
    // both versions live — an upstream regression in the diagnostic, not
    // in enforcement, which is identical). `path_not_granted` is the one
    // `reason` value confirmed stable across both: only that value means
    // "nothing grants this", so anything else on a `denied` verdict is
    // still a backend-imposed rule even when nono no longer names it.
    let backend_group = if json["reason"].as_str() == Some("path_not_granted") {
        None
    } else {
        json["policy_source"]
            .as_str()
            .map(|s| s.strip_prefix("group:").unwrap_or(s).to_string())
    };
    match json["status"].as_str() {
        Some("allowed") => Ok((true, backend_group)),
        Some("denied") => Ok((false, backend_group)),
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

    #[test]
    fn expand_home_rewrites_only_tilde_rooted_values() {
        let home = std::env::var("HOME").expect("HOME set in the test environment");
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~/.zed_server"), format!("{home}/.zed_server"));
        // Project-relative and absolute values are namespaces nono resolves
        // itself; a bare `~foo` is not a home reference at all.
        assert_eq!(expand_home("."), ".");
        assert_eq!(expand_home("/nix/store"), "/nix/store");
        assert_eq!(expand_home("~foo"), "~foo");
    }

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

    /// policy spec's "Backend-enforced denial names its group": `~/.bashrc`
    /// is denied by nono's required `deny_shell_configs` group, which
    /// devcroft neither compiles nor can exclude — distinct from
    /// `Origin::Baseline`, which means devcroft chose the denial itself.
    ///
    /// Asserts the shape (`BackendEnforced`, not `None`) rather than the
    /// exact group name: verified live that nono 0.74.0 no longer reports
    /// which required group fired (`policy_source` collapses to the
    /// generic `"filesystem.deny"`, `why.rs`'s own doc comment on
    /// `backend_group` has the detail), so pinning the name would make
    /// this test version-dependent for a fact the test doesn't need.
    #[test]
    fn why_path_attributes_backend_enforced_denial_to_a_backend_group() {
        let compiled = compile(&manifest_with_src_allow());
        let explanation = why_path(&compiled, "~/.bashrc", Op::Read).unwrap();

        assert!(!explanation.allowed);
        assert!(
            matches!(explanation.origin, Some(Origin::BackendEnforced(_))),
            "expected a BackendEnforced origin, got {:?}",
            explanation.origin
        );
        assert!(explanation.detail.starts_with("denied by rule backend:"));
    }

    /// policy spec's "Ungranted path is distinguished from a denied one":
    /// `/usr/bin/gcc` is a host binary own-policy-baseline's `GROUPS_EXCLUDE`
    /// makes unreachable — no devcroft rule denies it and no backend group
    /// denies it either, it is simply never granted.
    #[test]
    fn why_path_ungranted_host_binary_has_no_origin() {
        let compiled = compile(&manifest_with_src_allow());
        let explanation = why_path(&compiled, "/usr/bin/gcc", Op::Read).unwrap();

        assert!(!explanation.allowed);
        assert_eq!(explanation.origin, None);
        assert_eq!(explanation.detail, "denied: not granted by any rule");
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
