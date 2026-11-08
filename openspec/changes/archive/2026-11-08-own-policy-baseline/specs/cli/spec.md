# cli Delta Specification (own-policy-baseline)

## ADDED Requirements

### Requirement: The sandbox does not claim protections it does not apply
The system SHALL NOT emit backend rules that the invocation mode it uses
cannot enforce. Where a backend offers a mechanism that is inert in the
mode devcroft invokes, devcroft SHALL omit it rather than emit it
unenforced, so that a reader of the compiled profile cannot conclude a
protection is active when it is not.

#### Scenario: No unenforced mechanism in the emitted profile
- **WHEN** the compiled profile is inspected
- **THEN** it contains no rule of a kind the invocation mode leaves
  unenforced, and every rule present is one the sandbox actually applies

#### Scenario: Adopting such a mechanism is a stated decision
- **WHEN** a protection of that kind is later wanted
- **THEN** it is introduced with the enforcement mode that makes it real
  stated alongside it, rather than inherited as a side effect of
  extending a backend-supplied profile

## MODIFIED Requirements

### Requirement: doctor reports backend compatibility
The system SHALL report, through `doctor`, whether the installed backend
is compatible. The check SHALL exercise the interface devcroft depends on
rather than assert a version range alone, and its output SHALL name the
compatibility surface so a failure is actionable. `doctor` SHALL pass
against the current released backend version the suite has been run
against.

#### Scenario: Compatible backend passes
- **WHEN** `doctor` runs with a backend version the suite has been
  exercised against
- **THEN** the backend line passes, reporting the version found

#### Scenario: Failure names what is incompatible
- **WHEN** `doctor` runs with a backend outside the supported range
- **THEN** the failure line names the interface at issue and how to
  resolve it, not only the version numbers
