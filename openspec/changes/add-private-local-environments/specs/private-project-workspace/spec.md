## ADDED Requirements

### Requirement: Import creates an identified private source snapshot

Import SHALL report source root, included/excluded files and snapshot identity.
It SHALL preserve selected working-tree changes and distinguish them from HEAD.
The guest SHALL NOT receive a writable mount of the original checkout.

#### Scenario: Checkout contains dirty tracked and untracked files

- **WHEN** the default ordinary-Git import is requested
- **THEN** tracked working-tree changes are included
- **AND** untracked files stay out and are named with the way to select them
- **AND** ignored exclusions and non-preservation of staging state are reported

#### Scenario: The repository does not ignore a local secrets file

- **WHEN** a user's `.env` is untracked and no project rule ignores it
- **THEN** default import leaves it out
- **AND** selecting untracked files still applies the user's global excludes and
  the sensitive-path list

#### Scenario: History holds other refs and a token in a remote URL

- **WHEN** the default import rebuilds guest metadata
- **THEN** only history reachable from HEAD is present
- **AND** no stash, reflog or URL userinfo reaches the guest

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

Export SHALL create a bounded staged bundle tied to its base, carrying guest
commits as a validated series with messages and authorship. It SHALL validate
paths and host filesystem collisions, exclude Git administrative metadata, and
SHALL NOT automatically apply changes, run host project code or invoke host Git on
guest metadata. Each export SHALL record what it covered and the next SHALL
default to that point.

#### Scenario: The guest committed several changes

- **WHEN** export runs
- **THEN** the staged bundle holds a series with each message and author
- **AND** the host never reads guest Git metadata directly

#### Scenario: A second export after the first was applied

- **WHEN** the user exports again from the same instance
- **THEN** the bundle covers only work since the previous export

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

### Requirement: Refresh brings host changes into a running workspace

An explicit refresh SHALL import a new snapshot and merge it in the guest, leaving
conflicts there, without changing environment adaptations. It SHALL NOT discard
guest work, SHALL NOT change the host checkout, and SHALL drop direct credentials
until they are restated.

#### Scenario: The host branch advanced while the guest has local work

- **WHEN** the user refreshes
- **THEN** conflicts remain in the guest for the user to resolve
- **AND** no guest work is discarded and the host checkout is unchanged

### Requirement: Destructive operations name unexported work

`rm` and `up --recreate` SHALL state that the guest workspace may be the only copy
of unexported work, with counts labelled guest-reported or unknown. Non-interactive
use SHALL require `--yes`.

#### Scenario: The instance is stopped

- **WHEN** `rm` runs and the guest cannot report
- **THEN** unexported work is reported as unknown rather than none

### Requirement: The definition that prepared an instance is recorded

The controller SHALL record the digest of the definition that prepared an instance.
Editing a definition inside the guest SHALL NOT change that instance's environment
or any evidence.

#### Scenario: A definition is edited through a guest session

- **WHEN** the user edits the workspace copy of a definition
- **THEN** the instance's recorded environment is unchanged
- **AND** the edit takes effect only through export or save and a new preparation
