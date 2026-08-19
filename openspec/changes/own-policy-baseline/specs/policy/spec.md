# policy Delta Specification (own-policy-baseline)

## ADDED Requirements

### Requirement: The compiled profile is self-contained
The system SHALL emit a backend profile that depends on no profile it
does not itself define. The profile SHALL NOT inherit rules from the
backend's built-in profiles, and every rule reaching the backend SHALL
originate in devcroft's own compilation.

#### Scenario: No inherited profile reference
- **WHEN** a manifest is compiled to a backend profile
- **THEN** the emitted profile carries no reference to a backend-defined
  base profile, and the rules it contains are exactly those compilation
  produced

#### Scenario: Backend version change does not change the policy
- **WHEN** the backend binary is replaced with a different version whose
  built-in profiles differ
- **THEN** the compiled profile is byte-identical, because none of its
  content came from the backend

### Requirement: devcroft owns the system-access baseline
The system SHALL grant, as baseline rules, the system paths a sandboxed
process needs in order to execute at all — at minimum the dynamic
linker, system binary directories, and the character devices and locale
data a normal process opens. Each such rule SHALL carry the `baseline`
origin, and the set SHALL be selected per operating system.

#### Scenario: A sandbox can execute a system binary
- **WHEN** a sandbox is brought up with a manifest granting only the
  project root
- **THEN** a session can exec a system binary and read the project root,
  with no rule outside devcroft's own compilation involved

#### Scenario: Baseline rules are attributable
- **WHEN** the compiled policy is rendered
- **THEN** each system-access rule appears with the `baseline` origin,
  distinguishable from rules originating in the manifest or a provider

### Requirement: Rendering is complete by construction
The system SHALL render every rule present in the emitted backend
profile. A rule that reaches the backend and does not appear in the
rendered policy SHALL be treated as a defect, and this SHALL be verified
by comparing the two rather than by inspection.

#### Scenario: Render and emitted profile agree
- **WHEN** a policy is compiled and both the rendered output and the
  emitted profile are produced
- **THEN** they describe the same set of rules, with no rule present in
  one and absent from the other

#### Scenario: Internally required grants are rendered too
- **WHEN** the compiled policy includes a grant devcroft requires for its
  own operation rather than one the user or provider requested
- **THEN** that grant appears in the rendered policy with an origin,
  exactly as any other rule does

### Requirement: A denial caused by a baseline rule is explainable
The system SHALL attribute a denial arising from the baseline to the
baseline rule set, naming the rule involved, so that an incomplete
baseline is diagnosable rather than surfacing only as an unexplained
execution failure.

#### Scenario: Denied path with no matching rule
- **WHEN** the user asks why a path outside every granted and baseline
  rule is denied
- **THEN** the answer states that no rule grants it, distinguishable from
  a path denied by an explicit deny entry

#### Scenario: Baseline denial is named
- **WHEN** a denial results from a rule devcroft compiled as baseline
- **THEN** the explanation names that rule and its `baseline` origin

## MODIFIED Requirements

### Requirement: Nothing reaches the backend that rendering cannot show
The system SHALL ensure that the compiled backend profile and the
rendered policy are two views of one artifact. Previously this held only
for rules devcroft compiled, while rules inherited from the backend's
built-in base profile reached the backend unrendered; that inheritance
is removed, so the guarantee now covers the entire profile.

#### Scenario: The whole profile is inspectable
- **WHEN** a sandbox is brought up and its policy rendered
- **THEN** the rendered output accounts for every rule the backend
  receives, with no unrendered remainder of any origin

### Requirement: Backend compatibility is checked against a surface
The system SHALL check backend compatibility against the interface it
depends on — the profile schema it emits and the invocation shape it
uses — rather than against a version range standing in for an
undocumented rule set. The reported compatibility range SHALL reflect
versions the system has actually been exercised against, and the failure
message SHALL name what compatibility means.

#### Scenario: Emitted profile validates against the installed backend
- **WHEN** the environment check runs with the backend present
- **THEN** the profile devcroft emits is validated against that
  backend's own schema, so a schema change is detected as a schema
  change rather than as an unexplained runtime failure

#### Scenario: Incompatible backend names the surface
- **WHEN** the installed backend is outside the supported range
- **THEN** the failure names the interface that is incompatible, not
  only the version numbers
