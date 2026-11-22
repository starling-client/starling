"""tools/client_packet_diff.py: the strict verdict compares every packet in order; its summary keeps the
differences every run reproduces, whatever tick real-time input landed on. tools/input_recording.py cuts a
recording at its markers."""
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import client_packet_diff as diff
from tools import input_recording as recording
from tools import packet_parity_runs as parity_runs


def wire(fields):
    return json.dumps(fields, sort_keys=True).encode().hex()


def tick(*packets, raw=None):
    """A decoded tick whose wire payloads follow its fields, unless `raw` gives other ones."""
    packets = [*packets, ('client_tick_end', {})]
    return {'packets': packets, 'frame': [], 'misplaced': [],
            'raw': raw if raw is not None else [('packets', n, wire(f)) for n, f in packets]}


def move(z, yaw=0.0):
    return ('move_player_pos_rot', {'x': 0.5, 'y': 101.0, 'z': z, 'yaw': yaw, 'pitch': 0.0, 'on_ground': True,
                                    'horizontal_collision': False})


MANIFEST = {'complete': True, 'recording': 'r', 'settings': 's', 'course': 'c', 'routes': ['walk']}


def run(name, ticks, keep_alive=(), **manifest):
    return {'run': name, 'number': int(name.rsplit('-', 1)[1]), 'routes': {'walk': diff.features(ticks)},
            'ticks': {'walk': ticks}, 'keep_alive': list(keep_alive), 'manifest': dict(MANIFEST, **manifest),
            'problems': []}


def c2s(name, data=''):
    return {'dir': 'c2s', 'state': 'play', 'name': name, 'data': data}


def mark(number):
    text = f'wire-mark {number:02d}'.encode()
    return c2s('chat_command', (bytes([len(text)]) + text).hex())


def kinds(rep):
    return sorted({(f['kind'], f.get('side')) for f in rep['findings']})


class ClientPacketDiffTests(unittest.TestCase):
    def test_the_same_packets_a_tick_later_are_no_finding(self):
        walk = [tick(), tick(('player_input', {'keys': ['forward']}), move(0.6)), tick(move(0.8)), tick(move(1.0))]
        late = [tick(), tick(), tick(('player_input', {'keys': ['forward']}), move(0.6)), tick(move(0.8)),
                tick(move(1.0))]
        rep = diff.report([run('vanilla-1', walk), run('vanilla-2', late)],
                          [run('sunrise-1', late), run('sunrise-2', walk)])
        self.assertEqual(rep['findings'], [])
        self.assertFalse(rep['strict']['bit_exact'], 'the strict verdict still lists the tick it moved by')
        self.assertEqual([d['runs'] for d in rep['strict']['differences']],
                         [['vanilla-1', 'sunrise-1'], ['vanilla-2', 'sunrise-2']])

    def test_strict_compares_every_field_unrounded_and_passes_only_identical_runs(self):
        same = [tick(move(0.6, 10.0))]
        rep = diff.report([run('vanilla-1', same)], [run('sunrise-1', same)])
        self.assertTrue(rep['strict']['bit_exact'])
        rep = diff.report([run('vanilla-1', same)], [run('sunrise-1', [tick(move(0.6, 10.0000001))])])
        self.assertFalse(rep['strict']['bit_exact'])
        self.assertEqual(rep['strict']['differences'][0]['changed'], 2)

    def test_runs_that_played_something_else_are_problems(self):
        same = [tick(move(0.6))]
        rep = diff.report([run('vanilla-1', same)], [run('sunrise-1', same, recording='other')])
        self.assertEqual([p['kind'] for p in rep['strict']['problems']], ['manifests_differ'])
        self.assertFalse(rep['strict']['bit_exact'])

    def test_a_tick_shape_only_one_client_sends_in_every_run_is_a_finding(self):
        vanilla = [tick(('swing', {'raw': '00'}), ('attack', {'entity': 'e1'}))]
        sunrise = [tick(('attack', {'entity': 'e1'}), ('swing', {'raw': '00'}))]
        rep = diff.report([run('vanilla-1', vanilla), run('vanilla-2', vanilla)],
                          [run('sunrise-1', sunrise), run('sunrise-2', sunrise)])
        self.assertIn(('shapes', 'only_sunrise'), kinds(rep))
        self.assertIn(('shapes', 'only_vanilla'), kinds(rep))

    def test_a_difference_one_run_lacks_is_noise(self):
        vanilla = [tick(move(1.0))]
        odd = [tick(move(1.0)), tick(('swing', {'raw': '00'}))]
        rep = diff.report([run('vanilla-1', vanilla), run('vanilla-2', vanilla)],
                          [run('sunrise-1', odd), run('sunrise-2', vanilla)])
        self.assertEqual(rep['findings'], [])

    def test_block_sequences_compare_as_steps_not_absolute_numbers(self):
        def digs(first):
            return [tick(('player_action', {'action': 0, 'pos': [0, 101, 2], 'direction': 2, 'sequence': first})),
                    tick(('player_action', {'action': 2, 'pos': [0, 101, 2], 'direction': 2,
                                            'sequence': first + 1}))]
        rep = diff.report([run('vanilla-1', digs(4)), run('vanilla-2', digs(9))],
                          [run('sunrise-1', digs(1)), run('sunrise-2', digs(30))])
        self.assertEqual(rep['findings'], [])

    def test_an_end_yaw_outside_the_other_clients_range_is_a_finding(self):
        rep = diff.report([run('vanilla-1', [tick(move(0, 0.0)), tick(move(0, 59.7))])] * 2,
                          [run('sunrise-1', [tick(move(0, 0.0)), tick(move(0, 119.7))])] * 2)
        ends = [f for f in rep['findings'] if f['kind'] == 'end_yaw']
        self.assertEqual(ends[0]['vanilla'], [59.7, 59.7])
        self.assertEqual(ends[0]['sunrise'], [119.7, 119.7])


class StrictTests(unittest.TestCase):
    def strict(self, a, b):
        return diff.report([run('vanilla-1', a)], [run('sunrise-1', b)])['strict']

    def test_bytes_the_decoder_hides_still_differ(self):
        decoded = ('attack', {'entity': 'e1'})
        for name, x, y in (('a trailing byte', '0a', '0a00'), ('another entity id', '0a', '0b'),
                           ('another click state id', '0a0001', '0a0002')):
            with self.subTest(name):
                a = [tick(decoded, raw=[('packets', 'attack', x), ('packets', 'client_tick_end', '')])]
                b = [tick(decoded, raw=[('packets', 'attack', y), ('packets', 'client_tick_end', '')])]
                self.assertFalse(self.strict(a, b)['bit_exact'])

    def test_shifted_repeated_missing_and_extra_packets_differ(self):
        one, two, three = ('swing', {'raw': '00'}), ('swing', {'raw': '01'}), ('swing', {'raw': '02'})
        base = [tick(one, two)]
        for name, other in (('shifted', [tick(two, one)]), ('repeated', [tick(one, one, two)]),
                            ('missing', [tick(one)]), ('extra', [tick(one, two, three)])):
            with self.subTest(name):
                self.assertFalse(self.strict(base, other)['bit_exact'])
        self.assertTrue(self.strict(base, [tick(one, two)])['bit_exact'])

    def test_server_picked_ids_are_left_to_the_keep_alive_check(self):
        a = [tick(('pong', {'raw': '01'}), raw=[('frame', 'pong', '01'), ('packets', 'client_tick_end', '')])]
        b = [tick(('pong', {'raw': '02'}), raw=[('frame', 'pong', '02'), ('packets', 'client_tick_end', '')])]
        self.assertTrue(self.strict(a, b)['bit_exact'])

    def test_runs_pair_by_number_and_a_pending_keep_alive_is_a_note(self):
        same = [tick(move(0.6))]
        rep = diff.report([run('vanilla-1', same), run('vanilla-2', same)],
                          [run('sunrise-2', same, keep_alive=[{'kind': 'keep_alive_pending', 'id': '07'}])])
        self.assertEqual([p['kind'] for p in rep['strict']['problems']], ['unpaired'])
        self.assertEqual([n['kind'] for n in rep['strict']['notes']], ['keep_alive_pending'])

    def test_a_route_that_started_from_another_fixture_is_a_problem(self):
        same = [tick(move(0.6))]
        played = [{'route': 'walk', 'mark': 1, 'end_tick': 1, 'fixture': {'mode': '0'}}]
        other = [dict(played[0], fixture={'mode': '1'})]
        rep = diff.report([run('vanilla-1', same, played=played)], [run('sunrise-1', same, played=other)])
        self.assertEqual([p['kind'] for p in rep['strict']['problems']], ['fixture_differs'])


class StrictExitTests(unittest.TestCase):
    """client_packet_diff.py --strict on run folders: exit 1 on problems alone and on differences alone."""

    def write_run(self, folder, name, swing, **manifest):
        run = folder / name
        run.mkdir()
        entries = [mark(1), c2s('swing', swing), c2s('client_tick_end')]
        (run / 'capture.jsonl').write_text(''.join(json.dumps(e) + '\n' for e in entries))
        played = [{'route': 'walk', 'mark': 1, 'end_tick': 1, 'fixture': {}}]
        (run / 'run.json').write_text(json.dumps(dict(MANIFEST, played=played, **manifest)))

    def strict_exit(self, sunrise_swing, **sunrise_manifest):
        with tempfile.TemporaryDirectory() as tmp:
            self.write_run(Path(tmp), 'vanilla-1', '00')
            self.write_run(Path(tmp), 'sunrise-1', sunrise_swing, **sunrise_manifest)
            with mock.patch.object(sys, 'argv', ['client_packet_diff.py', tmp, '--strict']), \
                    mock.patch('builtins.print'):
                try:
                    diff.main()
                except SystemExit as stop:
                    return stop.code
                return 0

    def test_strict_exits_1_on_a_problem_or_a_difference_alone(self):
        self.assertEqual(self.strict_exit('00'), 0)
        self.assertEqual(self.strict_exit('00', recording='other'), 1, 'a problem alone')
        self.assertEqual(self.strict_exit('01'), 1, 'a difference alone')


class RunFolderTests(unittest.TestCase):
    def test_a_run_folder_stays_one_plain_name_and_is_never_replaced_silently(self):
        with tempfile.TemporaryDirectory() as tmp:
            for label in ('../x', '/abs', 'a/b', ''):
                with self.subTest(label), self.assertRaises(ValueError):
                    parity_runs.run_folder(tmp, 'vanilla', 1, label or '..')
            first = parity_runs.run_folder(tmp, 'vanilla', 1, 'x')
            other = parity_runs.run_folder(tmp, 'sunrise', 1, 'x')
            self.assertNotEqual(first, other, 'two clients of one label keep their own folders')
            self.assertEqual(first.parent, Path(tmp))
            with self.assertRaises(FileExistsError):
                parity_runs.run_folder(tmp, 'vanilla', 1, 'x')
            (first / 'capture.jsonl').write_text('kept')
            parity_runs.run_folder(tmp, 'vanilla', 1, 'x', replace=True)
            self.assertFalse((first / 'capture.jsonl').exists())

    def test_folders_that_are_not_numbered_runs_are_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            for name in ('vanilla-old', 'x-vanilla-1', 'vanilla-2'):
                (Path(tmp) / name).mkdir()
            self.assertEqual([r['run'] for r in diff.runs_of(Path(tmp), 'vanilla')], ['vanilla-2'])


class RecordingCheckTests(unittest.TestCase):
    def test_input_before_the_first_marker_or_an_unknown_event_is_refused(self):
        late = recording.MARKER_SECONDS + 0.1
        for name, events in (
                ('before the marker', [{'t': 0.0, 'type': 'key', 'code': 'KEY_W', 'down': True},
                                       {'t': 1.0, 'type': 'marker', 'route': 'walk'}]),
                ('an unknown kind', [{'t': 0.0, 'type': 'marker', 'route': 'walk'},
                                     {'t': late, 'type': 'joystick'}]),
                ('a key with no X code', [{'t': 0.0, 'type': 'marker', 'route': 'walk'},
                                          {'t': late, 'type': 'key', 'code': 'KEY_NOPE', 'down': True}]),
                ('half a count', [{'t': 0.0, 'type': 'marker', 'route': 'walk'},
                                  {'t': late, 'type': 'rel', 'dx': 0.5, 'dy': 0}])):
            with self.subTest(name), self.assertRaises(ValueError):
                recording.routes(events)


class StreamTests(unittest.TestCase):
    def test_a_route_runs_from_its_own_marker_to_its_recorded_end(self):
        entries = [c2s('client_tick_end'), mark(1), c2s('client_tick_end'), c2s('client_tick_end'),
                   c2s('client_tick_end'), mark(2), c2s('client_tick_end')]
        cut, problems = diff.cut_routes(entries, [{'route': 'walk', 'mark': 1, 'end_tick': 3},
                                                  {'route': 'jump', 'mark': 2, 'end_tick': 5}])
        self.assertEqual(problems, [])
        self.assertEqual({k: len(v) for k, v in cut.items()}, {'walk': 2, 'jump': 1})

    def test_a_missing_marker_is_a_problem_not_a_shifted_route(self):
        entries = [mark(2), c2s('client_tick_end')]
        cut, problems = diff.cut_routes(entries, [{'route': 'walk', 'mark': 1, 'end_tick': 1},
                                                  {'route': 'jump', 'mark': 2, 'end_tick': 1}])
        self.assertEqual(list(cut), ['jump'])
        self.assertEqual([p['kind'] for p in problems], ['markers', 'route_missing'])

    def test_keep_alives_are_checked_in_connection_order(self):
        def ka(direction, data):
            return {'dir': direction, 'state': 'play', 'name': 'keep_alive', 'data': data}
        good = [ka('s2c', '01'), ka('c2s', '01'), ka('s2c', '02'), ka('c2s', '02')]
        self.assertEqual(diff.keep_alive_problems(good), [])
        twice = [ka('s2c', '01'), ka('c2s', '01'), ka('c2s', '01'), ka('s2c', '02'), ka('s2c', '03')]
        self.assertEqual([p['kind'] for p in diff.keep_alive_problems(twice)],
                         ['keep_alive_unasked', 'keep_alive_unanswered', 'keep_alive_pending'])


class InputRecordingTests(unittest.TestCase):
    def test_a_recording_is_cut_at_its_markers_with_times_from_each_marker(self):
        late = recording.MARKER_SECONDS + 0.5
        events = [{'t': 0.0, 'type': 'marker', 'route': 'walk'},
                  {'t': late, 'type': 'key', 'code': 'KEY_W', 'down': True},
                  {'t': 5.0, 'type': 'marker', 'route': 'jump'},
                  {'t': 5.0 + late, 'type': 'key', 'code': 'KEY_SPACE', 'down': True}]
        cut = recording.routes(events)
        self.assertEqual([name for name, _ in cut], ['walk', 'jump'])
        self.assertEqual([round(e['t'], 6) for e in cut[1][1]], [0.0, round(late, 6)])

    def test_input_while_the_marker_is_typed_or_an_unsafe_name_is_refused(self):
        with self.assertRaises(ValueError):
            recording.routes([{'t': 0.0, 'type': 'marker', 'route': 'walk'},
                              {'t': 0.5, 'type': 'key', 'code': 'KEY_W', 'down': True}])
        with self.assertRaises(ValueError):
            recording.routes([{'t': 0.0, 'type': 'marker', 'route': '../../x'}])

    def test_recording_keeps_the_marker_time_free_after_each_marker(self):
        events = recording.reserve_markers([{'t': 0.0, 'type': 'marker', 'route': 'a'},
                                            {'t': 0.1, 'type': 'key', 'code': 'KEY_W', 'down': True},
                                            {'t': 1.0, 'type': 'marker', 'route': 'b'},
                                            {'t': 1.1, 'type': 'key', 'code': 'KEY_W', 'down': False}])
        self.assertEqual([[round(e['t'], 6) for e in evs] for _, evs in recording.routes(events)],
                         [[0.0, round(recording.MARKER_SECONDS + 0.1, 6)], [0.0, round(recording.MARKER_SECONDS + 0.1, 6)]])

    def test_a_marker_types_the_chat_command_and_ends_before_the_route_starts(self):
        keys = [e['code'] for e in recording.marker_events(0.0) if e['down']]
        self.assertEqual(keys[0], 'KEY_T')
        self.assertEqual(keys[-1], 'KEY_ENTER')
        typed = ''.join({'KEY_SLASH': '/', 'KEY_MINUS': '-', 'KEY_SPACE': ' '}.get(k, k[4:].lower())
                        for k in keys[1:-1])
        self.assertEqual(typed, '/wire-mark 01')
        self.assertLess(max(e['t'] for e in recording.marker_events(0.0)), recording.MARKER_SECONDS)


if __name__ == '__main__':
    unittest.main()
