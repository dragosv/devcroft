# policy Delta Specification (use-nono-library)

## ADDED Requirements

### Requirement: Rendering does not depend on an installed backend
The system SHALL render the compiled policy on a host with no backend
binary installed. Inspecting what a sandbox would enforce SHALL NOT
require the ability to run it.

#### Scenario: Render with no backend present
- **WHEN** `policy --render` runs on a host with no backend binary
- **THEN** it prints the complete compiled policy with origins, exactly
  as it would on a host where the sandbox can run

#### Scenario: Explanation without a backend
- **WHEN** the user asks why a path is allowed or denied on such a host
- **THEN** the answer is produced from the compiled policy, since the
  compilation is devcroft's own and needs nothing external

### Requirement: The compiled policy is projected, not replaced
The system SHALL retain its own annotated representation of the compiled
policy, carrying the origin of every rule, and SHALL derive whatever the
enforcement layer consumes from it as a projection. The enforcement
layer's own types SHALL NOT become the system's internal representation,
so that origins — which exist only in devcroft's model — cannot be lost.

#### Scenario: Origins survive the projection
- **WHEN** a policy is compiled and handed to the enforcement layer
- **THEN** every rule the enforcement layer receives corresponds to a
  rule in the annotated representation, and the rendered output still
  names each rule's origin

#### Scenario: Determinism is unchanged
- **WHEN** the same manifest and provider grants are compiled twice
- **THEN** both the annotated representation and the projection are
  byte-identical, the same guarantee the compilation already carries

## MODIFIED Requirements

### Requirement: Degraded capabilities are reported from the enforcement layer
The system SHALL determine which requested aspects the host can enforce
by asking the enforcement layer what the running platform supports,
rather than by inferring it from the platform alone. Where an aspect
cannot be enforced, `up` SHALL print exactly one warning naming the
aspect, the reason, and the fallback — unchanged in contract, but now
derived from the layer that would do the enforcing.

#### Scenario: Unsupported aspect is named from platform support
- **WHEN** a manifest requests an aspect the running kernel or platform
  cannot enforce
- **THEN** `up` warns once, naming the aspect and the fallback, based on
  the enforcement layer's reported support rather than an assumption
  about the operating system

#### Scenario: Supported aspect produces no warning
- **WHEN** every requested aspect is enforceable on the host
- **THEN** `up` prints no degradation warning
