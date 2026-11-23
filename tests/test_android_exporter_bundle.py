"""APK preparation carries original exporters, never their Minecraft classpath."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


builder = load('android_exporter_builder', ROOT / 'tools/build_android_exporters.py')
apk = load('android_exporter_apk', ROOT / 'packaging/android/build_apk.py')
CLASS = 'server/dev/sunriselab/parity/PrepareWorker.class'
BYTECODE = b'original exporter fixture'


def bundle(path, name=CLASS):
    path.mkdir(parents=True)
    output = path / name
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(BYTECODE)
    (path / 'manifest.json').write_text(json.dumps({
        'key': 'fixture', 'classes': {name: hashlib.sha256(BYTECODE).hexdigest()},
    }))


class ExporterBundle(unittest.TestCase):
    def test_fresh_build_fetches_the_pinned_manifest_package(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            client, server = b'client fixture', b'server fixture'
            manifest = json.dumps({'downloads': {
                'client': {'url': 'https://example.invalid/client', 'sha1': hashlib.sha1(client).hexdigest()},
                'server': {'url': 'https://example.invalid/server', 'sha1': hashlib.sha1(server).hexdigest()},
            }, 'libraries': []}).encode()
            checksum = hashlib.sha1(manifest).hexdigest()
            (root / 'research-lock.json').write_text(json.dumps({'minecraft': {
                'version': 'test', 'manifest_sha1': checksum,
                'client_sha1': hashlib.sha1(client).hexdigest(), 'server_sha1': hashlib.sha1(server).hexdigest()}}))
            responses = {f'https://piston-meta.mojang.com/v1/packages/{checksum}/test.json': manifest,
                         'https://example.invalid/client': client, 'https://example.invalid/server': server}
            with patch.dict(builder.os.environ, {'SUNRISE_MINECRAFT_DIR': ''}), patch.object(
                    builder.urllib.request, 'urlopen', side_effect=lambda url, **kw: io.BytesIO(responses[url])):
                home, made = builder.inputs(root)
            self.assertEqual((home / 'versions/test/test.json').read_bytes(), manifest)
            self.assertEqual((home / 'versions/test/test.jar').read_bytes(), client)
            self.assertEqual(made.read_bytes(), server)

    def test_supplied_minecraft_is_read_only_and_inputs_stay_in_build_cache(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            supplied = root / 'player-install'
            contents = {'versions/test/test.jar': b'client fixture', 'server.jar': b'server fixture',
                        'libraries/example/library.jar': b'library fixture'}
            for name, data in contents.items():
                path = supplied / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            downloads = {name: {'url': 'https://example.invalid/' + name,
                                'sha1': hashlib.sha1(contents[path]).hexdigest()}
                         for name, path in [('client', 'versions/test/test.jar'), ('server', 'server.jar')]}
            manifest = json.dumps({'downloads': downloads, 'libraries': [{'downloads': {'artifact': {
                'path': 'example/library.jar', 'url': 'https://example.invalid/library',
                'sha1': hashlib.sha1(contents['libraries/example/library.jar']).hexdigest()}}}]}).encode()
            (supplied / 'versions/test/test.json').write_bytes(manifest)
            (root / 'research-lock.json').write_text(json.dumps({'minecraft': {
                'version': 'test', 'manifest_sha1': hashlib.sha1(manifest).hexdigest(),
                'client_sha1': downloads['client']['sha1'], 'server_sha1': downloads['server']['sha1']}}))
            before = {p.relative_to(supplied): p.read_bytes() for p in supplied.rglob('*') if p.is_file()}
            with patch.dict(builder.os.environ, {'SUNRISE_MINECRAFT_DIR': str(supplied)}), patch.object(
                    builder.urllib.request, 'urlopen', side_effect=AssertionError('unexpected download')):
                home, server = builder.inputs(root)
            self.assertEqual(home, root / '.local/android-exporter-inputs')
            self.assertEqual(server.read_bytes(), b'server fixture')
            for name, data in contents.items():
                self.assertEqual((home / name).read_bytes(), data)
            self.assertEqual({p.relative_to(supplied): p.read_bytes() for p in supplied.rglob('*') if p.is_file()}, before)

    def test_copy_omits_unlisted_classpath_and_keeps_original_bytes(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source, destination = root / 'compiled', root / 'exporters'
            bundle(source)
            (source / 'minecraft.jar').write_bytes(b'classpath fixture')
            with patch.object(builder, 'ROOT', root):
                builder.copy_bundle(source, destination)
            self.assertEqual({p.relative_to(destination).as_posix() for p in destination.rglob('*') if p.is_file()},
                             {'manifest.json', CLASS})
            self.assertEqual((destination / CLASS).read_bytes(), BYTECODE)

    def test_damage_is_rejected_before_replacing_previous_bundle(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source, destination = root / 'compiled', root / 'exporters'
            bundle(source)
            bundle(destination)
            (source / CLASS).write_bytes(b'corrupt')
            with patch.object(builder, 'ROOT', root), self.assertRaisesRegex(ValueError, 'Damaged'):
                builder.copy_bundle(source, destination)
            self.assertEqual((destination / CLASS).read_bytes(), BYTECODE)

    def test_foreign_namespace_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / 'compiled'
            bundle(source, 'server/net/minecraft/Foreign.class')
            with self.assertRaisesRegex(ValueError, 'Non-Sunrise'):
                builder.copy_bundle(source, root / 'exporters')
            self.assertFalse((root / 'exporters').exists())

    def test_apk_kit_packages_only_manifest_listed_classes(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            bundle(root / 'exporters')
            (root / 'exporters/minecraft.jar').write_bytes(b'must stay out')
            foreign = root / 'exporters/server/net/minecraft/Foreign.class'
            foreign.parent.mkdir(parents=True)
            foreign.write_bytes(b'must stay out')
            for entry in ['research-lock.json', 'crates/sunrise-prepare/src/exporters.rs',
                          'mods/sunrise/src/test/java/dev/sunriselab/parity/PrepareWorker.java',
                          'crates/sunrise-theme/fonts/Geist-Regular.ttf',
                          'crates/sunrise-theme/fonts/Geist-SemiBold.ttf',
                          'packaging/lowbudget/pack/original.txt',
                          'packaging/shaderpacks/starling-visuals/original.txt']:
                path = root / entry
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'original fixture')
            out = root / 'out'
            out.mkdir()
            with patch.object(apk, 'ROOT', root), patch.object(apk, 'HERE', root / 'packaging/android'), patch.object(apk, 'run'):
                folder = apk.assets(out)
            with zipfile.ZipFile(folder / 'prepare-kit.zip') as archive:
                classes = {n for n in archive.namelist() if n.startswith('exporters/')}
                self.assertEqual(classes, {'exporters/manifest.json', 'exporters/' + CLASS})
                self.assertEqual(archive.read('exporters/' + CLASS), BYTECODE)
                self.assertFalse(any(n.endswith('.jar') for n in archive.namelist()))

    def test_bad_download_does_not_replace_existing_input(self):
        with tempfile.TemporaryDirectory() as folder:
            destination = Path(folder) / 'input.jar'
            destination.write_bytes(b'previous')
            with patch.object(builder.urllib.request, 'urlopen', return_value=io.BytesIO(b'damaged')), self.assertRaisesRegex(ValueError, 'Checksum'):
                builder.fetch('https://example.invalid/input', destination, hashlib.sha1(b'expected').hexdigest())
            self.assertEqual(destination.read_bytes(), b'previous')
            self.assertFalse(destination.with_suffix('.jar.part').exists())


if __name__ == '__main__':
    unittest.main()
