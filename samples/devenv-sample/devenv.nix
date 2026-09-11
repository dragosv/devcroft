{ pkgs, ... }:

{
  # Two packages, so a session can prove it is running the closure's
  # tooling rather than the host's.
  packages = [ pkgs.ripgrep pkgs.jq ];

  # Declared environment. Captured host-side at `devcroft up`, before any
  # restriction applies — the same phase flox, nix and devbox capture in.
  env.DEVCROFT_SAMPLE = "devenv";

  # Project code. devcroft never runs this on the host: it is captured as
  # data at `up` and executed *inside* the sandbox, after restriction.
  # The marker below is what makes that visible — see README.md.
  enterShell = ''
    mkdir -p .devcroft-sample
    date -u +%Y-%m-%dT%H:%M:%SZ > .devcroft-sample/enter-shell-ran
  '';
}
