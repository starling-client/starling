#!/usr/bin/env python3
"""Every Rust file under a workspace crate's `src/` is compiled: a `mod` item or an `include!` reaches it.

    python3 tools/ci/check_unreached_sources.py

Modules are plain `mod` items (`game.rs` names each file of a `game/<area>/` folder with a `#[path]`), so a file
nobody declares is not compiled, and its tests do not run. This walks every workspace target with the compiler's
module rules (sunrise-game-discovery `--files`) and names each `.rs` file under a member's `src/` that no target
reaches. CI's verify-static runs it.
"""

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from tools.ci import ignored_inventory  # noqa: E402

# Their build scripts write the module lists, which a source walk can't follow.
GENERATED_MODULE_LISTS = ("crates/sunrise-mod-api", "crates/sunrise-mod-vocab", "crates/sunrise-mod-api-gen/dispatch-tests",
                          "docs/modding/examples")


def reached(root: Path, targets) -> set[str]:
    binary = os.environ.get("SUNRISE_GAME_DISCOVERY")
    command = [binary] if binary else ["cargo", "run", "--quiet", "--locked", "-p", "sunrise-game-discovery",
                                      "--bin", "sunrise-game-discovery", "--"]
    inputs = "".join("\t".join(row) + "\n" for row in targets)
    result = subprocess.run([*command, str(root), "--files"], input=inputs, capture_output=True, text=True, cwd=ROOT)
    if result.returncode:
        raise SystemExit(f"module walk failed ({result.returncode}):\n{result.stderr}")
    return {line.replace(os.sep, "/") for line in result.stdout.splitlines()}


def unreached(root: Path) -> list[str]:
    root = root.resolve()
    targets = ignored_inventory.target_roots(root)
    compiled = reached(root, targets)
    trees = sorted({folder for _, _, _, folder in targets if folder not in GENERATED_MODULE_LISTS})
    return sorted(path.relative_to(root).as_posix() for tree in trees for path in (root / tree / "src").rglob("*.rs")
                  if path.relative_to(root).as_posix() not in compiled)


def main() -> int:
    missing = unreached(ROOT)
    for path in missing:
        print(f"{path}: no `mod` item or `include!` reaches this file; declare it in its parent module "
              "(game.rs for a file of a game/<area>/ folder)", file=sys.stderr)
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
