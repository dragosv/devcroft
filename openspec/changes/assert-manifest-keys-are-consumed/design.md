# Design — Manifest Key Liveness

## Context

`config::validate::SECTIONS` is the schema: a list of `(section, &[fields])`.
It is consulted by `check_unknown_keys` to reject typos, and by nothing else.
Whether a field is ever *read* is a separate question no code asks.

Three keys have been found in that state. The consistent shape is that the
schema entry and the spec are written together and the implementation is left
for later — after which every automated signal reports health.

## Goals / Non-Goals

**Goals:** a key with no consumer fails the suite, naming itself.

**Non-Goals:** not checking that a key does the *right* thing — only that
something reads it. A key read and then ignored still passes, and pretending
otherwise would oversell the check.

## Decisions

## D1 — Grep the source, and accept that it is a heuristic

**Decision.** The test walks `SECTIONS`, derives the Rust field path each key
maps to (`manifest.ssh.forward_agent`, `manifest.env.vars`), and requires a
reference outside `src/config/`.

**Why not something stronger.** A type-level proof — every field consumed by an
exhaustive destructuring somewhere — would be sound, and would mean threading a
`match` on the whole `Manifest` through code that has no reason to want one. The
grep is weaker and cheap; the failure it prevents is "nobody wrote the code at
all", which a grep detects perfectly well.

**Its known weakness, stated so nobody trusts it further than it goes**: a
reference inside a comment counts. `strip_string_literals` already exists in
this repo's lint tests for the neighbouring problem, and the same treatment
applies — but a doc comment naming the field is a legitimate reference, so this
cannot be made airtight without becoming annoying. It catches the case that has
actually happened three times.

## D2 — The escape hatch is one list with a reason per entry

`[sandbox] isolation` is accepted and ignored on purpose:
`remove-gvisor-backend` kept it so a manifest naming the removed tier gets a
real error rather than "unknown key". So "inert" is a legitimate state and the
check must allow it.

**Decision.** One `const DELIBERATELY_INERT: &[(&str, &str)]` — key and reason —
in the test itself. Not an attribute, not a naming convention, not a comment the
test parses.

**Rationale.** An escape hatch that is easy to reach for makes the check
decoration. A list that has to be edited, in the test file, with a sentence
saying why, is exactly as much friction as this should have: trivial when
genuine, visible in review when not.

## Risks / Trade-offs

- **[Risk] The check passes while the key still does nothing** — read into a
  variable that is then dropped. → Real, unmitigated, and stated in the
  proposal. The check's claim is "someone wrote code for this", not "the code
  is correct".
- **[Trade-off] Field paths are derived by convention** (`section.field` →
  `manifest.section.field`), which breaks if a field is renamed in Rust but not
  in TOML. → That mismatch would fail the check loudly rather than silently,
  which is the right direction to fail in.

## Open Questions

1. **Does this belong as a test, or as `openspec validate`'s business?** The
   spec/schema drift it detects is arguably a spec-tooling concern, and
   OpenSpec already validates changes. Kept as a test here because it needs to
   read Rust source, which is outside what that tool does.
