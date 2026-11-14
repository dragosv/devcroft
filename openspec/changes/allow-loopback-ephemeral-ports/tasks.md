# Tasks — Loopback ports chosen at runtime

## 0. Measure before building

> L1 rests on one claim, that the route-less namespace is the whole boundary.
> D9 measured it for egress. These tasks measure the two things D9 did not
> look at: what *else* listens inside a single-sandbox namespace, and what
> Landlock itself would allow. If 0.2 finds a listener that is not harmless,
> stop and revise design.md before any code.

- [x] 0.1 Record the motivating failure as a reproducible check: in
      `samples/devenv-antigravity-sample` (already written, uncommitted),
      `devcroft exec devenvagy -- agy -p ...` exits with `listen tcp
      127.0.0.1:0: bind: permission denied` (in the sandbox home's
      `.gemini/antigravity-cli/cli.log`). Verified by the log line, and
      recorded in design.md.
- [x] 0.2 With a deny sandbox up (allowlist and one declared service, so the
      relay and the supervisor both exist), list every TCP listener and
      every route inside its namespace (`/proc/net/tcp`, `/proc/net/route`
      from inside a session). Verified by the list in design.md, with each
      listener classified as harmless or a reason to stop.
- [x] 0.3 Measure L1 directly on this host, before touching devcroft:
      - in a route-less user+net namespace with Landlock net *unhandled*, a
        process binds `127.0.0.1:0` and connects to it;
      - a connect to a non-loopback address fails `ENETUNREACH`;
      - a connect to a port a host process listens on, on the host's
        loopback, does not reach it.

      Verified by the three outcomes, recorded in design.md.
- [x] 0.4 Measure the alternative L1 rejects, so the rejection is evidence
      rather than reasoning: does a Landlock rule for port 0 allow an
      ephemeral bind on this ABI, and does the subsequent connect fail?
      Verified by both outcomes, recorded in design.md.
- [x] 0.5 Confirm how nono expresses "TCP unhandled" while the keeper still
      applies its filesystem profile (`NetworkMode::AllowAll` or
      equivalent), and that nothing else in the capability set depends on
      the network mode being restricted. Verified by reading the nono 0.77
      source and stating the call in design.md.

## 1. Config

- [x] 1.1 `network.loopback`, taking `"declared"` (default) or `"any"`,
      parsed into `config::Network` and added to the key allowlist.
      Verified by unit tests for both values, the default, and an unknown
      value failing at layer `config`.
- [x] 1.2 `"any"` with `network.default = "allow"` fails at layer `config`,
      naming both keys and the shared-loopback reason (spec: "Allow-default
      network"). Verified by a unit test asserting the message.

## 2. Policy

- [x] 2.1 Compile `"any"` into the capability set per L1, with origin
      `manifest:network.loopback`. A manifest without the key compiles
      byte-identically. Verified by a golden test of an unchanged manifest's
      `profile.json` and a unit test of the new rule.
- [x] 2.2 `policy --render` shows the one rule from L4, with its origin and
      namespace scope (spec: "Rendering a sandbox that opted in"). Verified
      by a render test.
- [x] 2.3 ~~`why` answers a loopback-port question from the same rule.~~
      **Struck: `why` takes paths and hosts, never ports** (`why_path`,
      `why_host`), so there is no port question to answer. Adding one would
      be a new command surface, outside this change. design.md L4 is
      corrected.

## 3. Up

- [x] 3.1 macOS refuses `"any"` at layer `backend` before any state is
      written (spec: "macOS"). Verified by a `cfg(target_os = "macos")` test.
      Mark it unmeasured if no macOS run is available, rather than claiming
      it. **Written, unmeasured:** `loopback_any_is_refused_at_backend_on_macos`
      is compiled only on macOS and has not run (Linux devcontainer).
- [x] 3.2 Where `isolate_network` would be false (failed probe, or the
      proxy-port collision), `"any"` refuses at layer `backend` instead of
      degrading, naming the key and the reason (spec: "A host that cannot
      give the sandbox its own namespace"). Verified by a test that forces
      the probe to fail.
- [x] 3.3 Before the keeper restricts itself, verify that the namespace has
      no default route and refuse `"any"` if it does (design.md L3).
      Verified by a unit test of the route check on a crafted
      `/proc/net/route`.

## 4. Tests of the boundary

- [x] 4.1 End to end: a deny sandbox with `"any"` runs a program that binds
      `127.0.0.1:0` and connects to it. Both succeed. Verified by
      `tests/loopback_any_port.rs`.
- [x] 4.2 The same program without the key is refused (spec: "The same
      program without the key"). Same file.
- [x] 4.3 With `"any"`: a direct connect to a non-loopback address fails; a
      host loopback listener is not reached; a non-allowlisted domain
      through the proxy is refused (the three "never reaches beyond the
      sandbox" scenarios). Same file.
- [x] 4.4 Teeth-check: compile `"any"` while forcing the shared loopback (no
      namespace) in a test build, and confirm 4.3's host-loopback assertion
      fails. A boundary test that cannot fail measures nothing. Verified by
      the observed failure, recorded here.
      **Observed:** it failed earlier and harder than predicted, at the
      outside-address assertion. `connect(1.1.1.1:80)` reached the internet
      directly, bypassing the proxy, because with `"any"` and no namespace
      nothing filters TCP at all. Restored, it passes. The refusal in 3.2 is
      what stands between `"any"` and an open egress path.

## 5. Antigravity replaces Gemini CLI

- [x] 5.1 `samples/devenv-antigravity-sample` sets `network.loopback =
      "any"`, and `agy` runs inside it with a dummy key: the request reaches
      `generativelanguage.googleapis.com` and is refused there for the key.
      Record what the proxy refused along the way. Verified by the proxy log
      and the README's "What it reaches" section.
- [x] 5.2 Remove `samples/devenv-gemini-sample`, because Gemini CLI no
      longer exists. Update every reference: `docs/agents.md` (the row, and
      notes at lines ~40, ~48, ~64), `samples/devenv-claude-sample/README.md`,
      `tests/tooling_layer.rs`, `src/proxy/server.rs`'s comment, and
      `CLAUDE.md`'s sample list. `openspec/changes/add-egress-proxy` stays a
      historical record and is not rewritten. Verified by `grep -ri gemini`
      returning only intentional mentions.
      Remaining mentions are intentional: the historical measurement in
      `src/proxy/server.rs` and `docs/agents.md` (annotated "since replaced
      by Antigravity"), and agy's own variable names (`GEMINI_API_KEY`,
      `GOOGLE_GEMINI_BASE_URL`, `.gemini/`). The old sample's gitignored
      `.devenv/` and `.devcroft/` and its state (`devenvgemini`) are left on
      disk for the maintainer to remove (`devcroft rm devenvgemini`).
- [x] 5.3 `docs/agents.md`: the Antigravity row (`antigravity-cli`, unfree,
      `GEMINI_API_KEY` *plus* `modelProvider = "gemini"`, `network.loopback
      = "any"`), the measured fact that `agy` honours
      `GOOGLE_GEMINI_BASE_URL` (plain HTTP, key in `X-Goog-Api-Key`), and
      the macOS refusal. Verified by review against the measurements.
- [x] 5.4 Upstream note: add one line in `docs/agents.md` that a flag for
      the language server's ports would make the key unnecessary for `agy`.
      Filing the request upstream is the maintainer's call, not this
      change's.

## 6. Docs

- [x] 6.1 `docs/known-gaps.md`: `network.loopback = "any"` is Linux-only,
      and why macOS refuses it. Verified by review.
- [x] 6.2 README or the config reference, wherever `network.ports` is
      documented: the new key, its default, and its refusals. Verified by
      review.

## 7. Verification

- [x] 7.1 `cargo build`, `cargo clippy --all-targets`, `cargo fmt`,
      `cargo doc --no-deps` all clean, with zero rustdoc warnings.
- [x] 7.2 `openspec validate --all` passes with this change included.
- [x] 7.3 Full `cargo test` with skips listed (`-- --nocapture | grep
      skipping`). None of this change's tests may be among them on this
      devcontainer.
      **2026-11-14: 769 passed, 0 failed.** The 57 skips are fleet's cgroup
      delegation (`sudo cgroup-delegate enter $$` not run), Swift (macOS
      only), flox's FloxHub login and one `rsync`. None is this change's.

## Workflow follow-up

- Archive with `/opsx:archive` once 0–7 are done.
- If fleet adopts the key, that is `add-linux-agent-fleet`'s decision
  (design.md, Non-Goals).
