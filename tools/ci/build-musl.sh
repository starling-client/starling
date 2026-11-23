#!/bin/sh
# Called inside rust:alpine3.22 by game-build.yml, without release credentials.
set -eu

apk add --no-cache build-base pkgconf linux-headers alsa-lib-dev eudev-dev python3 git bash
git config --global --add safe.directory /work
# Match the shared release/SDK pin, not the image's bundled Rust or the moving stable channel.
bash tools/ci/rustup_install.sh --profile minimal --component llvm-tools
export RUSTUP_TOOLCHAIN="$(cat tools/ci/rust-version)"

# Static musl cannot dlopen the graphics/window libraries. Keep audio and controllers,
# and link against Alpine's native libraries rather than Ubuntu's glibc libraries.
export RUSTFLAGS='-C target-feature=-crt-static'
target=x86_64-unknown-linux-musl
bin="target/$target/release"
cargo build --release --locked --target "$target" \
  -p sunrise-official --bin sunrise-official -p sunrise-client-lib --bin sunrise-ctl \
  -p sunrise-prepare --bin sunrise-prepare -p sunrise-server --bin sunrise-server
# The official game (sunrise-client's with Sunrise's own parts, docs/layers.md) is what players
# get, under the game's name.
cp -f "$bin/sunrise-official" "$bin/sunrise-client"

set -- "$bin/sunrise-client" "$bin/sunrise-ctl" "$bin/sunrise-prepare" "$bin/sunrise-server"
python3 tools/check_imports.py --musl "$@"
python3 tools/check_cpu_baseline.py "$@"

# Even --help must never touch real player data.
export SUNRISE_HOME="$(mktemp -d)"
export SUNRISE_LAUNCHER_HOME="$(mktemp -d)"
export XDG_DATA_HOME="$(mktemp -d)"
export XDG_CONFIG_HOME="$(mktemp -d)"
export SUNRISE_LAUNCHER_NO_SELF_UPDATE=1 SUNRISE_NO_AUDIO=1
"$bin/sunrise-client" --help > "$SUNRISE_HOME/client-help"
grep -q -- '--resources' "$SUNRISE_HOME/client-help"
"$bin/sunrise-ctl" --help > /dev/null
"$bin/sunrise-prepare" --help > "$SUNRISE_HOME/prepare-help"
grep -q -- '--minecraft-dir' "$SUNRISE_HOME/prepare-help"
for binary in sunrise-prepare sunrise-server; do
  test "$("$bin/$binary" --version)" = "$binary $SUNRISE_VERSION"
done

cargo run --locked --target "$target" -p xtask -- package \
  "$bin/sunrise-client" dist/sunrise-linux-musl-x86_64
tar -C dist -czf dist/sunrise-linux-musl-x86_64.tar.gz sunrise-linux-musl-x86_64
