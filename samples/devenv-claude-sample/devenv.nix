{ pkgs, ... }:

{
  # The agent, from the closure: pinned by devenv.lock, reproducible, and
  # declared here rather than installed by hand inside the sandbox.
  #
  # Everything after it is what the agent's own shell tool runs. No closure
  # provides these by default, and the host's are denied inside the sandbox,
  # so an agent without them cannot so much as `ls` (measured; see README).
  packages = [
    pkgs.claude-code
    pkgs.bashInteractive
    pkgs.coreutils
    pkgs.findutils
    pkgs.gnugrep
    pkgs.gnused
    pkgs.git
  ];
}
