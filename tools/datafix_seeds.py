#!/usr/bin/env python3
"""Keyed input-seed cache for the DataFixer parity harness (tools/test_datafix_parity.py).

CI runs `datafix` across multiple buckets on the same host, each with a disposable private
SUNRISE_DATAFIX_ROOT. When SUNRISE_DATAFIX_SEED_KEEP is set (from $DYNA_CI_KEEP/datafix-seeds
in parity.yml), this module keeps immutable, complete legacy seed-world snapshots (`worlds/`
and `worlds-pre-1.17/` only) keyed by every input that can affect seed generation, publishes
under an exclusive host lock, copies or reflinks into each job's private root, and prunes the
keep by age and byte budget.

Outputs, `build/`, `run/`, downloaded jars and legacy JVMs stay out of the keep; both engines
still upgrade and compare every world fresh inside each job's private root. When
SUNRISE_DATAFIX_SEED_KEEP is unset, callers take the existing local generation path unchanged.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

try:
    from . import datafix_paths, file_lock
except ImportError:
    import datafix_paths
    import file_lock

ROOT = Path(__file__).resolve().parent.parent
KEEP_VARIABLE = 'SUNRISE_DATAFIX_SEED_KEEP'
MANIFEST_VARIABLE = 'SUNRISE_DATAFIX_MANIFEST'
OUTCOME_VARIABLE = 'SUNRISE_DATAFIX_SEEDS_OUTCOME'
LOCK_NAME = 'seeds.lock'
CACHE_FORMAT = 1
SNAPSHOT_DIRS = ('worlds', 'worlds-pre-1.17')
# Stage failures recorded in datafix-source.json['problems'] by tools/datafix_scene_12.py when a
# required fixture stage did not complete (deterministic version-skew command failures are still
# tolerated, matching tools/test_datafix_parity.py).
BLOCKING_PROBLEM_STAGES = frozenset({'last tick', 'lab_actions'})
# Dedicated budget beside ORACLE_KEEP_BUDGET (12 GiB in tools/parity.py): holds several revisions
# while keeping oracles + seeds under Dyna's 20 GiB CI keep ceiling and preventing oracle churn
# from evicting the active seed set.
SEED_KEEP_BUDGET = 2 << 30
# Match tools/parity.py's prune_oracle_keep cutoffs: ORACLE_KEEP_DAYS (14 days) for kept entries
# and 1 day (86400 s) for abandoned '.'-prefixed staging folders left behind by killed writers.
SEED_KEEP_DAYS = 14
STALE_STAGING_SECONDS = 86400
_FICLONE = 0x40049409  # Linux ioctl(2) FICLONE

# Every tracked source file that participates in seed world generation or snapshot layout.
SOURCE_FILES = (
    'tools/anvil_dump.py',
    'tools/datafix_paths.py',
    'tools/datafix_scene.py',
    'tools/datafix_scene_12.py',
    'tools/datafix_scene_old.py',
    'tools/datafix_seeds.py',
    'tools/datafix_worlds.py',
    'tools/lab.py',
)


def _worlds_module():
    try:
        from . import datafix_worlds
    except ImportError:
        import datafix_worlds
    return datafix_worlds


def _parity_shim_module():
    try:
        from . import parity_shim
    except ImportError:
        import parity_shim
    return parity_shim


def keep_root(path):
    """Resolve and validate the seed-keep directory at the boundary."""
    resolved = Path(path).expanduser().resolve()
    forbidden = datafix_paths.READ_ONLY.resolve()
    if resolved.is_relative_to(forbidden) or forbidden.is_relative_to(resolved):
        raise ValueError(f'{KEEP_VARIABLE} resolves to {resolved}: overlaps read-only vanilla fixtures')
    if resolved.exists() and not resolved.is_dir():
        raise ValueError(f'{KEEP_VARIABLE} resolves to {resolved}: not a directory')
    return resolved


def world_subdir(name):
    worlds = _worlds_module()
    return 'worlds-pre-1.17' if name.split('-')[0] in worlds.LEGACY else 'worlds'


def expected_worlds():
    """Ordered (subdir, world_name) pairs for every world datafix_worlds.py produces."""
    worlds = _worlds_module()
    names = []
    for version in worlds.VERSIONS:
        names.append(version)
        if version == '1.12.2':
            names.append('1.12.2-flat')
    names.extend(worlds.VARIANTS)
    return [(world_subdir(name), name) for name in names]


def required_versions():
    worlds = _worlds_module()
    return sorted({*worlds.VERSIONS, *(version for version, _ in worlds.VARIANTS.values())})


def source_digests(root=ROOT, files=SOURCE_FILES):
    return {rel: hashlib.sha256((Path(root) / rel).read_bytes()).hexdigest() for rel in sorted(files)}


def fetch_manifest(url=None):
    worlds = _worlds_module()
    with urllib.request.urlopen(url or worlds.MANIFEST) as response:
        return json.load(response)


def resolve_version_pins(versions=None, manifest=None):
    """Map each required version to its verified version-metadata sha1 from Mojang's manifest.
    Each sha1 commits to that version's version.json, including downloads.server.sha1 and javaVersion."""
    versions = required_versions() if versions is None else sorted(set(versions))
    manifest = fetch_manifest() if manifest is None else manifest
    by_id = {entry['id']: entry for entry in manifest['versions']}
    pins = {}
    for version in versions:
        entry = by_id.get(version)
        if not entry or not isinstance(entry.get('sha1'), str) or not entry['sha1']:
            raise ValueError(f'version manifest missing verified sha1 for {version}')
        pins[version] = entry['sha1']
    return pins


def default_java_identity():
    """Content identity of the default `java` JDK used for 1.20.5+ servers, reusing
    tools/parity_shim.py's direct-JDK check and JDK content hashing."""
    shim = _parity_shim_module()
    program = shim.real('java') if os.environ.get('PARITY_SHIM_REAL_PATH') else shutil.which('java')
    if not program:
        raise RuntimeError('java not found on PATH')
    if os.environ.get('JAVA_HOME') and not shim.direct_jdk(program, 'java'):
        raise RuntimeError(f'java ({program}) is not JAVA_HOME direct launcher; cannot key JDK by content')
    resolved = os.path.realpath(program)

    class _FileHasher:
        @staticmethod
        def file(path):
            with open(path, 'rb') as stream:
                return hashlib.file_digest(stream, 'sha256').hexdigest()

    identity = shim.toolchain(resolved, _FileHasher())
    if len(identity) <= 1:
        raise RuntimeError(f'java ({resolved}) lacks JDK runtime files ({", ".join(shim.JDK_FILES)})')
    return identity


def seed_key(root=ROOT, *, version_pins=None, platform_name=None, runtime_pins=None, default_java=None, expected=None):
    """Content key for a complete datafix seed snapshot."""
    worlds = _worlds_module()
    plat = worlds.platform() if platform_name is None else platform_name
    payload = {
        'format': CACHE_FORMAT,
        'sources': source_digests(root),
        'expected_worlds': sorted([subdir, name] for subdir, name in (expected_worlds() if expected is None else expected)),
        'versions': dict(sorted((resolve_version_pins() if version_pins is None else version_pins).items())),
        'platform': plat,
        'java_runtimes': worlds.LOCK[plat] if runtime_pins is None else runtime_pins,
        'default_java': default_java_identity() if default_java is None else default_java,
    }
    return hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(',', ':')).encode('utf-8')).hexdigest()


def snapshot_complete(root, expected=None, *, strict_top=False):
    """True only when every expected world has a non-empty level.dat and valid datafix-source.json,
    with no partial or unexpected worlds (and, for kept snapshots, no build/run/scratch beside them)."""
    root = Path(root)
    if not root.is_dir():
        return False
    expected = expected_worlds() if expected is None else expected
    by_subdir = {subdir: set() for subdir in SNAPSHOT_DIRS}
    for subdir, name in expected:
        by_subdir.setdefault(subdir, set()).add(name)
    if strict_top and {entry.name for entry in root.iterdir()} != set(by_subdir):
        return False
    for subdir, names in by_subdir.items():
        parent = root / subdir
        if not parent.is_dir() or parent.is_symlink():
            return False
        if {entry.name for entry in parent.iterdir()} != names:
            return False
        for name in names:
            world = parent / name
            if not world.is_dir() or world.is_symlink():
                return False
            level = world / 'level.dat'
            if not level.is_file() or level.is_symlink() or level.stat().st_size == 0:
                return False
            meta = world / 'datafix-source.json'
            if not meta.is_file() or meta.is_symlink():
                return False
            try:
                info = json.loads(meta.read_text(encoding='utf-8'))
            except (OSError, ValueError):
                return False
            if (not isinstance(info, dict) or not isinstance(info.get('version'), str)
                    or type(info.get('data_version')) is not int):
                return False
            problems = info.get('problems')
            if not isinstance(problems, list):
                return False
            for entry in problems:
                if (not isinstance(entry, list) or len(entry) != 2
                        or not isinstance(entry[0], str) or not isinstance(entry[1], list)
                        or not all(isinstance(line, str) for line in entry[1])):
                    return False
                stage, lines = entry
                if stage in BLOCKING_PROBLEM_STAGES:
                    return False
                if stage == 'save-all flush' and (not lines or any("Can't keep up" not in line for line in lines)):
                    return False
    return True


def copy_or_reflink(src, dst, *, follow_symlinks=True):
    """Copy a regular file, using a CoW reflink on Linux when the filesystem supports it."""
    if not follow_symlinks and os.path.islink(src):
        os.symlink(os.readlink(src), dst)
        return dst
    with open(src, 'rb') as source, open(dst, 'wb') as target:
        cloned = False
        if sys.platform.startswith('linux'):
            import fcntl
            try:
                fcntl.ioctl(target.fileno(), _FICLONE, source.fileno())
                cloned = True
            except OSError:
                cloned = False
        if not cloned:
            shutil.copyfileobj(source, target)
    shutil.copystat(src, dst, follow_symlinks=follow_symlinks)
    return dst


def copy_snapshot(snapshot_dir, dest_root):
    """Copy or reflink only worlds/ and worlds-pre-1.17/ from a kept snapshot into a private root."""
    snapshot_dir, dest_root = Path(snapshot_dir), Path(dest_root)
    dest_root.mkdir(parents=True, exist_ok=True)
    for subdir in SNAPSHOT_DIRS:
        src = snapshot_dir / subdir
        dst = dest_root / subdir
        partial = dest_root / f'.{subdir}.{os.getpid()}.partial'
        shutil.rmtree(partial, ignore_errors=True)
        try:
            shutil.copytree(src, partial, symlinks=False, copy_function=copy_or_reflink)
            shutil.rmtree(dst, ignore_errors=True)
            partial.rename(dst)
        except Exception:
            shutil.rmtree(partial, ignore_errors=True)
            raise


def stage_snapshot(src_root, staging_dir, expected):
    """Stage only the expected world folders into staging_dir (never servers, java, gen, build or run)."""
    src_root, staging_dir = Path(src_root), Path(staging_dir)
    for subdir in SNAPSHOT_DIRS:
        (staging_dir / subdir).mkdir(parents=True, exist_ok=True)
    for subdir, name in expected:
        shutil.copytree(src_root / subdir / name, staging_dir / subdir / name,
                        symlinks=False, copy_function=copy_or_reflink)


def prune_seed_keep(keep, budget=None, max_age_days=None):
    """Drop snapshots unused for SEED_KEEP_DAYS and abandoned staging folders, then least-recently-used
    snapshots until total size fits SEED_KEEP_BUDGET."""
    limit = SEED_KEEP_BUDGET if budget is None else budget
    days = SEED_KEEP_DAYS if max_age_days is None else max_age_days
    now, kept = time.time(), []
    for entry in Path(keep).iterdir():
        if entry.name == LOCK_NAME:
            continue
        try:
            stat = entry.stat()
        except FileNotFoundError:
            continue
        age = now - stat.st_mtime
        if entry.name.startswith('.'):
            if age > STALE_STAGING_SECONDS:
                shutil.rmtree(entry, ignore_errors=True)
            continue
        if not entry.is_dir():
            continue
        if age > days * 86400:
            shutil.rmtree(entry, ignore_errors=True)
            continue
        size = sum(f.stat().st_size for f in entry.rglob('*') if f.is_file() and not f.is_symlink())
        kept.append((stat.st_mtime, entry.name, size, entry))
    total = sum(size for _, _, size, _ in kept)
    for _, _, size, entry in sorted(kept):
        if total <= limit:
            break
        shutil.rmtree(entry, ignore_errors=True)
        total -= size


def _run_generator(scratch, manifest_path):
    env = {**os.environ, datafix_paths.VARIABLE: str(scratch)}
    if manifest_path.is_file():
        env[MANIFEST_VARIABLE] = str(manifest_path)
    subprocess.run([sys.executable, str(ROOT / 'tools' / 'datafix_worlds.py')], cwd=ROOT, env=env, check=True)


def prepare(dest_root, keep_dir, *, key=None, manifest=None, expected=None, generate=None):
    """Ensure dest_root holds a complete private copy of the keyed seed snapshot.

    Under the keep's exclusive lock: a complete kept snapshot is touched and copied/reflinked into
    dest_root. On a miss, seeds are generated into disposable private scratch, verified complete,
    staged with only worlds/ and worlds-pre-1.17/, atomically published, and copied into dest_root.
    The keep is pruned after copying so the caller always receives its private copy even if a
    snapshot exceeds the keep budget.
    """
    dest = Path(dest_root).expanduser().resolve()
    keep = keep_root(keep_dir)
    if dest == keep or dest.is_relative_to(keep) or keep.is_relative_to(dest):
        raise ValueError(f'{KEEP_VARIABLE} ({keep}) and {datafix_paths.VARIABLE} ({dest}) must be separate directories')
    expected = expected_worlds() if expected is None else expected
    if key is None:
        manifest = fetch_manifest() if manifest is None else manifest
        key = seed_key(version_pins=resolve_version_pins(manifest=manifest), expected=expected)
    started = time.monotonic()
    keep.mkdir(parents=True, exist_ok=True)
    dest.mkdir(parents=True, exist_ok=True)
    with open(keep / LOCK_NAME, 'w') as handle:
        file_lock.lock(handle)
        snapshot_dir = keep / key
        if snapshot_dir.exists() and snapshot_complete(snapshot_dir, expected, strict_top=True):
            os.utime(snapshot_dir)
            copy_snapshot(snapshot_dir, dest)
            prune_seed_keep(keep)
            return {'key': key, 'status': 'hit', 'worlds': len(expected), 'elapsed': time.monotonic() - started}
        if snapshot_dir.exists():
            shutil.rmtree(snapshot_dir, ignore_errors=True)
        scratch = Path(tempfile.mkdtemp(prefix='.seed-gen.', dir=dest))
        staging = keep / f'.{key}.{os.getpid()}.tmp'
        try:
            manifest_path = scratch / 'version_manifest_v2.json'
            if manifest is not None:
                manifest_path.write_text(json.dumps(manifest), encoding='utf-8')
            (generate or _run_generator)(scratch, manifest_path)
            if not snapshot_complete(scratch, expected, strict_top=False):
                raise RuntimeError(f'datafix seed generation did not produce a complete {len(expected)}-world snapshot')
            shutil.rmtree(staging, ignore_errors=True)
            stage_snapshot(scratch, staging, expected)
            if not snapshot_complete(staging, expected, strict_top=True):
                raise RuntimeError('staged datafix seed snapshot failed completeness check')
            os.rename(staging, snapshot_dir)
            copy_snapshot(snapshot_dir, dest)
            prune_seed_keep(keep)
        except Exception:
            shutil.rmtree(staging, ignore_errors=True)
            raise
        finally:
            shutil.rmtree(scratch, ignore_errors=True)
    return {'key': key, 'status': 'miss', 'worlds': len(expected), 'elapsed': time.monotonic() - started}


def main(arguments=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--keep', default=os.environ.get(KEEP_VARIABLE),
                        help=f'seed keep directory (default: ${KEEP_VARIABLE})')
    args = parser.parse_args(arguments)
    if not args.keep:
        raise SystemExit(f'{KEEP_VARIABLE} is not set and --keep was not given')
    result = prepare(datafix_paths.writable_root(), args.keep)
    print(f'datafix seeds: {result["status"]} {result["key"][:12]} ({result["worlds"]} worlds in {result["elapsed"]:.1f}s)',
          flush=True)


if __name__ == '__main__':
    main()
