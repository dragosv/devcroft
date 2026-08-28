# Add Egress Proxy

**Blocks:** `sandbox-provisioning`, `add-linux-agent-fleet`.

## Why

Domain-level network filtering is declared in the manifest and does not work. It
was already non-functional under the CLI-based invocation — a resident
supervisor is required and `wrap` has none — and under the library it compiles
to a plain network block. What ships today is binary: all network, or none.

That is tolerable for a single developer opening their own project. It blocks
both directions this project is moving in:

- **Provisioning.** Confining provider activation is only worth doing if the
  confined code cannot exfiltrate. Activation genuinely needs the network —
  `npm ci`, `poetry install`, `go mod download` — so the choice today is
  "activation fails" or "unreviewed code has open egress". Neither is the
  feature.
- **Fleet.** Per-agent egress policy is the point of running agents on
  unreviewed code. Without a proxy there is nothing to attribute a request to an
  agent, and nothing to filter.

Both are blocked on the same missing component, which is why it is one change
rather than a section inside each.

## What Changes

- **NEW** a resident egress proxy owned by the supervisor, enforcing a
  domain-level allowlist.
- **MODIFIED** `network`: `network.allow` becomes enforced rather than declared.
  The compiled policy routes egress through the proxy instead of compiling to a
  blanket block.
- ~~Manifest gains a per-context network policy, so provisioning and runtime can
  differ.~~ **Moved to `sandbox-provisioning`** (which now owns the
  requirement) when this change shipped: a per-context policy needs two
  contexts, and provisioning does not run inside a boundary of its own until
  that change creates one. This change ships the runtime context's enforcement,
  which is the mechanism the second context will reuse.
- Diagnostics report refusals with the destination and the deciding rule.

## Impact

- Affected specs: `network`
- Unblocks `sandbox-provisioning` (a provisioning profile that permits the
  package registries and nothing else) and `add-linux-agent-fleet` (per-agent
  policy and attribution).
- The proxy runs on the host, outside whatever sandbox the client is in. That
  placement is what gives attribution and keeps credentials out of the client's
  reach.

## Non-Goals

- Preventing exfiltration. A domain allowlist constrains destinations; any
  allowlisted endpoint that accepts a POST remains an outbound channel. The
  claim this change supports is "egress is constrained to allowlisted
  destinations" and no more.
- Content inspection or TLS interception.

## What the library already provides

Confirmed at the pinned version, and it changes the size of this change
considerably:

- `net_filter::HostFilter` — domain matching, described as being for
  proxy-level use. The policy engine exists.
- `install_seccomp_proxy_filter` / `prepare_seccomp_proxy_filter` — a
  seccomp-notify filter for proxy-only network mode, prepared in the parent and
  installed in the child. Enforcement is kernel-mediated.
- The notification API — receive, read the destination sockaddr, deny, continue,
  respond with an errno, inject a descriptor, and validate a notification is
  still pending for TOCTOU safety.

**That "Confirm before designing" instruction was followed, and it changed
this section.** The list above describes a seccomp-notify design that turned
out not to apply: `install_seccomp_proxy_filter` accepts no policy at all
(`has_bind_ports: bool`) and is only ever installed as `apply_auto`'s fallback
for Landlock ABI < V4 — measured live at **ABI V6** here, where it is never
installed. The notification API is real but is the library's own internal
business on those kernels; devcroft never touches it. What the library actually
provides at devcroft's level is `CapabilitySet::proxy_only_with_bind`, which
compiles to a plain Landlock `NetPort`/Seatbelt rule, plus `HostFilter` for the
hostname decision. See `design.md`'s Open Questions for the full trail.

What devcroft supplies is therefore smaller and more ordinary than this section
first assumed: a resident HTTP proxy process that terminates `CONNECT` and
absolute-URI requests and decides by hostname. The per-context half of the
policy moved to `sandbox-provisioning` (see What Changes). The point that
survives unchanged is why the CLI's wrap mode structurally could not do this:
whichever layer enforces the kernel gate, something resident still has to
terminate the connection and decide by name, and `wrap` has no such process.
