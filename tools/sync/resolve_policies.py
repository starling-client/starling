#!/usr/bin/env python3
"""Apply audited path policies only to unmerged paths; print remaining areas."""
import fnmatch
import json
from pathlib import Path
import subprocess
import sys
from collections import Counter


def git(*args):
    return subprocess.check_output(['git', *args]).decode()


def patterns(name):
    return [line for line in Path('tools/sync/' + name).read_text().splitlines()
            if line and not line.startswith('#')]


def area(path):
    if '/lang/' in path:
        return 'translations'
    if path.startswith(('launcher/', 'crates/horizon-client-render/')):
        return 'renderlauncher'
    if path.startswith('crates/horizon-client/src/game/'):
        return 'game'
    if path.startswith('crates/horizon-client/'):
        return 'clientroot'
    if path.startswith('crates/'):
        return 'crates'
    return 'toolsdocs'


def main(target):
    # Workspace membership belongs to the fork. Only import upstream's two
    # new, non-component-runtime crates; let Cargo refresh the fork lockfile.
    unmerged = git('diff', '--name-only', '--diff-filter=U').splitlines()
    if 'Cargo.toml' in unmerged:
        manifest = git('show', 'HEAD:Cargo.toml')
        for member in ('crates/horizon-addresses', 'crates/horizon-platform'):
            if member not in manifest:
                manifest = manifest.replace('members = [', f'members = [\n    "{member}",', 1)
        Path('Cargo.toml').write_text(manifest)
        subprocess.run(['git', 'add', 'Cargo.toml'], check=True)
    if 'Cargo.lock' in unmerged:
        subprocess.run(['git', 'checkout', 'HEAD', '--', 'Cargo.lock'], check=True)
        print('Kept fork Cargo.lock; cargo check must refresh it.')
    removed = patterns('fork-removed.txt')
    upstream = patterns('upstream-owned.txt')
    generated = [line.split('\t')[0] for line in patterns('generated.txt')]
    for path in git('diff', '--name-only', '--diff-filter=U').splitlines():
        if any(fnmatch.fnmatchcase(path, p) for p in removed):
            # Only remove when absent in our pre-merge tree, never suppress edits.
            exists = subprocess.run(['git', 'cat-file', '-e', 'HEAD:' + path], capture_output=True).returncode == 0
            if not exists:
                subprocess.run(['git', 'rm', '-f', '--', path], check=True)
                print('Kept fork removal:', path)
        elif path in generated or any(fnmatch.fnmatchcase(path, p) for p in upstream):
            subprocess.run(['git', 'checkout', target, '--', path], check=True)
            print('Took upstream (regenerate if listed):', path)
    remaining = git('diff', '--name-only', '--diff-filter=U').splitlines()
    # Client translations may only name canonical en_us keys. Upstream adds
    # translations for features intentionally removed from the fork's catalog.
    lang_root = Path('crates/horizon-client/assets/horizon/lang')
    canonical = lang_root / 'en_us.json'
    if canonical.exists() and str(canonical) not in remaining:
        keys = json.loads(canonical.read_text()).keys()
        for path in lang_root.glob('*.json'):
            if str(path) in remaining:
                continue
            data = json.loads(path.read_text())
            stale = data.keys() - keys
            if stale:
                path.write_text(json.dumps({k: v for k, v in data.items() if k in keys}, ensure_ascii=False, indent=2) + '\n')
                subprocess.run(['git', 'add', '--', str(path)], check=True)
                print(f'Pruned {len(stale)} noncanonical keys: {path}')
    for group, count in sorted(Counter(map(area, remaining)).items()):
        print(f'{group}: {count}')
    print(f'Remaining REAL conflicts: {len(remaining)}')


if __name__ == '__main__':
    main(sys.argv[1])
