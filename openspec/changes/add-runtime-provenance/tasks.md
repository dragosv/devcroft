## 0. Measurement — record the current state before writing code

Record each result in `design.md` under "Context", tagged measured, with OS,
arch and provider version.

- [ ] 0.1 On Linux, for each of nix, flox, devbox, devenv, mise and pixi (sample projects; a flox minimal manifest without coreutils included): execute every base name in both lookup forms inside the sandbox and record what ran. Confirm the table in `design.md` or correct it.
- [ ] 0.2 Same on macOS arm64. Confirm `EPERM` for `/bin/sh`, `/usr/bin/env` and shebang scripts in closure-tier sandboxes, and the reachable host `/bin` in mise and pixi sandboxes.
- [ ] 0.3 Confirm on Linux that a flox sandbox without coreutils fails a `#!/usr/bin/env` script (the inferred gap), so `known-gaps.md` states a measured fact.

## 1. The base-name set and the record

- [ ] 1.1 Add the base-name constant with a revision number and the inventory documentation page (`docs/base-runtime.md`), listing both lookup forms per name.
- [ ] 1.2 Add the provenance type (`closure`, `provider`, `host`, `absent` with reason) and store the record in `Meta`; old `Meta` without it deserialises as "not recorded".
- [ ] 1.3 Compute the record in `up` after provider and shell resolution, from `PATH`, grants, `ResolvedShell`, `Identity` (Linux) and the compiled policy's rules (macOS). No spawns.
- [ ] 1.4 Compute the same in `fleet::project::prepare` from the agent's view, sharing the function with `up`.

## 2. Reporting

- [ ] 2.1 `policy --render`: one line per base name and lookup form, rendered from `Meta`.
- [ ] 2.2 `status`: counts per provenance kind and every `host:` entry by name.
- [ ] 2.3 `up`: one warning naming every `host:` entry, through the existing degraded-capability warning path.

## 3. Tests

- [ ] 3.1 Provider-row test (`tests/common/mod.rs` rows): execute each base name in both forms inside the sandbox; assert the record matches; assert no `host:` on rows that satisfy the invariant; carry explicit expected `host:` entries for the rows that do not.
- [ ] 3.2 Unit tests for the computation: a grant covering a path by its parent, a `PATH` entry outside every grant, a missing view entry, a macOS refusal.
- [ ] 3.3 A macOS test (capability-guarded) asserting `absent (refused by policy)` for `/bin/sh` in a closure-tier sandbox and that the shebang script indeed fails.
- [ ] 3.4 Run the suite with `-- --nocapture 2>&1 | grep skipping` and record which rows actually ran on each platform.

## 4. Documentation

- [ ] 4.1 `docs/known-gaps.md`: on macOS, `#!/bin/sh`, `#!/usr/bin/env`, `system()` and `popen()` fail with `EPERM` inside a sandbox; on Linux, `#!/usr/bin/env` fails where the closure has no `env`. Both with the measurement and the follow-up change that addresses them.
- [ ] 4.2 README Status / Limitations: one line pointing at the provenance report.

## 5. Verification

- [ ] 5.1 `cargo build`, `cargo fmt`, `cargo clippy`, `cargo doc --no-deps` (zero rustdoc warnings), `cargo test`.
- [ ] 5.2 `openspec validate add-runtime-provenance --type change`.
