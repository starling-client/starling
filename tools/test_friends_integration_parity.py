#!/usr/bin/env python3
"""Real clients over LAN, direct invitations and a local relay, with a server-thread host.

Registered with parity.py so changes to the drivers and the game binary select
this integration coverage in CI as well as the deterministic library probes.
"""
import json
from pathlib import Path
import subprocess
import sys

import lab
import test_friends
from integration_env import suite_lock


if __name__ == '__main__':
    # The official executable: joining a friend is Play with Friends' hook, which sunrise-client's
    # own executable doesn't register (docs/layers.md, "How a Sunrise crate plugs in").
    subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'sunrise-official', '-p', 'sunrise-client-lib',
                    '-p', 'sunrise-relay', '--bin', 'sunrise-official', '--bin', 'sunrise-ctl',
                    '--bin', 'sunrise-relay'], cwd=lab.ROOT, check=True)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=lab.ROOT, text=True))
    bins = Path(metadata['target_directory']) / 'fast'
    sys.argv = [sys.argv[0], '--client', str(bins / 'sunrise-official'),
                '--relay', str(bins / 'sunrise-relay'), '--baseline',
                *(['--offscreen'] if sys.platform == 'darwin' else [])]
    with suite_lock():
        # The wrapper only selects binaries; test_friends isolates each game
        # through integration_env and lab.game_env.
        test_friends.main()
