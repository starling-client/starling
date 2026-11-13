#!/usr/bin/env bash
# Cargo nests RUSTC_WORKSPACE_WRAPPER (clippy-driver) inside RUSTC_WRAPPER.
# Keep hand-written workspace linting direct and cache ordinary rustc invocations.
set -euo pipefail
case "${1##*/}" in
  clippy-driver | clippy-driver.exe)
    crate_name= previous=
    for arg in "$@"; do
      if [ "$previous" = --crate-name ]; then
        crate_name=$arg
        break
      fi
      previous=$arg
    done
    if [ "$crate_name" != mod_points ]; then
      exec "$@"
    fi
    # The points API is generated: its 29 MB expansion takes seconds to type-check
    # but many minutes in Clippy's lint passes. Still type-check it, denying rustc
    # warnings, and leave every other workspace crate's Clippy invocation intact.
    shift # clippy-driver; the next argument is the real rustc
    args=()
    while [ "$#" -gt 0 ]; do
      case "$1" in
        -C)
          if [[ "${2:-}" == incremental=* ]]; then
            shift 2
            continue
          fi
          ;;
        -Cincremental=*) shift; continue ;;
      esac
      args+=("$1")
      shift
    done
    # Incremental units cannot be cached by sccache. Only this generated crate
    # drops its incremental flag; hand-written workspace crates keep theirs.
    set -- "${args[@]}" -D warnings
    ;;
esac
# Cargo has already selected incremental compilation in rustc's arguments.
# sccache refuses CARGO_INCREMENTAL=1 even for non-incremental dependencies.
unset CARGO_INCREMENTAL
exec sccache "$@"
