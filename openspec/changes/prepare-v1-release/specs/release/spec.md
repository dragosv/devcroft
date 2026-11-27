## ADDED Requirements

### Requirement: The stable surface is defined
`docs/stability.md` SHALL list what 1.0 keeps stable:

- commands, flags, exit codes and error layers;
- the `devcroft.toml` and user-configuration keys and their meaning;
- the behaviors it names.

It SHALL state that message wording and every Rust `pub` item are not
stable. After 1.0, a change that breaks a listed element SHALL need a major
version.

#### Scenario: A removed flag
- **WHEN** a change after 1.0 removes a flag listed in `docs/stability.md`
- **THEN** the release that ships it has a new major version

### Requirement: A private security channel exists before 1.0
`SECURITY.md` SHALL name a private channel for reporting vulnerabilities, what
a report should contain and the expected response window, and the README SHALL
point to it. No 1.0 release SHALL be cut while the README directs security
reports to a public issue.

#### Scenario: Reporter reads the README
- **WHEN** a reporter opens the README's security section in a 1.0 release
- **THEN** it names the private channel from `SECURITY.md`

### Requirement: The release gate proves the support matrix
A release workflow SHALL decide which platform, provider, backend and agent
combinations are stable from recorded results. A combination SHALL be stable
only when every mandatory probe for it ran and passed on its platform. A
skipped or unknown probe SHALL keep the combination out of the stable set, and
the release notes SHALL list that combination as unqualified.

#### Scenario: A probe skipped on macOS
- **WHEN** the broker probe for an agent did not run on `macos-arm64`
- **THEN** that combination is not in the stable set, and the release notes name it with the missing probe

### Requirement: The distributed artifact installs and upgrades
Before release, CI SHALL install devcroft from the package being published and
run a sandbox with it. It SHALL also start that build on the previous
release's sandboxes, `Meta` and configuration. Each existing sandbox SHALL
either run unchanged or be refused at layer `config` naming
`up --recreate`, never run with fields silently defaulted.

#### Scenario: Old state too old to read
- **WHEN** the new build finds a sandbox whose `Meta` is older than the oldest version it declares readable
- **THEN** `up` fails at layer `config` naming `up --recreate`, and the old state is unchanged

#### Scenario: Two worktrees on the published build
- **WHEN** the published build brings up two worktrees of one project in parallel, each with an agent
- **THEN** both complete an edit and a test, and neither sees the other's state
