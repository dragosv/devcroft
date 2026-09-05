# Feature request for nono: gate the optional integrations behind Cargo features

**Status:** drafted, not yet filed. Written to be sent upstream to the
[nono](https://github.com/nolabs-ai/nono) project. Filing it is an external
action on a third-party repository and is deliberately left to the project
owner — the same posture `use-nono-library` task 6.4 already takes for the
trust-module half of this request, which this document supersedes by combining
both asks into one.

## The request

Put `nono-proxy`'s AWS and SPIFFE integrations, and `nono`'s trust/verification
module, behind non-default Cargo features, so a downstream that uses neither
does not compile or ship them.

## Why it matters downstream

devcroft links both crates as libraries. It deliberately does not use TLS
interception, SPIFFE, or AWS routing — TLS interception is an explicit non-goal
of its threat model, not a feature it has yet to reach.

Measured against `nono-proxy` 0.75.0 with `default-features = false`, resolved
for `x86_64-unknown-linux-gnu`, the adoption costs **75 crates**. Their
composition:

| tail | crates | used by devcroft |
| --- | --- | --- |
| AWS (`aws-config`, `aws-sigv4`, `aws-sdk-{sso,ssooidc,sts}`, 15 × `aws-smithy-*`) | 20 | no |
| SPIFFE / gRPC (`spiffe`, `tonic`, `tonic-prost`, `prost`, `prost-derive`, `prost-types`) | 6 | no |
| TLS / certificate (`rcgen`, `rustls`, `tokio-rustls`, `hyper-rustls`, `x509-parser`, …) | 7 | no — TLS interception is a non-goal |
| everything else | 42 | yes |

**33 of 75 — 44% — serve capabilities the downstream has decided against.**
`system-keyring` is currently the crate's only feature, so none of them can be
compiled out.

The same shape applies to `nono` itself: its trust/verification module carries a
141-crate tail (`sigstore-*`, TUF, Rekor) that a library consumer applying a
`CapabilitySet` never invokes.

## What this is not

Not a complaint about the crates' scope, and not a request to remove anything.
Both integrations are clearly wanted by the CLI and by other consumers. The ask
is only that a library consumer be able to opt out, which is the ordinary Rust
convention for optional integrations.

Nor is it about binary size alone. The cost that matters to a downstream is
**audit surface**: devcroft's `THIRD-PARTY-LICENSES.md` exists for Apache-2.0
§4(a) compliance and is regenerated on every lockfile change, so every
unreachable dependency is a real, recurring obligation. Shipping an X.509
certificate generator inside a tool whose documentation says it never
intercepts TLS is also a claim/implementation mismatch a reader can reasonably
object to, even though the code is unreachable.

## Suggested shape

```toml
[features]
default = ["system-keyring", "aws", "spiffe"]   # CLI behaviour unchanged
aws     = ["dep:aws-config", "dep:aws-sigv4", "dep:aws-credential-types"]
spiffe  = ["dep:spiffe-workload"]
```

Defaults on keeps every current consumer working with no change; a downstream
then opts out with `default-features = false` plus the features it wants.

For `nono`, the equivalent: `trust = ["dep:sigstore-*", …]`, default on.

If TLS interception can be gated the same way, that would remove the remaining
7 crates and the mismatch above — but it is plausibly load-bearing for the
proxy's own architecture in a way the other two are not, so it is raised as a
question rather than as part of the ask.

## What the downstream will do either way

devcroft is adopting `nono-proxy` regardless of the outcome
(`openspec/changes/adopt-nono-proxy`), and asserts in its own test suite that
all three capabilities are off in the `ProxyConfig` it constructs. This request
would let that assertion be enforced by the compiler instead.
