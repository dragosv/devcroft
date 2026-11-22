# A Go service with its whole toolchain from pixi

A small HTTP service (`catalogd`, standard library only) with the tooling a
real Go project carries:

- `gofmt`, `go vet` and `go test`;
- `golangci-lint`;
- `make` as the task runner;
- `git`;
- a smoke test that serves on loopback and checks the JSON with `curl` and
  `jq`.

**Every one of those tools comes from conda-forge through pixi.** devcroft
grants no host userland, so `pixi.toml` also declares `coreutils`, `grep` and
`sed`, which the Makefile's shell lines use.

Locked for **macOS ARM64** and **Linux glibc ARM64**. Needs pixi **0.84.2 or
later** on the host.

```sh
devcroft up
devcroft exec -- make tools     # where each tool resolves: all in the prefix
devcroft exec -- make check     # fmt, vet, golangci-lint, test, build
devcroft exec -- make smoke     # serve on 127.0.0.1:18080, query with curl + jq
devcroft policy --render
devcroft rm --yes
```

Measured on macOS ARM64 with pixi 0.84.2: `make check` reports `0 issues.`
and passes the tests, and `make smoke` prints `smoke ok: …`.
`tests/pixi_provider_e2e.rs` runs the same steps on macOS and Linux.

## Choices this sample makes, and why

| Setting | Why |
|---|---|
| `CGO_ENABLED = "0"` in `[activation.env]` | Inside the sandbox, macOS has no usable C toolchain: conda-forge's clang needs the host SDK, and the host's `cc` is not reachable. Pure Go builds the same on both platforms. |
| `GOPROXY = "off"` | Sessions have no network, so a module download fails at once instead of timing out. The service uses the standard library only. To use modules, commit a `vendor/` directory. |
| `GOFLAGS = "-buildvcs=false"` | `go build` would otherwise ask `git` about the enclosing repository, which lies outside the sandbox's grants. |
| `GOBIN` set in `scripts/activate.sh` | `go install` writes into the project's `.bin/`, since the prefix is read-only. The script runs once, inside the sandbox. |
| `ports = [18080]` in `devcroft.toml` | A sandbox may listen only on the ports its manifest declares. Without it, `catalogd` fails with `bind: operation not permitted`. |

`GOTOOLCHAIN=local` needs no setting: conda-forge's `go` sets it. A `go.mod`
asking for a newer Go fails with `go.mod requires go >= …` rather than trying
to download a toolchain.

## What is not available

- **Tasks by name.** `pixi run check` does not work inside: the host's pixi
  is not granted. The `Makefile` holds the same targets, and the `[tasks]`
  entries are only for use outside the sandbox.
- **cgo on macOS.** There is no compiler that can link. On Linux, adding
  `c-compiler` to `[dependencies]` gives a `gcc` with its own sysroot; a C
  build with it was measured, but cgo through this sample was not.
- **The host's `git`, `make` or `curl`.** `/usr/bin/git` is refused, exit
  126. The declared ones are used.

## What `up` prints

On Linux, `up` warns that some packages want host libraries outside the
qualified inventory: `git`'s perl and Kerberos tail, and anything linking X11.
They are not granted, and nothing in `make check` or `make smoke` loads them.
On macOS the baseline already covers `/usr/lib` and `/System/Library`, so
there is no warning.
