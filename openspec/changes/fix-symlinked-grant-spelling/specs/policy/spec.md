# policy Delta Specification (fix-symlinked-grant-spelling)

## ADDED Requirements

### Requirement: A grant covers the path as written and as it resolves
Where a filesystem grant's path as the manifest spells it differs from
the path it canonicalizes to, the system SHALL grant both. Where they do
not differ, the system SHALL grant exactly one, unchanged.

This is not a widening. The two spellings name the same target — on
macOS `/tmp/proj` and `/private/tmp/proj` are one directory reached two
ways — so granting both grants one thing under two names. What *would*
be a widening is granting a symlink whose target lies elsewhere, which
the existing symlink-escape refusal already prevents and which this
requirement SHALL NOT weaken.

The rule applies wherever grants are derived, not only where they are
handed to the backend: the compiled capability set and the mount view
SHALL be computed from the same resolution, so the two cannot disagree
about what a manifest granted.

#### Scenario: Both spellings of a granted path work
- **WHEN** a sandbox is granted a path whose spelling in the manifest
  differs from its canonical form, and a process inside writes through
  each spelling
- **THEN** both writes succeed

#### Scenario: A grant with no symlink in it is unchanged
- **WHEN** a manifest's grants canonicalize to themselves
- **THEN** the compiled policy is byte-identical to what it was before
  this requirement existed

#### Scenario: The escape refusal still fires
- **WHEN** a project-relative grant resolves, through a symlink, outside
  the project root
- **THEN** compilation fails naming the entry and its real target, and
  neither spelling is granted

#### Scenario: Rendering shows what the backend received
- **WHEN** both spellings of a path are granted
- **THEN** `policy --render` shows both, because a rule the backend
  receives that rendering cannot show breaks the policy invariant
