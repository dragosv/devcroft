# nix-flake-sample

The same `citytime` CLI as [samples/flox-clap-sample](../flox-clap-sample/)
— on purpose. This sample's point is the environment provider underneath
(`env.provider = "nix"` instead of `"flox"`), not the CLI, so the two are
built to be directly comparable rather than showing off a different
feature each. Read that sample's README too if you haven't; this one only
covers what's different.

## What's different from flox-clap-sample

There is no `.flox/` here at all — the environment is a real
[nix flake](https://nixos.wiki/wiki/Flakes), `flake.nix` +
`flake.lock`, both committed. `devcroft.toml` sets
`env.provider = "nix"`; everything else about the sandbox (the two-phase
execution model, the store becoming a read-only grant, sessions running
under `network.default = "deny"` by default) is identical to flox — that
parity is the actual point of `add-nix-provider`: a second `Provider`
implementation behind the same contract, not a parallel set of concepts.

`flake.nix`'s `devShells.<system>.default` installs `cargo`, `rustc`,
`clippy`, and `rustfmt` from nixpkgs, pinned by `flake.lock`'s locked
`nixpkgs` revision — the same closure-level reproducibility guarantee
flox's own lockfile gives, from the same underlying store.

## Two real problems this sample hit, and how they were fixed

Both were found by actually running this sample against a live sandbox,
not written from documentation — same standard the rest of this repo
holds itself to (see `docs/ssh-validation.md` for the pattern).

**No `builtins.currentSystem` under pure evaluation.** The obvious way to
write a single-system flake is `system = builtins.currentSystem;`. That
fails outright: `error: attribute 'currentSystem' missing`. Nix flakes
evaluate *pure* by default, and `provider::nix` (this project's own nix
provider) deliberately never passes `--impure` — see
`docs/decisions.md` and the provider's own design doc on why purity is
part of the guarantee, not an inconvenience to route around.
`currentSystem` simply isn't defined in that mode. The fix is the
standard one: enumerate the systems devcroft supports
(`x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, `aarch64-darwin`) as a
plain list and let the `nix` CLI itself pick the matching
`devShells.<system>` entry — no evaluation-time knowledge of the host
required. `provider::nix`'s own test fixtures use the identical pattern.

**nixpkgs' cargo trusts no CA bundle.** The first real `cargo fetch`
inside the dev shell failed with `SSL peer certificate ... unable to get
local issuer certificate`, not a devcroft/sandbox denial — reproduced with
a bare `nix develop --command cargo fetch`, no devcroft involved at all.
A system-installed toolchain (Homebrew, apt, ...) links against the OS
trust store; a nix-built one has no such fallback until told where to
look. Fixed by adding `pkgs.cacert` to the dev shell's packages and
exporting `SSL_CERT_FILE`/`NIX_SSL_CERT_FILE` at its bundle path — the
standard nix devShell fix for any TLS-using tool (curl, cargo, git), not
specific to this sample.

## `$PWD`, not `self` — and not a project-local `$TMPDIR`

flox-clap-sample's `[hook] on-activate` redirects `CARGO_HOME` using
`$FLOX_ENV_PROJECT`, a variable flox itself provides. Nix has no
equivalent, so this flake's `shellHook` uses `$PWD` instead — safe
specifically because `provider::nix::capture_activated_env` runs
`nix develop` with the working directory set to the real project root, so
`$PWD` at shellHook time *is* that root. The tempting alternative,
`self` (the flake's own input reference), is the wrong choice: for a
local/path flake nix copies the source into its own read-only store and
`self` resolves there, not to the actual working directory on disk —
using it for a *writable* cache path would fail outright.

This sample also does **not** redirect `$TMPDIR` into the project, unlike
an earlier version of flox-clap-sample once did. That turned out to be
actively harmful, not merely unnecessary: the host's own `$TMPDIR` is
already writable inside a devcroft sandbox (unlike `~/.cargo`, which is
correctly denied), and a project-rooted `$TMPDIR` is long enough to blow
past macOS's 104-byte unix-socket path limit — the exact failure
documented in `docs/ssh-validation.md`'s VS Code Remote-SSH section. Kept
here as a deliberate omission, not an oversight.

## Try it

```sh
cd samples/nix-flake-sample
devcroft up
devcroft exec -- cargo build
devcroft exec -- ./target/debug/citytime time Bucharest
devcroft exec -- ./target/debug/citytime version
devcroft ssh                              # works the same as any other sandbox
devcroft policy --render                  # shows /nix/store with origin provider:nix
devcroft down
```

`devcroft exec -- cargo build` needs no network at all — `cargo fetch`
already ran host-side, inside the dev shell's `shellHook`, at `up`, before
the sandbox restriction applied (same two-phase model flox-clap-sample
uses, same reason: project code doesn't get host network access without
an explicit `network.allow` entry, and a per-build dependency fetch is
exactly that).
