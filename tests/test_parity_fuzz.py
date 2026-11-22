import json
import random
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools import parity_fuzz as fuzz
from tools import parity_fuzz_scenarios as families


def record(text):
    return {'cmd': '/data get entity @s', 'messages': [[[text, None]]], 'results': [[True, 1]]}


class SnbtDiffTests(unittest.TestCase):
    def test_a_difference_names_its_field_and_key_order_is_not_one(self):
        vanilla = record('Pig has the following entity data: {Motion: [0.0d, 0.1d, 0.0d], Tags: ["a b"], '
                         'UUID: [I; 1, 2, 3, 4], Brain: {memories: {}}, OnGround: 1b}')
        sunrise = record('Pig has the following entity data: {OnGround: 0b, Brain: {memories: {}}, '
                         'UUID: [I; 1, 2, 3, 4], Tags: ["a b"], Motion: [0.0d, 0.1d, 0.0d]}')
        self.assertEqual(fuzz.compare(vanilla, vanilla), [])
        self.assertEqual(fuzz.compare(vanilla, sunrise), [('OnGround', '1b', '0b')])

    def test_list_lengths_names_and_missing_data_differ(self):
        self.assertEqual(fuzz.diff({'Items': ['1', '2']}, {'Items': ['1']}), [('Items[#]', 2, 1)])
        self.assertEqual(fuzz.diff({'a': {'b': '1s'}}, {'a': {}}), [('a.b', '1s', None)])
        renamed = record('Arrow has the following entity data: {}')
        self.assertEqual(fuzz.compare(record('Spectral Arrow has the following entity data: {}'), renamed),
                         [('<name>', 'Spectral Arrow', 'Arrow')])
        missing = {'cmd': '/data get entity @s', 'messages': [[['No entity was found', None]]], 'results': [[False, 0]]}
        self.assertEqual(fuzz.compare(renamed, missing)[0][0], '<result>')
        self.assertEqual(fuzz.normalise('Items[3].components[0].id'), 'Items[].components[].id')

    def test_attributes_compare_in_any_order_but_by_value(self):
        a = {'attributes': [{'id': 'b', 'base': '1.0d'}, {'id': 'a', 'base': '2.0d'}]}
        self.assertEqual(fuzz.diff(a, {'attributes': a['attributes'][::-1]}), [])
        changed = {'attributes': [{'id': 'a', 'base': '2.0d'}, {'id': 'b', 'base': '3.0d'}]}
        self.assertEqual(fuzz.diff(a, changed), [('attributes[1].base', '1.0d', '3.0d')])


    def test_area_item_read_ignores_order_but_not_duplicate_stacks(self):
        stone = record('Stone has the following entity data: {Item:{id:"minecraft:stone",count:1}}')
        dirt = record('Dirt has the following entity data: {Item:{id:"minecraft:dirt",count:1}}')
        both = {**stone, 'messages': stone['messages'] + dirt['messages']}
        reversed_order = {**both, 'messages': both['messages'][::-1]}
        duplicate = {**both, 'messages': both['messages'] + stone['messages']}
        self.assertEqual(fuzz.compare(both, reversed_order, items=True), [])
        self.assertEqual(fuzz.compare(both, duplicate, items=True), [('items[#]', 2, 3)])


class RunnerTests(unittest.TestCase):
    def test_only_the_read_that_changes_between_vanilla_passes_is_excluded(self):
        scenario = {'family': 'persist', 'seed': 1, 'steps': [
            {'k': 'read_block', 'at': [1, 0, 1]}, {'k': 'read_block', 'at': [2, 0, 1]}]}
        commands = [op['cmd'] for op in fuzz.render(scenario, 0) if op.get('read')]

        def fake_harness(args, **kwargs):
            work = Path(args[args.index('--work-dir') + 1])
            work.mkdir()
            reference = '--reference-only' in args
            amounts = {'vanilla': [3, 4] if reference else [2, 4], 'rust': [5, 5]}
            for side in ('vanilla',) if reference else ('vanilla', 'rust'):
                rows = [{**record(f'Chest has the following block data: {{Items:[{{count:{n}}}]}}'), 'cmd': cmd}
                        for cmd, n in zip(commands, amounts[side], strict=True)]
                (work / f'{side}.jsonl').write_text(''.join(json.dumps(row) + '\n' for row in rows))
            return subprocess.CompletedProcess(args, 0, '', '')

        with tempfile.TemporaryDirectory() as temp, patch.object(fuzz.subprocess, 'run', side_effect=fake_harness):
            result, = fuzz.Runner(Path(temp), None, 40).run([scenario])
        self.assertEqual(result['vanilla-unstable'], commands[:1])
        self.assertEqual([cmd for _, cmd, _ in result['failures']], commands[1:])


    def test_demoted_but_not_released_chunk_invalidates_the_reload(self):
        scenario = {'family': 'persist', 'seed': 1, 'steps': [{'k': 'reload'}]}
        command, = [op['cmd'] for op in fuzz.render(scenario, 0) if 'released_chunk' in op]

        def fake_harness(args, **kwargs):
            work = Path(args[args.index('--work-dir') + 1])
            work.mkdir()
            reference = '--reference-only' in args
            row = {**record('Test passed'), 'cmd': command, 'results': [[True, 1]]}
            for side in ('vanilla',) if reference else ('vanilla', 'rust'):
                capture = {**row, 'unload_complete': not reference} if side == 'vanilla' else row
                (work / f'{side}.jsonl').write_text(json.dumps(capture) + '\n')
            return subprocess.CompletedProcess(args, 0, '', '')

        with tempfile.TemporaryDirectory() as temp, patch.object(fuzz.subprocess, 'run', side_effect=fake_harness):
            result, = fuzz.Runner(Path(temp), None, 40).run([scenario])
        self.assertEqual(result['invalid'], 'vanilla did not finish unloading the chunk')
        self.assertEqual(fuzz.signatures(result), [])


class ShrinkTests(unittest.TestCase):
    scenario = {'family': 'persist', 'seed': 1, 'steps': [
        {'k': 'summon', 'type': 'pig', 'at': [7.5, 0, 7.5], 'n': 0, 'nbt': {'NoAI': '1b', 'Fire': '20s'}},
        {'k': 'tick', 'n': 4}, {'k': 'read_entity', 'n': 0}]}

    def test_edits_drop_steps_and_keys_and_halve_ticks(self):
        edits = list(fuzz.edits(self.scenario))
        self.assertIn(('nbt', 0, 'Fire'), edits)
        self.assertIn(('half', 1), edits)
        reduced = fuzz.reduce(self.scenario, {('nbt', 0, 'Fire'), ('half', 1)})
        self.assertEqual(reduced['steps'][0]['nbt'], {'NoAI': '1b'})
        self.assertEqual(reduced['steps'][1]['n'], 2)
        self.assertEqual(len(fuzz.reduce(self.scenario, {('drop', 1)})['steps']), 2)
        self.assertEqual(self.scenario['steps'][0]['nbt'], {'NoAI': '1b', 'Fire': '20s'})

    def test_each_read_divergence_counts_not_only_the_first(self):
        result = {'invalid': None, 'failures': [('persist|pig|live|Air', '/a', []), ('persist|pig|live|Air', '/b', []),
                                                ('persist|pig|reloaded|Fire', '/c', [])]}
        self.assertEqual(fuzz.signatures(result), ['persist|pig|live|Air', 'persist|pig|reloaded|Fire'])
        self.assertEqual(fuzz.signatures({**result, 'invalid': 'kept loaded'}), [])
        self.assertEqual(fuzz.signatures({'crash': 'log'}), ['crash'])

    def test_rendered_scenarios_sync_randoms_and_give_their_entity_a_fixed_uuid(self):
        ops = fuzz.render(self.scenario, 2)
        self.assertEqual(ops[3:5], [{'op': 'align_game_time', 'target': fuzz.START_TIME},
                                    {'op': 'sync_randoms', 'uniquifier': 1, 'entity_id': 10020000,
                                     'level_seed': 1}])
        summon = next(op['cmd'] for op in ops if op['op'] == 'cmd' and op['cmd'].startswith('/summon'))
        self.assertIn('UUID:[I;1179998810,2,0,1]', summon)
        read = next(op for op in ops if op.get('read'))
        self.assertEqual(read['cmd'], '/data get entity 46555a5a-0000-0002-0000-000000000001')


class FamilyTests(unittest.TestCase):
    def test_discover_does_not_cache_a_crash_and_gives_items_a_stack(self):
        class CrashedRunner:
            def run(self, scenarios, *, repeat_vanilla):
                item, = [step for case in scenarios for step in case['steps'] if step.get('type') == 'item']
                self.item = item
                return [{'crash': 'fixture failed'}]

        registries = {'minecraft:entity_type': {'entries': ['minecraft:item']},
                      'minecraft:block_entity_type': {'entries': []}}
        with tempfile.TemporaryDirectory() as temp, patch.object(families.lab, 'RESOURCES', Path(temp)):
            work = Path(temp)
            (work / 'registries.json').write_text(json.dumps(registries))
            runner = CrashedRunner()
            with self.assertRaisesRegex(RuntimeError, 'defaults not written'):
                families.discover(runner, work)
            self.assertIn('Item', runner.item['nbt'])
            self.assertFalse((work / 'defaults.json').exists())

    def test_a_seed_gives_one_runnable_scenario(self):
        families.KNOWN.update({'entity': {'pig': {'PersistenceRequired': '0b', 'Air': '300s', 'Motion': ['0.0d'] * 3},
                                          'arrow': {'life': '0s', 'crit': '0b'}},
                               'block': {'chest': {'Items': []}, 'furnace': {'cooking_time_spent': '0s'}}})
        for name, family in families.FAMILIES.items():
            for seed in range(200):
                scenario = family(random.Random(seed), seed)
                self.assertEqual(scenario, family(random.Random(seed), seed))
                self.assertTrue(fuzz.runnable(scenario), (name, seed))
                self.assertTrue(any(op.get('read') for op in fuzz.render(scenario, 1)), (name, seed))


if __name__ == '__main__':
    unittest.main()
