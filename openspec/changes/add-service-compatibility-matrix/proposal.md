# Change: add-service-compatibility-matrix

## Why

**Every service failure so far was found by hand, one at a time, and each
one took an hour.** A devenv postgres, a devbox mariadb, a redis — three
attempts, three different causes: a missing temp directory in the
baseline, an activation hook running after the services that depend on
it, and a `shmget` refusal that took four wrong answers to explain.
Two of the three were
devcroft's own defects, and neither would have been found without
someone deciding to try that particular database that particular
afternoon.

devenv ships **42 service modules** — `postgres`, `mysql`, `redis`,
`mongodb`, `elasticsearch`, `kafka`, `rabbitmq`, `minio`, `vault`,
`clickhouse`, `nginx`, `caddy` and thirty more. devcroft claims to
supervise the services a provider declares. It has been tried against
two of the forty-two.

So the claim is untested at a scale that matters, the failures are
platform-dependent, and the only way anyone learns which services work is
by hitting one that does not. That is a bad way to find out, and a worse
way to publish a capability.

## What Changes

- **A script enumerates devenv's service catalog and runs each one**
  through a real `devcroft up`, recording whether it reaches ready, and
  where it fails if it does not.
- **The result is a published matrix** — service, outcome, and the reason
  — written to `docs/`, dated, and stamped with the platform and versions
  it was measured on. A matrix without those is a claim about nothing in
  particular.
- **Not a `devcroft` subcommand.** The MVP command surface is closed
  (CLAUDE.md), and this is a maintenance measurement rather than a user
  operation. It goes under `scripts/`, beside the licence generator.
- **It runs a subset on request**, because forty-two closures is not
  something anyone waits for casually.

## Capabilities

### New Capabilities

- `service-compatibility-matrix`: what the matrix records, what
  distinctions it must preserve, and what it may not claim.

### Modified Capabilities

None. This measures existing behaviour; it does not change it.

## Impact

- **Affected code**: a new script under `scripts/`, and a generated
  document under `docs/`. No change to the crate.
- **It is slow and it is not CI.** Forty-two service closures is a large
  amount of building and downloading, and several services need real
  daemons with real data directories. This is a periodic measurement a
  maintainer runs deliberately, not something a pull request waits for.
- **It will produce failures that are not devcroft's**, and the matrix
  has to say which is which. A service that cannot build on this host, a
  service that needs credentials nobody supplied, and a service devcroft
  breaks are three different rows.
- **Expected to change what is published.** `docs/known-gaps.md` and the
  README currently describe services in general terms because nobody had
  the data. The matrix is the data.

## Success Criteria

- Running the script with no arguments produces a matrix covering every
  devenv service module discoverable at that devenv version.
- Every non-working row carries the reason, not just a status — the
  reasons are the point, and today's three failures were each more
  informative than the fact of failing.
- A host that cannot build a service's closure is recorded as **not
  measured**, never as a failure. This is the repo's standing rule about
  skips, applied to a document rather than a test run.
- Re-running it on the same host and versions produces the same matrix.
- The matrix names the platform, the devenv version and the devcroft
  commit it was produced from.

## Open Questions

- **Whether devcroft can report "ready" at all.** Measured while writing
  this: `ServiceHealth` has six states and none of them is ready —
  `ServiceState::from_json` reads `status`, `is_running` and `exit_code`
  and ignores readiness, even though devcroft now emits readiness probes
  and gates dependencies on health. Whether process-compose reports it in
  the same payload decides whether the matrix has a `ready` column or
  publishes `started` for everything.
- **How much of the table can say more than "it stayed up".** 25 of the
  42 modules declare no readiness probe (measured: 17 do). For those,
  *started* is the strongest honest statement, and the matrix will be
  mostly that. Whether a mostly-`started` table is worth publishing is a
  fair question — the answer this change bets on is that the failures are
  the point and the successes are the background.
- **How much configuration counts as fair.** `postgres` needs nothing;
  `vault` and `keycloak` need credentials. Testing each service with its
  defaults measures something real; tuning each until it passes measures
  the tuner. Design D5 chooses defaults, and makes "needs configuration"
  an outcome rather than a failure.
- **Whether devbox belongs in the same matrix.** Decided in design D8:
  no, and for a measured reason rather than a preference — neither flox
  nor devbox has an enumerable catalog to walk, and enumerating devbox's
  runs project code.
