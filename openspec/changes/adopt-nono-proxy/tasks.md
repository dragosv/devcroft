# Tasks — Adopt `nono-proxy`

## 0. The dependency move

- [x] 0.1 Bump `nono` 0.74.0 → 0.75.0 and confirm devcroft still compiles.
      → **Clean**: `cargo check --all-targets`, zero warnings, no source change,
      31s. macOS 15.7.4/arm64.
- [ ] 0.2 Run the full test suite on 0.75.0 before adding the proxy crate, so a
      regression from the bump is distinguishable from one from the adoption.
      A clean `cargo check` is not a clean test run.
- [ ] 0.3 Add `nono-proxy = { version = "0.75.0", default-features = false }`.
      `system-keyring` is redundant — devcroft does not use the keystore path
      and D4 does not adopt one.
- [ ] 0.4 Regenerate `THIRD-PARTY-LICENSES.md` **on Linux**. On macOS the
      generator resolves for the host target and silently drops the Linux-only
      tail from a §4(a) compliance artifact.
- [ ] 0.5 Record the measured crate delta in the change and in
      `docs/decisions.md`, replacing E6's stale 116.

## 1. Swap the loop, keep the process

- [ ] 1.1 Replace `proxy::server`'s accept loop with
      `nono_proxy::start(ProxyConfig)`, leaving `proxy::spawn`, the pidfile,
      `up`/`down`/`rm` ownership, `network_proxy_port` and the `ProxyOnly` gate
      untouched (D1).
- [ ] 1.2 Resolve Open Question 1 before wiring auth: devcroft's per-session
      token and the crate's `require_auth` do the same job. Decide which
      survives, and make the refusal code deliberate — `407` today, and a
      user-visible change if it moves.
- [ ] 1.3 Verify every property `add-egress-proxy` shipped still holds:
      fail-closed with no listener, per-session auth, allowlist decisions,
      `policy --render` output. These are the regression surface.
- [ ] 1.4 Confirm the egress e2e suite passes unchanged, or that each
      difference is intended and recorded.

## 2. Keep the refused capabilities unreachable

- [ ] 2.1 Assert, in a test, that the `ProxyConfig` devcroft constructs has TLS
      interception, SPIFFE and AWS routing off — for an arbitrary manifest, not
      just a minimal one (D3).
- [ ] 2.2 Teeth-check it: flip one on in the constructor and confirm the test
      fails. A guard that cannot fail is not a guard.
- [ ] 2.3 Draft the upstream request to gate AWS and SPIFFE behind features,
      alongside `use-nono-library` 6.4's trust-module ask. **Left for the owner
      to send** — an agent does not open issues on third-party repositories.

## 3. Brokered credentials

- [ ] 3.1 Manifest surface: a route names an upstream and an indirection to a
      secret, never the secret (D4). Compile it into the policy with an origin,
      so `policy --render` shows the route and never the value.
- [ ] 3.2 Resolve the secret host-side at `up`, in the trusted phase, and fail
      there when it is absent — naming the route and leaving no sandbox running.
- [ ] 3.3 Point the client at the route: set `ANTHROPIC_BASE_URL` (and the
      equivalent for any other declared upstream) in the resolved environment.
- [ ] 3.4 Make the bypass failure legible (D5): a client that ignores the
      variable and dials the real upstream must be told the upstream is
      brokered, not merely that egress was denied.
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
