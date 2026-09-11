# lifecycle Delta Specification (fix-service-hook-ordering)

## ADDED Requirements

### Requirement: The activation script's failure fails `up`, wherever it runs
A failing provider activation script SHALL fail `up` at layer `keeper`
with exit code 5, and SHALL name the hook — regardless of which process
runs the script.

Stated because reordering it is the obvious way to satisfy the services
requirement, and the obvious reordering loses this: a script run by the
keeper at its own startup has no path back to `up`'s exit code unless one
is built. The guarantee is older than the reordering and is not
negotiable to achieve it.

`--skip-hooks` SHALL continue to suppress the script entirely rather than
running it and ignoring the result, keeping its promise that nothing
project-supplied runs.

#### Scenario: A denied hook still fails `up`
- **WHEN** a provider's activation script invokes something the compiled
  policy denies
- **THEN** `up` fails at layer `keeper` with exit code 5, naming the
  hook, and the sandbox does not come up reporting success

#### Scenario: The escape hatch still escapes
- **WHEN** `up` runs with `--skip-hooks`
- **THEN** the activation script does not run, and its absence does not
  fail `up`
