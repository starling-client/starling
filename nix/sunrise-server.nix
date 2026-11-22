# sunrise-server: server.toml, the CLI and the supervisor. It finds the game beside itself, so the
# engine is linked into the same bin/ (crates/sunrise-server/src/engine.rs `program_in`).
{
  lib,
  rustPlatform,
  pkg-config,
  callPackage,
  sunrise-engine,
  version,
  polyformShield,
}:
rustPlatform.buildRustPackage {
  pname = "sunrise-server";
  inherit version;
  src = callPackage ./source.nix { };
  cargoLock.lockFile = ../Cargo.lock;

  cargoBuildFlags = [
    "--package=sunrise-server"
    "--bin=sunrise-server"
  ];

  nativeBuildInputs = [ pkg-config ];

  doCheck = false;

  postInstall = ''
    ln -s ${lib.getExe sunrise-engine} $out/bin/sunrise-engine
  '';

  passthru.engine = sunrise-engine;

  meta = {
    description = "Host a Minecraft: Java Edition 26.3 server with Sunrise, the game rewritten in Rust";
    longDescription = ''
      Holds none of Mojang's files: the server reads the game files you prepare from your own copy of
      Minecraft 26.3 with sunrise-prepare (docs/hosting.md).
    '';
    homepage = "https://playsunrise.net";
    license = polyformShield; # flake.nix
    mainProgram = "sunrise-server";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
