# Proposal — Loopback ports chosen at runtime

## Why

A program that listens on a port the kernel picks (`127.0.0.1:0`) and then
connects to it cannot run in a `network.default = "deny"` sandbox today.
`network.ports` grants only explicit ports, because Landlock's `NetPort`
rules are per port.

This is no longer hypothetical. It was measured on 2026-11-14 with the
Antigravity CLI (`agy`, nixpkgs `antigravity-cli`), Google's coding agent and
Gemini CLI's successor:

- `agy` starts an embedded language server on two kernel-chosen loopback
  ports, then connects to them.
- The ports come from an internal `-http_server_port=0`, with no flag or
  environment variable that changes them.
- Inside a devcroft sandbox it exits before making a single request:
  `listen tcp 127.0.0.1:0: bind: permission denied`.

The sample written to replace `devenv-gemini-sample`
(`samples/devenv-antigravity-sample`) therefore cannot run, and the agents
matrix loses its Google row.

On Linux the restriction protects nothing. Every `"deny"` sandbox already
has a network namespace of its own with no route out (`add-port-allocation`,
`add-egress-proxy`). `add-linux-agent-fleet`'s D9 re-derivation established
that this route-less namespace is the egress boundary, and that Landlock's
`NetPort` is only a port filter inside it. A port on the sandbox's *own*
loopback is reachable by nothing outside that sandbox.

## What Changes

- **New opt-in manifest key, `network.loopback`, taking `"declared"` or
  `"any"`.** `"declared"` is the default and today's behaviour: only
  `network.ports`, declared services and the proxy port. `"any"` lets the
  sandbox bind and connect on any loopback TCP port, *inside its own network
  namespace*. Opt-in, so every existing manifest compiles byte-identically.
- **Refused where the argument does not hold, never degraded into a wider
  grant.** `"any"` is refused at `up`, naming the reason, in three cases:
  - with `network.default = "allow"`, whose sandboxes share the host's
    loopback (that is `port-allocation-sample`'s whole point);
  - on a Linux host that cannot create the network namespace;
  - on macOS, where Seatbelt has no namespace and `localhost:*` would reach
    every service on the host's loopback.

  In each case "any loopback port" would mean the *host's* loopback, which
  is a different grant from the one declared.
- **Inspectable.** `policy --render` shows the grant with origin
  `manifest:network.loopback`, and states the namespace it is scoped to. Its
  effect on the compiled profile is visible, not just the key.
- **The proxy remains the only way out.** `"any"` must not let a process
  inside the sandbox reach anything beyond its own loopback. That is the
  existing invariant, and this change re-asserts it with a test rather than
  assuming it.
- **Follow-up in this change, once `agy` runs.**
  - `devenv-antigravity-sample` replaces `devenv-gemini-sample`, because
    Gemini CLI no longer exists.
  - `docs/agents.md` gets an Antigravity row in place of Gemini's.
  - The matrix gains the measured fact that `agy` honours
    `GOOGLE_GEMINI_BASE_URL`, which is what `add-agent-workload`'s brokering
    needs to know.

## Capabilities

### New Capabilities

None. The behaviour belongs to the existing `network` capability.

### Modified Capabilities

- `network`: adds a requirement that a deny-default sandbox may opt in to
  runtime-chosen loopback ports within its own namespace, and that the
  opt-in is refused wherever the loopback is shared. The base capability is
  specified in `add-egress-proxy`. As with every change since, it is written
  as `ADDED` requirements, because `openspec/specs/` is still unsynced.

## Impact

- `src/config/mod.rs`, `src/config/validate.rs`: the new key, and refusal of
  `"any"` together with `default = "allow"` at layer `config`.
- `src/policy/` (`mod.rs`, `capability_set.rs`, `render.rs`, `degraded.rs`):
  compiling the grant, rendering it, and refusing per platform.
- `src/lifecycle/up.rs`: refusal when the namespace is unavailable, at layer
  `backend`.
- `tests/`: an end-to-end bind-and-connect on a kernel-chosen port; the
  refusals; a check that the proxy is still the only route out; and a check
  that the bind test fails without the key.
- `samples/devenv-antigravity-sample` (new), `samples/devenv-gemini-sample`
  (removed), `docs/agents.md`, `docs/known-gaps.md` (the macOS refusal),
  `CLAUDE.md`'s sample list.
- No new dependency.
