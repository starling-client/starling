import contextlib
import datetime as dt
import io
import json
import os
import subprocess
import tempfile
import unittest
from dataclasses import asdict
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from tests.os_support import needs_runner_shell
from tools.ci import daily_fuzz as fuzz
from tools.ci import vendored_yaml

ROOT = Path(__file__).resolve().parents[1]
yaml = vendored_yaml.load()

COMMIT = 'ab0123456789'
URL = 'https://github.com/example/project/actions/runs/1'
SCENE = {'family': 'persist', 'seed': 1, 'steps': [{'k': 'read_block', 'at': [7, 0, 7]}]}


def signature(n):
    return f'persist|chest|reloaded|field{n}'


class PlannerTests(unittest.TestCase):
    def test_groups_dedupe_and_cap_new_issues_without_losing_comments_or_closes(self):
        red = fuzz.Issue(1, fuzz.PREFIX + signature(1), SCENE)
        green = fuzz.Issue(2, fuzz.PREFIX + signature(8), SCENE)
        changed = fuzz.Issue(3, fuzz.PREFIX + signature(9), {**SCENE, 'seed': 2})
        replayed = fuzz.Issue(4, fuzz.PREFIX + signature(10), SCENE)
        issues = {issue.title: issue for issue in (red, green, changed, replayed)}
        report = {'corpus': ['case-a', 'case-b'],
                  'clean': [asdict(red), asdict(green), {**asdict(changed), 'scenario': SCENE}],
                  'still_failing': [asdict(replayed)],
                  'groups': [{'signature': signature(n), 'scenario': SCENE, 'differences': ['private']}
                             for n in range(7)]}
        actions, deferred = fuzz.plan(report, issues, COMMIT, URL)
        created = [(row[1], row[2]) for row in actions if row[0] == 'create']
        self.assertEqual([title for title, _ in created],
                         [fuzz.CORPUS, *(fuzz.PREFIX + signature(n) for n in (0, 2, 3, 4))])
        self.assertIn('`case-a`', created[0][1])
        self.assertIn('`case-b`', created[0][1])
        self.assertEqual(deferred, [fuzz.PREFIX + signature(n) for n in (5, 6)])
        self.assertEqual([(action, number, title) for action, number, title, *_ in actions if action != 'create'],
                         [('comment', 1, red.title), ('comment', 4, replayed.title), ('close', 2, green.title)])
        body = created[1][1]
        self.assertTrue(body.startswith(fuzz.github.DISCLOSURE + '\n'))
        self.assertNotIn('private', body)
        self.assertEqual(json.loads(fuzz.SCENE.search(body)[1]), SCENE)

    def test_crash_does_not_close_issues_or_discard_a_completed_corpus_verdict(self):
        issues = {fuzz.INCOMPLETE: fuzz.Issue(1, fuzz.INCOMPLETE),
                  fuzz.PREFIX + signature(2): fuzz.Issue(2, fuzz.PREFIX + signature(2), SCENE)}
        actions, deferred = fuzz.plan(None, issues, COMMIT, URL, corpus_failures=['case-a'])
        self.assertEqual([action[0] for action in actions], ['comment', 'create'])
        self.assertEqual(actions[0][2], fuzz.INCOMPLETE)
        self.assertEqual(actions[1][1], fuzz.CORPUS)
        self.assertEqual(deferred, [])

    def test_dry_run_reads_but_does_not_file_even_when_requested_on_a_branch(self):
        for ref, dry in [('refs/heads/main', 'true'), ('refs/heads/topic', 'false')]:
            with self.subTest(ref=ref, dry=dry):
                calls = []

                def call(method, path, body=None):
                    calls.append(method)
                    return []

                with (patch.dict(os.environ, {'GITHUB_REF': ref, 'DRY_RUN': dry, 'GITHUB_REPOSITORY': 'example/project',
                                               'GITHUB_RUN_ID': '1', 'COMMIT': COMMIT}, clear=True),
                      contextlib.redirect_stdout(io.StringIO()) as out):
                    fuzz.file_issues(None, call=call)
                self.assertEqual(calls, ['GET'])
                self.assertIn('dry run: nothing written', out.getvalue())
                self.assertIn(fuzz.INCOMPLETE, out.getvalue())

    def test_only_own_issues_with_supported_commands_supply_replays(self):
        title = fuzz.PREFIX + signature(1)
        other = fuzz.PREFIX + signature(2)
        unsafe = fuzz.PREFIX + signature(3)
        body = lambda s: f'{fuzz.github.DISCLOSURE}\n\n{fuzz.MARKER}\n\n```json\n{json.dumps(s)}\n```\n'
        rows = [
            {'number': 1, 'title': title, 'user': {'login': 'github-actions[bot]'}, 'body': body(SCENE)},
            {'number': 2, 'title': other, 'user': {'login': 'contributor'}, 'body': body(SCENE)},
            {'number': 3, 'title': unsafe, 'user': {'login': 'github-actions[bot]'},
             'body': body({**SCENE, 'steps': [*SCENE['steps'], {'k': 'cmd', 'cmd': '/stop'}]})},
        ]
        self.assertEqual(fuzz.open_issues(lambda *_: rows), {
            title: fuzz.Issue(1, title, SCENE), unsafe: fuzz.Issue(3, unsafe),
        })


class BatchTests(unittest.TestCase):
    def test_runs_date_window_and_artifacts_only_input_steps(self):
        day = dt.date(2026, 11, 19)
        issued = fuzz.Issue(1, fuzz.PREFIX + signature(1), SCENE)
        group = {'signature': signature(0), 'scenario': SCENE, 'original': SCENE,
                 'differences': ['private vanilla output']}
        modes = []

        def run_mode(argv, *, check):
            mode = next(arg for arg in argv if arg in ('--smoke', '--corpus'))
            modes.append(mode)
            if mode == '--corpus':
                fuzz.lab.write_json(Path(argv[argv.index('--work-dir') + 1]) / 'corpus-result.json',
                                    {'failed': ['case-a'], 'crashed': []})
            return SimpleNamespace(returncode=int(mode == '--corpus'))

        def run(scenarios):
            return [{'invalid': None, 'failures': [], 'vanilla-unstable': []} for _ in scenarios]

        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            fuzz.lab.write_json(work / 'open.json', [asdict(issued)])
            with (patch.object(fuzz.subprocess, 'run', side_effect=run_mode),
                  patch.object(fuzz.fuzz.Runner, 'run', side_effect=run) as batches,
                  patch.object(fuzz.families, 'discover'),
                  patch.dict(fuzz.families.KNOWN, {'entity': {'pig': {}}, 'block': {'chest': {}}}, clear=True),
                  patch.object(fuzz.fuzz, 'group_report', return_value={'groups': [group]}) as grouping):
                fuzz.run_batch(work, work / 'game', day, work / 'public.json')
            self.assertEqual(modes, ['--smoke', '--corpus'])
            self.assertEqual(batches.call_args_list[0].args[0], [SCENE])
            first = day.toordinal() * 200
            explored = batches.call_args_list[1].args[0]
            for family in ('persist', 'spawn'):
                self.assertEqual([s['seed'] for s in explored if s['family'] == family], list(range(first, first + 200)))
            self.assertEqual(grouping.call_args.args[3], 6)
            self.assertEqual(grouping.call_args.kwargs, {'known': {signature(1)}, 'shrink_limit': 5})
            report = json.loads((work / 'report.json').read_text())
            self.assertEqual(report['corpus'], ['case-a'])
            self.assertEqual(report['clean'], [asdict(issued)])
            self.assertEqual(json.loads((work / 'public.json').read_text()),
                             [{'signature': signature(0), 'scenario': SCENE, 'original': SCENE}])


class DailyWorkflowTests(unittest.TestCase):
    def test_fuzz_runs_after_parity_but_is_not_on_any_release_dependency_path(self):
        daily = yaml.load((ROOT / '.github/workflows/daily.yml').read_text(), Loader=yaml.BaseLoader)['jobs']

        def needs(job):
            parents = daily[job].get('needs', [])
            return [parents] if isinstance(parents, str) else parents

        def dependencies(job):
            return {p for parent in needs(job) for p in (parent, *dependencies(parent))}

        self.assertEqual(needs('fuzz'), ['resolve', 'parity'])
        for job in ('summary', 'sdk-publish', 'nightly'):
            self.assertNotIn('fuzz', dependencies(job))
        self.assertEqual(daily['fuzz']['uses'], './.github/workflows/parity-fuzz.yml')

    @needs_runner_shell
    def test_private_directory_step_prunes_only_old_runs_and_needs_no_keep_variable(self):
        workflow = yaml.load((ROOT / '.github/workflows/parity-fuzz.yml').read_text(), Loader=yaml.BaseLoader)
        step = next(s for job in workflow['jobs'].values() for s in job['steps'] if s.get('id') == 'private')
        with tempfile.TemporaryDirectory() as temporary:
            build = Path(temporary) / 'build'
            old = build / 'keep/fuzz/1-1'
            fresh = build / 'keep/fuzz/2-1'
            old.mkdir(parents=True)
            fresh.mkdir()
            os.utime(old, (0, 0))
            env = {key: value for key, value in os.environ.items() if key != 'DYNA_CI_KEEP'}
            env.update(DYNA_CI_BUILD=str(build), GITHUB_RUN_ID='3', GITHUB_RUN_ATTEMPT='1',
                       GITHUB_ENV=str(Path(temporary) / 'env'))
            subprocess.run(['bash', '-ec', step['run']], env=env, check=True)
            self.assertFalse(old.exists())
            self.assertTrue(fresh.exists())
            self.assertTrue((build / 'keep/fuzz/3-1').is_dir())


if __name__ == '__main__':
    unittest.main()
