"""The step summary of .github/workflows/hnmod-compat.yml: every shard's check.tsv, against the baseline.

    hnmod_compat_summary.py RESULTS BASELINE

RESULTS holds one folder per shard (check-N/check.tsv, hnmod_compat.sh's). BASELINE lists the mods that built on the
last measured Starling, one id a line (# comments). Writes Markdown: the counts, the mods of the baseline that no
longer build (with their first errors), the mods that build now and did not, and every failure.
"""

import pathlib
import sys


def main() -> None:
    results, baseline_path = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    rows = {}
    controls = []
    for tsv in sorted(results.glob("*/check.tsv")):
        for line in tsv.read_text(errors="replace").splitlines():
            cells = line.split("\t")
            if not cells or not cells[0]:
                continue
            if cells[0] == "_control":
                controls.append((tsv.parent.name, cells[1]))
                continue
            rows[cells[0]] = cells[1:]
    baseline = {
        line.strip()
        for line in baseline_path.read_text().splitlines()
        if line.strip() and not line.startswith("#")
    }
    ok = {m for m, r in rows.items() if r and r[0] == "ok"}
    checked = {m for m, r in rows.items() if r and r[0] in ("ok", "fail", "apply-fail")}
    lost = sorted(baseline - ok)
    gained = sorted(ok - baseline)
    print("## Native mods of the website on this commit\n")
    bad_controls = [name for name, state in controls if state != "ok"]
    if bad_controls:
        print(f"**The control (no mod) did not build in {', '.join(bad_controls)}**: those shards checked no mod.\n")
    print(f"- {len(ok)} of {len(checked)} checked build ({len(rows)} listed).")
    print(f"- Baseline: {len(baseline & ok)} of its {len(baseline)} still build.")
    if lost:
        print(f"\n### No longer build ({len(lost)})\n")
        for m in lost:
            r = rows.get(m)
            why = "not checked" if r is None else f"{r[0]}: {(r[3] if len(r) > 3 else '')[:300]}"
            print(f"- `{m}`: {why}")
    if gained:
        print(f"\n### Build now, not in the baseline ({len(gained)})\n")
        print(", ".join(f"`{m}`" for m in gained))
    failed = sorted(m for m in checked - ok if m not in lost)
    if failed:
        print(f"\n### Other failures ({len(failed)})\n")
        for m in failed:
            r = rows[m]
            print(f"- `{m}`: {r[0]}: {(r[3] if len(r) > 3 else '')[:300]}")
    print("\nOK ids, for the baseline:\n\n```")
    print("\n".join(sorted(ok)))
    print("```")


if __name__ == "__main__":
    main()
