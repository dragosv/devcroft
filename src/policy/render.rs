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
    // Rendered unconditionally, on both platforms, because the policy
    // invariant is that nothing reaches the backend that `--render` cannot
    // show — and the Linux/macOS split here is about which platform *acts*
    // on the entry, not about which one compiles it.
    render_section(&mut out, "unix_socket.bind", &compiled.unix_socket_bind);
    writeln!(out).unwrap();
    render_filesystem_view_note(&mut out);
    out
}

/// `add-mount-isolation` spec: "`policy --render` SHALL show what a
/// sandbox's view contains, with the same origin attribution every other
/// compiled rule carries."
///
/// **Not a second rendering of the same paths — a note explaining that
/// the sections already above are that rendering.** `fleet::mount::
/// construct_view`'s grants come from `CapabilityPlan::resolved_grants`,
/// which reads exactly `filesystem_allow` and `filesystem_read` — the
/// same two lists `filesystem.allow`/`filesystem.read` above already
/// render, origins included (`policy/capability_set.rs`'s own doc: the
/// two are computed by one shared resolver specifically so they cannot
/// diverge). A second, separate "filesystem.view" section repeating the
/// identical entries would violate the render module's own determinism
/// property in spirit even if not in fact — two sections that must
/// always agree are one invariant away from silently not agreeing.
///
/// **`/proc` is named as an unbounded exposure, not folded into "exactly
/// the granted paths" — found by adversarial review, and correctly.** An
/// earlier version of this note said the view contains "exactly" the
/// granted paths "plus... the keeper's own /proc", which reads as a
/// small, scoped addition. It is not: `fleet::mount::mount_proc` binds
/// the host's *entire* procfs, so a sandbox can enumerate and read
/// `/proc/<pid>` for every process on the host, sandboxed or not (subject
/// to ordinary DAC — this is visibility, not access). That is
/// deliberate and load-bearing (`mount_proc`'s own doc: a fresh instance
/// would need PID-namespace ownership this change doesn't take), but a
/// reader of `policy --render` — the one command that exists specifically
/// so nothing reaches the backend unshown — deserves to be told the real
/// shape, not a phrase that reads as bounded when it is not.
///
/// What else is not in either list above, named here instead of being
/// left for a reader to discover by surprise: the view also always
/// contains a private `/tmp` (when `/tmp` is granted — never a bind of
/// the host's shared one), a minimal `/dev`, and the standard
/// merged-`/usr` compatibility symlinks (`/lib`, `/lib64`, `/bin`,
/// `/sbin`) where this host has them. None of these has a
/// manifest/provider/baseline origin to attribute, because none is a
/// policy *rule* — they are the keeper's own unconditional construction
/// requirements, the same category `KEEPER_SYSTEM_READ`'s entries above
/// are, just not expressed as compiled grants.
fn render_filesystem_view_note(out: &mut String) {
    // No mount namespace, no view — and saying otherwise would misdescribe
    // the boundary on the one command that exists so nothing reaches the
    // backend unshown. The unix-socket half is called out explicitly
    // because it is the part a reader would otherwise assume is lost with
    // the view: it is not, it just moves to the network axis
    // (add-macos-unix-socket-scoping, measured — see
    // tests/unix_socket_not_mediated.rs).
    if !cfg!(target_os = "linux") {
        writeln!(
            out,
            "filesystem.view: none — mount namespaces are Linux-only, so this platform \
             builds no filesystem view and nothing narrows what the sandbox can see \
             beyond the rules above. Reachability of unix sockets is unaffected by \
             that: under `network.default = \"deny\"` this platform's sandbox denies \
             connect() to any pathname unix socket it was not granted, on the network \
             axis instead (see `devcroft doctor`'s pathname-unix-sockets capability)"
        )
        .unwrap();
        return;
    }
    writeln!(
        out,
        "filesystem.view: every sandbox's mount view (add-mount-isolation) contains the \
         paths listed under filesystem.allow/filesystem.read above, bind-mounted \
         read-write/read-only to match, plus a private /tmp (when granted), a minimal /dev, \
         and merged-/usr compatibility symlinks — none of which has a rule origin, since none \
         is granted by the manifest or a provider. It ALSO contains the host's entire /proc \
         (bind-mounted, not a fresh instance): every process on this host is visible and \
         enumerable from inside the sandbox, not only the sandbox's own — a known, deliberate \
         gap (no PID namespace is taken; see docs/known-gaps.md), not a bounded keeper need"
    )
    .unwrap();
}

/// How [`render`] describes where this sandbox's ports live.
///
/// Mirrors `CompiledPolicy::wants_network_isolation` exactly, including
/// its two-part condition — a sandbox is isolated when egress is denied
/// *and* there is something to isolate, which is either a declared port
/// or a provider-declared service. Only the first of those is visible in
/// a manifest, hence the conditional wording in the middle arm.
fn namespace_summary(compiled: &CompiledPolicy) -> &'static str {
    // Namespaces are a Linux primitive; `fleet::netns` has no macOS
    // equivalent and `up` degrades there. Rendering "own" off Linux
    // described an isolation the sandbox will not get — and, worse,
    // attributed the UDP denial to it, which is then also untrue.
    if !cfg!(target_os = "linux") {
        return "shared with the host (network namespaces are Linux-only; \
                this platform has no equivalent)";
    }
    if !compiled.network_block {
        // `default = "allow"` never isolates: an isolated namespace has
        // no route to the real network, and with no proxy there is
        // nothing to relay through.
        "shared with the host (network.default = \"allow\")"
    } else if !compiled.network_ports.is_empty() {
        "own (declared ports are reachable inside the sandbox and via \
         `devcroft ssh -L`, not on the host's loopback)"
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

    /// `add-mount-isolation` spec: "policy --render SHALL show what a
    /// sandbox's view contains." Not a separate section (see
    /// `render_filesystem_view_note`'s own doc for why) — just confirms
    /// the note is actually there rather than a stale claim in a comment.
    ///
    /// Asserted per-platform, because off Linux the honest answer is that
    /// there is no view at all: claiming one would misdescribe the
    /// boundary on the one command that exists so nothing reaches the
    /// backend unshown. A single shared assertion here would have to be
    /// weak enough to pass against either message, which is precisely the
    /// vacuous-assertion trap `tests/unix_socket_not_mediated.rs`'s own
    /// platform split exists to avoid.
    #[test]
    fn render_explains_the_filesystem_view() {
        let (manifest, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        let out = render(&compile(&manifest));
        assert!(out.contains("filesystem.view:"));
        if cfg!(target_os = "linux") {
            assert!(out.contains("filesystem.allow/filesystem.read"));
        } else {
            assert!(
                out.contains("none \u{2014} mount namespaces are Linux-only"),
                "off Linux the note must say there is no view, not describe one: {out}"
            );
            // The reachability half must survive the absence of a view,
            // and must say so here — a reader who sees "no filesystem
            // view" would otherwise reasonably assume the unix-socket
            // guarantee went with it.
            assert!(
                out.contains("pathname unix socket"),
                "the note must still state what does hold off Linux: {out}"
            );
        }
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
