# Change: fix-symlinked-grant-spelling

## Why

**A sandbox denies the spelling of a path it has itself granted.**
devcroft canonicalizes every filesystem grant before handing it to the
backend, so a manifest saying `/tmp/proj` compiles to `/private/tmp/proj`
and the sandbox then refuses `/tmp/proj`. On Linux this is invisible —
the paths involved are not symlinks. On macOS `/tmp` → `/private/tmp` and
`/var` → `/private/var` both are.

It has been published as a known gap since it was found, and the entry
recorded a flox `[hook].on-activate` writing to `$TMPDIR/...` as the case
that surfaced it. devenv's generated preamble looked like a second
instance of the same thing.

**Implementation measured that they are two different bugs wearing one
symptom**, and this change closes one of them. The flox case is a grant
*devcroft emits* — a project path under `$TMPDIR`, granted by the
manifest and spelled one way, compiled another. devenv's `mkdir -p
/tmp/devenv-<hash>` is `/tmp` itself, which no manifest grants and which
comes from the backend's own baseline. Same symptom, different grant
source, and only the first is devcroft's to spell.

The fix is known and small, and the same library already does it
elsewhere: unix-socket grants emit both the original and the resolved
spelling so `/tmp/x.sock` and `/private/tmp/x.sock` both match.
Filesystem grants emit only the resolved form.

## What Changes

- **A filesystem grant emits both spellings when they differ.** Where a
  manifest entry's pre-canonicalization path is not the same string as
  its canonical target, both are granted. Where they are the same — every
  grant on Linux, and most on macOS — nothing changes.
- **This is not a widening, and the change says why.** The two spellings
  name the *same* target: `/tmp/proj` and `/private/tmp/proj` are one
  directory reached two ways. What would be a widening is granting a
  symlink whose target lies elsewhere, and the existing symlink-escape
  guard already refuses that for project-relative entries — it runs
  before this, and this change must not weaken it.
- **The mount view gets the same treatment**, through the same resolver.
  `resolved_grants` and `to_capability_set` share `resolve_and_validate`
  precisely so Landlock's grants and the mount view cannot drift, and a
  fix applied to one and not the other would reintroduce exactly that.
- **`docs/known-gaps.md`'s entry moves from open to fixed**, keeping the
  measurement that made it concrete.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `policy`: a filesystem grant covers the path as the manifest spells it
  *and* as it resolves, where those differ. The rule that a grant's
  origin is inspectable and that nothing reaches the backend which
  `policy --render` cannot show is unchanged and constrains how this is
  implemented.

## Impact

- **Affected specs**: `policy`.
- **Affected code**: `src/policy/capability_set.rs` —
  `resolve_and_validate` and its two callers, `grant` (which feeds
  `nono`) and `CapabilityPlan::resolved_grants` (which feeds the mount
  view).
- **This touches what every sandbox on the host gets**, which is why
  `own-policy-baseline` was named as its owner rather than the change
  that found it. A grant set that grows by one entry per symlinked path
  changes `policy --render` output for real manifests, so the rendering
  and the compiled set have to stay congruent — the policy invariant is
  that nothing reaches the backend which `--render` cannot show.
- **Linux is unaffected in practice and that must be asserted, not
  assumed.** If the canonical and lexical spellings match, no second
  grant is emitted, so a Linux manifest compiles byte-identically. That
  is a regression test, not a claim.

## Success Criteria

- A sandbox granted a project at `/tmp/<name>` can write to both
  `/tmp/<name>/f` and `/private/tmp/<name>/f` on macOS.
- ~~devenv's generated preamble no longer fails on `mkdir -p
  /tmp/devenv-<hash>`.~~ **Measured false during implementation, and the
  criterion was wrong rather than unmet.** That path is not a manifest
  grant — it is `/tmp` itself, which comes from the backend's own
  baseline. This change fixes the spellings of grants *devcroft* emits;
  the baseline's spellings are not devcroft's to emit. See design.md
  decision 4.
- A manifest whose grants involve no symlinks compiles to a
  byte-identical policy — the existing golden test extended to cover it.
- The symlink-escape guard still refuses a project-relative entry whose
  target leaves the project root. This is the test that must not pass by
  accident: a change that emitted both spellings *before* the guard ran
  would grant the escaping symlink's own path.
- `policy --render` shows both spellings where both are granted, since a
  rule the backend receives that `--render` cannot show breaks the policy
  invariant.

## Open Questions

- **Whether to emit both spellings or teach the backend the alias.**
  Emitting both is devcroft-side and immediate; asking the library to
  resolve aliases the way it already does for sockets is upstream work
  with a longer tail. The first is proposed here; the second is worth
  raising regardless, because every consumer of that library has this
  bug.
- **How many spellings deep this can go.** `/tmp/x` → `/private/tmp/x` is
  one hop. A grant reached through two symlinked ancestors would have
  more, and whether to emit every intermediate spelling or only the
  manifest's own and the fully-resolved one is a decision this change
  must make rather than discover.
