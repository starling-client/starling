#!/usr/bin/env python3
"""Compares two state digests step by step and names the first differing step per group: A/digest.txt B/digest.txt; a --coverage run's coverage.txt beside them is compared too."""
import sys
from pathlib import Path

# `tick` and `dim` are compared too: a changed game time or level routing is a behaviour change.
GROUPS = ['tick', 'dim', 'entities', 'randoms', 'block_entities', 'level_random', 'counters', 'scheduled']


def parse(lines):
    """The running ticks' fields, and how many frozen ticks moved the level random."""
    running, frozen_moves, last_random = [], 0, None
    for line in lines:
        if not line.startswith('tick='):
            continue
        fields = dict(part.split('=', 1) for part in line.split())
        if fields['run'] == '1':
            running.append(fields)
        elif last_random is not None and fields['level_random'] != last_random:
            frozen_moves += 1
        last_random = fields['level_random']
    return running, frozen_moves


def compare(a_lines, b_lines):
    """The report lines, and whether every step of every group matches and both ran the same number of steps."""
    (a, fa), (b, fb) = parse(a_lines), parse(b_lines)
    if not a or not b:
        return [f'running ticks: {len(a)} vs {len(b)}', 'DIFFERENT'], False
    steps = range(min(len(a), len(b)))
    report = [f'running ticks: {len(a)} vs {len(b)}, game time {a[0]["tick"]}..{a[-1]["tick"]} vs '
              f'{b[0]["tick"]}..{b[-1]["tick"]}',
              f'frozen ticks that moved the level random: {fa} vs {fb}']
    same = len(a) == len(b)
    for group in GROUPS:
        diff = [t for t in steps if a[t][group] != b[t][group]]
        if diff:
            same = False
            t = diff[0]
            report.append(f'{group:15} DIFFERS on {len(diff)} steps, first step {t + 1} (tick {a[t]["tick"]}): '
                          f'{a[t][group]} vs {b[t][group]}')
        else:
            report.append(f'{group:15} identical')
    report.append('IDENTICAL' if same else 'DIFFERENT')
    return report, same


def compare_coverage(a_dir, b_dir):
    """The coverage.txt lines of two --coverage runs (spots reached, saved spawner data), or None when neither has one."""
    a, b = (Path(d) / 'coverage.txt' for d in (a_dir, b_dir))
    if not a.exists() and not b.exists():
        return None
    if a.exists() != b.exists():
        return ['coverage       only in one run'], False
    same = a.read_text() == b.read_text()
    return [f'coverage       {"identical" if same else "DIFFERS"}: ' + ' | '.join(a.read_text().splitlines())], same


def main():
    with open(sys.argv[1]) as a, open(sys.argv[2]) as b:
        report, same = compare(a, b)
    coverage = compare_coverage(Path(sys.argv[1]).parent, Path(sys.argv[2]).parent)
    if coverage:
        report = report[:-1] + coverage[0] + [report[-1] if same and coverage[1] else 'DIFFERENT']
        same = same and coverage[1]
    print('\n'.join(report))
    return 0 if same else 1


if __name__ == '__main__':
    sys.exit(main())
