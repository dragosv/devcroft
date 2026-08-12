# flox-clap-sample

A two-subcommand CLI (`citytime`) demonstrating clap's derive API — the
de facto standard for a Rust CLI with more than a couple of subcommands.
`#[derive(Parser)]`/`#[derive(Subcommand)]` give you `--help`, per-subcommand
help, and (via `clap_complete`, not wired up here) shell completions for
free from the same struct that parses args. structopt merged into clap v3;
there's no reason to reach for it separately anymore.

Unlike [samples/flox-rustup-sample](../flox-rustup-sample/) (worth reading
too — this README assumes it and only covers what's different), this one
has **no rustup at all**: `rustc`/`cargo`/`clippy`/`rustfmt` are installed
directly from flox, each pinned to an exact version in
`.flox/env/manifest.toml`'s `[install]` section. flox's own lockfile
(`.flox/env/manifest.lock`) already gives closure-level reproducibility for
the compiler, so a second toolchain manager on top of it isn't buying
anything here — `rust-toolchain.toml` is a rustup-specific mechanism and
doesn't exist in this sample at all. `devcroft.toml` still sandboxes
sessions the same way.

The pin is `1.95.0`, not the latest available (`1.97.1`): `flox show
rustc`/`cargo`/`clippy`/`rustfmt` all show `1.96.1` and `1.97.x` published
only for `aarch64-darwin`/`aarch64-linux`/`x86_64-linux` — no
`x86_64-darwin` build exists for those in nixpkgs. `1.95.0` is the most
recent version that covers all four platforms devcroft supports, so
`.flox/env/manifest.toml`'s `[options] systems` lists all four rather than
being scoped down to just this devcontainer's platform.

## Commands

```sh
citytime time <CITY>   # current time in CITY (a small fixed lookup table
                        # in main.rs — see below for why)
citytime version        # prints 0.1.0
```

## Try it

```sh
cd samples/flox-clap-sample
devcroft up
devcroft exec -- cargo build
devcroft exec -- ./target/debug/citytime time Bucharest
devcroft exec -- ./target/debug/citytime version
devcroft ssh                              # works too — see below
devcroft down
```

## Real crates.io dependencies still need a host-side fetch

flox-rustup-sample had zero dependencies. This sample adds `clap`,
`chrono`, and `chrono-tz` — real crates.io dependencies — so the first
`devcroft exec -- cargo build` attempt failed trying to reach
`index.crates.io`, correctly denied by the sandbox's default
`network.default = "deny"`. Project code doesn't get host network access
without an explicit `network.allow` entry, and a per-build dependency
fetch is exactly that: session-time network use, not one-time
provisioning.

Fixed in `[hook] on-activate`: `cargo fetch` downloads every dependency
into a project-local registry cache (`CARGO_HOME` redirected into the
project via `$FLOX_ENV_PROJECT`, since cargo otherwise defaults it to
somewhere outside the project too, e.g. `/usr/local/cargo` or `~/.cargo`)
— host-side, at `up`, before the sandbox restriction, matching CLAUDE.md's
two-phase execution model. `devcroft exec -- cargo build` then compiles
fully offline.

## Why the city table is hardcoded, not looked up from a service

A real CLI would resolve a city name against a geocoding/timezone API.
That needs network access **at session time** (a lookup per invocation,
not a one-time provisioning step), which devcroft's default deny-all
correctly refuses — and rightly so; project code doesn't get host network
access without an explicit `network.allow` entry. Kept as a small fixed
`&[(&str, Tz)]` table in `main.rs` instead, so the sample needs no network
access at all once built, consistent with flox-rustup-sample's whole point
about what belongs at host-side provisioning versus inside a sandboxed
session.

## `devcroft ssh` works the same as any other sandbox

No sample-specific wiring needed — `devcroft ssh` (or plain `ssh` through
`devcroft proxy`, or an editor's Remote-SSH) reaches the same sandboxed
session as `devcroft exec`, pty or not:

```sh
$ printf './target/debug/citytime time Bucharest\nexit\n' | devcroft ssh --no-up
Bucharest: 2026-08-12 23:35:50 EEST
```
