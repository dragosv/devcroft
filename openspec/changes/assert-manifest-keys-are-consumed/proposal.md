## Why

**Three manifest keys parse, validate, produce good typo suggestions — and do
nothing.** Found one after another while implementing something else
(`own-sandbox-environment`), which is the point: nobody was looking for them,
and nothing would have found them.

| key | state before | found by |
|---|---|---|
| `ssh.forward_agent` | parsed, validated, has a scenario in `add-mvp-core`'s `ssh` spec, **no implementation anywhere in `src/`** | auditing what reached a sandbox's environment |
| `[env.vars]` | parsed, schema-exempted so arbitrary names are allowed, warns when a value contains `$` — **never applied** | building `[env] forward` beside it |
| `[[broker]]` *(would have been)* | — | caught only because its author was also its implementer, in the same change |

The shape is consistent and it is not carelessness: a key is added to the schema
in the same commit as the spec that describes it, the implementation is left for
the task after, and the task after does something else. Everything downstream
then *confirms* the key is fine — `openspec validate` passes, the schema check
accepts it, a typo in it gets a helpful suggestion naming the correct spelling.

**The user-visible failure is the worst kind: silent and confidence-inspiring.**
Someone writes `[env.vars] RUST_LOG = "debug"`, devcroft accepts the manifest
without complaint, and the variable reaches nothing. There is no error to search
for and no reason to doubt the manifest.

## What Changes

- **NEW** `manifest-key-liveness`: every field the schema accepts must be read
  by something outside the config layer, and that is checked mechanically
  rather than by review.
- The check runs in the test suite, so a key added without an implementation
  fails before it ships rather than after someone relies on it.

## Capabilities

### New Capabilities

- `manifest-key-liveness`: what it means for a manifest key to be implemented,
  how that is established, and what a key is allowed to look like when it is
  deliberately inert.

### Modified Capabilities

- (none — `openspec/specs/` holds no synced specs.)

## Impact

- **Affected code**: `config::validate`'s `SECTIONS` becomes the source of
  truth a test reads, rather than a list only the validator consults.
- **This will need an escape hatch and it must be narrow.** `[sandbox]
  isolation` is deliberately accepted-and-ignored (`remove-gvisor-backend`
  kept it to give a removed tier a real error), so "unused" is a legitimate
  state — but it has to be *declared* as such, in one place, with a reason.
  An escape hatch that is easy to reach for turns this check into decoration.
- **Not a guarantee the key does the right thing**, only that something reads
  it. A key read and then ignored would still pass. That is a real limit and
  is worth stating rather than overselling the check.

## Non-Goals

- **Not a general dead-code check.** Rust already reports unused private items;
  the gap here is specific — a *public schema* surface whose consumers live in
  another module, where the compiler is happy either way.
- **Not retroactive blame.** The three keys above are fixed or filed; this
  change exists so the fourth is caught by a test rather than by luck.
