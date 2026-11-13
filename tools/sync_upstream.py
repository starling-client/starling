#!/usr/bin/env python3
"""Prepare an upstream sync without committing or pushing.

Pass the full SHA of an upstream commit whose own ci.yml passed:
    python3 tools/sync_upstream.py <sha>
Run from a clean fork checkout. On a conflict, resolve it by hand, then run
    python3 tools/sync_upstream.py --continue
Review converted declarations and duplicate moments, and run rustfmt --edition
2024 on the generated Rust files before committing the sync.
The script does not decide which upstream CI run is eligible.
"""
import argparse
import importlib.util
from collections import defaultdict
from pathlib import Path
import re
import subprocess
import sys

spec = importlib.util.spec_from_file_location(
    'convert_sync_fragment', Path(__file__).with_name('convert_sync_fragment.py'))
converter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(converter)

ROOT = Path(__file__).resolve().parent.parent
HOOKS = Path('crates/horizon-mod-vocab/src/hooks')
FRAGMENTS = Path('tools/native-hook-fragments')
DECL = re.compile(r'^\s*pub\s+fn\s+(native_\w+)\s*\(', re.MULTILINE)


def git(*args, capture=False):
    return subprocess.run(['git', *args], cwd=ROOT, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None).stdout


def check_duplicates(root=ROOT):
    names = defaultdict(list)
    for path in (root / 'crates').rglob('*.rs'):
        for match in DECL.finditer(path.read_text()):
            names[match.group(1)].append(str(path.relative_to(root)))
    duplicates = {name: paths for name, paths in names.items() if len(paths) > 1}
    if duplicates:
        raise ValueError('duplicate public native functions: ' + '; '.join(
            f'{name}: {", ".join(paths)}' for name, paths in sorted(duplicates.items())))


def finish():
    fragments = ROOT / FRAGMENTS
    out = ROOT / HOOKS
    index = out / 'mod.rs'
    for path in sorted(fragments.glob('*.json')):
        try:
            stem = converter.convert(path, out)
        except (ValueError, KeyError, TypeError, OSError) as error:
            raise ValueError(f'cannot convert {path.relative_to(ROOT)}: {error}') from error
        lines = index.read_text().splitlines(keepends=True)
        modules = [i for i, line in enumerate(lines) if re.fullmatch(r'mod [a-z_][a-z0-9_]*;\n', line)]
        insertion = next((i for i in modules if lines[i] != 'mod base;\n' and lines[i] > f'mod {stem};\n'), modules[-1] + 1)
        lines.insert(insertion, f'mod {stem};\n')
        index.write_text(''.join(lines))
        path.unlink()
        git('add', str(HOOKS / f'{stem}.rs'), str(HOOKS / 'mod.rs'), str(path.relative_to(ROOT)))
    readme = fragments / 'README.md'
    if readme.exists():
        readme.unlink()
        git('add', str(FRAGMENTS / 'README.md'))
    check_duplicates(ROOT)
    tracked = git('ls-files', capture=True).splitlines()
    retired = [p for p in tracked if p == 'native-hooks.json' or p.startswith('tools/native-hook-fragments/')]
    if retired:
        raise ValueError('retired catalog files remain: ' + ', '.join(retired))
    print('Sync prepared. Review the merge and converted hooks; no commit or push was made.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('sha', nargs='?', help='full upstream commit SHA with passing ci.yml')
    parser.add_argument('--continue', dest='continuing', action='store_true')
    parser.add_argument('--check', action='store_true', help='check duplicate public native functions')
    args = parser.parse_args()
    if args.check:
        check_duplicates()
        return
    if args.continuing:
        merge_head = Path(git('rev-parse', '--git-path', 'MERGE_HEAD', capture=True).strip())
        if args.sha or not (ROOT / merge_head).exists():
            raise ValueError('--continue requires an unfinished merge and no SHA')
        if git('diff', '--name-only', '--diff-filter=U', capture=True).strip():
            raise ValueError('resolve and stage merge conflicts before --continue')
    else:
        if not args.sha or not re.fullmatch(r'[0-9a-f]{40}', args.sha):
            raise ValueError('provide the full 40-character upstream commit SHA')
        if git('status', '--porcelain', capture=True).strip():
            raise ValueError('start with a clean working tree')
        git('fetch', 'https://github.com/horizon-rust/horizon.git', 'main')
        git('merge-base', '--is-ancestor', args.sha, 'FETCH_HEAD')
        git('merge', '--no-commit', '--no-ff', args.sha)
    finish()


if __name__ == '__main__':
    try:
        main()
    except (ValueError, subprocess.CalledProcessError) as error:
        print(f'upstream sync stopped: {error}', file=sys.stderr)
        sys.exit(1)
