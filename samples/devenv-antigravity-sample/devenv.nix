{ pkgs, ... }:

{
  # The agent, from the closure (pinned by devenv.lock), then the tools its
  # shell runs: no closure provides them unless asked, and the host's are
  # denied inside the sandbox.
  packages = [
    pkgs.antigravity-cli
    pkgs.bashInteractive
    pkgs.coreutils
    pkgs.findutils
    pkgs.gnugrep
    pkgs.gnused
    pkgs.git
  ];

  # API-key mode is a setting, not an environment variable: without
  # `modelProvider = "gemini"` agy ignores GEMINI_API_KEY and starts a
  # browser sign-in instead (measured). Written into the sandbox's own
  # home, inside the sandbox, never into yours.
  enterShell = ''
    mkdir -p "$HOME/.gemini/antigravity-cli"
    settings="$HOME/.gemini/antigravity-cli/settings.json"
    [ -e "$settings" ] || echo '{"modelProvider":"gemini"}' > "$settings"
  '';
}
