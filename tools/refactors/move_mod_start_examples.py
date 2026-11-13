#!/usr/bin/env python3
"""Move mod-starting examples above the engine to avoid a dependency cycle."""
from pathlib import Path
import sys
from verify_refactor import verify

if '--verify' in sys.argv:
    verify(Path(__file__).resolve(), 'refactor: move mod starting examples above engine')
    raise SystemExit(0)

root = Path(__file__).resolve().parents[2]
for name in ['native_gui_overlay_probe', 'native_waypoints', 'wind_burst_hook_probe']:
    p = root / f'crates/horizon-client/examples/{name}.rs'
    target = root / f'crates/horizon-mod-host/examples/{name}.rs'
    target.parent.mkdir(exist_ok=True)
    target.write_text(p.read_text().replace('horizon_client::entry::main_with', 'horizon_mod_host::main_with'))
    p.unlink()
    manifest = root / 'crates/horizon-client/Cargo.toml'
    s = manifest.read_text().replace(f'[[example]]\nname = "{name}"\npath = "examples/{name}.rs"\n\n', '')
    manifest.write_text(s)
