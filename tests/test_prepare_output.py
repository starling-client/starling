import json
import tempfile
import unittest
from pathlib import Path

from tools import prepare_output


class PrepareOutputTests(unittest.TestCase):
    def preparation(self, root: Path, files: dict[str, bytes]) -> Path:
        for rel, data in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        return root

    def test_what_varies_between_runs_does_not_count(self):
        manifest = {'schema': 7, 'data_version': 5023, 'render_files_sha256': {'a.json': 'x'}}
        a = {
            'sunrise-resources.json': json.dumps(manifest).encode(),
            'render-definitions.json': b'{"a": 1, "b": [1, 2]}',
            'creative-inventory.json': b'{"tabs":[{"search_metadata":[{"stack":"stone","tags":["b","a"]}]}]}',
            'block-behaviours.json': b'{"c": "Foo$$Lambda/0x0000012a"}',
            'export.log': b'run 1',
            'assets/minecraft/sounds/a.ogg': b'one',
        }
        manifest['render_files_sha256'] = {'a.json': 'y'}
        b = {
            'sunrise-resources.json': json.dumps(manifest).encode(),
            'render-definitions.json': b'{"b": [1, 2], "a": 1}',
            'creative-inventory.json': b'{"tabs":[{"search_metadata":[{"stack":"stone","tags":["a","b"]}]}]}',
            'block-behaviours.json': b'{"c": "Foo$$Lambda/0x00000ff0"}',
            'export.log': b'run 2',
            'assets/minecraft/sounds/a.ogg': b'two',
            # From the launcher's asset store (CI clones the languages in later), not the preparer's export.
            'assets/pack.mcmeta': b'{"language": {"fr_fr": {}}}',
            'assets/minecraft/lang/fr_fr.json': b'{"a": "b"}',
            'asset-index.json': b'{"objects": {}}',
            # The game's run-time caches, there once a harness ran the game on this preparation.
            'sunrise-sprites.cache': b'sprites',
            'sunrise-data.bin': b'data',
            'sunrise-data.bin.123.tmp': b'half',
            'sunrise-prepare.json': b'{"output_version": 1}',
            '.sunrise-prepare.lock': b'',
        }
        with tempfile.TemporaryDirectory() as tmp:
            first = prepare_output.digest(self.preparation(Path(tmp) / 'a', a))
            second = prepare_output.digest(self.preparation(Path(tmp) / 'b', b))
            self.assertEqual(first, second)
            # The data does, and so do the manifest's schema and the files there are.
            (Path(tmp) / 'b/render-definitions.json').write_bytes(b'{"b": [2, 1], "a": 1}')
            self.assertNotEqual(first, prepare_output.digest(Path(tmp) / 'b'))
            (Path(tmp) / 'b/render-definitions.json').write_bytes(b'{"b": [1, 2], "a": 1}')
            (Path(tmp) / 'b/sunrise-resources.json').write_text(json.dumps({**manifest, 'schema': 8}))
            self.assertNotEqual(first, prepare_output.digest(Path(tmp) / 'b'))
            (Path(tmp) / 'b/sunrise-resources.json').write_text(json.dumps(manifest))
            (Path(tmp) / 'b/new.nbt').write_bytes(b'')
            self.assertNotEqual(first, prepare_output.digest(Path(tmp) / 'b'))
            # en_us and deprecated are the JAR's, made by the preparer: they count.
            (Path(tmp) / 'b/new.nbt').unlink()
            self.assertEqual(first, prepare_output.digest(Path(tmp) / 'b'))
            (Path(tmp) / 'b/assets/minecraft/lang/en_us.json').write_bytes(b'{}')
            self.assertNotEqual(first, prepare_output.digest(Path(tmp) / 'b'))

    def test_server_profile_digest_covers_its_resource_inventory(self):
        manifest = {'profile': 'server', 'server_files': ['facts.json']}
        with tempfile.TemporaryDirectory() as tmp:
            folder = self.preparation(Path(tmp), {
                prepare_output.MANIFEST: json.dumps(manifest).encode(),
                'facts.json': b'{}',
            })
            before = prepare_output.digest(folder)
            manifest['server_files'] = []
            (folder / prepare_output.MANIFEST).write_text(json.dumps(manifest))
            self.assertNotEqual(before, prepare_output.digest(folder))

    def test_the_versions_file_ends_at_the_constant(self):
        versions = prepare_output.parse_versions('# comment\n1 aa\n2 bb\n')
        self.assertEqual(versions, [(1, 'aa'), (2, 'bb')])
        self.assertEqual(prepare_output.problems(versions, 2), [])
        self.assertTrue(prepare_output.problems(versions, 1))
        self.assertTrue(prepare_output.problems([(1, 'aa'), (3, 'bb')], 3))
        self.assertTrue(prepare_output.problems([(1, 'aa'), (2, 'aa')], 2))
        root = Path(prepare_output.__file__).resolve().parent.parent
        current = prepare_output.parse_versions((root / prepare_output.VERSIONS).read_text())
        constant = prepare_output.CONSTANT.search((root / prepare_output.LIB).read_text())
        self.assertEqual(prepare_output.problems(current, int(constant.group(1))), [])


if __name__ == '__main__':
    unittest.main()
