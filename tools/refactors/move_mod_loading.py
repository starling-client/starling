#!/usr/bin/env python3
"""Move the installed-code loading algorithm above the engine (#2534)."""
from pathlib import Path

import sys
from verify_refactor import verify

if '--verify' in sys.argv:
    verify(Path(__file__).resolve(), 'refactor: move installed code loading to mod host')
    raise SystemExit(0)

root = Path(__file__).resolve().parents[2]
entry = root / 'crates/horizon-client/src/entry.rs'
text = entry.read_text()
native = text[text.index('    let exe = std::env::current_exe();'):text.index('    horizon_threads::configure_allocator();')]
order = text[text.index('    let wasm = horizon_mod_wasm::load_installed'):text.index('    for id in ids {')]
# Move the algorithm verbatim, including its current refusal behavior.
body = native.replace('crate::Result<_>', 'Result<_, Box<dyn std::error::Error + Send + Sync>>').replace('game::exit_code::ERROR', '1')
body += order
host = root / 'crates/horizon-mod-host/src/loading.rs'
host.write_text('//! Installed native and wasm code loading.\n'
    'use horizon_mod_package::selection::LaunchArguments;\n'
    'pub fn load(mods: Vec<mod_api::ModEntry>, built: &\'static [horizon_mod_package::runtime::BuiltMod], launch: &LaunchArguments) -> (horizon_mod_package::runtime::Runtime, Vec<&\'static str>) {\n'
    + body + '    (native, ids)\n}\n')
text = text.replace(native, '').replace(order, '')
text = text.replace('    main_with(Vec::new(), &[], &[], startup);', '    start(horizon_mod_package::runtime::Runtime::default(), Vec::new(), &[], startup);')
text = text.replace('pub fn main_with(\n    mods: Vec<mod_api::ModEntry>,\n    slots: &[NativeSlot],\n    built: &\'static [horizon_mod_package::runtime::BuiltMod],', 'pub fn start(\n    native: horizon_mod_package::runtime::Runtime,\n    ids: Vec<&\'static str>,\n    slots: &[NativeSlot],')
text = text.replace('    launch: LaunchArguments,', '    pub launch: LaunchArguments,', 1)
entry.write_text(text)
