## ADDED Requirements

### Requirement: Import creates an identified private source snapshot

Import SHALL report source root, included/excluded files and snapshot identity.
It SHALL preserve selected working-tree changes and distinguish them from HEAD.
The guest SHALL NOT receive a writable mount of the original checkout.

#### Scenario: Checkout contains dirty tracked and untracked files

- **WHEN** the default ordinary-Git import is requested
- **THEN** tracked working-tree changes and non-ignored untracked files are included
- **AND** ignored exclusions and non-preservation of staging state are reported

#### Scenario: Source changes while being captured

- **WHEN** snapshot consistency cannot be established
- **THEN** import fails or retries a new snapshot without claiming a mixed one is stable

### Requirement: Import does not activate host Git authority

Import SHALL avoid host execution of repository-controlled hooks, config, filters
or helpers. It SHALL rebuild controlled guest Git metadata and reject or explicitly
avoid unsupported worktree/submodule/alternate semantics. File transfer SHALL
enforce size/type/path bounds without following outside-root links.

#### Scenario: Git configuration names a command and an external alternate

- **WHEN** the project is imported
- **THEN** no command executes on the host and no outside path is read implicitly
- **AND** unsupported history semantics produce a named refusal

#### Scenario: Selected source contains a symlink to the host home

- **WHEN** transfer encounters the link
- **THEN** it does not copy the target or grant the guest access to that host path

### Requirement: Export is staged and cannot overwrite host administrative state

Export SHALL create a bounded staged patch/file bundle tied to the import base.
It SHALL validate paths and host filesystem collisions, exclude Git administrative
metadata, and SHALL NOT automatically apply changes or run host project code.

#### Scenario: Guest exports traversal paths or a replacement Git config

- **WHEN** export validates the result
- **THEN** the unsafe entries are refused and the original checkout remains unchanged

#### Scenario: Host changed after import

- **WHEN** an export is staged against the earlier base
- **THEN** its base identity and conflict risk are visible
- **AND** no newer host content is overwritten

### Requirement: Local work survives stop independently of the original checkout

Guest workspace and declared private service data SHALL persist across stop and
restart. Removal SHALL NOT delete the host checkout or external definition.

#### Scenario: Local edits exist only inside the guest

- **WHEN** the instance stops and restarts
- **THEN** the edits remain available at the guest workspace and export endpoint
