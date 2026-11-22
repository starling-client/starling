# The server build of the game (`sunrise-client --no-default-features`): the game without sound,
# controllers or Sunrise's online extras, so it links neither ALSA nor udev. sunrise-server runs it
# headless as `sunrise-engine` (docs/hosting.md, packaging/server/Dockerfile).
{
  lib,
  rustPlatform,
  pkg-config,
  callPackage,
  version,
  polyformShield,
}:
rustPlatform.buildRustPackage {
  pname = "sunrise-engine";
  inherit version;
  src = callPackage ./source.nix { };
  cargoLock.lockFile = ../Cargo.lock;

  cargoBuildFlags = [
    "--package=sunrise-client"
    "--bin=sunrise-client"
  ];
  buildNoDefaultFeatures = true;

  nativeBuildInputs = [ pkg-config ];

  # The game's tests read the prepared vanilla files in .local/, which a build never has.
  doCheck = false;

  postInstall = ''
    mv $out/bin/sunrise-client $out/bin/sunrise-engine
  '';

  meta = {
    description = "Sunrise's headless game engine, run by sunrise-server";
    homepage = "https://playsunrise.net";
    license = polyformShield; # flake.nix
    mainProgram = "sunrise-engine";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
