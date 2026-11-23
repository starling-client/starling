#!/usr/bin/env python3
"""Move the engine's thin executable to the platform game crate."""
from pathlib import Path
import sys
from verify_refactor import verify

if '--verify' in sys.argv:
    verify(Path(__file__).resolve(), 'refactor: move thin game executable above engine')
    raise SystemExit(0)

root = Path(__file__).resolve().parents[2]
source = root / 'crates/sunrise-client/src/main.rs'
target = root / 'crates/sunrise-game/src/main.rs'
target.write_text(source.read_text())
source.unlink()
p = root / 'crates/sunrise-client/Cargo.toml'
s = p.read_text().replace('default-run = "sunrise-client"\n', '')
s = s.replace('[[bin]]\nname = "sunrise-client"\npath = "src/main.rs"\n\n', '')
p.write_text(s)
p = root / 'crates/sunrise-game/Cargo.toml'
p.write_text(p.read_text() + '\n[[bin]]\nname = "sunrise-client"\npath = "src/main.rs"\n')
# Rewrite explicit Cargo package selections only; library references stay unchanged.
for base in ['tools', '.github', 'docs']:
    for p in (root / base).rglob('*'):
        if not p.is_file() or p == Path(__file__).resolve():
            continue
        try:
            s = p.read_text()
        except UnicodeError:
            continue
        updated = s.replace('-p sunrise-client --bin sunrise-client', '-p sunrise-game --bin sunrise-client')
        if updated != s:
            p.write_text(updated)
