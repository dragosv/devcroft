## ADDED Requirements

### Requirement: The trust command
The host CLI SHALL provide `devcroft trust [--sandbox <name>]` and `devcroft
trust --revoke [--sandbox <name>]`, listed in help and in `USAGE`. `trust`
SHALL print the checkout's root, the manifest's SHA-256 and the grants that
would become effective, and, when an earlier record exists, what changed since
it. On a TTY it SHALL ask for confirmation. When stdout is not a TTY it SHALL
require `--yes --sha256 <hex>` and SHALL fail at layer `config` when the hash
does not match the bytes on disk. Run inside a sandbox, it SHALL fail at layer
`config` as a host command.

#### Scenario: Approving interactively
- **WHEN** the user runs `devcroft trust` in a terminal and confirms
- **THEN** the record is `approved` for the printed hash, and the next `up` applies the listed grants

#### Scenario: Non-interactive without a hash
- **WHEN** `devcroft trust --yes` runs with stdout not a TTY and no `--sha256`
- **THEN** it fails at layer `config` naming `--sha256`, and nothing is approved

#### Scenario: Hash that does not match
- **WHEN** `devcroft trust --yes --sha256 <h>` runs and the manifest hashes to something else
- **THEN** it fails at layer `config` printing both hashes, and nothing is approved

#### Scenario: Revoking
- **WHEN** the user runs `devcroft trust --revoke` and then `up --recreate`
- **THEN** the sandbox starts untrusted

### Requirement: Trust state is visible
`status` SHALL show whether the running generation started trusted or
untrusted, the manifest hash it recorded, whether the file on disk still
matches, the keys not applied, and whether the manifest is masked or absent
from listings in the sandbox. `init` SHALL record an approval for exactly the
bytes it writes and say so.

#### Scenario: Status of an untrusted sandbox
- **WHEN** `status` runs for a sandbox started untrusted
- **THEN** it prints `untrusted` and each key not applied

#### Scenario: Manifest written by init
- **WHEN** `devcroft init` writes a manifest and `up` runs without editing it
- **THEN** the checkout is trusted, and `init`'s output said it approved that hash
