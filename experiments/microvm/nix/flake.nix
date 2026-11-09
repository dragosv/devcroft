{
  description = "Pinned Linux closure for the isolated microVM experiment";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/0e251e24a4f24e036a084b6b4b2d2491af4167f4";
  outputs = { self, nixpkgs }:
    let
      systems = [ "aarch64-linux" "x86_64-linux" ];
      forSystem = system:
        let
          pkgs = import nixpkgs { inherit system; };
          tools = with pkgs; [
            bashInteractive coreutils gcc gnumake redis code-server
            gnutar gzip findutils diffutils gnugrep gnused gawk cacert
          ];
        in {
          versions = {
            gcc = pkgs.gcc.version;
            redis = pkgs.redis.version;
            codeServer = pkgs.code-server.version;
          };
          packages.default = pkgs.buildEnv {
            name = "devcroft-microvm-closure";
            paths = tools;
            pathsToLink = [ "/bin" "/lib" "/share" ];
          };
          devShells.default = pkgs.mkShell {
            packages = tools;
            # No activation hook: preparing this fixture does not run project code.
          };
        };
    in {
      packages = builtins.listToAttrs (map (system: { name = system; value = (forSystem system).packages; }) systems);
      devShells = builtins.listToAttrs (map (system: { name = system; value = (forSystem system).devShells; }) systems);
      fixtureVersions = builtins.listToAttrs (map (system: { name = system; value = (forSystem system).versions; }) systems);
    };
}
