#!/usr/bin/env python3
"""SIGKILL a real dedicated server at the publish boundary of each save format."""
from contextlib import nullcontext
import signal

try:
    from . import save_fault_harness as h
except ImportError:
    import save_fault_harness as h

NEW_SPAWN = (2, 5, 0)
NEW_ITEM = 'gold_ingot'


def kill(server, kind):
    server.wait_for(rf'save crash fixture reached {kind}$', timeout=120)
    server.process.kill()
    assert server.process.wait(timeout=30) == -signal.SIGKILL, f'{kind}: the server was not killed'


def matrix(binary, root, reference):
    x, z = h.CHUNK
    for kind, point in [('level', 'save-level'), ('player', 'save-player'), ('region', f'save-region:{x},{z}')]:
        directory = root / kind
        port = h.prepare(directory, reference)
        world = directory / 'world'
        with h.dedicated(binary, directory, f'kill-{kind}', point=point) as server:
            server.console('save-off')
            if kind == 'level':
                server.console('setworldspawn ' + ' '.join(map(str, NEW_SPAWN)))
            elif kind == 'region':
                h.wait_for_chunk(server, h.TARGET, 'before-kill')
                server.console(f'setblock {h.TARGET} minecraft:diamond_block')
                h.check_choices(server, f'block {h.TARGET}', ['diamond_block'], 'marked-new-block')
            with h.joined(binary, server, port) if kind == 'player' else nullcontext():
                if kind == 'player':
                    server.console(f'item replace entity {h.PLAYER} hotbar.0 with minecraft:{NEW_ITEM}')
                    h.check_choices(server, f'items entity {h.PLAYER} hotbar.0', [NEW_ITEM], 'marked-new-item')
                (directory / '.sunrise-save-fault-armed').touch()
                server.console('save-all flush')
                kill(server, kind)

        # Inspect the committed file (or, if absent, its fallback) before a
        # second server could overwrite it. The real read paths are checked below.
        level = world / 'level.dat'
        saved_spawn = h.spawn(level if level.exists() else world / 'level.dat_old')
        assert saved_spawn in (h.BASE_SPAWN, NEW_SPAWN), f'{kind}: lost the saved spawn'
        if kind == 'player':
            player = h.player_file(world)
            assert h.items(player if player.exists() else player.with_suffix('.dat_old')) in (
                {f'minecraft:{h.BASE_ITEM}'}, {f'minecraft:{NEW_ITEM}'}), 'lost the saved inventory'
        with h.dedicated(binary, directory, f'reopen-{kind}') as server:
            assert h.spawn(level) == saved_spawn, f'{kind}: reopening changed the saved spawn'
            assert h.generator_settings(world) == h.generator_settings(reference), f'{kind}: generator settings changed'
            h.wait_for_chunk(server, h.TARGET, f'reopened-{kind}')
            h.check_choices(server, f'block {h.TARGET}',
                            ['gold_block', 'diamond_block'] if kind == 'region' else ['gold_block'], 'reloaded-block')
            if kind == 'region':
                h.wait_for_chunk(server, h.NEIGHBOR, 'reopened-neighbor')
                h.check_choices(server, f'block {h.NEIGHBOR}', ['emerald_block'], 'reloaded-neighbor')
            if kind == 'player':
                with h.joined(binary, server, port):
                    h.check_choices(server, f'items entity {h.PLAYER} hotbar.0', [h.BASE_ITEM, NEW_ITEM], 'reloaded-item')
        print(f'kill {kind}: world reopened with an intact save', flush=True)


if __name__ == '__main__':
    h.run(matrix)
