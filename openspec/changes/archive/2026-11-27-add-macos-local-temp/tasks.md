## 0. Measurement gate

- [x] 0.1 Prototype D3 under `spikes/macos-local-temp/`: which path calls the samples' tools make under a literal `/tmp` (devenv's `enterShell`, process-compose, postgres and mysql init, Python, Node, Go, git, ssh, cargo), which can be interposed, and what is left refused. Record in `design.md`.
  - MariaDB 11.4.12 (nixpkgs), 2026-11-26, in a real nix sandbox, `network.default = "allow"`: `mariadb-install-db --datadir=/tmp/mydb`, `mariadbd` with `/tmp/my.sock`, and a client through it all work, every file in the sandbox's directory and none in the host's `/tmp`. It needs `--force`, because `resolveip` cannot look up the host name in a macOS sandbox, a gap of its own. MySQL itself not run.
- [x] 0.2 The cost: a `cargo build` and a file-heavy loop with and without the prototype.
- [x] 0.3 Which samples and tests rely on sharing `/tmp` with the host.
- [x] 0.4 Stop criterion (D3): report to the owner if a listed case does not work or the cost passes the budget.

Two sandboxes sharing a devenv checkout hit the stop criterion. D3a records
the approved provider adjustment.

## 1. The directory (no change to what is shared)

- [x] 1.1 Create, check, empty and remove it (D1, D5); refuse a path that is not the caller's own.
  - `runtime::local_temp`, with a key file in the parent against collisions of the 8-hex name.
- [x] 1.2 `TMPDIR`, `TMP`, `TEMP` for sessions, hooks, services and the provisioning process; a usable value of the project's wins.
  - `apply_temp_env`, also over what the activation script exported. Linux: `/tmp`.
- [x] 1.3 The baseline grants it, beside `/tmp`; `policy --render` shows both, and `docs/known-gaps.md` says `/tmp` is still shared.
  - Superseded by 2.4: the host's `/tmp` leaves the baseline in the same change, once 2.1 and 2.2 hold.
- [x] 1.4 Pass sandbox-specific devenv paths to environment, hook and service capture (D3a). Verify with two sandboxes of one checkout and document migration from a literal `.devenv/run`.
  - `tests/macos_local_temp.rs` two sandboxes over one devenv checkout; migration in `docs/base-runtime.md` and `docs/known-gaps.md`.

## 2. Literal paths

- [x] 2.1 The mapping and its exceptions, with unit tests of the table: `.` and `..`, `/private/tmp`, a project kept there, a manifest that grants `/tmp`, names that only start with `/tmp`.
  - `redirect::map_temp`, unit-tested; `redirect_temp.h` for the library.
- [x] 2.2 The calls (D3), a test each; two-path calls map both; a socket path that does not fit.
  - `tests/fixtures/macos-local-temp.c`, run in a sandbox by `compiled_library_handles_path_calls_and_passwd`.
- [x] 2.3 The keeper's own path uses (hooks' working directory, `rewrite`) agree with the library.
  - `session::command` maps the working directory and the program.
- [x] 2.4 The host's `/tmp` leaves the baseline on macOS; a manifest's `/tmp` is honoured; the first `up` after says so.
  - A manifest's `/tmp` is the host's, and the other sandboxes' directories stay denied (`protect_local_temp`). `up` says it once.

## 3. The user database

- [x] 3.1 `getpwuid`, `getpwnam` and their `_r` forms (D4); `up`'s macOS line says the name is cooperative.
  - In the same fixture: `getpwuid`, `getpwnam` and both `_r` forms, `ERANGE` included.

## 4. Tests

- [x] 4.1 Two sandboxes do not see each other's `/tmp`; neither sees nor removes the host's, by literal path and by `/private/tmp`.
- [x] 4.2 `mktemp`, Python, Node and Go temp files by variable and by literal path; a unix socket where the network mode allows one.
  - Python and the C calls in the suite; Node with `DEVCROFT_TEST_NODE`; Go when `go` is installed. A socket: the fixture, with `network.default = "allow"`.
- [x] 4.3 A process that lost the library is refused at `/tmp`, and works through `TMPDIR`.
  - The fixture's `plain` case: no library, `/tmp` refused, `$TMPDIR` works.
- [x] 4.4 `HOME` by variable and by `getpwuid`; it survives `down`, the temp directory does not.
- [x] 4.5 devenv and devbox rows; a project kept in `/private/tmp`; the suite's own sandboxes, most of which are there.
  - devenv in `tests/macos_local_temp.rs`; the suite's own projects, most under `/private/tmp`, pass. The devbox rows pass in the suite and have no assertion of their own about temp.

## 5. Documentation and verification

- [x] 5.1 `docs/known-gaps.md`, `docs/base-runtime.md`, `CLAUDE.md`.
- [x] 5.2 Both suites; `openspec validate add-macos-local-temp --type change`.
  - 2026-11-26. macOS arm64, with devenv on `PATH` and `DEVCROFT_TEST_NODE`: 783 passed, 1 failed (`denial_hook`: a Linux-only phrase, asserted now only there; it had never run on macOS, devenv not being on `PATH`), then 4/4. Linux aarch64, `test-support`: 870 passed, 0 failed; clippy clean.
