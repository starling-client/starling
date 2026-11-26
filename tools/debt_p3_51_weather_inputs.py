#!/usr/bin/env python3
"""Replay the flat weather hook's timer argument plumbing on d46d3820a36."""

from pathlib import Path


def main() -> None:
    crates = Path(__file__).resolve().parents[1] / "crates"
    vocab = crates / "sunrise-mod-vocab/src"
    game = crates / "sunrise-client/src/game"
    edits = {
        vocab / "hooks/weather_change.rs": [
            (
                "    fn weather_change(dimension: &str, raining: bool, thundering: bool, cancel: &mut bool);",
                """    fn weather_change(
        dimension: &str,
        clear_ticks: i32,
        weather_ticks: i32,
        raining: bool,
        thundering: bool,
        cancel: &mut bool,
    );""",
            ),
        ],
        game / "native_hooks/base.rs": [
            (
                "    pub(in crate::game) fn native_weather_change(\n"
                "        &mut self,\n"
                "        dimension: &str,\n",
                "    pub(in crate::game) fn native_weather_change(\n"
                "        &mut self,\n"
                "        dimension: &str,\n"
                "        clear_ticks: i32,\n"
                "        weather_ticks: i32,\n",
            ),
            (
                "        crate::game::level_control_hooks::LevelControl::Weather\n"
                "            .hear(|| self.fire_weather_change(dimension, raining, thundering, cancel));",
                """        crate::game::level_control_hooks::LevelControl::Weather.hear(|| {
            self.fire_weather_change(dimension, clear_ticks, weather_ticks, raining, thundering, cancel);
        });""",
            ),
        ],
        game / "modding/native_level_weather.rs": [
            (
                "        self.native_weather_change(self.dim.id(), raining, thundering, &mut cancel);",
                "        self.native_weather_change(self.dim.id(), clear, time, raining, thundering, &mut cancel);",
            ),
        ],
        vocab / "hooks_tests.rs": [
            (
                "dispatch_weather_change(host, event.dimension, true, true, &mut false);",
                "dispatch_weather_change(host, event.dimension, 0, 2, true, true, &mut false);",
            ),
            (
                "dispatch_weather_change(host, event.dimension, false, false, &mut false);",
                "dispatch_weather_change(host, event.dimension, 3, 0, false, false, &mut false);",
            ),
            (
                'dispatch_weather_change(&mut host, "minecraft:overworld", true, false, &mut false);',
                'dispatch_weather_change(&mut host, "minecraft:overworld", 0, 1, true, false, &mut false);',
            ),
            (
                '                "minecraft:overworld",\n                true,\n                false,\n                &mut cancelled,',
                '                "minecraft:overworld",\n                0,\n                1,\n                true,\n                false,\n                &mut cancelled,',
            ),
            (
                'dispatch_weather_change(&mut host, "minecraft:overworld", true, true, &mut cancelled);',
                'dispatch_weather_change(&mut host, "minecraft:overworld", 0, 2, true, true, &mut cancelled);',
            ),
        ],
    }
    updated = {}
    for path, replacements in edits.items():
        source = path.read_text()
        for old, new in replacements:
            if source.count(old) != 1:
                raise SystemExit(f"{path.relative_to(crates)}: expected one instance of {old!r}")
            source = source.replace(old, new)
        updated[path] = source
    for path, source in updated.items():
        path.write_text(source)


if __name__ == "__main__":
    main()
