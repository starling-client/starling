#!/usr/bin/env python3
"""Remove the generator's retired split tools, empty fragments and guest markers."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
GEN = ROOT / "crates/sunrise-mod-api-gen/src"
MACROS = ROOT / "crates/sunrise-mod-api-macros/src/lib.rs"
VOCAB = ROOT / "crates/sunrise-mod-vocab/src"
OLD_NOTES = b"\n/// Original upstream notes; the existing fork event above owns this moment.\n"
MARKER = b"    #[sunrise_mod_api_macros::computed_in_mod]\n"


def remove_block(source: bytes, start: bytes, end: bytes) -> bytes:
    before, rest = source.split(start)
    _, after = rest.split(end)
    return before + end + after


def main():
    deleted = [
        *(GEN / f"bin/{name}.rs" for name in (
            "host-manifest", "part-table-layout", "dispatch-fixture-layout", "native-hook-layout"
        )),
        GEN / "layout.rs",
        *(ROOT / f"tools/split_{name}.py" for name in (
            "part_table", "dispatch_fixtures", "native_hooks"
        )),
        *(VOCAB / f"hooks/{name}.rs" for name in ("server_player_entity_id", "upstream_notes")),
    ]
    for name in ("crash", "enchantments", "item_attributes", "stations", "wire"):
        path = VOCAB / f"exports/{name}.rs"
        if path.read_bytes().strip():
            raise ValueError(f"not an empty export fragment: {path}")
        deleted.append(path)
    for path in deleted:
        if not path.is_file():
            raise ValueError(f"missing retired file: {path}")

    lib = GEN / "lib.rs"
    source = lib.read_bytes()
    for declaration in (
        b"#[doc(hidden)]\npub use dispatch_fixtures::layout as dispatch_fixture_layout;\n",
        b"mod layout;\n",
        b"pub use layout::native_hook_layout;\n",
    ):
        if source.count(declaration) != 1:
            raise ValueError(f"expected one declaration in {lib}: {declaration!r}")
        source = source.replace(declaration, b"")
    edits = {lib: source}
    fixtures = GEN / "dispatch_fixtures.rs"
    edits[fixtures] = remove_block(
        fixtures.read_bytes(),
        b"/// Lossless source ranges and catalog ownership for the one-shot fixture move.\n",
        b"#[cfg(test)]\nmod tests {\n",
    )
    edits[MACROS] = remove_block(
        MACROS.read_bytes(),
        b"/// Marks a provided trait method computed from the trait's other methods.\n",
        b"/// Derive subscriptions for internal test fixtures that construct their own mods.\n",
    )
    for name in ("entity_place", "farmland_trample", "item_toss"):
        path = VOCAB / f"hooks/{name}.rs"
        live, retired = path.read_bytes().split(OLD_NOTES)
        if not retired.startswith(b"pub trait UpstreamNotes {\n"):
            raise ValueError(f"missing retired notes in {path}")
        edits[path] = live
    for name, count in (("random", 2), ("types", 1)):
        path = VOCAB / f"world/{name}.rs"
        source = path.read_bytes()
        if source.count(MARKER) != count:
            raise ValueError(f"expected {count} provided-method markers in {path}")
        edits[path] = source.replace(MARKER, b"")

    for path in deleted:
        path.unlink()
    for path, source in edits.items():
        path.write_bytes(source)


if __name__ == "__main__":
    main()
