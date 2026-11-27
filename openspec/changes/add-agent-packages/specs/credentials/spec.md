## ADDED Requirements

### Requirement: Selecting an agent recipe is an explicit credential request
Selecting a recipe and a mode in the user configuration or by a flag SHALL count
as the explicit request that credential delivery requires, for that recipe's
credential variables and that sandbox only. A project's manifest SHALL NOT be
able to make that request. Baseline denials SHALL stand.

#### Scenario: Sandbox without the recipe
- **WHEN** a brokered key is available for a recipe and a sandbox selects none
- **THEN** no credential or broker of that recipe exists there

### Requirement: A recipe's active hosts are the only user-configured egress
A user's own broker entry SHALL keep granting no egress. The active hosts of a
selected recipe SHALL be added to the sandbox's egress with origin
`agent:<recipe>`, and no other user configuration SHALL widen it.

#### Scenario: User broker grants no egress
- **WHEN** a user broker names a host that neither `network.allow` nor the selected recipe allows
- **THEN** `up` fails at layer `config`

#### Scenario: Recipe broker on an empty allowlist
- **WHEN** a project with `network.allow = []` selects a recipe under `api-key`
- **THEN** the recipe's active hosts are reachable and no other host is

### Requirement: Host-side helpers start from a cleared environment
The egress proxy and any relay devcroft starts host-side for a sandbox SHALL
start from a cleared environment plus an explicit list of the variables each
needs. They SHALL NOT inherit the invoking shell's environment. A broker's key
SHALL reach only the proxy that injects it.

#### Scenario: Unrelated host secret
- **WHEN** the user's shell exports an unrelated secret and `up` starts the egress proxy
- **THEN** the proxy's environment contains only its listed variables, and the secret is absent from it

### Requirement: A broker's upstream cannot be redirected outside test builds
Overriding a broker's upstream, or adding a trust anchor for it, SHALL exist
only under the non-default `test-support` feature, for tests that use a fake
LLM server. A build without that feature, the published crate and `cargo
install devcroft` included, SHALL contain no flag, environment variable,
configuration key or file that sends a broker's credential to any host other
than the recipe's or user broker's declared host, or that makes the proxy trust
another certificate for it.

#### Scenario: Release build given the test hook's inputs
- **WHEN** devcroft built without `test-support` brings up a brokered recipe while every variable and file the test hook reads names a fake upstream and a test CA
- **THEN** the broker connects only to the recipe's declared host with the normal trust store, and the fake upstream receives nothing

#### Scenario: Test build reaches the fake server
- **WHEN** devcroft built with `test-support` runs a harness against `fake-llm` through the broker
- **THEN** the fake server receives the synthetic key and never the placeholder
