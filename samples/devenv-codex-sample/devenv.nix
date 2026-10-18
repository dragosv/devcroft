{ pkgs, ... }:

{
  # The agent, from the closure (pinned by devenv.lock), then the tools its
  # shell runs: no closure provides them unless asked, and the host's are
  # denied inside the sandbox.
  packages = [
    pkgs.codex
    pkgs.bashInteractive
    pkgs.coreutils
    pkgs.findutils
    pkgs.gnugrep
    pkgs.gnused
    pkgs.git
  ];
}
