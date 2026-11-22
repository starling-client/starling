#!/usr/bin/env python3
"""Replay the mechanical Sunrise-layer base of delta 9, before integration fixes.

Default is a dry run. Run with --apply in a disposable checkout; this intentionally
replaces mixed files with their upstream base. The subsequent delta 9 integration
commit reapplies the fork's mod-host, resource-boundary and launcher install-source
changes. This is a historical recipe, not the policy for future upstream syncs. Upstream's
cosmetics (their crate, the game's cosmetics module and docs/cosmetics.md) are left out: Sunrise
removed them, and tools/check_public_tree.py refuses their code.
"""
import argparse
from pathlib import Path
import subprocess

TARGET = "1d6e973ab867d599fcbc253242ccb2a218d99696"
WHOLE = [
    "launcher",
    "crates/sunrise-addresses",
    "crates/sunrise-telemetry",
    "crates/sunrise-telemetry-server",
    "crates/sunrise-system",
    "crates/sunrise-bench",
    "crates/sunrise-hooks",
    "crates/sunrise-game",
    "crates/sunrise-official",
    "crates/sunrise-heartbeat",
    "crates/sunrise-platform",
    "crates/sunrise-client/src/discord_presence.rs",
    "crates/sunrise-client/src/telemetry.rs",
    "crates/sunrise-client-input/src/actions.rs",
    "crates/sunrise-client-input/src/keybinds.rs",
    "crates/sunrise-client-input/src/gamepad.rs",
    "crates/sunrise-client-render/src/ban_notice.rs",
    "crates/sunrise-client-render/src/social.rs",
    "crates/sunrise-client/src/game/discord.rs",
    "docs/telemetry.md",
    "docs/layers.md",
    "tools/addresses.json",
    "tools/discord_presence_e2e.py",
    ".github/workflows/telemetry-server.yml",
]
MOVES = {
    "discord.rs": "runtime/discord.rs",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    for path in WHOLE:
        print(f"upstream base: {path}")
    for source, destination in MOVES.items():
        print(f"game area: {source} -> {destination}")
    if not args.apply:
        return
    subprocess.run(["git", "checkout", TARGET, "--", *WHOLE], check=True)
    game = Path("crates/sunrise-client/src/game")
    for source, destination in MOVES.items():
        destination = game / destination
        destination.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "mv", "-f", str(game / source), str(destination)], check=True)
    tracked = subprocess.check_output(["git", "ls-files", "-z", "--", *WHOLE])
    paths = [path.decode() for path in tracked.split(b"\0") if path]
    paths.extend(str(game / destination) for destination in MOVES.values())
    # This repository deliberately tracks restored source and scripts as non-executable.
    subprocess.run(["git", "update-index", "--chmod=-x", "--", *paths], check=True)
    subprocess.run(["git", "diff", "--check"], check=True)


if __name__ == "__main__":
    main()
