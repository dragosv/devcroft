# Change: add-devbox-services

Status: proposed. Every measurement below is against devbox 0.17.5 and
process-compose 1.120.0 on aarch64-darwin.

## Why

**The contract question has been decided, and this change records the
decision rather than re-arguing it.** `docs/decisions.md` left devbox
services not built for one reason: the declarations are the *plugin
author's*, not the project's — someone writing `"packages":
["postgresql"]` has not declared a service. The project owner has
decided to build them anyway. That judgement is theirs; this proposal
implements it and keeps the reasoning visible so the trade stays legible
rather than being quietly forgotten.

The two other costs that entry recorded have both moved:

- **Readiness probes existed as a blocker and no longer do.**
  `add-service-readiness` built them. Measured: `postgresql`'s plugin
  config declares `readiness_probe.exec`, which is why the most common
  devbox service would have failed on day one. It now translates.
- **The route that runs project code is avoidable, and always was.**
  `devbox services ls` executes `shell.init_hook` (sentinel-measured).
  `devbox install` alone — zero hook executions — already writes the
  plugin configs, and reading a file executes nothing. That is the route
  this change takes.

## What Changes

- **`devbox`'s `ServiceSupport` moves from `Unsupported` to
  `Declared`**, read from `.devbox/virtenv/<plugin>/process-compose.yaml`
  as data at `up`, never through `devbox services ls`.
- **The declarations translate into `ServiceDecl`**, so devbox services
  get every guarantee flox's and devenv's already have: supervised by
  the keeper, enumerable in `status`/`ps`, reaped at teardown, subject to
  the compiled policy.
- **A field the system cannot carry is refused by name**, the same rule
  `add-devenv-services` set. Measured across four plugins: nothing
  currently needs refusing (see Impact), which makes the refusal a guard
  against future plugins rather than a present obstacle.
- **`docs/decisions.md`'s entry moves from "not built" to built**,
  keeping the contract argument as recorded reasoning — the decision was
  made against it, not because it was wrong.
- **One new dependency**, a YAML parser. devcroft has none today; see
  Impact.

## Capabilities

### New Capabilities

None. A third implementation of the existing `services` capability.

### Modified Capabilities

- `services`: adds devbox as a declaring provider, and states that a
  provider's declarations may be authored by the provider's own plugins
  rather than by the project — the thing this change decided.
- `env-provider`: devbox's resolution captures service declarations;
  the "refused for a measured reason" requirement is replaced.

## Impact

- **Affected specs**: `services`, `env-provider`.
- **Affected code**: `src/provider/devbox.rs` (read and translate),
  `docs/decisions.md`, `Cargo.toml`.
- **The plugin vocabulary is small, and all of it already maps.**
  Surveyed across `postgresql`, `redis`, `nginx` and `mysql` — every
  plugin that ships a config, of the four installed:

  | field used | maps to |
  |---|---|
  | `command` | `ServiceDecl::command` |
  | `is_daemon` | `ServiceDecl::is_daemon` |
  | `shutdown.command` | `Shutdown::Command` |
  | `availability.restart` (`always`/`on_failure`) | `RestartPolicy` |
  | `availability.max_restarts` | `RestartPolicy`'s `max` |
  | `readiness_probe.exec.command` | `Probe::Command` |

  Nothing uses `depends_on`, `http_get`, or any form `ServiceDecl` lacks.
  **Zero translation loss for every plugin measured** — which is a
  statement about four plugins, not about the registry.
- **A plugin contributes one *or more* services.** `nginx` declares
  three (`nginx`, `nginx-error`, `nginx-access`), `mysql` two. The
  mapping is per-process, not per-plugin.
- **The dependency, stated plainly.** Reading these files needs a YAML
  parser, and devcroft's tree has none — checked: zero YAML crates
  across 335 dependencies. It emits JSON precisely to avoid a serializer.
  `nginx`'s config uses a block scalar (`command: |`) with a multi-line
  shell body, which rules out the hand-rolled subset parser devcroft used
  for devenv's `declare -x` dump: that one succeeded because its grammar
  was trivial, and YAML's is not.

## Success Criteria

- A devbox project declaring `postgresql` comes up with the service
  supervised, visible in `status` with a pid, ready only once its probe
  passes, and reaped at `down`.
- **Reading the declarations runs no project code**, proven with the
  sentinel method rather than by choosing the right command: a project
  with an `init_hook` that writes outside the project root has that file
  untouched across a full `up`.
- A plugin declaring several processes yields several services.
- A field devcroft cannot carry fails `up` at layer `provider` naming the
  plugin, the process and the field.
- flox, nix and devenv are unaffected — the flox golden stays
  byte-identical.

## Open Questions

- **Which YAML crate.** `serde_yaml` is archived upstream, its forks vary
  in provenance, and `yaml-rust2` is a parser rather than a serde
  integration. This is a supply-chain decision in a project that has
  twice recorded objections to dependency tails (141 crates for nono's
  trust module, 116 for `nono-proxy`), so it gets measured — crate count
  and maintenance status — rather than picked by familiarity.
- **What happens when `.devbox/virtenv/` does not exist.** It is written
  by `devbox install`, which devcroft's lockfile precondition already
  effectively requires. Whether its absence means "no services" or "not
  installed" needs measuring before one is assumed.
- **Whether devcroft's own capture creates or refreshes those files**,
  since `shellenv --pure` runs during resolution. If it does, the
  working-tree rule devenv's provider follows applies here too and the
  files must be read at a defined point rather than whenever.
- **Whether a plugin config can reference another plugin's process**
  through `depends_on`. None of the four does. If one can, the ordering
  question devenv raised returns in a different form.
