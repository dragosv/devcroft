## ADDED Requirements

### Requirement: The degraded-isolation opt-in is a flag on `up`
The system SHALL accept `--allow-degraded-isolation` on `up` and on no
other command, SHALL list it in `up`'s usage line and in `--help`, and
SHALL NOT read the same decision from the manifest or from the
environment.

#### Scenario: The flag is discoverable
- **WHEN** the user runs `devcroft --help` or `devcroft up` with bad
  arguments
- **THEN** the usage text shows `--allow-degraded-isolation` on `up`

#### Scenario: The flag is not ambient
- **WHEN** an environment variable or manifest key attempts to express
  the same decision
- **THEN** it has no effect, and `up` on such a host fails as it would
  without the flag
