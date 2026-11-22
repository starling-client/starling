#!/usr/bin/env bash
# Build a client SDK for this computer and lay it out as a local release folder.
# Run from the root of a Sunrise checkout: tools/developer_sdk.sh SDK_BASE
set -euo pipefail
if [[ $# != 1 || -z ${1%/} ]]; then
  echo 'usage: tools/developer_sdk.sh SDK_BASE (a new folder)' >&2
  exit 2
fi
base=${1%/}
if [[ -e "$base" || -L "$base" ]]; then
  echo "$base already exists; choose a new SDK base folder" >&2
  exit 1
fi
commit=$(git rev-parse HEAD)
parent=$(dirname -- "$base")
target=${CARGO_TARGET_DIR:-target}
mkdir -p -- "$parent" "$target"
CARGO_TARGET_DIR=$(cd -- "$target" && pwd)
export CARGO_TARGET_DIR
work=$(mktemp -d "$(cd -- "$parent" && pwd)/.developer-sdk.XXXXXX")
snapshot="$work/checkout"
cleanup() {
  if [[ -f "$snapshot/.git" ]]; then
    git worktree remove --force "$snapshot"
  fi
  rm -rf -- "$work"
}
trap cleanup EXIT
export TMPDIR="$work/tmp"
mkdir "$TMPDIR"

# The surface and API-source builders read the working tree. Give every stage the same
# commit, leaving edits and untracked (including ignored) declarations behind.
git worktree add --quiet --detach "$snapshot" "$commit"
(
  cd "$snapshot"
  rust=$(<tools/ci/rust-version)
  if [[ $(rustc --version) != "rustc $rust "* ]]; then
    echo "Build the SDK with Rust $rust (see tools/ci/rust-version)." >&2
    exit 1
  fi
  python3 tools/prebuilt_game.py build "$work/prebuilt"
  python3 tools/prebuilt_game.py pack "$work/prebuilt" "$work/legs/sdk-leg-client" \
    --stage-tool "$CARGO_TARGET_DIR/prebuilt/tools/release/sunrise-stage"
  GITHUB_ENV="$work/key-env" tools/ci/assemble_sdk_folder.sh "$work/legs" "$work/sdk" "$commit"
)
IFS= read -r setting < "$work/key-env"
printf '%s\n' "${setting#SUNRISE_SDK_TRUSTED_KEY=}" > "$work/sdk/developer-key.pub"

# Commands that default to `nightly` resolve this pointer, then verify the signed sdk-<commit> release.
mkdir "$work/sdk/nightly"
cp "$work/sdk/sdk-$commit/sdk.json" "$work/sdk/nightly/sdk.json"
mkdir -- "$base"
mv -- "$work/sdk/"* "$base/"
printf 'Local SDK %s in %s (public key: developer-key.pub)\n' "$commit" "$base"
