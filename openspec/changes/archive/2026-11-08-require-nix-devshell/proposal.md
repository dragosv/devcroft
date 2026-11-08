## Why

With `env.provider = "nix"`, `up` accepted any flake that had a
`flake.nix` and a `flake.lock`, and asked nix for "the" dev environment of
the flake (`nix print-dev-env <root>`). Nix's own default search does not
stop at `devShells.<system>.default`: it falls through to
`packages.<system>.default` and `defaultPackage.<system>`. Measured on nix
2.31: a flake whose only output is `packages.<system>.default = hello`
brought up a sandbox whose environment was `hello`'s build environment, and
`up` reported it started, exit 0. The same happened for a flake with a dev
shell for another platform only.

A project can use Nix for packaging alone. devcroft has to consume the
environment the project declares for development, not infer one from how it
builds its package: a package's build environment looks valid and is not
the developer's workflow.

## What Changes

- The nix provider requires `devShells.<system>.default` for the current
  platform. A flake without it fails `up` at layer `provider`, exit code 3,
  with a message saying the project declares no Nix development environment
  and naming `devShells.<system>.default` as the fix.
- The provider never substitutes a package, `defaultPackage`, or a
  derivation's build environment: the capture names the shell in full, so
  nix's default search is never used.

## Impact

- `src/provider/nix.rs` (the check, the explicit installable),
  `src/provider/mod.rs` (`ProviderError::NoDevShell`).
- Every nix sample already declares the shell (checked: `nix-flake-sample`,
  `nix-go-sample`, `nix-probe-sample`, `kotlin-ktor-sample`). A project that
  relied on the fallback now fails `up` with the fix named, which is the
  point.
