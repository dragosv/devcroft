# nix-probe-sample

The boundary probe from devcroft's own [README](../../README.md), as a
runnable project. The front page shows this code and the output it
produces; this sample is where that output is actually generated,
against a live sandbox, so the claim is a measurement rather than a
promise.

Provider is nix flakes (`add-nix-provider`), alongside
[nix-flake-sample](../nix-flake-sample/) (Rust) and
[nix-go-sample](../nix-go-sample/) (Go, a server). This one serves
nothing and depends on nothing outside the standard library — no
`go.sum`, no `cacert`. A probe that measures the sandbox boundary should
not also depend on the network being reachable in order to build.

## What it does

Three requests, all for things outside the project root:

| Probe | Expected |
|---|---|
| read `$HOME/.ssh/known_hosts` | refused — a credential |
| write `/etc/devcroft-probe` | refused — a system path |
| delete `$HOME/devcroft.tmp` | refused — a file in your home |

Every one is expected to fail. **Anything that succeeds is the finding.**

## The deletion probe creates its own target

This is the one part that is deliberately not the obvious thing. An
earlier version of the front-page probe called `os.RemoveAll(home)`,
which is a demonstration whose only failure mode is catastrophic: it is
copy-pasteable code whose entire purpose is to be refused, so the one
situation it is written for is the situation where it is *not* refused —
run outside devcroft, or inside a sandbox that turns out not to be
enforcing, and it deletes the reader's home directory.

So deletion is probed against `$HOME/devcroft.tmp`, a file this program
creates itself if it is missing. A demonstration of a boundary must not
be catastrophic when the boundary is absent, and here the worst an
unconfined run can do is remove the throwaway it just made. Verified on
the host, outside any sandbox:

```console
$ go run . probe "$HOME"
probing home: /Users/you
open /etc/devcroft-probe: permission denied

$ ls ~/devcroft.tmp
ls: /Users/you/devcroft.tmp: No such file or directory
```

Created, deleted, nothing left behind — and `known_hosts` read fine,
because nothing was enforcing. That is the whole point: the unsafe path
is now boring.

## Measured output

Everything below was produced by running this sample, on macOS 15 against Seatbelt, with a live nix daemon. Seatbelt
reports `EPERM` (`operation not permitted`); Linux's Landlock reports
`EACCES` (`permission denied`) for the same denials, which is the
wording the top-level README uses.

```console
$ devcroft exec -- go run .
hello from inside
/Users/you/devcroft/samples/nix-probe-sample

$ devcroft exec -- go run . probe "$HOME"
probing home: /Users/you
open /Users/you/.ssh/known_hosts: operation not permitted
open /etc/devcroft-probe: operation not permitted
open /Users/you/devcroft.tmp: operation not permitted
remove /Users/you/devcroft.tmp: no such file or directory
```

Four lines, not three, and the last one is worth reading carefully: the
removal reports the file *missing* rather than refused, because the
creation immediately above it was already denied. There was never a file
to delete. Both halves of "create it if absent, then delete it" were
refused, which is the result — the errno on the final line is a
consequence of the boundary holding, not a hole in it.

## `$HOME` is not your home under this provider

The probe takes the home directory as an optional argument, defaulting
to `os.UserHomeDir()`. The default is what the README's version uses and
what an ordinary program would do — and under the nix provider it is
**not** your home:

```console
$ devcroft exec -- go run . probe
probing home: /homeless-shelter
open /homeless-shelter/.ssh/known_hosts: no such file or directory
open /etc/devcroft-probe: operation not permitted
open /homeless-shelter/devcroft.tmp: no such file or directory
remove /homeless-shelter/devcroft.tmp: no such file or directory
```

`nix print-dev-env` exports `HOME=/homeless-shelter`, its own
build-sandbox value, and devcroft captures the environment as the
provider reports it. So a probe that trusts `$HOME` measures a path that
does not exist instead of the credentials it claims to be testing — it
prints four `no such file or directory` lines and proves nothing.

That is not a devcroft denial and this sample does not present it as
one. Passing the real path in is what makes the measurement mean what it
says, which is why the `"$HOME"` argument above is expanded by *your*
shell, on the host, before `devcroft exec` ever sees it.

## Three things that had to be discovered by running it

**`shellHook` never runs under the nix provider.** The first version of
this sample set `GOPATH`/`GOCACHE`/`GOENV` as `shellHook` exports, the
way [nix-flake-sample](../nix-flake-sample/) and
[nix-go-sample](../nix-go-sample/) both document their own redirects.
They arrived empty, and no `.go`/`.gocache` directory was ever created.
This is correct behaviour, not a bug: `provider::nix` resolves with `nix
print-dev-env --json` and treats the `shellHook` as inert data it never
evaluates, because a shellHook is project code and the two-phase
execution invariant says provisioning runs pinned tooling instead
(`fix-provisioning-hooks`, `src/provider/nix.rs`). A `mkShell`
*attribute* becomes a real variable in that JSON and survives; a
`shellHook` export does not. Everything this sample needs is therefore
an attribute.

**nix's `TMPDIR` points at a directory that is gone.**
`print-dev-env` reports `TMPDIR`, `TMP`, `TEMPDIR` and `NIX_BUILD_TOP`
as the per-invocation build directory nix used at `up` — something like
`/nix/var/nix/builds/nix-73810-3469085311`. By session time that
directory no longer exists, and it sits under a prefix the policy denies
besides, so Go dies before compiling anything:

```
go: creating work dir: stat /nix/var/nix/builds/nix-73810-3469085311: no such file or directory
```

A `mkShell` attribute named `TMPDIR` does not fix it — nix's own stdenv
overwrites that one before reporting the environment (measured:
`GOCACHE`, `GOFLAGS` and `GOENV` set the same way all survive; `TMPDIR`
does not). `flake.nix` sets **`GOTMPDIR`** instead, which Go's build
honours first and nix does not touch.

**`devcroft.toml`'s `[env.vars]` is a silent no-op.** The obvious fix for
the above was to set `TMPDIR` there. It parses, it validates — the
validator even rejects `$` interpolation in its values — and then
nothing consumes it: no code outside `src/config/` reads `env.vars` at
all, so it never reaches the keeper or a session. Worth knowing before
reaching for it; this sample deliberately does not use it.

## Try it

```sh
cd samples/nix-probe-sample
devcroft up
devcroft exec -- go run .                    # hello from inside
devcroft exec -- go run . probe "$HOME"      # three refusals
devcroft policy --render                     # /nix/store, origin provider:nix
devcroft down
```

One gotcha that is nix's rather than devcroft's: a flake only sees
git-tracked files, so a freshly created sample fails `up` with
`error: Path 'samples/nix-probe-sample' ... is not tracked by Git` until
you `git add` it.
