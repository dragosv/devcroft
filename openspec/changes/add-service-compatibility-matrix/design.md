# Design — add-service-compatibility-matrix

Every measurement below was taken on aarch64-darwin against devenv 2.2.2
(module rev `6e32383`) while writing this. They are cited because the last
four things this project assumed about services were each wrong, and each
was caught by running something rather than by reading something.

## Decision 1: the catalog comes from `devenv.lock`, not from a list

devenv's service modules are one `.nix` file per service under
`src/modules/services/`, and the project's own `devenv.lock` pins the rev
they come from:

```json
"devenv": { "locked": { "owner": "cachix", "repo": "devenv",
                        "dir": "src/modules", "rev": "6e32383…" } }
```

So the enumeration is: read the rev from the lock, `nix flake prefetch`
that rev, list `src/modules/services/*.nix`. Measured — **42 modules**,
the same 42 this session enumerated by hand from a different store path,
which is the point: the number came from the lock rather than from
whoever wrote the script.

**Two smaller measurements shape the implementation.** `nix flake
prefetch` ignores the input's `?dir=`, returning the whole repository, so
the path is `<store>/src/modules/services` and not `<store>/services` —
a script written from the lock's `dir` field alone looks right and finds
nothing. And `trafficserver` has both a `.nix` file and a directory
beside it; counting directory entries gives 43, counting `*.nix` gives
42.

**Rejected: `devenv eval services`.** The obvious enumerator — ask devenv
for its own service tree — **fails**, measured:

```
error: The option `services.trafficserver.runroot' was accessed but has
       no value defined. Try setting the option.
```

Forcing the whole tree forces every option in it, including ones with no
default, so one unconfigured service takes the enumeration down for all
forty-two. This is worth stating rather than quietly working around: the
provider cannot describe its own catalog, so the catalog has to be read
off the filesystem.

## Decision 2: a script under `scripts/`, not a devcroft subcommand

The MVP command surface is closed (CLAUDE.md), and every addition to it
has to be discoverable in `USAGE` and tested there. More decisively, this
is not a thing a user of a sandbox does — it is a thing a maintainer does
to find out what to publish. `scripts/gen-third-party-licenses.py` is the
existing precedent for exactly that shape, and this sits beside it.

Consequence accepted: no shell completion, no `--help` contract, no test
guaranteeing it still runs. It is maintainer tooling and is allowed to be
maintainer tooling.

## Decision 3: five outcomes, because pass/fail throws away the news

| outcome | means | who acts |
|---|---|---|
| `ready` | supervised, probe passed | nobody |
| `started` | supervised, **no probe declared** | nobody, but see D4 |
| `failed` | devcroft ran it and it did not work | devcroft, usually |
| `refused` | `up` failed at layer `provider` | devcroft, definitely |
| `needs-config` | it wants credentials/state the run did not give | the matrix reader |
| `not-measured` | the closure would not build on this host | nobody |

Six rows, not five, because `not-measured` is not an outcome — it is the
absence of one, and it earns its own name for the reason the repo's
standing rule about skips exists: *a skip that looks like a pass is worse
than a failure*. A matrix with `postgres: —` in the darwin column says
something true. A matrix with `postgres: ✗` when the package never built
says something false about devcroft.

`refused` is separated from `failed` because they have different fixes.
A refusal is devcroft declining to carry a declared field, by name, on
purpose — `add-devenv-services` refuses eight. A failure is devcroft
carrying everything and the service still not working, which is what
postgres does today.

**Measured prediction, and it should be checked rather than trusted:**
zero of the 42 modules use the `process-compose` passthrough, `watch`, or
`proxy` — the fields devcroft refuses. So `refused` is expected to be an
empty column. If the run produces refusals anyway, the survey was wrong
and the refusal list is more expensive than this change assumed.

## Decision 4: `started` is a real outcome, not a weaker `ready`

**25 of the 42 modules declare no readiness probe** (measured: 17 do —
`postgres`, `redis`, `kafka`, `keycloak`, `prometheus` and twelve
others). For those, devcroft observes that a process was spawned and
nothing more. Reporting them as working would be devcroft asserting a
condition it did not check, which is the same defect `add-service-readiness`
was written to remove from the dependency edge.

So `started` is published as its own outcome, and the matrix says what it
means: *the process stayed up; nobody asked it whether it worked.* The
majority of the table will be in this state, and pretending otherwise
would make the table more confident than the measurement.

**devcroft does not currently report ready, and this change has to deal
with that.** `ServiceHealth` has six states — `Running`, `Failed`,
`Exited`, `NotStarted`, `Pending`, `Skipped` — and none of them is
*ready*. `ServiceState::from_json` reads process-compose's `status`,
`is_running` and `exit_code`, and ignores readiness entirely, even though
`add-service-readiness` now emits `readiness_probe` into the config and
gates `depends_on` on `process_healthy`. So devcroft asks its supervisor
to wait for ready and then cannot tell anyone whether it happened.

That is a gap this matrix *exposes* rather than one it may paper over.
Group 0 measures whether process-compose reports readiness in the same
JSON devcroft already parses; if it does, surfacing it is a small
addition to `ServiceState` and the matrix reads it like any other status.
If it does not, the `ready` column cannot be produced honestly and the
matrix publishes `started` for all 42 — which is a worse table and a true
one.

**Rejected: writing a probe per service so every row can be `ready`.**
Forty-two hand-written health checks is a second project, and every one
of them is a claim about the service that devcroft's users do not make.
A service the *project* declared without a probe is a service devcroft
supervises without a probe, and the matrix should measure devcroft as it
is used.

## Decision 5: defaults only — the run configures nothing

Each case is `services.<name>.enable = true;` and nothing else. Where
that is not enough, the row is `needs-config` and says what was missing.

The alternative — tune each service until it comes up — measures the
person doing the tuning. A matrix produced that way says "all 42 work,
given enough work", which is true of anything and useful about nothing.
`needs-config` rows are a finding in their own right: they are where a
user hits friction devcroft could reduce.

## Decision 6: one project, one sandbox, one teardown, per service

Each case gets its own temporary project directory and its own short
sandbox name. Both halves were learned the hard way this session:

- **The name must be short.** The supervisor's socket lives at
  `<project>/.devcroft/<name>/services.sock` and the OS caps a unix
  socket path at 103 bytes; `devenv-services-sample` produced 112 and
  `up` refused. Forty-two generated project paths under a temp root make
  this a certainty rather than a risk, so the name is derived to a bound
  and the script checks the bound rather than hoping.
- **Teardown is verified by absence, not by exit status.** `down`
  returning 0 is not evidence the service stopped — `add-flox-services`
  found a child still holding a port after the supervisor died. A case
  that cannot be verified torn down stops the run, because the next
  case's result would otherwise be about an unknown starting state.

## Decision 7: the failure's reason is the deliverable

A row carries the error text, and for `failed` rows the tail of the
service's own log (`.devcroft/<name>/services.log`). This is the entire
value of the exercise. The three service failures found so far were each
diagnosed from one line:

- `ls: cannot access '/tmp/devenv-827aed2/postgres': Operation not
  permitted` → a missing baseline grant, now fixed.
- `web is waiting for postgres to be healthy` → an ordering defect.
- `shmget … EPERM` → still unexplained, and the reason
  `docs/known-gaps.md` says so instead of guessing.

A matrix of forty-two red cells with no text would have found none of
them.

## Decision 8: devenv only, and the matrix says why

flox and devbox declare services too, and a reader comparing providers
wants one table. They are out of scope here for a measured reason:
neither has an enumerable catalog. flox's services are whatever the
project's manifest declares — there is no upstream set to walk. devbox's
come from installed plugins, and enumerating them at all runs
`shell.init_hook` (`add-devbox-services`, measured). devenv is the only
provider that ships a fixed, enumerable, version-pinned catalog, which is
what makes a matrix possible rather than arbitrary.

If devbox services are ever measured this way, it is a different
mechanism and should be a different change rather than a flag on this
one.

## Decision 9: the output is a document, and it is dated and stamped

The run writes `docs/service-matrix.md`: platform, devenv rev,
process-compose version, devcroft commit, the date, then the table.
Regenerated, not hand-edited — the same discipline as
`THIRD-PARTY-LICENSES.md`, and for the same reason: a generated file
somebody edits by hand becomes a file nobody trusts.

**A partial run marks itself partial.** The subset mode exists so a
maintainer can re-check one service after a fix (D6's isolation makes
that sound), and a subset run must not overwrite a full matrix with
thirty-nine blank rows.

## Rejected outright

- **Running it in CI.** Forty-two closures, several of them databases
  with real data directories. This is minutes-to-hours of building on a
  cold store, and a pull request cannot wait for it. It is a periodic
  maintainer measurement; if it ever runs unattended it should be a
  scheduled job that opens an issue, not a gate.
- **Parallelising the cases.** D6's isolation is what makes the rows
  trustworthy, and services that bind fixed default ports would collide
  on macOS, where nothing separates two sandboxes' loopback
  (`add-macos-service-vm`, measured). A wrong matrix produced quickly is
  worse than a right one produced slowly.
- **Asserting the matrix in a test.** The matrix's rows are facts about
  the host and about upstream, not about devcroft's code. A test that
  fails when nixpkgs breaks `cassandra` on darwin is a test that trains
  people to ignore it.
