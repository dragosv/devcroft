# env-provider Delta Specification (own-policy-baseline)

## ADDED Requirements

### Requirement: A provider needing host libraries SHALL declare them
The system SHALL require any environment provider whose runtime links
against host libraries to declare those paths as provider grants, which
are compiled into the policy and rendered with that provider's origin. A
provider SHALL NOT rely on the baseline to supply host library access,
because the baseline grants none.

#### Scenario: Host-linked provider declares its grants
- **WHEN** a provider whose artifacts link against host libraries
  resolves an environment
- **THEN** the host library paths it requires appear in the compiled
  policy attributed to that provider, and `policy --render` shows them
  with its origin

#### Scenario: Undeclared host dependency fails rather than works by accident
- **WHEN** such a provider does not declare the paths its runtime needs
- **THEN** its environment fails to execute rather than succeeding
  because the baseline happened to grant them

#### Scenario: Closure providers declare no host library paths
- **WHEN** a provider supplies a complete closure including its own
  linker and C library
- **THEN** it declares no host library grants, and its sandboxes are
  reachable-only-from-the-closure by construction

### Requirement: A weaker guarantee is visible in the policy, not only in a label
The system SHALL make the difference between a self-contained closure
and a host-linked runtime observable in the compiled policy itself. A
provider offering the weaker guarantee SHALL be distinguishable by the
grants it requires, so the difference is inspectable through the same
mechanism that shows every other rule rather than resting on a tier name
in documentation.

#### Scenario: The compiled policy distinguishes the two
- **WHEN** the policies of a closure-backed project and a host-linked
  project are rendered
- **THEN** the host-linked one carries provider-attributed host library
  grants that the closure-backed one does not

#### Scenario: The provider's own resolution does not widen the policy silently
- **WHEN** a provider's declared host library grants would exceed what
  the project's manifest permits
- **THEN** `up` fails naming the paths, exactly as it does for any other
  provider grant that would widen the policy
