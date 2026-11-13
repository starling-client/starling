#!/usr/bin/env bash
# The vanilla data for this checkout on the self-hosted runner. Horizon never downloads the game: the runner
# holds the maintainer's own copy, imported and decompiled once (HORIZON_SHARED_SOURCE, default
# ~/code/horizon-parity/shared-local; tools/ci/linux-runner.sh sets it on Linux). It is cloned once into the runner's
# work folder ($RUNNER_WORKSPACE/shared-local; APFS clones, or reflinks where the file system has them, so no second
# copy of the bytes), and .local/ links to it as in a parity worktree.
# .local/horizon-client and the exporter outputs beside it, what horizon-prepare makes, are prepared by this
# checkout's own preparer once per preparer version (its sources, the exporters and research-lock.json), so a
# change to the preparer is checked with what it prepares; the four newest preparations are kept on legacy runners.
# Dyna uses a per-slot HORIZON_PREPARED_CACHE and read-only HORIZON_SHARED_SOURCE; old keys stay until slot GC.
set -euo pipefail
shared="${HORIZON_PREPARED_CACHE:-${RUNNER_WORKSPACE:?}/shared-local}"
source="${HORIZON_SHARED_SOURCE:-$HOME/code/horizon-parity/shared-local}"
dyna="${HORIZON_DYNA_CACHE:-false}"
version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' research-lock.json | head -1)
if [ ! -f "$shared/minecraft/$version/prepared.json" ]; then
  [ -f "$source/minecraft/$version/prepared.json" ] || { echo "::error::no imported Minecraft $version"; exit 1; }
fi
mkdir -p .local "$shared/prepared"

# HORIZON_PREPARE_KEY: the same key, computed where the Git history is (tools/pr_ready.py on another machine).
sha256() { if command -v shasum > /dev/null; then shasum -a 256; else sha256sum; fi; }
key=${HORIZON_PREPARE_KEY:-$(git ls-files -s crates/horizon-prepare mods/horizon/src/test/java research-lock.json tools/ci/vanilla-data.sh tools/ci/seed_prepared_extras.py tools/prepare_output.py | sha256 | cut -c1-16)}
prepared="$shared/prepared/$key"
report_prepared() {
  echo "prepared cache footprint KiB: $(du -sk "$prepared" | cut -f1)"
}
# The lock protects preparation; flock releases on death, mkdir is the portable fallback.
lock="$shared/prepared/.lock"
if command -v flock >/dev/null; then
  exec 9>"$shared/prepared/.lock-file"
  flock 9
else
  until mkdir "$lock" 2>/dev/null; do
    if [ -n "$(find "$lock" -maxdepth 0 -mmin +30 2>/dev/null)" ]; then rmdir "$lock" 2>/dev/null || true; fi
    sleep 5
  done
  trap 'rmdir "$lock" 2>/dev/null || true' EXIT
fi
if [ "$dyna" = true ] && [ -f "$prepared/.entries" ] && grep -qx horizon-client "$prepared/.entries"; then
  echo "prepared key $key fresh"
  for name in minecraft tools; do ln -sfn "$source/$name" ".local/$name"; done
  while read -r name; do ln -sfn "$prepared/$name" ".local/$name"; done < "$prepared/.entries"
  report_prepared
  exit 0
fi
if [ "$dyna" = true ]; then
  echo "prepared key $key stale"
  # The preparer owns its scratch data; never let it write through the source mount.
  for name in minecraft tools; do ln -sfn "$source/$name" ".local/$name"; done
  # A killed preparation may have left private scratch that must not enter the next key.
  for name in datagen command-registries horizon-client horizon-client-motion horizon-client-health horizon-client-collision horizon-client-render; do
    if [ ! -L ".local/$name" ]; then rm -rf ".local/$name"; fi
  done
else
  for entry in "$source"/*; do
    name=$(basename "$entry")
    [ "$name" = horizon-client ] || [ -e "$shared/$name" ] && continue
    if [ "$(uname)" = Darwin ]; then cp -cR "$entry" "$shared/"; else cp -R --reflink=auto "$entry" "$shared/"; fi
  done
  for entry in "$shared"/*; do
    name=$(basename "$entry")
    case "$name" in prepared|sounds|languages|*.log) continue ;; esac
    if [ -L ".local/$name" ] || [ ! -e ".local/$name" ]; then ln -sfn "$entry" ".local/$name"; fi
  done
fi
# horizon-prepare writes .local/horizon-client and its exporters' outputs beside it: all of them live in
# $prepared, listed in $prepared/.entries, and .local/ links to them. A real folder of that kind in .local/ is
# what an older failed preparation left: it would hide the preparations (and a new one would not count it as its
# own output), so it goes.
outputs=$({ cat "$shared"/prepared/*/.entries; cd .local && ls -d horizon-client*; } 2>/dev/null || true)
for name in $(echo "$outputs" | LC_ALL=C sort -u); do
  if [ -e ".local/$name" ] && [ ! -L ".local/$name" ]; then
    rm -rf ".local/${name:?}"
    echo "Removed .local/$name, left by a failed preparation"
  fi
done
# A preparation made while such a folder was there did not list it (horizon-client above all), so it is made again.
if [ -f "$prepared/.entries" ] && ! grep -qx horizon-client "$prepared/.entries"; then
  echo "Preparation $key lacks horizon-client: preparing again"
  rm -rf "$prepared"
fi
if [ ! -f "$prepared/.entries" ]; then
  echo "Preparing resources with this checkout's horizon-prepare ($key)"
  rm -rf "$prepared" && mkdir -p "$prepared"
  # Links to another preparation's outputs go first, so what this one writes shows up as new entries.
  for entry in .local/*; do
    if [ -L "$entry" ] && [[ "$(readlink "$entry")" == "$shared/prepared/"* ]]; then rm "$entry"; fi
  done
  before=$(ls -1A .local | LC_ALL=C sort)
  started=$SECONDS
  if [ "$dyna" = true ]; then
    cargo build --release --locked -p horizon-prepare >"$RUNNER_TEMP/prepare-build.log" 2>&1 || { echo "::error::preparer build failed"; exit 1; }
  else
    cargo build --release --locked -p horizon-prepare
  fi
  if ! "${CARGO_TARGET_DIR:-target}/release/horizon-prepare" --root "$PWD" > "$RUNNER_TEMP/prepare.log" 2>&1; then
    if [ "$dyna" = true ]; then echo "::error::preparation failed; private log retained in runner temp"; else tail -40 "$RUNNER_TEMP/prepare.log"; fi
    # Nothing of a failed preparation stays in .local/: the next job prepares again.
    LC_ALL=C comm -13 <(echo "$before") <(ls -1A .local | LC_ALL=C sort) |
      while read -r name; do rm -rf ".local/${name:?}"; done
    rm -rf "$prepared"
    exit 1
  fi
  LC_ALL=C comm -13 <(echo "$before") <(ls -1A .local | LC_ALL=C sort) > "$RUNNER_TEMP/prepared-entries"
  while read -r name; do mv ".local/$name" "$prepared/$name"; done < "$RUNNER_TEMP/prepared-entries"
  if [ "$dyna" = true ]; then
    # Optional dispatch-only measurement before shared extras can affect the output.
    if [ -n "${HORIZON_PREPARE_EXPORT_DIGEST:-}" ]; then
      python3 tools/prepare_output.py digest "$prepared/horizon-client/$version" > "$HORIZON_PREPARE_EXPORT_DIGEST" 2> "$RUNNER_TEMP/export-digest.log" || { echo "::error::export digest unavailable"; exit 1; }
    fi
    # Only sounds, languages and their metadata may come from the shared preparation.
    python3 -m tools.ci.seed_prepared_extras "$source/horizon-client/$version" "$prepared/horizon-client/$version/" > "$RUNNER_TEMP/seed-extras.log" 2>&1 || { echo "::error::seeding launcher extras failed"; exit 1; }
  fi
  mv "$RUNNER_TEMP/prepared-entries" "$prepared/.entries"
  if [ "$dyna" != true ]; then echo "Prepared in $((SECONDS - started)) s: $(tr '\n' ' ' < "$prepared/.entries")"; fi
fi
# The game finds some data relative to its resources (<resources>/../../datagen): beside the prepared folders,
# the same neighbours as in the shared folder.
if [ "$dyna" = true ]; then
  for name in minecraft tools; do [ -e "$prepared/$name" ] || ln -sfn "$source/$name" "$prepared/$name"; done
else
  for entry in "$shared"/*; do
    name=$(basename "$entry")
    case "$name" in prepared|sounds|languages|*.log) continue ;; esac
    [ -e "$prepared/$name" ] || ln -sfn "$entry" "$prepared/$name"
  done
fi
while read -r name; do
  if [ -L ".local/$name" ] || [ ! -e ".local/$name" ]; then ln -sfn "$prepared/$name" ".local/$name"; else
    echo "::error::.local/$name is not a link to the prepared resources"; exit 1; fi
done < "$prepared/.entries"
# The game's sounds (horizon-prepare sounds copies them from the official launcher's asset store) depend only on the
# Minecraft version: kept once in $shared/sounds/<version>/ (sounds.json and sounds/) and copied into each preparation
# as clones (APFS, or reflinks on Linux: no second copy of the bytes). Not links: vanilla's resource packs refuse
# symbolic links inside a pack, so its sound oracles would find no sound files.
assets="$prepared/horizon-client/$version/assets/minecraft"
if [ -f "$shared/sounds/$version/sounds.json" ] && [ -d "$assets" ]; then
  for entry in sounds.json sounds; do
    [ -L "$assets/$entry" ] && rm "$assets/$entry"
    [ -e "$assets/$entry" ] && continue
    if [ "$(uname)" = Darwin ]; then cp -cR "$shared/sounds/$version/$entry" "$assets/"
    else cp -R --reflink=auto "$shared/sounds/$version/$entry" "$assets/"; fi
  done
fi
# The languages (horizon-prepare languages copies them from the official launcher's asset store, which no runner
# has) depend only on the Minecraft version too: kept once in $shared/languages/<version>/ (assets/pack.mcmeta, the
# language list, and assets/minecraft/lang/) and cloned into each preparation to fill missing files. Without them a
# preparation has en_us alone, and pack_screen's harness, which draws ten languages, fails.
languages="$shared/languages/$version/assets"
if [ -d "$languages/minecraft/lang" ] && [ -d "$assets" ]; then
  mkdir -p "$assets/lang"
  # File by file: `cp -n` exits 1 on macOS (and on some coreutils) when it skips one, which ends the script.
  for file in "$languages/minecraft/lang/"*; do
    [ -e "$assets/lang/${file##*/}" ] && continue
    if [ "$(uname)" = Darwin ]; then cp -c "$file" "$assets/lang/"; else cp --reflink=auto "$file" "$assets/lang/"; fi
  done
  if [ ! -e "$assets/../pack.mcmeta" ]; then
    if [ "$(uname)" = Darwin ]; then
      cp -c "$languages/pack.mcmeta" "$assets/../"
    else
      cp --reflink=auto "$languages/pack.mcmeta" "$assets/../"
    fi
  fi
fi
touch "$prepared"
if [ "$dyna" = true ]; then
  echo "prepared in $((SECONDS - started)) s"
  report_prepared
fi
# The four newest preparations stay.
# Dyna keeps old keys because another active job may still read them; slot GC handles them.
if [ "$dyna" != true ]; then
  ls -1dt "$shared/prepared"/*/ 2>/dev/null | tail -n +5 | while read -r old; do rm -rf "$old"; done
fi
