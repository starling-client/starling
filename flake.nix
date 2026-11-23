{
  description = "Sunrise: Minecraft Java Edition 26.3 rewritten in Rust";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      linuxSystems = [ "x86_64-linux" "aarch64-linux" ];
      systems = linuxSystems ++ [ "aarch64-darwin" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      forLinuxSystems = f: nixpkgs.lib.genAttrs linuxSystems (system: f nixpkgs.legacyPackages.${system});

      # The game's packages, built from this workspace (nix/, docs/nix.md). Releases are tagged by
      # release.yml, which never uses this flake, so a build from it says which commit it is.
      version = "0-unstable-${builtins.substring 0 8 (self.lastModifiedDate or "19700101")}";
      gamePackages = pkgs:
        let
          call = pkgs.lib.callPackageWith (pkgs // packages // { inherit version; });
          packages = {
            sunrise-engine = call ./nix/sunrise-engine.nix { };
            sunrise-server = call ./nix/sunrise-server.nix { };
            sunrise-prepare = call ./nix/sunrise-prepare.nix { };
            sunrise-client = call ./nix/sunrise-client.nix { };
          };
        in packages;
    in {
      packages = forAllSystems (pkgs: gamePackages pkgs // nixpkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux (
        let
          version = "0.2.0";
          pname = "sunrise-launcher";

          unwrapped = pkgs.rustPlatform.buildRustPackage {
            pname = "${pname}-unwrapped";
            inherit version;

            src = self;
            cargoRoot = "launcher";
            buildAndTestSubdir = "launcher";
            cargoLock = {
              lockFile = ./launcher/Cargo.lock;
            };

            nativeBuildInputs = with pkgs; [
              pkg-config
            ];

            buildInputs = with pkgs; [
              libxkbcommon
              wayland
              libx11
              libxcb
              fontconfig
              vulkan-loader
              alsa-lib
            ];

            doCheck = false;
          };

          desktopItem = pkgs.makeDesktopItem {
            name = pname;
            desktopName = "Sunrise";
            genericName = "Game launcher";
            comment = "Install, prepare and play Sunrise, Minecraft: Java Edition rewritten in Rust";
            exec = pname;
            icon = "sunrise";
            categories = [ "Game" ];
            startupWMClass = "sunrise-launcher";
          };

          sunriseLauncher = pkgs.buildFHSEnv {
            inherit pname version;

            targetPkgs = pkgs: with pkgs; [
              # Windowing, graphics, Vulkan, Wayland, X11
              vulkan-loader
              libglvnd
              wayland
              libxkbcommon
              libx11
              libxcb
              libxcursor
              libxext
              libxi
              libxrandr
              libxrender
              libxxf86vm

              # Audio, udev & dev packages for native builds
              udev
              udev.dev
              alsa-lib
              alsa-lib.dev

              # Desktop integration, fonts, keyring
              fontconfig
              freetype
              dbus
              xdg-utils
              procps

              # Mojang Java runtime dependencies
              zlib
              libxtst

              # Gamemode support
              gamemode

              # Build environment for native mods
              gcc
              glibc.dev
              pkg-config
            ];

            runScript = "${unwrapped}/bin/sunrise-launcher";

            profile = ''
              export SUNRISE_LAUNCHER_NO_SELF_UPDATE=1
              export SUNRISE_LAUNCHER_DESKTOP_EXEC="''${SUNRISE_LAUNCHER_DESKTOP_EXEC:-${pname}}"
              export PKG_CONFIG_PATH="/usr/lib/pkgconfig:/usr/share/pkgconfig:''${PKG_CONFIG_PATH:-}"
            '';

            extraInstallCommands = ''
              install -Dm444 ${desktopItem}/share/applications/${pname}.desktop -t $out/share/applications
              if [ -d ${./packaging/icons/hicolor} ]; then
                mkdir -p $out/share/icons/hicolor
                cp -r ${./packaging/icons/hicolor}/. $out/share/icons/hicolor/
              fi
            '';

            passthru = { inherit unwrapped; };

            meta = with pkgs.lib; {
              description = "Launcher for Sunrise, Minecraft: Java Edition rewritten in Rust";
              homepage = "https://playsunrise.net";
              license = licenses.mit;
              platforms = [ "x86_64-linux" "aarch64-linux" ];
              mainProgram = pname;
            };
          };
        in {
          sunrise-launcher = sunriseLauncher;
          default = sunriseLauncher;
        })
      );

      overlays.default = final: prev: {
        sunrise-launcher = self.packages.${final.stdenv.hostPlatform.system}.sunrise-launcher;
      } // gamePackages final;

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
