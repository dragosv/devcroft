## Context

See proposal.md — Why, for the measurements. What matters here is the
shape they leave behind: devenv declares more about a service than
devcroft can currently represent, and the gap is not with the supervisor
but with devcroft's own intermediate type.

`ServiceDecl` carries five fields and they are flox's schema. What
`devenv eval processes` returns, per process, measured on 2.2.2:

```
exec, env, cwd, ready, restart{max,on,window}, shutdown{signal,grace},
listen, ports, before, after, watch{paths,extensions,ignore},
proxy{hostname,https}, linux{capabilities}, start{enable},
supervisionMode, process-compose{…}
```

Three of those — `cwd`, `restart`, `shutdown` — describe things flox
also expresses, differently. The rest are devenv concepts devcroft has
no position on yet.

## Goals / Non-Goals

**Goals:**

- Read devenv's declarations as data, through an entry point measured to
  run no project code.
- Carry what devcroft can represent, whole.
- Refuse what it cannot, by name, before anything starts.
- Keep `ServiceDecl` supervisor-neutral, which `decouple-service-supervisor`
  just made true.

**Non-Goals:**

- devbox services. Measured and refused; see proposal.md — Why.
- A general fidelity rule for every provider — that is
  `fix-lossy-service-translation`'s.
- devenv's own supervisor. devcroft never touches it, and reading
  declarations is not an exception to that.

## Decisions

### Decision 1: read declarations via `devenv eval processes`

Measured: it returns every declared process as normalized JSON and runs
`enterShell` zero times (sentinel method). It shares the `devenv eval`
entry point already measured hook-free when the devenv provider reads
`enterShell` itself, so the provider now reads *both* the hook and the
service declarations through the same hook-free command.

**Alternative considered: `devenv processes up` / `devenv up`.**
Rejected on sight — it starts the processes, which is activation rather
than enumeration, and would run project code host-side.

**Alternative considered: reading devenv's generated process-compose
file under `.devenv/`.** Rejected for the reason `add-flox-services`
decision 1 gave for flox's generated `service-config.yaml`: a generated
file is not a contract. That refusal is the whole reason devenv
qualifies where devbox does not, so consuming devenv's generated output
here would make the distinction meaningless.

### Decision 2: `ServiceDecl` grows by a supervisor-neutral vocabulary, not by a union of two providers' field names

The tempting shape is to add devenv's fields beside flox's — `cwd`,
`restart_on`, `restart_max`, `shutdown_signal`, `shutdown_grace` — and
leave `shutdown_command` where it is. That produces a type where two
optional field groups describe the same concept and nothing says which
combinations are legal.

The other tempting shape is to make `ServiceDecl` process-compose's
vocabulary, since that is what devcroft generates and both providers
express themselves in it. Rejected: `decouple-service-supervisor` just
put the supervisor behind a seam, and pushing its vocabulary into the
type every provider fills would undo that in the one place it matters.

So the type grows by *concepts*, each expressible by both providers and
requestable from any plausible supervisor:

- `working_dir: Option<PathBuf>` — devenv's `cwd`.
- `depends_on: Vec<String>` — devenv's `before`/`after`. Names of other
  declared services, validated to exist.
- `restart: RestartPolicy` — `Never` | `OnFailure { max }` | `Always`.
- `shutdown: Shutdown` — an **enum**, not two optional fields:
  `Command(String)` for flox's `shutdown.command`, `Signal { signal,
  grace }` for devenv's. This is the decision that keeps the type
  honest: the two providers stop a service by genuinely different
  mechanisms, and an enum says so where a pair of `Option`s would let
  both be set and neither be meaningful.

`is_daemon` stays and is always `false` for devenv, which has no such
concept. It is flox's, it is still flox's, and the field's doc comment
says so rather than pretending it is general.

### Decision 3: everything else is refused by name, not ignored

`ready`, `watch`, `proxy`, `ports`, `listen`, `linux.capabilities`,
`start.enable = false`, and any `supervisionMode` other than `native`
each cause `up` to fail at layer `provider`, naming the process and the
field.

This is deliberately stricter than it needs to be for a first version,
and the reason is the failure mode it avoids. A devenv project declaring
a readiness probe and getting a service without one has been *lied to*:
the service reports healthy on a condition nobody checked. Refusing is
recoverable — the user removes the field or waits for support. A
silently dropped probe is not, because nothing surfaces it.

Two of these are refusals with a stated successor rather than
permanently:

- `ports`/`listen` overlap `network.ports` and `add-port-allocation`
  rather than sitting beside them. Deciding what a devenv `ports`
  declaration means for a sandbox's port policy is that change's, not
  this one's.
- `ready` is the one most likely to be wanted next, and process-compose
  supports readiness probes natively — so the work is translation, not
  mechanism.

### Decision 4: the `process-compose` passthrough field is refused

devenv lets a project write arbitrary process-compose settings under a
`process-compose` key, and devcroft's supervisor *is* process-compose,
so forwarding them would work today.

Refused anyway, and the reason is the seam. `ServiceDecl` is
supervisor-neutral by decision 2; a passthrough field is
supervisor-specific project configuration, and carrying it would mean
every future supervisor has to answer "what do I do with the
process-compose block" — which is the coupling
`decouple-service-supervisor` removed.

**This has a real cost and it is not hypothetical**: expressing a
dependency between processes through `process-compose.depends_on` is
common in real devenv projects, and such a project is refused here even
though `depends_on` is carried when declared through devenv's own
`before`/`after`. The refusal message must say that, since the user's
fix is to restate the dependency in devenv's own vocabulary rather than
to give up.

### Decision 5: devbox's refusal moves from deferral to measurement

No behaviour change — `ServiceSupport::Unsupported` already fails
distinguishably through `ensure_no_services_declared_for_another_provider`.
What changes is that the reason in `devbox.rs` stops saying "a separate
change's decision to make" and states the three measured findings, and
that `docs/decisions.md` carries them where a future reader looks.

`add-devbox-provider` task 3.6 closes on this plus the loud-failure test
it asked for — not because the test was hard, but because its stated
blocking reason ("the mechanism exists for no provider") stopped being
true when `add-flox-services` built it, and CLAUDE.md already flags that
as stale.

## Risks / Trade-offs

- **`devenv eval processes` is not separately version-pinned.** Measured
  hook-free on 2.2.2 only. → The sentinel test covers it on every run
  where devenv is available, so a version that starts running the hook
  breaks CI rather than a user's host.
- **Decision 3 refuses more than most first versions would**, so some
  real devenv projects will not come up. → Accepted, with the message
  naming the field and the reason. The alternative is a service whose
  declared behaviour and actual behaviour differ silently.
- **Decision 4 refuses the common way to express dependencies.** →
  Accepted, with the message pointing at `before`/`after`. Revisiting it
  means deciding what a supervisor-specific block means in a
  supervisor-neutral type, which is a real design question rather than a
  small feature.
- **`ServiceDecl` growing here partly anticipates
  `fix-lossy-service-translation`.** → Bounded deliberately: this change
  refuses devenv's unrepresentable fields at devenv's own translation
  point; the general rule, and the shape of the error every provider
  uses, stays in that change. Both proposals say so, so neither absorbs
  the other by accident.

## Migration Plan

None required. Additive for devenv, unchanged for every other provider:
a manifest whose provider declares no services compiles and renders
identically. `ServiceDecl`'s new fields have defaults that reproduce
today's behaviour for flox, which is asserted rather than assumed.

## Open Questions

- Whether `supervisionMode` can take a value other than `native`, and
  what it would mean for a process devcroft supervises itself. Refused
  until measured.
- Whether devenv validates `before`/`after` against declared process
  names itself, or whether devcroft must — a dependency on a
  non-existent service is a startup failure worth catching at `up`.
- Whether `devenv eval processes` reflects processes contributed by
  devenv *integrations* (its language and service modules) as well as
  ones the project wrote by hand. If it does, a project enabling a
  database integration gets supervised services it never wrote in
  `processes` — desirable, but it changes what "the project declared"
  means and should be measured before it is claimed either way.
