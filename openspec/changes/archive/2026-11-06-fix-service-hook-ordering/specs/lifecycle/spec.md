# lifecycle Delta Specification (fix-service-hook-ordering)

## ADDED Requirements

### Requirement: A hook's failure fails `up`, wherever it runs
A failing provider activation script, or a failing `hooks.post_create` /
`hooks.post_start`, SHALL fail `up` at layer `keeper` with exit code 5,
and SHALL name the hook — regardless of which process runs it.

Stated because reordering it is the obvious way to satisfy the services
requirement, and the obvious reordering loses this: a script run by the
keeper at its own startup has no path back to `up`'s exit code unless one
is built. The guarantee is older than the reordering and is not
negotiable to achieve it.

`--skip-hooks` SHALL continue to suppress every hook entirely — the
provider's script and the manifest's alike — rather than running one and
ignoring its result, keeping its promise that nothing project-supplied
runs. It already suppresses declared services for the same reason, so a
`--skip-hooks` run has neither hooks nor services and the ordering
requirement is vacuous there rather than violated.

#### Scenario: A denied hook still fails `up`
- **WHEN** a provider's activation script invokes something the compiled
  policy denies
- **THEN** `up` fails at layer `keeper` with exit code 5, naming the
  hook, and the sandbox does not come up reporting success

#### Scenario: A failing manifest hook fails `up` the same way
- **WHEN** `hooks.post_create` exits non-zero
- **THEN** `up` fails at layer `keeper` with exit code 5, naming the
  hook, and the sandbox does not come up reporting success

#### Scenario: The escape hatch still escapes
- **WHEN** `up` runs with `--skip-hooks`
- **THEN** neither the activation script nor the manifest's hooks run,
  and their absence does not fail `up`
