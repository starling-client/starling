"""tools/hot_paths.py: what a published record may hold, which runs a trend compares, and what a dump counts."""
import copy
import importlib.util
import json
import tempfile
import tomllib
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

# Loaded by path: tools/ on sys.path would shadow tests/ modules that share a name with a tools/ harness.
_spec = importlib.util.spec_from_file_location('hot_paths', Path(__file__).resolve().parents[1] / 'tools' / 'hot_paths.py')
hot_paths = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(hot_paths)


def run(commit, date, value, host='alpha', checksum='0' * 16, wall=100.0):
    return {'schema': 1, 'commit': commit * 40, 'date': date, 'run': '7.1', 'host': host, 'cpu': 'cpu', 'kernel': '6',
            'rustc': 'rustc 1', 'valgrind': 'valgrind-3',
            'workloads': {'server.tick.mobs_128': {'operations': 12800, 'checksum': checksum}},
            'metrics': {'server.tick.mobs_128.instructions': [value, value, value],
                        'server.tick.mobs_128.wall_ns': [wall, wall * 1.05]},
            'notes': []}


class Install(unittest.TestCase):
    def test_native_listeners_have_separate_matching_packages(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            game = root / 'game'
            game.write_bytes(b'benchmark executable')
            copy = Path(hot_paths.install(game, root, 10))
            self.assertEqual(copy.read_bytes(), game.read_bytes())
            packages = root / 'sunrise-native-mods'
            self.assertEqual({p.name for p in packages.iterdir()},
                             {'hot_paths_listener', *(f'hot_paths_fanout_{i}' for i in range(10))})
            for package in packages.iterdir():
                manifest = tomllib.loads((package / 'sunrise-mod.toml').read_text())
                cargo = tomllib.loads((package / 'Cargo.toml').read_text())['package']
                self.assertEqual(manifest['id'], package.name)
                self.assertEqual(cargo['name'], package.name)
                self.assertEqual(manifest['version'], '1.0.0')
                self.assertEqual(manifest['side'], 'both')
                self.assertIn('native', manifest)


class Measure(unittest.TestCase):
    def test_direct_and_callgrind_runs_inherit_the_main_stack_limit(self):
        def benchmark(command, *, cwd, env, **kwargs):
            self.assertIsNone(kwargs.get('preexec_fn'), command)
            self.assertFalse(any(arg.startswith('--main-stacksize') for arg in command), command)
            Path(env['SUNRISE_HOT_PATHS']).write_text('{"name":"tick","operations":1}\n')
            if command[0] == 'valgrind':
                (cwd / 'callgrind.out').write_text('desc: Trigger: Client Request: tick\ntotals: 7\n')
            return SimpleNamespace(returncode=0)

        with (tempfile.TemporaryDirectory() as directory,
              patch.object(hot_paths, 'scratch_env', side_effect=lambda _, **env: env),
              patch.object(hot_paths.subprocess, 'run', side_effect=benchmark)):
            root = Path(directory)
            game = root / 'sunrise-client'
            expected = {'tick': {'name': 'tick', 'operations': 1}}
            self.assertEqual(hot_paths.run_game(game, root, root / 'direct', 'none', ''), expected)
            self.assertEqual(hot_paths.callgrind('valgrind', game, root, root / 'counted', 'none', ''),
                             ({'tick': (7, 0)}, expected))

    def test_the_same_filter_selects_and_runs_fanout_workloads(self):
        inherited = 'mod.hook.fanout.entity_tick.10'  # prefix also selects .100 and .1000
        explicit = 'mod.hook.fanout.server_started.1000'

        def run_game(game, resources, work, mods, only):
            if not mods.startswith('fanout-'):
                return {}
            count = mods.removeprefix('fanout-')
            return {name: {'operations': 3, 'checksum': '0' * 16, 'wall_ns': 6}
                    for hook in ('entity_tick', 'server_started')
                    if (name := f'mod.hook.fanout.{hook}.{count}').startswith(only)}

        def callgrind(valgrind, game, resources, work, mods, only):
            rows = run_game(game, resources, work, mods, only)
            return {name: (9, 0) for name in rows}, rows

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            game, out = root / 'game', root / 'result.json'
            args = SimpleNamespace(game=game, out=out, resources=root, work=root, valgrind='valgrind',
                                   wall_samples=1, count_samples=1, commit='a' * 40, host='test', run='')
            for only, expected in ((None, {f'mod.hook.fanout.entity_tick.{n}' for n in (10, 100, 1000)}),
                                   (explicit, {explicit})):
                args.only = only
                with (patch.dict('os.environ', SUNRISE_HOT_PATHS_ONLY=inherited),
                      patch.object(hot_paths, 'install', return_value=game),
                      patch.object(hot_paths, 'run_game', run_game), patch.object(hot_paths, 'callgrind', callgrind),
                      patch.object(hot_paths.shutil, 'which', return_value='valgrind'),
                      patch.object(hot_paths, 'first_line', return_value='test'),
                      patch('builtins.print')):
                    hot_paths.measure(args)
                result = json.loads(out.read_text())
                self.assertEqual(set(result['workloads']), expected)
                self.assertEqual(result['metrics'], {f'{name}.{metric}': [value] for name in expected
                                                     for metric, value in (('instructions', 3), ('allocations', 0),
                                                                           ('wall_ns', 2))})


class Record(unittest.TestCase):
    def test_a_record_keeps_known_fields_only(self):
        result = run('a', '2026-11-12T10:00:00Z', 5000.0)
        result['game_dir'] = '/home/someone/world'
        result['notes'] = ['read /home/someone/assets/secret.json',
                           'instructions and allocations not measured: no valgrind on this machine']
        kept = hot_paths.clean(result)
        self.assertNotIn('game_dir', kept)
        self.assertEqual(kept['notes'], ['instructions and allocations not measured: no valgrind on this machine'])

    def test_a_path_or_an_unknown_metric_is_refused(self):
        for change in ({'host': '/box/work/runner'}, {'cpu': 'C:\\Users\\someone'}, {'commit': 'main'},
                       {'metrics': {'server.tick.mobs_128.assets': [1.0]}},
                       {'metrics': {'other.workload.instructions': [1.0]}},
                       {'workloads': {'../escape': {'operations': 1, 'checksum': '0' * 16}}}):
            result = {**run('a', '2026-11-12T10:00:00Z', 5000.0), **change}
            with self.assertRaises(ValueError, msg=change):
                hot_paths.clean(result)


class Trend(unittest.TestCase):
    NAME = 'server.tick.mobs_128.instructions'

    def test_a_step_needs_another_commit_and_more_than_the_noise(self):
        runs = [run('a', '2026-11-10T10:00:00Z', 5000.0), run('b', '2026-11-11T10:00:00Z', 5100.0)]
        points = hot_paths.series(runs, self.NAME)
        self.assertAlmostEqual(hot_paths.change(points, 1), 0.02)
        self.assertTrue(hot_paths.stepped(points))
        self.assertIsNone(hot_paths.change(points, 7))
        # The same move between two runs of one commit is noise, not a step.
        again = [runs[0], run('a', '2026-11-11T10:00:00Z', 5100.0)]
        points = hot_paths.series(again, self.NAME)
        self.assertFalse(hot_paths.stepped(points))
        self.assertAlmostEqual(hot_paths.noise(points), 100 / 5050)
        # Samples that lie 2% apart hide a 2% move.
        noisy = copy.deepcopy(runs)
        noisy[1]['metrics'][self.NAME] = [5050.0, 5100.0, 5150.0]
        self.assertFalse(hot_paths.stepped(hot_paths.series(noisy, self.NAME)))

    def test_a_changed_workload_starts_a_new_series(self):
        runs = [run('a', '2026-11-10T10:00:00Z', 5000.0), run('b', '2026-11-11T10:00:00Z', 9000.0, checksum='1' * 16)]
        points = hot_paths.series(runs, self.NAME)
        self.assertEqual(len(points), 1)
        self.assertFalse(hot_paths.stepped(points))

    def test_wall_time_is_compared_within_one_host_label_only(self):
        runs = [run('a', '2026-11-10T10:00:00Z', 5000.0, host='alpha'), run('b', '2026-11-11T10:00:00Z', 5000.0, host='beta'),
                run('c', '2026-11-12T10:00:00Z', 5000.0, host='beta')]
        self.assertEqual(len(hot_paths.series(runs, 'server.tick.mobs_128.wall_ns')), 2)
        self.assertEqual(len(hot_paths.series(runs, self.NAME)), 3)

    def test_another_cpu_or_toolchain_starts_a_new_series_without_a_step(self):
        for key in ('cpu', 'rustc', 'valgrind'):
            runs = [run('a', '2026-11-10T10:00:00Z', 5000.0), run('b', '2026-11-11T10:00:00Z', 5000.0),
                    {**run('c', '2026-11-12T10:00:00Z', 5600.0), key: 'another'}]
            points = hot_paths.series(runs, self.NAME)
            self.assertEqual(len(points), 1, key)
            self.assertFalse(hot_paths.stepped(points), key)


class Dump(unittest.TestCase):
    def test_a_dump_gives_the_region_its_instructions_and_its_allocator_calls(self):
        dump = '\n'.join([
            'desc: Trigger: Client Request: mod.attachment.read', 'events: Ir',
            'fn=(1) game::tick', '10 100',
            'cfn=(2) mi_malloc_aligned', 'calls=3 0', '10 60',
            'cfn=(3) game::helper', 'calls=9 0', '10 40',
            'fn=(2)', '20 30',
            # The allocator's own call into another entry point is not a second allocation.
            'cfn=(4) mi_malloc', 'calls=3 0', '20 30',
            'fn=(3)', '30 10',
            'cfn=(2)', 'calls=2 0', '30 30',
            'totals: 300', ''])
        self.assertEqual(hot_paths.parse_dump(dump), ('mod.attachment.read', 300, 5))


if __name__ == '__main__':
    unittest.main()
