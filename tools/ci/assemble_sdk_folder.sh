#!/usr/bin/env bash
# Lays the daily run's own SDK out as a release folder a mod tool reads through SUNRISE_SDK_BASE, as sdk-publish.yml's
# index job would publish it: one build data and surface across the legs, the API source, sdk.json, all signed with a
# throwaway key made for this run. No release key is ever used. Run from the checkout of the commit.
#
#   tools/ci/assemble_sdk_folder.sh <legs> <base> <commit>
#
# <legs> holds the `sdk-leg-*` folders (the sdk job's artifacts). The release lands in <base>/sdk-<commit>. The key's
# public half goes to SUNRISE_SDK_TRUSTED_KEY, in $GITHUB_ENV for the steps after this one (stdout without it): only a
# tool built with debug_assertions reads that variable (sunrise_release_sign::sdk_keys_for).
set -euo pipefail
legs=${1:?the folder of sdk-leg-* artifacts}
base=${2:?the folder that receives sdk-<commit>}
commit=${3:?the commit}
# Beside <base>, not in /tmp: on a Dyna runner /tmp is a small tmpfs, and the caller's scratch is on the build volume.
mkdir -p "$base"
work=$(mktemp -d "${base%/}.work.XXXXXX")
trap 'rm -rf "$work"' EXIT
dist=$work/dist
python3 tools/prebuilt_game.py gather "$legs" "$dist"
python3 tools/sdk_api_source.py build "$work/api-source"
python3 tools/sdk_api_source.py archive "$work/api-source" "$dist/api-source.tar.xz"
python3 tools/prebuilt_game.py index "$dist"
# A throwaway key made for this run: no release key is ever set here. Its public half goes to the tool.
EPHEMERAL=yes bash tools/ci/sign_release.sh "sdk-$commit" "$dist"/*
key=$(cat "$dist/ephemeral.pub")
rm "$dist/ephemeral.pub"
mv "$dist" "$base/sdk-$commit"
echo "SUNRISE_SDK_TRUSTED_KEY=$key" >> "${GITHUB_ENV:-/dev/stdout}"
