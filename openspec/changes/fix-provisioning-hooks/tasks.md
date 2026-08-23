## 1. Reproduce both defects as failing tests first

> Written before the fix, so each is confirmed to fail against the
> current code. A test that has never been red is not evidence.

- [x] 1.1 nix: an integration test with a flake whose devShell defines a
      `shellHook` writing a sentinel file, asserting after resolution
      that the file does **not** exist. **Confirmed red against the old
      mechanism**: running `nix develop --command sh -c 'env -0'` — the
      exact command `nix.rs` used — against the same fixture creates the
      sentinel, and `print-dev-env --json` does not
- [x] 1.2 flox: a test with a manifest defining `[hook].on-activate`
      writing a sentinel. **Asserts the honest thing rather than the
      drafted one**: flox cannot be stopped from running the hook, so
      the test asserts the hook *did* run and that resolution *reported*
      it — the fact `up` needs in order to warn. A third test covers the
      converse, since `flox init`'s stock manifest ships `[hook]` with
      `on-activate` commented out and a naive text search would warn on
      every new environment
- [x] 1.3 Equivalence fixture: a flake with **no** hook, whose captured
      environment is recorded before the change, so 2.x can be shown not
      to alter it

## 2. Fix nix

- [x] 2.1 Replace the `nix develop --command sh -c 'env -0'` capture in
      `nix.rs` with `nix print-dev-env --json`, keeping
      `--no-update-lock-file` and the absence of `--impure`
- [x] 2.2 Parse with `serde_json` into the existing env map: take
      variables whose `type` is `exported`; ignore `var` and `array`,
      and ignore `bashFunctions` — the previous mechanism could not see
      any of them, so ignoring them is what preserves behavior
      (design.md Non-Goals).
      **One thing the schema did not tell us, and the test did:**
      `value` cannot be typed `Option<String>`. `array` entries carry it
      as a JSON list, and `#[serde(default)]` covers a *missing* field,
      not one of the wrong shape — so the first array failed the entire
      parse. It is a `serde_json::Value`, matched on `String`
- [x] 2.3 Keep the canonical-baseline diff exactly as it is. The capture
      mechanism changes; what it is diffed against does not
- [x] 2.4 A malformed or unexpected JSON shape fails at layer `provider`
      with exit code 3, naming what could not be read — never falls back
      to the old mechanism, which would reintroduce the defect silently
- [x] 2.5 1.1 now passes: the hook does not run
- [x] 2.6 1.3 now passes: a hook-free flake captures the same
      environment as before, allowing for the shell bookkeeping the old
      mechanism picked up (`PWD`, `SHLVL`, `_`, `OLDPWD`) — assert the
      *absence* of those rather than tolerating an arbitrary diff

## 3. Detect and report the flox case

- [x] 3.1 Detect `[hook].on-activate` in `.flox/env/manifest.toml`,
      reusing the TOML parsing already in `flox.rs`
- [x] 3.2 Err toward warning: an unparsable or unexpectedly-shaped
      manifest warns rather than assuming safety (design.md decision 3).
      A false negative defeats the change; a false positive is noise
- [x] 3.3 Carry the fact out of resolution. `Resolution` is the provider
      contract, so this rides along with it rather than being discovered
      separately by `up` — and it is a fact about *this* resolution, not
      about the provider in general
- [x] 3.4 `up` prints exactly one warning naming the provider, the
      construct, and that its code ran on the host outside the sandbox.
      Not per session, not repeated, and `up`'s exit code is unchanged
- [x] 3.5 No warning when no hook is defined — asserted, since a warning
      that always fires is one users stop reading
- [x] 3.6 1.2 now passes

## 4. Correct the documentation that asserts the opposite

- [x] 4.1 CLAUDE.md's two-phase invariant: it currently says
      provisioning is "trusted because it executes pinned tooling from a
      lockfile, not project code". That is false for flox and was false
      for nix. State the real contract — pinned tooling plus whatever
      activation hook the environment defines, avoided where the
      provider allows and reported where it does not
- [x] 4.2 README Limitations: `devcroft up` on an untrusted repository
      executes that repository's activation hook on the host, for
      providers that give no way to avoid it. This belongs with the
      other published limitations rather than in a design document
- [x] 4.3 `docs/decisions.md` §1: criterion 4 says "capturable
      activation". Make explicit that it means capturable *without
      executing project code*, which is what it always meant and never
      said — and note which providers satisfy it how, since that is now
      measured for three of them
- [x] 4.4 `add-devbox-provider`: its `env-provider` delta specs this
      rule devbox-locally. Point it at the general requirement here
      instead of restating it, now that one exists

## 5. Verification

- [x] 5.1 `cargo build`, `cargo clippy`, `cargo fmt` clean (the one
      remaining clippy warning is the pre-existing `spike.rs` zombie
      lint, untouched by this change)
- [x] 5.2 `openspec validate --all` passes — 11/11
- [x] 5.3 Full suite green. All three tests in
      `tests/provisioning_runs_no_project_code.rs` **ran for real**
      rather than self-skipping: nix and flox are both usable in this
      devcontainer. One unrelated pre-existing flake
      (`lifecycle_hooks::up_recreate_reruns_post_create`) appeared once
      under parallel load and passed in isolation and on re-run — the
      same flakiness `add-flox-services` already records as present on
      the unmodified base
- [x] 5.4 Re-measured by hand through the real binary, which is how the
      defect was found in the first place:
      **flox with `[hook].on-activate`** — `up` succeeds, the hook still
      runs (`HOOK_RAN` created, as expected since flox cannot be stopped),
      and `up` prints exactly one warning naming the provider and what
      happened.
      **flox with the stock manifest** (whose `on-activate` is only a
      comment) — `up` succeeds and prints nothing, so the warning stays
      worth reading.
      **nix with a `shellHook`** — the old command
      (`nix develop --command sh -c 'env -0'`) creates the sentinel
      against the same fixture; `print-dev-env --json` does not. Red
      before, green after, measured rather than asserted
