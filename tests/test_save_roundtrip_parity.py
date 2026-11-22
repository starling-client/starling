import contextlib
import gzip
import io
import struct
import sys
import tempfile
import unittest
from pathlib import Path, PureWindowsPath
from unittest.mock import patch
ROOT = Path(__file__).resolve().parent.parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

# The CLI adds tools/ to sys.path; do not let it shadow unittest modules.
with patch.object(sys, 'path', sys.path.copy()):
    from tools import test_save_roundtrip_parity as roundtrip

def tag(kind, name, payload):
    raw = name.encode()
    return bytes([kind]) + struct.pack('>H', len(raw)) + raw + payload


def compound(*tags):
    return b''.join(tags) + b'\0'


def integer(name, value):
    return tag(3, name, struct.pack('>i', value))


class WindowsPath(type(Path())):
    """Keep real file access while relative paths have Windows separators."""
    def relative_to(self, *args, **kwargs):
        return PureWindowsPath(*super().relative_to(*args, **kwargs).parts)

    def __truediv__(self, key):
        return super().__truediv__(PureWindowsPath(key).as_posix())


class SaveRoundtripPathsTests(unittest.TestCase):
    def test_windows_paths_accept_expected_changes_but_retain_real_differences(self):
        with tempfile.TemporaryDirectory() as folder:
            vanilla, sunrise = WindowsPath(folder) / 'vanilla', WindowsPath(folder) / 'sunrise'
            for world, rule, health, slot, score in (
                (vanilla, 24, 20.0, 0, 1),
                (sunrise, 30, 18.0, 1, 2),
            ):
                files = {
                    'level.dat': compound(),
                    'data/minecraft/game_rules.dat': compound(
                        tag(10, 'data', compound(integer('minecraft:max_entity_cramming', rule))),
                    ),
                    'players/data/player.dat': compound(
                        tag(5, 'Health', struct.pack('>f', health)),
                        integer('SelectedItemSlot', slot),
                    ),
                    'data/minecraft/custom.dat': compound(integer('score', score)),
                }
                for rel, payload in files.items():
                    path = world / rel
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(gzip.compress(tag(10, '', payload)))
            # This fixture has no region entities or leash holders.
            with patch.object(roundtrip.scene, 'LEADS', 0):
                report = roundtrip.compare_worlds(vanilla, sunrise)
            self.assertCountEqual(report, [
                ('data/minecraft/custom.dat.score', 1, 2),
                ('players/data/player.dat.Health', ('f', 20.0), ('f', 18.0)),
                ('players/data/player.dat.SelectedItemSlot', 0, 1),
            ])
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(roundtrip.summarize(report), 2)


def terrain_sections():
    return {y: {'Y': ('b', y), 'block_states': {'palette': [{'Name': 'minecraft:air'}]},
                'biomes': {'palette': ['minecraft:desert']}} for y in range(-4, 20)}


class SaveRoundtripChunkTests(unittest.TestCase):
    key = ('r.0.0.mca', 0, 0)

    def compare(self, a, b, observations=None):
        report = []
        roundtrip.compare_chunks({self.key: {'Status': 'minecraft:full', 'sections': a}},
                                 {self.key: {'Status': 'minecraft:full', 'sections': b}}, report,
                                 lower_sky_observations=observations)
        return report

    def test_light_only_pads_compare_both_layers_and_forbid_terrain(self):
        terrain = list(terrain_sections().values())
        for y in (-5, 20):
            pad = {'Y': ('b', y), 'BlockLight': ('B', [1] * 2048), 'SkyLight': ('B', [2] * 2048)}
            for sections, paths in (
                ([pad], []),
                ([], ['BlockLight', 'SkyLight']),
                ([{**pad, 'BlockLight': ('B', [0] * 2048),
                   'SkyLight': ('B', [0] * 2048)}], ['BlockLight', 'SkyLight']),
                ([{**pad, 'block_states': {'palette': [{'Name': 'minecraft:air'}]}}], ['block_states']),
                ([{**pad, 'biomes': {'palette': ['minecraft:desert']}}], ['biomes']),
            ):
                with self.subTest(y=y, paths=paths):
                    report = self.compare(terrain + [pad], terrain + sections)
                    self.assertEqual([path for path, _, _ in report],
                                     [f'chunk.sections.{name}[{y}] @{self.key}' for name in paths])

    def test_full_in_range_section_requires_both_containers_even_when_air(self):
        rows = terrain_sections()
        rows[-4]['SkyLight'] = ('B', [2] * 2048)
        vanilla = list(rows.values())
        for missing in [('block_states',), ('biomes',), ('block_states', 'biomes')]:
            with self.subTest(missing=missing):
                invalid = {tag: value for tag, value in rows[-4].items() if tag not in missing}
                sunrise = [invalid] + list(rows.values())[1:]
                self.assertEqual(self.compare(vanilla, sunrise), [
                    (f'chunk.sections.{tag}[-4] @{self.key}', 'present', 'missing') for tag in missing])

        # A common omission is still an invalid layout, not equality.
        incomplete = list(rows.values())[1:]
        self.assertEqual(self.compare(incomplete, incomplete), [
            (f'chunk.sections.{tag}[-4] @{self.key}', 'missing', 'missing') for tag in ('block_states', 'biomes')])

    def test_only_observed_dark_lower_sky_accepts_packed_zero_in_place_of_empty(self):
        zero = ('B', [0] * 2048)
        observed = ({self.key: 'empty'}, {self.key: 'packed-zero'})
        for y in range(-5, 21):
            for layer in ('SkyLight', 'BlockLight'):
                for observations in (None, observed):
                    with self.subTest(y=y, layer=layer, observed=observations is not None):
                        a, b = terrain_sections(), terrain_sections()
                        b.setdefault(y, {'Y': ('b', y)})[layer] = zero
                        for left, right in ((a, b), (b, a)):
                            report = self.compare(list(left.values()), list(right.values()), observations)
                            expected = [] if observations and y == -5 and layer == 'SkyLight' else [
                                f'chunk.sections.{layer}[{y}] @{self.key}']
                            self.assertEqual([p for p, _, _ in report], expected)

        a = terrain_sections()
        a[-4]['SkyLight'] = ('B', [0] * 128 + [1] * 1920)
        for state, payload in (
            ('empty', ('B', [0] * 2047 + [0x10])),
            ('empty', ('B', [0] * 2047)),
            ('empty', ('I', [0] * 2048)),
            # The saved section above can be queued without visible light support.
            # Its zero bottom row does not make an absent first-packet pad dark.
            ('absent', zero),
            ('unsupported', zero),
            ('packed-nonzero', zero),
            (None, zero),
        ):
            with self.subTest(state=state, kind=payload[0], size=len(payload[1])):
                b = list(a.values()) + [{'Y': ('b', -5), 'SkyLight': payload}]
                pads = ({self.key: state} if state else {}, {self.key: 'packed-zero'})
                report = self.compare(list(a.values()), b, pads)
                self.assertEqual([p for p, _, _ in report], [f'chunk.sections.SkyLight[-5] @{self.key}'])


class SaveRoundtripModeTests(unittest.TestCase):
    def test_only_the_sunrise_resave_created_by_this_run_is_warm(self):
        def utf(value):
            data = value.encode()
            return struct.pack('>H', len(data)) + data

        def world_with_pad(world, packed):
            def container(name, kind, entry):
                return tag(10, name, compound(tag(9, 'palette', struct.pack('>Bi', kind, 1) + entry)))

            blocks = container('block_states', 10, compound(tag(8, 'Name', utf('minecraft:air'))))
            biomes = container('biomes', 8, utf('minecraft:desert'))
            rows = [compound(tag(1, 'Y', struct.pack('>b', y)), blocks, biomes) for y in range(-4, 20)]
            if packed:
                rows.insert(0, compound(tag(1, 'Y', struct.pack('>b', -5)),
                                        tag(7, 'SkyLight', struct.pack('>i', 2048) + bytes(2048))))
            nbt = tag(10, '', compound(tag(8, 'Status', utf('minecraft:full')),
                      tag(9, 'sections', struct.pack('>Bi', 10, len(rows)) + b''.join(rows))))
            chunk = struct.pack('>iB', len(nbt) + 1, 3) + nbt
            sectors = (len(chunk) + 4095) // 4096
            header = bytes((0, 0, 2, sectors)).ljust(8192, b'\0')
            region = world / 'region'
            region.mkdir(parents=True)
            (region / 'r.0.0.mca').write_bytes(header + chunk.ljust(sectors * 4096, b'\0'))
            (world / 'level.dat').write_bytes(gzip.compress(tag(10, '', compound())))
            rules = world / 'data' / 'minecraft' / 'game_rules.dat'
            rules.parent.mkdir(parents=True)
            rules.write_bytes(gzip.compress(tag(10, '', compound(tag(10, 'data', compound(
                integer('minecraft:max_entity_cramming', 30 if packed else 24)))))))

        for supplied in (False, True):
            with self.subTest(supplied=supplied), tempfile.TemporaryDirectory() as folder:
                run = Path(folder)
                v0, h = run / 'vanilla', run / 'sunrise'
                world_with_pad(v0, False)
                world_with_pad(h, True)
                key = ('r.0.0.mca', 0, 0)
                argv = ['save_roundtrip', '--vanilla', str(v0), '--keep']
                argv += ['--sunrise', str(h)] if supplied else ['--bin-dir', str(run)]
                # Only the game/process boundaries are stubs; parse both regions and compare them.
                def view(directory, observe):
                    layer = 'empty' if directory.name == 'v0view' else 'packed-zero'
                    return {}, [], {key: layer} if observe else {}

                with (patch.object(roundtrip, 'RUN', run), patch.object(sys, 'argv', argv),
                      patch.object(roundtrip.scene, 'LEADS', 0),
                      patch.object(roundtrip, 'sunrise_resave', return_value=h) as resave,
                      patch.object(roundtrip, 'lower_sky_observer'), patch.object(roundtrip, 'view_of_world', view),
                      contextlib.redirect_stdout(io.StringIO()) as log, self.assertRaises(SystemExit) as stopped):
                    roundtrip.main()
                self.assertEqual(stopped.exception.code, int(supplied), log.getvalue())
                self.assertEqual(resave.call_count, int(not supplied))


class SaveRoundtripTimersTests(unittest.TestCase):
    def test_elapsed_despawn_timers_do_not_hide_other_saved_state_changes(self):
        report = [
            ('entity.DespawnDelay @minecraft:trader_llama#57', 47384, 47375),
            ('entity.Passengers[].DespawnDelay @minecraft:trader_llama#58', 100, 91),
            ('entity.PersistenceRequired @minecraft:trader_llama#57', 0, 1),
        ]
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(roundtrip.summarize(report), 1)



class SaveRoundtripLogTests(unittest.TestCase):
    def test_ignores_offline_auth_discovery_but_retains_other_warnings(self):
        discovery = '[12:00:00] [Minecraft Services Discovery/WARN]: '
        unavailable = 'com.mojang.authlib.exceptions.MinecraftClientException: Services are unavailable'
        key_warning = 'Services are unavailable, unable to fetch uri for AUTHENTICATION/getPublicKeys'
        key_failure = 'Failed to request Minecraft Services public key'
        server = roundtrip.Vanilla.__new__(roundtrip.Vanilla)
        failures = [
            '[12:00:01] [Server thread/WARN]: ' + key_warning,
            discovery + key_warning.replace('getPublicKeys', 'otherEndpoint'),
            '[12:00:02] [Server thread/WARN]: Failed to save chunk',
            '[12:00:03] [Server thread/WARN]: World diagnostic',
            unavailable,
            key_failure + ' (unrecognized detail)',
        ]
        server.lines = [
            discovery + 'Failed to connect', unavailable,
            discovery + key_warning, unavailable,
            key_failure,
            *failures,
        ]
        self.assertEqual(server.problems(), failures)

    def test_the_containers_missing_network_interface_is_not_a_save_problem(self):
        server = object.__new__(roundtrip.Vanilla)
        server.lines = [
            '[17:01:02] [Server thread/WARN]: Failed to find a usable hardware address from the network interfaces; '
            'using random bytes: 1a:2b:3c:4d:5e:6f:70:81',
            '[17:01:03] [Server thread/WARN]: Failed to load a block entity at 1, 2, 3',
        ]
        self.assertEqual(server.problems(), server.lines[1:])


if __name__ == '__main__':
    unittest.main()
