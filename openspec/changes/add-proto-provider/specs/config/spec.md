## ADDED Requirements

### Requirement: proto provider value
The system SHALL accept `env.provider = "proto"` once the qualification in
this change admits a profile, and SHALL then no longer reject it as a version
manager. Until then, and if the qualification ends in a rejection, `proto`
SHALL keep failing at layer `config`, exit code 2, with a reason that names
the property that fails. `asdf` SHALL stay rejected as a version manager.

#### Scenario: Admitted profile
- **WHEN** the manifest declares `provider = "proto"` on a qualified platform
- **THEN** validation succeeds and `status` reports tier `artifact`

#### Scenario: Qualification ended in a rejection
- **WHEN** group 0 keeps the rejection and the manifest declares `provider = "proto"`
- **THEN** validation fails at layer `config` with exit code 2
- **AND** the message names the failing property, not "lacks a lockfile"

### Requirement: Optional proto selection table
The system SHALL accept an optional `[env.proto]` table holding only `config`,
a path to the `.prototools` file relative to the project root. Unknown keys,
a path that leaves the canonical project root, and `[env.proto]` under another
provider SHALL fail at layer `config`.

#### Scenario: Table under another provider
- **WHEN** `provider = "mise"` and `[env.proto]` is present
- **THEN** validation fails at layer `config` naming `[env.proto]`
