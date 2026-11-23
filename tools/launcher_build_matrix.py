#!/usr/bin/env python3
"""Repository-selected launcher build matrix; upstream entries retain their metadata."""
import json
import os

PLATFORMS = [
    dict(name='linux', os='ubuntu-22.04', asset='sunrise-launcher-linux-x86_64.tar.gz', binary='sunrise-launcher'),
    dict(name='macos', mac=True, asset='sunrise-launcher-macos-arm64.zip', binary='sunrise-launcher', mac_arch='arm64'),
    dict(name='macos-x64', mac=True, target='x86_64-apple-darwin', asset='sunrise-launcher-macos-x64.zip', binary='sunrise-launcher', mac_arch='x64'),
    dict(name='windows', os='windows-latest', asset='sunrise-launcher-windows-x64.zip', binary='sunrise-launcher.exe', pe_arch='x64', setup_asset='sunrise-launcher-setup.exe', rustflags='-C target-feature=+crt-static'),
    dict(name='windows-arm64', os='windows-11-vs2026-arm', target='aarch64-pc-windows-msvc', asset='sunrise-launcher-windows-arm64.zip', binary='sunrise-launcher.exe', pe_arch='arm64', setup_asset='sunrise-launcher-setup-arm64.exe', rustflags='-C target-feature=+crt-static'),
]

def matrix(repository):
    if repository not in ('arismoko/horizon', 'horizon-rust/horizon'):
        raise ValueError(f'Unsupported launcher repository: {repository}')
    entries = [dict(entry) for entry in PLATFORMS]
    if repository == 'arismoko/horizon':
        entries = [entry for entry in entries if entry['name'] in ('linux', 'macos', 'windows')]
        for entry in entries:
            for key in ('asset', 'binary', 'setup_asset'):
                if key in entry:
                    entry[key] = entry[key].replace('sunrise-launcher', 'sunrise-fork-launcher')
    return {'include': entries}

if __name__ == '__main__':
    print(json.dumps(matrix(os.environ.get('GITHUB_REPOSITORY', 'arismoko/horizon')), separators=(',', ':')))
