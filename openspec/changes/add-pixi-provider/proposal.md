# Change: add-pixi-provider

## Why

The owner explicitly requested pixi support on 2026-11-16, reopening the
artifact-provider scheduling decision in `docs/decisions.md` §1. Existing Pixi
projects should use their locked environments in devcroft without maintaining
an equivalent Nix definition.

Revised 2026-11-17 after an adversarial review of the first draft. The main
corrections: no `/usr/bin` grant on Linux, which contradicted the draft's own
policy requirements; activation order comes from pixi rather than being
reimplemented; the artifact store gets a location and a way to reclaim it; the
claim that `pixi run` stays usable inside the sandbox is withdrawn because
pixi is not in the sandbox; and whether locked PyPI installation runs the
environment's interpreter on the host is now measured before PyPI is admitted.

## What Changes

- Accept `env.provider = "pixi"` with **artifact-tier** guarantees on
  qualified native Linux/macOS profiles, through `pixi.toml` or a
  `pyproject.toml` containing `[tool.pixi]`.
- Add optional `[env.pixi]` with `manifest` and `environment`; use the
  workspace's `default` environment when none is selected, never the caller's
  shell.
- Require an up-to-date, unchanged `pixi.lock`. Materialize binary Conda
  packages. Admit locked PyPI wheels **only if** Group 0 shows that installing
  them executes nothing downloaded on the host; otherwise refuse environments
  that carry PyPI dependencies, by name, until materialization can be
  confined. Refuse source distributions, editable, path and VCS dependencies
  and build backends in every case.
- Run host-side pixi under an environment built from an allowlist, with an
  isolated `HOME`, against a staging workspace holding byte copies of the
  manifest and lock.
- Obtain activation as text, with pixi composing the order across features and
  targets, rewrite the staging root to the real project root, and run it once
  in confined provisioning.
- Materialize into immutable per-recipe objects under
  `~/.local/share/devcroft-artifacts/pixi/`, exposed read-only. A sandbox's
  recorded state is its reference; unreferenced objects are removed at `rm`
  and `up --recreate`.
- Supply the session shell and the Bash activation needs from the locked
  prefix or, failing that, from the narrowest host grant that works: files on
  Linux, the shell's directory on macOS. **No `/usr/bin` grant on either.**
- Report service supervision as unsupported. Pixi tasks are commands, not
  service declarations.

## Capabilities

### New Capabilities

None: this is another implementation of the provider contract.

### Modified Capabilities

- `config`: provider value, manifest selection and named environment selection.
- `env-provider`: locked installation, inert activation capture, read-only
  objects, staleness and the explicit lack of service supervision.
- `policy`: attributed prefix and host-runtime grants, the narrow shell grant,
  and refusal of a manifest write grant that overlaps the artifact store.
- `cli`: discovery, scoped doctor, guarantee and precise errors.

The main spec tree is empty, so these are provider-scoped **ADDED**
requirements. `add-mvp-core`'s "Only declarative providers" scenario names
mise, not pixi, so nothing there needs amending; the unit test and the
`NOT_YET_SUPPORTED` entry in `src/provider/validate.rs` do.

## Impact

Expected implementation touches `src/provider/{mod,validate,pixi}.rs`,
`src/config/{mod,validate}.rs`, `src/policy/capability_set.rs`, `src/shell.rs`,
`src/lifecycle/{up,status,state}.rs`, `src/bin/devcroft.rs`, tests and samples.

`[env.pixi]` is the first provider-specific sub-table; the existing precedent
is a flat key (`require_native_apple_evidence`) checked against the provider
in `src/config/validate.rs`. The selection has to reach three places that
today take only a provider name and a project root: `Provider::resolve`, and
the free functions `manifest_fingerprint` and `is_stale` in
`src/provider/mod.rs`. It is persisted in `Meta` so `status` and `policy
--render` use what `up` used.

Two limitations are stated rather than worked around:

- **The default `pyproject.toml` layout is refused.** `pixi init --format
  pyproject` installs the project itself as an editable path dependency, which
  means running its build backend. Such a workspace is recognized as Pixi and
  refused with that reason, not passed over as "no Pixi project here".
- **`pixi run` is not available inside a sandbox**, because pixi is not in
  it. A project's tasks are not runnable by name there.

The artifact store, the allowlisted subprocess environment and the narrow
shell grant are the same contract as `add-mise-provider`'s. They are extracted
when the second of the two lands, not before.

This proposal creates implementation work; it claims no provider, VM,
Asterinas or fleet qualification. See [design.md](design.md).
