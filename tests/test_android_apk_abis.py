"""APK ABI entries are consumed by Android's native-library installer."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("starling_apk_abis", ROOT / "packaging/android/build_apk.py")
apk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(apk)


class ApkAbis(unittest.TestCase):
    def test_native_libraries_are_packaged_only_under_the_selected_abi(self):
        for abi in ("arm64-v8a", "armeabi-v7a", "x86", "x86_64"):
            with self.subTest(abi=abi), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                base, output = root / "base.apk", root / "output.apk"
                with zipfile.ZipFile(base, "w") as archive:
                    archive.writestr("AndroidManifest.xml", b"manifest")
                library, dex = root / "libsunrise_android.so", root / "classes.dex"
                library.write_bytes(b"native library")
                dex.write_bytes(b"dex bytecode")
                apk.add_code(base, {library.name: library}, dex, output, abi)
                with zipfile.ZipFile(output) as archive:
                    self.assertEqual(set(archive.namelist()), {
                        "AndroidManifest.xml", "classes.dex", f"lib/{abi}/libsunrise_android.so",
                    })
                    self.assertEqual(archive.read(f"lib/{abi}/libsunrise_android.so"), b"native library")
                    self.assertEqual(archive.read("AndroidManifest.xml"), b"manifest")


if __name__ == "__main__":
    unittest.main()
