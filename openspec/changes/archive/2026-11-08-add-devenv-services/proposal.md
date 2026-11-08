# Change: add-devenv-services

Status: proposed. Written against a running devenv 2.2.2 and devbox
0.17.5 on aarch64-darwin. Every claim below about which entry point
returns what, and which one runs project code, is measured with the
sentinel method — not read from documentation.

## Why

**devcroft supports services for exactly one provider, and the two
deferrals that kept it that way have now been measured. They came back
with opposite answers.**

`add-devbox-provider` left task 3.6 open and `add-devenv-provider`
deferred services outright, both for the same stated reason: it was not
known whether those providers' service declarations are readable as a
*contract* or only as generated output. That distinction is not
pedantry — `add-flox-services` decision 1 refused flox's own generated
`service-config.yaml` on precisely that ground, and devcroft generates
its own supervisor config instead. Answering the question for two
providers at once is what makes this worth doing now.

**devenv passes, and cleanly.** `devenv eval processes` returns every
declared process as normalized JSON and runs `enterShell` **zero**
times. The declarations come from `processes.<name>.exec` in the
project's own `devenv.nix` — a documented option in devenv's manifest
schema, which is the same bar flox met. Read as data, host-side,
hook-free.

**devbox fails, on three measured counts**, and the third is the one
that settles it:

1. `devbox.json` has no service schema at all. The binary's own JSON
   tags carry `packages`, `env`, `env_from`, `include`, `init_hook`,
   `scripts` and the `environment*` family — and nothing for a service.
2. Plugin services arrive as `.devbox/virtenv/<plugin>/process-compose.yaml`
   — generated, per-plugin, under a cache directory. That is the exact
   shape decision 1 rejected.
3. **`devbox services ls` executes `shell.init_hook`.** Measured against
   a sentinel: `devbox install` runs it 0 times, `devbox shellenv --pure`
   0 times (the control, and the route devcroft's capture already uses),
   `devbox services ls` **once**. So merely *enumerating* devbox's
   services host-side runs project code, which is a two-phase violation
   on the one path support would require.

So this change gives devenv services and gives devbox a **recorded
rejection with a falsifiable reason**, rather than leaving a task open
whose blocking reason nobody has checked. If devbox ever adds a
declaration schema and a hook-free way to read it, reason 3 is the one
to re-measure first.

## What Changes

- **`env-provider`: devenv resolution reads service declarations.**
  `ServiceSupport` moves from `Unsupported` to `Declared`, populated
  from `devenv eval processes` during resolution — host-side, before
  restriction, as data. The command is never `devenv processes up` or
  anything else that would start or exec a process to find out what is
  declared.
- **`services`: devenv joins flox as a provider whose declarations
  devcroft supervises.** Every existing invariant holds unchanged —
  services run inside the boundary, the keeper owns their lifetime for
  the sandbox's lifetime, failure is visible rather than silent,
  services and hooks stay distinct mechanisms with a stated precedence,
  and service artifacts belong to one sandbox.
- **`ServiceDecl` grows, and this change says what it is growing
  toward.** Today it carries five fields and they are *flox's* schema:
  `name`, `command`, `vars`, `is_daemon`, `shutdown_command`. devenv
  declares more than that — a restart policy, dependencies between
  processes, a readiness probe, a working directory, and shutdown as a
  signal plus a grace period rather than as a command. Dropping those
  silently is forbidden by `fix-lossy-service-translation`, which named
  devenv as the live case it was written for. See Impact for how the two
  changes divide the work rather than overlap.
- **devbox stays `ServiceSupport::Unsupported`**, now with the measured
  reason inline in the code and in `docs/decisions.md` rather than a
  deferral. `add-devbox-provider` task 3.6 closes on that measurement
  plus the loud-failure test it asked for.
- **`samples/devenv-sample` gains processes.** It currently states that
  services are unsupported, which this change makes false.

## Capabilities

### New Capabilities

None. devenv is a second implementation of the existing `services`
capability, the same reasoning every additional provider has given.

### Modified Capabilities

- `services`: adds a requirement that a provider's declarations are read
  as data through an entry point that runs no project code, and that a
  provider whose service declarations cannot be read that way is
  refused rather than partially supported. Adds devenv as a declaring
  provider alongside flox.
- `env-provider`: devenv's resolution requirement gains the service
  declarations it reads and the entry point it reads them through;
  devbox's `ServiceSupport::Unsupported` gains its measured reason.

## Impact

- **Affected specs**: `services`, `env-provider`.
- **Affected code**: `src/provider/devenv.rs` (read and map the
  declarations), `src/provider/mod.rs` (`ServiceDecl`),
  `src/services/mod.rs` (`render_config`, the translation point the
  supervisor seam already isolates), `src/provider/devbox.rs` (the
  comment that states why, updated from deferral to measurement).
- **How this divides with `fix-lossy-service-translation` (0/13), which
  must not be quietly absorbed.** That change owns the *general* rule: a
  declaration devcroft cannot represent fails at layer `provider` naming
  what it could not carry. This change owns *devenv's* fields
  specifically, and it cannot wait — landing devenv services on today's
  five-field `ServiceDecl` would silently discard most of what a real
  devenv project declares, creating exactly the silence that change
  exists to remove. So this one carries devenv's fields and refuses what
  it cannot map, for devenv; the general rule, and its application to
  every future provider, stays there.
- **A fact that makes the translation cheaper than it looks, and is
  worth stating because it also tests that change's thesis.** devenv's
  process schema *is* process-compose's vocabulary — it even carries a
  literal `process-compose` passthrough field — and devcroft's own
  generated config is process-compose. The mismatch is therefore not
  between devenv and the supervisor; it is between devenv and
  `ServiceDecl`, devcroft's flox-shaped intermediate. That is evidence
  for `fix-lossy-service-translation`'s diagnosis rather than against
  it.
- **Not in scope: devenv's `supervisionMode`, `watch`, `proxy` and
  `ports`.** devenv has its own supervision and file-watching concepts,
  and `ports`/`listen` overlap with `network.ports` and
  `add-port-allocation` rather than sitting beside them. Each is refused
  loudly rather than ignored, and each is a separate decision.
- **Unblocks nothing, closes one thing**: `add-devbox-provider` task
  3.6, whose stated blocking reason CLAUDE.md already flags as stale.

## Success Criteria

- A devenv project declaring `processes.web.exec` comes up with that
  service supervised by the keeper, visible in `devcroft ps`, and reaped
  at `down` — the same guarantees flox services already have.
- **`devenv eval processes` runs no project code**, proven by the
  sentinel method rather than by reading devenv's output: an
  `enterShell` with an observable side effect leaves it untouched across
  a full resolution that reads the declarations.
- A devenv process declaring something devcroft cannot carry **fails at
  layer `provider` naming the field**, and never starts a partially
  translated service.
- A devbox project declaring services still fails distinguishably from
  "supports services, none declared", and `doctor`/`up` messages name
  devbox's own reason rather than suggesting another provider.
- `policy --render` and the compiled policy are unchanged for any
  manifest whose provider declares no services.

## Open Questions

- **Whether `devenv eval processes` stays hook-free across versions.**
  Measured for 2.2.2 only. It shares the `devenv eval` entry point
  already measured hook-free for `enterShell`, which is reassuring but
  is not the same command.
- **What `supervisionMode: "native"` means for a process devcroft
  supervises itself.** devenv returns it on every process; devcroft
  never uses devenv's supervisor. Whether a non-`native` value changes
  what the declaration means has to be measured before any value other
  than `native` is accepted.
- **Whether the `process-compose` passthrough field should reach
  devcroft's generated config.** It is project-controlled configuration
  for devcroft's own supervisor, which is a different trust question
  from a project-controlled *command* — the command already runs inside
  the sandbox, the config shapes how devcroft runs it.
