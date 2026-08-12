# flox-clap-sample

A two-subcommand CLI (`citytime`) demonstrating clap's derive API — the
de facto standard for a Rust CLI with more than a couple of subcommands.
`#[derive(Parser)]`/`#[derive(Subcommand)]` give you `--help`, per-subcommand
help, and (via `clap_complete`, not wired up here) shell completions for
free from the same struct that parses args. structopt merged into clap v3;
there's no reason to reach for it separately anymore.

Same toolchain split as [samples/flox-rustup-sample](../flox-rustup-sample/)
(read that one first — this README only covers what's different): rustup
(`rust-toolchain.toml`) pins Rust **1.97.0**, flox provides the C toolchain
and rustup itself, devcroft.toml sandboxes sessions around both.

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

## What's different from flox-rustup-sample: real crates.io dependencies

flox-rustup-sample had zero dependencies, so its `[hook] on-activate` only
needed to materialize the *toolchain* (`rustup show`) host-side before the
sandbox restriction applies. This sample adds `clap`, `chrono`, and
`chrono-tz` — real crates.io dependencies — so the first `devcroft exec --
cargo build` attempt failed trying to reach `index.crates.io`, correctly
denied by the sandbox's default `network.default = "deny"`.

Fixed the same way as the toolchain itself: the hook also runs `cargo
fetch` (after redirecting `CARGO_HOME` into the project, same as before),
downloading every dependency into the project-local registry cache
host-side, at `up`, before restriction — matching CLAUDE.md's two-phase
execution model. `devcroft exec -- cargo build` then compiles fully
offline.

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
