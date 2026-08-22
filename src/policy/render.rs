//! `devcroft policy --render`: a human-readable dump of the compiled
//! policy with every rule's origin shown alongside it.

use super::{AnnotatedValue, CompiledPolicy};
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
