#!/usr/bin/env python3
"""Add an area declaration and its inherent Game implementation, never a forwarder.

Usage: add_method.py AREA TIER NAME SIGNATURE ACCESS GAME_FILE IMPLEMENTATION_FILE < doc
The implementation file contains a complete `impl Game<'_>` block with the named
method. GAME_FILE is its destination under crates/horizon-client/src/game/.
ACCESS is WorldOnly, Metadata, ClientSetup or PublishedResources.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
API = ROOT / 'crates/horizon-mod-api/src/host'
DESCRIPTION = {
    "Host": "The host's methods both sides share of the `{a}` area, implemented by the game and reached through [`Host`].",
    "ServerHost": "The server host's methods of the `{a}` area, implemented by the game and reached through [`ServerHost`].",
    "ClientHost": "The client host's methods of the `{a}` area, implemented by the game and reached through [`ClientHost`].",
    "Backend": "The handles' reads of the `{a}` area, implemented by the game and reached through [`Backend`].",
    "ClientBackend": "The handles' client reads of the `{a}` area, implemented by the game and reached through [`Backend`].",
    "BackendMut": "The handles' writes of the `{a}` area, implemented by the game and reached through [`BackendMut`].",
    "ClientBackendMut": "The handles' client writes of the `{a}` area, implemented by the game and reached through [`ClientBackendMut`].",
}
TIERS = {'Host', 'ServerHost', 'ClientHost', 'Backend', 'ClientBackend', 'BackendMut', 'ClientBackendMut'}


def main():
    area, tier, name, signature, access, destination, implementation = sys.argv[1:]
    if tier not in TIERS or access not in {'WorldOnly', 'Metadata', 'ClientSetup', 'PublishedResources'}:
        sys.exit('unknown host tier or load access')
    if not re.fullmatch(r'[a-z][a-z0-9_]*', area) or not re.fullmatch(r'[a-z][a-z0-9_]*', name):
        sys.exit('area and method must be Rust identifiers')
    game = (ROOT / destination).resolve()
    if not game.is_relative_to(ROOT / 'crates/horizon-client/src/game'):
        sys.exit('GAME_FILE must be in the game source directory')
    body = Path(implementation).read_text()
    if not re.search(r'\bfn ' + re.escape(name) + r'\b', body) or not re.search(r'\bimpl Game', body):
        sys.exit('implementation must contain the inherent Game method')
    if not signature.startswith(f'fn {name}('):
        sys.exit('signature and method name disagree')
    trait = ''.join(p.capitalize() for p in area.split('_')) + tier
    path = API / f'{area}.rs'
    source = path.read_text() if path.exists() else (
        '//! Host method declarations; access defaults and game forwarders are generated.\n'
        '#![allow(unused_variables)]\nuse crate::*;\n'
    )
    if re.search(r'\bfn ' + re.escape(name) + r'\b', source):
        sys.exit('method already declared')
    side = 'client' if tier.startswith('Client') else 'server' if tier == 'ServerHost' else 'both'
    doc = ''.join(f'    /// {line}\n' for line in sys.stdin.read().rstrip('\n').splitlines())
    method = f'    /// Side: {side}.\n{doc}    #[access({access})]\n    {signature};\n'
    marker = f'pub trait {trait} {{\n'
    if marker in source:
        end = source.index('\n}\n', source.index(marker)) + 1
        source = source[:end] + method + source[end:]
    else:
        hidden = '// hidden: handle backend, reached through the union\n#[doc(hidden)]\n' if 'Backend' in tier else ''
        source += f'/// {DESCRIPTION[tier].format(a=area)}\n{hidden}#[horizon_mod_api_macros::host_area]\n{marker}{method}}}\n'
    game_source = game.read_text() if game.exists() else 'use crate::game::Game;\n'
    path.write_text(source)
    game.write_text(game_source + '\n' + body)


if __name__ == '__main__':
    main()
