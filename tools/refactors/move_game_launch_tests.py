#!/usr/bin/env python3
"""Move executable integration tests with their Cargo binary owner."""
from pathlib import Path
import sys
from verify_refactor import verify

if '--verify' in sys.argv:
    verify(Path(__file__).resolve(), 'refactor: move launch integration tests with executable')
    raise SystemExit(0)

root = Path(__file__).resolve().parents[2]
for name in ['launch_api', 'busy_launch']:
    source = root / f'crates/horizon-client/tests/{name}.rs'
    target = root / f'crates/horizon-game/tests/{name}.rs'
    target.parent.mkdir(exist_ok=True)
    target.write_text(source.read_text())
    source.unlink()
    p = root / 'crates/horizon-client/Cargo.toml'
    p.write_text(p.read_text().replace(f'[[test]]\nname = "{name}"\npath = "tests/{name}.rs"\n\n', ''))
