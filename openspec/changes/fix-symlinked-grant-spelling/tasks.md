# Tasks: fix-symlinked-grant-spelling

## 0. Measurement gate

- [ ] 0.1 **Does `nono` canonicalize what it is handed?** Grant both
      spellings of one path and check whether the compiled profile
      carries two entries or one. If it collapses them, this fix belongs
      upstream and the rest of this list is wrong — stop and report.
- [ ] 0.2 Confirm the failure it fixes, on a real devenv sandbox: the
      generated preamble's `mkdir -p /tmp/devenv-<hash>` fails today, and
      the sandbox log shows it.
- [ ] 0.3 Measure how the socket path does it, since the library already
      emits both spellings there — the shape to match, and the evidence
      that both-spellings is the accepted answer rather than a new idea.

## 1. Implementation

- [ ] 1.1 `resolve_and_validate` returns the manifest's spelling and the
      canonical one, with the escape guard still running first
      (design.md decision 2).
- [ ] 1.2 `grant` emits both to `nono`, with the same origin
      (decision 3).
- [ ] 1.3 `resolved_grants` emits both to the mount view, through the
      same resolver — the two must not diverge.
- [ ] 1.4 Nothing is emitted twice where the spellings match.

## 2. Tests

- [ ] 2.1 Both spellings writable inside a real sandbox on macOS;
      self-skips elsewhere with its reason.
- [ ] 2.2 Linux-shaped manifest (no symlinks) compiles byte-identically —
      extend the existing golden rather than adding a parallel one.
- [ ] 2.3 **The escape guard still refuses**, and this is the test that
      must not pass by accident: an implementation that emitted spellings
      before validating would grant the escaping path.
- [ ] 2.4 A deny nested inside the newly-emitted spelling behaves as it
      does inside the existing one (`check_no_deny_overlaps_allow`).
- [ ] 2.5 `policy --render` shows both spellings, since a rule the
      backend receives that rendering cannot show breaks the policy
      invariant.

## 3. Documentation

- [ ] 3.1 `docs/known-gaps.md`: the entry moves to fixed, keeping the
      measurement. Both instances it names — the flox hook and devenv's
      preamble — are closed by this.
- [ ] 3.2 `add-devenv-provider`'s design decision 9 recorded this as
      deliberately not fixed there; point it here.
- [ ] 3.3 `docs/implementation-log.md`: what group 0 measured.

## 4. Verification

- [ ] 4.1 `cargo build`, `cargo clippy --all-targets`, `cargo fmt`,
      `cargo doc --no-deps` clean.
- [ ] 4.2 Full suite, skips reviewed.
- [ ] 4.3 `openspec validate --all`.
- [ ] 4.4 Re-run on Linux — this changes what every sandbox is granted,
      and the no-op claim for Linux is the half that cannot be checked on
      macOS.
