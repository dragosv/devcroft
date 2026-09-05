## Why

**`add-egress-proxy` E6 decided to adopt `nono-proxy` and deferred it on one
number. The project owner has now accepted the trade — and the number was
wrong, in devcroft's favour.**

E6 recorded "116 additional crates, measured", the same order as the 141-crate
trust tail `use-nono-library` accepted reluctantly. Re-measured against
`nono-proxy` **0.75.0** with `default-features = false`
(`system-keyring` is redundant — devcroft does not use the keystore path, and
`nono` already carries its own): **75 crates** for a bare consumer, and
**66 for devcroft**, which already shares nine of them — mostly the rustls tail
it pulls through sigstore. Measured on this manifest, `cargo tree -e normal`
against the Linux target, with and without the dependency: 303 → 369.

The composition matters more than the total:

| tail | crates | serves |
|---|---|---|
| AWS (`aws-sigv4`, `aws-sdk-{sso,ssooidc,sts}`, 15 × `aws-smithy-*`) | 20 | AWS routing — **off by decision** |
| SPIFFE / gRPC (`spiffe`, `tonic`, `tonic-prost`, `prost*`) | 6 | SPIFFE — **off by decision** |
| TLS / certificate (`rcgen`, `rustls`, `tokio-rustls`, `hyper-rustls`, `x509`…) | 7 | TLS interception — **an explicit non-goal** |
| everything else | 42 | what devcroft actually wants |

**33 of 75 — 31 of devcroft's own 66, 47% — exist solely to serve the three
capabilities devcroft refuses.** E6 said they are "present in the dependency, unused by devcroft, and
not to be enabled silently". True, and incomplete: `system-keyring` is the
crate's *only* feature, so none of them can be compiled out. devcroft would
ship an X.509 certificate generator for a feature its own threat model calls an
explicit non-goal.

That is not a reason to refuse — it is a reason to ask, and the ask has a
precedent already open: `use-nono-library` task 6.4 asks the same maintainer to
gate `nono`'s trust module behind a feature. This change adds the second half
of that request and proceeds either way.

**And the real prize is not egress filtering, which devcroft already has.**
`nono_proxy::credential`: *"Loads API credentials from the system keystore…
injected into requests via headers, URL paths, query parameters, or Basic Auth.
**The sandboxed agent never sees the real credentials.**"* That is precisely the
position `docs/decisions.md` staked out — secrets "never via mounted files or
plain env vars" — and which `add-agent-workload` task 7.1 was preparing to
retract as unachievable. It is achievable; it lives in this crate.

Verified as reverse-proxy, **not** interception: the agent calls
`http://localhost:PORT/anthropic/v1/…` and the proxy rewrites upstream with the
real key attached. The TLS non-goal survives adoption intact — checked, because
"credential injection" is exactly the phrase that would otherwise imply MITM.

## What Changes

- **NEW** `brokered-credentials`: a sandbox reaches an upstream API through a
  route the proxy owns, and the credential never enters the sandbox — not its
  environment, not its filesystem, not its process table.
- `crate::proxy::server`'s hand-written CONNECT loop is replaced by
  `nono_proxy::start(ProxyConfig)`. **devcroft keeps the process**: `proxy::spawn`,
  the pidfile, `up`/`down`/`rm` ownership, `CompiledPolicy::network_proxy_port`
  and the `NetworkMode::ProxyOnly` kernel gate are unchanged, which is what
  preserves E1's placement argument and the fail-closed property.
- **BREAKING (internal)**: `nono` moves 0.74.0 → 0.75.0. `nono-proxy` 0.75.0
  requires it, so E6's "pinned to the same 0.74.0" premise no longer holds.
  This is the change's first task and its first risk.
- `add-agent-workload`'s credential group is rewritten against a mechanism that
  exists, and task 7.1's planned retraction is withdrawn rather than filed.
- **Not in this change**: TLS interception, SPIFFE, AWS routing. Present in the
  dependency graph, unreachable in devcroft's configuration, and enabling any
  of them is a change to what devcroft claims.

## Capabilities

### New Capabilities

- `brokered-credentials`: what a brokered credential guarantees, what it does
  not, where the route is declared, and what must be true of a sandbox that has
  one.

### Modified Capabilities

- (none — `openspec/specs/` holds no synced specs. The `egress-proxy` capability
  this re-implements lives in the unarchived `add-egress-proxy`; its
  requirements are preserved exactly and only their implementation moves.)

## Impact

- **Affected code**: `src/proxy/` (the loop, not the lifecycle), `Cargo.toml`,
  `THIRD-PARTY-LICENSES.md` (regenerate **on Linux** — the generator resolves
  for the host target and silently drops the Linux-only tail on macOS).
- **The audit obligation grows with the graph.** 75 new crates in a project
  whose attribution file is a §4(a) compliance artifact, not a formality.
- **Two capabilities arrive that devcroft has written as open and not built**:
  `start_with_approval` answers `add-agent-interaction`'s approval half at the
  L7 endpoint level, and the append-only audit with chain and Merkle commit
  answers its "durable record of every request and decision". Both are
  consequences of this change, not goals of it, and neither is claimed until
  exercised.
- **Risk concentration.** After this, devcroft's boundary, its egress filter and
  its credential handling all come from one upstream maintained by one author.
  `use-nono-library` accepted that for the boundary. This extends it, and the
  extension should be stated plainly rather than discovered later.

## Non-Goals

- **Not TLS interception, SPIFFE, or AWS routing.** Compiled in, unreachable,
  and each would be a change to devcroft's claims rather than a configuration.
- **Not replacing the proxy's placement or lifecycle.** E1/E2 are unchanged;
  what moves is the loop inside a process devcroft still owns.
- **Not a general keystore integration.** Where a brokered credential comes
  from on the host is this change's narrowest possible answer, deliberately —
  the broad version is `nono`'s keystore, whose adoption is its own decision.
