#!/usr/bin/env python3
"""Package a built sunrise-client with its prepared resources into a private
tarball for one of your own Linux or macOS machines. The resources are Mojang
assets: keep the archive in ignored .local/ and never publish or commit it.

    cargo build --release -p sunrise-game -p sunrise-client-lib --bins
    python3 tools/package_native_client.py
"""
import argparse
import hashlib
import io
import json
import platform
import subprocess
import tarfile
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parent.parent
LOCAL = ROOT / '.local'
# Inputs of the exporters, not read by the game at runtime.
SKIP = {'classes', 'client-classes', 'export.log', 'model-state-export.log'}

LAUNCHER = r'''#!/bin/sh
# Starts Sunrise. Saves and options.txt live in $SUNRISE_HOME
# (default ~/.local/share/sunrise), so unpacking an update keeps them.
here=$(cd "$(dirname "$0")" && pwd)
if [ "$1" = "--install" ]; then
    apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    mkdir -p "$apps"
    cat > "$apps/sunrise.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Sunrise
Comment=Minecraft Java Edition @MINECRAFT@ in Rust
Exec="$here/sunrise.sh"
Icon=$here/sunrise.png
Terminal=false
Categories=Game;
EOF
    echo "Added Sunrise to the application menu ($apps/sunrise.desktop)"
    exit 0
fi
home="${SUNRISE_HOME:-${XDG_DATA_HOME:-$HOME/.local/share}/sunrise}"
mkdir -p "$home"
cd "$home" || exit 1
exec "$here/sunrise-client" "$@"
'''

README = '''Sunrise {version} ({commit}) for {platform}

Run ./sunrise.sh (add --help for options, e.g. --render-distance 24 or --creative).
./sunrise.sh --install adds Sunrise to the application menu.

Saves and options.txt are kept in ~/.local/share/sunrise (override with SUNRISE_HOME).
Esc pauses and releases the mouse; F3 shows the debug screen.

The resources folder contains Mojang's game assets for personal use only;
do not redistribute this archive.
'''


def git(*args):
    return subprocess.run(['git', *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def add_bytes(tar, name, data, mode=0o644):
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mode = mode
    tar.addfile(info, io.BytesIO(data))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, help='built sunrise-client (default: cargo release target)')
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    parser.add_argument('--out', type=Path, default=LOCAL / 'dist')
    args = parser.parse_args()

    if args.binary is None:
        metadata = json.loads(subprocess.run(
            ['cargo', 'metadata', '--format-version', '1', '--no-deps'],
            cwd=ROOT, check=True, capture_output=True, text=True).stdout)
        args.binary = Path(metadata['target_directory']) / 'release/sunrise-client'
    if not args.binary.is_file():
        raise SystemExit(f'missing {args.binary}: run cargo build --release -p sunrise-game')
    if not (args.resources / 'sunrise-resources.json').is_file():
        raise SystemExit(f'missing prepared resources in {args.resources}: run sunrise-prepare --prepare (cargo run --release -p sunrise-prepare -- --prepare)')
    if not (args.resources / 'assets/minecraft/sounds.json').is_file():
        print('warning: no sound assets; run cargo run -p sunrise-prepare -- sounds for audio')

    system = {'Linux': 'linux', 'Darwin': 'macos'}.get(platform.system())
    if system is None:
        raise SystemExit(f'unsupported platform {platform.system()}: Sunrise targets Linux and macOS')
    machine = {'AMD64': 'x86_64', 'arm64': 'aarch64'}.get(platform.machine(), platform.machine())
    plat = f'{system}-{machine}'
    commit = git('rev-parse', '--short', 'HEAD') + ('-dirty' if git('status', '--porcelain', '--untracked-files=no') else '')
    version = '0.1.0'
    name = f'sunrise-{version}-{commit}-{plat}'

    args.out.mkdir(parents=True, exist_ok=True)
    archive = args.out / f'{name}.tar.gz'
    with tarfile.open(archive, 'w:gz', compresslevel=1) as tar:
        tar.add(args.binary, 'sunrise/sunrise-client')
        tar.add(ROOT / 'LICENSE', 'sunrise/LICENSE')
        tar.add(ROOT / 'THIRD_PARTY_NOTICES.md', 'sunrise/THIRD_PARTY_NOTICES.md')
        ctl = args.binary.with_name('sunrise-ctl')  # built with --bins: scripts and agents play through it
        if ctl.is_file():
            tar.add(ctl, 'sunrise/sunrise-ctl')
        add_bytes(tar, 'sunrise/sunrise.sh', LAUNCHER.replace('@MINECRAFT@', lab.VERSION).encode(), 0o755)
        tar.add(ROOT / 'docs/media/icon-256.png', 'sunrise/sunrise.png')
        add_bytes(tar, 'sunrise/README.txt', README.format(version=version, commit=commit, platform=plat).encode())
        for path in sorted(args.resources.iterdir()):
            if path.name not in SKIP:
                tar.add(path, f'sunrise/resources/{path.name}')

    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (args.out / f'{archive.name}.sha256').write_text(f'{digest}  {archive.name}\n')
    print(f'{archive} ({archive.stat().st_size / 1e6:.0f} MB)\nsha256 {digest}')


if __name__ == '__main__':
    main()
