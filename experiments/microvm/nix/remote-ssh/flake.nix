{
  description = "Trusted Remote-SSH fixture: the microVM closure plus the userland VS Code Server needs";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/0e251e24a4f24e036a084b6b4b2d2491af4167f4";
  outputs = { self, nixpkgs }:
    let
      systems = [ "aarch64-linux" "x86_64-linux" ];
      forSystem = system:
        let
          pkgs = import nixpkgs { inherit system; };
          # buildEnv links none of glibc's own lib/, so the loader and libraries
          # VS Code Server is pointed at get one stable directory of their own.
          vscodeSysroot = pkgs.runCommand "vscode-server-sysroot" { } ''
            mkdir -p $out/lib/vscode-sysroot
            for f in ${pkgs.glibc}/lib/*; do ln -s "$f" $out/lib/vscode-sysroot/; done
            for f in ${pkgs.stdenv.cc.cc.lib}/lib/*; do ln -sfn "$f" $out/lib/vscode-sysroot/; done
            test -e $out/lib/vscode-sysroot/ld-linux-aarch64.so.1 -o -e $out/lib/vscode-sysroot/ld-linux-x86-64.so.2
            test -e $out/lib/vscode-sysroot/libstdc++.so.6
          '';
          tools = with pkgs; [
            bashInteractive coreutils gcc gnumake redis code-server
            gnutar gzip findutils diffutils gnugrep gnused gawk cacert
            # Separately labelled trusted Remote-SSH row only. VS Code Server's
            # prebuilt node expects a glibc loader and libstdc++; on a store-only
            # closure it is pointed at these via VSCODE_SERVER_CUSTOM_GLIBC_* and
            # rewritten by patchelf. getconf/ps/which are what its installer probes.
            vscodeSysroot patchelf getconf procps which
          ];
        in {
          versions = {
            gcc = pkgs.gcc.version;
            redis = pkgs.redis.version;
            codeServer = pkgs.code-server.version;
            glibc = pkgs.glibc.version;
            patchelf = pkgs.patchelf.version;
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
