"""The game folder in Android's Files app: the provider's manifest entry and GameFolder's rules."""

import os
import re
import shutil
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ANDROID = ROOT / "packaging" / "android"
JAVA = ANDROID / "java" / "io" / "github" / "juanluci" / "starling"
NS = "{http://schemas.android.com/apk/res/android}"
PROVIDER = "io.github.juanluci.starling.GameFolderProvider"


def provider():
    application = ET.parse(ANDROID / "AndroidManifest.xml").getroot().find("application")
    return next(p for p in application.findall("provider") if p.get(NS + "name") == PROVIDER)


class GameFolderManifest(unittest.TestCase):
    def test_only_the_system_binds_it(self):
        entry = provider()
        # DocumentsProvider.attachInfo throws without each of these.
        self.assertEqual(entry.get(NS + "exported"), "true")
        self.assertEqual(entry.get(NS + "grantUriPermissions"), "true")
        self.assertEqual(entry.get(NS + "permission"), "android.permission.MANAGE_DOCUMENTS")
        action = entry.find("intent-filter/action")
        self.assertEqual(action.get(NS + "name"), "android.content.action.DOCUMENTS_PROVIDER")

    def test_authority_follows_the_package(self):
        # Starling and Starling Preview can't install together with one authority.
        self.assertEqual(provider().get(NS + "authorities"), "@APPLICATION_ID@.documents")

    def test_the_game_ending_does_not_end_it(self):
        # StarlingActivity.onDestroy kills its process, which would cut a copy short.
        self.assertEqual(provider().get(NS + "process"), ":documents")


class PlayerFiles(unittest.TestCase):
    def test_same_list_as_the_game_moves(self):
        rust = (ROOT / "crates/sunrise-client/src/android.rs").read_text(encoding="utf-8")
        java = (JAVA / "GameFolder.java").read_text(encoding="utf-8")
        moved = re.search(r"const PLAYER_FILES: &\[&str\] = &\[(.*?)\];", rust, re.S).group(1)
        shown = re.search(r"List<String> PLAYER_FILES = Arrays\.asList\((.*?)\);", java, re.S).group(1)
        self.assertEqual(re.findall(r'"([^"]+)"', shown), re.findall(r'"([^"]+)"', moved))


@unittest.skipUnless(shutil.which("javac") and shutil.which("java"), "no JDK")
class GameFolderRules(unittest.TestCase):
    def test_rules_on_real_files(self):
        with tempfile.TemporaryDirectory() as temp:
            classes = Path(temp) / "classes"
            work = Path(temp) / "work"
            classes.mkdir()
            work.mkdir()
            # As build_apk.py compiles the app.
            subprocess.run(
                ["javac", "--release", "8", "-nowarn", "-Xlint:-options", "-d", str(classes),
                 str(JAVA / "GameFolder.java"), str(ROOT / "tests/java/GameFolderChecks.java")],
                check=True, capture_output=True, text=True,
            )
            ran = subprocess.run(
                ["java", "-cp", str(classes), "io.github.juanluci.starling.GameFolderChecks", str(work)],
                capture_output=True, text=True,
            )
            self.assertEqual(ran.returncode, 0, ran.stdout + ran.stderr)
            self.assertIn("ok", ran.stdout)
            # Windows may need a privilege to make links; everywhere else they must have run.
            if os.name != "nt":
                self.assertNotIn("symlinks skipped", ran.stdout)


if __name__ == "__main__":
    unittest.main()
