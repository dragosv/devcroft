# Change: add-runtime-provenance

## Why

The owner's goal (2026-11-22) is that devcroft is reproducible at the level of
shell and userland too: every shell or utility a sandbox can execute comes from
a pinned source — the provider's lock or a devcroft-owned runtime pinned by the
project — and never from the host, except for named exceptions. Today that
property is neither visible nor true everywhere, and nothing reports where it
breaks:

- mise and pixi fall back to the host's `/bin/sh` when the environment has no
  shell, and on macOS grant the whole of `/bin` plus `/var/select`
  (`src/provider/mise.rs:987–1012`, `src/provider/pixi.rs:790–815`). mise also
  grants the host's `/usr/bin/env`; pixi does not, so the same shebang works
  under one and fails under the other.
- On macOS, every absolute-path shell use inside a sandbox fails: `#!/bin/sh`,
  `#!/usr/bin/env sh`, `system()` and `popen()` all get `EPERM` (measured
  2026-11-22 in a devbox sandbox; `initdb` reports `postgres` missing because
  its `popen("postgres -V")` never runs). `docs/known-gaps.md` does not say so.
- On Linux, the view writes `/usr/bin/env` only "where the closure has one"
  (`src/fleet/mount.rs:232,262`), so `#!/usr/bin/env` scripts fail in a flox
  sandbox without coreutils.

This change makes the property inspectable and testable **before** any of the
three follow-up changes (`add-own-userland`, `add-macos-exec-redirect`,
`embed-brush-runtime`) alters it, so that each of them is measured against a
witness rather than asserted.

## What Changes

- Define the **base runtime namespace**: the absolute paths a sandbox resolves
  for shell and userland (`/bin/sh`, `/usr/bin/env`, `/bin/<name>`,
  `/usr/bin/<name>` for a fixed set of base names) and the same names looked up
  through `PATH`.
- `up` records, per base name and per lookup form, where it resolves:
  `closure:<path>`, `provider:<name>:<path>` (an artifact provider's own
  install), `host:<path>`, or `absent`. Recorded in `Meta`, so `policy --render`
  and `status` show it from the same record the backend got.
- `up` prints **one** warning naming every base name that resolves to the host,
  per the degraded-capabilities rule. It never refuses on this ground in this
  change.
- A provider-row test asserts that the report matches what actually executes
  inside the sandbox, on every row and platform the suite runs, and asserts the
  invariant (no `host:` entry) on the rows that already satisfy it.
- `docs/known-gaps.md` publishes the macOS absolute-path failure and the Linux
  `/usr/bin/env` gap.

No resolution behaviour changes. This is reporting, a test witness and
documentation.

## Capabilities

### New Capabilities
- `base-runtime`: the base runtime namespace (shell and userland by absolute
  path and by `PATH`), its provenance record, how it is reported, and the
  invariant that nothing in it comes from the host outside a named exception.
  This change adds the namespace definition and provenance requirements; the
  follow-up changes add the runtime itself.

### Modified Capabilities
- None. Rendering provenance is added under `base-runtime`; the `policy` and
  `cli` requirements that `policy --render` and `status` show what the backend
  got are unchanged and are what this relies on.

## Impact

- `src/lifecycle/up.rs`, `src/lifecycle/state.rs` (`Meta`): the provenance
  record, computed after provider resolution and shell resolution.
- `src/policy/` render path and `src/lifecycle/status.rs`: show the record.
- `src/fleet/`: fleet agents record the same, from their own view.
- `tests/`: a provider-row test (`tests/common/mod.rs` rows) and a macOS probe.
- `docs/known-gaps.md`, `CLAUDE.md` (the origin list gains no new origin here;
  the record is reporting, not a grant).
- No new dependencies.
