#!/usr/bin/env bash
# Exercise the built artifact with private resources, never in a player's game folder.
set -euo pipefail
# Boot the integrated server at its shipped stack floor, not a runner's test override.
unset RUST_MIN_STACK
home=$(mktemp -d "$RUNNER_TEMP/sunrise-smoke.XXXXXX")
version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' research-lock.json | head -1)
client="$(cd "$BIN" && pwd)/sunrise-client"
resources="$PWD/.local/sunrise-client/$version"
tools=$(cd "$(dirname "$0")" && pwd)
display=()
if [[ "$RUNNER_OS" == Linux ]]; then
  display=(python3 "$tools/with_xvfb.py")
fi
(cd "$home" && SUNRISE_HOME="$home" SUNRISE_NO_AUDIO=1 \
  SUNRISE_LAUNCHER_HOME="$home/launcher" XDG_DATA_HOME="$home/data" XDG_CONFIG_HOME="$home/config" \
  SUNRISE_LAUNCHER_NO_SELF_UPDATE=1 SUNRISE_LAUNCHER_TRASH="$home/trash" \
  SUNRISE_LAUNCHER_CLIPBOARD="$home/clipboard" SUNRISE_LAUNCHER_DESKTOP="$home/desktop" \
  "${display[@]}" "$client" --resources "$resources" --offscreen \
  --world smoke --seed 1 --render-distance 4 --world-frames 1500 --timeout-seconds 180 \
  --smoke-report "$home/smoke.json" > "$home/client.log" 2>&1) || { tail -40 "$home/client.log"; exit 1; }
python3 - "$home/smoke.json" <<'PY'
import json
import sys
report = json.load(open(sys.argv[1]))
print(json.dumps(report))
if report.get("error") is not None or report.get("frames", 0) < 1500 or not report.get("chunks") \
        or not report.get("meshed_sections") or not report.get("player"):
    sys.exit("the smoke run did not draw a world")
PY
cp "$home/smoke.json" "$RUNNER_TEMP/smoke-test.json"
