#!/usr/bin/env python3
"""How far along is the fork's split of game.rs, which a sync should wait for.

Moving a function between files is the worst case for a merge: git sees the old file lose it
and a new file gain it, so our own edits stay behind in the file that went. The fork takes
upstream's split of `game.rs` one destination at a time (`tools/sync/take_upstream_game_moves.py`
and `tools/sync/game-moves.tsv`), and a sync that lands in the middle of it pays that cost twice.

    python3 tools/starling/fork_split_watch.py [REF]      (default: arismoko/main)

Fetch first: `git fetch arismoko main`. Prints one line per destination and a summary; it only
reports, so it never fails.
"""
import re
import subprocess
import sys

MANIFEST = "tools/sync/game-moves.tsv"
GAME = "crates/horizon-client/src/game.rs"
CLIENT = "crates/horizon-client/src/"


def show(ref, path):
    """A file at `ref`, or None where it isn't there."""
    done = subprocess.run(
        ["git", "show", f"{ref}:{path}"], capture_output=True, text=True, encoding="utf-8",
        errors="replace",
    )
    return done.stdout if done.returncode == 0 else None


def defines(source, name):
    """Whether `source` still defines `fn name`."""
    return re.search(rf"\bfn\s+{re.escape(name)}\s*[(<]", source) is not None


def progress(manifest, game_rs, exists=lambda _: False):
    """Each destination of the manifest: (fork_path, functions, the ones still in game.rs, moved).

    A destination counts as moved once its file is there and none of its functions are left in
    `game.rs`; `exists` answers for a path under crates/horizon-client/src/.
    """
    out = []
    for line in manifest.splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) < 3 or parts[0] == "upstream_path":
            continue
        fork_path = parts[1].strip()
        names = [n.strip() for n in parts[2].split(",") if n.strip()]
        left = [n for n in names if defines(game_rs, n)]
        out.append((fork_path, names, left, exists(fork_path) and not left))
    return out


def lines(moves):
    """What to print: a line per destination, then the counts."""
    out = []
    for fork_path, names, left, moved in moves:
        mark = "done" if moved else "    "
        out.append(f"  {mark} {fork_path:<44} {len(names):>2} fn, still in game.rs: {len(left)}")
    done = sum(1 for *_, moved in moves if moved)
    named = sum(len(names) for _, names, _, _ in moves)
    left = sum(len(rest) for *_, rest, _ in moves)
    out.append("")
    out.append(f"destinations: {done} of {len(moves)} moved")
    out.append(f"functions: {named - left} of {named} out of game.rs")
    out.append(
        "The fork's split is done here; a sync no longer lands in the middle of it."
        if moves and done == len(moves)
        else "Wait for this to empty before syncing, or expect to pay the moves twice."
    )
    return out


def main():
    ref = sys.argv[1] if len(sys.argv) > 1 else "arismoko/main"
    manifest = show(ref, MANIFEST)
    if manifest is None:
        print(f"{ref} has no {MANIFEST}: nothing to wait for (fetch it first?).")
        return
    game_rs = show(ref, GAME)
    if game_rs is None:
        print(f"{ref} has no {GAME}: the split is further along than this script knows.")
        return
    here = {}

    def exists(fork_path):
        if fork_path not in here:
            here[fork_path] = show(ref, CLIENT + fork_path) is not None
        return here[fork_path]

    print(f"{ref}: {MANIFEST}")
    print("\n".join(lines(progress(manifest, game_rs, exists))))


if __name__ == "__main__":
    main()
