# Tasks: fix-symlinked-grant-spelling

## 0. Measurement gate

- [x] 0.1 **`nono` does canonicalize what it is handed — and that is the
      fix, not the obstacle.** `FsCapability::new_dir` stores the
      pre-canonical path as `original` alongside `resolved`, and nono's
      macOS backend (`path_filters_for_cap`) emits a rule for each when
      they differ, with a comment naming `/tmp` vs `/private/tmp`
      explicitly. devcroft canonicalizes *before* the call, so
      `original == resolved` and the branch never fires. Design decision
      1 replaced.
- [x] 0.2 The failure it fixes is already recorded: devenv's generated
      preamble fails on `mkdir -p /tmp/devenv-<hash>` inside a macOS
      sandbox, in the sandbox log, in every devenv project
      (`add-devenv-provider` design decision 9).
- [x] 0.3 The socket path does it through the same `original`/`resolved`
      pair — so this is one mechanism the library applies to two
      capability kinds, and devcroft was defeating it for one of them.

## 1. Implementation

- [x] 1.1 `resolve_and_validate` returns both the resolved-but-not-
      canonicalized path and the canonical one, with the escape guard
      still running first on the canonical form (design.md decision 2).
- [x] 1.2 `grant` hands `nono` the **non-canonical** path, so the
      library keeps the manifest's spelling as `original` and emits a
      rule for it.
- [x] 1.3 `resolved_grants` keeps handing the mount view the canonical
      path — it needs the real target — through the same resolver, so
      the two cannot drift.
- [x] 1.4 Where the spellings match, nothing changes: nono's own branch
      is what skips the second rule, so this is a property of the library
      rather than a condition devcroft writes.

## 2. Tests

- [x] 2.1 Both spellings writable inside a real sandbox on macOS,
      measured A/B by recreating one sandbox with and without the change:
      before, the manifest's own spelling was denied; after, both work.
      Asserted in CI at the resolver level instead of through a live
      sandbox — the integration form needs a provider environment and
      skipped on every host that lacks one, which is the failure mode
      this repo's testing rule exists to prevent.
- [x] 2.1a **The fix closes grants devcroft emits, not the baseline's.**
      devenv's preamble still fails on `/tmp`, because `/tmp` is not a
      manifest grant. The proposal's success criterion claiming otherwise
      was wrong and is struck (design.md decision 4).
- [x] 2.2 The no-symlink case is asserted by the resolver test's second
      half — with no symlink on the path the two spellings are one — which
      is exactly the Linux case. The policy golden is untouched because
      `compile` does no path resolution; that happens in
      `to_capability_set`, below it.
- [x] 2.3 **The escape guard still refuses** — the existing
      `project_relative_symlink_escaping_the_project_root_is_rejected`
      covers it and still passes, which is the point: the guard runs on
      the canonical form before anything is emitted.
- [x] 2.4 Void as written, and recorded rather than ticked silently:
      devcroft emits **one** grant and the library expands it, so
      `check_no_deny_overlaps_allow` — which compares manifest strings —
      sees exactly what it saw before. The risk existed only for the
      emit-two-grants design that group 0 replaced.
- [x] 2.5 **Re-read against the invariant, not just ticked.** The
      backend now receives two rules where `--render` shows one entry,
      which looks like the policy invariant's "nothing goes to the
      backend that `--render` cannot show". It is not a violation: both
      rules are the same grant under the same origin, naming one target
      by its two names — the same relationship `--render` already has
      with a granted directory and the files beneath it. What would
      violate it is a *different* path reaching the backend, and none
      does. Recorded because the reading is not self-evident.

## 3. Documentation

- [x] 3.1 `docs/known-gaps.md`: the entry moves to fixed, keeping the
      measurement. Both instances it names — the flox hook and devenv's
      preamble — are closed by this.
- [x] 3.2 `add-devenv-provider`'s design decision 9 recorded this as
      deliberately not fixed there; point it here.
- [x] 3.3 `docs/implementation-log.md`: what group 0 measured.

## 4. Verification

- [x] 4.1 `cargo build`, `cargo clippy --all-targets`, `cargo fmt`,
      `cargo doc --no-deps` clean.
- [x] 4.2 Full suite with devenv on PATH: **488 passed, 0 failed**.
- [x] 4.3 `openspec validate --all`.
- [ ] 4.4 Re-run on Linux — this changes what every sandbox is granted,
      and the no-op claim for Linux is the half that cannot be checked on
      macOS.
