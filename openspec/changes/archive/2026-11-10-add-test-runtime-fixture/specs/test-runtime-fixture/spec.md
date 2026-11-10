## Purpose

Let one neutral-surface test run against any provider, so the 25 test files
that build a real environment only to get past `up` stop paying for a
provider they are not testing — without letting a cheaper row quietly become
the only one that ever runs.

## ADDED Requirements

### Requirement: A neutral-surface test is written once and runs against a selected row

A test that exercises devcroft's own behaviour rather than a provider's SHALL
be written against the fixture contract, not against a named provider, and
SHALL run against whichever row `DEVCROFT_TEST_PROVIDER` selects.

The neutral surface is defined by exclusion, and the boundary is declared
rather than discovered: lifecycle (`up`/`down`/`rm`/`status`), locks and
concurrency, keeper sessions, `exec`, signals, SSH channels, devcroft's own
`[hooks]`, policy compilation, rendering and `why`, mount view construction,
and the network namespace. Provider *contract* behaviour — environment
capture, lockfile preservation, activation-hook handling, staleness inputs —
is outside it and stays per-provider.

#### Scenario: A lifecycle test under the default row

- **WHEN** `cargo test` runs with `DEVCROFT_TEST_PROVIDER` unset
- **THEN** the neutral test runs against the Nix flake row
- **AND** the test source names no provider

#### Scenario: The same test under another row

- **WHEN** the same test runs with `DEVCROFT_TEST_PROVIDER=devbox`
- **THEN** it exercises the identical assertions against a devbox project
- **AND** no change to the test source was required

### Requirement: The default row is a real environment

With `DEVCROFT_TEST_PROVIDER` unset, the suite SHALL use a real Nix flake
row — a real closure, a shell resolved out of it, a real dynamic loader.

The cheap row must be asked for by name. A developer running `cargo test`
locally is entitled to assume they ran the realistic suite; a default that
silently optimised for speed would make that assumption false, which is the
same defect as a skip that reads like a pass.

#### Scenario: No selection made

- **WHEN** `DEVCROFT_TEST_PROVIDER` is unset
- **THEN** the Nix flake row is used
- **AND** no synthetic or host-derived environment is substituted

### Requirement: An unavailable row is reported, never silently replaced

Where a selected row cannot be set up on this host, the run SHALL report that
row as skipped, naming the row and the reason, and SHALL NOT fall back to a
different row.

This project's characteristic failure is a skip that reads like a pass, and a
fixture that quietly downgraded would industrialise it: every row would be
green and the expensive ones would never have run.

#### Scenario: The default row's toolchain is missing

- **WHEN** the Nix flake row is selected and its setup fails
- **THEN** the run fails, naming `DEVCROFT_TEST_PROVIDER=test` as the
  explicit alternative
- **AND** it does not substitute another row

#### Scenario: Running every row

- **WHEN** `DEVCROFT_TEST_PROVIDER=all` is set
- **THEN** each row's outcome is reported separately, distinguishing ran from
  skipped and naming the reason for each skip
- **AND** a run in which every row skipped is not reported as success

#### Scenario: A row that is available but broken

- **WHEN** a row's setup succeeds and one of its tests then fails
- **THEN** the run fails
- **AND** the failure is not reported as an unavailable row

### Requirement: Row capability differences are declared by the fixture, not branched on in tests

Where a neutral test needs a capability some rows lack, the fixture SHALL
declare that capability and the test SHALL consult the declaration.

A test SHALL NOT branch on a row's name. Name-branching reintroduces
per-provider conditionals through the back door, and the first such branch
makes every later one look normal.

#### Scenario: A services test on a row without services

- **WHEN** a neutral test requires supervised services and the selected row
  declares no services capability
- **THEN** the test skips, naming the missing capability
- **AND** it does not inspect the row's name to decide

### Requirement: The staleness input is the fixture's to name

Each row SHALL be able to mutate its own project into a state that changes
the environment fingerprint, so a shared staleness test can be written once.

Stated as a requirement because the shared test cannot know what to touch:
the fingerprint is computed from different files per provider — a flox
manifest and lock, a `flake.nix` and `flake.lock`, a `devbox.json` and its
lock.

#### Scenario: Shared staleness assertion

- **WHEN** a neutral test asks the fixture to drift and then re-runs `status`
- **THEN** the sandbox reports stale, on every row
- **AND** the test names no provider-specific file

### Requirement: A row's realism is not weakened to make it pass

No row SHALL satisfy the fixture contract by resolving its shell, its
`process-compose`, or its toolchain from the host.

The invariant `own-policy-baseline` established — that what a sandbox runs
comes from the environment, not from ambient host tooling — applies to the
rows too. A row that reached for `/bin/sh` would be green precisely where
devcroft's own regression once was, and the matrix would certify it.

#### Scenario: A row backed by host tooling

- **WHEN** a row's environment resolves its shell to a host path
- **THEN** that row does not satisfy this contract
- **AND** the suite does not report it as a passing row
