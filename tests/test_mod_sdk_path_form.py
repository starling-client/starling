"""The fixture mods name the SDK the way a mod author's project does: by the path `cargo sunrise-mod`
fetches the pinned SDK into, never by git (design §5). They are found, not listed: every folder of
`tools/ci/fn-replace-check/` with a `sunrise-mod.toml` and a `Cargo.toml`. (The book's examples and
the in-tree tool fixtures build inside this workspace, against its crates.)"""
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SDK_PACKAGE = "sunrise-mod-api"


FIXTURES = ROOT / "tools/ci/fn-replace-check"


def mod_projects() -> list[Path]:
    return sorted(folder for folder in FIXTURES.iterdir()
                  if (folder / "sunrise-mod.toml").is_file() and (folder / "Cargo.toml").is_file())


class SdkPathFormTests(unittest.TestCase):
    def test_every_mod_project_names_the_sdk_by_its_fetched_path(self):
        projects = mod_projects()
        self.assertTrue(projects)
        for project in projects:
            dependencies = tomllib.loads((project / "Cargo.toml").read_text()).get("dependencies", {})
            if SDK_PACKAGE in dependencies:
                with self.subTest(project=str(project.relative_to(ROOT))):
                    self.assertEqual(dependencies[SDK_PACKAGE], {"path": f".sunrise/sdk/api/crates/{SDK_PACKAGE}"})


if __name__ == "__main__":
    unittest.main()
