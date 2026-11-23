"""A release needs every ABI; partial matrix output must not be published."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

ANDROID = Path(__file__).resolve().parents[1] / "packaging/android"
sys.path.insert(0, str(ANDROID))
spec = importlib.util.spec_from_file_location("android_release_assets", ANDROID / "release_assets.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
sys.path.pop(0)


class ReleaseAssets(unittest.TestCase):
    def test_partial_matrix_fails_before_shared_assets_and_complete_matrix_preserves_apks(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            visuals = root / "visuals"
            visuals.mkdir()
            (visuals / "sunrise-shaders.toml").write_bytes(b"original pack metadata")
            apks = {}
            for abi in ("arm64-v8a", "x86", "x86_64"):
                path = root / f"starling-0.2.3-{abi}.apk"
                path.write_bytes(abi.encode())
                apks[path] = abi.encode()
            with self.assertRaisesRegex(ValueError, "armeabi-v7a"):
                release.prepare(root, "v0.2.3", visuals)
            self.assertFalse((root / "StarlingVisuals.zip").exists())
            armv7 = root / "starling-0.2.3-armeabi-v7a.apk"
            armv7.write_bytes(b"armv7 APK")
            apks[armv7] = b"armv7 APK"
            release.prepare(root, "v0.2.3", visuals)
            with zipfile.ZipFile(root / "StarlingVisuals.zip") as archive:
                self.assertEqual(archive.read("sunrise-shaders.toml"), b"original pack metadata")
            for path, data in apks.items():
                self.assertEqual(path.read_bytes(), data)
