#!/usr/bin/env bash
# Run on the glibc Rust host, with Alpine's loader configured to run its C tools.
set -euo pipefail

sysroot=$(realpath "$1")
target=x86_64-unknown-linux-musl
build_dir=${CARGO_TARGET_DIR:-target}
bin="$build_dir/$target/release"

# Only target C code uses Alpine's headers, libraries and compiler. Proc macros
# and build scripts (including libudev-sys's bare rustc probe) run on glibc.
cat > "$sysroot/sunrise-cc" <<EOF
#!/usr/bin/env bash
exec "$sysroot/usr/bin/gcc" --sysroot="$sysroot" -B"$sysroot/usr/bin/" "\$@"
EOF
chmod +x "$sysroot/sunrise-cc"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$sysroot/sunrise-cc"
export CC_x86_64_unknown_linux_musl="$sysroot/sunrise-cc"
export AR_x86_64_unknown_linux_musl="$sysroot/usr/bin/ar"
export PKG_CONFIG_SYSROOT_DIR_x86_64_unknown_linux_musl="$sysroot"
export PKG_CONFIG_LIBDIR_x86_64_unknown_linux_musl="$sysroot/usr/lib/pkgconfig:$sysroot/usr/share/pkgconfig"
export PKG_CONFIG_PATH_x86_64_unknown_linux_musl=

# Keep the desktop build able to dlopen graphics and window libraries. A global
# RUSTFLAGS, even an empty one from the matrix, would override the target flags.
unset RUSTFLAGS
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS='-C target-feature=-crt-static'
cargo build --release --locked --timings --target "$target" \
  -p sunrise-official --bin sunrise-official -p sunrise-client-lib --bin sunrise-ctl \
  -p sunrise-perf --bin sunrise-perf \
  -p sunrise-prepare --bin sunrise-prepare -p sunrise-server --bin sunrise-server \
  -p sunrise-port-pack --bin sunrise-port-pack

# libudev-sys 0.1.4 decides hwdb by linking on the host. Alpine's eudev exports
# udev_hwdb_new, so losing the cfg (e.g. if host libudev-dev is absent) is a build error.
for output in "$bin"/build/libudev-sys-*/output; do
  grep -Fxq 'cargo:rustc-cfg=hwdb' "$output" || {
    echo "::error::expected libudev-sys to enable hwdb in $output; check the host's libudev-dev" >&2
    exit 1
  }
done

# The official game (sunrise-client's with Sunrise's own parts, docs/layers.md) is what players
# get, under the game's name.
cp -f "$bin/sunrise-official" "$bin/sunrise-client"
