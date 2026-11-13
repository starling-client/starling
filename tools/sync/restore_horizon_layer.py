#!/usr/bin/env python3
"""Replay the mechanical Horizon-layer base of delta 9, before integration fixes.

Default is a dry run. Run with --apply in a disposable checkout; this intentionally
replaces mixed files with their upstream base. The subsequent delta 9 integration
commit reapplies the fork's mod-host, resource-boundary and launcher install-source
changes. This is a historical recipe, not the policy for future upstream syncs.
"""
import argparse
from pathlib import Path
import subprocess

TARGET = "1d6e973ab867d599fcbc253242ccb2a218d99696"
WHOLE = [
    "launcher",
    "crates/horizon-addresses",
    "crates/horizon-cosmetics",
    "crates/horizon-telemetry",
    "crates/horizon-telemetry-server",
    "crates/horizon-system",
    "crates/horizon-bench",
    "crates/horizon-hooks",
    "crates/horizon-game",
    "crates/horizon-official",
    "crates/horizon-heartbeat",
    "crates/horizon-platform",
    "crates/horizon-client/src/discord_presence.rs",
    "crates/horizon-client/src/telemetry.rs",
    "crates/horizon-client-input/src/actions.rs",
    "crates/horizon-client-input/src/keybinds.rs",
    "crates/horizon-client-input/src/gamepad.rs",
    "crates/horizon-client-render/src/ban_notice.rs",
    "crates/horizon-client-render/src/social.rs",
    "crates/horizon-client/src/game/cosmetics.rs",
    "crates/horizon-client/src/game/cosmetics",
    "crates/horizon-client/src/game/discord.rs",
    "docs/cosmetics.md",
    "docs/telemetry.md",
    "docs/layers.md",
    "tools/addresses.json",
    "tools/discord_presence_e2e.py",
    ".github/workflows/telemetry-server.yml",
]
MOVES = {
    "cosmetics.rs": "interface/cosmetics/mod.rs",
    "cosmetics/emote_wheel.rs": "interface/cosmetics/emote_wheel.rs",
    "cosmetics/emotes.rs": "interface/cosmetics/emotes.rs",
    "cosmetics/fx.rs": "interface/cosmetics/fx.rs",
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
    game = Path("crates/horizon-client/src/game")
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
