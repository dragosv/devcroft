# Sandbox Environment

## ADDED Requirements

### Requirement: A variable inherited from the invoking shell SHALL NOT reach the sandbox

devcroft SHALL remove from the keeper's environment every variable that arrived
unchanged from its own ambient environment, keeping what the provider's
activation contributed or modified. A sandbox's environment is a property of the
project, not of whoever happened to run `up`.

Subtraction rather than an allowlist: the provider's output cannot be enumerated
ahead of time, and a denylist of known-sensitive names is a guess about what
secrets are called.

#### Scenario: A shell variable does not cross

- **GIVEN** a variable exported in the shell that runs `up`, which no provider
  sets
- **WHEN** a session runs inside the resulting sandbox
- **THEN** that variable SHALL NOT be present

#### Scenario: The provider's own environment survives intact

- **GIVEN** a provider whose activation sets or modifies variables
- **WHEN** a session runs inside the sandbox
- **THEN** every variable the activation contributed SHALL be present with the
  value the activation gave it

#### Scenario: A variable the shell and the provider agree on is kept

- **GIVEN** a variable present in both the ambient environment and the
  provider's activation output with the same value
- **WHEN** that variable is one a POSIX session cannot function without
- **THEN** it SHALL be kept, and the set of such variables SHALL be enumerated
  in the implementation rather than decided per call site

### Requirement: A project SHALL be able to declare an exception

A variable the sandbox genuinely needs from the host SHALL be declarable in the
manifest, and SHALL be visible there rather than inferred. This is also the
supported way to give a sandbox a credential without a brokered route.

#### Scenario: A declared variable is forwarded

- **GIVEN** a manifest declaring a variable in `[env] forward`
- **WHEN** a session runs inside the sandbox
- **THEN** that variable SHALL be present with the host's value

#### Scenario: A declared variable that is not set on the host

- **GIVEN** a manifest declaring a variable the host does not export
- **WHEN** the user runs `up`
- **THEN** devcroft SHALL warn, naming the variable
- **AND** SHALL NOT fail — a forwarded variable is an optional convenience, not
  a precondition, and failing would make an unrelated project unbuildable on a
  machine that simply does not set it

### Requirement: A removed variable SHALL be diagnosable

The failure this change introduces is a tool that worked yesterday no longer
finding a variable. That failure surfaces inside the sandbox, far from its
cause, so devcroft SHALL make the cause discoverable.

#### Scenario: The user can see what was removed

- **WHEN** the user asks devcroft what a sandbox's environment contains
- **THEN** the removed variables SHALL be reportable, with the manifest key
  that would restore any of them

### Requirement: `HOME` SHALL point somewhere the sandbox can write

`HOME` currently names the host's home directory, which the baseline denies. Any
tool that writes to it fails, and an agent's own installer cannot run at all.

`HOME` SHALL be a per-sandbox directory inside the project's artifact
directory — writable, already ignored by git, already removed by `rm`.

#### Scenario: A tool writing to HOME succeeds

- **GIVEN** a sandbox
- **WHEN** a session writes a file under `$HOME`
- **THEN** the write SHALL succeed
- **AND** the file SHALL NOT appear in the host user's home directory

#### Scenario: HOME survives a restart and not a removal

- **GIVEN** a sandbox whose session wrote under `$HOME`
- **WHEN** the sandbox is stopped and brought up again
- **THEN** the contents SHALL still be there
- **AND** after `rm`, they SHALL be gone

### Requirement: `ssh.forward_agent` SHALL do what it says or not exist

`ssh.forward_agent` SHALL either control whether the SSH agent is reachable
inside the sandbox, or SHALL be removed from the manifest schema. It SHALL NOT
remain a key that parses, validates, and changes nothing.

The key is parsed and validated, `add-mvp-core`'s `ssh` spec requires that
without it "no agent socket exists inside the sandbox", and nothing implements
it. Once the environment is closed, `SSH_AUTH_SOCK` stops arriving by accident,
and the key becomes implementable in one place.

#### Scenario: Agent forwarding off

- **GIVEN** a manifest without `ssh.forward_agent = true`
- **WHEN** a session runs inside the sandbox
- **THEN** `SSH_AUTH_SOCK` SHALL NOT be present

#### Scenario: Agent forwarding on

- **GIVEN** a manifest with `ssh.forward_agent = true`
- **WHEN** a session runs inside the sandbox
- **THEN** `SSH_AUTH_SOCK` SHALL be present and the agent SHALL be usable
- **OR** the key SHALL be removed from the schema entirely, with the spec
  scenario it belongs to removed alongside it
