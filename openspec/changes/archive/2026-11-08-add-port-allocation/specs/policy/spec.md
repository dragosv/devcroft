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

### Requirement: Rendering without a recorded allocation states the absence
The system SHALL render a policy for a manifest that requests allocation
but has no recorded allocation yet — before the first `up`, or after
`rm` — showing the request as pending rather than omitting it, inventing
a port, or failing. `policy --render` is a pure function of the manifest
plus recorded state, and must stay usable before any state exists.

This follows the precedent provider grants already set: they are also
knowable only after an `up`, and `--render` shows them as none for a
project that has never been up rather than refusing to render.

#### Scenario: Rendered before the first up
- **WHEN** `policy --render` runs for a manifest requesting allocation,
  for a sandbox that has never been up
- **THEN** the output names the request and shows no port granted for
  it, rather than omitting the request or failing

#### Scenario: Rendering never invents a port
- **WHEN** a policy is rendered with no recorded allocation
- **THEN** no allocated port appears in the compiled rules, preserving
  the invariant that nothing reaches the backend which the rendered
  policy cannot show
