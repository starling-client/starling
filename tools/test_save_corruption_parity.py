#!/usr/bin/env python3
"""Open corrupt NBT and Anvil files through the real dedicated-server load paths."""
try:
    from . import save_fault_harness as h
except ImportError:
    import save_fault_harness as h

SECTOR = 4096


def corrupt(path, kind):
    data = bytearray(path.read_bytes())
    if kind == 'empty':
        data.clear()
    elif kind == 'truncated':
        del data[3:]
    elif kind == 'bad-gzip-method':
        assert data[:3] == b'\x1f\x8b\x08'
        data[2] = 0xff
    else:
        x, z = h.CHUNK
        entry = 4 * ((z % 32) * 32 + x % 32)
        start = int.from_bytes(data[entry:entry + 3], 'big')
        count = data[entry + 3]
        assert start >= 2 and count > 0 and (start + count) * SECTOR <= len(data)
        if kind == 'bad-region-codec':
            assert data[start * SECTOR + 4] < 128, 'expected an inline chunk'
            data[start * SECTOR + 4] = 0x7f
        elif kind == 'zero-sector-count':
            data[entry + 3] = 0
        elif kind == 'sector-past-eof':
            data[entry:entry + 3] = (len(data) // SECTOR + 2).to_bytes(3, 'big')
        else:
            raise ValueError(kind)
    path.write_bytes(data)


def matrix(binary, root, reference):
    nbt_faults = ('empty', 'truncated', 'bad-gzip-method')
    for kind in (*nbt_faults, 'missing'):
        label = f'level-{kind}'
        directory = root / label
        h.prepare(directory, reference, seed=54321 if kind == 'missing' else None)
        world = directory / 'world'
        generation = h.generator_settings(world)
        level = world / 'level.dat'
        if kind == 'missing':
            level.unlink()
        else:
            corrupt(level, kind)
        with h.dedicated(binary, directory, label) as server:
            assert h.spawn(level) == h.OLD_SPAWN, f'{label}: did not restore the old spawn'
            assert h.generator_settings(world) == generation, f'{label}: overwrote saved generator settings'
            h.wait_for_chunk(server, h.TARGET, label)
            h.check_choices(server, f'block {h.TARGET}', ['gold_block'], 'level-preserved-block')
        print(f'{label}: restored level metadata and reopened', flush=True)

    directory = root / 'level-no-recovery'
    h.prepare(directory, reference)
    level = directory / 'world' / 'level.dat'
    backup = level.with_name('level.dat_old')
    corrupt(level, 'empty')
    corrupt(backup, 'truncated')
    before = level.read_bytes(), backup.read_bytes()
    with h.dedicated(binary, directory, directory.name, ready=False) as server:
        assert server.process.wait(timeout=120) != 0, 'corrupt level without a usable backup opened'
    assert any('its level data does not read' in line for line in server.lines), 'level refusal gave no diagnosis'
    assert (level.read_bytes(), backup.read_bytes()) == before, 'refusal overwrote the corrupt level files'
    print('level-no-recovery: refused without overwriting the files', flush=True)

    for kind in nbt_faults:
        label = f'player-{kind}'
        directory = root / label
        port = h.prepare(directory, reference)
        corrupt(h.player_file(directory / 'world'), kind)
        with h.dedicated(binary, directory, label) as server, h.joined(binary, server, port):
            h.check_choices(server, f'items entity {h.PLAYER} hotbar.0', [h.OLD_ITEM], 'player-recovered-item')
        print(f'{label}: loaded the backup inventory', flush=True)

    for kind in ('empty', 'truncated', 'bad-region-codec', 'zero-sector-count', 'sector-past-eof'):
        label = f'region-{kind}'
        directory = root / label
        h.prepare(directory, reference)
        corrupt(h.region_file(directory / 'world'), kind)
        with h.dedicated(binary, directory, label) as server:
            h.wait_for_chunk(server, h.TARGET, label)
            h.check_choices(server, f'block {h.TARGET}', ['air'], 'region-regenerated')
            if kind not in ('empty', 'truncated'):
                h.wait_for_chunk(server, h.NEIGHBOR, 'neighbor')
                h.check_choices(server, f'block {h.NEIGHBOR}', ['emerald_block'], 'region-preserved-neighbor')
            if kind == 'bad-region-codec':
                x, z = h.CHUNK
                server.wait_for(rf'Failed to load chunk {x},{z}:', timeout=60)
        print(f'{label}: regenerated the unreadable chunk', flush=True)


if __name__ == '__main__':
    h.run(matrix)
