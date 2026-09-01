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
    // Where every port in this policy actually lives — its own line
    // rather than a footnote under `network.ports`, because network
    // isolation is a property of the *sandbox*, not of the ports.
    //
    // The first version of this annotated the ports section instead,
    // which silently omitted the case that matters most: a manifest
    // declaring services and no ports still gets its own namespace (the
    // services bind ports of their own, from the provider's
    // declarations), and with nothing in `network.ports` to annotate,
    // the rendered policy said nothing at all about it.
    //
    // Two sandboxes whose rendered policy is byte-identical must not
    // behave differently, and "can the host reach this" is the first
    // difference a user meets. Stated as a condition where it is one:
    // `--render` reads the manifest alone, so it cannot know whether the
    // provider declares services, and cannot know whether this host
    // supports namespaces at all — only a real `up` probes that, and
    // warns there when it does not.
    writeln!(out, "network.namespace: {}", namespace_summary(compiled)).unwrap();
    // `network_proxy_port` is `None` until an actual `up` starts the
    // proxy and folds it in (`CompiledPolicy::with_proxy_port`'s doc) —
    // a fresh `policy --render` against the manifest alone can only ever
    // show whether filtering is *requested*, matching the same
    // provider-grants-are-live-only caveat this command already carries.
    match compiled.network_proxy_port {
        Some(port) => writeln!(out, "network.proxy: 127.0.0.1:{port} (running)").unwrap(),
        None if compiled.wants_egress_proxy() => {
            writeln!(out, "network.proxy: requested, not yet started (run `up`)").unwrap()
        }
        None => writeln!(out, "network.proxy: not requested").unwrap(),
    }
    out
}

/// How [`render`] describes where this sandbox's ports live.
///
/// Mirrors `CompiledPolicy::wants_network_isolation` exactly, including
/// its two-part condition — a sandbox is isolated when egress is denied
/// *and* there is something to isolate, which is either a declared port
/// or a provider-declared service. Only the first of those is visible in
/// a manifest, hence the conditional wording in the middle arm.
fn namespace_summary(compiled: &CompiledPolicy) -> &'static str {
    if !compiled.network_block {
        // `default = "allow"` never isolates: an isolated namespace has
        // no route to the real network, and with no proxy there is
        // nothing to relay through.
        "shared with the host (network.default = \"allow\")"
    } else if !compiled.network_ports.is_empty() {
        "own (declared ports are reachable inside the sandbox and via \
         `ssh -L <local>:127.0.0.1:<port> <name>.devcroft`, not on the host's loopback)"
    } else {
        // No longer conditional on services: every `network.default =
        // \"deny\"` sandbox is isolated, because Landlock's network rules
        // are TCP-only and the namespace is what denies UDP.
        "own (denies UDP, which Landlock's TCP-only network rules do not)"
    }
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
