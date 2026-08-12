# flox-rustup-sample

A minimal Rust project demonstrating the toolchain split devcroft's own
`init` (cli spec) is built around, and how it fits inside a devcroft
sandbox:

- **rustup** (`rust-toolchain.toml`) pins the exact Rust *version*.
- **flox** (`.flox/env/manifest.toml`) provides everything rustup itself
  doesn't: a C toolchain (`gcc`, so `rustc` can link anything) and rustup
  itself, reproducibly and pinned by flox's own lockfile.
- **devcroft** (`devcroft.toml`) wraps the two in a sandboxed session, so
  `devcroft exec -- cargo build` runs this exact combination inside a
  kernel-enforced boundary (Landlock/Seatbelt) instead of directly on the
  host.

## Try it

```sh
cd samples/flox-rustup-sample
devcroft up                          # host-side: flox activates, rustup
                                      # installs the 1.90.0 pin (see below)
devcroft exec -- cargo build
devcroft exec -- ./target/debug/flox-rustup-sample
devcroft down
```

Without devcroft, `flox activate -- cargo build` works the same way, just
unsandboxed.

## Four real problems this setup runs into, and why the fixes look the way they do

Building this sample against a real, already-sandboxed devcroft surfaced
four genuine friction points between "a Rust toolchain manager" and "a
filesystem-confining sandbox." None of them are devcroft bugs — they're
exactly the kind of thing CLAUDE.md's two-phase execution model and
default-deny filesystem policy are supposed to force into the open rather
than fail silently on:

1. **`RUSTUP_HOME`/`CARGO_HOME` default outside the project root.**
   rustup/cargo otherwise inherit whatever `RUSTUP_HOME`/`CARGO_HOME` the
   *host* has set (e.g. `/usr/local/rustup` in devcroft's own
   devcontainer) — outside this project, so a sandboxed session correctly
   denies writing there instead of silently succeeding against the wrong
   toolchain. Fixed by redirecting both into the project.

2. **Cargo's workspace search walks upward past the project root.**
   Because this sample lives nested inside devcroft's own repository,
   plain `cargo build` walks up looking for a workspace root and reaches
   devcroft's own `Cargo.toml` — outside the sandboxed project root, so
   nono correctly denies reading it. Fixed the standard Cargo way: an
   explicit empty `[workspace]` table in this crate's own `Cargo.toml`
   stops the upward search here. (Any project sandboxed by devcroft, not
   just a nested one, should have this as good practice regardless.)

3. **The linker needs a writable temp directory.** `cc` uses `/tmp` by
   default, which is outside the project root and correctly denied.
   Fixed by pointing `TMPDIR` at a project-local directory, same as (1).

4. **`nono` requires some well-known vars to be absolute.** The first
   attempt set `TMPDIR = ".tmp"` (relative) via flox's `[vars]`, which
   `nono` rejected outright at `up` — the keeper never started, and the
   stale pidfile/socket it left behind then made `status`/`exec` hang
   trying to reach a keeper that was never running, rather than fail
   fast. `[vars]` is also the wrong place for any of this regardless:
   its values are literal strings with no shell interpolation (same
   constraint `devcroft.toml`'s own `[env] vars` documents), so it can't
   build an absolute path from the project's own location anyway.

All three redirects (1 and 3) live in flox's `[hook] on-activate` instead,
which is real bash and so can use `$FLOX_ENV_PROJECT` (flox's own absolute
path to the project root) to build absolute values:

```sh
export RUSTUP_HOME="$FLOX_ENV_PROJECT/.rustup"
export CARGO_HOME="$FLOX_ENV_PROJECT/.cargo"
export TMPDIR="$FLOX_ENV_PROJECT/.tmp"
```

Not `devcroft.toml`'s `[env] vars`: the config spec documents it as static
environment variables applied after provider resolution, and it parses
and validates correctly, but **nothing in `up`/the keeper actually
applies it to a session yet** — found while building this sample. flox's
own hook was used instead, since that path is proven working (it's how
`PATH` itself reaches sessions). If `[env] vars` gets wired up later,
these three redirects could move into `devcroft.toml` directly — though
they'd still need real interpolation support to reference the project
root, which `[env] vars` explicitly does not offer either.

## Why toolchain installation happens in a flox hook, not at `cargo build` time

The first `rustup show` (which triggers rustup's own auto-install of the
`rust-toolchain.toml` pin) needs network access. Running it inside
`devcroft exec` would hit devcroft's default `network.default = "deny"` —
correctly, since session-time code doesn't get host network access. So it
runs instead in flox's `[hook] on-activate`, which devcroft's `up` runs
*host-side, before the sandbox restriction is applied* (CLAUDE.md's
"Two-phase execution": provisioning is trusted, host-network work; only
sessions and hooks run inside the boundary). By the time `devcroft exec`
runs, the toolchain is already materialized and no network is needed.
