# network

## ADDED Requirements

### Requirement: A deny-default sandbox may opt in to any port on its own loopback

The system SHALL accept `network.loopback = "any"` in the project manifest.
When the key is set and the sandbox has a network namespace of its own,
processes inside the sandbox SHALL be able to bind a loopback TCP port the
kernel chooses (port 0) and SHALL be able to connect to any loopback TCP port
inside that namespace.

#### Scenario: A program listens on a port the kernel chooses

- **WHEN** a sandbox with `network.default = "deny"` and
  `network.loopback = "any"` runs a program that binds `127.0.0.1:0` and
  then connects to the port it was given
- **THEN** both the bind and the connect succeed

### Requirement: Declared ports remain the default

The default SHALL be `network.loopback = "declared"`, under which binding
and connecting are limited to `network.ports`, declared service ports and
the egress proxy port. A manifest without the key SHALL compile to the same
profile it compiled to before the key existed.

#### Scenario: The same program without the key

- **WHEN** the same program runs in a sandbox that does not set
  `network.loopback`
- **THEN** the bind is refused, exactly as before this change

#### Scenario: An existing manifest is unaffected

- **WHEN** a manifest that does not mention `network.loopback` is compiled
- **THEN** the compiled profile is byte-identical to the one produced
  without this change

### Requirement: Any-port loopback never reaches beyond the sandbox

Setting `network.loopback = "any"` SHALL NOT give a sandboxed process any
network destination outside its own network namespace. The egress proxy
SHALL remain the only route out, and `network.allow` SHALL remain the only
way to name a destination beyond the sandbox.

#### Scenario: A direct connection to an outside address

- **WHEN** a process in a sandbox with `network.loopback = "any"` opens a
  TCP connection directly to a non-loopback address, bypassing the proxy
- **THEN** the connection fails

#### Scenario: A service on the host's loopback

- **WHEN** a process in a sandbox with `network.loopback = "any"` connects
  to a port on which a host process is listening on the host's loopback
- **THEN** the connection does not reach the host process

#### Scenario: A destination not on the allowlist

- **WHEN** a process in a sandbox with `network.loopback = "any"` connects
  through the proxy to a domain not in `network.allow`
- **THEN** the proxy refuses it, as it would without the key

### Requirement: Any-port loopback is refused where the loopback is shared

The system SHALL refuse `network.loopback = "any"`, naming the key and the
reason, wherever the sandbox would share a loopback interface with the host
or with other sandboxes. In those places the same words would grant a
different, wider thing. It SHALL NOT degrade the key to a host-loopback
grant, and it SHALL NOT drop the key silently.

#### Scenario: Allow-default network

- **WHEN** a manifest sets both `network.default = "allow"` and
  `network.loopback = "any"`
- **THEN** validation fails at layer `config`, naming both keys and stating
  that an allow-default sandbox shares the host's loopback

#### Scenario: A host that cannot give the sandbox its own namespace

- **WHEN** `up` runs a manifest with `network.loopback = "any"` on a host
  where the sandbox cannot have a network namespace of its own
- **THEN** `up` fails at layer `backend` before any state is written,
  naming the key and why the namespace is unavailable

#### Scenario: macOS

- **WHEN** `up` runs a manifest with `network.loopback = "any"` on macOS
- **THEN** `up` fails at layer `backend`, stating that the platform has no
  per-sandbox loopback and that the grant would reach every host loopback
  service

### Requirement: Any-port loopback is visible in the rendered policy

`policy --render` SHALL show `network.loopback = "any"` as a network rule
with origin `manifest:network.loopback`. The rule SHALL state that it is
scoped to the sandbox's own network namespace. It SHALL be distinguishable
from `network.ports` entries and from the proxy port.

#### Scenario: Rendering a sandbox that opted in

- **WHEN** `policy --render` runs for a sandbox with
  `network.loopback = "any"`
- **THEN** the output contains one rule for it, carrying origin
  `manifest:network.loopback` and naming its namespace scope
