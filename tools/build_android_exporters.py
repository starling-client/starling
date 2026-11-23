#!/usr/bin/env python3
"""Build Sunrise's original exporter bundle for the APK, never package Minecraft files.

Requires a host JDK of the pinned major. SUNRISE_MINECRAFT_DIR can supply the
already downloaded input files; otherwise fetch the pinned jars and libraries
into an ignored build cache. No accounts, assets or player data are read.
"""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def sha1(path):
    if not path.is_file():
        return None
    digest = hashlib.sha1()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def fetch(url, destination, checksum, local=None):
    if sha1(destination) == checksum:
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    part = destination.with_suffix(destination.suffix + '.part')
    try:
        if local is not None and sha1(local) == checksum:
            shutil.copyfile(local, part)
        else:
            with urllib.request.urlopen(url, timeout=60) as response, part.open('wb') as out:
                shutil.copyfileobj(response, out)
        if sha1(part) != checksum:
            raise ValueError(f'Checksum mismatch: {destination.name}')
        part.replace(destination)
    finally:
        part.unlink(missing_ok=True)


def inputs(root):
    pins = json.loads((root / 'research-lock.json').read_text(encoding='utf-8'))['minecraft']
    version = pins['version']
    home = root / '.local/android-exporter-inputs'
    supplied = os.environ.get('SUNRISE_MINECRAFT_DIR')
    supplied = Path(supplied) if supplied else None
    manifest = home / 'versions' / version / (version + '.json')
    accepted = [pins['manifest_sha1'], *pins.get('accepted_manifest_sha1', [])]
    local_manifest = supplied / 'versions' / version / (version + '.json') if supplied else None
    if sha1(manifest) not in accepted and local_manifest and sha1(local_manifest) in accepted:
        manifest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(local_manifest, manifest)
    if sha1(manifest) not in accepted:
        # The latest version index may point at a republished manifest. Fetch
        # the immutable package pinned by the checkout, not a mutable latest URL.
        checksum = pins['manifest_sha1']
        fetch(f'https://piston-meta.mojang.com/v1/packages/{checksum}/{version}.json', manifest, checksum)
    data = json.loads(manifest.read_text(encoding='utf-8'))
    client = manifest.with_suffix('.jar')
    server = home / 'server.jar'
    for name, path in [('client', client), ('server', server)]:
        item = data['downloads'][name]
        if item['sha1'] != pins[name + '_sha1']:
            raise ValueError(f'The {name} jar is not pinned')
        local = supplied / path.relative_to(home) if supplied else None
        fetch(item['url'], path, item['sha1'], local)
    for library in data['libraries']:
        artifact = library.get('downloads', {}).get('artifact')
        if not artifact:
            continue
        relative = PurePosixPath(artifact['path'])
        if relative.is_absolute() or '..' in relative.parts or '\\' in artifact['path']:
            raise ValueError('Unsafe library path')
        local = supplied / 'libraries' / relative if supplied else None
        fetch(artifact['url'], home / 'libraries' / relative, artifact['sha1'], local)
    return home, server


def copy_bundle(source, destination):
    """Copy only the manifest's original classes; never copy classpath jars/data."""
    manifest = json.loads((source / 'manifest.json').read_text(encoding='utf-8'))
    classes = manifest['classes']
    if not classes:
        raise ValueError('Empty exporter bundle')
    for name, checksum in classes.items():
        path = PurePosixPath(name)
        if (path.is_absolute() or '..' in path.parts or '\\' in name
                or not name.startswith(('server/dev/sunriselab/parity/', 'client/dev/sunriselab/parity/'))
                or not name.endswith('.class')):
            raise ValueError('Non-Sunrise class in exporter bundle')
        if hashlib.sha256((source / path).read_bytes()).hexdigest() != checksum:
            raise ValueError('Damaged exporter class')
    if source.resolve() == destination.resolve():
        return
    if destination.exists():
        if destination.is_symlink() or not destination.resolve().is_relative_to(ROOT.resolve()):
            raise ValueError('Exporter destination is outside the checkout')
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    for name in classes:
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source / name, target)
    shutil.copyfile(source / 'manifest.json', destination / 'manifest.json')


def main():
    home, server = inputs(ROOT)
    env = dict(os.environ)
    # This is a host build, even when the caller builds an Android library.
    for name in ['CARGO_BUILD_TARGET', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS']:
        env.pop(name, None)
    result = subprocess.run([
        'cargo', 'run', '--manifest-path', str(ROOT / 'Cargo.toml'), '--profile', 'quick',
        '-p', 'sunrise-prepare', '--', 'compile-exporters', '--root', str(ROOT),
        '--prepare', '--minecraft-dir', str(home), '--server-jar', str(server),
    ], cwd=ROOT, env=env, text=True, encoding='utf-8', stdout=subprocess.PIPE, check=True)
    print(result.stdout, end='')
    bundle = Path(result.stdout.strip().splitlines()[-1])
    copy_bundle(bundle, ROOT / 'exporters')


if __name__ == '__main__':
    main()
