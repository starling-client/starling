"""pad_menu_e2e reports a case the window can't run as skipped, apart from passes and failures."""

import contextlib
import io
import sys
import unittest
from unittest import mock

# pad_menu_e2e puts tools/ first on sys.path, where same-named parity scripts would shadow tests/ at discovery.
_path = list(sys.path)
try:
    from tools import pad_menu_e2e
finally:
    sys.path[:] = _path


def passes(args, name):
    return []


def fails(args, name):
    return ['the focus went nowhere']


def skips(args, name):
    raise pad_menu_e2e.Skipped('every page fits at [468.0, 509.0] GUI px: the list never scrolls in this window')


CASES = {'passes': passes, 'fails': fails, 'skips': skips}


def run(names):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        status = pad_menu_e2e.run_cases(CASES, names, 1, None)
    return status, out.getvalue().splitlines()


class SkippedCases(unittest.TestCase):
    def test_a_skip_beside_a_pass_is_counted_apart_and_the_run_passes(self):
        status, lines = run(['passes', 'skips'])
        self.assertEqual(status, 0)
        self.assertIn('ok   passes', lines)
        self.assertIn('skip skips: every page fits at [468.0, 509.0] GUI px: the list never scrolls in this window',
                      lines)
        self.assertEqual(lines[-1], '1 ok, 0 failed, 1 skipped (skips)')

    def test_a_failure_beside_a_skip_fails_the_run(self):
        status, lines = run(['skips', 'fails'])
        self.assertEqual(status, 1)
        self.assertIn('FAIL fails: the focus went nowhere', lines)
        self.assertEqual(lines[-1], '0 ok, 1 failed, 1 skipped (skips)')

    def test_a_run_where_every_case_skipped_does_not_pass(self):
        status, lines = run(['skips'])
        self.assertEqual(status, 2)
        self.assertEqual(lines[-2:], ['0 ok, 0 failed, 1 skipped (skips)', 'nothing ran'])



class EveryPageFits:
    """A game whose Sunrise Settings shows every page's tab (a tall window): records what the case asks of it."""
    games = []

    def __init__(self, args, name):
        self.calls = []
        self.quit_called = False
        EveryPageFits.games.append(self)

    def ctl(self, *words, timeout=60):
        self.calls.append(words)
        return {'gui_size': [468.0, 509.0]} if words == ('ui',) else {'ok': True}

    def press_through(self, *presses, click=None):
        self.calls.append(('press_through',) + presses)

    def wait_targets(self, timeout=120):
        pass

    def widgets(self):
        return [{'id': page.lower().replace(' ', '-'), 'label': page, 'kind': 'tab'}
                for page in pad_menu_e2e.SUNRISE_PAGES]

    def quit(self):
        self.quit_called = True


class SunriseBumpersWherePagesFit(unittest.TestCase):
    def test_the_real_case_skips_quits_its_game_and_the_run_exits_2(self):
        EveryPageFits.games = []
        out = io.StringIO()
        with mock.patch.object(pad_menu_e2e, 'Game', EveryPageFits), mock.patch.object(pad_menu_e2e, 'SETTLE', 0), \
                contextlib.redirect_stdout(out):
            status = pad_menu_e2e.run_cases({'sunrise_bumpers': pad_menu_e2e.sunrise_bumpers}, ['sunrise_bumpers'], 1,
                                            None)
        lines = out.getvalue().splitlines()
        self.assertEqual(status, 2)
        self.assertEqual(lines, ['skip sunrise_bumpers: every page fits at [468.0, 509.0] GUI px: the list never '
                                 'scrolls in this window',
                                 '0 ok, 0 failed, 1 skipped (sunrise_bumpers)', 'nothing ran'])
        [game] = EveryPageFits.games
        self.assertTrue(game.quit_called)
        # It got as far as Sunrise Settings at GUI scale 4 and pressed nothing on it.
        self.assertIn(('options-set', 'guiScale', '4'), game.calls)
        self.assertIn(('options', 'sunrise'), game.calls)
        self.assertNotIn(('pad', 'rb'), game.calls)


if __name__ == '__main__':
    unittest.main()
