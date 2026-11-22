{
  description = "Sunrise: Minecraft Java Edition 26.3 rewritten in Rust";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      linuxSystems = [ "x86_64-linux" "aarch64-linux" ];
      systems = linuxSystems ++ [ "aarch64-darwin" ];
      # Sunrise's licence (LICENSE, LICENSING.md), for the packages' meta. nixpkgs has no entry for it. Its terms
      # restrict use, so like the source-available licences nixpkgs lists (bsl11, elastic20, sspl) it is not free in
      # nixpkgs' sense; it lets anyone pass Sunrise on with its notice, so redistributable, as bsl11 and sspl are.
      polyformShield = {
        fullName = "PolyForm Shield License 1.0.0";
        spdxId = "PolyForm-Shield-1.0.0";
        url = "https://polyformproject.org/licenses/shield/1.0.0/";
        free = false;
        redistributable = true;
      };
      # The packages under that licence. The flake's nixpkgs allows exactly these as unfree, so its own builds need no
      # allowUnfree; every other package keeps nixpkgs' default.
      sunrisePackages = [ "sunrise-engine" "sunrise-server" "sunrise-prepare" "sunrise-client" ];
      pkgsFor = system: import nixpkgs {
        inherit system;
        config.allowUnfreePredicate = pkg: builtins.elem (nixpkgs.lib.getName pkg) sunrisePackages;
      };
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f (pkgsFor system));
      forLinuxSystems = f: nixpkgs.lib.genAttrs linuxSystems (system: f (pkgsFor system));

      # The game's packages, built from this workspace (nix/, docs/nix.md). Releases are tagged by
      # release.yml, which never uses this flake, so a build from it says which commit it is.
      version = "0-unstable-${builtins.substring 0 8 (self.lastModifiedDate or "19700101")}";
      gamePackages = pkgs:
        let
          call = pkgs.lib.callPackageWith (pkgs // packages // { inherit version polyformShield; });
          packages = {
            sunrise-engine = call ./nix/sunrise-engine.nix { };
            sunrise-server = call ./nix/sunrise-server.nix { };
            sunrise-prepare = call ./nix/sunrise-prepare.nix { };
            sunrise-client = call ./nix/sunrise-client.nix { };
          };
        in packages;
    in {
      packages = forAllSystems gamePackages;

      overlays.default = final: prev: gamePackages final;

      # services.sunrise-server (docs/nix.md).
      nixosModules = {
        sunrise-server = import ./nix/module.nix { inherit self; };
        default = self.nixosModules.sunrise-server;
      };

      checks = forAllSystems (pkgs: {
        inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) sunrise-server sunrise-prepare;
      } // nixpkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
        # The module in a VM, without any of Mojang's files.
        nixos-module = pkgs.testers.runNixOSTest (import ./nix/tests/server.nix { inherit self; });
      });

      devShells = forLinuxSystems (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            coreutils
            findutils
            gnused
            gnugrep
            pkg-config
            rustc
            cargo
            clang
            mold
            python3
          ];
          buildInputs = with pkgs; [
            libxkbcommon
            wayland
            vulkan-loader
            fontconfig
            freetype
            alsa-lib
            openssl
            udev
            libx11
            libxcursor
            libxrandr
            libxi
          ];
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
            libxkbcommon
            wayland
            vulkan-loader
            fontconfig
            freetype
            alsa-lib
            udev
            libx11
            libxcursor
            libxrandr
            libxi
          ]);
        };
      });
    };
}
