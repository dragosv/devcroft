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

## Measured (group 0)

devenv 2.2.2, aarch64-darwin. Two results corrected this design before
any code depended on it.

| question | answer |
|---|---|
| `devenv eval processes` runs `enterShell`? | **no** — 0 sentinel appends |
| does it include integration-contributed processes? | **yes** — `services.redis.enable = true` yields a `redis` process whose `exec` is a store path |
| are `before`/`after` process names? | **no — task names** |
| does devenv validate them? | **no** — a bogus entry evaluates fine and creates no edge |
| can a project set `supervisionMode`? | **no** — the option is read-only |
| is `start.enable = false` reachable? | **yes** |

Per-process defaults, on every process devenv returns:
`restart = {max: 5, on: "on_failure", window: null}`,
`shutdown = {signal: 15, grace: 5}`, `cwd = null`, `ready = null`.

**The `before`/`after` finding is the one that moved the design.** They
are edges in devenv's *task* graph, whose nodes are things like
`devenv:enterShell`, `devenv:files` and `devenv:processes:<name>`.
Measured both ways:

- `after = [ "web" ]` — a bare process name — evaluates without error
  and produces **no edge**. In devenv itself it is a silent no-op.
- `after = [ "devenv:processes:web" ]` produces the edge, visible in
  `devenv tasks list` as `devenv:processes:worker → devenv:processes:web`.

Decision 2 originally mapped `before`/`after` straight onto
`depends_on`. That would have taken a declaration devenv treats as
nothing and given it a meaning inside devcroft — the two would then
disagree about what the same project does. See decision 2a.

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
- `depends_on: Vec<String>` — names of other declared services. What
  fills it for devenv is decision 2a, not `before`/`after` verbatim.
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

### Decision 2a: only the `devenv:processes:<name>` form becomes a dependency

Measured (group 0): `before`/`after` are task-graph edges, not process
references, and devenv validates neither.

So each entry is treated by what devcroft can faithfully honour:

- **`devenv:processes:<name>`, where `<name>` is another process this
  project declares** → `depends_on: <name>`. This is the one form that
  means the same thing on both sides: devenv builds exactly that edge,
  and devcroft's supervisor can reproduce it.
- **A bare name** (`after = [ "web" ]`) → **refused**, naming the
  process and the entry, and saying that devenv itself creates no edge
  for it. This is the important refusal: it is the spelling a user is
  most likely to reach for, it does nothing in devenv, and mapping it to
  a dependency would make devcroft *more* featureful than the provider
  it is reading — the two would disagree about the same project.
- **Any other task reference** (`devenv:enterShell`, `devenv:files`, a
  project's own task) → **refused**. devcroft does not run devenv's task
  graph, so it cannot honour an ordering against a node it never
  executes.
- **`devenv:processes:<name>` naming a process this project does not
  declare** → refused, since the dependency cannot be satisfied.

The alternative — carry `before`/`after` verbatim and let the supervisor
sort it out — was rejected for the reason above: it invents a meaning
the provider does not give them.

### Decision 2b: a declared restart policy is honoured, and that is not a reversal of `add-flox-services` decision 3

`render_config` currently writes `availability.restart = "no"` for every
service, unconditionally, and that was a considered choice: *"a flapping
database is worse than a visibly dead one for the agent-fleet case this
exists to serve."*

devenv declares a restart policy, and its default is
`on_failure` with `max = 5` — so carrying it changes behaviour that an
earlier change deliberately chose. Recorded rather than quietly done.

The two are compatible once the earlier decision is read for what it
actually settled. It was made when **no supported provider declared a
restart policy at all** — flox's `[services]` schema has no such field —
so it chose devcroft's default in the absence of a declaration. It did
not choose to override a declaration, because there were none to
override. So:

- A provider that declares nothing still gets `no`. flox is unaffected,
  asserted by a byte-identical rendered config.
- A provider that declares one gets what it declared, bounded by the
  `max` it declared.

**The visible consequence, which belongs in the docs rather than in a
surprise:** most devenv services will restart on failure where flox
services do not, because that is devenv's default and not usually an
explicit user choice. devcroft cannot distinguish "declared
`on_failure`" from "inherited devenv's default" — the evaluation
normalizes both — so it cannot treat them differently without guessing.
Honouring the value is the option that never lies about what the project
says; the alternative silently discards an explicit `restart.on =
"always"`, which is exactly the silence this change exists to remove.

### Decision 3: everything else is refused by name, not ignored

`ready`, `watch`, `proxy`, `ports`, `listen`, `linux.capabilities` and
`start.enable = false` each cause `up` to fail at layer `provider`,
naming the process and the field. Measured: `start.enable = false` is
reachable from a normal declaration, so that refusal refuses something
real rather than something devenv never emits.

`supervisionMode` is the exception, and for a reason worth recording:
the option is **read-only** — a project cannot set it, and every process
comes back `native`. Refusing a different value is therefore insurance
against devenv itself starting to emit one, not a restriction on users,
and the message says so.

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

### Decision 4a: integration-contributed processes are supervised, and that is said out loud

Measured: `services.redis.enable = true` produces a `redis` process in
`devenv eval processes`, with `exec` pointing into the store. So what
devcroft reads is every process devenv's evaluation produces, not only
the ones written by hand under `processes`.

Taken as-is, because the alternative is worse in both directions:
filtering to handwritten processes would need devcroft to distinguish
them (devenv does not mark them), and would drop exactly the services a
user enabling an integration *wants* supervised.

But it changes what "the project declared" means, so it is stated in the
spec, in the sample's README, and in the refusal messages' framing — a
user who never wrote `processes.redis` and sees `redis` in `devcroft ps`
should be able to find out why without reading devcroft's source.

A pleasant consequence for the closure tier: an integration's `exec` is
a store path, so the supervised command comes from the closure rather
than from whatever is on `PATH`.

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
