## ADDED Requirements

### Requirement: Project execution starts only after the VM boundary exists

The VM path SHALL isolate provider/tooling evaluation, builds, extraction of
untrusted images, hooks and setup before they execute. It SHALL NOT require host
project toolchains or use the operator's package daemon for project execution.

#### Scenario: Provider definition runs project-controlled build code

- **WHEN** resolution evaluates or executes that definition
- **THEN** it runs in a confined preparation VM for the selected Linux target
- **AND** host checkout, host daemon and sibling writable state are unavailable

#### Scenario: A provider lock does not cover the Linux target

- **WHEN** the selected definition is locked only for another system
- **THEN** preparation refuses by name before evaluating project code
- **AND** it does not re-lock the definition silently

### Requirement: Bootstrap identity derives from user-trusted distribution

The controller SHALL verify helper/kernel/init/runtime and the preparation image
against a user-trusted manifest and SHALL NOT accept repository-declared bootstrap
paths as authority.
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

### Requirement: Preparation sees declared inputs and its outputs stay in their namespace

A preparation job SHALL receive only its declared input set, hashed into the
recipe identity. A provider that cannot enumerate its inputs SHALL use the whole
selected source as that set. Published output SHALL be served only within its
project namespace, unless it was obtained as a fixed-output artifact or verified
against a signature the user trusts.

#### Scenario: A file outside the declared set changes

- **WHEN** a branch changes a file that is not in the declared input set
- **THEN** that file was never present in the preparation VM
- **AND** the artifact remains valid for both sources

#### Scenario: A provider cannot enumerate its inputs

- **WHEN** a flake imports repository files the definition does not list
- **THEN** the whole selected source is its input set
- **AND** the artifact is not reused for a different source

#### Scenario: Another project has an identical definition

- **WHEN** it requests an environment matching another project's published output
- **THEN** that output is not served to it

### Requirement: Effective authority requires user scope

Project and imported definition requests SHALL be constrained by user-owned
grants and runtime hard limits. With no grant, preparation and setup SHALL use the
versioned exploration profile, which SHALL give runtime nothing and carry no
credentials. The rendered plan SHALL include phase, origin, host descriptors and
devices, guest grants, channels and credential reference names. Unsupported
required requests SHALL be refused by name.

#### Scenario: First open of a project with no user grants

- **WHEN** no grant exists for the project
- **THEN** preparation and setup use the exploration profile, rendered with its origin
- **AND** runtime receives nothing from it and no credential is delivered

#### Scenario: The proxy refuses a destination during local work

- **WHEN** a guest request is refused
- **THEN** the refused host is reported so it can become a declared request

#### Scenario: Manifest requests home and global credentials

- **WHEN** those requests have no matching user-owned authorization
- **THEN** launch refuses the requests or reports them as unresolved before setup
- **AND** the manifest does not gain authority by containing those declarations

### Requirement: Every host channel is bounded and instance-scoped

The initial runtime SHALL expose only its declared control, SSH and proxy
channels, with bounded framing and timeouts. Instance association SHALL derive
from the channel the controller created, never from a guest assertion, and phase
SHALL be host-held proxy state. NIC, TSI, arbitrary host-connect and implicit SSH
reverse/agent/X11 forwarding SHALL be unavailable. Host proxy policy SHALL mediate
actual network destinations, including host-local and private addresses.

#### Scenario: Client ignores proxy variables or forges another instance ID

- **WHEN** a guest attempts direct egress or another instance's channel
- **THEN** no alternate network route or authority is obtained
- **AND** correctly scoped allowed traffic still succeeds

#### Scenario: Allowed hostname resolves to an ungranted host-local address

- **WHEN** the proxy resolves or reconnects to that address
- **THEN** the connection is refused instead of reaching a host service

### Requirement: Setup authority ends on the host

Setup completion SHALL switch the channel to runtime policy and close streams
opened under setup, on the host, before runtime starts. Creation-time setup SHALL
end with a guest reboot. Runtime SHALL receive its own policy, not the union of
setup and runtime permissions. The system SHALL NOT claim that guest policy
separates setup from runtime once setup has had guest root.

#### Scenario: Hook leaves a background process with an open connection

- **WHEN** creation-time setup completes
- **THEN** the host closes the connection and the reboot ends the process
- **AND** failure to complete either prevents runtime admission

#### Scenario: A guest reports that cleanup succeeded

- **WHEN** a guest claims setup's processes are gone
- **THEN** that claim is not what admits runtime

### Requirement: Setup windows reopen setup explicitly

A setup window SHALL be a host operation on a running instance. It SHALL apply
setup policy to the instance's channel and provide an administrative session that
keeper sessions cannot create. It SHALL end on request or timeout and SHALL be
visible in status.

#### Scenario: A session tries to install a system package

- **WHEN** no setup window is open
- **THEN** the session lacks the authority and cannot obtain it through the keeper
- **AND** the setup window is the documented way to perform the install

#### Scenario: A window is opened on an instance holding credentials

- **WHEN** direct credentials have been delivered to that instance
- **THEN** opening the window names that exposure before egress widens

### Requirement: Credentials are not reusable base-image contents

Devcroft SHALL deliver authorized direct credentials only after boot and SHALL
NOT persist their values in launch configuration, logs or shared artifacts. They
SHALL be stated per instance and SHALL NOT be inherited through project identity.
Shared-base preparation in this phase SHALL exclude personal credentials and
ordinary local session disks. Status SHALL disclose direct guest exposure.

#### Scenario: A new instance of a known project

- **WHEN** it is created from an external branch fetched into the same checkout
- **THEN** it receives no credential merely because project identity matches

#### Scenario: User requests promotion of a disk used with credentials

- **WHEN** that disk is proposed as a shared fleet base
- **THEN** promotion is refused and clean reconstruction is required
- **AND** devcroft does not claim it can scrub all copies made by guest code
