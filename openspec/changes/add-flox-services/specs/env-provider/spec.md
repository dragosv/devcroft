# env-provider Delta Specification (add-flox-services)

## ADDED Requirements

### Requirement: Provider declares services or explicitly declares none
The system SHALL obtain service declarations from the resolved provider
as part of resolution, alongside the existing environment diff and
read-only grants. A provider with no service concept SHALL declare that
it supports none explicitly, rather than being assumed to by returning an
empty list. Service declarations SHALL be captured host-side at `up`,
during the same trusted provisioning phase as the rest of resolution, and
SHALL NOT require running project code to discover.

#### Scenario: flox declares services from its manifest
- **WHEN** provider is `flox` and the flox manifest's `[services]`
  section declares a service
- **THEN** resolution yields that service declaration, and it is started
  inside the sandbox at `up`

#### Scenario: Provider without services declares none
- **WHEN** provider is `nix`, which has no service concept
- **THEN** resolution reports that the provider supports no services —
  distinguishable from a provider that supports services and happens to
  have zero declared

### Requirement: Services requested from a provider that cannot supply them fail loudly
The system SHALL fail `up` at layer `provider` with exit code 3 when
services are requested from a provider that declares no service support,
naming the provider and the reason. The system SHALL NOT silently start
nothing, and SHALL NOT report the sandbox as fully up while quietly
ignoring a service request.

#### Scenario: Services requested under a provider that has none
- **WHEN** the project asks for services while `env.provider = "nix"`
- **THEN** `up` fails at layer `provider` with exit code 3, naming `nix`
  and stating that the provider has no service mechanism

### Requirement: Service declarations do not widen the policy
The system SHALL NOT grant a service any filesystem or network access
beyond what the compiled policy already allows. Discovering a service
declaration SHALL NOT add rules to the compiled profile, and a service
whose command needs access the manifest does not grant SHALL fail at
runtime rather than causing `up` to widen the policy on its behalf.

#### Scenario: Service needing an ungranted port does not widen the policy
- **WHEN** a declared service binds a port the manifest does not permit
- **THEN** `policy --render` is byte-identical to the same manifest with
  no services declared, and the service fails at runtime instead
