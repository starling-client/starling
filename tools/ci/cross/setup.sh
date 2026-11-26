#!/usr/bin/env bash
# Puts a Dev release's cross-compiling tools on a Linux Dyna runner's PATH and its build environment in GITHUB_ENV:
#   tools/ci/cross/setup.sh <target>      aarch64-apple-darwin, x86_64-unknown-linux-gnu,
#                                         x86_64-pc-windows-msvc (the game) or x86_64-pc-windows-gnu (the SDK)
# The tools are downloaded once into the runner's /keep (checked against the sha256 below) and reused. The Apple SDK
# (apple-sdk step of the workflow, which sets SDKROOT first) and Microsoft's CRT and Windows SDK (xwin, /keep/xwin, the
# owner accepted its licence) stay in /keep: nothing of them goes into an artifact, a cache or a log.
set -euo pipefail

target="$1"
shims="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
keep="${DYNA_CI_KEEP:-/keep}"
tools="$keep/cross-tools"
mkdir -p "$tools"

# fetch NAME URL SHA256 STRIP: the archive extracted into $tools/NAME, once.
fetch() {
  local name="$1" url="$2" sha="$3" strip="$4"
  if [[ ! -d "$tools/$name" ]]; then
    rm -rf "$tools/$name.tmp"
    mkdir -p "$tools/$name.tmp"
    curl -fsSL --retry 3 -o "$tools/$name.tmp/archive" "$url"
    echo "$sha  $tools/$name.tmp/archive" | sha256sum -c --status - || { echo "::error::$url does not have sha256 $sha"; exit 1; }
    tar -xf "$tools/$name.tmp/archive" -C "$tools/$name.tmp" --strip-components="$strip"
    rm "$tools/$name.tmp/archive"
    mv "$tools/$name.tmp" "$tools/$name"
  fi
}

path() { echo "$1" >> "$GITHUB_PATH"; export PATH="$1:$PATH"; }
env_set() { echo "$1=$2" >> "$GITHUB_ENV"; }

# The pinned Rust (tools/ci/rust-version) and the target's standard library.
if ! command -v rustup > /dev/null; then
  curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
fi
path "${CARGO_HOME:-$HOME/.cargo}/bin"
# `python`, which the workflows call, is python3 on this image.
mkdir -p "$tools/python-shim"
ln -sf "$(command -v python3)" "$tools/python-shim/python"
path "$tools/python-shim"
rustup toolchain install "$(cat "$shims/../rust-version")" --profile minimal --component llvm-tools --component clippy --target "$target"
rustup default "$(cat "$shims/../rust-version")"

# Cargo relinks when the linker's path changes, not its content, and the runner's target persists: the wrappers are
# used from a folder named by their hash, so a changed wrapper relinks what the target holds.
hash=$(cat "$shims"/zcc-* "$shims"/zld-* "$shims"/zar-* "$shims"/mingw-gcc | sha256sum | cut -c1-16)
wrappers="$tools/wrappers-$hash"
if [[ ! -d "$wrappers" ]]; then
  staged=$(mktemp -d "$tools/wrappers.XXXXXX")
  cp "$shims"/zcc-* "$shims"/zld-* "$shims"/zar-* "$shims"/mingw-gcc "$staged/"
  chmod 755 "$staged"
  mv -T "$staged" "$wrappers" 2> /dev/null || rm -rf "$staged"
fi

triple="${target//-/_}"
upper="$(tr a-z A-Z <<< "$triple")"
case "$target" in
  aarch64-apple-darwin | x86_64-unknown-linux-gnu)
    if [[ "$target" == aarch64-apple-darwin ]]; then
      # clang drives the link (zld-macos); zig compiles the C of build scripts.
      fetch llvm-mingw-20261117 https://github.com/mstorsjo/llvm-mingw/releases/download/20261117/llvm-mingw-20261117-ucrt-ubuntu-22.04-x86_64.tar.xz \
        e7a15a49a2ea95e9f3d8a9d0cfabe5052a4b10fb4777cc54de5a8c27b64bb701 1
      path "$tools/llvm-mingw-20261117/bin"
    fi
    fetch zig-0.17.1 https://ziglang.org/download/0.17.1/zig-x86_64-linux-0.17.1.tar.xz 3581863e3cbcfd0fce0efc682ce7e9e93da96668a977791d356a26d2a7e9b17a 1
    path "$tools/zig-0.17.1"
    flavour="${target##*-}"
    [[ "$flavour" == darwin ]] && flavour=macos || flavour=linux
    env_set "CC_$triple" "$wrappers/zcc-$flavour"
    env_set "CARGO_TARGET_${upper}_LINKER" "$wrappers/zld-$flavour"
    if [[ "$flavour" == macos ]]; then
      env_set "AR_$triple" "$wrappers/zar-macos"
    fi
    ;;
  x86_64-pc-windows-msvc)
    fetch llvm-mingw-20261117 https://github.com/mstorsjo/llvm-mingw/releases/download/20261117/llvm-mingw-20261117-ucrt-ubuntu-22.04-x86_64.tar.xz \
      e7a15a49a2ea95e9f3d8a9d0cfabe5052a4b10fb4777cc54de5a8c27b64bb701 1
    fetch cargo-xwin-0.24.1 https://github.com/rust-cross/cargo-xwin/releases/download/v0.24.1/cargo-xwin-v0.24.1.x86_64-unknown-linux-musl.tar.gz \
      207ec5436db138b60d90cb41513ce2e3db4893a9c4e25da6209790c1a2db88cf 0
    # clang-cl is clang started under that name.
    mkdir -p "$tools/clang-cl-shim"
    ln -sf "$(readlink -f "$tools/llvm-mingw-20261117/bin/clang")" "$tools/clang-cl-shim/clang-cl"
    path "$tools/clang-cl-shim"
    path "$tools/cargo-xwin-0.24.1"
    path "$tools/llvm-mingw-20261117/bin"
    export XWIN_CACHE_DIR="$keep/xwin" XWIN_ACCEPT_LICENSE=1 XWIN_ARCH=x86_64
    cargo xwin env --target "$target" > "$RUNNER_TEMP/xwin.env"
    # The environment cargo-xwin would give cargo, for every later step. RUSTFLAGS, which the job sets for the C runtime,
    # replaces the target's own flags, so the target's (the library paths) join it.
    eval "$(grep -E '^(export|unset) ' "$RUNNER_TEMP/xwin.env")"
    while read -r _ name; do
      [[ "$name" == RUSTFLAGS ]] && continue
      env_set "$name" "${!name}"
    done < <(grep -oE '^export [A-Za-z_0-9]+' "$RUNNER_TEMP/xwin.env")
    env_set RUSTFLAGS "${CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS} ${RUSTFLAGS:-}"
    ;;
  x86_64-pc-windows-gnu)
    fetch llvm-mingw-20261117 https://github.com/mstorsjo/llvm-mingw/releases/download/20261117/llvm-mingw-20261117-ucrt-ubuntu-22.04-x86_64.tar.xz \
      e7a15a49a2ea95e9f3d8a9d0cfabe5052a4b10fb4777cc54de5a8c27b64bb701 1
    path "$tools/llvm-mingw-20261117/bin"
    # The GNU standard library imports the MinGW runtime's two DLLs, which the SDK ships beside the game: the
    # packages MSYS2 builds them in. tools/prebuilt_game.py takes their folder from the gcc's name, which compiles nothing.
    runtime="$tools/mingw-runtime"
    if [[ ! -d "$runtime" ]]; then
      rm -rf "$runtime.tmp"
      mkdir -p "$runtime.tmp/bin" "$runtime.tmp/download"
      while read -r package sha; do
        curl -fsSL --retry 3 -o "$runtime.tmp/download/$package" "https://repo.msys2.org/mingw/mingw64/$package"
        echo "$sha  $runtime.tmp/download/$package" | sha256sum -c --status - || { echo "::error::$package does not have sha256 $sha"; exit 1; }
        tar --zstd -xf "$runtime.tmp/download/$package" -C "$runtime.tmp/download"
      done <<'PACKAGES'
mingw-w64-x86_64-gcc-libs-16.2.0-7-any.pkg.tar.zst 2df8a63906d569892b49719fb8ff73b7c5c8abb49bdaccf9a35dc709f182b7e2
mingw-w64-x86_64-libwinpthread-14.0.0.r598.ge3d18511f-1-any.pkg.tar.zst 942a62a3ecafbb4f5a80016685a0e3002a8de903012eed49c213e4f5b693668a
PACKAGES
      cp "$runtime.tmp/download/mingw64/bin/libgcc_s_seh-1.dll" "$runtime.tmp/download/mingw64/bin/libwinpthread-1.dll" "$runtime.tmp/bin/"
      printf '#!/bin/sh\nexit 1\n' > "$runtime.tmp/bin/x86_64-w64-mingw32-gcc"
      chmod +x "$runtime.tmp/bin/x86_64-w64-mingw32-gcc"
      rm -rf "$runtime.tmp/download"
      mv "$runtime.tmp" "$runtime"
    fi
    path "$runtime/bin"
    # llvm-mingw has no libgcc: -lgcc and -lgcc_s resolve to empty archives and the unwinder stands in for libgcc_eh.
    stubs="$RUNNER_TEMP/mingw-stubs"
    mkdir -p "$stubs"
    llvm-ar rc "$stubs/libgcc.a"
    llvm-ar rc "$stubs/libgcc_s.a"
    cp "$tools/llvm-mingw-20261117/x86_64-w64-mingw32/lib/libunwind.a" "$stubs/libgcc_eh.a"
    env_set CROSS_MINGW_STUBS "$stubs"
    env_set CC_x86_64_pc_windows_gnu x86_64-w64-mingw32-clang
    env_set AR_x86_64_pc_windows_gnu llvm-ar
    env_set CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER "$wrappers/mingw-gcc"
    ;;
  *)
    echo "::error::no cross tools for $target"
    exit 1
    ;;
esac
