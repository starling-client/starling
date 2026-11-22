#!/usr/bin/env bash
# Puts the GitHub CLI on the PATH of a Dyna runner, whose image has none (GitHub's runners do): a pinned release,
# checked against its published SHA-256 and kept in the slot's build folder for the next job. No-op where gh exists.
set -euo pipefail
if command -v gh > /dev/null; then exit 0; fi
version=2.104.0
case "$(uname -m)" in
  x86_64) arch=amd64 sum=98cb101ad87dbbd3f6b2f2bfcbd43197ccd07643140121d8a9cfaa91eadbc24e ;;
  aarch64) arch=arm64 sum=cb34cefc6f4e77b90731374a2b737516bb8f3033c6ca46aa949e5344f68069e3 ;;
  *) echo "::error::ensure_gh.sh: no pinned gh for $(uname -m)"; exit 1 ;;
esac
dir="${DYNA_CI_BUILD:-$RUNNER_TEMP}/gh-$version-$arch"
if [ ! -x "$dir/bin/gh" ]; then
  archive="$RUNNER_TEMP/gh.tar.gz"
  curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$archive" \
    "https://github.com/cli/cli/releases/download/v$version/gh_${version}_linux_$arch.tar.gz"
  echo "$sum  $archive" | sha256sum -c --quiet
  stage="$dir.new.$$"
  mkdir -p "$stage"
  tar -xzf "$archive" -C "$stage" --strip-components=1
  rm -rf "$dir"
  mv "$stage" "$dir"
fi
echo "$dir/bin" >> "$GITHUB_PATH"
