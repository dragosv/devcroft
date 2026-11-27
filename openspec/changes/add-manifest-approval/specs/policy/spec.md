## ADDED Requirements

### Requirement: The workspace grant is a base rule
The project root's read-write grant SHALL carry origin `workspace` and SHALL be
compiled whether the checkout is trusted or not. It SHALL NOT depend on the
parser's default for `filesystem.allow`. An explicit `"."` in the manifest SHALL
add no second rule.

#### Scenario: Untrusted checkout keeps its workspace
- **WHEN** an unapproved checkout is brought up and `policy --render` runs
- **THEN** the project root is read-write with origin `workspace`

### Requirement: Unapplied manifest keys are rendered as not applied
`policy --render` SHALL list each manifest key the effective manifest did not
apply, with its origin `manifest:<key>` and the reason `not applied:
untrusted`. The compiled profile SHALL contain no rule from such a key.

#### Scenario: Rendering an untrusted manifest
- **WHEN** an unapproved manifest allows a host and `policy --render` runs
- **THEN** the host appears as not applied, and `why` on that host reports it denied

### Requirement: The workload cannot reach the authoritative manifest
In every sandbox, trusted or not, processes SHALL be unable to open, write,
truncate, remove, rename or replace the project's `devcroft.toml`, even when the
workspace is read-write. This includes aliases that reach the same file:

- case variants on a case-insensitive volume;
- symlinks inside the workspace;
- hard links created inside the sandbox;
- prefixed forms of the path.

On Linux the view SHALL mask the file with an unreadable read-only mount. On
macOS the profile SHALL deny reading and writing it. A host where this
protection cannot be enforced SHALL refuse `up` at layer `backend`, never start
without it. `status` SHALL report whether the name is masked or absent from
listings. A program inside the sandbox that needs the configuration SHALL
receive the effective configuration, never the file.

#### Scenario: Read-write workspace
- **WHEN** a process with a read-write workspace tries to read, write, delete or rename over `devcroft.toml`
- **THEN** each attempt fails and the file is unchanged, while an ordinary control file in the same directory stays editable

#### Scenario: Alias by case
- **WHEN** on a case-insensitive macOS volume a process opens `DEVCROFT.TOML`
- **THEN** the open fails as it does for `devcroft.toml`

#### Scenario: Backend without a view
- **WHEN** the Linux host cannot construct a mount view for the sandbox
- **THEN** `up` fails at layer `backend` naming the manifest protection
