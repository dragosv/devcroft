//! `devcroft policy --render`: a human-readable dump of the compiled
//! policy with every rule's origin shown alongside it.

use super::{AnnotatedValue, BACKEND_ENFORCED_GROUPS, CompiledPolicy, Origin, WhyError};
use std::fmt::Write;

/// Render `compiled` for `policy --render`. Deterministic: same input,
/// same output, since it walks [`CompiledPolicy`]'s already-ordered lists.
pub fn render(compiled: &CompiledPolicy) -> String {
    let mut out = String::new();
    writeln!(out, "sandbox: {}", compiled.sandbox_name).unwrap();
    writeln!(out).unwrap();
    render_section(&mut out, "filesystem.allow", &compiled.filesystem_allow);
    render_section(&mut out, "filesystem.read", &compiled.filesystem_read);
    render_section(&mut out, "filesystem.deny", &compiled.filesystem_deny);
    writeln!(out).unwrap();
    writeln!(out, "network.block: {}", compiled.network_block).unwrap();
    render_section(
        &mut out,
        "network.allow_domain",
        &compiled.network_allow_domain,
    );
    // Rendered even though it is a different value type, for the reason
    // the whole command exists: nothing may reach the backend that
    // `--render` cannot show. An `open_port` in `profile.json` that the
    // rendered policy omitted would be exactly the invisible rule this
    // invariant forbids.
    writeln!(out, "network.ports:").unwrap();
    if compiled.network_ports.is_empty() {
        writeln!(out, "  (none)").unwrap();
    } else {
        for p in &compiled.network_ports {
            writeln!(out, "  {:<40} {}", p.value, p.origin).unwrap();
        }
    }
    out
}

fn render_section(out: &mut String, title: &str, values: &[AnnotatedValue]) {
    writeln!(out, "{title}:").unwrap();
    if values.is_empty() {
        writeln!(out, "  (none)").unwrap();
        return;
    }
    for v in values {
        writeln!(out, "  {:<40} {}", v.value, v.origin).unwrap();
    }
}

/// The other half of `policy --render`'s completeness (own-policy-baseline
/// Decision 5, policy spec's "Rendering accounts for every rule reaching
/// the backend"): `render` above covers what devcroft compiled, this
/// covers [`BACKEND_ENFORCED_GROUPS`] — the groups that reach the backend
/// regardless, sourced from `nono profile groups <name> --json` rather
/// than a list devcroft maintains, so a change to what those groups
/// actually grant shows up here without a devcroft release. Backend-
/// dependent (unlike `render`, a pure function of `CompiledPolicy`) —
/// design.md's own stated cost of this decision, not an oversight.
pub fn render_backend_enforced() -> Result<String, WhyError> {
    let mut out = String::new();
    writeln!(
        out,
        "backend-enforced (reached regardless of what devcroft compiles):"
    )
    .unwrap();
    let mut any = false;
    for group in BACKEND_ENFORCED_GROUPS {
        let output = std::process::Command::new("nono")
            .arg("profile")
            .arg("groups")
            .arg(group)
            .arg("--json")
            .output()
            .map_err(|e| {
                WhyError::Backend(format!("running `nono profile groups {group}`: {e}"))
            })?;
        if !output.status.success() {
            return Err(WhyError::Backend(format!(
                "`nono profile groups {group}` exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
            WhyError::Backend(format!("parsing `nono profile groups {group}` output: {e}"))
        })?;
        for path in group_paths(&json) {
            any = true;
            writeln!(
                out,
                "  {:<40} {}",
                path,
                Origin::BackendEnforced(group.to_string())
            )
            .unwrap();
        }
    }
    if !any {
        writeln!(out, "  (none)").unwrap();
    }
    Ok(out)
}

/// Every path a group's `--json` output names — under `allow.read`,
/// `allow.write`, or `deny.access` (the three shapes [`BACKEND_ENFORCED_GROUPS`]'s
/// members actually use, verified against all thirteen live). The `raw`
/// form, not `expanded`: devcroft renders every other path `~`-relative
/// too, and mixing forms here would make the two halves of `--render`
/// inconsistent with each other for no reason.
fn group_paths(json: &serde_json::Value) -> Vec<String> {
    ["allow.read", "allow.write", "deny.access"]
        .iter()
        .flat_map(|pointer_path| {
            let mut v = json;
            for key in pointer_path.split('.') {
                v = &v[key];
            }
            v.as_array().cloned().unwrap_or_default()
        })
        .filter_map(|entry| entry["raw"].as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::{Origin, compile};
    use super::*;
    use crate::config::parse;

    #[test]
    fn render_shows_manifest_and_baseline_origins() {
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
        let out = render(&compile(&manifest));

        assert!(out.contains("sandbox: myproj"));
        assert!(out.contains("src") && out.contains("manifest:filesystem.allow"));
        assert!(out.contains("~/.ssh") && out.contains("baseline"));
        assert!(out.contains("network.block: false"));
        assert!(out.contains("github.com") && out.contains("manifest:network.allow"));
    }

    /// policy spec's "Rendering accounts for every rule reaching the
    /// backend" / own-policy-baseline Decision 5. Self-skips when nono is
    /// absent, like every other real-tooling test in this crate.
    #[test]
    fn render_backend_enforced_shows_required_and_optional_groups() {
        if std::process::Command::new("nono")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let out = render_backend_enforced().unwrap();

        assert!(out.contains("backend-enforced"));
        // A required deny group (credentials) and an optional allow-shaped
        // one (user_tools) — proves both shapes (`deny.access`,
        // `allow.read`/`allow.write`) are rendered, not just one.
        assert!(out.contains("~/.ssh") && out.contains("backend:deny_credentials"));
        assert!(out.contains("backend:user_tools"));
    }

    #[test]
    fn render_is_deterministic() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        assert_eq!(render(&compile(&manifest)), render(&compile(&manifest)));
    }

    #[test]
    fn empty_section_prints_none() {
        // filesystem.allow defaults to ["."] and filesystem.read is never
        // empty since own-policy-baseline (it always carries the keeper's
        // own baseline system grants, KEEPER_SYSTEM_READ) — neither is a
        // reliable "nothing granted" case any more. network.allow_domain
        // still is: nothing populates it unless the manifest declares
        // `network.allow`.
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let out = render(&compile(&manifest));
        assert!(out.contains("network.allow_domain:\n  (none)"));
    }

    #[test]
    fn origin_display_matches_spec_vocabulary() {
        assert_eq!(
            Origin::Manifest("filesystem.allow").to_string(),
            "manifest:filesystem.allow"
        );
        assert_eq!(Origin::Provider("flox").to_string(), "provider:flox");
        assert_eq!(Origin::Baseline.to_string(), "baseline");
    }
}
