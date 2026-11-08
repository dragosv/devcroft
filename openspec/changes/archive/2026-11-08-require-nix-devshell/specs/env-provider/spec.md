# env-provider Delta Specification (require-nix-devshell)

## ADDED Requirements

### Requirement: The nix provider requires a declared development shell
When `env.provider` is `nix`, the system SHALL use the development shell the
flake declares explicitly for the current platform, `devShells.<system>.default`,
where `<system>` is nix's own configured platform. A `flake.nix` and a
`flake.lock` SHALL NOT be sufficient on their own. When that shell is absent,
`up` SHALL fail at layer `provider` with exit code 3 and a message that says
the project declares no Nix development environment and names declaring
`devShells.<system>.default` as the fix. The provider SHALL NOT select a
package, `defaultPackage`, or a derivation's build environment in its place,
and SHALL NOT leave the choice to nix's default attribute search.

#### Scenario: A flake used only for packaging
- **WHEN** provider is `nix` and the flake declares `packages.<system>.default`
  but no `devShells`
- **THEN** `up` fails at layer `provider` with exit code 3
- **AND** the message says the project declares no Nix development environment
  and names `devShells.<system>.default`
- **AND** no sandbox is started

#### Scenario: A dev shell for another platform only
- **WHEN** the flake declares `devShells.<other>.default` and not
  `devShells.<system>.default` for the current platform
- **THEN** `up` fails at layer `provider` with exit code 3, as above

#### Scenario: A dev shell that is not the default one
- **WHEN** the flake declares `devShells.<system>.<name>` but no
  `devShells.<system>.default`
- **THEN** `up` fails at layer `provider` with exit code 3, as above

#### Scenario: No substitution even if the check is bypassed
- **WHEN** the environment is captured
- **THEN** the capture names `devShells.<system>.default` in full, so a flake
  without it fails capture rather than resolving to a package's build
  environment

#### Scenario: A declared shell is used
- **WHEN** the flake declares `devShells.<system>.default`
- **THEN** `up` resolves that shell's environment, as before
