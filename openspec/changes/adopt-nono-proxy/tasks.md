# Tasks — Adopt `nono-proxy`

## 0. The dependency move

- [x] 0.1 Bump `nono` 0.74.0 → 0.75.0 and confirm devcroft still compiles.
      → **Clean**: `cargo check --all-targets`, zero warnings, no source change,
      31s. macOS 15.7.4/arm64.
- [x] 0.2 Run the full test suite on 0.75.0 before adding the proxy crate, so a
      regression from the bump is distinguishable from one from the adoption.
      A clean `cargo check` is not a clean test run.
      → **416 passed, 0 failed, 49 test binaries.** 19 skips, every one with a
      named reason and every one pre-existing: 14 Linux-namespace (net, mount),
      2 macOS pty, 1 loopback alias, 1 rsync-under-Seatbelt, 1 netns
      enforcement.
      **The check that makes this meaningful**: not one skip came from
      `backend_supported()`. A nono release that broke capability detection
      would empty this suite silently — every e2e test guards on that probe and
      a skip reads as a pass — so its absence, not the exit code, is what says
      0.75 is safe. No 0.74 baseline needed as a result.
      macOS 15.7.4/arm64 only; Linux unverified.
- [x] 0.3 Add `nono-proxy = { version = "0.75.0", default-features = false }`.
      `system-keyring` is redundant — devcroft does not use the keystore path
      and D4 does not adopt one.
      → Compiles clean, all targets. **The cost for devcroft is 66 crates, not
      75**: measured `cargo tree -e normal` on this manifest, Linux target,
      with and without the dependency (303 → 369). The 75 figure is a bare
      consumer's; devcroft already shares 9 of them, mostly the rustls tail it
      pulls through sigstore. **31 of the 66 — 47% — still serve the three
      refused capabilities** (18 AWS, 6 SPIFFE/gRPC, 7 TLS/certificate), which
      is what task 2.3's upstream request asks to make optional.
- [ ] 0.4 Regenerate `THIRD-PARTY-LICENSES.md` **on Linux**. On macOS the
      generator resolves for the host target and silently drops the Linux-only
      tail from a §4(a) compliance artifact.
- [ ] 0.5 Record the measured crate delta in the change and in
      `docs/decisions.md`, replacing E6's stale 116.

## 1. Swap the loop, keep the process

- [x] 1.2 Resolve Open Question 1 before wiring auth.
      → **D6.** devcroft keeps minting the token; the crate checks it
      (`require_auth: true`, `session_token: Some(..)`); both answer `407`.
      **And `strict_connect_auth` is forced `true`, against the crate's
      default** — its default rests on undici not echoing URL userinfo as
      `Proxy-Authorization` on CONNECT, which does not reproduce: measured
      `yes` for both curl 8.7.1 and undici 8.10.2 on Node 22.22.3. Taking the
      default would have silently reopened the open-relay hole task group 4a
      closed.
- [x] 1.1 **Reshaped by D7 — this is not a drop-in.** `nono_proxy::start` binds
      its own TCP listener and accepts neither a pre-bound fd nor a unix
      socket, and devcroft's netns path reaches the proxy over
      `StatePaths::proxy_socket` because a loopback-only namespace cannot see
      the host's `127.0.0.1`. Keep devcroft's unix acceptor in the same
      process, splicing to a loopback `nono-proxy` it starts itself; leave
      `proxy::spawn`, the pidfile, `up`/`down`/`rm`, `network_proxy_port` and
      the `ProxyOnly` gate untouched.
      → Done: `src/proxy/backend.rs` (config + start + audit drain),
      `server::bridge_tcp_to_tcp` as the TCP twin of the unix bridge that
      already existed, wired in `egress_proxy_main`. Compiles clean, clippy and
      fmt clean, and the full suite is **416 passed / 0 failed — identical to
      the pre-swap baseline**.
      **That identical count is not evidence the swap works.** It is evidence
      nothing else broke: `tests/egress_proxy_e2e.rs`, the one test that
      exercises this path, skips on macOS for want of `127.0.0.3`/`127.0.0.4`
      loopback aliases. See 1.4.
      One thing this nearly missed: the proxy log is a devcroft *interface* —
      `logs` surfaces it and the e2e asserts its shape — so nono-proxy's audit
      events are rendered into it (`backend::drain_into_log`). Swapping without
      that would have kept every kernel property and silently emptied the one
      file a user reads to see what the proxy decided.
      `server::run` and its 13 unit tests are **kept, not deleted**: it is
      `pub` in a `pub` module so it raises no dead-code warning, and removing a
      working, tested, security-sensitive loop in the same commit that
      introduces an unverified replacement would leave no fallback.
- [ ] 1.1b Measure the extra hop's cost on the namespaced path before accepting
      it. That path is the fleet path, so "negligible" is a claim, not a given.
- [ ] 1.1c Consider asking upstream for `start_on_listener` (a pre-bound
      `TcpListener`/`UnixListener`), which would remove both the hop and the
      relay. Deliberately **not** folded into
      `docs/nono-feature-gating-issue.md` — that ask is nearly free to grant,
      and bundling an API change would weaken it.
- [ ] 1.3 Verify every property `add-egress-proxy` shipped still holds:
      fail-closed with no listener, per-session auth, allowlist decisions,
      `policy --render` output. These are the regression surface.
- [ ] 1.4 Confirm the egress e2e suite passes unchanged, or that each
      difference is intended and recorded.
      **Blocked on this host, and it is the gating verification for the whole
      of group 1.** `tests/egress_proxy_e2e.rs` needs `127.0.0.3` and
      `127.0.0.4`; macOS assigns only `127.0.0.1`. Unblocked by
      `sudo ifconfig lo0 alias 127.0.0.3` and `... 127.0.0.4`, or by running on
      Linux. Not run here: adding a host network alias needs root and is not an
      agent's call to make unprompted.
      **`server::run` is not deleted until this passes.**

## 2. Keep the refused capabilities unreachable

- [x] 2.1 Assert, in a test, that the `ProxyConfig` devcroft constructs has TLS
      interception, SPIFFE and AWS routing off — for an arbitrary manifest, not
      just a minimal one (D3).
      → `tests/proxy_refused_capabilities.rs`, 4 tests. Built on a realistic
      allowlist rather than an empty one, because a guard that only holds for
      the empty case is not a guard. Covers the three CA fields, empty
      credential routes, `require_auth` + `session_token`, `strict_filter`, and
      `strict_connect_auth`.
- [x] 2.2 Teeth-check it: flip one on in the constructor and confirm the test
      fails. A guard that cannot fail is not a guard.
      → Both directions checked. `strict_connect_auth: false` (the crate's own
      default) fails `authentication_is_required_including_on_connect`;
      `intercept_ca_dir: Some(..)` fails `tls_interception_is_off`. Each flip
      failed **only** its own test — a flip that reddened everything would mean
      a broken harness rather than a working guard.
- [x] 2.3 Draft the upstream request to gate AWS and SPIFFE behind features,
      alongside `use-nono-library` 6.4's trust-module ask. **Left for the owner
      to send** — an agent does not open issues on third-party repositories.
      → `docs/nono-feature-gating-issue.md`. Combines both asks into one, since
      they go to the same maintainer and share a rationale, and supersedes
      6.4's separate draft. Raises TLS-interception gating as a *question*
      rather than part of the ask: unlike AWS and SPIFFE it is plausibly
      load-bearing for the proxy's own architecture, and asking for something
      that cannot be given weakens the two asks that can.

## 3. Brokered credentials

- [~] 3.1 Manifest surface: a route names an upstream and an indirection to a
      secret, never the secret (D4). Compile it into the policy with an origin,
      so `policy --render` shows the route and never the value.
      → **Manifest half done**: `[[broker]]` with `provider` (the route prefix,
      naming the upstream API rather than any agent — D5), `upstream`, `secret`
      as an indirection, and an optional `env_var` override. Six unit tests.
      **The rule that carries the policy invariant**: a broker's upstream host
      must already be in `network.allow`, refused by name if not. The proxy
      dials upstream for the sandbox, so a route to an unallowed host would be
      egress `policy --render` never shows — the invariant broken in the one
      place a reader is least likely to look.
      Two things worth knowing about the tests: the wildcard case is the
      *control*, since a check that refused everything would pass the refusal
      test on its own; and `[[broker]]` is the schema's only array of tables,
      so `check_unknown_keys` saw `None` from `as_table()` and left every field
      unchecked — the one shape where a typo was silently accepted. Now
      reported as `broker[0].upstrem` with a suggestion.
      **Still open**: compiling the route into `CompiledPolicy` with an origin
      so `policy --render` shows it.
- [ ] 3.2 Resolve the secret host-side at `up`, in the trusted phase, and fail
      there when it is absent — naming the route and leaving no sandbox running.
- [ ] 3.3 Point the client at the route from the **provider prefix**, not from
      any agent's name (D5): `ProxyHandle::credential_env_vars` derives
      `{PREFIX}_BASE_URL`, and the manifest can override it where an SDK does
      not follow that convention. devcroft hardcodes no agent's variable.
- [ ] 3.3b Carry the phantom token too, and understand why before doing it:
      many SDKs refuse to start without an API key present, so `{KEY}_API_KEY`
      is set to the *session token* and the proxy swaps it upstream. Without
      this the route resolves correctly and the SDK still fails a missing-key
      check.
- [ ] 3.4 Make the bypass failure legible (D5): a client that dials the real
      upstream must be told the upstream is **brokered and the route was not
      used**, not merely that egress was denied.
- [ ] 3.4b Establish which class a project's client falls in, and refuse to
      degrade. A client honouring only `HTTPS_PROXY` speaks end-to-end TLS to
      the real host and **cannot** be brokered without interception, which is a
      non-goal. devcroft must not answer that by letting the client carry its
      own credential — that is exactly what the route was declared to prevent.
- [ ] 3.5 Confirm no interception: the proxy opens its own upstream connection
      and installs no CA into the sandbox.

## 4. Tests

- [ ] 4.1 **The one that matters**: from inside a real session, the credential
      is absent from the environment, from every granted path, and from the
      process table — while a request through the route reaches the upstream
      authenticated.
- [ ] 4.2 An undeclared upstream is refused, distinguishably from an upstream
      error.
- [ ] 4.3 A missing credential fails `up`, not first use, and leaves nothing
      running.
- [ ] 4.4 Skip-guard audit: 4.1 needs a live sandbox and a reachable upstream.
      Guard on the capability and say what was skipped — a green run that
      tested nothing is this project's recurring failure, and a credential test
      that silently skips is the worst instance of it.

## 5. Record what changed, including the retraction that is now withdrawn

- [ ] 5.1 `docs/decisions.md`: the secret-injection position stands rather than
      being retracted. `add-agent-workload` 7.1 was drafted to amend it as
      unachievable; the mechanism exists, so 7.1 is withdrawn and the entry
      gains the measurement instead.
- [ ] 5.2 Rewrite `add-agent-workload`'s credential group against this
      mechanism, and record what it still does not solve: subscription/OAuth
      auth has no brokered form either, which ArcBox independently confirms.
- [ ] 5.3 State the risk concentration in the README's dependency posture:
      boundary, egress filter and credentials now share one upstream and one
      author.
- [ ] 5.4 `docs/known-gaps.md`: brokering protects the secret from the sandbox,
      not from the network the sandbox is allowed to reach. An agent that can
      call the route can spend the credential; it just cannot exfiltrate it.
      That distinction is the one most likely to be overstated.
