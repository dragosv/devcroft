{
  description = "devcroft nix-go-sample: hello-world Gin web server";

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
          packages = [ pkgs.go pkgs.cacert ];

          # Same nixpkgs-has-no-CA-bundle-by-default gotcha
          # nix-flake-sample's own cargo hit — `go mod download` needs TLS
          # to reach proxy.golang.org/sum.golang.org just like cargo needs
          # it for crates.io.
          SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
          NIX_SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

          # `go build` auto-embeds VCS info (Go 1.18+) by shelling out to
          # `git` when it detects the module sits inside a git working
          # tree — true here, since this sample is a subdirectory of
          # devcroft's own repo, not its own. This devShell doesn't
          # install git (nothing else here needs it), so that probe fails
          # with an opaque "error obtaining VCS status: exit status 1"
          # instead of a clear "git not found". Disabling VCS stamping is
          # the correct fix either way: this sample's binary has no
          # business being tagged with devcroft's own commit hash.
          #
          # Not a plain `GOFLAGS` env var: verified live (own-policy-
          # baseline task 2.4) that `nono wrap` silently drops an
          # environment variable named exactly `GOFLAGS` from the wrapped
          # process — every other var checked (`GOPATH`, `GOCACHE`, an
          # arbitrary custom name) survives unchanged, so this is nono's
          # own behavior, not a devcroft policy effect, and not something
          # `groups.exclude` touches. `GOENV` does survive, so this routes
          # through Go's own persistent-config mechanism instead — set in
          # `shellHook` below, host-side, before restriction.

          # Go's defaults (`$HOME/go` for GOPATH, `$HOME/.cache/go-build`
          # for GOCACHE) are outside the project root, so a
          # devcroft-sandboxed session correctly denies writing there —
          # same shape as flox-clap-sample's CARGO_HOME and
          # nix-flake-sample's CARGO_HOME redirect. `$PWD`, not `self`,
          # for the same reason those use it: `provider::nix` runs `nix
          # develop` with the working directory set to the real project
          # root, so `$PWD` at shellHook time *is* that root, while
          # `self` for a local/path flake resolves to nix's own
          # read-only store copy of the source.
          #
          # `go mod download` then populates the module cache from
          # `go.sum`'s pinned checksums, host-side, at `up`, before the
          # sandbox restriction — so `devcroft exec -- go build` never
          # needs network access at session time.
          shellHook = ''
            export GOPATH="$PWD/.go"
            export GOMODCACHE="$GOPATH/pkg/mod"
            export GOCACHE="$PWD/.gocache"
            export GOENV="$PWD/.goenv"
            mkdir -p "$GOMODCACHE" "$GOCACHE"
            go env -w GOFLAGS=-buildvcs=false
            go mod download
          '';
        };
    in {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = { default = shellFor system; };
      }) systems);
    };
}
