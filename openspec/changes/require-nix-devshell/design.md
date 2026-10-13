## Context

`nix print-dev-env <root>` resolves the flake's default installable with
nix's own attribute search, which ends in `packages.<system>.default` and
`defaultPackage.<system>`. That fallback is what this change removes.

## Decisions

### D1: Two layers, the second structural

1. **A check** before capture: `nix eval --json <root>#devShells --apply …`
   returns whether `devShells.<system>` has a `default`. A boolean from the
   flake's own outputs, through the same installable machinery `print-dev-env`
   uses.
2. **The capture names the shell**: `print-dev-env <root>#devShells.<system>.default`.
   Nix still tries its prefixes, but every candidate it tries is under a
   `devShells.<system>` path; none can be a package.

Only a flake with no `devShells` at all reaches the check as an error rather
than `false`, and that one is recognised from nix's text ("does not provide
attribute"). If the wording changes, the failure stays a resolution failure
at layer `provider`, never a fallback, because of layer 2. Measured: with the
check removed, a packages-only flake still fails at capture, without nix
reaching the package's derivation.

### D2: The platform comes from `nix config show system`, not evaluation

`builtins.currentSystem` needs `--impure`, which nix resolution never passes
(`add-nix-provider`). `nix config show system` is the same setting nix's
own default search uses, read without evaluating anything.

### D3: Not `builtins.getFlake`

A pure `getFlake` on the locked URL from `nix flake metadata` gives a clean
boolean for every shape, but refuses a dirty git tree ("unlocked flake
reference"), measured, and a dirty tree is the normal state while
developing. `--impure` would accept it, and is ruled out by D2's rule.
