#!/usr/bin/env python3
"""Write crates/sunrise-client/assets/bedrock-touch.json: which of Bedrock's touch textures the
game fetches on the phone from Mojang's own samples repository (bedrock-samples, at one pinned
revision), with each file's git blob SHA-1 and size to check it by. Names, paths and hashes only:
no texture is in the project.

    python tools/bedrock_touch_manifest.py [REVISION]    # default: the repository's main now

Needs `gh` (signed in) for GitHub's API; nothing is downloaded but the file list.
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "crates" / "sunrise-client" / "assets" / "bedrock-touch.json"
REPO = "Mojang/bedrock-samples"
BASE = "resource_pack/textures/"

# Starling's sprite (starling:touch/<name>) and the Bedrock texture that is it, up and pressed.
WHOLE = {
    "attack": "ui/attack.png",
    "attack_pressed": "ui/attack_pressed.png",
    "use": "ui/interact.png",
    "use_pressed": "ui/interact_pressed.png",
    "jump": "ui/jump.png",
    "jump_pressed": "ui/jump_pressed.png",
    "sneak": "ui/sneak.png",
    "sneak_pressed": "ui/sneak_pressed.png",
    "dpad_up": "gui/controls/up.png",
    "dpad_up_pressed": "gui/controls/up_pressed.png",
    "dpad_down": "gui/controls/down.png",
    "dpad_down_pressed": "gui/controls/down_pressed.png",
    "dpad_left": "gui/controls/left.png",
    "dpad_left_pressed": "gui/controls/left_pressed.png",
    "dpad_right": "gui/controls/right.png",
    "dpad_right_pressed": "gui/controls/right_pressed.png",
    "stick": "ui/joystick_frame.png",
    "stick_knob": "ui/joystick_knob.png",
}
# The plate the other buttons are drawn on: sprint's, its icon taken off (bedrock_touch.rs).
PLATES = {"plate": "ui/sprint.png", "plate_pressed": "ui/sprint_pressed.png"}


def gh(path):
    return json.loads(subprocess.run(["gh", "api", path], capture_output=True, check=True,
                                     text=True).stdout)


def main():
    revision = sys.argv[1] if len(sys.argv) > 1 else gh(f"repos/{REPO}/commits/main")["sha"]
    files = []
    for name, source in {**WHOLE, **PLATES}.items():
        info = gh(f"repos/{REPO}/contents/{BASE}{source}?ref={revision}")
        files.append({"name": name, "source": BASE + source, "sha1": info["sha"],
                      "size": info["size"]})
        print(f"{info['size']:>6} {name:<20} {source}")
    licence = gh(f"repos/{REPO}/contents/LICENSE.md?ref={revision}")
    manifest = {
        "repository": REPO,
        "revision": revision,
        # Kept beside the textures on the phone: Mojang's terms they come with.
        "license": {"source": "LICENSE.md", "sha1": licence["sha"], "size": licence["size"]},
        "files": files,
    }
    OUT.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(OUT, revision)


if __name__ == "__main__":
    main()
