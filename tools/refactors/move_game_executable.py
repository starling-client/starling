#!/usr/bin/env python3
"""Move the engine's thin executable to the platform game crate."""
from pathlib import Path
import sys
from verify_refactor import verify

if '--verify' in sys.argv:
    verify(Path(__file__).resolve(), 'refactor: move thin game executable above engine')
    raise SystemExit(0)

root = Path(__file__).resolve().parents[2]
source = root / 'crates/horizon-client/src/main.rs'
target = root / 'crates/horizon-game/src/main.rs'
target.write_text(source.read_text())
source.unlink()
p = root / 'crates/horizon-client/Cargo.toml'
s = p.read_text().replace('default-run = "horizon-client"\n', '')
s = s.replace('[[bin]]\nname = "horizon-client"\npath = "src/main.rs"\n\n', '')
p.write_text(s)
p = root / 'crates/horizon-game/Cargo.toml'
p.write_text(p.read_text() + '\n[[bin]]\nname = "horizon-client"\npath = "src/main.rs"\n')
# Rewrite explicit Cargo package selections only; library references stay unchanged.
for base in ['tools', '.github', 'docs']:
    for p in (root / base).rglob('*'):
        if not p.is_file() or p == Path(__file__).resolve():
            continue
        try:
            s = p.read_text()
        except UnicodeError:
            continue
        updated = s.replace('-p horizon-client --bin horizon-client', '-p horizon-game --bin horizon-client')
        if updated != s:
            p.write_text(updated)
