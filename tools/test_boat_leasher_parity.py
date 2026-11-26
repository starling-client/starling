#!/usr/bin/env python3
"""Check a boat caller's live leasher through the real Game command path.

Use --raw-diagnostic to retain the independently scoped movement observations.
All other arguments, including --binary, are the command-oracle runner's.
"""
import sys

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import command_exec_corpus as corpus, test_command_exec_parity as runner
from tools.boat_leasher_corpus import boat_leasher


def main():
    raw = '--raw-diagnostic' in sys.argv
    if raw:
        sys.argv.remove('--raw-diagnostic')
    if not any(a == '--work-dir' or a.startswith('--work-dir=') for a in sys.argv[1:]):
        sys.argv.extend(['--work-dir', str(runner.lab.LOCAL / 'boat-leasher-parity')])
    operations = boat_leasher(corpus.cmd, corpus.setup, raw)
    runner.corpus_module.groups = lambda: [
        ('setup', corpus.group_setup()), ('boat live leasher', operations)]
    runner.main()


if __name__ == '__main__':
    main()
