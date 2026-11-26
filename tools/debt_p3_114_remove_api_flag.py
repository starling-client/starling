#!/usr/bin/env python3
"""Remove the obsolete dependency resolver API flag and its callers (P3-114).

Run once on the part-two baseline (99c5682ffe6), then format the touched Rust files.
"""

import re
from pathlib import Path

root = Path(__file__).resolve().parents[1] / "crates"
resolver = root / "sunrise-mod-package/src/dependencies.rs"
source = resolver.read_text()
for pattern in (
    r"^    NoApi \{\n.*?^    \},\n",
    r"^            Self::NoApi \{\n.*?^            \}\n",
    r"^            if item\.code && other\.code && .*?^            \}\n",
    r"^        let required = [^\n]+\n        let mut base = .*?^        \)\);\n",
):
    source, count = re.subn(pattern, "", source, flags=re.M | re.S)
    if count != 1:
        raise SystemExit(f"expected one resolver match for {pattern}, got {count}")
source = source.replace("fn malformed_and_no_api()", "fn malformed_dependency_is_refused()")

updates = {resolver: source}
for file in ("api.rs", "contract.rs", "discover.rs", "registry_compile.rs"):
    path = root / "sunrise-native-build/src" / file
    updates[path] = path.read_text()
for path, text in updates.items():
    updates[path], count = re.subn(
        r"^ +(?:pub )?has_api: (?:bool|code|true|false),\n", "", text, flags=re.M
    )
    expected = 8 if path == resolver else 1
    if count != expected or re.search(r"\b(?:has_api|NoApi)\b", updates[path]):
        raise SystemExit(f"unexpected API flag usage in {path}")
for path, text in updates.items():
    path.write_text(text)
