# credentials Delta Specification (add-agent-workload)

## Purpose

How a secret reaches a process running inside the sandbox boundary, for
the cases where a tool must authenticate to an external service from
inside. Covers both auth shapes in use — an environment-variable key and
a file-based subscription credential — and bounds what each exposes.

## ADDED Requirements

### Requirement: Credentials are opt-in and never implicit
The system SHALL expose a credential to a sandbox only when the manifest
explicitly asks for that specific credential. No other manifest key, and
no tooling or provider declaration, SHALL cause a credential to become
readable inside the boundary as a side effect. The set of sensitive paths
denied by baseline SHALL remain denied regardless of any credential
request.

#### Scenario: No credential without an explicit request
- **WHEN** a manifest declares a tooling layer but requests no credential
- **THEN** no credential material is readable inside the sandbox, and the
  compiled policy contains no credential grant

#### Scenario: Baseline denials are not overridable by a credential request
- **WHEN** a credential request names a path inside a baseline-denied
  location
- **THEN** the request is rejected, and the baseline denial stands

### Requirement: Key-shaped credentials are delivered as environment variables
The system SHALL deliver an API-key-shaped credential to the sandbox as
an environment variable through the backend's credential mechanism,
without granting any filesystem path for it. The secret's value SHALL NOT
be written into the compiled policy, into `meta.json`, or into any file
under the state directory.

#### Scenario: Key credential grants no filesystem access
- **WHEN** an API-key credential is requested
- **THEN** the process inside the sandbox sees it in its environment, and
  the compiled policy contains no additional filesystem grant

#### Scenario: Secret value is not persisted by devcroft
- **WHEN** a sandbox with a key credential is up
- **THEN** the secret's value does not appear in the compiled policy, the
  recorded metadata, or the logs

### Requirement: File-shaped credentials grant a single file, read-only
The system SHALL, for a credential that exists only as a file, grant read
access to that single file and nothing more. Granting the containing
directory SHALL NOT be an accepted implementation, and the grant SHALL be
read-only. A credential request naming a directory SHALL be rejected.

#### Scenario: Only the named file becomes readable
- **WHEN** a file-shaped credential names a single credentials file
- **THEN** exactly that file is readable inside the sandbox, and its
  sibling files in the same directory remain unreadable

#### Scenario: Directory request rejected
- **WHEN** a credential request names a directory rather than a file
- **THEN** validation fails, naming the requirement that credentials are
  granted per file

#### Scenario: Credential file is not writable
- **WHEN** a process inside the sandbox attempts to write the granted
  credential file
- **THEN** the write is denied

### Requirement: Exposure is disclosed exactly once, and is inspectable
The system SHALL print exactly one line at `up` naming each credential
exposed and the shape it was delivered in, and SHALL represent a
file-shaped credential in the rendered policy with its own origin,
distinguishable from manifest, provider, and tooling grants. Credential
exposure SHALL NOT be discoverable only by reading the manifest.

#### Scenario: Up discloses the exposure
- **WHEN** a sandbox with one file-shaped credential comes up
- **THEN** `up` prints one line naming the exposed file, neither silently
  nor repeatedly

#### Scenario: Credential grant is attributable in the rendered policy
- **WHEN** `policy --render` runs for a sandbox with a file credential
- **THEN** the grant appears with an origin identifying it as a
  credential, not as an ordinary manifest filesystem grant

### Requirement: The residual exposure is stated, not implied
The system's documentation SHALL state plainly that a credential exposed
to a sandbox is readable by every process in that sandbox, including
project code and any agent operating on it, and that the mitigation is
narrowness and visibility rather than isolation from the code under
edit.

#### Scenario: Documented limitation
- **WHEN** a user consults the documentation for credential support
- **THEN** it states that in-sandbox code can read an exposed credential,
  rather than implying the credential is isolated from it
