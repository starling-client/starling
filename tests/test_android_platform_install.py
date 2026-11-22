"""The phone installs what the desktop's mod host installs before the engine starts.

On the desktop, horizon-mod-host's `main_with` (crates/horizon-mod-host/src/loading.rs) installs the engine's
seal, the mods' instrumentation, the engine platform socket and the render points before `entry::start`. Android
starts the engine from horizon-android instead (android_lib.rs `install_platform`, run by `android::main` right
before its `start`). Sync 7 shipped an APK without them: it stopped at start, "engine platform socket is not
installed". A sync that adds an install to `main_with` fails here until the phone does it too.

The phone's `start` (android_lib.rs) loads the code as `main_with` does, through the host's `load_code` and
`code_runtime`: on 64-bit Android Wasmtime compiles wasm mods to the phone's code, on 32-bit it runs them on
Pulley, its interpreter.
"""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HOST = ROOT / "crates/horizon-mod-host/src/loading.rs"
ANDROID_LIB = ROOT / "crates/horizon-client/src/android_lib.rs"
ANDROID = ROOT / "crates/horizon-client/src/android.rs"
# What the host installs that `install_platform` leaves out: the wasm runtime, which the phone's `start` loads
# through `load_code`, and the installed native libraries' runtime (the phone's mods are compiled into the game).
DESKTOP_ONLY = {"horizon_mod_wasm", "Runtime::load"}


def function(text: str, name: str) -> str:
    """The body of the first `fn name`, braces included."""
    start = text.index("{", text.index(f"fn {name}("))
    depth, i = 0, start
    while True:
        depth += {"{": 1, "}": -1}.get(text[i], 0)
        if depth == 0:
            return text[start : i + 1]
        i += 1


def installs(body: str) -> list[str]:
    """Each `install_*`, `initialize()` or `install()` call, by its last two path segments."""
    calls = re.findall(r"((?:\w+::)*(?:install\w*|initialize))\s*\(", body)
    return [
        "::".join(call.split("::")[-2:])
        for call in calls
        if not any(skip in call for skip in DESKTOP_ONLY) and call.split("::")[-1] != "install_parts"
    ]


class AndroidPlatformInstall(unittest.TestCase):
    def test_android_installs_what_the_mod_host_installs_in_its_order(self):
        host = installs(function(HOST.read_text(encoding="utf-8"), "main_with").split("entry::start")[0])
        phone = installs(function(ANDROID_LIB.read_text(encoding="utf-8"), "install_platform"))
        self.assertTrue(host, "main_with installs nothing before entry::start: read loading.rs again")
        self.assertEqual(phone, host)

    def test_android_main_runs_the_installs_before_the_engine_starts(self):
        main = function(ANDROID.read_text(encoding="utf-8"), "main")
        self.assertIn("platform()", main)
        self.assertLess(main.index("platform()"), main.index("start(crate::entry::prepare())"))
        self.assertIn("android::main(app, install_platform, start)", ANDROID_LIB.read_text(encoding="utf-8"))

    def test_android_starts_with_the_code_the_mod_host_loads(self):
        lib = ANDROID_LIB.read_text(encoding="utf-8")
        start = function(lib, "start")
        self.assertIn("entry::start(", start)
        self.assertIn("horizon_mod_host::load_code(", start)
        self.assertIn("horizon_mod_host::code_runtime()", start)
        main_with = function(HOST.read_text(encoding="utf-8"), "main_with")
        self.assertIn("load_code(", main_with)
        self.assertIn("code_runtime()", main_with)


if __name__ == "__main__":
    unittest.main()
