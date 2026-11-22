#!/usr/bin/env bash
# Checks out the sunrise-launcher commit pinned in tools/ci/launcher-pin into DEST: fetches that exact SHA, checks it
# out detached, and refuses a HEAD that is not the pin. LAUNCHER_URL overrides the repository (tests only).
set -euo pipefail
dest=${1:?usage: launcher_checkout.sh DEST}
pin=$("$(dirname "$0")/launcher_pin.sh")
rm -rf "$dest"
git init --quiet "$dest"
git -C "$dest" fetch --quiet --depth 1 "${LAUNCHER_URL:-https://github.com/sunrise-org/sunrise-launcher}" "$pin"
git -C "$dest" checkout --quiet --detach FETCH_HEAD
head=$(git -C "$dest" rev-parse HEAD)
if [ "$head" != "$pin" ]; then
  echo "::error::sunrise-launcher HEAD is $head, not the pinned $pin" >&2
  exit 1
fi
