# Change: add-service-readiness

Status: proposed. devenv's readiness schema below is read from devenv
2.2.2's own `src/modules/lib/ready.nix`, not inferred from field names —
an earlier guess at it (`failureThreshold`, `ready.http.path`) was wrong
in both spelling and shape, which is why this one cites the source.

## Why

**A readiness probe is the one refused field whose absence is a lie
rather than a gap.** `add-devenv-services` refuses eight of devenv's
process fields, and seven of them fail honestly: the user asked for
something devcroft does not do, and is told so. Readiness is different in
kind. A service with a probe reports healthy *when the probe passes*; the
same service without one reports healthy the moment it is spawned. Carry
the declaration wrongly and a dependent service starts against a database
that is still opening its socket.

That is why it was refused rather than dropped, and it is also why it is
the first refusal worth removing.

**It is also the prerequisite that decides a second question.**
`docs/decisions.md` records devbox's services as not built, with the cost
of reconsidering measured — and one measured item is that `postgresql`,
the most common devbox service, declares a `readiness_probe` while
`redis` does not. So devbox support is gated on this change existing,
whatever is decided about the contract question. Building it here turns
that decision from "can it be done" into the one question worth arguing
about.

**The translation is nearly mechanical, which is the good news.**
devenv's schema and process-compose's differ in spelling, not in
concepts, and the devbox plugin files use process-compose's shape
directly. One `ServiceDecl` field serves both providers.

## What Changes

- **`ServiceDecl` gains `readiness: Option<Readiness>`**, a
  supervisor-neutral description of when a service is ready: a command to
  run, or an HTTP endpoint to poll, plus the timing and thresholds.
- **devenv's `ready` is carried instead of refused**, mapped from the
  schema below.
- **`render_config` emits process-compose's `readiness_probe`**, and —
  this is the part that makes the feature worth having — a dependency on
  a service that declares one waits for **ready**, not merely for
  *started*. Today `depends_on` is emitted with
  `condition: process_started` precisely because nothing established
  readiness; that comment stops being true here.
- **Two of devenv's readiness fields stay refused**, by name: `notify`
  (systemd's `READY=1` protocol, which devcroft's supervisor does not
  implement) and the overall `timeout` deadline (process-compose's
  readiness probe has no equivalent — its thresholds bound attempts, not
  wall-clock).
- **No devbox change.** This removes devbox's *mechanical* blocker and
  nothing else; the contract question in `docs/decisions.md` is untouched
  and stays the deciding one.

## Capabilities

### New Capabilities

None. This extends the existing `services` capability.

### Modified Capabilities

- `services`: adds readiness as a declarable property of a service, and
  strengthens the dependency requirement — where a dependency's target
  declares readiness, the dependent SHALL wait for ready rather than for
  started.

## Impact

- **Affected specs**: `services`.
- **Affected code**: `src/provider/mod.rs` (`ServiceDecl`, a new
  `Readiness` type), `src/provider/devenv.rs` (map instead of refuse),
  `src/services/mod.rs` (`render_config`, and the `depends_on` condition
  it emits).
- **devenv 2.2.2's schema**, from `src/modules/lib/ready.nix`:

  | field | type | default |
  |---|---|---|
  | `exec` | string? | `null` |
  | `http.get` | `{host, port, path, scheme}`? | `null` |
  | `notify` | bool | `false` |
  | `initial_delay` | int | `0` |
  | `period` | int | `10` |
  | `probe_timeout` | int | `1` |
  | `timeout` | int? | `null` |
  | `success_threshold` | int | `1` |
  | `failure_threshold` | int | `3` |

  process-compose's `readiness_probe` takes `exec.command`,
  `http_get{host,port,path,scheme}`, `initial_delay_seconds`,
  `period_seconds`, `timeout_seconds`, `success_threshold` and
  `failure_threshold`. Every devenv field but `notify` and `timeout` has
  a counterpart.
- **A behaviour change for existing devenv sandboxes, and it is the
  point.** A project whose `depends_on` currently means "the other
  process has been spawned" will, once its target declares readiness,
  mean "the other process answered". That is what the declaration asks
  for. It is called out because a service that used to start immediately
  may now wait, and a probe that never passes is a sandbox that never
  finishes starting — see Open Questions.

## Success Criteria

- A devenv project declaring `ready.exec` or `ready.http.get` comes up,
  and `status` reports the service ready only once the probe passes.
- A dependent service declared with `after = [ "devenv:processes:<name>" ]`
  on a target with a probe starts **after ready**, verified by observing
  the order rather than by reading the generated config.
- `notify` and the overall `timeout` fail `up` at layer `provider`,
  naming the process and the field, exactly as the other refusals do.
- A service declaring no readiness renders byte-identically to today —
  asserted by the existing golden, not by inspection.
- flox services are untouched: flox declares no readiness, so its
  rendered config is unchanged.

## Open Questions

- **What happens when a probe never passes.** process-compose's
  `failure_threshold` marks the service unhealthy, but what devcroft's
  `up` does then is this change's to decide: block, or come up and report
  the service as not ready. The `services` spec's "services do not block
  sandbox availability" requirement points at the second, and if so the
  interaction with a dependent service that is still waiting has to be
  stated rather than left to process-compose.
- **Whether `http.get` inside a sandbox needs a port grant.** The probe
  runs from process-compose, inside the boundary, against a loopback port
  the service binds — so it should be covered by the same
  `network.ports` grant the service needs. Needs measuring, not assuming;
  on macOS the port grant is degraded anyway
  (`samples/devenv-services-sample`).
- **Whether devbox's plugin probes translate unchanged.** They are
  already written in process-compose's own shape, so they should need no
  mapping at all — but that is an argument from reading one file, and
  devbox is not in this change's scope regardless.
