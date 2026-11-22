#!/usr/bin/env python3
"""Compare Sunrise's light reads with the real 26.3 LevelLightEngine.

`tools/light_storage_scenario.txt` is replayed by the Java oracle on the real engine (the sections a client
stores, the padding past the build range, sections between stored ones, columns that unload or are never
loaded) and by `light_storage_tests` on a `World`; both must give the answers recorded in
`tools/light_storage_witness.txt`, and so each other. An ordered fixture also compares the lower sky pad's
storage, saved light NBT and first-packet light masks and bytes when section status or the saved array comes first.
"""
import os
from pathlib import Path

try:
    from . import lab, oracle_harness, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, oracle_harness, test_rust_parity as base, test_model_baking_parity as baking


def main():
    build = Path(os.environ.get("SUNRISE_LIGHT_STORAGE_PARITY_DIR", lab.LOCAL / "light-storage-parity"))
    build.mkdir(parents=True, exist_ok=True)
    lock, jars, _ = base.verify_artifacts()
    classes, dependencies, _ = oracle_harness.compile_oracle(
        build, "VanillaLightStorageOracle", jars, baking.client_libraries(), lock["minecraft"]["java_major"])
    scenario = lab.ROOT / "tools" / "light_storage_scenario.txt"
    expected = build / "vanilla.txt"
    expected_pads = build / "vanilla-padding.txt"
    oracle_harness.java_run(build, classes, dependencies, "VanillaLightStorageOracle",
                            [scenario, expected, expected_pads], "vanilla")
    if expected.read_text() != (lab.ROOT / "tools" / "light_storage_witness.txt").read_text():
        raise AssertionError("the real engine's answers differ from tools/light_storage_witness.txt")
    test = "light_storage_tests::light_reads_follow_vanilla_storage"
    oracle_harness.run_logged([
        "cargo", "test", "--locked", "-p", "sunrise-client-render", "--lib",
        test, "--", "--exact",
    ], build, "rust", env=lab.game_env())
    if f"test {test} ... ok" not in (build / "rust.log").read_text():
        raise AssertionError(f"Rust witness replay did not run: {test}")
    print(f"{sum(line.startswith('read ') for line in expected.read_text().splitlines())} light reads match the real engine")

    probe = oracle_harness.build_probe(build, "light_parity")
    actual_pads = build / "rust-padding.txt"
    oracle_harness.run_logged([*probe, "--ordered-padding", lab.RESOURCES, actual_pads], build, "rust-padding")
    # The outputs include payload bytes: name the first differing row, not its data.
    expected_lines = expected_pads.read_text().splitlines()
    actual_lines = actual_pads.read_text().splitlines()
    for i, (expected_line, actual_line) in enumerate(zip(expected_lines, actual_lines), 1):
        if expected_line != actual_line:
            raise AssertionError(f"ordered pad storage/save/first packet differs at row {i}")
    if len(expected_lines) != len(actual_lines):
        raise AssertionError("incomplete ordered pad storage/save/first-packet output")
    print("both ordered pad representations, saves and first packets match vanilla")


if __name__ == "__main__":
    main()
