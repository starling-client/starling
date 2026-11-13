#!/usr/bin/env bash
# Run the workflow's parity harnesses with scratch outputs on the disk-backed slot.
set -euo pipefail
/usr/bin/python3 "$@"
case "${1:-}" in
  tools/test_*_parity.py) rm -rf "$DYNA_CI_BUILD"/parity-out/rust-* ;;
esac
