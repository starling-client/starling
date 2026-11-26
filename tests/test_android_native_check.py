"""`android_native_sdk.py mods --check`: each package's answer from cargo's JSON, the game's own errors failing all."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

TOOLS = Path(__file__).resolve().parents[1] / "tools"
spec = importlib.util.spec_from_file_location("android_native_sdk_check", TOOLS / "android_native_sdk.py")
sdk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sdk)


def error(package_id, message, code="E0308", file="src/lib.rs", line=7):
    return json.dumps({"reason": "compiler-message", "package_id": package_id, "message": {
        "level": "error", "message": message, "code": {"code": code},
        "spans": [{"is_primary": True, "file_name": file, "line_start": line}]}})


class CheckMods(unittest.TestCase):
    def answers(self, output, done=False, refused=None):
        built = [("alpha", "1.0.0", "", Path("/m/alpha.smod"), False),
                 ("beta", "0.2.0", "", Path("/m/beta.smod"), False)]
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(sdk.build_apk, "build_library", return_value=(done, output)):
            out = Path(temp) / "out"
            sdk.check_mods(out, built, [], refused or {}, "fast", Path("ndk"), Path("sdk"), "arm64-v8a", ())
            return json.loads((out / "check.json").read_text())

    def test_a_mods_error_fails_it_alone_in_either_form_of_cargos_id(self):
        answer = self.answers("\n".join([
            error("path+file:///s/native-mods-build/alpha#smod-alpha@1.0.0", "mismatched types"),
            error("smod-alpha 1.0.0 (path+file:///s/native-mods-build/alpha)", "no field `x`", "E0609"),
            '{"reason": "compiler-artifact", "package_id": "path+file:///s/native-mods-build/beta#smod-beta@0.2.0"}',
            "error: could not compile `smod-alpha` (lib) due to 2 previous errors",
        ]))
        self.assertTrue(answer["game"]["ok"])
        alpha, beta = answer["packages"]["alpha.smod"], answer["packages"]["beta.smod"]
        self.assertFalse(alpha["ok"])
        self.assertEqual(alpha["errors"], 2)
        self.assertEqual(alpha["first"][0], "E0308 mismatched types (src/lib.rs:7)")
        self.assertTrue(beta["ok"])

    def test_an_own_crates_error_is_its_mods(self):
        answer = self.answers(error("path+file:///s/native-mods-build/beta/core#beta-core@0.1.0", "unresolved"))
        self.assertFalse(answer["packages"]["beta.smod"]["ok"])
        self.assertTrue(answer["packages"]["alpha.smod"]["ok"])

    def test_an_error_in_the_games_crates_fails_every_mod(self):
        answer = self.answers(error("path+file:///s/crates/sunrise-mod-api#0.1.0", "cannot find type"))
        self.assertFalse(answer["game"]["ok"])
        self.assertFalse(any(row["ok"] for row in answer["packages"].values()))

    def test_cargo_stopping_before_the_compiler_fails_every_mod_with_its_words(self):
        answer = self.answers("\nerror: failed to load manifest for workspace member")
        self.assertFalse(answer["game"]["ok"])
        self.assertIn("failed to load manifest", answer["game"]["first"][0])

    def test_a_refused_package_is_answered(self):
        answer = self.answers("", done=True, refused={"gamma.smod": "gamma.smod: not a native mod"})
        self.assertEqual(answer["packages"]["gamma.smod"], {"ok": False, "refused": "gamma.smod: not a native mod"})
        self.assertTrue(answer["packages"]["alpha.smod"]["ok"])


if __name__ == "__main__":
    unittest.main()
