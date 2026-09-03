## Purpose

Give the row matrix a member that needs no Nix daemon, so the neutral test
surface can migrate without turning every daemon-less host's suite red —
without letting "cheap" become "fake".

## ADDED Requirements

### Requirement: The row supplies a working environment it owns

The row SHALL create a directory, place a real POSIX shell in it, and
declare that directory in its `read_only_grants`, so the shell resolves as
being inside something the sandbox is granted.

It SHALL NOT obtain that shell from the Nix store, and SHALL NOT obtain it
from the ambient host `PATH` — the two sources this row exists to be
independent of, and the one `test-runtime-fixture` already forbids.

#### Scenario: A sandbox comes up on the row

- **WHEN** a test brings a sandbox up on this row on a host with no Nix
  daemon
- **THEN** `up` succeeds
- **AND** the shell recorded in `meta.json` is inside the row's own
  directory, not the store and not a host path

#### Scenario: A session actually runs

- **WHEN** a test runs a command in that sandbox
- **THEN** the command executes and its output is returned
- **AND** a shell that starts but cannot execute does not count as the row
  working

### Requirement: The row's binaries run on the platform they are built for

The row SHALL verify at setup that its shell is executable on this host, and
report the row unavailable — with the reason — if it is not.

Stated as a requirement because of how this fails in practice rather than in
theory: a copied macOS platform binary does not error, it **hangs**, and a
hang in fixture setup is indistinguishable from a slow test. A row that
cannot be checked cheaply for liveness will eventually be debugged as a
mysterious CI timeout instead of as a missing prerequisite.

#### Scenario: The row's shell does not work on this host

- **WHEN** the row's shell cannot be executed
- **THEN** the row reports unavailable, naming what could not run
- **AND** setup does not block waiting for it

### Requirement: The row is not offered as closure-tier evidence

Documentation, `capabilities()`, and any report this row appears in SHALL
NOT present it as evidence that a real toolchain works inside the sandbox's
filesystem view.

The row has no dynamic loader on Linux and a deliberately minimal
environment on both platforms, so it exercises none of the `/lib` →
`ld-linux` → merged-`/usr` path that `fleet::mount::setup_merged_usr_compat`
exists to serve. A green board from this row alone would say something this
row cannot know.

#### Scenario: Reporting a run that used only this row

- **WHEN** a run covers this row and no real-provider row
- **THEN** what it demonstrates is devcroft's own orchestration, not that a
  provider's closure functions
- **AND** the real-provider rows remain required rather than optional

### Requirement: The row does not become the default

Selecting rows SHALL continue to default to a real environment. This row
SHALL be reachable only by naming it.

`test-runtime-fixture` decided this and the decision is restated here
because this is the change that creates the temptation: a fast row that
needs no daemon is exactly the one that would drift into being the default,
and the whole point of the default being real is that a developer running
`cargo test` gets the realistic suite without asking.

#### Scenario: No row named

- **WHEN** no row is selected explicitly
- **THEN** the default is unchanged and is a real-environment row
- **AND** this row is not substituted when that one is unavailable
