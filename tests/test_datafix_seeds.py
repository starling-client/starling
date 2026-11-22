"""Unit tests for the keyed datafix input-seed cache (tools/datafix_seeds.py)."""
import hashlib
import importlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest import mock

from tools import datafix_paths, datafix_seeds


def make_world(root, subdir, name, version='1.19', data_version=3105, level_bytes=b'nbt'):
    world = Path(root) / subdir / name
    world.mkdir(parents=True, exist_ok=True)
    (world / 'level.dat').write_bytes(level_bytes)
    (world / 'datafix-source.json').write_text(
        json.dumps({'version': version, 'data_version': data_version, 'problems': []}),
        encoding='utf-8',
    )
    return world


class DatafixSeedKeyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for rel in datafix_seeds.SOURCE_FILES:
            path = self.root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f'# {rel}\n', encoding='utf-8')
        self.expected = [('worlds-pre-1.17', '1.12.2'), ('worlds', '1.19')]
        self.version_pins = {'1.12.2': 'a' * 40, '1.19': 'b' * 40}
        self.runtime_pins = {
            'jre-legacy': {'manifest_sha1': 'c' * 40, 'manifest_url': 'https://invalid.test/jre'},
            'java-runtime-gamma': {'manifest_sha1': 'd' * 40, 'manifest_url': 'https://invalid.test/gamma'},
        }
        self.default_java = {'/jdk/bin/java': '1' * 64, '/jdk/lib/modules': '2' * 64}

    def key(self, **overrides):
        params = dict(
            root=self.root,
            version_pins=self.version_pins,
            platform_name='linux',
            runtime_pins=self.runtime_pins,
            default_java=self.default_java,
            expected=self.expected,
        )
        params.update(overrides)
        return datafix_seeds.seed_key(**params)

    def test_key_is_stable_across_input_ordering(self):
        base = self.key()
        reordered = self.key(
            version_pins={'1.19': 'b' * 40, '1.12.2': 'a' * 40},
            runtime_pins={
                'java-runtime-gamma': {'manifest_url': 'https://invalid.test/gamma', 'manifest_sha1': 'd' * 40},
                'jre-legacy': {'manifest_url': 'https://invalid.test/jre', 'manifest_sha1': 'c' * 40},
            },
            expected=[('worlds', '1.19'), ('worlds-pre-1.17', '1.12.2')],
        )
        self.assertEqual(base, reordered)
        self.assertEqual(len(base), 64)

    def test_key_changes_for_every_defining_input(self):
        base = self.key()
        for rel in datafix_seeds.SOURCE_FILES:
            with self.subTest(source=rel):
                path = self.root / rel
                original = path.read_bytes()
                path.write_bytes(original + b'# changed\n')
                self.assertNotEqual(self.key(), base)
                path.write_bytes(original)

        self.assertNotEqual(self.key(version_pins={**self.version_pins, '1.19': 'e' * 40}), base)
        self.assertNotEqual(self.key(platform_name='mac-os-arm64'), base)
        self.assertNotEqual(
            self.key(runtime_pins={**self.runtime_pins, 'jre-legacy': {**self.runtime_pins['jre-legacy'], 'manifest_sha1': 'f' * 40}}),
            base,
        )
        self.assertNotEqual(
            self.key(default_java={**self.default_java, '/jdk/lib/modules': '3' * 64}),
            base,
        )
        self.assertNotEqual(self.key(expected=[*self.expected, ('worlds', '1.20.1')]), base)

    def test_default_java_identity_hashes_jdk_content_via_parity_shim(self):
        jdk = self.root / 'jdk'
        launcher = jdk / 'bin' / 'java'
        launcher.parent.mkdir(parents=True)
        launcher.write_bytes(b'#!/bin/sh\necho "openjdk version 25.0.5"\n')
        launcher.chmod(0o755)
        (jdk / 'release').write_text('JAVA_VERSION="25.0.5"\n', encoding='utf-8')
        modules = jdk / 'lib' / 'modules'
        modules.parent.mkdir(parents=True)
        modules.write_bytes(b'modules-v1')
        libjvm = jdk / 'lib' / 'server' / 'libjvm.so'
        libjvm.parent.mkdir(parents=True)
        libjvm.write_bytes(b'jvm-v1')

        env = {'JAVA_HOME': str(jdk), 'PATH': str(launcher.parent)}
        with mock.patch.dict(os.environ, env, clear=True):
            before = datafix_seeds.default_java_identity()
            # In-place JDK patch keeping identical java -version banner changes the content identity.
            libjvm.write_bytes(b'jvm-v2')
            after = datafix_seeds.default_java_identity()
        self.assertNotEqual(before, after)
        self.assertIn(str(libjvm.resolve()), after)

        # A wrapper outside JAVA_HOME is refused rather than keyed only by its script text.
        wrapper_dir = self.root / 'wrappers'
        wrapper_dir.mkdir()
        wrapper = wrapper_dir / 'java'
        wrapper.write_bytes(b'#!/bin/sh\nexec "$JAVA_HOME/bin/java" "$@"\n')
        wrapper.chmod(0o755)
        with mock.patch.dict(os.environ, {'JAVA_HOME': str(jdk), 'PATH': str(wrapper_dir)}, clear=True):
            with self.assertRaises(RuntimeError):
                datafix_seeds.default_java_identity()

    def test_resolve_version_pins_verifies_every_required_version(self):
        manifest = {'versions': [
            {'id': '1.19', 'sha1': 'b' * 40, 'url': 'https://invalid.test/1.19.json'},
            {'id': '1.12.2', 'sha1': 'a' * 40, 'url': 'https://invalid.test/1.12.2.json'},
        ]}
        self.assertEqual(
            datafix_seeds.resolve_version_pins(versions=['1.19', '1.12.2'], manifest=manifest),
            {'1.12.2': 'a' * 40, '1.19': 'b' * 40},
        )
        with self.assertRaisesRegex(ValueError, '1.20.1'):
            datafix_seeds.resolve_version_pins(versions=['1.19', '1.20.1'], manifest=manifest)
        bad = {'versions': [{'id': '1.19', 'sha1': ''}]}
        with self.assertRaisesRegex(ValueError, '1.19'):
            datafix_seeds.resolve_version_pins(versions=['1.19'], manifest=bad)

    def test_expected_worlds_covers_all_41_generated_worlds(self):
        expected = datafix_seeds.expected_worlds()
        self.assertEqual(len(expected), 41)
        self.assertEqual(len(set(expected)), 41)
        self.assertIn(('worlds-pre-1.17', '1.12.2-flat'), expected)
        self.assertIn(('worlds-pre-1.17', '1.14.4-buffet'), expected)
        self.assertIn(('worlds', '26.2'), expected)


class DatafixSeedSnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.shared = self.base / 'shared-readonly'
        self.shared.mkdir()
        self.stack = mock.patch.object(datafix_paths, 'READ_ONLY', self.shared)
        self.stack.start()
        self.addCleanup(self.stack.stop)
        self.expected = [('worlds-pre-1.17', '1.12.2'), ('worlds-pre-1.17', '1.12.2-flat'), ('worlds', '1.19')]

    def populate(self, root):
        make_world(root, 'worlds-pre-1.17', '1.12.2', '1.12.2', 1343, b'pre17-main')
        make_world(root, 'worlds-pre-1.17', '1.12.2-flat', '1.12.2', 1343, b'pre17-flat')
        make_world(root, 'worlds', '1.19', '1.19', 3105, b'modern-119')

    def test_completeness_rejects_partial_corrupt_or_polluted_snapshots(self):
        snap = self.base / 'snap'
        self.populate(snap)
        self.assertTrue(datafix_seeds.snapshot_complete(snap, self.expected, strict_top=True))

        # Extra top-level folders (build/run/servers) are rejected in kept snapshots, allowed in working roots.
        (snap / 'run').mkdir()
        self.assertFalse(datafix_seeds.snapshot_complete(snap, self.expected, strict_top=True))
        self.assertTrue(datafix_seeds.snapshot_complete(snap, self.expected, strict_top=False))
        (snap / 'run').rmdir()

        # Leftover .partial directory is rejected.
        (snap / 'worlds' / '1.19.partial').mkdir()
        self.assertFalse(datafix_seeds.snapshot_complete(snap, self.expected, strict_top=True))
        (snap / 'worlds' / '1.19.partial').rmdir()

        # Empty level.dat is rejected.
        level = snap / 'worlds' / '1.19' / 'level.dat'
        level.write_bytes(b'')
        self.assertFalse(datafix_seeds.snapshot_complete(snap, self.expected, strict_top=True))
        level.write_bytes(b'modern-119')

        # Truncated metadata JSON is rejected.
        meta = snap / 'worlds' / '1.19' / 'datafix-source.json'
        meta.write_text('{bad', encoding='utf-8')
        self.assertFalse(datafix_seeds.snapshot_complete(snap, self.expected, strict_top=True))
        meta.write_text(json.dumps({'version': '1.19', 'data_version': 3105, 'problems': []}), encoding='utf-8')

        # Missing world is rejected.
        (snap / 'worlds-pre-1.17' / '1.12.2-flat' / 'level.dat').unlink()
        self.assertFalse(datafix_seeds.snapshot_complete(snap, self.expected, strict_top=True))

    def test_recorded_required_case_failure_blocks_publication(self):
        keep = self.base / 'keep'
        dest = self.base / 'job' / 'datafix'
        key = '5' * 64

        for label, recorded in (
            ('last-tick', [['last tick', ['the transient states were not saved in one tick']]]),
            ('lab-actions', [['lab_actions', ['the bot lost its connection: EOF']]]),
            ('save-all-flush', [['save-all flush', ['Failed to save chunks']]]),
        ):
            with self.subTest(case=label):
                def generate_with_failure(scratch, _manifest, problems=recorded):
                    self.populate(scratch)
                    meta = scratch / 'worlds-pre-1.17' / '1.12.2' / 'datafix-source.json'
                    meta.write_text(
                        json.dumps({'version': '1.12.2', 'data_version': 1343, 'problems': problems}),
                        encoding='utf-8',
                    )

                with self.assertRaises(RuntimeError):
                    datafix_seeds.prepare(dest, keep, key=key, expected=self.expected, generate=generate_with_failure)
                self.assertEqual([p.name for p in keep.iterdir()], [datafix_seeds.LOCK_NAME])

        # Deterministic version-skew command failures and server tick-lag warnings remain tolerated,
        # matching tools/test_datafix_parity.py.
        def generate_with_tolerated_problems(scratch, _manifest):
            self.populate(scratch)
            meta = scratch / 'worlds' / '1.19' / 'datafix-source.json'
            meta.write_text(
                json.dumps({
                    'version': '1.19',
                    'data_version': 3105,
                    'problems': [
                        ['summon minecraft:piglin 1 100 1', ['Unknown entity']],
                        ['save-all flush', ["Can't keep up! Is the server overloaded?"]],
                    ],
                }),
                encoding='utf-8',
            )

        res = datafix_seeds.prepare(dest, keep, key=key, expected=self.expected,
                                    generate=generate_with_tolerated_problems)
        self.assertEqual(res['status'], 'miss')
        self.assertTrue(datafix_seeds.snapshot_complete(keep / key, self.expected, strict_top=True))

    def test_cold_publish_strips_scratch_and_warm_hit_copies_independently(self):
        keep = self.base / 'keep'
        dest1 = self.base / 'job1' / 'datafix'
        dest2 = self.base / 'job2' / 'datafix'
        key = '1' * 64
        calls = []

        def fake_generate(scratch, manifest_path):
            calls.append((scratch, manifest_path))
            self.populate(scratch)
            # Artifacts that must stay out of the kept snapshot and destination root.
            (scratch / 'servers' / '1.19').mkdir(parents=True)
            (scratch / 'servers' / '1.19' / 'server.jar').write_bytes(b'jar')
            (scratch / 'java' / 'jre-legacy-linux').mkdir(parents=True)
            (scratch / 'gen' / '1.19').mkdir(parents=True)
            (scratch / 'build').mkdir(parents=True)
            (scratch / 'run').mkdir(parents=True)

        cold = datafix_seeds.prepare(dest1, keep, key=key, expected=self.expected, generate=fake_generate)
        self.assertEqual(cold['status'], 'miss')
        self.assertEqual(len(calls), 1)

        # Keep contains only the lock file and the keyed snapshot with worlds/ and worlds-pre-1.17/.
        self.assertEqual(sorted(p.name for p in keep.iterdir()), [key, datafix_seeds.LOCK_NAME])
        self.assertEqual(sorted(p.name for p in (keep / key).iterdir()), ['worlds', 'worlds-pre-1.17'])
        self.assertTrue(datafix_seeds.snapshot_complete(keep / key, self.expected, strict_top=True))

        # Destination root received only worlds/ and worlds-pre-1.17/ (scratch cleaned up), no escaping symlinks.
        self.assertEqual(sorted(p.name for p in dest1.iterdir()), ['worlds', 'worlds-pre-1.17'])
        with mock.patch.dict(os.environ, {'SUNRISE_DATAFIX_ROOT': str(dest1)}, clear=True):
            self.assertEqual(datafix_paths.writable_root(), dest1.resolve())

        # Mutating dest1 does not affect the kept snapshot.
        (dest1 / 'worlds' / '1.19' / 'level.dat').write_bytes(b'mutated-by-job1')
        self.assertEqual((keep / key / 'worlds' / '1.19' / 'level.dat').read_bytes(), b'modern-119')

        # Second job hits the keep without re-running generation and touches mtime.
        old_time = time.time() - 7200
        os.utime(keep / key, (old_time, old_time))
        warm = datafix_seeds.prepare(dest2, keep, key=key, expected=self.expected, generate=fake_generate)
        self.assertEqual(warm['status'], 'hit')
        self.assertEqual(len(calls), 1)
        self.assertGreater((keep / key).stat().st_mtime, old_time + 3600)
        self.assertEqual((dest2 / 'worlds' / '1.19' / 'level.dat').read_bytes(), b'modern-119')

    def test_incomplete_generation_publishes_nothing(self):
        keep = self.base / 'keep'
        dest = self.base / 'job' / 'datafix'
        key = '2' * 64

        def partial_generate(scratch, _manifest_path):
            make_world(scratch, 'worlds', '1.19', '1.19', 3105)

        with self.assertRaisesRegex(RuntimeError, 'complete 3-world snapshot'):
            datafix_seeds.prepare(dest, keep, key=key, expected=self.expected, generate=partial_generate)
        self.assertFalse((keep / key).exists())
        self.assertEqual([p.name for p in keep.iterdir()], [datafix_seeds.LOCK_NAME])
        self.assertEqual(list(dest.iterdir()), [])

    def test_corrupted_kept_snapshot_is_replaced(self):
        keep = self.base / 'keep'
        dest = self.base / 'job' / 'datafix'
        key = '3' * 64
        make_world(keep / key, 'worlds', '1.19', '1.19', 3105)  # missing pre-1.17 worlds

        res = datafix_seeds.prepare(
            dest, keep, key=key, expected=self.expected,
            generate=lambda scratch, _m: self.populate(scratch),
        )
        self.assertEqual(res['status'], 'miss')
        self.assertTrue(datafix_seeds.snapshot_complete(keep / key, self.expected, strict_top=True))
        self.assertTrue(datafix_seeds.snapshot_complete(dest, self.expected, strict_top=True))

    def test_boundary_rejects_overlapping_keep_and_read_only_root(self):
        with self.assertRaisesRegex(ValueError, 'read-only'):
            datafix_seeds.keep_root(self.shared / 'seeds')
        keep = self.base / 'keep'
        with self.assertRaisesRegex(ValueError, 'separate directories'):
            datafix_seeds.prepare(keep / 'sub', keep, key='4' * 64, expected=self.expected)

    def test_pruning_drops_expired_stale_staging_and_lru_over_budget(self):
        keep = self.base / 'keep'
        keep.mkdir()
        (keep / datafix_seeds.LOCK_NAME).write_text('', encoding='utf-8')
        now = time.time()

        def entry(name, age, size):
            folder = keep / name
            folder.mkdir()
            (folder / 'payload').write_bytes(b'x' * size)
            os.utime(folder, (now - age, now - age))

        entry('expired', 15 * 86400, 1)
        entry('.killed-writer.tmp', 2 * 86400, 1)
        entry('.being-written.tmp', 60, 1)
        entry('used-long-ago', 3600, 8)
        entry('used-lately', 60, 8)

        with mock.patch.object(datafix_seeds, 'SEED_KEEP_BUDGET', 100):
            datafix_seeds.prune_seed_keep(keep)
        self.assertEqual(
            sorted(p.name for p in keep.iterdir()),
            ['.being-written.tmp', datafix_seeds.LOCK_NAME, 'used-lately', 'used-long-ago'],
        )

        with mock.patch.object(datafix_seeds, 'SEED_KEEP_BUDGET', 10):
            datafix_seeds.prune_seed_keep(keep)
        self.assertEqual(
            sorted(p.name for p in keep.iterdir()),
            ['.being-written.tmp', datafix_seeds.LOCK_NAME, 'used-lately'],
        )

    def test_run_vanilla_forces_fresh_oracle_only_with_seed_keep(self):
        checkout = self.base / 'checkout'
        checkout.mkdir()
        with_path = mock.patch.object(sys, 'path', [str(Path(__file__).resolve().parents[1] / 'tools'), *sys.path])
        with (with_path,
              mock.patch.object(datafix_paths, 'ROOT', checkout),
              mock.patch.dict(sys.modules, {'datafix_paths': datafix_paths})):
            sys.modules.pop('test_datafix_parity', None)
            parity_mod = importlib.import_module('test_datafix_parity')
            out = self.base / 'run' / '1.19-vanilla'
            out.parent.mkdir(parents=True)
            world = self.base / 'worlds' / '1.19'
            ok = mock.Mock(returncode=0)

            with (mock.patch.dict(os.environ, {}, clear=True),
                  mock.patch.object(parity_mod.subprocess, 'run', return_value=ok) as runner):
                parity_mod.run_vanilla('cp', world, out)
            self.assertIsNone(runner.call_args.kwargs['env'])

            with (mock.patch.dict(os.environ, {'SUNRISE_DATAFIX_SEED_KEEP': str(self.base / 'keep')}, clear=True),
                  mock.patch.object(parity_mod.subprocess, 'run', return_value=ok) as runner):
                parity_mod.run_vanilla('cp', world, out)
            self.assertEqual(runner.call_args.kwargs['env']['PARITY_JAVA_CACHE'], '0')

    def test_version_meta_uses_pinned_manifest_file_when_provided(self):
        checkout = self.base / 'checkout'
        checkout.mkdir()
        metadata = json.dumps({'downloads': {'server': {'url': 'https://invalid.test/jar', 'sha1': '0' * 40}}}).encode()
        manifest_file = self.base / 'manifest.json'
        manifest_file.write_text(json.dumps({
            'versions': [{'id': '1.19', 'url': 'https://invalid.test/1.19.json',
                          'sha1': hashlib.sha1(metadata).hexdigest()}],
        }), encoding='utf-8')
        fetched = []

        def fake_urlopen(url):
            fetched.append(url)
            return io.BytesIO(metadata)

        with_path = mock.patch.object(sys, 'path', [str(Path(__file__).resolve().parents[1] / 'tools'), *sys.path])
        with (with_path,
              mock.patch.object(datafix_paths, 'ROOT', checkout),
              mock.patch.dict(sys.modules, {'datafix_paths': datafix_paths})):
            sys.modules.pop('datafix_worlds', None)
            worlds = importlib.import_module('datafix_worlds')
            with (mock.patch.dict(os.environ, {'SUNRISE_DATAFIX_MANIFEST': str(manifest_file)}, clear=True),
                  mock.patch.object(worlds.urllib.request, 'urlopen', fake_urlopen)):
                got = worlds.version_meta('1.19')
        self.assertEqual(got['downloads']['server']['sha1'], '0' * 40)
        self.assertEqual(fetched, ['https://invalid.test/1.19.json'])

    def test_harness_fails_fast_when_seed_prepare_outcome_failed(self):
        checkout = self.base / 'checkout'
        checkout.mkdir()
        local_root = self.base / 'job-datafix'
        # Even if a snapshot already sits in the private root, a failed prepare outcome fails datafix at once.
        self.populate(local_root)
        env = {
            datafix_paths.VARIABLE: str(local_root),
            datafix_seeds.KEEP_VARIABLE: str(self.base / 'keep'),
            datafix_seeds.OUTCOME_VARIABLE: 'failure',
        }
        with_path = mock.patch.object(sys, 'path', [str(Path(__file__).resolve().parents[1] / 'tools'), *sys.path])
        with (with_path,
              mock.patch.dict(os.environ, env, clear=True),
              mock.patch.object(datafix_paths, 'ROOT', checkout),
              mock.patch.dict(sys.modules, {'datafix_paths': datafix_paths, 'datafix_seeds': datafix_seeds})):
            sys.modules.pop('test_datafix_parity', None)
            parity_mod = importlib.import_module('test_datafix_parity')
            with (mock.patch.object(sys, 'argv', ['test_datafix_parity.py']),
                  mock.patch.object(datafix_seeds, 'expected_worlds', return_value=self.expected),
                  mock.patch.object(datafix_seeds, 'prepare', side_effect=AssertionError('must not retry prepare')),
                  mock.patch.object(parity_mod.subprocess, 'run', side_effect=AssertionError('must not generate'))):
                # The private root holds a complete snapshot, as the harness itself would judge it.
                self.assertTrue(datafix_seeds.snapshot_complete(local_root))
                with self.assertRaises(SystemExit) as raised:
                    parity_mod.main()
        self.assertNotIn(raised.exception.code, (None, 0))


if __name__ == '__main__':
    unittest.main()
