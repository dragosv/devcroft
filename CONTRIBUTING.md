# Contributing to devcroft

This guide covers setting up a checkout, choosing and running the right tests,
changing behavior through OpenSpec, and keeping dependencies and the published
package correct. For code locations and the rules every change must keep, see
[AGENTS.md](AGENTS.md). The reasoning behind those rules is in
[the architecture invariants](docs/invariants.md).

## Set up a checkout

You need a Rust toolchain that meets `rust-version` in [Cargo.toml](Cargo.toml),
currently 1.95. Build from the repository root:

```sh
cargo build --locked
```

The [devcontainer configuration](.devcontainer/devcontainer.json) documents
the Linux setup, including the Nix daemon and the cgroup delegation that fleet
tests need. Validate Seatbelt behavior, and anything Swift or Xcode, on a macOS
host.

Most integration tests need an environment provider (flox, nix, devbox,
devenv, mise or pixi) that can actually build. An installed provider binary
isn't enough; see [Read a test run correctly](#read-a-test-run-correctly).

## Run the checks CI runs

Run the test that owns the behavior first. The CLI help and version tests need
no provider:

```sh
cargo test --locked --test cli_help_and_version
```

Before you push a Rust change, run the lint and documentation checks:

```sh
cargo fmt --all -- --check
cargo clippy --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps
```

Run `cargo clippy` without `--all-targets`; that's the exact command CI
checks. Keep rustdoc at zero warnings, because docs.rs renders it publicly.
After a sample whose closure carries the Apple SDK has been brought up,
`cargo doc` also prints about 24 "File system loop found" warnings. They come
from cargo walking the SDK's self-referential ncurses symlinks under
`samples/*/.devenv/profile`, not from devcroft, and `[workspace] exclude`
doesn't suppress them. Count rustdoc's own warnings.

## Choose a test row

Row-driven tests in [`tests/common/mod.rs`](tests/common/mod.rs) run against
the row that `DEVCROFT_TEST_PROVIDER` selects. The default is the real `nix`
row, so a plain `cargo test` stays realistic:

- **Without Nix:** select the in-process fixture row. It exists only under the
  `test-support` feature:

  ```sh
  DEVCROFT_TEST_PROVIDER=test cargo test --locked --features test-support -- --nocapture
  ```

  Backend-dependent cases still need a working backend. This row doesn't
  qualify a real provider or stand in for a cold CLI start.
- **With a real provider:** name the row you want to qualify (`nix`, `flox`,
  `devbox` or `swift`):

  ```sh
  DEVCROFT_TEST_PROVIDER=nix cargo test --locked -- --nocapture
  ```

  Some providers run dedicated integration binaries rather than a row; CI
  shows which.
- **With every available row:**

  ```sh
  DEVCROFT_TEST_PROVIDER=all cargo test --locked --features test-support -- --nocapture
  ```

  Unavailable rows are reported, not replaced. The matrix fails if every
  selected row is unavailable, but tests outside the matrix can still skip
  themselves.

Keep `test-support` out of the default build. It's the seam that lets tests
drive `up` with a provider no manifest could name.

On Linux, fleet cgroup tests also need `DEVCROFT_TEST_CGROUP_ROOT` to point to
a delegated cgroup that contains the test process. See the
[devcontainer helper](.devcontainer/cgroup-delegate.sh) for how to set one up.

## Read a test run correctly

A green `cargo test` is not the same as a run that tested anything. Every e2e
test skips itself on a host that can't support it, and in cargo's default
output a skip looks exactly like a pass. To list what a run skipped:

```sh
cargo test -- --nocapture 2>&1 | grep skipping
```

On a devcontainer whose `nix-daemon` isn't running, that's about 80 tests,
most of the interesting ones.

Guard each test on the capability, never on the binary. `flox --version`,
`flox init`, `nix flake --help` and `devbox version` all succeed with an
unreachable Nix store, and every test that then tried to build an environment
failed in a way that read as a devcroft regression. Use the shared probes:
`provider::host_can_build_nix_closures()`, which connects to the daemon socket
and treats a missing socket as usable because a single-user store has none,
and `policy::backend_supported()`.

A reachable store still isn't enough for flox. Since flox 1.4, resolving a
manifest that has no lock yet requires a FloxHub login, while `search`, `show`,
`init` and activating an already-locked environment work without one. So:

- The flox row never resolves. It copies the pre-locked
  `tests/fixtures/flox-row`, which only `scripts/lock-flox-test-row.sh`
  re-locks.
- `tests/flox_derived_env.rs` writes fresh manifests, so it probes by resolving
  one (`flox list`), not with `flox auth status`.
- `tests/flox_fresh_resolution.rs` keeps first-lock resolution covered. After
  flox is selected, it fails without a login instead of skipping.

When you report a run, give the OS, the backend, the provider rows and every
material skip.

## Test a boundary change

For a change to what the sandbox allows, test a denied operation and a
permitted control together; a check that refuses everything passes a denial
test alone. Run the affected tests on Linux and on macOS, or say which platform
you didn't verify. When you upgrade `nono`, keep `nono` and `nono-proxy` at
matching exact versions, and run the affected boundary tests with their
prerequisites, including fleet coverage in a delegated cgroup where it
applies.

## Change behavior with OpenSpec

A change to a capability, to policy, to the public CLI or to enforcement goes
through an OpenSpec change before code:

1. Run `openspec list`. Find the requirement and the active changes that
   touch it. The base specs are in
   [`openspec/changes/add-mvp-core/specs/`](openspec/changes/add-mvp-core/specs/),
   because archived changes weren't synced into `openspec/specs/`. Read later
   active and archived changes for what amended them.
2. Propose or update the change with the [repository workflows](.agents/skills/).
   Keep its proposal, design, scenarios and tasks consistent with each other.
   Each change lives in `openspec/changes/CHANGE_ID/` with its `.openspec.yaml`
   marker. Project context is in [`openspec/config.yaml`](openspec/config.yaml).
3. Validate the change:

   ```sh
   openspec validate CHANGE_ID --type change --strict
   ```

   Replace *`CHANGE_ID`* with the change's directory name, for example
   `add-agent-packages`. After you change shared OpenSpec configuration or
   several specs, run `openspec validate --all`.
4. Keep the implementation, tests, `USAGE` in `src/bin/devcroft.rs` and the
   user-facing docs consistent with the accepted requirement.

Before you propose a provider, check its fit against the six criteria in
[the decisions](docs/decisions.md): a declarative manifest, a restorable
lockfile, an immutable-capable shared store, capturable activation,
completeness and verifiable preconditions. A provider that covers language
runtimes but leaves the C toolchain to the host brings host passthrough back
under another name.

## Update dependencies and licenses

devcroft is Apache-2.0, matching nono and the sigstore crates it links. A
dual `MIT OR Apache-2.0` license would let a user take MIT and lose the patent
grant without changing their real obligations, because nono, russh and every
`sigstore-*` crate are Apache-2.0 only and linked regardless. Apache-2.0
§4(a) requires that recipients get a copy of each license, which is what
`THIRD-PARTY-LICENSES.md` is for.

After any change to `Cargo.lock`, regenerate it and review the diff:

```sh
python3 scripts/gen-third-party-licenses.py
```

The generator unions the dependencies of all four release targets (`TARGETS`
in the script), so the file is the same on every host. An earlier version read
`cargo tree -e normal` for the host target alone. It silently dropped the Linux
keyring tail on a Mac and an x86_64-only crate on aarch64 Linux, and each diff
looked like a legitimate shrink.

`nono`, `russh` and the `sigstore-*` crates declare Apache-2.0 but vendor no
license file, so the generator substitutes the canonical Apache-2.0 text. That
substitution is exact because the Apache-2.0 text has no per-holder copyright
line; MIT does, so MIT is never substituted. If a crate has no text the
generator can use, the generator fails. Take the text from the crate's own
repository at the packaged revision, save it in `scripts/license-texts/`, and
name the exact version in `SUPPLIED` in the script.

## Keep the package correct

The published crate is an anchored `include` allowlist in `Cargo.toml`.
Without the leading `/` on each pattern, the globs match at any depth and sweep
in `samples/`, flox store symlinks and a vendored Go module cache. Without the
allowlist, the package would ship `openspec/` and `.claude/` permanently. When
the crate needs a new file, add it there.

The one negation, `!/src/bin/spike.rs`, is load-bearing. Cargo auto-discovers
`src/bin/` targets, so shipping that file makes `cargo install devcroft` put a
second, generically named `spike` binary on the user's `PATH`.

After you change build inputs or add a binary, list the package and check what
an install would expose:

```sh
cargo package --locked --allow-dirty --no-verify --list
```

A file list isn't a package build or an install test. For anything the
installed binary must contain, build from the unpacked `cargo package` output
and run it.

## Release

The first release is `0.0.1`, deliberately. `0.0.z` is the only range cargo
treats as incompatible with itself, which matches what `src/lib.rs` states:
internals are published so `tests/` can drive them, with no stability
offered. [The roadmap](docs/roadmap.md) holds the rule for when `0.1.0` is
cut. Publishing (`cargo publish`, reserving the npm name) needs the
maintainer's own accounts.

Publishing is two crates, in order. devcroft depends on `devcroft-russh`, the
fork of russh in `vendor/russh` that returns SSH receive-window credit only
after a child consumes its stdin. Publish the fork first, from `vendor/russh`;
it packages and verifies on its own. Until it's on crates.io, `cargo package`
for devcroft fails with "no matching package named `devcroft-russh`", which is
expected. `vendor/russh/NOTICE` says what changed, and
`vendor/russh/devcroft-changes.patch` reproduces the fork from the upstream
crate.

Don't replace the fork with a `[patch.crates-io]` override. `cargo package`
drops `[patch]`, so the published binary would use unpatched russh without any
error.

## Write documentation

Each document has one job; don't duplicate content between them:

- `README.md` is for someone deciding whether to use devcroft. Keep it short,
  and link to detail instead of repeating it.
- [docs/known-gaps.md](docs/known-gaps.md) holds every published limitation.
- [docs/implementation-log.md](docs/implementation-log.md) holds what was built
  and what turned out to be wrong.
- [docs/decisions.md](docs/decisions.md) answers "why doesn't devcroft support
  X" falsifiably.
- [docs/prior-art.md](docs/prior-art.md) records ideas taken from other
  projects, and from where.
- `AGENTS.md` holds durable rules for coding agents, and
  [docs/invariants.md](docs/invariants.md) holds the reasoning behind them.

## Report a security problem

Read [the threat model](docs/threat-model.md) first: the isolation is accident
protection, not a security boundary. To report a problem, open an issue in
[the issue tracker](https://github.com/dragosv/devcroft/issues).
