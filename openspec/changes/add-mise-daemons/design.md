## Context

Status: planning, 2026-11-17. Nothing here has been run. Split from
`add-mise-provider`; see that change's design for the provider itself.

### Evidence, by how it was obtained

**Read in upstream documentation, 2026-11-17, not run:**

- [mise daemons](https://mise.jdx.dev/daemons.html): enabled by `experimental
  = true` under `[settings]` or `MISE_EXPERIMENTAL=1`. Supervised by
  pitchfork, 2.25.0 or later. mise's own keys: `run`, `task`, `args`, `image`,
  `preset`, `version`, `tool`, `options`, `port`, `ports`, `init`, `mise`,
  `data_dir`, `proxy`, `proxy_tls`, `proxy_idle_timeout`, `project`, `name`,
  `provider`, `resource`. **Any other key is passed through to pitchfork.**
  `init` "must succeed before the process starts" and "runs on every start and
  restart"; the page does not say whether it shares a shell with `run`.
- [pitchfork configuration](https://pitchfork.jdx.dev/reference/configuration):
  `run` (string through a shell, or array run directly), `dir`, `env`,
  `env_file`, `user`, `pty`, `mise`, `retry` (integer or bool, default `0`,
  `true` retries forever), `auto`, `oneshot`, `ready_delay` (default `3`
  seconds, applied only when no other ready check is set), `ready_output`,
  `ready_http`, `ready_port`, `ready_cmd`, `health_cmd`, `health_http`,
  `health_port`, `depends`, `watch`, `watch_mode`, `cron`, `hooks`,
  `memory_limit`, `cpu_limit`, `stop_signal` (default `SIGTERM`; object form
  `{ signal, timeout }`), `boot_start`, `proxy` (default `true`) and its
  siblings, `port` (number, array or object; injects `$PORT`), `logs`. **There
  is no daemon-level startup timeout.** Each ready check takes an optional
  `timeout` in its object form; with none set the check stays open.
- [presets](https://mise.jdx.dev/daemons/presets.html): `postgres`, `redis`,
  `cockroachdb`, `nats`, `spicedb`. "A preset installs its server from the tool
  in the table."
- [process-compose schema](https://github.com/F1bonacc1/process-compose/blob/main/schemas/process-compose-schema.json),
  `main`: a probe is `exec` or `http_get`, nothing else. Shutdown keys are
  `command`, `timeout_seconds`, `signal`, `parent_only`, `send_keys`.

**Measured locally, mise 2026.7.5 (predates daemons; re-measure when pinned):**
`mise registry` maps `postgres` to `vfox:mise-plugins/vfox-postgres` and
`asdf:mise-plugins/mise-postgres`, and `redis` likewise. Both are plugin
backends that record a version only.

**Read in this repository:**

- `services::SHUTDOWN_TIMEOUT_SECS` is 1 and `Shutdown::Signal`'s grace is
  clamped to it. `keeper::connection::DEFAULT_GRACE_PERIOD` is 2s and is also
  the grace a disconnected session gets. `lifecycle::terminate::GRACE_PERIOD`
  is 5s before `down` kills the keeper.
- `docs/implementation-log.md`: process-compose 1.116.0 was measured not to
  escalate to SIGKILL at all. What keeps "no service survives `down`" true is
  the keeper sweeping each service's process group.
- `services::render_config` emits `shutdown.timeout`. The upstream schema's
  key is `timeout_seconds`. **Unverified whether these are the same thing**;
  if the emitted key is ignored, that may be the non-escalation the log
  records. Task 0.2 settles it before anything is built on the clamp.
- `ensure_no_services_declared_for_another_provider` runs only when the
  selected provider reports `ServiceSupport::Unsupported`, reads only flox,
  and returns `Ok` under `--skip-hooks`.
- `ports::substitute` writes an allocated port into the service's `vars`;
  `ports::check_substitutable` requires the command to read the variable.

## Goals / Non-Goals

**Goals:** a mise project's command daemons running under the existing keeper,
with readiness, ordering and shutdown that mean what the project wrote, and a
loud refusal for everything else.

**Non-Goals:** presets; task daemons; containers; pitchfork as a supervisor;
its proxy, TLS, cron, file watching and resource limits; shared databases;
companion or VM services.

## Decisions

### D1. Declarations are data; mise and pitchfork are not involved

devcroft parses `[daemons]` from the selected manifest itself. The derived
configuration `add-mise-provider` hands to mise holds `[tools]` only, so mise
never sees the table and no pitchfork process can be started by it.

The project's opt-in is `[settings] experimental = true` in the manifest,
which that change already admits. `[daemons]` without it is refused by name,
not ignored. The caller's `MISE_EXPERIMENTAL` is not consulted: the manifest
is the input.

`process-compose` must be a tool in `[tools]`, locked through an admitted
backend like any other. Missing, the error names it and gives the two
commands that add and lock it. No host binary is substituted.

All declared daemons start with the sandbox unless `--skip-hooks` is passed.
mise's shell-driven `auto` and its groups are not a start policy devcroft
inherits.

### D2. The admitted field set

| Key | Carried as |
| --- | --- |
| `run`, string | `ServiceDecl::command`, run through devcroft's resolved shell |
| `run`, array | the same, each element single-quoted, so the argument vector is what was written |
| `dir` | `working_dir`; relative to the project; must resolve inside a granted path; `~` refused |
| `env` | `vars` |
| `depends` | `depends_on`; local daemon names only; cycles and unknown names refused before anything starts |
| `port`, one number | `PORT` in `vars`, which is what pitchfork injects |
| `init`, string or list | D3 |
| `retry`, integer | `RestartPolicy::OnFailure { max }`, `0` being `Never` |
| `ready_port`, number | `Probe::Tcp` on `127.0.0.1` (D4) |
| `ready_cmd`, string or array | `Probe::Command` |
| `ready_http`, URL string | `Probe::Http` |
| `stop_signal`, name or `{ signal, timeout }` | `Shutdown::Signal` (D5) |

Everything else that declares something is refused by daemon and dotted key,
through `provider::Unconsumed` and its existing rule. Named so the messages
can say why: `retry = true` (devcroft's restart policies are bounded);
`port` as an array, `"auto"` or an object; `ports`; `mise = false` (asks for
an environment other than the captured one); the object forms of the ready
checks, which exist to carry `timeout`; `ready_output`; an explicit
`ready_delay`; `health_*`; `env_file`; `user`; `pty`; `oneshot`; `watch`;
`cron`; `hooks`; `memory_limit`; `cpu_limit`; `auto`; `boot_start`; `logs`;
`proxy` set to anything but `false`, and its siblings; and the forms in D8.

### D3. Upstream defaults devcroft cannot inherit silently

The first draft required "effective defaults from the pinned schema" to be
preserved. Four of them need a decision, because preserving them is either
impossible or unmeasured.

**`ready_delay`.** With no ready check, pitchfork counts a daemon ready once
it has stayed up three seconds, and dependents wait for that. devcroft's
existing rule for a probe-less target is `process_started`, which does not
wait. Rendering: a `depends` edge onto a daemon with no ready check gives that
target a probe that succeeds trivially after an initial delay of three
seconds, so the dependent waits as it would upstream. Task 0.2 measures
whether process-compose leaves a process unready when it has exited before
the delay elapses. If it does not, the edge is refused by name with the fix
(declare a ready check on the target), since the alternative is starting the
dependent early without saying so.

**`proxy`.** Defaults to `true` upstream, giving a port-bearing daemon a
hostname. devcroft runs no proxy. A default is not a declaration under the
`Unconsumed` rule, so this is not refused; it is documented as not carried.

**`init`.** Upstream says it must succeed before the process starts and runs
on every start and restart. Whether its exports and working directory reach
`run` is not documented. Task 0.1 measures it, and the rendering reproduces
what is measured: one shell if they share one, separate subshells if they do
not. The requirement states the ordering and the failure rule, which are
documented, and does not assert shell sharing.

**`run` through a shell.** Upstream does not say which. devcroft uses its own
resolved POSIX shell. A command that relies on another shell's syntax fails
visibly at start. Documented.

Probe timing uses devcroft's existing defaults, since pitchfork documents
none. `retry` as an integer is mapped to `OnFailure` on the understanding
that pitchfork retries a failed process; 0.1 confirms when it applies.

### D4. TCP readiness is devcroft's own probe

process-compose cannot probe a TCP port. The draft required a "dedicated
check, not an HTTP check or an unverified `nc` command", which left nothing.

`Probe::Tcp { host, port }` is added and rendered as an `exec` probe running
`<devcroft> __probe-tcp <host> <port>`: connect with a timeout, exit zero on
success. The subcommand is hidden, like `__keeper` and `__provision`. It
depends on nothing in the project's environment, which is the point: a probe
that needs `nc` from the closure is ready or not depending on what the
project happened to install.

It needs devcroft's binary to be executable inside the sandbox. Task 0.3
measures whether it already is on both backends. If not, the binary's file is
granted read-only with origin `baseline`, shown by `policy --render` like
every other rule.

The per-check `timeout` is refused, by its real name (`ready_port.timeout`,
`ready_cmd.timeout`, `ready_http.timeout`), for the reason
`add-service-readiness` Decision 2 gives for devenv's: process-compose has no
wall-clock deadline and devcroft will not approximate one with attempt
counts. Per Decision 4 a probe that never passes does not block `up`;
`status` reports `started-but-not-ready` and dependents wait.

### D5. A declared grace is honoured, which means changing teardown

Today a grace above one second is clamped, as a teardown guarantee: the
keeper sweeps service process groups two seconds after it is told to stop,
and `down` kills the keeper at five. Lifting the clamp alone would have the
keeper kill a service mid-shutdown.

- A sandbox's **service grace** is the largest grace any of its services
  declares, with a floor of today's value. It is computed at `up` and recorded
  in `Meta`.
- A declared grace above `MAX_SERVICE_GRACE_SECS` (30) is **refused by
  name**. Not clamped: a clamp is a silent change to what was written.
- The keeper's wait before sweeping service groups becomes the service grace.
  The wait a disconnected session gets stays `DEFAULT_GRACE_PERIOD`; the two
  stop being one number, and the comment that says they must be is rewritten.
- `down` waits `terminate::GRACE_PERIOD` plus the service grace before
  killing the keeper.
- The ordering that matters is asserted on the computed values, by test:
  supervisor timeout below keeper wait below `down`'s kill.
- `Shutdown::Default` and `Shutdown::Command` keep one second.

This applies to every provider that declares `Shutdown::Signal`. devenv's
declared grace stops being clamped. That is a behavior change for devenv and
is called out in the docs and the implementation log, not smuggled in.

`stop_signal` accepts the six names pitchfork allows. With no `timeout`,
pitchfork falls back to a supervisor-level default whose value the reference
does not give; 0.1 measures it, and that value is used, subject to the cap.

### D6. Ports

A daemon's fixed `port` becomes `PORT` in its `vars`. That composes with what
exists: `[network.services.<name>] var = "PORT"` asks for an allocation where
sandboxes share loopback, `substitute` writes the allocated port over it, and
`check_substitutable` requires `run` to read `$PORT`.

One extension: when a daemon's `ready_port` equals its declared `port` and
that port is allocated, the probe follows the allocation. When `ready_port`
names a different number under an allocation, the service is refused, since
the probe would watch a port nothing was told to bind.

Discovery opens nothing. An undeclared port stays refused by the existing
network policy.

### D7. The cross-provider check keeps its condition

`ensure_no_services_declared_for_another_provider` gains a best-effort read of
mise `[daemons]`, beside flox's. Its condition is unchanged: it fires only
when the selected provider has no service concept, and never under
`--skip-hooks`. A flox or devenv project that also carries a `mise.toml` with
daemons is not refused, exactly as a devenv project carrying a flox manifest
with services is not refused today. The draft's unconditional wording would
have broken those projects. An unreadable `mise.toml` in a project that did
not select mise is not an error.

### D8. Presets and the other forms are refused, with a way back for presets

Refused by daemon and key: `preset`, the shorthand `name = "version"`,
`version`, `tool`, `options`, `data_dir`; `task` and `args`; `image`;
`project` and `name`; `provider` and `resource`.

Presets are refused because their server tool comes from a plugin backend,
which records no URL or checksum and runs plugin code to install, on the one
version measured. Supporting them also needs per-instance data directories,
connection variables that follow an allocated port, an upgrade rule for
on-disk formats, and on macOS an answer to PostgreSQL's shared-memory needs
under Seatbelt. That is a change of its own.

What reopens it: the pinned mise resolving a preset's server through an
admitted backend, or the owner deciding that `tool = "conda:…"` is an
acceptable required override. Task 0.4 records the first.

### D9. A daemon is workload code

Imported daemons are ordinary `ServiceDecl` values under the sandbox's own
policy. `run`, `init` and `ready_cmd` are project code whatever the daemon is
called. Nothing here classifies a service as a trusted companion, and the VM
material in `new/` is not implemented by it.

## Risks / Trade-offs

- Experimental schema churn → fixtures pinned per qualified version; an
  unknown key is refused rather than dropped, so drift is loud.
- The admitted set is narrower than upstream → published exactly; each
  refusal says what to write instead where there is something.
- Longer `down` → bounded by the cap and only when a service declares it.
- devenv changes behavior → stated; its tests gain a grace above one second.
- No presets → the headline upstream feature is absent. Stated up front in
  the proposal and in the docs.

## Migration Plan

1. Group 0, after `add-mise-provider`'s.
2. Shutdown grace (D5), on its own, verified against devenv before any mise
   daemon exists.
3. `Probe::Tcp` and the helper.
4. The mise reader and its refusals.
5. Ports, the cross-provider check, tests and the sample.

Rollback removes the reader; D5 and D4 stand on their own.

## Open Questions

All in Group 0:

- Does `init` share a shell with `run`?
- When does `retry` apply, and what is the default stop timeout?
- Does process-compose honour `shutdown.timeout`, or only `timeout_seconds`,
  and does it escalate with the right key?
- Does a trivial probe after an initial delay reproduce `ready_delay`?
- Is devcroft's binary executable inside the sandbox today?
