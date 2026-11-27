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
