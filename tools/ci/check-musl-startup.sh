#!/bin/sh
# Run in a runtime-only Alpine container, against the read-only release folder.
set -eu
bin=$1

# Even --help must never touch real player data.
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
cd "$scratch"
export SUNRISE_HOME="$scratch/game" SUNRISE_LAUNCHER_HOME="$scratch/launcher"
export XDG_DATA_HOME="$scratch/data" XDG_CONFIG_HOME="$scratch/config" XDG_CACHE_HOME="$scratch/cache"
export SUNRISE_LAUNCHER_NO_SELF_UPDATE=1 SUNRISE_NO_AUDIO=1
mkdir -p "$SUNRISE_HOME" "$SUNRISE_LAUNCHER_HOME" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME"

for binary in sunrise-client sunrise-ctl sunrise-perf sunrise-prepare sunrise-server sunrise-port-pack; do
  echo "$binary libraries:"
  /lib/ld-musl-x86_64.so.1 --list "$bin/$binary"
done

"$bin/sunrise-client" --help > client-help
grep -q -- '--resources' client-help
"$bin/sunrise-ctl" --help > /dev/null
"$bin/sunrise-perf" --help > perf-help
grep -q -- 'sunrise-perf summary' perf-help
"$bin/sunrise-prepare" --help > prepare-help
grep -q -- '--minecraft-dir' prepare-help
for binary in sunrise-prepare sunrise-server sunrise-port-pack; do
  out=$("$bin/$binary" --version)
  echo "$out"
  [ "$out" = "$binary $SUNRISE_VERSION" ] || {
    echo "::error::expected $binary $SUNRISE_VERSION" >&2
    exit 1
  }
done
