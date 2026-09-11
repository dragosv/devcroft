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

- [x] 0.1 Entry-point table re-confirmed on **devenv 2.2.2,
      aarch64-darwin**, sentinel method, clean canonical baseline. Holds:
      `build shell`, `info`, `eval <attr>` run the hook 0 times;
      `direnv-export` once (80,256 bytes); `shell -- <cmd>` twice. Full
      table in design.md — Measured (group 0).
- [x] 0.2 **Completeness diff taken, and it is not empty in either
      direction** — 103 keys from `build shell` against 69 from
      `devenv shell -- env -0`. 38 extra (the Nix builder's own
      variables, including `HOME=/homeless-shelter` and
      `SSL_CERT_FILE=/no-cert-file.crt`), 6 missing, of which the two
      that matter (`IN_NIX_SHELL`, `MANPATH`) are exported by
      `enterShell` itself and so come back when the sandbox runs it.
      Recorded verbatim in design.md decision 8, which is the answer.
- [x] 0.3 `enterShell`'s text comes from **`devenv eval enterShell`** —
      returns it as JSON, runs it 0 times, 3,896 bytes. The
      `…-devenv-enterShell` derivation exists and is readable; rejected
      because it means finding a store path by name pattern where a
      documented command exists (design.md decision 3).
- [x] 0.4 **`devenv build shell` writes `devenv.lock` when the project
      has none** — it resolves and locks rather than refusing. So the
      byte comparison after capture is necessary but not sufficient: the
      up-front precondition is what stops the first `up` of an unlocked
      project from resolving inputs at `up`.
- [x] 0.5 The captured `PATH` carries a store-backed shell —
      `…-bash-interactive-5.3p15/bin/{sh,bash}` and `…-bash-5.3p15/bin/…`,
      all resolving inside `/nix/store`. `src/shell.rs`'s rule is
      satisfiable for devenv without the requisites fallback.
- [x] 0.6 With `devenv.lock` present, a second capture modifies exactly
      one path in the project tree: `.devenv/nix-eval-cache.db`. Nothing
      outside `.devenv/`, and no declaration file. Decision 6 measured.
- [x] 0.7 **`devenv update`** creates a missing `devenv.lock`, runs no
      hook, and is what `init` advises.
- [x] 0.8 `devenv shell -- <cmd>` runs `enterShell` twice under a clean
      environment too, so it is not an artifact of the measuring shell.
      Still unexplained; not on the chosen path. Recorded.
- [x] 0.9 Gate passed. The hook-free route is not incomplete — it is a
      superset with a different `HOME`, and what it lacks is restored by
      the hook devcroft already runs inside the sandbox. Criterion 4
      holds; the change proceeds with decision 8 added.

## 1. Provider skeleton

- [x] 1.1 `ProviderKind::Devenv` in `src/provider/mod.rs`: dispatch,
      `static_name` (`"devenv"`), `manifest_fingerprint` over
      `devenv.nix` + `devenv.yaml` + `devenv.lock`.
- [x] 1.2 Move `devenv` out of `NOT_YET_SUPPORTED` into `SUPPORTED` in
      `src/provider/validate.rs`; no alias is registered (config spec).
- [x] 1.3 Unit tests: canonical name accepted, near-miss (`dev-env`)
      rejected rather than normalized, default still `flox`.
- [x] 1.4 Regression test: a manifest not naming `devenv` compiles to a
      byte-identical policy (config spec's last scenario).

## 2. Resolution

- [x] 2.1 `src/provider/devenv.rs`: preconditions — `devenv` usable,
      `nix` usable (reported as devenv's own requirement, not as advice
      to switch providers), `devenv.nix` present with a `devenv init`
      hint, and `devenv.lock` present **before** capture with the lock
      command from 0.7 as the hint — all at layer `provider`, exit 3.
      The last one mirrors devbox's `ensure_everything_locked`: without
      it an unlocked project fails through 2.7's after-the-fact byte
      comparison, whose message is about capture resolving rather than
      about the project never having been locked.
- [x] 2.2 Capture through the hook-free route chosen in group 0, diffed
      against the shared fixed baseline. Reuse `capture`'s existing
      machinery; if it needs changing, that is a finding about the trait
      generalizing and gets recorded rather than absorbed (proposal's
      last success criterion).
- [x] 2.3 Parse the `declare -x` output. **Fail loudly on anything
      unrecognized** rather than skipping it — a partial environment that
      looks like a whole one is the failure mode design.md decision 2
      accepts the internal-artifact risk to avoid.
- [x] 2.4 **Filter the Nix builder's own variables out of the capture**
      (design.md decision 8). `HOME=/homeless-shelter`,
      `SSL_CERT_FILE=/no-cert-file.crt`, the `TMP*` quartet pointing into
      a build directory, `out`/`stdenv`/`buildInputs`/`shellHook` and the
      rest. Not devenv's own `unset` list: it runs inside the hook's own
      shell and never reaches the environment the keeper injects, and it
      omits `HOME` and the certificate paths anyway.
- [x] 2.5 Equivalence test for that filter, on a host that can run both
      routes: filtered hook-free capture, plus what the hook exports,
      equals `devenv shell -- env -0` modulo an enumerated set written
      into the test. A key on neither side of that comparison fails CI —
      the only way the list stays correct across devenv versions.
- [x] 2.6 Store grants from the resolved closure, annotated
      `provider:devenv`; assert provider resolution adds no write grants.
- [x] 2.7 Lockfile precondition: byte-compare `devenv.lock` after
      capture; on mismatch restore the original (or delete one capture
      created), then fail at layer `provider`, exit 3.
- [x] 2.8 `ServiceSupport::Unsupported`, declared explicitly with the
      reasoning inline, the way `nix.rs` does — so a devenv project
      declaring services fails distinguishably rather than silently
      starting nothing.
- [x] 2.9 Working-tree test (design.md decision 6): after a capture —
      successful and refused — every created or modified path in the
      project tree is under `.devenv/`, and `devenv.nix`, `devenv.yaml`
      and `devenv.lock` are byte-identical to their pre-`up` content.
      devcroft does not delete `.devenv/`; it is devenv's own cache and
      GC roots.
- [x] 2.10 Determinism test: capture twice from shells with different
      `PATH`/environment, assert byte-identical diffs
      (`tests/flox_env_capture_is_deterministic.rs` has the shape).

## 3. The hook

- [x] 3.1 Populate `Resolution::activation_script` from
      `devenv eval enterShell` (0.3). What comes back wraps the project's
      block in devenv's own preamble; devcroft runs the whole thing, and
      the test asserts the project's own text is present in it rather
      than assuming the two are equal.
- [x] 3.2 `ran_activation_hook` stays false, and `up` prints no
      host-side-hook warning for a devenv project that defines one
      (env-provider spec). Test asserts the absence, not just the
      presence of the right value.
- [x] 3.3 **The sentinel test**: a project whose `enterShell` writes
      outside the project root leaves that file untouched across `up`,
      and written exactly once after the sandbox has run it. This is the
      criterion-4 guarantee expressed as a test rather than a claim.
- [x] 3.4 A hook invoking a policy-denied binary fails the hook and fails
      `up` at layer `keeper`, naming it — not a sandbox that comes up as
      though it succeeded.

## 4. Staleness, CLI surface

- [x] 4.1 Staleness over all three files; test that editing each one
      independently flips `status` to stale, and that editing an
      unrelated file does not.
- [x] 4.2 `doctor`: devenv check scoped to projects declaring it,
      including the "devenv present, Nix is not" scenario.
- [x] 4.3 `init`: detect `devenv.nix`; insert devenv into the documented
      detection order (flox → devbox → devenv → bare flake); name the
      alternatives found in one line. When `devenv.lock` is absent,
      advise the lock command **measured in 0.7** — the spec states the
      property ("the command that creates it") precisely so this task
      supplies the name rather than the spec asserting it unmeasured.
- [x] 4.4 `src/bin/devcroft.rs`'s `USAGE` needs no new command, but
      confirm `tests/cli_help_and_version.rs` still passes — the surface
      is closed and this change does not widen it.

## 5. End to end

- [x] 5.1 `samples/devenv-sample/`: a real devenv project with a
      lockfile, an `enterShell`, and its own `README.md` stating what it
      demonstrates. No `[workspace]` exclusion needed unless it is a Rust
      project — see CLAUDE.md's samples note.
- [x] 5.2 E2E: `up`, `exec` sees the devenv toolchain, everything runs
      under `network.default = "deny"`. Written against the same
      scaffolding the devbox e2e tests use; if `add-test-runtime-fixture`
      has landed its row contract by then, devenv is a row rather than a
      bespoke setup, and that choice is stated in the test rather than
      left to whoever reads it next.
- [x] 5.5 Login-session E2E (design.md decision 7): `Meta.shell` records
      an absolute shell from the devenv closure, `devcroft shell` opens,
      and an SSH login session works. `exec` passing proves nothing here
      — it does not go through the resolved shell.
- [x] 5.3 Closure-tier measurement, the one that makes "closure tier" a
      claim about *this provider* rather than about Nix in the abstract:
      a full build inside the sandbox needs the project root, `/tmp` and
      the store, with `/usr/bin/gcc` denied. Written and **Linux-gated**:
      macOS can make neither half, since it executes host binaries at
      ungranted paths and denies `/tmp`'s symlinked spelling (both in
      `docs/known-gaps.md`), so a version that passed there would be
      asserting the platform's gaps. It skips loudly on macOS; 5b.2 is
      where it actually runs.
- [x] 5.4 Format-pin test for the capture artifact, so an upstream change
      to `devenv build shell`'s output breaks CI rather than a user's
      sandbox (design.md decision 2's second obligation).

## 5b. Found during implementation

- [x] 5b.1 devenv's generated preamble writes to `/tmp`, which a macOS
      sandbox denies (it grants the canonical `/private/tmp`). Recorded in
      `docs/known-gaps.md` as a second instance of the existing
      symlinked-grant entry, and in design.md decision 9. **Not fixed
      here**: the fix is dual-spelling grants, which belongs to
      `own-policy-baseline` because it changes every sandbox on the host.
- [ ] 5b.2 Re-run the full e2e suite on **Linux**, where that gap does not
      exist and where task 5.3's `/usr/bin/gcc`-denied measurement is
      meaningful — macOS cannot make it, since host binaries execute at
      ungranted paths there (`docs/known-gaps.md`). Everything in this
      change was measured on aarch64-darwin.

## 6. Documentation

- [x] 6.1 `docs/decisions.md` §1: replace the "Not yet built: devenv"
      entry with the measured outcome, including the internal-artifact
      dependency decision 2 takes on (its third obligation).
- [x] 6.2 `openspec/config.yaml`: devenv moves out of "qualified but
      unscheduled". While there, fix that list's criterion-4 wording,
      which still says "env-diff capturable activation" and predates
      `fix-provisioning-hooks` adding "without executing project code" —
      the same stale phrasing this change's proposal had to flag.
- [x] 6.3 `README.md`'s Environments section: devenv moves from "next,
      and only unbuilt" to supported; the provider count changes.
- [x] 6.4 `docs/roadmap.md`: drop the devenv paragraph from 0.5.
- [x] 6.5 `CLAUDE.md`: devenv in the provider list and the samples note.
- [x] 6.6 `docs/implementation-log.md`: what group 0 measured, and
      anything the proposal got wrong — especially if 0.2's diff was not
      empty.

## 7. Verification

- [x] 7.1 `cargo build`, `cargo clippy --all-targets`, `cargo fmt`,
      `cargo doc --no-deps` all clean.
- [x] 7.2 `cargo test -- --nocapture 2>&1 | grep skipping` reviewed — a
      green run that skipped every devenv test is not a passing run. On
      aarch64-darwin with devenv on `PATH`: **395 passed, 0 failed**, and
      every devenv test ran except the Linux-gated closure-tier
      measurement (5.3), which skips with its reason. The skips in that
      run are unprivileged namespaces, loopback aliases and that one —
      all platform, none devenv.
- [x] 7.3 `openspec validate --all` passes.
- [x] 7.4 `cargo package --list` includes any new source file; the
      anchored `include` allowlist in `Cargo.toml` does not silently drop
      it (CLAUDE.md's packaging note).
