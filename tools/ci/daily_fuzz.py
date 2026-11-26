#!/usr/bin/env python3
"""Run report-only parity fuzz and file input-only reproductions."""
import argparse
import datetime as dt
import json
import os
import random
import re
import subprocess
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools import lab, parity_fuzz as fuzz, parity_fuzz_scenarios as families  # noqa: E402
from tools.parity_fuzz_corpus import validate_scenario  # noqa: E402
from tools.ci import daily_summary as github  # noqa: E402

PREFIX = 'Daily fuzz: '
INCOMPLETE = 'Daily fuzz did not complete'
CORPUS = 'Daily fuzz corpus regressions'
MARKER = '<!-- sunrise:daily-fuzz -->'
SCENE = re.compile(r'^```json\n(.*?)\n```$', re.MULTILINE | re.DOTALL)
CASES_PER_FAMILY = 200
MAX_NEW = 5
SHRINK_ROUNDS = 6


@dataclass(frozen=True)
class Issue:
    number: int
    title: str
    scenario: dict | None = None


def open_issues(call):
    """Only this workflow's own issue bodies are eligible for replay and automatic close."""
    issues = {}
    for row in github.pages(call, f'issues?state=open&labels={github.LABEL}'):
        body = row['body'] or ''
        if ('pull_request' in row or row['user']['login'] != 'github-actions[bot]'
                or not body.startswith(f'{github.DISCLOSURE}\n\n{MARKER}\n')):
            continue
        title = row['title']
        if title not in (INCOMPLETE, CORPUS) and not title.startswith(PREFIX):
            continue
        scenario = None
        if title.startswith(PREFIX) and (match := SCENE.search(body)):
            try:
                saved = validate_scenario(json.loads(match[1]))
                # The generator's only raw command breaks one block at a relative position. A
                # report body is not authority to run other commands in the fixture.
                if all(step['cmd'] == families.BREAK_BLOCK for step in saved['steps'] if step['k'] == 'cmd'):
                    scenario = saved
            except ValueError:
                # A changed or damaged reproduction is not a passing replay.
                pass
        issues[title] = Issue(row['number'], title, scenario)
    return issues


def run_batch(work, binary, day, scenarios_path):
    """Raw outputs stay in work. The only artifact is rebuilt from the input fields."""
    def run_mode(mode):
        directory = work / mode
        return subprocess.run([sys.executable, str(fuzz.ROOT / 'tools/parity_fuzz.py'), f'--{mode}',
                               '--work-dir', str(directory), '--binary', str(binary)], check=False).returncode

    if run_mode('smoke'):
        raise RuntimeError('daily fuzz smoke failed')
    corpus_exit = run_mode('corpus')
    corpus = json.loads((work / 'corpus/corpus-result.json').read_text(encoding='utf-8'))
    if corpus['crashed'] or corpus_exit != int(bool(corpus['failed'])):
        raise RuntimeError('daily fuzz corpus did not complete')

    issues = [Issue(**row) for row in json.loads((work / 'open.json').read_text(encoding='utf-8'))]
    known = {issue.title[len(PREFIX):] for issue in issues if issue.title.startswith(PREFIX)}
    replay = [issue for issue in issues if issue.scenario is not None]
    runner = fuzz.Runner(work, binary, 40, bisect_failures=False)
    replay_results = runner.run([issue.scenario for issue in replay])
    clean, still_failing = [], []
    for issue, result in zip(replay, replay_results, strict=True):
        signatures = fuzz.signatures(result)
        if issue.title[len(PREFIX):] in signatures:
            still_failing.append(asdict(issue))
        elif not (signatures or result['invalid'] or result['vanilla-unstable']):
            clean.append(asdict(issue))

    first = day.toordinal() * CASES_PER_FAMILY
    seeds = range(first, first + CASES_PER_FAMILY)
    families.discover(runner, work)
    scenarios = [build(random.Random(seed), seed) for build in families.FAMILIES.values() for seed in seeds]
    results = runner.run(scenarios)
    report = fuzz.group_report(runner, scenarios, results, SHRINK_ROUNDS, known=known, shrink_limit=MAX_NEW)
    report.update({'day': day.isoformat(), 'seed_window': [first, seeds.stop - 1],
                   'corpus': corpus['failed'], 'clean': clean, 'still_failing': still_failing})
    lab.write_json(scenarios_path, [{key: group[key] for key in ('signature', 'scenario', 'original')}
                                    for group in report['groups']])
    pending = work / 'report.pending.json'
    lab.write_json(pending, report)
    pending.replace(work / 'report.json')


def plan(report, issues, commit, run_url, *, corpus_failures=()):
    """Plan only title, steps and pass/fail writes; None means a tool failure, not a parity verdict."""
    actions, deferred, reds = [], [], set()
    new = 0
    first, still_red = github.red_marks(run_url)
    commit_text = f'on `{commit[:9]}`'

    def red(title, detail, scenario=None):
        nonlocal new
        reds.add(title)
        if title in issues:
            actions.append(('comment', issues[title].number, title,
                            f'🤖 Agent: {still_red} {commit_text}.\n\n{detail}\n'))
        elif new == MAX_NEW:
            deferred.append(title)
        else:
            new += 1
            body = f'{github.DISCLOSURE}\n\n{MARKER}\n\n{detail}\n\n{first} {commit_text}.\n'
            if scenario is not None:
                body += '\nSave the scenario as `case.json` and run `python3 tools/parity_fuzz.py --replay case.json`.\n\n'
                body += f'```json\n{json.dumps(scenario, indent=2)}\n```\n'
            actions.append(('create', title, body))

    if report is None:
        red(INCOMPLETE, 'The report-only fuzz tool did not finish. No exploratory parity verdict was recorded.')
    else:
        corpus_failures = report['corpus']
    if corpus_failures:
        red(CORPUS, 'Promoted cases that did not match:\n\n' + '\n'.join(f'- `{name}`' for name in corpus_failures))
    if report is None:
        return actions, deferred

    for group in report['groups']:
        signature = group['signature']
        red(PREFIX + signature, 'A generated parity fuzz scenario diverged.', group['scenario'])
    for row in report['still_failing']:
        issue = Issue(**row)
        if issue.title not in reds and issues.get(issue.title) == issue:
            red(issue.title, 'The saved scenario still diverges.')

    clean = {row['title']: Issue(**row) for row in report['clean']}
    for title, issue in sorted(issues.items()):
        if title in reds or (title not in (CORPUS, INCOMPLETE) and clean.get(title) != issue):
            continue
        actions.append(('close', issue.number, title,
                        f'🤖 Agent: {github.green_mark(run_url)} {commit_text}; closing.\n'))
    return actions, deferred


def file_issues(report, call=github.api, *, corpus_failures=()):
    env = os.environ
    writes = env.get('GITHUB_REF') == 'refs/heads/main' and env.get('DRY_RUN') == 'false'
    run_url = f"{env.get('GITHUB_SERVER_URL', 'https://github.com')}/{env['GITHUB_REPOSITORY']}/actions/runs/{env['GITHUB_RUN_ID']}"
    actions, deferred = plan(report, open_issues(call), env['COMMIT'], run_url, corpus_failures=corpus_failures)
    title = '## Daily fuzz (report-only)' + ('' if writes else ' (dry run: nothing written)')
    lines = [title, '', 'The tool ran to completion.' if report is not None else 'The tool did not complete.']
    if report is not None or corpus_failures:
        failed = report['corpus'] if report is not None else corpus_failures
        lines.append(f'Corpus: {"did not match" if failed else "matched"}.')
    verbs = {'create': 'File', 'comment': 'Comment on', 'close': 'Close'}
    for kind, *fields in actions:
        subject = fields[0] if kind == 'create' else f'#{fields[0]} {fields[1]}'
        lines.append(f'- {verbs[kind]} {subject}')
    lines.extend(f'- Not filed (new issue limit): {title}' for title in deferred)
    text = '\n'.join(lines) + '\n'
    print(text)
    if env.get('GITHUB_STEP_SUMMARY'):
        with Path(env['GITHUB_STEP_SUMMARY']).open('a', encoding='utf-8') as output:
            output.write(text)
    if writes:
        github.apply(actions, call, run_url)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['open', 'run', 'file'])
    parser.add_argument('--work-dir', type=Path, required=True)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--scenarios', type=Path, help='input-only artifact path')
    parser.add_argument('--day', type=dt.date.fromisoformat, default=dt.datetime.now(dt.UTC).date())
    args = parser.parse_args()
    work = args.work_dir.resolve()
    if args.mode == 'open':
        lab.write_json(work / 'open.json', [asdict(issue) for issue in open_issues(github.api).values()])
    elif args.mode == 'run':
        if args.binary is None or args.scenarios is None:
            parser.error('run requires --binary and --scenarios')
        run_batch(work, args.binary.resolve(strict=True), args.day, args.scenarios.resolve())
    else:
        report = work / 'report.json'
        if report.is_file():
            file_issues(json.loads(report.read_text(encoding='utf-8')))
        else:
            case_report = work / 'corpus/corpus-result.json'
            corpus_failures = json.loads(case_report.read_text(encoding='utf-8'))['failed'] if case_report.is_file() else []
            file_issues(None, corpus_failures=corpus_failures)


if __name__ == '__main__':
    main()
