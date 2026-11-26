"""One-off conversion for #4907: the parity harnesses that run a sunrise-client lib test through
`cargo test -p sunrise-client --lib FILTER... -- ARGS...` run `lab.parity_fixtures()` with `FILTER... ARGS...`
instead, the same libtest command line without the lib's other tests. Prints the harnesses it changed."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CARGO_TEST = re.compile(r"""\[(\s*)(['"])cargo\2,\s*\2test\2,\s*(?:\2--locked\2,\s*)?\2-p\2,\s*\2sunrise-client\2,"""
                        r"""\s*\2--lib\2,""")

for harness in sorted(ROOT.glob("tools/test_*_parity.py")):
    text = harness.read_text()
    changed, start = False, 0
    while match := CARGO_TEST.search(text, start):
        quote = match.group(2)
        rest = text[match.end():]
        close = rest.index("]")
        if "--no-run" in rest[:close]:
            start = match.end()  # a build for its artifact, converted by hand
            continue
        separator = re.search(rf"{quote}--{quote},\s*", rest)
        if separator is None or separator.start() > close:
            raise SystemExit(f"{harness.name}: no `--` before the command's end")
        rest = rest[:separator.start()] + rest[separator.end():]
        text = text[:match.start()] + f"[{match.group(1)}lab.parity_fixtures()," + rest
        changed = True
    if changed:
        if not re.search(r"\bimport\b.*\blab\b", text):
            raise SystemExit(f"{harness.name}: does not import lab")
        harness.write_text(text)
        print(harness.name)
