#!/usr/bin/env python3
"""Kill the slot-duplication and lost-component mutants in production game code."""
import argparse
import hashlib
import re
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--output", type=Path, default=Path("target/review-1674-mutants"))
    args = parser.parse_args()
    repo = args.repo.resolve()
    output = (repo / args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    source = repo / "crates/horizon-client/src/game/modding/api_values.rs"
    original = source.read_bytes()
    text = original.decode()
    mutants = [
        (
            "unstaged_clicked_slot",
            "let accepted = event.clicked.with_staged(&mut run);",
            "let accepted = run(event.clicked);",
            "game::api_values::stacked_slot_tests::invalid_cursor_rolls_back_insert_and_take_without_item_duplication",
        ),
        (
            "drop_component_patch",
            "horizon_items::item_data::encode_extra_components(self)",
            "None::<Tag>",
            "game::api_values::stack_limit_tests::production_snapshots_preserve_component_corpus_contents",
        ),
    ]

    def run(name, selected, exact=False):
        command = ["cargo", "test", "-p", "horizon-client", "--lib", selected, "--", "--nocapture"]
        if exact:
            command.append("--exact")
        with (output / f"{name}.log").open("w") as log:
            result = subprocess.run(command, cwd=repo, stdout=log, stderr=subprocess.STDOUT, check=False)
        return result.returncode, (output / f"{name}.log").read_text()

    print(f"Production source SHA256: {hashlib.sha256(original).hexdigest()}", flush=True)
    status, log = run("baseline", "api_values::")
    if status or not re.search(r"test result: ok\. \d+ passed; 0 failed;", log) or not all(
        f"test {selected} ... ok" in log for _, _, _, selected in mutants
    ):
        raise RuntimeError("Production baseline failed or missed a regression; see baseline.log")
    try:
        for name, before, after, selected in mutants:
            if text.count(before) != 1:
                raise RuntimeError(f"{name}: expected one production mutation site")
            source.write_text(text.replace(before, after))
            status, log = run(name, selected, exact=True)
            if not status or "test result: FAILED. 0 passed; 1 failed" not in log:
                raise RuntimeError(f"{name}: not killed by an executable assertion; see its log")
            print(f"KILLED {name}: 1 selected regression failed", flush=True)
    finally:
        source.write_bytes(original)
    status, log = run("restored", "api_values::")
    result = re.search(r"test result: ok\. (\d+) passed; 0 failed;", log)
    if status or result is None or not all(
        f"test {selected} ... ok" in log for _, _, _, selected in mutants
    ):
        raise RuntimeError("Restored production baseline failed or missed a regression; see restored.log")
    print(f"Restored production source: {result[1]} tests passed", flush=True)


if __name__ == "__main__":
    main()
