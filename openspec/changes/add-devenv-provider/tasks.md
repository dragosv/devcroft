# Tasks: add-devenv-provider

Ordered so every claim this change rests on is measured before code
depends on it. Group 0 exists because `add-devbox-provider` shipped a
proposal written entirely from documentation and had to correct itself
twice during implementation; the entry-point table in proposal.md is
already measured, but the *completeness* of what that entry point returns
is not.

devenv is not installed in this repo's devcontainer. It is reachable via
`nix run nixpkgs#devenv` (2.2.2 measured). Every test added here guards
on the **capability**, never on the binary — `devenv --version` succeeds
with an unreachable Nix store, and `provider::host_can_build_nix_closures()`
is the shared probe.

## 0. Measurement gate — no code until these are answered

- [ ] 0.1 Re-confirm the proposal's entry-point table against a fresh
      devenv project, using the sentinel method (an `enterShell` that
      appends to a file outside the project root, counted before and
      after each invocation). Record the devenv version measured.
- [ ] 0.2 **Diff the captured environment against the truth.** Capture
      via `devenv build shell`, capture via `devenv shell -- env -0` (the
      hook-running route), and diff. Any variable present in the second
      and absent from the first is a hole in the chosen route. Record the
      diff verbatim — an empty diff is the result this change assumes,
      and it has not been verified.
- [ ] 0.3 Decide where `enterShell`'s text comes from: the
      `…-devenv-enterShell` derivation file, or `devenv eval`. Measure
      both across at least two devenv versions if reachable; pick on
      stability, record the loser and why (design.md Open Questions).
- [ ] 0.4 Check whether `devenv.yaml` inputs can resolve at `up` without
      `devenv.lock` changing — the failure devbox's base nixpkgs entry
      turned out to have. If they can, the byte-comparison precondition
      is necessary but insufficient and the spec needs amending before
      implementation, not after.
- [ ] 0.5 Investigate why `devenv shell -- <cmd>` runs `enterShell`
      twice. Not on the chosen path, so this does not block; record the
      finding either way, since an unexplained doubling at the hook
      boundary is the kind of detail that matters later.
- [ ] 0.6 If 0.2 shows the hook-free route is incomplete and no other
      hook-free route is complete, **stop and report**. That outcome
      means devenv fails criterion 4 as measured, which is a
      qualification finding for `docs/decisions.md`, not a problem to
      engineer around.

## 1. Provider skeleton

- [ ] 1.1 `ProviderKind::Devenv` in `src/provider/mod.rs`: dispatch,
      `static_name` (`"devenv"`), `manifest_fingerprint` over
      `devenv.nix` + `devenv.yaml` + `devenv.lock`.
- [ ] 1.2 Move `devenv` out of `NOT_YET_SUPPORTED` into `SUPPORTED` in
      `src/provider/validate.rs`; no alias is registered (config spec).
- [ ] 1.3 Unit tests: canonical name accepted, near-miss (`dev-env`)
      rejected rather than normalized, default still `flox`.
- [ ] 1.4 Regression test: a manifest not naming `devenv` compiles to a
      byte-identical policy (config spec's last scenario).

## 2. Resolution

- [ ] 2.1 `src/provider/devenv.rs`: preconditions — `devenv` usable,
      `nix` usable (reported as devenv's own requirement, not as advice
      to switch providers), `devenv.nix` present with a `devenv init`
      hint, all at layer `provider`, exit 3.
- [ ] 2.2 Capture through the hook-free route chosen in group 0, diffed
      against the shared fixed baseline. Reuse `capture`'s existing
      machinery; if it needs changing, that is a finding about the trait
      generalizing and gets recorded rather than absorbed (proposal's
      last success criterion).
- [ ] 2.3 Parse the `declare -x` output. **Fail loudly on anything
      unrecognized** rather than skipping it — a partial environment that
      looks like a whole one is the failure mode design.md decision 2
      accepts the internal-artifact risk to avoid.
- [ ] 2.4 Store grants from the resolved closure, annotated
      `provider:devenv`; assert provider resolution adds no write grants.
- [ ] 2.5 Lockfile precondition: byte-compare `devenv.lock` after
      capture; on mismatch restore the original (or delete one capture
      created), then fail at layer `provider`, exit 3.
- [ ] 2.6 `ServiceSupport::Unsupported`, declared explicitly with the
      reasoning inline, the way `nix.rs` does — so a devenv project
      declaring services fails distinguishably rather than silently
      starting nothing.
- [ ] 2.7 Determinism test: capture twice from shells with different
      `PATH`/environment, assert byte-identical diffs
      (`tests/flox_env_capture_is_deterministic.rs` has the shape).

## 3. The hook

- [ ] 3.1 Populate `Resolution::activation_script` from `enterShell`,
      via the source chosen in 0.3.
- [ ] 3.2 `ran_activation_hook` stays false, and `up` prints no
      host-side-hook warning for a devenv project that defines one
      (env-provider spec). Test asserts the absence, not just the
      presence of the right value.
- [ ] 3.3 **The sentinel test**: a project whose `enterShell` writes
      outside the project root leaves that file untouched across `up`,
      and written exactly once after the sandbox has run it. This is the
      criterion-4 guarantee expressed as a test rather than a claim.
- [ ] 3.4 A hook invoking a policy-denied binary fails the hook and fails
      `up` at layer `keeper`, naming it — not a sandbox that comes up as
      though it succeeded.

## 4. Staleness, CLI surface

- [ ] 4.1 Staleness over all three files; test that editing each one
      independently flips `status` to stale, and that editing an
      unrelated file does not.
- [ ] 4.2 `doctor`: devenv check scoped to projects declaring it,
      including the "devenv present, Nix is not" scenario.
- [ ] 4.3 `init`: detect `devenv.nix`; insert devenv into the documented
      detection order (flox → devbox → devenv → bare flake); name the
      alternatives found in one line. Advise `devenv update` when
      `devenv.lock` is absent — verify in group 0 that this is the
      command that writes it, rather than assuming from the name.
- [ ] 4.4 `src/bin/devcroft.rs`'s `USAGE` needs no new command, but
      confirm `tests/cli_help_and_version.rs` still passes — the surface
      is closed and this change does not widen it.

## 5. End to end

- [ ] 5.1 `samples/devenv-sample/`: a real devenv project with a
      lockfile, an `enterShell`, and its own `README.md` stating what it
      demonstrates. No `[workspace]` exclusion needed unless it is a Rust
      project — see CLAUDE.md's samples note.
- [ ] 5.2 E2E: `up`, `exec` sees the devenv toolchain, everything runs
      under `network.default = "deny"`.
- [ ] 5.3 Closure-tier measurement, the one that makes "closure tier" a
      claim about *this provider* rather than about Nix in the abstract:
      a full build inside the sandbox needs the project root, `/tmp` and
      the store, with `/usr/bin/gcc` denied.
- [ ] 5.4 Format-pin test for the capture artifact, so an upstream change
      to `devenv build shell`'s output breaks CI rather than a user's
      sandbox (design.md decision 2's second obligation).

## 6. Documentation

- [ ] 6.1 `docs/decisions.md` §1: replace the "Not yet built: devenv"
      entry with the measured outcome, including the internal-artifact
      dependency decision 2 takes on (its third obligation).
- [ ] 6.2 `openspec/config.yaml`: devenv moves out of "qualified but
      unscheduled". While there, fix that list's criterion-4 wording,
      which still says "env-diff capturable activation" and predates
      `fix-provisioning-hooks` adding "without executing project code" —
      the same stale phrasing this change's proposal had to flag.
- [ ] 6.3 `README.md`'s Environments section: devenv moves from "next,
      and only unbuilt" to supported; the provider count changes.
- [ ] 6.4 `docs/roadmap.md`: drop the devenv paragraph from 0.5.
- [ ] 6.5 `CLAUDE.md`: devenv in the provider list and the samples note.
- [ ] 6.6 `docs/implementation-log.md`: what group 0 measured, and
      anything the proposal got wrong — especially if 0.2's diff was not
      empty.

## 7. Verification

- [ ] 7.1 `cargo build`, `cargo clippy --all-targets`, `cargo fmt`,
      `cargo doc --no-deps` all clean.
- [ ] 7.2 `cargo test -- --nocapture 2>&1 | grep skipping` reviewed — a
      green run that skipped every devenv test is not a passing run.
- [ ] 7.3 `openspec validate --all` passes.
- [ ] 7.4 `cargo package --list` includes any new source file; the
      anchored `include` allowlist in `Cargo.toml` does not silently drop
      it (CLAUDE.md's packaging note).
