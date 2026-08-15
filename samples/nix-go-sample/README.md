# nix-go-sample

A hello-world HTTP server using [Gin](https://github.com/gin-gonic/gin) —
the most-starred Go web framework by a wide margin (~80k GitHub stars vs.
Echo's and Fiber's each well under half that) — demonstrating that the
nix provider (`add-nix-provider`) is language-agnostic.
[samples/nix-flake-sample](../nix-flake-sample/) covers Rust; this one
covers Go, and needed nothing devcroft-specific beyond the same
flake.nix + project-local-cache-redirect pattern that sample already
established.

## What it is

```sh
GET /         -> {"message":"hello from a devcroft sandbox"}
GET /health    -> ok
```

`flake.nix`'s `devShells.<system>.default` installs `go` from nixpkgs,
pinned by `flake.lock`'s locked nixpkgs revision — closure-level
reproducibility for the toolchain itself. `go.sum` layers Go's own
lockfile on top of that, pinning every dependency (Gin and its full
transitive graph) by content hash — the same two-lockfile shape
nix-flake-sample has with `flake.lock` (toolchain) + `Cargo.lock`
(dependencies).

## Three real problems this sample hit, and how they were fixed

All three were found by actually building and running this sample
against a live sandbox — same standard the rest of this repo holds
itself to (see `docs/ssh-validation.md` for the pattern this follows).

**`go mod download` alone doesn't populate `go.sum`.** The intuitive
first attempt — write `go.mod` with just `require gin v1.10.0`, run `go
mod download` in the flake's `shellHook` — leaves `go build` failing with
`missing go.sum entry for module providing package github.com/gin-gonic/gin`.
`go mod download` only fetches what the current module graph already
declares; it doesn't resolve *and pin* the full transitive graph the way
`cargo fetch` does from an existing `Cargo.lock`. That resolution step is
`go mod tidy`, run once, host-side, with real network, the same way
`Cargo.lock`/`flake.lock` themselves get generated — its output (`go.mod`'s
added `require (... // indirect)` block, and a fully populated `go.sum`)
is what's committed here. The flake's `shellHook` only ever runs `go mod
download` after that, exactly the "restore from a lockfile" step it was
supposed to be.

**nixpkgs' `go` trusts no CA bundle.** The first real `go mod tidy`
failed reaching `proxy.golang.org` — not a devcroft/sandbox denial, the
exact same gap `nix-flake-sample`'s `cargo` hit reaching crates.io: a
nix-built toolchain has no OS trust store to fall back on. Fixed the same
way — `pkgs.cacert` plus `SSL_CERT_FILE`/`NIX_SSL_CERT_FILE` pointing at
its bundle.

**`go build` shells out to `git` — which this devShell doesn't have.** Go
1.18+ auto-embeds VCS info into binaries by detecting a surrounding git
working tree and running `git` to read it. This sample lives inside
devcroft's own repo (a real git working tree), so the detection fires —
but `git` isn't on this minimal devShell's `PATH`, and the failure is
opaque: `error obtaining VCS status: exit status 1`, no mention of `git`
anywhere in it. Fixed with `GOFLAGS = "-buildvcs=false"` in `flake.nix`
rather than adding `git` as a dependency this sample otherwise has no use
for — the binary has no business being stamped with devcroft's own commit
hash anyway.

## `$PWD`, not `self` — same reasoning as nix-flake-sample

`GOPATH`/`GOMODCACHE`/`GOCACHE` all default outside the project root
(`$HOME/go`, `$HOME/.cache/go-build`), which a devcroft-sandboxed session
correctly denies writing to. Redirected via `$PWD` in the `shellHook` —
safe specifically because `provider::nix::capture_activated_env` runs
`nix develop` with the working directory set to the real project root,
so `$PWD` at shellHook time *is* that root. `self` would be the wrong
choice: for a local/path flake, nix copies the source into its own
read-only store and `self` resolves there, not to the working directory
on disk. See nix-flake-sample's README for the fuller version of this
same point (it made the identical choice for `CARGO_HOME`).

## The listen-socket gap, hit directly

This is the first devcroft sample that's actually a *server*, and it ran
straight into `docs/ssh-validation.md`'s own tracked, highest-priority
gap: **no dev server can bind a port under the default policy.**
Confirmed directly — under `network.default = "deny"` (the manifest
default), the server starts and immediately fails:

```
[GIN-debug] [ERROR] listen tcp :8080: bind: operation not permitted
```

not a Gin or Go problem — `network.block: true` denies `bind`/`listen`
outright, loopback included. `devcroft.toml` here sets
`network.default = "allow"`, the documented workaround, with a comment
pointing back at the tracked gap rather than presenting it as this
sample's own default network posture.

## Try it

```sh
cd samples/nix-go-sample
devcroft up
devcroft exec -- go build -o hello-server .
devcroft exec -- sh -c './hello-server & sleep 1; curl localhost:8080/; curl localhost:8080/health; kill %1'
devcroft policy --render                  # shows /nix/store with origin provider:nix
devcroft down
```

`go build` needs no network at all — `go mod tidy`'s resolution already
happened once, host-side, and every `up` re-runs only `go mod download`
against the committed `go.sum`, before the sandbox restriction applies.
