# session-backend

> **Capability note, from integration.** There is no `session-backend`
> capability in the repo today, and no requirement by these names. The
> hardened tier's requirements live in the capabilities `add-hardened-tier`
> actually used: `config` (its "Isolation tier selection" requirement),
> plus `lifecycle`, `policy`, `cli` and `ssh`. So the REMOVED and MODIFIED
> requirements below do not yet name anything that exists, and a `REMOVED`
> block whose requirement cannot be found is a rename in disguise. Worth
> settling before implementation: either this change targets `config`
> and its siblings, or `session-backend` is introduced deliberately and the
> tier requirements move into it.

## REMOVED Requirements

### Requirement: A hardened isolation tier is selectable

**Reason:** The tier cannot compose with the sandboxing core — Landlock cannot
mediate `mount()`, which the runtime requires — and under rootless operation it
shares the host's network namespace, so it cannot support concurrent
environments. See `design.md`, G1 and G2.

**Migration:** Manifests selecting the hardened tier fail with a message naming
the removed tier and pointing to the process tier and to the VM path for
stronger isolation. Users needing a boundary above the process tier run devcroft
inside a VM, as the macOS path already does.

## MODIFIED Requirements

### Requirement: Isolation level is declared in the manifest

The manifest SHALL declare its isolation requirement, and `up` SHALL reject a
value the tool no longer provides with a message naming the removed value and
the supported alternative.

#### Scenario: Manifest selects a removed tier

- **WHEN** a manifest requests an isolation tier that no longer exists
- **THEN** `up` fails at the configuration layer
- **AND** the message names the removed tier, the supported tier, and the VM
  path for stronger isolation
- **AND** it does not silently fall back to a weaker tier

#### Scenario: Manifest omits the isolation level

- **WHEN** no isolation level is declared
- **THEN** the supported tier is used
- **AND** no deprecation output is produced

### Requirement: The session backend abstraction is retained

The session backend trait SHALL remain, with a single implementation.

#### Scenario: A future backend is added

- **WHEN** a new backend is introduced
- **THEN** it implements the existing trait
- **AND** no consumer of the trait requires restructuring to accommodate it

### Requirement: The isolation ceiling is stated accurately

Documentation and diagnostics SHALL state that the isolation ceiling is the
process tier, and SHALL name running inside a VM as the supported path to a
stronger boundary.

#### Scenario: Operator asks what isolation is available

- **WHEN** the operator inspects available isolation
- **THEN** one tier is reported
- **AND** the VM path is named as the answer for stronger isolation, rather than
  the limitation being presented as temporary
