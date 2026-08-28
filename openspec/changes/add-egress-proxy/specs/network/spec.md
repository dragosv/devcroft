# network

## ADDED Requirements

### Requirement: Domain allowlists are enforced

A declared `network.allow` SHALL be enforced. A manifest declaring an allowlist
SHALL NOT compile to a blanket network block or to unrestricted access.

#### Scenario: Allowed destination

- **WHEN** a sandboxed process connects to an allowlisted domain
- **THEN** the connection succeeds through the proxy

#### Scenario: Destination not on the allowlist

- **WHEN** a sandboxed process connects to a domain that is not allowlisted
- **THEN** the connection is refused
- **AND** the refusal names the destination and the deciding rule

#### Scenario: Allowlist cannot be enforced on this platform

- **WHEN** the platform cannot enforce domain-level filtering as declared
- **THEN** the degradation is named at `up` and in `doctor`
- **AND** access broader than the manifest declared is not granted silently

### Requirement: The proxy is the only route out

The compiled policy SHALL continue to deny direct egress at the kernel level.
The proxy endpoint SHALL be the sole permitted network path.

#### Scenario: Process bypasses the proxy

- **WHEN** a sandboxed process opens a socket directly to an address, ignoring
  any proxy configuration
- **THEN** the connection is refused by the enforced policy, not merely
  unfiltered

#### Scenario: Proxy is unavailable

- **WHEN** the proxy is not running
- **THEN** the sandbox does not start, or its network is closed
- **AND** it never falls back to unfiltered access

### Requirement: The proxy runs outside the sandbox it filters

The proxy SHALL run in the supervisor, outside the policy domain and process
namespace of any sandbox it serves.

#### Scenario: Client inspects the proxy

- **WHEN** a sandboxed process attempts to trace or read the memory of the proxy
- **THEN** the proxy is not reachable from inside that sandbox
- **AND** credentials held by the proxy are never resident inside it

#### Scenario: Request attribution

- **WHEN** a request reaches the proxy
- **THEN** the originating sandbox is identified from the listener it arrived on
- **AND** the identification requires nothing from the client

### Requirement: Refusals are legible to the developer

A refused connection SHALL be reportable in terms of the destination and rule,
not only as a transport failure.

#### Scenario: Package install fails on a refused host

- **WHEN** a package manager fails because a host was refused
- **THEN** the operator can determine which host and which rule caused it
- **AND** the failure is distinguishable from an unrelated network error

### Requirement: Guarantees are stated as constraint, not prevention

Documentation and diagnostics SHALL describe the allowlist as constraining
egress to permitted destinations, and SHALL NOT claim exfiltration is prevented.

#### Scenario: Allowlist includes an upload-capable destination

- **WHEN** an allowlisted destination accepts uploads
- **THEN** the tooling makes clear that it remains an outbound channel
