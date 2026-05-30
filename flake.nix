{
  description = "WhippleScript development environment";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          apalacheVersion = "0.57.1";
        in {
          apalache = pkgs.stdenvNoCC.mkDerivation {
            pname = "apalache";
            version = apalacheVersion;

            src = pkgs.fetchurl {
              url = "https://github.com/apalache-mc/apalache/releases/download/v${apalacheVersion}/apalache-${apalacheVersion}.tgz";
              hash = "sha256-kAMaMzni6Zj1RBMfZGRie7NwQDr6pJjDQClUu2YKRI0=";
            };

            installPhase = ''
              runHook preInstall
              mkdir -p "$out"
              cp -R ./* "$out/"
              runHook postInstall
            '';
          };

          default = self.packages.${system}.apalache;
        });

      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in {
          default = pkgs.mkShell {
            packages = [
              pkgs.jdk21
              pkgs.maude
              self.packages.${system}.apalache
            ];
          };
        });
    };
}
