## ADDED Requirements

### Requirement: A rootfs grant is an ordinary provider grant
The system SHALL compile a `devcontainer` sandbox's rootfs directory as
a read-only grant with origin `provider:devcontainer`, rendered by
`policy --render` by its real host path, subject to the same
deny-overlap check as every grant, and SHALL NOT grant the rootfs store
as a whole or any other digest.

#### Scenario: Only the resolved digest is granted
- **WHEN** the rootfs store holds two digests and the project resolves
  to one
- **THEN** the compiled policy grants that one directory and `why` on a
  path under the other reports it denied by the baseline

#### Scenario: Execute follows the grant
- **WHEN** a session executes a binary from the rootfs
- **THEN** it is allowed because the rootfs is read-granted, and a host
  binary at an ungranted path is refused — the existing execute-scoping
  rule, with no devcontainer-specific exception
