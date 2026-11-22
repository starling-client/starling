#!/usr/bin/env bash
# Signs a release (docs/release-flow.md, "Signed checksums"): writes SHA256SUMS for the files given, then
# SHA256SUMS.sig beside them, one base64 Ed25519 signature per line over "horizon release <tag>\n" + SHA256SUMS,
# for every key set in RELEASE_SIGNING_KEY and RELEASE_SIGNING_KEY_NEXT (base64 PKCS#8, the `release`
# environment's secrets). The job that runs this holds the private keys, so it runs nothing from the repository
# but this file, which calls only the runner's sha256sum and OpenSSL; the publish job then checks the result
# with sunrise-release-sign, as launchers do.
# EPHEMERAL=yes (dry runs) signs with a throwaway key instead and writes its public key to ephemeral.pub.
# ALSO_FOR=<version> (Nightlies and Dev builds) signs the same SHA256SUMS a second time, for
# "horizon release <ALSO_FOR>\n", into SHA256SUMS.v2.sig, with the same key(s): the build's own version
# (nightly-<date>-<commit> on the nightly channel, dev-<date>-<commit> on the Dev channel), which current
# launchers require, while SHA256SUMS.sig stays signed for the channel for launchers from before (docs/security.md).
#   tools/ci/sign_release.sh v2026.10.19 dist/sunrise-linux-x86_64.tar.gz dist/sunrise-windows-x64.zip ...
#   ALSO_FOR=nightly-2026-10-28-0c02067 tools/ci/sign_release.sh nightly dist/...
#   ALSO_FOR=26.3-0.1.0 tools/ci/sign_release.sh 26.3-0.1.0 dist/...   # a stable release: both files for its tag
#   ALSO_FOR=dev-2026-11-13-0c02067 tools/ci/sign_release.sh dev dist/...
set -euo pipefail
tag=$1; shift
dir=$(dirname "$1")
openssl=${OPENSSL:-openssl}
if command -v sha256sum > /dev/null; then sha=(sha256sum); else sha=(shasum -a 256); fi
umask 077
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

names=()
for file in "$@"; do
  [[ "$(dirname "$file")" == "$dir" ]] || { echo "$file: every file must be in $dir" >&2; exit 1; }
  names+=("$(basename "$file")")
done
also=${ALSO_FOR:-}
# A build's version is <channel>-<date>-<commit> for the channel signed above it (nightly or dev), so a
# Dev build's version is never signed beside the nightly channel, nor the reverse. A stable tag's is the tag itself.
stable='^[0-9]+\.[0-9]+(\.[0-9]+)?-(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
if [[ -n "$also" && ! ( "$tag" =~ ^(nightly|dev)$ && "$also" =~ ^$tag-[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9a-f]{7,40}$ ) \
      && ! ( "$tag" =~ $stable && "$also" == "$tag" ) ]]; then
  echo "ALSO_FOR=$also: not a $tag-<date>-<commit> build of the nightly or dev channel, nor a stable tag's own" >&2; exit 1
fi
# LAUNCHER_FOR=vX.Y.Z (launcher-bridge.yml, tag launcher@X.Y.Z) also signs SHA256SUMS for "sunrise-launcher release vX.Y.Z\n" into SHA256SUMS.launcher.sig.
launcher_for=${LAUNCHER_FOR:-}
if [[ -n "$launcher_for" && ( -n "$also" || ! "$launcher_for" =~ ^v(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})$ || "$tag" != "launcher@${launcher_for#v}" ) ]]; then
  echo "LAUNCHER_FOR=$launcher_for: not the vX.Y.Z of a launcher@X.Y.Z tag ($tag), or set with ALSO_FOR" >&2; exit 1
fi
(cd "$dir" && "${sha[@]}" -- "${names[@]}") > "$dir/SHA256SUMS"
{ printf 'horizon release %s\n' "$tag"; cat "$dir/SHA256SUMS"; } > "$work/message"
if [[ -n "$also" ]]; then
  { printf 'horizon release %s\n' "$also"; cat "$dir/SHA256SUMS"; } > "$work/message.v2"
fi
if [[ -n "$launcher_for" ]]; then
  { printf 'sunrise-launcher release %s\n' "$launcher_for"; cat "$dir/SHA256SUMS"; } > "$work/message.launcher"
fi
rm -f "$dir/SHA256SUMS.sig" "$dir/SHA256SUMS.v2.sig" "$dir/SHA256SUMS.launcher.sig" "$dir/ephemeral.pub"

sign_one() {  # base64 key, message file, signature file to append to
  # The key reaches OpenSSL through an anonymous pipe: never a file, an argument or output.
  "$openssl" pkeyutl -sign -rawin -inkey /dev/fd/3 -keyform DER -in "$2" -out "$work/signature" 3< <(printf '%s' "$1" | base64 --decode)
  # base64 of the 64 bytes on one line (GNU base64 wraps at 76 characters).
  base64 < "$work/signature" | tr -d '\n' >> "$3"
  echo >> "$3"
  rm -f "$work/signature"
}

sign_with() {  # the base64 key in $1: every message
  sign_one "$1" "$work/message" "$dir/SHA256SUMS.sig"
  if [[ -n "$also" ]]; then
    sign_one "$1" "$work/message.v2" "$dir/SHA256SUMS.v2.sig"
  fi
  if [[ -n "$launcher_for" ]]; then
    sign_one "$1" "$work/message.launcher" "$dir/SHA256SUMS.launcher.sig"
  fi
}

if [[ -n "${EPHEMERAL:-}" ]]; then
  ephemeral=$("$openssl" genpkey -algorithm ed25519 -outform DER | base64 | tr -d '\n')
  printf '%s' "$ephemeral" | base64 --decode | "$openssl" pkey -inform DER -pubout -outform DER | tail -c 32 | base64 > "$dir/ephemeral.pub"
  sign_with "$ephemeral"
else
  for name in RELEASE_SIGNING_KEY RELEASE_SIGNING_KEY_NEXT; do
    if [[ -n "${!name:-}" ]]; then
      sign_with "${!name}"
    fi
  done
fi
echo "$tag: SHA256SUMS lists ${#names[@]} files; $( [[ -f "$dir/SHA256SUMS.sig" ]] && echo "signed with $(wc -l < "$dir/SHA256SUMS.sig" | tr -d ' ') key(s)" || echo "no signing key, so unsigned")$( [[ -f "$dir/SHA256SUMS.v2.sig" ]] && echo ", and for $also in SHA256SUMS.v2.sig")$( [[ -f "$dir/SHA256SUMS.launcher.sig" ]] && echo ", and for the launcher's $launcher_for in SHA256SUMS.launcher.sig")"
