{
  description = "devcroft nix-probe-sample: what the sandbox refuses";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      # See samples/nix-flake-sample's own flake.nix for why this is a
      # static list rather than `builtins.currentSystem` (unavailable
      # under nix's pure evaluation, which `provider::nix` never
      # overrides with `--impure`).
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];

      shellFor = system:
        let pkgs = import nixpkgs { inherit system; };
        in pkgs.mkShell {
          # `go` alone. No `cacert`, unlike nix-go-sample: that sample
          # fetches Gin and needs a CA bundle a nix-built toolchain
          # doesn't otherwise have, while this one imports `fmt` and `os`
          # and nothing else. Same reason there is no `go.sum` here -- a
          # probe that measures the sandbox boundary should not also
          # depend on the network being reachable to build.
          packages = [ pkgs.go ];

          # Everything below is a `mkShell` *attribute*, not a
          # `shellHook` export, and that distinction is the whole reason
          # this sample builds.
          #
          # `provider::nix` resolves the environment with `nix
          # print-dev-env --json` and hands the `shellHook` back as inert
          # data it never evaluates -- deliberately, because a shellHook
          # is project code and the two-phase execution invariant says
          # provisioning runs pinned tooling, not project code
          # (fix-provisioning-hooks; `src/provider/nix.rs`). Measured
          # here rather than assumed: with these same settings written as
          # `shellHook` exports, `GOPATH`/`GOCACHE`/`GOENV` all arrive
          # empty inside the sandbox and no `.go`/`.gocache` directory is
          # ever created. Attributes become real variables in
          # `print-dev-env --json`'s output, so they survive; exports in
          # a shellHook do not. nix-flake-sample and nix-go-sample both
          # document their redirects as shellHook exports.
          #
          # The values are absolute literals rather than `$PWD`-relative
          # for the same reason: nothing expands them. A `mkShell`
          # attribute is set verbatim, so `$PWD` would arrive as four
          # literal characters. `/tmp` is the target because
          # `devcroft.toml` already has to grant it for `go build`'s work
          # directory anyway.
          # `GOTMPDIR`, not `TMPDIR`. Go's build work directory honours
          # `GOTMPDIR` first, and nix does not set it -- whereas a
          # `mkShell` attribute named `TMPDIR` is overwritten by nix's
          # own stdenv before `print-dev-env` reports the environment
          # (measured: `GOCACHE`, `GOFLAGS` and `GOENV` set the same way
          # all survive; `TMPDIR` comes through as nix's build dir
          # regardless). `devcroft.toml`'s `[env.vars]` is not an
          # alternative either -- it parses and validates but nothing
          # consumes it, so setting `TMPDIR` there is a silent no-op.
          GOTMPDIR = "/tmp";

          # nix's own `print-dev-env` sets `TMPDIR`, `TMP`, `TEMPDIR` and
          # `NIX_BUILD_TOP` to the per-invocation build directory it used
          # at `up` -- e.g. `/nix/var/nix/builds/nix-71740-3772453530`.
          # That path is gone by session time *and* lives under a denied
          # prefix, so Go fails before it compiles anything: `go:
          # creating work dir: stat /nix/var/nix/builds/...: no such file
          # or directory`. Overriding `TMPDIR` here is what fixes it.
          # Go's defaults live under `$HOME`, which is denied -- and
          # under this provider `$HOME` is nix's own `/homeless-shelter`
          # (see README), so leaving them alone fails for two independent
          # reasons at once.
          GOPATH = "/tmp/devcroft-nix-probe-sample/go";
          GOCACHE = "/tmp/devcroft-nix-probe-sample/gocache";

          # `off` rather than a path: Go's persistent config defaults to
          # `$HOME/.config/go/env`, and this sample has no use for one.
          GOENV = "off";

          # This sample sits inside devcroft's own git tree, so Go 1.18+
          # VCS stamping fires and shells out to a `git` this minimal
          # devShell doesn't install -- failing opaquely with "error
          # obtaining VCS status: exit status 1". nix-go-sample documents
          # `nono` dropping a variable named exactly `GOFLAGS` under the
          # old `nono wrap` execution model; verified live here under the
          # current library-linked model (use-nono-library) that it
          # arrives intact, so this is set as a plain variable.
          GOFLAGS = "-buildvcs=false";
        };
    in {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = { default = shellFor system; };
      }) systems);
    };
}
