## ADDED Requirements

### Requirement: VM execution is explicit and cannot silently downgrade

The runtime SHALL identify Linux guest target and VM mode independently from
provider and guarantee tier. A missing required VM capability SHALL refuse
launch, without selecting the native runtime as fallback.

#### Scenario: macOS project requires Xcode

- **WHEN** a Darwin-only environment is selected for a Linux guest
- **THEN** the target mismatch is reported before project execution
- **AND** no Darwin binary is attempted in the guest or run on the host instead

### Requirement: Runtime kind is a user-owned decision

Repository content SHALL NOT select or imply native execution. Native execution
SHALL require a user-owned grant bound to project identity. Without one, launch
SHALL use a qualified VM row or refuse, naming the grant, and SHALL NOT prompt
without a terminal. Status SHALL show the runtime and who selected it.

#### Scenario: A repository declares a Darwin-only provider

- **WHEN** an unreviewed project's manifest names `swift` and no grant exists
- **THEN** it does not reach the native path
- **AND** launch refuses, naming the grant that would allow it

#### Scenario: No VM row is qualified on this host

- **WHEN** a project without a grant is opened on an unqualified host
- **THEN** launch refuses rather than falling back to native execution

#### Scenario: An instance predates the rule

- **WHEN** an existing native instance is restarted
- **THEN** it keeps its runtime and status reports it as native

### Requirement: Editor clients are classified by what a guest can do to them

The untrusted-project claim SHALL cover only terminal SSH, SFTP and guest-served
editor paths. A client whose remote can execute code on the host SHALL require a
user-owned trusted-editor grant, and `ssh-config` SHALL emit no entry for a
project without one. Generated client configuration SHALL offer no agent, X11 or
remote forwarding.

#### Scenario: A project is not marked trusted

- **WHEN** `ssh-config` runs for that instance
- **THEN** no editor-facing host entry is emitted
- **AND** the output names the guest-served editor path and the grant

#### Scenario: A guest replaces its SSH server

- **WHEN** a modified server asks the client for agent or X11 forwarding
- **THEN** the generated client configuration does not offer them

### Requirement: Readiness requires a compatible instance-bound handshake

The controller SHALL verify launch artifacts and accept a handshake bound to the
instance generation, target and compatible protocol/capabilities before exposing
sessions. It SHALL distinguish helper, transport, keeper and service health.

#### Scenario: Helper exists but keeper never responds

- **WHEN** the guest misses the bounded readiness deadline
- **THEN** launch fails at the appropriate layer and owned transient resources
  are cleaned up
- **AND** helper PID existence is not reported as project readiness

#### Scenario: Old endpoint responds after restart

- **WHEN** a handshake names a previous instance generation
- **THEN** it is rejected and cannot control the current guest

### Requirement: Guest sessions preserve complete local session semantics

Keeper and SSH SHALL execute inside the guest. Exec, shell, PTY/resize, signals,
stdout/stderr, exit status, SFTP and permitted local forwarding SHALL operate
against guest processes, files and network endpoints.

#### Scenario: Guest process and host process have the same numeric PID

- **WHEN** a client sends a signal to the guest session
- **THEN** the guest keeper signals the guest process group
- **AND** the host process is unaffected

#### Scenario: SFTP and forwarded service use guest state

- **WHEN** a client writes `/workspace/file` and forwards the guest database port
- **THEN** the file and connection belong to that guest instance
- **AND** no host checkout path or host loopback service is substituted

### Requirement: Stop preserves data and forced stop is reported as a crash

Normal shutdown SHALL stop services and synchronize guest state before helper
exit. `down` SHALL retain private workspace and service data. Forced termination
SHALL be labeled distinctly and checked for recovery at restart.

#### Scenario: Service ignores shutdown

- **WHEN** the shutdown deadline expires
- **THEN** the helper is terminated under the recorded ownership identity
- **AND** status records forced stop and retains the disk for recovery

### Requirement: Removal and recovery respect ownership and compatibility

Removal SHALL affect only stopped resources owned by the selected instance.
Restart SHALL check disk/runtime compatibility and process identity before
reattaching or signaling. Shared artifacts SHALL remain while leased.

#### Scenario: Stale state contains a reused host PID

- **WHEN** recovery finds that the PID belongs to an unrelated process
- **THEN** it does not signal that process or claim the VM is running

#### Scenario: Removing one of two consumers

- **WHEN** one instance is removed while another leases the same base
- **THEN** its private data is removed and the shared base remains usable
- **AND** the original checkout and external definition remain unchanged

### Requirement: Resource budgets include host-side support processes

Launch SHALL declare finite guest and storage/log/transport budgets, identify
the enforcement available for host helpers, and refuse a required unsupported
resource limit. Guest RAM SHALL NOT be advertised as total host memory usage.

#### Scenario: Guest fills its writable disk or floods logs

- **WHEN** the configured disk or log limit is reached
- **THEN** further growth is bounded and an attributable error is surfaced
- **AND** cleanup does not delete another instance's data
