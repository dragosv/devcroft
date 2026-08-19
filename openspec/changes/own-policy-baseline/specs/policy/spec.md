# policy Delta Specification (own-policy-baseline)

## ADDED Requirements

### Requirement: The compiled profile states what it excludes
The system SHALL name, in the compiled profile, every backend policy
group it declines. Declining a group SHALL be explicit and inspectable
rather than achieved by omission, since the backend applies its group
set to any profile that does not say otherwise.

#### Scenario: Excluded groups are named
- **WHEN** a policy is compiled
- **THEN** the emitted profile names the groups devcroft declines, and
  resolving that profile through the backend confirms they are absent

#### Scenario: A mandatory group cannot be declined
- **WHEN** compilation would decline a group the backend enforces
  unconditionally
- **THEN** this is treated as a compilation error rather than emitted
  and silently ignored

### Requirement: System access is granted for a closure, not a host
The system SHALL grant the system paths its own processes require, and
SHALL NOT grant host toolchain access on behalf of project code. A
sandbox's executable environment SHALL come from the environment
provider's closure.

#### Scenario: Host binary not supplied by the closure
- **WHEN** project code inside a sandbox attempts to execute a host
  binary that the provider's closure does not supply
- **THEN** the attempt is denied, and the denial is explainable

#### Scenario: Closure-supplied toolchain works
- **WHEN** a project builds using the toolchain its provider supplies
- **THEN** the build succeeds, resolving its interpreter and libraries
  from the provider's store

### Requirement: Inherited settings are declared, not inherited
The system SHALL set explicitly every backend setting its guarantees
depend on, rather than receiving it through profile inheritance. A
setting that would be lost by changing what the profile extends SHALL be
treated as undeclared.

#### Scenario: A depended-upon setting survives an inheritance change
- **WHEN** the compiled profile's inheritance changes
- **THEN** every setting devcroft's guarantees depend on is still
  present, because each is set by devcroft rather than inherited

#### Scenario: Declared settings are rendered
- **WHEN** the compiled policy is rendered
- **THEN** such settings appear in the output, since they are policy

### Requirement: A denial is attributed to whoever imposed it
The system SHALL distinguish, when explaining a denial, between a rule
devcroft compiled and a rule the backend enforces unconditionally,
naming the imposing group in the latter case. An incomplete grant set
SHALL be diagnosable rather than surfacing only as an execution failure.

#### Scenario: Backend-enforced denial names its group
- **WHEN** the user asks why a path denied by a backend-enforced policy
  group is denied
- **THEN** the answer names that group, distinguishing it from a rule
  devcroft chose

#### Scenario: Ungranted path is distinguished from a denied one
- **WHEN** the user asks about a path that no rule grants and no group
  denies
- **THEN** the answer states that nothing grants it, distinct from a
  path actively denied

## MODIFIED Requirements

### Requirement: Rendering accounts for every rule reaching the backend
The system SHALL render the complete effective policy, including rules
the backend enforces unconditionally and which devcroft neither chose
nor can remove. Previously the rendered policy covered only rules
devcroft compiled, while the backend's own group set reached the sandbox
unrendered; rendering now accounts for both, distinguishing them by
origin. Completeness SHALL be verified by comparing the rendered output
against the profile as the backend resolves it, not as devcroft wrote
it.

#### Scenario: Render matches the resolved profile
- **WHEN** a policy is compiled, emitted, and resolved by the backend
- **THEN** the rendered output accounts for every rule in the resolved
  profile, with no unrendered remainder of any origin

#### Scenario: Internally required grants are rendered
- **WHEN** the compiled policy includes a grant devcroft requires for
  its own operation rather than one the user or provider requested
- **THEN** that grant appears in the rendered policy with an origin,
  exactly as any other rule does

#### Scenario: Backend-enforced rules are marked as such
- **WHEN** the rendered policy includes rules the backend imposes
  regardless of what devcroft emits
- **THEN** they are distinguishable from rules devcroft compiled, so the
  reader can tell what devcroft controls from what it does not

### Requirement: Backend compatibility is checked against a surface
The system SHALL check backend compatibility against the interface it
depends on — the profile schema it emits, the group semantics it relies
on, and the invocation shape it uses — rather than against a version
range standing in for an undocumented rule set. The reported range SHALL
reflect versions actually exercised, and the failure message SHALL name
what compatibility means.

#### Scenario: Emitted profile validates against the installed backend
- **WHEN** the environment check runs with the backend present
- **THEN** the emitted profile is validated against that backend's own
  schema, so a schema change is detected as a schema change

#### Scenario: Group semantics are verified, not assumed
- **WHEN** the environment check runs
- **THEN** it confirms the backend still applies its group set the way
  the compiled policy assumes, so a change in that behavior surfaces as
  a named incompatibility rather than as altered enforcement

#### Scenario: Incompatible backend names the surface
- **WHEN** the installed backend is outside the supported range
- **THEN** the failure names the interface that is incompatible, not
  only the version numbers
