## 0. Measurement gate

- [ ] 0.1 Prototype D3 under `spikes/macos-local-temp/`: which path calls the samples' tools make under a literal `/tmp` (devenv's `enterShell`, process-compose, postgres and mysql init, Python, Node, Go, git, ssh, cargo), which can be interposed, and what is left refused. Record in `design.md`.
- [x] 0.2 The cost: a `cargo build` and a file-heavy loop with and without the prototype.
- [x] 0.3 Which samples and tests rely on sharing `/tmp` with the host.
- [x] 0.4 Stop criterion (D3): report to the owner if a listed case does not work or the cost passes the budget.

Two sandboxes sharing a devenv checkout hit the stop criterion. D3a records
the approved provider adjustment. The MySQL probe in 0.1 is still pending.

## 1. The directory (no change to what is shared)

- [ ] 1.1 Create, check, empty and remove it (D1, D5); refuse a path that is not the caller's own.
- [ ] 1.2 `TMPDIR`, `TMP`, `TEMP` for sessions, hooks, services and the provisioning process; a usable value of the project's wins.
- [ ] 1.3 The baseline grants it, beside `/tmp`; `policy --render` shows both, and `docs/known-gaps.md` says `/tmp` is still shared.
- [ ] 1.4 Pass sandbox-specific devenv paths to environment, hook and service capture (D3a). Verify with two sandboxes of one checkout and document migration from a literal `.devenv/run`.

## 2. Literal paths

- [ ] 2.1 The mapping and its exceptions, with unit tests of the table: `.` and `..`, `/private/tmp`, a project kept there, a manifest that grants `/tmp`, names that only start with `/tmp`.
- [ ] 2.2 The calls (D3), a test each; two-path calls map both; a socket path that does not fit.
- [ ] 2.3 The keeper's own path uses (hooks' working directory, `rewrite`) agree with the library.
- [ ] 2.4 The host's `/tmp` leaves the baseline on macOS; a manifest's `/tmp` is honoured; the first `up` after says so.

## 3. The user database

- [ ] 3.1 `getpwuid`, `getpwnam` and their `_r` forms (D4); `up`'s macOS line says the name is cooperative.

## 4. Tests

- [ ] 4.1 Two sandboxes do not see each other's `/tmp`; neither sees nor removes the host's, by literal path and by `/private/tmp`.
- [ ] 4.2 `mktemp`, Python, Node and Go temp files by variable and by literal path; a unix socket where the network mode allows one.
- [ ] 4.3 A process that lost the library is refused at `/tmp`, and works through `TMPDIR`.
- [ ] 4.4 `HOME` by variable and by `getpwuid`; it survives `down`, the temp directory does not.
- [ ] 4.5 devenv and devbox rows; a project kept in `/private/tmp`; the suite's own sandboxes, most of which are there.

## 5. Documentation and verification

- [ ] 5.1 `docs/known-gaps.md`, `docs/base-runtime.md`, `CLAUDE.md`.
- [ ] 5.2 Both suites; `openspec validate add-macos-local-temp --type change`.
