"""The reset between lan_e2e checks restores its arena, or fails before anyone is teleported into a dirty one."""

import sys
import unittest

# lan_e2e puts tools/ first on sys.path, where same-named parity scripts would shadow tests/ at discovery.
_path = list(sys.path)
try:
    from tools import lan_e2e
finally:
    sys.path[:] = _path

JOINED = (100.5, 70.0, -20.5)
FLOOR_BLOCKS = (2 * lan_e2e.ARENA_RADIUS + 1) ** 2


class FakeHost:
    """A host whose world is the arena's floor and clearance counts; fills change them as the game's would."""

    def __init__(self, loaded=True, floor=FLOOR_BLOCKS, clutter=0, fills_work=True):
        self.joined_at = JOINED
        self.loaded = loaded
        self.floor = floor
        self.clutter = clutter
        self.fills_work = fills_work
        self.commands = []

    def __call__(self, tool, **args):
        assert tool == 'command', tool
        command = args['command']
        self.commands.append(command)
        if ' if loaded ' in command:
            return {'ok': True, 'success': self.loaded, 'result': int(self.loaded)}
        if ' run fill ' in command and not self.loaded:
            return {'ok': True, 'success': False, 'result': None}
        if ' run fill ' in command and self.fills_work:
            changed = self.floor != FLOOR_BLOCKS or self.clutter
            if command.endswith('minecraft:stone'):
                self.floor = FLOOR_BLOCKS
            else:
                self.clutter = 0
            return {'ok': True, 'success': bool(changed), 'result': None}
        return {'ok': True, 'success': False, 'result': None}

    def raw(self, *words):
        assert words[0] == 'blocks', words
        floor_box = words[2] == words[5]
        counts = {'minecraft:stone': self.floor} if floor_box and self.floor else {}
        if not floor_box and self.clutter:
            counts = {'minecraft:stone': self.clutter}
        return {'counts': counts}

    def until(self, observe, predicate, ticks=100):
        last = observe()
        for _ in range(3):
            if predicate(last):
                return last
            last = observe()
        assert predicate(last), f'host: state did not arrive within {ticks} ticks: {last}'
        return last


class RestoreArenaTests(unittest.TestCase):
    def test_a_dirty_arena_is_restored(self):
        host = FakeHost(floor=FLOOR_BLOCKS - 1, clutter=1)
        lan_e2e.restore_arena(host)
        self.assertEqual((FLOOR_BLOCKS, 0), (host.floor, host.clutter))

    def test_an_arena_that_is_already_right_passes_though_its_fills_report_failure(self):
        host = FakeHost()
        lan_e2e.restore_arena(host)
        fills = [c for c in host.commands if ' run fill ' in c]
        self.assertEqual(2, len(fills))

    def test_an_unloaded_arena_fails_before_anything_is_filled(self):
        host = FakeHost(loaded=False, floor=0, clutter=1)
        with self.assertRaisesRegex(lan_e2e.Failed, 'did not load'):
            lan_e2e.restore_arena(host)
        self.assertEqual([], [c for c in host.commands if ' run fill ' in c])

    def test_fills_that_leave_the_arena_dirty_fail_the_reset(self):
        host = FakeHost(floor=0, clutter=1, fills_work=False)
        with self.assertRaisesRegex(lan_e2e.Failed, 'not the open stone floor'):
            lan_e2e.restore_arena(host)


if __name__ == '__main__':
    unittest.main()
