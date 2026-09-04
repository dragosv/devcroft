# Design — Decouple the Service Supervisor

## Context

`src/services` is mostly devcroft's own machinery with four
process-compose-shaped holes in it. The module already separates cleanly:
paths, reconciliation and the service-state vocabulary know nothing about any
supervisor, while config rendering, binary lookup, status querying and the
spawn invocation are entirely specific to one.

This change moves those four behind a trait. It is a refactor with no
behavioural change, and its value is what it makes possible rather than what
it does.

## Goals / Non-Goals

**Goals:**
- One place that knows what a supervisor is, instead of four.
- Behaviour identical: same config, same protocol, same refusal.
- The supervisor becomes nameable, so output can say which one is in use.

**Non-Goals:**
- Not a second supervisor, not a manifest key, not consuming the provider's
  supervisor — see proposal Non-Goals.

## Decisions

## D1 — The trait is exactly the four coupling points

**Decision.**

```rust
pub trait Supervisor {
    /// The executable that must be in the resolved environment.
    fn binary(&self) -> &'static str;
    /// The config devcroft writes for it, from its own declarations.
    fn render_config(&self, services: &[ServiceDecl], shell: &Path) -> String;
    /// How to launch it: argv after the binary.
    fn spawn_args(&self, config: &Path, socket: &Path) -> Vec<String>;
    /// Ask it what is running.
    fn query(&self, socket: &Path) -> Result<Vec<ServiceState>, Unreachable>;
}
```

**Rationale.** Each method is one of the four measured couplings and nothing
else. `ServiceState`/`ServiceHealth` stay shared: they are devcroft's
vocabulary for reporting, and a supervisor's job is to answer *in* it, not to
define it. `reconcile` stays outside the trait for the same reason — it
compares what was declared against what a supervisor reported, which is
devcroft's guarantee regardless of who reported.

**Alternative considered and rejected: put `socket_path`/`config_path` on the
trait.** They are devcroft's layout decisions — the artifact directory exists
because that is the only place the sandbox can both write and read — and a
supervisor has no business choosing them. Moving them would widen the trait
without moving any actual coupling.

## D2 — Selection is a function today, not configuration

**Decision.** `services::supervisor()` returns the one devcroft ships. No
manifest key, no environment variable, no dispatch.

**Rationale.** There is one implementation; a selection mechanism with one
option is configuration theatre, and it is easier to add later than to remove
once someone depends on it. The seam's whole point is that adding a second is
an `impl`, and the selection question can be answered then, with a real second
option to reason about.

## D3 — The keeper is told which supervisor, not which binary

**Decision.** `start_services_if_requested` currently hardcodes
`cmd: "process-compose"` and its arguments. It asks the seam instead.

**Why this one needs care.** That code runs in the *keeper*, across an exec
boundary from `up`, and reads its instructions from environment variables
(`DEVCROFT_START_SERVICES`, `DEVCROFT_SERVICES_ROOT`, `DEVCROFT_SANDBOX_NAME`).
With one supervisor it can simply call the seam and get the same answer `up`
would have. With two it would need to be *told* which — an env var carrying
the supervisor's name, resolved back to an implementation on the other side.

Not built now, deliberately: it is exactly the "configuration with one
option" D2 rejects. But it is the piece a second supervisor must add, and
naming it here is cheaper than rediscovering it.

## D4 — The refusal message names the supervisor

Today: *"`process-compose` is not in the resolved environment; add it to the
environment manifest"*. The literal becomes `supervisor.binary()`.

Small, and the reason it is in this change rather than deferred: that message
is the user-visible face of the coupling this change exists to name. Leaving
it hardcoded would keep the one place the user actually meets the assumption
tied to a specific tool.

## Risks / Trade-offs

- **[Risk] A refactor that quietly changes behaviour.** The config is a JSON
  document a third-party binary parses; a subtly different rendering could
  fail at runtime rather than at compile time. → **Mitigation**: the existing
  `render_config` unit tests assert the document's shape, and the services
  e2e suite runs it for real. Both must pass unchanged, and the rendered
  output should be byte-identical — worth asserting explicitly rather than
  inferring from green tests.
- **[Trade-off] A trait with one implementation is a smell** until the second
  arrives, and this change deliberately does not bring it. The justification
  is not "we might need it" but that the *user-visible requirement* it
  enables removing is real and present today. If the second supervisor never
  ships, this change leaves the codebase marginally more indirect for no
  gain, and that is the honest downside.

## Open Questions

1. **Does a devcroft-owned supervisor offer less, or reimplement?** Restart
   policy, service dependencies and daemon handling are why process-compose
   was chosen. A minimal supervisor drops them; a complete one is a project.
   Which is acceptable determines whether "no third-party binary" is a real
   option or a worse one. Not answered here, and it is the question that
   decides whether this seam ever pays off.
2. **If there are two, how does a project get the other one?** D2 defers
   selection. The answer interacts with the manifest's own principles — a
   supervisor is closer to a devcroft setting than to an environment
   declaration, which argues against `devcroft.toml` growing a key for it.
