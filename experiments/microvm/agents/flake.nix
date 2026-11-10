{
  description = "Isolated, pinned Codex and Claude Code microVM probe";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/0e251e24a4f24e036a084b6b4b2d2491af4167f4";
  outputs = { self, nixpkgs }:
    let
      systems = [ "aarch64-linux" "x86_64-linux" ];
      fixture = system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config.allowUnfreePredicate = pkg: builtins.elem (nixpkgs.lib.getName pkg) [ "claude-code" ];
          };
        in {
          packages.default = pkgs.buildEnv {
            name = "devcroft-microvm-agents";
            paths = with pkgs; [
              codex claude-code gitMinimal bashInteractive coreutils gcc gnumake redis
              gnutar gzip findutils diffutils gnugrep gnused gawk cacert curl python3
            ];
            pathsToLink = [ "/bin" "/lib" "/share" "/etc/ssl" ];
          };
          versions = {
            codex = pkgs.codex.version;
            claudeCode = pkgs.claude-code.version;
            gcc = pkgs.gcc.version;
            redis = pkgs.redis.version;
          };
        };
    in {
      packages = builtins.listToAttrs (map (system: { name = system; value = (fixture system).packages; }) systems);
      fixtureVersions = builtins.listToAttrs (map (system: { name = system; value = (fixture system).versions; }) systems);
    };
}
