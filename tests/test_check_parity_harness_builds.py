"""tools/ci/check_parity_harness_builds.py: no parity harness builds the sunrise-client lib test (#4896)."""
import contextlib
import io
from pathlib import Path
import tempfile
import unittest

from tools.ci import check_parity_harness_builds as check


class HarnessBuildTests(unittest.TestCase):
    def run_check(self, harnesses):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / "tools").mkdir()
        for name, source in harnesses.items():
            (root / "tools" / name).write_text(source)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            status = check.main(root)
        return status, output.getvalue().splitlines()

    def test_each_form_that_builds_the_client_lib_test_is_reported_with_its_line(self):
        status, lines = self.run_check({
            "test_list_parity.py": 'x = 1\nrun(["cargo", "test", "--locked", "-p", "sunrise-client", "x"])\n',
            "test_tuple_parity.py": "run(('sunrise-client', '--no-run', '--lib'))\n",
            "test_helper_parity.py": "\n\nbinary = lab.client_test_binary('sunrise-client', 'x')\n",
            "test_name_parity.py": "binary = test_binary('sunrise-client')\n",
        })
        self.assertEqual(status, 1)
        self.assertEqual([line.split(";")[0] for line in lines], [
            "tools/test_helper_parity.py:3: client_test_binary() builds the sunrise-client lib test",
            "tools/test_list_parity.py:2: a Cargo command builds the sunrise-client lib test",
            "tools/test_name_parity.py:1: test_binary() builds the sunrise-client lib test",
            "tools/test_tuple_parity.py:1: a Cargo command builds the sunrise-client lib test",
        ])

    def test_other_packages_and_the_fixtures_program_are_not_reported(self):
        status, lines = self.run_check({
            "test_world_parity.py": 'run(["cargo", "test", "--locked", "-p", "sunrise-world", "--lib", "x"])\n'
                                    "binary = lab.test_binary('sunrise-world')\n",
            "test_fixture_parity.py": "run([lab.parity_fixtures(), 'v'])\n"
                                      "run([lab.parity_fixtures(default_features=False), 'u'])\n",
            "test_probe_parity.py": "run(['cargo', 'build', '-p', 'sunrise-client', '--example', 'probe'])\n",
            "helper.py": 'run(["cargo", "test", "-p", "sunrise-client", "--lib"])\n',
        })
        self.assertEqual((status, lines), (0, []))


if __name__ == "__main__":
    unittest.main()
