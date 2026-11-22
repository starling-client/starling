#!/usr/bin/env bash
# Checks a launcher release folder for launcher-bridge.yml with only sha256sum, base64 and OpenSSL: its files are exactly SHA256SUMS's and match it, and, given a tag, keys and a signature file, a key signed SHA256SUMS for that tag.
set -euo pipefail
usage="usage: verify_launcher_release.sh <folder> [<vX.Y.Z or launcher@X.Y.Z> <keys file> <signature file>]"
dir=${1:?$usage}
tag=${2:-} keys=${3:-} signatures=${4:-}
[[ -z "$tag" || ( -n "$keys" && -n "$signatures" ) ]] || { echo "$usage" >&2; exit 1; }
openssl=${OPENSSL:-openssl}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
number='(0|[1-9][0-9]{0,8})'

[[ -f "$dir/SHA256SUMS" ]] || { echo "$dir has no SHA256SUMS" >&2; exit 1; }
listed=$(awk '{print $2}' "$dir/SHA256SUMS" | sed 's/^\*//' | sort)
present=$(cd "$dir" && find . -mindepth 1 -maxdepth 1 -printf '%f\n' | grep -vx -e SHA256SUMS -e SHA256SUMS.sig -e SHA256SUMS.launcher.sig | sort)
[[ "$listed" == "$present" ]] || { echo "$dir holds other files than SHA256SUMS lists: listed [$listed], present [$present]" >&2; exit 1; }
while read -r name; do
  [[ "$name" =~ ^sunrise-launcher-[A-Za-z0-9._-]+$ ]] || { echo "SHA256SUMS lists $name, not a launcher file" >&2; exit 1; }
done <<< "$listed"
(cd "$dir" && sha256sum --check --strict --quiet SHA256SUMS) || { echo "$dir: a file doesn't match SHA256SUMS" >&2; exit 1; }
[[ -n "$tag" ]] || { echo "$dir: every file is SHA256SUMS's and matches it"; exit 0; }

# A vX.Y.Z tag is the launcher repository's own release; launcher@X.Y.Z is this repository's, what launchers from before the split check.
if [[ "$tag" =~ ^v$number\.$number\.$number$ ]]; then prefix="sunrise-launcher release"
elif [[ "$tag" =~ ^launcher@$number\.$number\.$number$ ]]; then prefix="horizon release"
else echo "$tag is neither vX.Y.Z nor launcher@X.Y.Z" >&2; exit 1; fi
mapfile -t trusted < <(grep -v '^[[:space:]]*\(#\|$\)' "$keys" || true)
[[ ${#trusted[@]} -gt 0 ]] || { echo "$keys lists no key: nothing can be checked" >&2; exit 1; }
{ printf '%s %s\n' "$prefix" "$tag"; cat "$dir/SHA256SUMS"; } > "$work/message"
signed=
while read -r signature; do
  [[ -n "$signature" ]] || continue
  base64 --decode <<< "$signature" > "$work/signature" 2> /dev/null || continue
  for key in "${trusted[@]}"; do
    # An Ed25519 public key's DER: the fixed SubjectPublicKeyInfo header, then its 32 bytes.
    { printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00'; base64 --decode <<< "$key"; } > "$work/key.der"
    if "$openssl" pkeyutl -verify -pubin -inkey "$work/key.der" -keyform DER -rawin -in "$work/message" -sigfile "$work/signature" > /dev/null 2>&1; then
      signed=yes
    fi
  done
done < "$signatures"
[[ -n "$signed" ]] || { echo "$tag: SHA256SUMS isn't signed for \"$prefix $tag\" by a key in $keys" >&2; exit 1; }
echo "$tag: every file is SHA256SUMS's, and SHA256SUMS is signed for \"$prefix $tag\""
