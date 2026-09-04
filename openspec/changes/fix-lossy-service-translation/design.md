# Design — Service Translation Fidelity

## Context

devcroft reads a provider's service declarations into `ServiceDecl`, renders
a process-compose config from them, and lets process-compose supervise. Two
couplings live in that sentence and they are usually conflated:

- **the declaration vocabulary** (`ServiceDecl`) — five fields, which are
  flox's documented schema;
- **the supervisor** (process-compose) — its config format, its binary name,
  its HTTP API.

The second is the one that looks alarming and turns out to be defensible.
The first is the one that silently loses information today.

## Goals / Non-Goals

**Goals:**
- A declaration devcroft cannot carry fails loudly, at the layer that owns
  it, naming the field.
- Record the supervisor coupling precisely enough that a future provider
  with a different supervisor starts from a map rather than a blank page.

**Non-Goals:**
- Not building the supervisor abstraction (proposal Non-Goals).
- Not widening `ServiceDecl` ahead of a provider that needs it.

## Decisions

## S1 — Refuse at translation, not at render

**Decision.** The check belongs in `prepare_services` (`up.rs`), beside the
existing "process-compose is not in the resolved environment" refusal, and
fails the same way: `UpError::Provider`, naming the field.

**Rationale.** That refusal already establishes the shape — a services
problem that would otherwise surface as an opaque runtime failure is caught
host-side, before any state is written, at layer `provider`. Losing a
declaration is the same class of problem, discovered at the same moment, and
belongs next to it rather than in `render_config`, which should stay a pure
projection.

**Why not "log a warning and continue".** Because the failure mode this
prevents is precisely a sandbox that starts, reports healthy, and does not do
what the manifest said. `add-flox-services` states that as the thing
`reconcile` exists to prevent one step later; a warning here would leave the
earlier half unclosed while looking closed.

## S2 — The boundary is "the provider said something we did not carry"

**Decision.** The check is on the *parse* side: whichever provider reader
builds `ServiceDecl` knows which keys it saw and which it consumed, and
reports the residue. It is not a check inside `render_config`, which by then
has only the five fields and cannot know what was dropped.

**Consequence worth stating**: this makes the obligation a provider's, not
`services`'. A provider reader that silently ignores an unknown key is the
bug; the shared machinery cannot detect it after the fact. That is the same
division `ServiceSupport` already draws — the provider decides what it
declares, `services` decides what happens to it.

## S3 — The supervisor coupling, recorded and not abstracted

devcroft touches process-compose at exactly **four** points. Everything else
in `src/services` is its own and supervisor-agnostic — `socket_path`,
`config_path`, `log_path`, `artifact_dir`, `reconcile`, `ServiceState`,
`ServiceHealth`, which is half the module's public surface.

| point | what is process-compose-specific |
|---|---|
| `services::render_config` | the config schema (`processes`, `shell`, `availability.restart`) |
| `services::resolve_in_env` | the binary name, `"process-compose"` |
| `services::query` | `GET /processes` HTTP over a unix socket |
| `bin/devcroft.rs` (`start_services_if_requested`) | `cmd: "process-compose"` and its arguments |

**Decision: do not abstract these yet**, and the reason is a measurement
rather than taste. The prompting question was "what if a provider's services
are based on something else". They are not: `add-devenv-provider` records
that devenv's `processes` are process-compose-backed, and flox uses it
internally. process-compose is the common denominator of this ecosystem, so
a `Supervisor` trait's only second implementer today would be a test double
— the shape this project rejects elsewhere, most recently when it dropped
`up_with_resolution` as "a second entry point whose only distinction is
covering less of the real path".

**What would change the answer**: a provider whose services are genuinely
supervised by something else, or process-compose becoming unavailable,
unlicensable, or incompatible. At that point the four rows above are the
seam, and the second implementation is not a crate — see S4.

## S4 — If it is ever built, it is devcroft's own code, not a dependency

Measured while asking whether a crate could supply the simple supervisor a
test row would want:

| crate | what it actually is |
|---|---|
| `duct` 1.1.2, `subprocess` 1.2.1 | process spawning and pipelines |
| `command-group` 5.0.1 | process groups — genuinely useful for reaping |
| `procfile` 0.2.1 | a Procfile parser |
| `supervisor` 0.1.0 | a placeholder: one dependency |

**Nothing offers "supervise N processes and expose their status."** That is
informative rather than disappointing: it is an application concern, which is
why process-compose is a binary everyone shells out to rather than a crate
everyone links. A minimal supervisor would therefore be devcroft's own
~150 lines on `std` — no new dependency, and no supply-chain question of the
kind a prebuilt process-compose binary would carry.

**And it would prove less than it appears to.** `reconcile` exists to catch
"the supervisor accepted fewer services than were declared"; asserted against
a minimal supervisor that always accepts, that test is circular. Restart
policy, inter-service dependencies and daemon handling are the *reason*
process-compose was chosen (`add-flox-services` decision 1), not incidental.
A simple supervisor would let the *lifecycle* half be tested without
process-compose — does the keeper start it, does `down` reap it, does
`status` report it — and nothing about service semantics. Any change that
builds it must say so, or a green board will be read as more than it is.

## Risks / Trade-offs

- **[Risk] Refusing breaks a project that works today**, if some provider
  already emits a key devcroft ignores and the services run fine without it.
  → **Mitigation**: measure before enforcing — enumerate what each
  implemented provider's reader actually sees and discards. If the residue is
  empty for flox, nix and devbox (expected: only flox declares services at
  all), the change is inert for every existing project and only binds future
  ones.
- **[Trade-off] The obligation lands on provider readers**, which are the
  files most likely to be written by someone adding a provider and least
  likely to be read closely. → A shared helper that returns the unconsumed
  keys, rather than each reader hand-rolling the check, keeps it one thing
  to get right rather than N.

## Open Questions

1. **What does each provider's reader actually discard today?** Not
   measured. flox is the only provider that declares services, so the
   expected answer is "nothing", making this change inert on arrival — which
   would be the ideal outcome for a guard. Worth confirming rather than
   assuming, since an inert guard and an unenforceable one look identical
   until a provider grows.
2. **Does `is_daemon`/`shutdown_command` already lose something?** flox's
   `shutdown.command` is carried, but whether flox's schema has other keys
   devcroft ignores has not been enumerated against flox's current
   documentation.
