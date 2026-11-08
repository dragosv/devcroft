## Context

Measured, devbox 0.17.5 and process-compose 1.120.0, aarch64-darwin.

**Where the declarations live.** `devbox install` — zero hook
executions — writes `.devbox/virtenv/<plugin>/process-compose.yaml` for
each package whose plugin ships services. Reading them executes nothing.
`devbox services ls` executes `shell.init_hook` and is not used.

**What they contain.** Surveyed across the four plugins that ship a
config out of `postgresql`, `redis`, `nginx`, `mysql`:

| field | appears in |
|---|---|
| `command` | all four |
| `availability.restart` (`always`, `on_failure`) | all four |
| `availability.max_restarts` | redis, nginx |
| `is_daemon` | postgresql, mysql |
| `shutdown.command` | postgresql, mysql |
| `readiness_probe.exec.command` | postgresql |

Every one maps onto `ServiceDecl` as it stands after
`add-service-readiness`. `nginx` declares three processes and `mysql`
two, so a plugin contributes one *or more* services.

**What is not there.** No `depends_on`, no `http_get`, no field
`ServiceDecl` lacks — across four plugins, which is not the registry.

## Measured (group 0)

- **devcroft's own capture does not disturb the files.** `shellenv
  --pure` leaves `.devbox/virtenv/` byte- and mtime-identical, and runs
  the hook zero times. So they can be read at any point in resolution.
- **`.devbox/virtenv/` is stale cache, not a manifest — and this changes
  the design.** Removing `postgresql` from `devbox.json` and re-running
  `devbox install` leaves `.devbox/virtenv/postgresql/process-compose.yaml`
  in place. Enumerating the directory naively would start a service for
  a package the project no longer declares. Decision 5.
- Not every directory under `virtenv/` is a package: `runx` is devbox's
  own. It ships no config, so it would not have been noticed without
  looking.

## Goals / Non-Goals

**Goals:**

- Read plugin declarations as data, through a route that runs no project
  code.
- Translate into `ServiceDecl`, so devbox services get the same
  guarantees flox's and devenv's have.
- Refuse by name what cannot be carried.

**Non-Goals:**

- Re-arguing the contract question. It was decided; `docs/decisions.md`
  keeps the argument.
- Reading a user-authored root `process-compose.yaml`. That is the
  supervisor's own config rather than devbox's, it is not
  devbox-specific, and consuming it is the coupling
  `decouple-service-supervisor` removed.

## Decisions

### Decision 1: translate the declarations, do not forward the files

process-compose accepts repeated `-f`, and merges them — measured:
`--dry-run -f a.yaml -f b.yaml` reports "Validated 2 configured processes
from 3 files". So devcroft *could* hand the plugin files straight to its
supervisor alongside its own generated config, with no YAML parser and no
translation loss at all.

Rejected, and this is the load-bearing decision of the change.

Forwarding means devcroft runs services it never read: it cannot list
them in its own vocabulary, cannot refuse a field it does not support,
cannot apply the fidelity rule `fix-lossy-service-translation` owns, and
cannot answer "what is this sandbox running" except by asking the
supervisor. It is also precisely what `add-devenv-services` decision 4
refused for devenv's `process-compose` passthrough — project-controlled
supervisor configuration reaching the supervisor unread. Doing for
devbox what was refused for devenv would leave the codebase holding two
opposite answers to one question.

The cost of translating is one dependency. The cost of forwarding is the
seam.

### Decision 2: a YAML parser is a real dependency and is chosen on measurement

devcroft's tree carries **zero** YAML crates across 335 dependencies; it
emits JSON specifically to avoid a serializer. So this adds a genuinely
new one.

The hand-rolled route that worked for devenv's `declare -x` dump does not
transfer: that grammar was one line per entry with two quoting forms.
`nginx`'s plugin config uses a block scalar (`command: |`) carrying a
multi-line shell body with `$(...)`, redirections and embedded quotes.
A subset parser that got that subtly wrong would produce a *working*
config running a slightly different command, which is the failure mode
devcroft refuses everywhere else.

Measured, net additions to devcroft's 288-crate tree: `serde_yaml` +2,
`serde_norway` +2, `yaml-rust2` +4. The serde route is both cheaper and
less code, because devcroft already carries serde and these share its
dependencies; `yaml-rust2` costs more precisely because it shares
nothing. **`serde_norway`** wins between the two at equal cost, being a
maintained fork where `serde_yaml` is archived upstream.

Worth stating plainly rather than leaving the framing above to imply
otherwise: **two crates on 288 is a different order from the 141 and 116
this project objected to before.** The objection recorded here is that
the dependency is *new*, not that it is large.

### Decision 3: the plugin name is part of the error, not part of the service name

A service keeps the process name the plugin gave it — `postgresql`,
`nginx-access`, `mariadb_logs` — because that is what a devbox user sees
in devbox's own tooling, and renaming would make the two disagree.

But a *refusal* names the plugin as well as the process, because "the
field `x` on process `nginx-access` cannot be carried" leaves the user
hunting for where that process came from. They did not write it.

### Decision 4: the origin of a service nobody wrote is documented, not inferred

A user who adds `postgresql` to `devbox.json` and then sees a
`postgresql` service in `status` did not declare it. devenv has the same
property through its integrations, and the answer there was to state it
in the spec, the sample and the docs rather than let a user discover it.

Same answer here, and it matters more: with devenv the contributing
declaration is at least reachable by evaluating the project's own
`devenv.nix`, where devbox's lives in a generated file under a cache
directory. The documentation is the only place a user can find out.

### Decision 5: the declared package set decides, not the directory listing

Measured (group 0): removing a package leaves its plugin directory and
its `process-compose.yaml` behind. `.devbox/virtenv/` records what was
*ever* installed, not what is declared now.

So the directory listing is not the source of truth. Each plugin config
is read only where its directory name matches a package `devbox.json`
declares — devcroft already parses that list for the lockfile
precondition, so the check costs nothing new.

A config whose package is no longer declared is **skipped, not refused**,
and the distinction matters given how much else in this area refuses. A
stale directory is not a declaration devcroft failed to understand; it is
cache for a package the project removed. Refusing would make `devcroft
up` fail until the user cleaned a directory devbox owns and devcroft does
not. Skipping is the correct reading of what the project declared —
stated here so it is a decision rather than an omission.

The failure this prevents is concrete: remove `postgresql`, and without
the check every subsequent `up` starts a postgres the project no longer
asks for.

## Risks / Trade-offs

- **Four plugins are not the registry.** A plugin using `depends_on`
  across plugins, or a probe form devcroft lacks, is refused rather than
  mistranslated → which is the designed behaviour, and the reason the
  refusal requirement is stated even though nothing currently trips it.
- **The generated files are a cache artifact devbox rewrites** → they are
  read at a defined point in resolution, and whether devcroft's own
  capture refreshes them is measured in group 0 rather than assumed.
- **A new dependency in a project that counts them** → stated in the
  proposal rather than buried, and chosen on measurement.

## Migration Plan

Additive for devbox projects: services that previously caused a refusal
now run. No other provider changes, which the flox golden asserts.

## Open Questions

- ~~Whether `.devbox/virtenv/` can be stale relative to `devbox.json`.~~
  Measured: it can, and does. Decision 5.
