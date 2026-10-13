## 1. Requirement

- [x] 1.1 Measure the fallback: on nix 2.31, `print-dev-env <root>` on a
      packages-only flake, and on one with another platform's shell only,
      builds the package's `-env` derivation; `up` on a packages-only
      nixpkgs flake (`hello`) started a sandbox, exit 0.
- [x] 1.2 `ProviderError::NoDevShell { system }`: layer `provider`, exit 3,
      message naming `devShells.<system>.default`.
- [x] 1.3 `ensure_dev_shell_declared` before capture; the platform from
      `nix config show system` (design D2).
- [x] 1.4 The capture names `devShells.<system>.default` in full (design D1).

## 2. Tests

- [x] 2.1 `a_flake_without_this_platforms_default_dev_shell_is_refused`:
      packages only, another platform only, a non-default shell only. Fails
      with the check removed, and with a check that accepts everything.
- [x] 2.2 `a_declared_default_dev_shell_passes_the_check` (control).
- [x] 2.3 `the_capture_never_reaches_a_package`: fails when the capture is
      given the bare flake path (nix reaches the package's derivation).
- [x] 2.4 `up_fails_at_provider_layer_when_the_flake_declares_no_dev_shell`
      (e2e, real nixpkgs `hello`): exit 3 and the message; with the old
      behaviour restored it starts, exit 0.
- [x] 2.5 Every nix sample declares the shell (`nix-flake-sample`,
      `nix-go-sample`, `nix-probe-sample`, `kotlin-ktor-sample`).
