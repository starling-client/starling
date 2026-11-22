"""What the mod build service tells a player about a failed build (packaging/android/mod_build_server.py)."""
import importlib.util
import io
import json
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

SERVER = Path(__file__).resolve().parents[1] / "packaging" / "android" / "mod_build_server.py"
spec = importlib.util.spec_from_file_location("mod_build_server", SERVER)
server = importlib.util.module_from_spec(spec)
spec.loader.exec_module(server)


def why_failed(log):
    with tempfile.TemporaryDirectory() as folder:
        path = Path(folder) / "build.log"
        path.write_text(log, encoding="utf-8")
        return server.why_failed(path)


class WhyFailed(unittest.TestCase):
    def test_a_refused_injection_names_its_mod(self):
        said = why_failed(
            '{\n  "code": "E_UNSUPPORTED_CONTROL_FLOW",\n  "message": "closures are unsupported",\n'
            '  "file": "crates/horizon-client/src/game.rs",\n  "mod_id": "jade",\n'
            '  "injection_id": "tooltip"\n}\n'
        )
        self.assertEqual(
            said,
            "The mod jade does not fit this game (E_UNSUPPORTED_CONTROL_FLOW): closures are unsupported",
        )

    def test_without_a_file_it_still_names_it(self):
        said = why_failed('{"code": "E_X_Y", "message": "no", "mod_id": "treefall"}')
        self.assertTrue(said.startswith("The mod treefall does not fit"), said)

    def test_a_refusal_with_no_mod_says_so_plainly(self):
        said = why_failed('{"code": "E_X_Y", "message": "no"}')
        self.assertEqual(said, "The mod does not fit this game (E_X_Y): no")

    def test_a_compile_error_keeps_its_first_lines(self):
        said = why_failed("Compiling\nerror[E0308]: mismatched types\n")
        self.assertIn("error[E0308]: mismatched types", said)

    def test_nothing_readable_is_a_plain_failure(self):
        self.assertEqual(why_failed("killed\n"), "The build failed.")


def bundle(mod_id):
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w") as archive:
        archive.writestr("horizon-native.json", json.dumps({"id": mod_id}))
        archive.writestr(f"mixins/{mod_id}/horizon-mixin.toml", "")
    data = out.getvalue()
    return len(data).to_bytes(4, "big") + data


class AskedAgain(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        for patcher in (
            mock.patch.object(server, "ROOT", Path(folder.name)),
            mock.patch.object(server, "commit_of", return_value="commit"),
        ):
            patcher.start()
            self.addCleanup(patcher.stop)
        server.jobs.clear()
        server.line.clear()
        self.key = server.submit("0.2.3", bundle("jade"), "a")
        server.line.clear()

    def fail_with(self, error):
        server.jobs[self.key] = {"state": "failed", "error": error}
        return server.submit("0.2.3", bundle("jade"), "b")

    def test_a_refused_mod_is_not_built_again(self):
        key = self.fail_with("The mod jade does not fit this game (E_UNSUPPORTED_MACRO): no")
        self.assertEqual(key, self.key)
        self.assertEqual(server.line, [])
        self.assertEqual(server.jobs[key]["state"], "failed")

    def test_a_compile_error_is_not_built_again(self):
        self.fail_with("This mod does not compile on this version of the game. error[E0308]: mismatched")
        self.assertEqual(server.line, [])

    def test_what_may_be_the_machines_fault_builds_again(self):
        for error in ("The build failed.", "The build took too long.", "This mod does not compile on this version of the game. error: No space left on device"):
            server.line.clear()
            self.fail_with(error)
            self.assertEqual(server.line, [self.key], error)


def body(count, each=b"mod"):
    return b"".join(len(each).to_bytes(4, "big") + each for _ in range(count))


class HowManyMods(unittest.TestCase):
    """The service's own machine counts the mods; a runner that lives for one build does not."""

    def test_the_service_takes_its_own_count(self):
        self.assertEqual(len(server.split_body(body(server.MAX_MODS))), server.MAX_MODS)

    def test_over_the_count_the_service_points_at_github(self):
        with self.assertRaises(server.Refused) as refused:
            server.split_body(body(server.MAX_MODS + 1))
        self.assertIn("GitHub", str(refused.exception))
        self.assertIn(str(server.MAX_MODS), str(refused.exception))

    def test_a_runner_of_its_own_takes_as_many_as_the_body_holds(self):
        self.assertEqual(len(server.split_body(body(server.MAX_MODS * 4), most=None)), server.MAX_MODS * 4)

    def test_no_mods_is_refused_either_way(self):
        for most in (server.MAX_MODS, None):
            with self.assertRaises(server.Refused) as refused:
                server.split_body(b"", most=most)
            self.assertIn("at least one", str(refused.exception))

    def test_a_cut_request_is_still_a_cut_request(self):
        with self.assertRaises(server.Refused) as refused:
            server.split_body(b"\x00\x00\x00\x09mod", most=None)
        self.assertIn("cut short", str(refused.exception))


if __name__ == "__main__":
    unittest.main()
