{
  description = "devcroft kotlin-ktor-sample: hello-world Ktor web server";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      # See nix-flake-sample's own flake.nix for why this is a static
      # list rather than `builtins.currentSystem` (unavailable under
      # nix's pure evaluation, which `provider::nix` never overrides
      # with `--impure`).
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];

      shellFor = system:
        let pkgs = import nixpkgs { inherit system; };
        in pkgs.mkShell {
          packages = [ pkgs.jdk21 pkgs.gradle pkgs.cacert ];

          # Same nixpkgs-has-no-CA-bundle-by-default gotcha nix-flake-sample's
          # cargo and nix-go-sample's go both hit — Gradle needs TLS to
          # resolve dependencies from Maven Central.
          SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
          NIX_SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

          # Gradle's default GRADLE_USER_HOME ($HOME/.gradle) is outside
          # the project root, so a devcroft-sandboxed session correctly
          # denies writing there — same shape as nix-go-sample's GOPATH
          # and nix-flake-sample's CARGO_HOME. `$PWD`, not `self`, for
          # the identical reason those use it: `provider::nix` runs `nix
          # develop` with the working directory set to the real project
          # root, so `$PWD` at shellHook time *is* that root, while
          # `self` for a local/path flake resolves to nix's own
          # read-only store copy of the source.
          #
          # `gradle --no-daemon build` in the shellHook resolves and
          # caches every dependency (Ktor, Netty, logback, and their full
          # transitive graphs) into that redirected GRADLE_USER_HOME,
          # host-side, at `up`, before the sandbox restriction applies —
          # the same "resolve once, with real network, before
          # restriction" shape go.sum/Cargo.lock give the other samples,
          # just without a single committed lockfile: Gradle's module
          # cache is content-addressed by checksum the same way, it is
          # just keyed by this project's build script rather than a
          # separate lockfile artifact. `devcroft exec -- gradle` never
          # needs network access at session time as a result.
          shellHook = ''
            export GRADLE_USER_HOME="$PWD/.gradle-home"
            mkdir -p "$GRADLE_USER_HOME"
            gradle --no-daemon build
          '';
        };
    in {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = { default = shellFor system; };
      }) systems);
    };
}
