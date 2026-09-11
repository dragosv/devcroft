## Context

`resolve_and_validate` in `src/policy/capability_set.rs` turns one
manifest entry into one canonical `PathBuf`, and two callers consume it:
`grant`, which hands it to `nono`, and `CapabilityPlan::resolved_grants`,
which hands it to the mount view. They share that resolver deliberately —
a view narrower than Landlock's grants breaks the sandbox, a wider one
reintroduces what `add-mount-isolation` removed.

Canonicalization is also load-bearing for the symlink-escape guard, which
compares the *canonical* target against the canonical project root. That
guard is why a project-relative `credential-link -> ~/.ssh` is refused
rather than silently granting `~/.ssh`.

So the fix has to add a spelling without disturbing either property.

## Goals / Non-Goals

**Goals:**

- A grant works under the spelling the manifest uses and the one it
  resolves to.
- One resolver, so Landlock and the mount view cannot drift.
- No change at all where no symlink is involved.

**Non-Goals:**

- Teaching the backend library to resolve aliases itself. Worth raising
  upstream — every consumer has this bug — but not this change.
- Anything about *which* paths are granted. This changes spellings, not
  the grant set's reach.

## Decisions

### Decision 1: emit the manifest's own spelling and the fully-resolved one, and nothing in between

A path reached through two symlinked ancestors has more than two
spellings. Emitting every intermediate is unbounded work for a case
nobody has hit; emitting only the resolved form is today's bug.

So: exactly two, when they differ. The manifest's spelling is what a
reviewer reads and what project code is most likely to use; the resolved
form is what the kernel sees. The intermediate spellings are reachable
only by a path expression nobody writes by hand, and if one ever turns
up it will turn up as a bug report with a concrete path rather than as a
guess here.

### Decision 2: the escape guard runs first, unchanged

Order matters more than it looks. The guard refuses a project-relative
entry whose canonical target escapes the root — and it must run *before*
any spelling is emitted, or the escaping symlink's own path would be
granted on the way to being refused.

Stated as a decision because the natural implementation (resolve, collect
spellings, then validate) gets it backwards, and the resulting bug would
grant exactly what the guard exists to refuse.

### Decision 3: both spellings carry the same origin

`policy --render` must show both, and both must be attributable. A second
spelling is not a second rule from a different source — it is the same
manifest entry, rendered twice because the filesystem has two names for
it. Giving it the origin of the entry it came from keeps `why` honest.

## Risks / Trade-offs

- **The grant set grows for real macOS manifests**, so a golden
  comparison against a recorded policy changes → intended, and the
  Linux-unchanged case is asserted separately so the two cannot be
  confused.
- **`nono` may itself canonicalize what it is handed**, collapsing the
  second spelling back to the first → measured before relying on it; if
  it does, the fix belongs upstream rather than here, and that is a
  finding rather than a workaround to invent.
- **Two grants where there was one may trip `check_no_deny_overlaps_allow`**
  in a manifest that previously compiled → covered by a test, since a
  deny nested inside the *new* spelling is the case that would surface
  it.

## Migration Plan

None. Additive per-grant, and a no-op for any manifest whose paths
canonicalize to themselves.

## Open Questions

- Whether `nono` collapses the spellings itself (see Risks) — this is the
  first thing the task list measures, because if it does, everything
  below it changes.
