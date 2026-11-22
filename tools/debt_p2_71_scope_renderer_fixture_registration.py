#!/usr/bin/env python3
"""Separate renderer fixture registration from resource reads in the reviewed files."""

from pathlib import Path
import re
import textwrap

RENDER = Path(__file__).resolve().parents[1] / "crates/sunrise-client-render/src"


def scope_registration(source, indent, fixture, factory, count, drop_first):
    opening = "mod_platform::testing::with_mods(mods, || {\n"
    setup_end = f"{indent}    let ({fixture}, kept) = {factory};\n"
    closing = f"{indent}}});\n"
    pattern = re.compile(
        f"^{re.escape(indent + opening)}(?P<setup>.*?)"
        f"^{re.escape(setup_end)}(?P<checks>.*?)^{re.escape(closing)}",
        re.MULTILINE | re.DOTALL,
    )
    matches = list(pattern.finditer(source))
    if len(matches) != count or source.count(setup_end) != count:
        raise SystemExit(f"expected {count} registration scopes for {factory}")

    for index, match in reversed(list(enumerate(matches))):
        setup = match["setup"]
        if not setup.startswith(f"{indent}    let mut ") or "testing::with_mods(" in setup:
            raise SystemExit(f"unexpected registration setup for {factory}")
        checks = match["checks"]
        if not checks.startswith(f"{indent}    assert_eq!("):
            raise SystemExit(f"unexpected fixture checks for {factory}")
        ends_early = index == 0 and drop_first
        binding = fixture.removeprefix("_") if ends_early else fixture
        replacement = (
            f"{indent}let ({binding}, kept) = {opening}{setup}"
            f"{indent}    {factory}\n{closing}"
            + textwrap.indent(textwrap.dedent(checks), indent)
        )
        if ends_early:
            # This test checks an ordinary load after the legacy override is gone.
            replacement += f"{indent}drop({binding});\n"
        source = source[: match.start()] + replacement + source[match.end() :]
    return source


def main():
    changes = {}
    for name, indent, fixture, factory, items, count, drop_first in (
        ("mod_model_loaders", 8, "_fixture", "LoadersFixture", "loaders", 3, True),
        ("mod_block_state_models_tests", 4, "_def_fixture", "DefinitionsFixture", "definitions", 4, True),
        ("mod_sprite_sources", 8, "_fixture", "SourcesFixture", "registrations", 2, False),
    ):
        path = RENDER / f"{name}.rs"
        changes[path] = scope_registration(
            path.read_text(), " " * indent, fixture, f"{factory}::from_registrations({items})",
            count, drop_first,
        )
    for path, source in changes.items():
        path.write_text(source)


if __name__ == "__main__":
    main()
