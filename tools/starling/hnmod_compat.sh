#!/usr/bin/env bash
# Whether the website's native mods (.hnmod, written against Horizon's main API) still build on this Starling:
# each mod alone through what a phone's build does (packaging/android/native_build.sh: the source's preparation,
# horizon-materialize, native_mod_adapt.py, `horizon-mixins apply`), then `cargo ndk check` of horizon-android for
# arm64-v8a. Run by .github/workflows/hnmod-compat.yml on GitHub's runners, which hold no secrets: mods with crates of
# their own ([[crate]], build scripts and macros) are checked too. It was the build VM's ~/compat (sync 6).
#
#   hnmod_compat.sh SOURCE MODS OUT [MOD...]
#
# SOURCE is a Starling checkout (git archive of its HEAD). MODS is hnmod_fetch.py's folder. OUT gets check.tsv
# (mod, ok | fail | apply-fail | skipped, seconds, error count, the first errors) and logs/. No MOD: every mod of
# MODS/_list.tsv that is ok. The control (no mod) runs first; when it fails, no mod is checked.
set -uo pipefail

fail() { echo "hnmod_compat: $*" >&2; exit 1; }
[ $# -ge 3 ] || fail "usage: hnmod_compat.sh SOURCE MODS OUT [MOD...]"
source=$(cd "$1" && pwd)
mods=$(cd "$2" && pwd)
mkdir -p "$3/logs"
out=$(cd "$3" && pwd)
shift 3

# One horizon-mixins for both: native_build.sh's pin.
MIXINS_GIT=$(sed -n 's/^MIXINS_GIT=//p' "$source/packaging/android/native_build.sh")
MIXINS_REV=$(sed -n 's/^MIXINS_REV=//p' "$source/packaging/android/native_build.sh")
[ -n "$MIXINS_REV" ] || fail "no MIXINS_REV in native_build.sh"
cache=${NATIVE_CACHE:-$HOME/.cache/starling-native}
tools=$cache/mixins-${MIXINS_REV:0:12}
if [ ! -x "$tools/bin/horizon-mixins" ]; then
  cargo install --locked --git "$MIXINS_GIT" --rev "$MIXINS_REV" --root "$tools" horizon-mixins-cli || exit 1
fi

commit=$(git -C "$source" rev-parse HEAD)
# horizon-mod-host's build script names the game build by `git rev-parse HEAD` unless this says it:
# the build folder below is a copy without the repository (native_build.sh sets it the same way).
export HORIZON_GAME_BUILD_ID=$commit
work=${WORK:-$(mktemp -d)}
mkdir -p "$work"
build=$work/build
manifest=crates/horizon-android/Cargo.toml
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_TERM_COLOR=never
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$work/target}

# The source as native_build.sh prepares it (read the reasons there): no links, horizon-android's library an empty
# file while horizon-mixins plans, lib.rs without its conditional attribute, the game's modules written out, the
# modules named through a parent folder moved beside their module.
rm -rf "$work/source"
mkdir -p "$work/source"
git -C "$source" archive --format=tar HEAD | tar -x -C "$work/source"
find "$work/source" -type l -delete
echo "$commit" > "$work/source/.horizon-revision"
lib_line='path = "../horizon-client/src/android_lib.rs"'
grep -qxF "$lib_line" "$work/source/$manifest" || fail "$manifest no longer has: $lib_line"
sed -i "s|^path = \"../horizon-client/src/android_lib.rs\"\$|path = \"plan-stub.rs\"|" "$work/source/$manifest"
: > "$work/source/crates/horizon-android/plan-stub.rs"
lib_attr='#![cfg_attr(not(target_os = "android"), forbid(unsafe_code))]'
grep -qxF "$lib_attr" "$work/source/crates/horizon-client/src/lib.rs" \
  || fail "crates/horizon-client/src/lib.rs no longer has: $lib_attr"
grep -vxF "$lib_attr" "$work/source/crates/horizon-client/src/lib.rs" > "$work/lib.rs"
cp "$work/lib.rs" "$work/source/crates/horizon-client/src/lib.rs"
cargo fetch --locked --manifest-path "$work/source/Cargo.toml" > "$out/logs/_fetch.log" 2>&1 \
  || { tail -20 "$out/logs/_fetch.log"; fail "cargo fetch failed"; }
CARGO_TARGET_DIR=$work/materialize-target cargo run --quiet --locked --offline \
  --manifest-path "$work/source/Cargo.toml" -p horizon-mod-api-gen --bin horizon-materialize \
  -- "$work/source" > "$out/logs/_materialize.log" 2>&1 \
  || { tail -20 "$out/logs/_materialize.log"; fail "horizon-materialize failed"; }
python3 "$work/source/packaging/android/native_mixins_source.py" "$work/source" > "$out/logs/_mixins_source.log" 2>&1 \
  || { cat "$out/logs/_mixins_source.log"; fail "native_mixins_source.py failed"; }

errors() { grep -E '^[^ ]*: *error|^error' "$1" | grep -vE 'could not compile|aborting due to' | cut -c1-200 | head -n 3 | paste -sd'|'; }
why() {  # horizon-mixins' first diagnostics, else the log's end
  python3 -I - "$1" <<'EOF'
import re, sys
text = open(sys.argv[1], errors="replace").read()
found = re.findall(r'"code": "(E_[A-Z_]+)",\s*"message": "([^"]*)"', text)
seen = []
for pair in found:
    if pair not in seen:
        seen.append(pair)
print(" | ".join(f"{a}: {b[:150]}" for a, b in seen[:3]) if seen else " ".join(text.split())[-240:])
EOF
}

# Into the build folder: a file whose bytes did not change keeps its time, so cargo rechecks only what a mod changed.
original=$work/manifest
git -C "$source" show "HEAD:$manifest" > "$original"
check_tree() {  # TREE LOG FLAGS
  mkdir -p "$build"
  rsync -r --checksum --delete --exclude /target/ --exclude "/$manifest" \
    --exclude /crates/horizon-android/plan-stub.rs --exclude /native-hooks.json "$1/" "$build/"
  cmp -s "$original" "$build/$manifest" || cp "$original" "$build/$manifest"
  # shellcheck disable=SC2086
  (cd "$build" && timeout 2400 cargo ndk -t arm64-v8a check -p horizon-android $3 --message-format=short) > "$2" 2>&1
}
record() {  # MOD LOG SECONDS CODE
  if [ "$4" = 0 ]; then
    printf '%s\tok\t%s s\n' "$1" "$3" >> "$out/check.tsv"
  else
    printf '%s\tfail\t%s s\t%s\t%s\n' "$1" "$3" "$(grep -cE '^[^ ]*: *error|^error\[' "$2")" "$(errors "$2")" \
      >> "$out/check.tsv"
  fi
}

: > "$out/check.tsv"
start=$SECONDS
check_tree "$work/source" "$out/logs/_control.check.log" "--locked --offline"
code=$?
record _control "$out/logs/_control.check.log" $((SECONDS - start)) $code
if [ $code != 0 ]; then
  echo "hnmod_compat: the control (no mod) does not build:"
  errors "$out/logs/_control.check.log"
  exit 1
fi

[ $# -gt 0 ] || set -- $(awk -F'\t' '$3 == "ok" {print $1}' "$mods/_list.tsv")
for mod in "$@"; do
  [ -d "$mods/$mod" ] || { printf '%s\tskipped\tnot fetched\n' "$mod" >> "$out/check.tsv"; continue; }
  rm -rf "$work/mods" "$work/stage"
  mkdir "$work/mods"
  cp -r "$mods/$mod/." "$work/mods/"
  find "$work/mods" -maxdepth 2 -name horizon-mixin.toml -exec \
    sed -i -E "s|^horizon_revision = \"[0-9a-f]*\"|horizon_revision = \"$commit\"|" {} +
  python3 "$work/source/packaging/android/native_mod_adapt.py" "$work/source" "$work/mods" \
    > "$out/logs/$mod.adapt.log" 2>&1
  if ! "$tools/bin/horizon-mixins" apply --workspace "$work/source" --mods "$work/mods" --out "$work/stage" \
      > "$out/logs/$mod.apply.log" 2>&1; then
    printf '%s\tapply-fail\t\t\t%s\n' "$mod" "$(why "$out/logs/$mod.apply.log")" >> "$out/check.tsv"
    continue
  fi
  # A mod with crates of its own changes the workspace and its lockfile, and may fetch.
  flags="--locked --offline"
  ! grep -qs '^\[\[crate\]\]' "$work/mods"/*/horizon-mixin.toml || flags=""
  start=$SECONDS
  check_tree "$work/stage" "$out/logs/$mod.check.log" "$flags"
  code=$?
  record "$mod" "$out/logs/$mod.check.log" $((SECONDS - start)) $code
  # The errors in full (which call, which argument): cargo replays them from its cache, in seconds.
  if [ $code != 0 ]; then
    # shellcheck disable=SC2086
    (cd "$build" && timeout 600 cargo ndk -t arm64-v8a check -p horizon-android $flags) > "$out/logs/$mod.full.log" 2>&1
  fi
done
echo "hnmod_compat: $(grep -c $'\tok\t' "$out/check.tsv") ok (the control included)," \
  "$(grep -c $'\tfail\t' "$out/check.tsv") fail, $(grep -c $'\tapply-fail\t' "$out/check.tsv") apply-fail"
