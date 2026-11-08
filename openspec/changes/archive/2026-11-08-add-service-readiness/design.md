## Context

devenv 2.2.2's readiness schema, read from its own
`src/modules/lib/ready.nix` rather than inferred:

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
`period_seconds`, `timeout_seconds`, `success_threshold`,
`failure_threshold`.

Everything but `notify` and `timeout` has a counterpart. The devbox
plugin files are already written in process-compose's shape, which is
what makes one `ServiceDecl` field serve both providers.

## Goals / Non-Goals

**Goals:**

- Carry readiness as a supervisor-neutral concept, the way
  `add-devenv-services` carried restart and shutdown.
- Make a dependency mean "ready" where readiness is declared.
- Refuse what cannot be translated, by name.

**Non-Goals:**

- devbox services. This removes their mechanical blocker; the contract
  question in `docs/decisions.md` is untouched and stays deciding.
- A devcroft-native probe runner. The supervisor owns probing, the same
  way it owns restart policy and daemon handling.

## Decisions

### Decision 1: `Readiness` is an enum over the probe, with timing beside it

```
Readiness { probe: Probe, initial_delay, period, timeout, success, failure }
Probe = Command(String) | Http { host, port, path, scheme }
```

The probe is an enum because a service is ready by *one* means; the
timing applies whichever it is. Two optional probe fields would permit
both set and neither meaningful — the same reasoning that made
`Shutdown` an enum in `add-devenv-services`.

Supervisor-neutral by the same rule as the rest of `ServiceDecl`: these
are concepts any supervisor could be asked for, not process-compose's
field names.

### Decision 2: `notify` and the overall `timeout` are refused, by name

`notify` is systemd's `READY=1` protocol over `NOTIFY_SOCKET`. The
supervisor does not implement it, and devcroft is not going to speak it
on the supervisor's behalf — a probe devcroft simulated would be a probe
whose result devcroft invented.

devenv's `timeout` is an overall deadline for becoming ready.
process-compose's readiness probe bounds *attempts* (`failure_threshold`
× `period`), not wall-clock. Approximating the deadline by computing a
threshold would give a different guarantee under the same name, which is
the failure mode every refusal in `add-devenv-services` exists to avoid.

Both refusals name the process and the field, and say what they would
have meant.

### Decision 3: the dependency condition becomes conditional

`render_config` currently emits `condition: process_started` for every
dependency, with a comment saying readiness is refused so waiting on
health would wait on a condition nothing establishes. That comment stops
being true here.

So: `process_healthy` where the target declares readiness,
`process_started` where it does not. Not `process_healthy` everywhere —
against a target with no probe that would wait on something the
supervisor never reports, which is a hang rather than an ordering.

This is the change's real payoff. Carrying a probe that nothing consults
would be bookkeeping.

### Decision 4: a probe that never passes does not block `up`

The `services` capability already requires that services do not block
sandbox availability, and readiness does not get an exception: `up`
completes, the service is reported started-but-not-ready, and its
dependents stay waiting.

The alternative — `up` blocking until every probe passes — turns one
mistyped health path into a sandbox that never comes up, and hides the
diagnosis behind a hang. Reported state is the better failure: `status`
shows what is ready and what is not.

Consequence to state plainly: a dependent service of a never-ready target
never starts, and is reported as waiting rather than as failed. That is
what the declaration asked for, and it is visible.

## Risks / Trade-offs

- **Existing devenv sandboxes change behaviour** where a target declares
  readiness: dependents that used to start immediately now wait → that is
  the declaration's meaning, and it is called out in the sample and the
  changelog rather than discovered.
- **A wrong probe turns into a service that is never ready** → decision 4
  makes that visible in `status` instead of a hang, and the probe is the
  project's to fix.
- **`http.get` from inside the sandbox may need a port grant** → measured
  before it is claimed either way; it is a loopback connection from the
  supervisor to a port the service already binds, so the existing grant
  should cover it.

## Migration Plan

None. A service declaring no readiness renders byte-identically, which
the existing golden asserts. flox declares no readiness at all.

## Open Questions

- Whether process-compose's `http_get` needs the scheme spelled
  separately or accepts a URL — the devbox plugin files use the
  structured form, which is what this design assumes.
- What `status` should call a service that is started, not ready, and has
  no dependents waiting on it. "Running" is what the supervisor says;
  whether devcroft distinguishes it is a reporting decision, not a
  translation one.
