## ADDED Requirements

### Requirement: The rendered VM authority is the enforced authority

`policy --render` for a VM instance SHALL be computed from the same compiled
authority the controller hands to the host proxy, the helper and the guest. Every
rendered grant SHALL carry its real origin. A project request without a matching
user grant SHALL render as a request that was not granted, never as granted.

#### Scenario: A manifest asks for a host nobody granted

- **WHEN** a project's `network.allow` names a host and no user grant covers it
- **THEN** the render shows it as requested and not granted
- **AND** the host proxy refuses it

#### Scenario: Comparing render and enforcement

- **WHEN** a test compiles authority for an instance and inspects the proxy's
  allowlist and the launch record
- **THEN** both equal what `policy --render` printed

### Requirement: Project-controlled keys do not move host values into a VM

A VM instance SHALL NOT receive a host environment value because a
project-controlled key (`[env] forward`, `[env.vars]` referencing the host, or an
imported definition) asks for it. Such a value SHALL require a user-owned grant
bound to the project identity. `up` SHALL name every host-sourced value that
reaches the guest.

#### Scenario: A repository forwards a cloud secret

- **WHEN** an unreviewed project's manifest lists `AWS_SECRET_ACCESS_KEY` under
  `[env] forward` and no user grant names it
- **THEN** the variable is absent in the guest
- **AND** `up --vm` reports the refused name and the grant that would allow it

#### Scenario: A granted forward

- **WHEN** a user grant allows a forwarded name for that project
- **THEN** the guest receives it and `up` lists it among delivered values

### Requirement: Transfers out of the guest cannot plant host-executed files

A transfer of guest files into the host checkout SHALL refuse, or write elsewhere
for review, any path the host or devcroft would later read as configuration or
execute: `devcroft.toml`, files under `.devcroft/`, `.envrc`, VCS hooks and
configuration, editor task and launch files, build tool configuration that runs
code, and any file arriving with an executable mode it did not have on the host.
The refused paths SHALL be named.

#### Scenario: The guest rewrites devcroft.toml

- **WHEN** the guest modified `devcroft.toml` and the user transfers the workspace out
- **THEN** the host `devcroft.toml` is unchanged
- **AND** the command names the refused path

#### Scenario: The guest adds an executable

- **WHEN** a transfer would create a new executable file in the host checkout
- **THEN** it is written without the executable bit, or refused, and named

### Requirement: Task evidence reflects the reachable path

A task in a VM change SHALL be marked done only when the behavior it names is
reachable from the command a user runs, and its evidence SHALL cite that path.
A component that exists only as a tested library function SHALL be recorded as
such, with the task left open.

#### Scenario: Reviewing a done task

- **WHEN** a done task cites a function as evidence
- **THEN** the function is called on the corresponding CLI path, or the task is open

### Requirement: Evaluation evidence records every run of a row

An evaluation result SHALL list every run of each row it reports, including
failed and excluded runs with the reason, so that a passing row is not reported
from one run while earlier failures of the same configuration are omitted.

#### Scenario: A row passed after failing

- **WHEN** a configuration failed a case in earlier runs and passed in a later one
- **THEN** the result lists both, and its status reflects the failures until they
  are explained
