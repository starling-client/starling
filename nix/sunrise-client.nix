# The game (sunrise-client, with sound, controllers and Sunrise's extras) and sunrise-ctl, the tool
# that drives it (docs/agent-control.md). Start it with `sunrise-client --resources DIR`, DIR being
# what sunrise-prepare wrote.
{
  lib,
  stdenv,
  rustPlatform,
  pkg-config,
  callPackage,
  alsa-lib,
  udev,
  vulkan-loader,
  wayland,
  libxkbcommon,
  libx11,
  libxcursor,
  libxi,
  libxrandr,
  libxcb,
  version,
  polyformShield,
}:
let
  # wgpu and winit open these with dlopen, so the linker never records them.
  dlopened = [
    vulkan-loader
    wayland
    libxkbcommon
    libx11
    libxcursor
    libxi
    libxrandr
    libxcb
  ];
in
rustPlatform.buildRustPackage {
  pname = "sunrise-client";
  inherit version;
  src = callPackage ./source.nix { };
  cargoLock.lockFile = ../Cargo.lock;

  cargoBuildFlags = [
    "--package=sunrise-client"
    "--bin=sunrise-client"
    "--package=sunrise-client-lib"
    "--bin=sunrise-ctl"
  ];

  nativeBuildInputs = [ pkg-config ];
  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
    alsa-lib
    udev
  ];

  doCheck = false;

  postFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
    patchelf --add-rpath ${lib.makeLibraryPath dlopened} $out/bin/sunrise-client
  '';

  meta = {
    description = "Sunrise, Minecraft: Java Edition 26.3 rewritten in Rust, and sunrise-ctl";
    homepage = "https://playsunrise.net";
    license = polyformShield; # flake.nix
    mainProgram = "sunrise-client";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
