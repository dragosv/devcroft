# cli Delta Specification (add-flox-services)

## ADDED Requirements

### Requirement: Services surface through existing commands
The system SHALL expose service state through the existing
`status`, `ps`, and `logs` commands rather than a new top-level command,
keeping the MVP command surface closed. `ps` SHALL list services
alongside sessions and label which is which; `logs` SHALL include service
output attributed to the service that produced it.

#### Scenario: ps distinguishes services from sessions
- **WHEN** a sandbox has one interactive session and two services running
- **THEN** `ps` lists all three and makes clear which entries are
  services and which is a session

#### Scenario: Service output is attributable in logs
- **WHEN** two services both write output
- **THEN** `logs` shows each line attributed to the service that emitted
  it, not merged indistinguishably

### Requirement: doctor reports whether listening sockets work
The system SHALL report, as a `doctor` diagnostic, whether a sandbox on
this host can bind a listening socket under a deny-default network
policy. When it cannot, `doctor` SHALL state that services requiring a
listening port will not work under that policy and name the current
workaround, so the limitation is discoverable before a service silently
fails to bind.

#### Scenario: Host where deny-default blocks binding
- **WHEN** `doctor` runs on a host whose backend denies `bind`/`listen`
  under `network.default = "deny"`
- **THEN** it reports that services needing a port will fail under that
  policy, and names `network.default = "allow"` as the current workaround
  along with the egress-filtering it costs

#### Scenario: Host where binding works
- **WHEN** `doctor` runs on a host where a deny-default policy still
  permits loopback binding
- **THEN** it reports listening sockets as available, with no warning

### Requirement: Service failure is reflected in exit codes
The system SHALL NOT fail `up` because a service failed to start —
`up` succeeds and reports the failure through service state. A command
that explicitly reports sandbox health SHALL make service failure
discoverable rather than reporting success unconditionally.

#### Scenario: up succeeds with a failed service
- **WHEN** `up` runs and one declared service fails to start
- **THEN** `up` exits 0, prints that the service failed, and `status`
  shows the failure
