#!/usr/bin/env bash
# Route rustc and clippy-driver through sccache, falling back to an uncached compile on a cache outage.
# Cargo nests RUSTC_WORKSPACE_WRAPPER (clippy-driver) inside RUSTC_WRAPPER. A workspace crate's Clippy run is
# cached like any compile: its key holds the sources and clippy.toml (both in its dep-info), the arguments
# (the [lints] table among them) and the toolchain, and a hit replays its warnings, so check_warnings.py still
# fails on them. Without it, every crate the slot's target did not hold fresh was linted again on each job:
# the server lint step's other feature graph took 356-590 s whatever the target held.
set -euo pipefail
# Cargo has already selected incremental compilation in rustc's arguments.
# sccache refuses CARGO_INCREMENTAL=1 even for non-incremental dependencies.
unset CARGO_INCREMENTAL
# sccache 0.18.0 src/lib.rs: an internal error prints this prefix and exits 2.
# commands.rs::handle_compile_finished forwards compiler stderr and its exit code.
# The exit code alone cannot distinguish the two (rustc can also exit 2).
if ! command -v sccache > /dev/null; then
  echo '::warning::sccache is unavailable; compiling uncached.' >&2
  exec "$@"
fi
errors=$(mktemp) || { echo '::warning::Cannot capture sccache diagnostics; compiling uncached.' >&2; exec "$@"; }
trap 'rm -f "$errors"' EXIT
for attempt in 1 2; do
  status=0
  sccache "$@" 2> "$errors" || status=$?
  cat "$errors" >&2
  if [ "$status" -ne 2 ] || ! grep -q '^sccache: error:' "$errors"; then
    exit "$status"
  fi
done
echo '::warning::sccache failed twice; compiling uncached.' >&2
rm -f "$errors"
trap - EXIT
exec "$@"
