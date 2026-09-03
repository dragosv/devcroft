# config Delta Specification (add-swift-provider)

## ADDED Requirements

### Requirement: swift provider value

The system SHALL accept `env.provider = "swift"` as a valid manifest
value, validated by the same path that accepts `flox`, `nix` and
`devbox`, and SHALL NOT reject it as an unknown provider name.

`swift` SHALL have exactly one spelling. No alias SHALL be accepted —
in particular not `swiftpm` or `xcode`. `swiftpm` names the package
manager, and this provider deliberately does not resolve packages;
`xcode` names one of two possible backing installations, the other being
the Command Line Tools, and a project must not have to change its
manifest because a colleague installed the other one.

#### Scenario: The swift provider value validates

- **WHEN** a manifest declares `env.provider = "swift"` on macOS
- **THEN** validation SHALL succeed
- **AND** the canonical provider name reaching dispatch, `status` and
  policy rule origins SHALL be `swift`

#### Scenario: Package-manager and IDE spellings are rejected

- **WHEN** a manifest declares `env.provider = "swiftpm"` or
  `env.provider = "xcode"`
- **THEN** validation SHALL fail
- **AND** the message SHALL name `swift` as the provider that exists

### Requirement: Provider rejection distinguishes platform from support

The system SHALL distinguish a provider that is unsupported from one that
is supported but unavailable on this platform.

`env.provider = "swift"` on a non-macOS host SHALL be reported as a
platform mismatch at layer `provider`, and SHALL NOT be reported through
the "not yet supported", "out of scope by design", "version manager" or
"unknown provider" categories. Each of those tells the user something
false: the provider exists, is in scope, is not a version manager, and is
not unknown — it simply cannot be backed here.

#### Scenario: Manifest validation on a non-macOS host

- **WHEN** a manifest declaring `env.provider = "swift"` is validated on
  Linux
- **THEN** the error SHALL be a platform mismatch naming macOS
- **AND** it SHALL be distinguishable in type, not only in wording, from
  the existing rejection categories
