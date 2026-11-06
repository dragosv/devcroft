# services Delta Specification (fix-service-hook-ordering)

## ADDED Requirements

### Requirement: Services start after everything project-supplied that prepares them

Where a provider supplies an activation script, or the manifest declares
`hooks.post_create` / `hooks.post_start`, the system SHALL run all of
them to completion before starting any declared service, preserving their
existing order relative to each other: the provider's activation script
first, then the manifest's hooks.

**Both kinds, not just the provider's.** The case that found this was a
devbox plugin's `setup_db.sh` creating a database's data directory, which
is a provider activation script — but the same defect reaches the
manifest's own hooks, and that is where a user meets it. `hooks.post_create
= "initdb"` is the natural way to express one-time setup a service
requires, and it is the shape Dev Containers established. A requirement
that ordered only the provider's script would fix the reported case and
leave the expressible one broken.

**They are not ordered today — they race.** Measured with
`hooks.post_create` against a devbox `redis` project, four consecutive
runs: the hook completed 9–10 ms before the service started, every time.
`up` spawns the keeper (which starts services at its own startup) and
then runs the hooks, so the intended order is services-first; the hook
wins anyway because process-compose needs ~9 ms to boot, load its config
and launch a process. A trivial hook wins that race and a slow one loses
it, which is why the same code produced a working `redis` and a broken
`mariadb`.

This SHALL NOT be achieved by moving the services' lifetime out of the
keeper. A session tied to `up`'s connection is killed when `up` exits —
measured — so services started that way would be reaped seconds after the
sandbox came up.

#### Scenario: A service depending on hook-prepared state starts after it

- **WHEN** a provider's activation script prepares state a declared
  service requires, and the sandbox comes up
- **THEN** the script has completed before the service is started

#### Scenario: A manifest hook preparing a service's state starts before it

- **WHEN** the manifest declares `hooks.post_create` that prepares state
  a declared service requires, and the sandbox comes up for the first
  time
- **THEN** `post_create` has completed before the service is started

#### Scenario: The guarantee holds on every start, not only the first

- **WHEN** a sandbox with declared services is brought up again, so
  `post_start` runs and `post_create` does not
- **THEN** `post_start` has completed before the service is started

#### Scenario: The ordering is observed, not inferred from success

- **WHEN** the ordering is asserted
- **THEN** it SHALL be asserted by observing that the hook completed
  before the service started, and NOT by observing that the service came
  up — a service can come up from state a previous run left behind,
  which is how this defect stayed invisible, and a 9 ms margin means an
  "it works" assertion passes most of the time without the fix

#### Scenario: Teardown is unaffected

- **WHEN** the sandbox is torn down
- **THEN** no service process started by it remains alive, verified by
  observing process absence rather than by a stop command's exit status

#### Scenario: Providers whose services do not depend on the hook are unchanged

- **WHEN** neither a provider's activation script nor the manifest's
  hooks prepare anything a declared service requires
- **THEN** the services behave exactly as before this requirement existed
