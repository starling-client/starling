# sunrise-prepare: builds the game files Sunrise reads from the player's own Minecraft 26.3 and the
# server jar they downloaded themselves. Nothing of Mojang's is fetched or packaged here.
#
# It works in a "root": research-lock.json (the pinned jars' hashes), the exporters' Java sources it
# compiles against those jars, and a writable .local/ for its scratch files. The store is read-only, so
# the package keeps that root in share/sunrise/prepare and bin/sunrise-prepare (sunrise-prepare.sh)
# copies it somewhere writable unless --root is given. The exporters need a JDK of the pinned Java
# version (25): the default JAVA_HOME.
{
  lib,
  stdenv,
  rustPlatform,
  callPackage,
  coreutils,
  jdk25_headless,
  version,
  polyformShield,
}:
rustPlatform.buildRustPackage {
  pname = "sunrise-prepare";
  inherit version;
  src = callPackage ./source.nix { };
  cargoLock.lockFile = ../Cargo.lock;

  cargoBuildFlags = [
    "--package=sunrise-prepare"
    "--bin=sunrise-prepare"
  ];

  doCheck = false;

  postInstall = ''
    root=$out/share/sunrise/prepare
    install -Dm444 research-lock.json -t $root
    install -Dm444 crates/sunrise-prepare/src/exporters.rs -t $root/crates/sunrise-prepare/src
    mkdir -p $root/mods/sunrise/src/test
    cp -r mods/sunrise/src/test/java $root/mods/sunrise/src/test/

    mkdir -p $out/libexec
    mv $out/bin/sunrise-prepare $out/libexec/sunrise-prepare
    substitute ${./sunrise-prepare.sh} $out/bin/sunrise-prepare \
      --subst-var out \
      --subst-var-by shell ${stdenv.shell} \
      --subst-var-by coreutils ${coreutils} \
      --subst-var-by java ${jdk25_headless}
    chmod +x $out/bin/sunrise-prepare
  '';

  passthru.java = jdk25_headless;

  meta = {
    description = "Prepare Sunrise's game files from your own copy of Minecraft: Java Edition 26.3";
    homepage = "https://playsunrise.net";
    license = polyformShield; # flake.nix
    mainProgram = "sunrise-prepare";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
