import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from tests.os_support import needs_runner_shell

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools/ci/keep_jdk_on_disk.sh"
DOWNLOAD = "8309b892-4600-4032-b712-207917b8b639"
EXTRACTION = "01667ad8-e6d1-44e8-a872-cd4753e8e48d"
JDK = "Java_Temurin-Hotspot_jdk/25.0.5/x64"


class KeepJdkOnDiskTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.build = Path(self.temporary.name, "build")
        self.build.mkdir()

    def tearDown(self):
        self.temporary.cleanup()

    def job(self, setup_java):
        """One job on this slot: fresh tmpfs RUNNER_TEMP and tool cache, the script around a stand-in setup-java."""
        job = Path(tempfile.mkdtemp(dir=self.temporary.name))
        temp, tools = job / "temp", job / "tools"
        temp.mkdir()
        # The job's own files before setup-java, one of them with a UUID name.
        (temp / "0b1e5c1a-7d2f-4c61-9a0e-3f6a8b2d4c10").write_text("step script")
        (temp / "plan").mkdir()
        env = {**os.environ, "RUNNER_TEMP": str(temp), "RUNNER_TOOL_CACHE": str(tools),
               "DYNA_CI_BUILD": str(self.build)}
        listing = job / "listing"
        subprocess.run(["bash", SCRIPT, "before", listing], env=env, check=True)
        setup_java(temp, tools)
        subprocess.run(["bash", SCRIPT, "after", listing], env=env, check=True)
        return temp, tools

    @needs_runner_shell
    def test_the_jdk_leaves_memory_and_a_later_job_reuses_it(self):
        def download(temp, tools):
            # setup-java finds no JDK: it downloads, extracts and installs one.
            self.assertFalse((tools / JDK).exists())
            (temp / DOWNLOAD).write_bytes(b"archive")
            (temp / EXTRACTION / "jdk-25").mkdir(parents=True)
            (tools / JDK / "bin").mkdir(parents=True)
            (tools / JDK / "bin/java").write_text("java")
            (temp / "report.json").write_text("{}")

        temp, tools = self.job(download)
        self.assertEqual(sorted(os.listdir(temp)),
                         ["0b1e5c1a-7d2f-4c61-9a0e-3f6a8b2d4c10", "plan", "report.json"])
        self.assertTrue((tools / "Java_Temurin-Hotspot_jdk").is_symlink())
        self.assertEqual((tools / JDK / "bin/java").read_text(), "java")
        self.assertEqual(os.listdir(self.build / "tool-cache"), ["Java_Temurin-Hotspot_jdk"])

        def cached(temp, tools):
            # The next job's setup-java finds the kept JDK in its tool cache and downloads nothing.
            self.assertEqual((tools / JDK / "bin/java").read_text(), "java")

        temp, tools = self.job(cached)
        self.assertEqual(sorted(os.listdir(temp)), ["0b1e5c1a-7d2f-4c61-9a0e-3f6a8b2d4c10", "plan"])
        self.assertEqual((tools / JDK / "bin/java").read_text(), "java")
        self.assertEqual(os.listdir(self.build / "tool-cache"), ["Java_Temurin-Hotspot_jdk"])


if __name__ == "__main__":
    unittest.main()
