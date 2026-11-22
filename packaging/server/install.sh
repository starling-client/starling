#!/bin/sh
# Installs sunrise-server (docs/server.md) on Linux or macOS: by default the current Sunrise Nightly for this computer
# (GitHub's latest release while there is none), or the release --version names, downloaded from GitHub Releases and
# checked against the release's signed SHA256SUMS (docs/release-flow.md, "Signed checksums") by sunrise-verify, a
# small program this script downloads and checks against the SHA-256 pinned below before it runs it. A download that
# can't be verified is not installed. On Linux that is the server build (sunrise-server-linux-x86_64.tar.gz), which needs neither ALSA nor udev.
#
#   curl -fsSL https://raw.githubusercontent.com/sunrise-org/sunrise/main/packaging/server/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --version 26.3-0.1.0
#   curl -fsSL .../install.sh | sh -s -- --version 26.3-0.1.0 --dir /opt/sunrise --bin /usr/local/bin
#   sh install.sh --archive sunrise-server-linux-x86_64.tar.gz --version mybuild
#
# A stable release's tag (26.3-0.1.0) puts the server on that stable line, which `sunrise-server update` then
# follows. --archive installs an archive you have, such as your own build.
#
# It needs curl, tar or unzip, and sha256sum or shasum; it never asks for sudo (pick folders you can write).
# Releases go to <dir>/releases/<version>, <dir>/current points at the one in use, and <bin>/sunrise-server
# runs it. The game files you prepare from your own Minecraft stay in <dir>/game-files across updates.
# Nothing of Mojang's is downloaded.
set -eu

REPO=sunrise-org/sunrise
VERSION=latest
DIR="${XDG_DATA_HOME:-$HOME/.local/share}/sunrise"
BIN="$HOME/.local/bin"
ARCHIVE=
INSECURE=
# sunrise-verify, the release checker the install pins: its SHA-256 per platform,
# from release verifier@VERIFIER_VERSION. "unpinned" until the first verifier release exists.
# >>> verifier pin (tools/ci/pin_verifier.py rewrites this block; do not edit it by hand)
VERIFIER_VERSION=1.0.0
VERIFIER_SHA256_linux_x86_64=310bba30dd6dba7147d9a14692c4efb5887875c6d6eca953a5e888106f5a7630
VERIFIER_SHA256_linux_aarch64=5e293dbc2552db7a7df79105005425463b7b0eef2d97ff30a8e65d156f0c50b6
VERIFIER_SHA256_macos_arm64=0b934c07fe4003bc25554a95adef2dfc3a3b73e84b9647e100d0a3acd472e11e
VERIFIER_SHA256_macos_x64=912223491295e60f2d338a141b47fe2deab161608b1b8b02134ca7b7bdf0430b
VERIFIER_SHA256_windows_x64=d26077550c2b8695db181d3f32ba1cb7f2f22cff455f78d6148a298df56f9b79
VERIFIER_SHA256_windows_arm64=ef5bbdf1440c303c0e8c6056159fc46166727257f1e6a1b98ef63def7932d1d9
# <<< verifier pin

die() { echo "sunrise install: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version) [ $# -ge 2 ] || die "--version needs a tag"; VERSION=$2; shift 2 ;;
    --dir) [ $# -ge 2 ] || die "--dir needs a folder"; DIR=$2; shift 2 ;;
    --bin) [ $# -ge 2 ] || die "--bin needs a folder"; BIN=$2; shift 2 ;;
    --archive) [ $# -ge 2 ] || die "--archive needs a file"; ARCHIVE=$2; shift 2 ;;
    # For CI fixtures only (never in the docs): GitHub's own SHA-256 is all that is checked.
    --insecure-checksum-only) INSECURE=yes; shift ;;
    -h|--help) sed -n '2,19p' "$0" 2>/dev/null || true; exit 0 ;;
    *) die "unknown option $1" ;;
  esac
done

# The archives to look for, in order: on Linux the server build, else (releases before it) the full game.
MACHINE=$(uname -m)
# A shell under Rosetta 2 says x86_64 on Apple Silicon: the native build runs faster there.
[ "$(uname -s)" = Darwin ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = 1 ] && MACHINE=arm64
case "$(uname -s)-$MACHINE" in
  Linux-x86_64) PLATFORM=linux-x86_64; ASSETS="sunrise-server-linux-x86_64.tar.gz sunrise-linux-x86_64.tar.gz" ;;
  Darwin-arm64) PLATFORM=macos-arm64; ASSETS=sunrise-macos-arm64.zip ;;
  # Intel Macs: the Nightlies built since #1161.
  Darwin-x86_64) PLATFORM=macos-x64; ASSETS=sunrise-macos-x64.zip ;;
  *) die "no Sunrise release for $(uname -s) $(uname -m) yet: build it from source (docs/server.md)" ;;
esac

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
if [ -n "$ARCHIVE" ]; then
  # An archive already here (a build of your own, or one fetched by hand): checking it is up to you.
  [ -f "$ARCHIVE" ] || die "no archive $ARCHIVE"
  if [ "$VERSION" = latest ]; then TAG=local; else TAG=$VERSION; fi
  ASSET=$(basename "$ARCHIVE")
  cp "$ARCHIVE" "$TMP/$ASSET"
else
  command -v curl >/dev/null || die "curl is needed"
  if command -v sha256sum >/dev/null; then SHA="sha256sum"
  elif command -v shasum >/dev/null; then SHA="shasum -a 256"
  else die "sha256sum or shasum is needed"; fi

  api() { curl -fsSL -H "Accept: application/vnd.github+json" "https://api.github.com/repos/$REPO/releases/$1"; }
  if [ "$VERSION" = latest ]; then
    JSON=$(api tags/nightly) || JSON=$(api latest) || die "no release on GitHub"
  else
    JSON=$(api "tags/$VERSION") || die "no release $VERSION on GitHub"
  fi

  TAG=$(printf '%s\n' "$JSON" | sed -n 's/^  "tag_name": "\([^"]*\)".*/\1/p' | head -n 1)
  [ -n "$TAG" ] || die "could not read the release's tag"
  RAWTAG=$TAG
  case "$TAG" in
    dev|dev-*) die "$TAG is a Dev build, made to test main: servers don't install it. Install the Nightly (the default) or a stable release (--version <tag>)." ;;
  esac
  # A Nightly installs under its date and commit, as `sunrise-server update` names it:
  # "Nightly 2026-10-28 · 0c02067" is nightly-2026-10-28-0c02067. Its signature is checked for exactly that
  # version, so the title must name it (as sunrise_release_sign::build_title_version reads it, strictly).
  NAME=
  case "$TAG" in
    nightly|nightly-*)
      NAME=$(printf '%s\n' "$JSON" | sed -n 's/^  "name": "Nightly \([0-9]\{4\}-[0-9]\{2\}-[0-9]\{2\}\) · \([0-9a-f]\{7,40\}\)"[,}]*$/nightly-\1-\2/p' | head -n 1)
      [ -n "$NAME" ] || die "this Nightly's title doesn't name its date and commit as Sunrise signs them, so it can't be checked: not installed"
      TAG=$NAME ;;
  esac
  # A release asset's download address, from its block in the release's JSON.
  asset_url() {
    printf '%s\n' "$JSON" | awk -v asset="$1" '
      /"name": / { line = $0; sub(/.*"name": "/, "", line); sub(/".*/, "", line); current = line }
      current == asset && /"browser_download_url": / { line = $0; sub(/.*"browser_download_url": "/, "", line); sub(/".*/, "", line); print line; exit }'
  }
  # The first asset the release has: its download address and digest, from its block in the release's JSON.
  for ASSET in $ASSETS; do
    FIELDS=$(printf '%s\n' "$JSON" | awk -v asset="$ASSET" '
      /"name": / { line = $0; sub(/.*"name": "/, "", line); sub(/".*/, "", line); current = line }
      current == asset && /"digest": / { line = $0; sub(/.*"digest": "sha256:/, "", line); sub(/".*/, "", line); digest = line }
      current == asset && /"browser_download_url": / { line = $0; sub(/.*"browser_download_url": "/, "", line); sub(/".*/, "", line); url = line }
      END { print url; print digest }')
    URL=$(printf '%s\n' "$FIELDS" | sed -n 1p)
    DIGEST=$(printf '%s\n' "$FIELDS" | sed -n 2p)
    [ -z "$URL" ] || break
  done
  [ -n "$URL" ] || die "release $TAG has none of $ASSETS"

  # A Nightly is checked against SHA256SUMS.v2.sig, signed for its own version (nightly-<date>-<commit>, from its
  # title), never the channel's SHA256SUMS.sig: an older Nightly can't be passed off as the current one
  # (docs/security.md, "Nightly signatures").
  if [ -n "$NAME" ]; then SIGTAG=$NAME; SIGFILE=SHA256SUMS.v2.sig; else SIGTAG=$RAWTAG; SIGFILE=SHA256SUMS.sig; fi
  mkdir "$TMP/files"
  if [ -n "$INSECURE" ]; then
    echo "WARNING: --insecure-checksum-only: the release signature is NOT checked, only the SHA-256 GitHub publishes for the" >&2
    echo "WARNING: download. Anyone who can change the release can change that too. This is for test fixtures, not for servers." >&2
    [ -n "$DIGEST" ] || die "release $TAG publishes no SHA-256 for $ASSET"
    echo "Downloading Sunrise $TAG ($ASSET)..."
    curl -fL --progress-bar -o "$TMP/files/$ASSET" "$URL"
    GOT=$($SHA "$TMP/files/$ASSET" | cut -d' ' -f1)
    [ "$GOT" = "$DIGEST" ] || die "$ASSET doesn't match its SHA-256 (got $GOT, expected $DIGEST): not installed"
  else
    # The verifier: the pinned version's binary for this platform, run only when its SHA-256 is the pinned one. The
    # pin is the trust anchor (tools/ci/pin_verifier.py); nothing outside this file can change it.
    case "$PLATFORM" in
      linux-x86_64) PIN=$VERIFIER_SHA256_linux_x86_64 ;;
      macos-arm64) PIN=$VERIFIER_SHA256_macos_arm64 ;;
      macos-x64) PIN=$VERIFIER_SHA256_macos_x64 ;;
    esac
    VERIFIER_FROM=$VERIFIER_VERSION
    [ "$VERIFIER_FROM" != unpinned ] && [ -n "$VERIFIER_FROM" ] || die "this script has no pinned sunrise-verify yet, so it can't check the release signature: not installed. Use the install.sh from Sunrise's main branch."
    [ -n "$PIN" ] && [ "$PIN" != unpinned ] || die "there is no sunrise-verify for $PLATFORM (the signature check needs it): not installed"
    VERIFIER="$TMP/sunrise-verify"
    curl -fsSL -o "$VERIFIER" "https://github.com/$REPO/releases/download/verifier%40$VERIFIER_FROM/sunrise-verify-$PLATFORM" \
      || die "could not download sunrise-verify $VERIFIER_FROM for $PLATFORM (needed to check the release signature): not installed"
    GOT=$($SHA "$VERIFIER" | cut -d' ' -f1)
    [ "$GOT" = "$PIN" ] || die "sunrise-verify for $PLATFORM isn't the pinned build (got $GOT, expected $PIN): not installed. The download may have been tampered with; please report it on Discord or GitHub."
    chmod +x "$VERIFIER"

    SUMS_URL=$(asset_url SHA256SUMS)
    SIG_URL=$(asset_url "$SIGFILE")
    if [ -z "$SUMS_URL" ]; then
      die "release $TAG has no SHA256SUMS (its checksum list), so it can't be checked: not installed"
    elif [ -z "$SIG_URL" ] && [ -n "$NAME" ]; then
      die "$TAG has no $SIGFILE (its signature for this exact build): it was published before Nightlies were signed for their own version, so it can't be checked: not installed. Install the current Nightly (--version latest) instead."
    elif [ -z "$SIG_URL" ]; then
      die "release $TAG has no $SIGFILE (the signature of its checksum list), so it can't be checked: not installed"
    fi
    curl -fsSL -o "$TMP/files/SHA256SUMS" "$SUMS_URL" || die "could not download SHA256SUMS: not installed"
    curl -fsSL -o "$TMP/files/$SIGFILE" "$SIG_URL" || die "could not download $SIGFILE: not installed"
    echo "Downloading Sunrise $TAG ($ASSET)..."
    curl -fL --progress-bar -o "$TMP/files/$ASSET" "$URL" || die "could not download $ASSET: not installed"
    # The same check the launcher and `sunrise-server update` make, with the keys built into the verifier.
    "$VERIFIER" verify --tag "$SIGTAG" --signature "$SIGFILE" --only "$ASSET" "$TMP/files" > "$TMP/verify.out" 2>&1 \
      || die "the release check failed ($SIGFILE for $SIGTAG), so $TAG was not installed: $(cat "$TMP/verify.out"). The download may have been tampered with; please report it on Discord or GitHub."
    echo "Release signature OK (Sunrise release key)."
  fi
  mv "$TMP/files/$ASSET" "$TMP/$ASSET"
fi

mkdir -p "$TMP/x"
case "$ASSET" in
  *.tar.gz) tar -xzf "$TMP/$ASSET" -C "$TMP/x" ;;
  *.zip) command -v unzip >/dev/null || die "unzip is needed"; unzip -q "$TMP/$ASSET" -d "$TMP/x" ;;
esac
SOURCE=$(dirname "$(find "$TMP/x" -name sunrise-server -type f | head -n 1)")
[ -x "$SOURCE/sunrise-server" ] || die "$ASSET holds no sunrise-server (a release from before it existed?)"

RELEASE="$DIR/releases/$TAG"
mkdir -p "$DIR/releases" "$DIR/game-files/sunrise-client" "$BIN"
rm -rf "$RELEASE"
mv "$SOURCE" "$RELEASE"
# Prepared game files live outside the release, so updates and rollbacks keep them.
mkdir -p "$RELEASE/.local"
ln -sfn "$DIR/game-files/sunrise-client" "$RELEASE/.local/sunrise-client"
ln -sfn "$RELEASE" "$DIR/current"
# Small launchers rather than links: the programs find their release folder from where they run.
for tool in sunrise-server sunrise-prepare; do
  printf '#!/bin/sh\nexec "%s/current/%s" "$@"\n' "$DIR" "$tool" > "$BIN/$tool"
  chmod +x "$BIN/$tool"
done

echo "Installed Sunrise $TAG in $RELEASE; sunrise-server and sunrise-prepare are in $BIN."
# The full game (releases before the server build) links ALSA and udev; minimal Linux images often lack them.
if [ "$(uname -s)" = Linux ] && [ ! -e "$RELEASE/sunrise-engine" ] && command -v ldd >/dev/null; then
  MISSING=$(ldd "$RELEASE/sunrise-client" 2>/dev/null | awk '/not found/ { printf "%s ", $1 }')
  if [ -n "$MISSING" ]; then
    echo "The game needs libraries this system doesn't have: $MISSING"
    echo "Install them before starting a server:"
    echo "  Debian, Ubuntu: sudo apt install libasound2 libudev1   (Ubuntu 24.04 and later: libasound2t64)"
    echo "  Fedora, RHEL: sudo dnf install alsa-lib systemd-libs"
    echo "  Arch: sudo pacman -S alsa-lib systemd-libs"
    echo "  NixOS: enable programs.nix-ld with alsa-lib and udev in its libraries"
  fi
fi
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "Add $BIN to your PATH, for example: echo 'export PATH=\"$BIN:\$PATH\"' >> ~/.profile" ;;
esac
if [ -z "$(ls -A "$DIR/game-files/sunrise-client" 2>/dev/null)" ]; then
  echo "Next, prepare the game files from your own Minecraft 26.3 (it needs Java 25 and the game installed once"
  echo "by the official launcher; see docs/server.md):"
  echo "  sunrise-prepare --prepare --server-jar /path/to/server.jar"
fi
echo "Then set up a server: mkdir my-server && cd my-server && sunrise-server init"
