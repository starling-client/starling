#!/usr/bin/env python3
"""Plan the vanilla-parity shares for .github/workflows/parity.yml.

Usage:
  parity_shards.py --matrix                  `matrix=` (the shares) and `of=` (how many)
  parity_shards.py --split SELECTION OUTDIR  the same two lines, after writing OUTDIR's bucket-N.json
                                             and OUTDIR's copy of the whole selection

One selection, computed once by the plan job from the tested tree, is split here into disjoint shares:
the names in sorted order, every Nth from the Ith. Each runner is handed its own list (`tools/parity.py
--harness-file`), so it runs exactly that share, and the merge checks their union against the selection.
This file is the one place that names the count. Raise it when the harness set outgrows a share's time
limit again: each step up is one more runner and one more share, and the shares pay the same setup time.
The runner's own pass records (`tools/parity.py --status`, kept in PARITY_SHARED) say what the set costs.
"""

import json
import sys
from pathlib import Path

BUCKETS = 4


def write_selection(selection, outdir, count=BUCKETS):
    """Write the whole selection and one file per share into outdir; return the count."""
    names = sorted(json.loads(Path(selection).read_text(encoding="utf-8")))
    outdir = Path(outdir)
    outdir.mkdir(parents=True, exist_ok=True)
    (outdir / "selection.json").write_text(json.dumps(names) + "\n", encoding="utf-8")
    for index in range(count):
        (outdir / f"bucket-{index}.json").write_text(json.dumps(names[index::count]) + "\n", encoding="utf-8")
    return count


def print_matrix(count=BUCKETS):
    print(f"matrix={json.dumps({'bucket': list(range(count))})}")
    print(f"of={count}")


def main(arguments):
    if arguments == ["--matrix"]:
        return print_matrix()
    if len(arguments) == 3 and arguments[0] == "--split":
        return print_matrix(write_selection(arguments[1], arguments[2]))
    if len(arguments) == 5 and arguments[0] == "--split" and arguments[3] == "--of":
        return print_matrix(write_selection(arguments[1], arguments[2], int(arguments[4])))
    raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
