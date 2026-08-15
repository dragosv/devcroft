{
  description = "devcroft nix-flake-sample: citytime CLI, offline-built via nix + cargo fetch";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      # No `builtins.currentSystem` here: nix flakes evaluate pure by
      # default, and `provider::nix` (add-nix-provider) deliberately never
      # passes `--impure` — `currentSystem` isn't even defined under pure
      # evaluation. Enumerating the four platforms devcroft supports and
      # letting the `nix develop`/`nix flake` CLI itself pick the matching
      # `devShells.<system>` entry is the standard workaround, same as the
      # provider's own test fixtures.
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];

      shellFor = system:
        let pkgs = import nixpkgs { inherit system; };
        in pkgs.mkShell {
          packages = [ pkgs.cargo pkgs.rustc pkgs.clippy pkgs.rustfmt pkgs.cacert ];

          # nixpkgs' cargo/curl trusts no CA bundle by default — unlike a
          # system-installed toolchain (Homebrew, apt, ...), which links
          # against the OS trust store, a nix-built one has no such
          # fallback and every TLS connection fails with "unable to get
          # local issuer certificate" until told where to look. This is
          # the standard fix (`pkgs.cacert`'s bundle), not devcroft- or
          # sandbox-specific — confirmed by reproducing the same failure
          # with a bare `nix develop --command cargo fetch`, no devcroft
          # involved at all.
          SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
          NIX_SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

          # cargo defaults CARGO_HOME to the *host's* location (~/.cargo or
          # similar) — outside this project, so a devcroft-sandboxed
          # session correctly denies writing there. Redirected into the
          # project instead, same fix flox-clap-sample's `[hook]
          # on-activate` makes for flox — but there `$FLOX_ENV_PROJECT` is
          # a variable flox itself provides; nix has no equivalent, so
          # this uses `$PWD` instead. That's safe specifically because
          # `provider::nix::capture_activated_env` runs `nix develop`
          # with `.current_dir(project_root)`, so `$PWD` at shellHook
          # time is the real project root — *not* `self`, which for a
          # local/path flake resolves to nix's own read-only store copy
          # of the source, not the working directory on disk.
          #
          # `cargo fetch` then downloads this crate's real crates.io
          # dependencies (clap, chrono, chrono-tz) into that project-local
          # registry cache, host-side, at `up`, before the sandbox
          # restriction — so `devcroft exec -- cargo build` never needs
          # network access at session time. Runs once per `up`, same
          # cadence as flox's `on-activate` (this provider's own
          # `capture_activated_env` calls `nix develop` exactly once at
          # `up`, never per session).
          shellHook = ''
            export CARGO_HOME="$PWD/.cargo"
            mkdir -p "$CARGO_HOME"
            cargo fetch --manifest-path "$PWD/Cargo.toml"
          '';
        };
    in {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = { default = shellFor system; };
      }) systems);
    };
}
