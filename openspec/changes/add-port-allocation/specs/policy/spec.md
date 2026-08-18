# policy Delta Specification (add-port-allocation)

## ADDED Requirements

### Requirement: Allocated ports carry their own origin
The system SHALL compile an allocated port into the same backend rule a
manifest-declared port produces, annotated with an origin identifying it
as allocated. `policy --render` SHALL show it, preserving the invariant
that nothing reaches the backend which the rendered policy cannot show —
which matters more here than for manifest rules, since the user did not
choose this value and cannot predict it.

#### Scenario: Rendered with an allocated origin
- **WHEN** a sandbox has one allocated and one manifest-declared port
- **THEN** `policy --render` shows both, with origins distinguishing
  which devcroft chose

#### Scenario: Compilation stays deterministic
- **WHEN** the same manifest and the same recorded allocation are
  compiled repeatedly
- **THEN** the rendered policy is byte-identical each time
