## ADDED Requirements

### Requirement: Project execution starts only after the VM boundary exists

The VM path SHALL isolate provider/tooling evaluation, builds, extraction of
untrusted images, hooks and setup before they execute. It SHALL NOT require host
project toolchains or use the operator's package daemon for project execution.

#### Scenario: Provider definition runs project-controlled build code

- **WHEN** resolution evaluates or executes that definition
- **THEN** it runs in a confined preparation VM for the selected Linux target
- **AND** host checkout, host daemon and sibling writable state are unavailable

### Requirement: Bootstrap identity derives from user-trusted distribution

The controller SHALL verify helper/kernel/init/runtime against a user-trusted
manifest and SHALL NOT accept repository-declared bootstrap paths as authority.
Host confinement SHALL precede the first guest instruction on every VMM path.

#### Scenario: Project provides a kernel and a matching self-declared hash

- **WHEN** the project requests that kernel as trusted bootstrap
- **THEN** the request is refused unless separately authorized by distribution
  policy outside project control

### Requirement: Preparation cannot mutate shared authority

Preparation SHALL have private writable state and no writable shared store.
Publication SHALL validate and hash immutable output, bind provenance, and
commit atomically under controller-owned identities. Runtime consumers SHALL
hold read-only access enforced outside the guest and a retention lease.

#### Scenario: Guest tries to publish over an existing cache entry

- **WHEN** it supplies another artifact's identity or host path
- **THEN** the controller refuses substitution and preserves the existing object

#### Scenario: Publisher dies before commit

- **WHEN** preparation or publication is interrupted
- **THEN** no partially written artifact becomes a reusable cache entry

### Requirement: Effective authority requires user scope

Project and imported definition requests SHALL be constrained by user-owned
grants and runtime hard limits. The rendered plan SHALL include phase, origin,
host descriptors/devices, guest grants, channels and credential reference names.
Unsupported required requests SHALL be refused by name.

#### Scenario: Manifest requests home and global credentials

- **WHEN** those requests have no matching user-owned authorization
- **THEN** launch refuses the requests or reports them as unresolved before setup
- **AND** the manifest does not gain authority by containing those declarations

### Requirement: Every host channel is bounded and instance-scoped

The initial runtime SHALL expose only its declared control, SSH and proxy
channels, with bounded framing, authenticated instance/phase association and
timeouts. NIC, TSI, arbitrary host-connect and implicit SSH reverse/agent/X11
forwarding SHALL be unavailable. Host proxy policy SHALL mediate actual network
destinations, including host-local and private addresses.

#### Scenario: Client ignores proxy variables or forges another instance ID

- **WHEN** a guest attempts direct egress or another instance's channel
- **THEN** no alternate network route or authority is obtained
- **AND** correctly scoped allowed traffic still succeeds

#### Scenario: Allowed hostname resolves to an ungranted host-local address

- **WHEN** the proxy resolves or reconnects to that address
- **THEN** the connection is refused instead of reaching a host service

### Requirement: Setup authority is revoked before runtime admission

Setup completion SHALL terminate setup descendants, revoke its credentials and
close its active proxy streams before runtime starts. Runtime SHALL receive its
own policy, not the union of setup and runtime permissions.

#### Scenario: Hook leaves a background process with an open connection

- **WHEN** setup completes
- **THEN** the descendant and connection lose setup authority
- **AND** unsuccessful cleanup prevents runtime admission

### Requirement: Credentials are not reusable base-image contents

Devcroft SHALL deliver authorized direct credentials only after boot and SHALL
NOT persist their values in launch configuration, logs or shared artifacts.
Shared-base preparation in this phase SHALL exclude personal credentials and
ordinary local session disks. Status SHALL disclose direct guest exposure.

#### Scenario: User requests promotion of a disk used with credentials

- **WHEN** that disk is proposed as a shared fleet base
- **THEN** promotion is refused and clean reconstruction is required
- **AND** devcroft does not claim it can scrub all copies made by guest code
