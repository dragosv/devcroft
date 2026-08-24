# devbox-citytime-sample

A third `citytime` CLI, alongside
[samples/flox-clap-sample](../flox-clap-sample/) and
[samples/nix-flake-sample](../nix-flake-sample/) — on purpose, same as
those two are deliberately identical in concept to each other. This
sample's point is the environment provider underneath
(`env.provider = "devbox"`), not the CLI. Read those two READMEs first if
you haven't; this one only covers what's different, and there is one
real difference in the CLI itself — see below.

## What's different from the other two

There is no `.flox/` and no `flake.nix` here — the environment is a real
[devbox](https://www.jetify.com/devbox) project, `devbox.json` +
`devbox.lock`, both committed. `devcroft.toml` sets
`env.provider = "devbox"`. Everything else about the sandbox (the
two-phase execution model, the store becoming a read-only grant, sessions
running under `network.default = "deny"` by default) is identical to
flox and nix — that parity is the actual point of `add-devbox-provider`:
a third `Provider` implementation behind the same contract, confirming
the trait generalizes to a provider whose activation mechanism (its own
resolver, its own lockfile format, no flake) isn't built on the same
substrate the first two share.

`devbox.json` declares `rustc` and `cargo`, pinned by `devbox.lock`'s
per-system resolutions — the same closure-level reproducibility guarantee
flox's and nix's own lockfiles give, from the same underlying store.

## Why no clap, no chrono

flox-clap-sample and nix-flake-sample both fetch their real crates.io
dependencies (`clap`, `chrono`, `chrono-tz`) host-side, once, at `up` —
flox's via `[hook] on-activate`, nix's via the flake's `shellHook`. Both
hooks run inside the mechanism devcroft actually uses to capture
activation for those providers.

**devbox's provider does not have an equivalent hook to put that in.**
`add-devbox-provider`'s design measured, rather than assumed, that
devbox's `shellenv --pure` — the only capture mechanism that does not run
a project's `shell.init_hook` — is the one devcroft has to use precisely
*because* the other candidate (`devbox run`) does run it, which would
execute project code during the trusted, pre-restriction phase (see
`docs/decisions.md`/`add-devbox-provider/design.md` decision 2). So
unlike the other two providers, a devbox project genuinely has **no
host-side hook devcroft will ever execute** — not a gap to work around,
a deliberate consequence of the two-phase rule holding for real.

A real devbox project that needs crates.io dependencies has two honest
options: vendor them into the repo (`cargo vendor` + a committed
`vendor/` directory, so `cargo build` never touches the network at all),
or grant an explicit `[network] allow = [...]` entry and accept that the
fetch happens at session time, denied by default like any other network
use. Neither is a CLI concern, so this sample sidesteps the question
entirely by depending on nothing beyond `std` — `civil_from_days` (a
public-domain days-since-epoch → calendar-date algorithm) replaces
`chrono`, and a three-line `match` on `env::args()` replaces `clap`. The
city table is a fixed UTC offset per city rather than a real IANA
timezone lookup (no DST), which is a bigger simplification than the other
two samples make — worth stating plainly rather than glossing over.

## Commands

```sh
citytime time <CITY>   # current time in CITY (a small fixed UTC-offset
                        # table in main.rs — see above for why)
citytime version        # prints 0.1.0
```

## `/tmp` needs an explicit grant

Found live, not assumed: the devbox-resolved `gcc`'s linker step failed
with `Cannot create temporary file in /tmp/: Permission denied` on the
first real `devcroft exec -- cargo build` — `/tmp` is not part of what
devcroft grants a closure-tier project by default
(`own-policy-baseline`), the same requirement
[samples/nix-go-sample](../nix-go-sample/)'s own manifest documents for
`go build`'s scratch directory. Fixed by declaring it like any other
filesystem need:

```toml
[filesystem]
allow = [".", "/tmp"]
```

## Try it

```sh
cd samples/devbox-citytime-sample
devcroft up
devcroft exec -- cargo build
devcroft exec -- ./target/debug/citytime time Bucharest
devcroft exec -- ./target/debug/citytime version
devcroft ssh                              # works the same as any other sandbox
devcroft policy --render                  # shows /nix/store with origin provider:devbox
devcroft down
```

`devcroft exec -- cargo build` needs no network at all: the toolchain
itself (rustc, cargo, the C compiler cargo's linker step shells out to)
was already materialized host-side at `up`, before the sandbox
restriction applied, exactly like the other two providers — and, per the
section above, this sample has no crates.io dependencies to fetch either,
so there is genuinely nothing session-time network could be needed for.

## `devcroft ssh` works the same as any other sandbox

```sh
$ printf './target/debug/citytime time Bucharest\nexit\n' | devcroft ssh --no-up
Bucharest: 2026-08-24 18:55:30 UTC+02:00
```
