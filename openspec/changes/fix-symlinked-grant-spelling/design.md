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

## Measured (group 0)

**The backend already does this, and devcroft is what prevents it.**
nono 0.74.0's macOS emission (`src/sandbox/macos.rs`,
`path_filters_for_cap`) carries the comment:

> If the original path differs (e.g. `/tmp` vs `/private/tmp`), emit a
> rule for the original too so Seatbelt allows traversing the symlink.

and does exactly that — `FsCapability` stores `original` and `resolved`
separately, and the macOS backend emits a `subpath`/`literal` rule for
each when they differ.

The branch never fires for devcroft, because `resolve_and_validate`
canonicalizes **before** calling `allow_path`. nono therefore receives
the already-canonical path as `original`, `original == resolved`, and the
dual emission is skipped. devcroft destroys the manifest's own spelling
one step before the library that wanted it.

`allow_path` canonicalizes internally anyway, and its own comment says
why that is safe: *"Canonicalize first — this atomically resolves
symlinks and verifies existence. No separate exists() check needed,
eliminating TOCTOU window."* So handing it the un-canonical path loses
nothing.

**This replaces decision 1.** The change is not "emit two grants" — it is
"stop canonicalizing before the call". Smaller, and it uses a mechanism
that is already tested upstream rather than adding a parallel one.

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

### Decision 1: hand the backend the manifest's spelling, and let it do what it already does

~~Emit both spellings from devcroft.~~ Replaced by the group-0
measurement above: nono already emits both when they differ, and only
fails to because devcroft canonicalizes first.

So `grant` passes the resolved-but-not-canonicalized absolute path to
`allow_path`/`allow_file`. nono canonicalizes it itself — atomically, no
TOCTOU window, by its own documented design — keeps the manifest's
spelling as `original`, and its macOS backend emits a rule for each.

How many spellings deep this goes stops being devcroft's question too:
the library keeps one `original` and one `resolved`, so the answer is
whatever it already does for every other consumer, rather than a policy
devcroft invents at this call site.

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

### Decision 4: this closes the grants devcroft emits, and says what it does not

Measured A/B on one sandbox, recreating it with and without the change,
against a manifest granting `/tmp/dcspell-probe`:

| spelling | before | after |
|---|---|---|
| `/tmp/dcspell-probe` (as the manifest writes it) | **denied** | granted |
| `/private/tmp/dcspell-probe` (canonical) | granted | granted |

So the fix works, for grants devcroft emits.

**It does not fix devenv's preamble, and the proposal was wrong to say it
would.** `mkdir -p /tmp/devenv-<hash>` fails on `/tmp` itself, which no
manifest grants — it comes from the backend's baseline group set.
devcroft does not choose those paths and cannot respell them here.
Measured after the fix: the preamble still fails, identically.

Recorded rather than quietly narrowing the claim, because a fix that
closes half a published gap and is written up as closing all of it is
worse than one that closes half and says so. `docs/known-gaps.md` keeps
the entry open for the baseline half, with the two cases now
distinguished.

The baseline half is `own-policy-baseline`'s, or upstream's: the library
already emits both spellings for capabilities it is *given*, so the
question is why its own baseline grants only one — which is a question
for the library, not for this call site.

## Risks / Trade-offs

- **The grant set grows for real macOS manifests**, so a golden
  comparison against a recorded policy changes → intended, and the
  Linux-unchanged case is asserted separately so the two cannot be
  confused.
- ~~`nono` may itself canonicalize what it is handed.~~ It does, and
  that turned out to be the point: it keeps the pre-canonical path as
  `original` and emits a rule for it. Measured, group 0.
- **Two grants where there was one may trip `check_no_deny_overlaps_allow`**
  in a manifest that previously compiled → covered by a test, since a
  deny nested inside the *new* spelling is the case that would surface
  it.

## Migration Plan

None. Additive per-grant, and a no-op for any manifest whose paths
canonicalize to themselves.

## Open Questions

- ~~Whether `nono` collapses the spellings itself.~~ Answered in group 0:
  it keeps both and emits both. The remaining question is **Linux**,
  where the emission path is Landlock rather than Seatbelt and the
  dual-rule branch measured above is macOS-specific. If Landlock resolves
  symlinks at enforcement time the fix is a no-op there, which is the
  outcome to expect but not to assume.
