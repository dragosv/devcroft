## ADDED Requirements

### Requirement: Artifact-tier host grants are attributed and rendered
The system SHALL compile an artifact-tier provider's host grants with a
`provider:<name>` origin, and `policy --render` SHALL show them under
that origin. This is where the difference between a self-contained
closure and a host-linked runtime stops being a tier name and becomes a
visible difference in the compiled policy; an artifact-tier provider is
the first case where such rules are numerous enough to be worth
reading.

#### Scenario: Rendering a swift sandbox's policy
- **WHEN** `policy --render` runs for a sandbox with `env.provider = "swift"`
- **THEN** the toolchain, SDK, host-data and host-userland grants appear
  with origin `provider:swift`, byte-identical for identical inputs

#### Scenario: Comparing a closure-tier and an artifact-tier sandbox
- **WHEN** `policy --render` is run for a flox sandbox and a swift
  sandbox of the same project shape
- **THEN** the swift output contains host paths the flox output does
  not, and the difference is attributable to the provider origin rather
  than to the baseline

### Requirement: A provider grants only paths the host has
The system SHALL NOT emit a provider grant for a path that does not
exist on the host: the swift provider existence-checks every grant it
derives and omits the absent ones. On macOS the backend matches paths
as spelled and accepts a rule for an absent path that then enforces
nothing; the system libraries a Swift artifact links against
(`/usr/lib/libSystem.B.dylib` and its siblings) are served from the
dynamic linker's shared cache and are absent from the filesystem, so a
provider that granted them per `otool` output would produce rules that
look right and do nothing. The provider grants the toolchain's library
*directories* that exist, and relies on the shared cache for the rest,
which needs no grant.

#### Scenario: A shared-cache dylib is not granted by name
- **WHEN** the toolchain's runtime library paths are derived
- **THEN** no grant names an individual system dylib, and the rendered
  policy carries only directories and files that exist on this host
