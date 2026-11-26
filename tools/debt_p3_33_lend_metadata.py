#!/usr/bin/env python3
"""Migrate the Part 3-33 level metadata APIs and their Rust consumers from Part 2.

Run once on the pre-migration tree. Check all old snippets before writing any files.
"""

from pathlib import Path
from textwrap import indent

ROOT = Path(__file__).resolve().parents[1]
VOCAB = ROOT / "crates/sunrise-mod-vocab/src"
GAME = ROOT / "crates/sunrise-client/src/game"


def main():
    updates = {}

    def replace(path, old, new):
        source = updates[path] if path in updates else path.read_text(encoding="utf-8")
        count = source.count(old)
        if count != 1:
            raise SystemExit(f"{path.relative_to(ROOT)}: expected one match, found {count}: {old!r}")
        updates[path] = source.replace(old, new, 1)

    refs = GAME / "modding/native_level_refs.rs"
    property_docs = """    /// Side: both.
    /// Property `key` of the block state at `pos` (`BlockState.getValue`):
    /// `east` for `facing`. None for a block without it, or as for
    /// `block_state`. API 381.
"""
    replace(
        VOCAB / "host/level_blocks.rs",
        property_docs + """    #[access(WorldOnly)]
    fn level_block_property(&self, level: LevelRef, pos: [i32; 3], key: &str) -> Option<String>;
""",
        property_docs + """    #[access(WorldOnly)]
    #[manual_handle]
    fn level_block_property(&self, level: LevelRef, pos: [i32; 3], key: &str) -> Option<&str>;
""",
    )
    types = VOCAB / "world/types.rs"
    replace(
        types,
        "    /// Borrow the named server level's live random stream",
        property_docs + """    /// The value borrows `host`, not `key`.
    pub fn block_property<'h>(
        self,
        host: &'h dyn Host,
        pos: [i32; 3],
        key: &str,
    ) -> Option<&'h str> {
        crate::backend::backend_of(host)?.level_block_property(self, pos, key)
    }
    /// Borrow the named server level's live random stream""",
    )
    replace(
        refs,
        """    ) -> Option<String> {
        let state = self.level_block_state(level, pos)?;
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .cloned()
""",
        """    ) -> Option<&str> {
        let state = self.level_block_state(level, pos)?;
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .map(String::as_str)
""",
    )

    dimension = VOCAB / "world/dimension.rs"
    replace(
        dimension,
        """/// [`DimensionSettings`] that apply, plus the type's id.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct LevelDimensionType {
    /// The dimension type's id (`minecraft:the_nether`, or a mod's or pack's).
    pub id: String,
""",
        """/// [`DimensionSettings`] that apply, plus the type's id. The ids and timelines borrow the host.
// no wire: borrows the level's dimension type metadata.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct LevelDimensionType<'a> {
    /// The dimension type's id (`minecraft:the_nether`, or a mod's or pack's).
    pub id: &'a str,
""",
    )
    replace(
        dimension,
        "    /// The block tag whose blocks burn forever.\n    pub infiniburn: String,",
        "    /// The block tag whose blocks burn forever.\n    pub infiniburn: &'a str,",
    )
    replace(
        dimension,
        """    /// The timelines the dimension type names, as ids (`minecraft:day`).
    pub timelines: Vec<String>,
    /// The clock `/time` reads here, if the dimension type has a default one.
    pub default_clock: Option<String>,
""",
        """    /// The timelines the dimension type names, as ids (`minecraft:day`).
    pub timelines: &'a [String],
    /// The clock `/time` reads here, if the dimension type has a default one.
    pub default_clock: Option<&'a str>,
""",
    )
    # DimensionSettings also has some of these fields; change only LevelDimensionType.
    start = updates[dimension].index("pub struct LevelDimensionType<'a> {")
    end = updates[dimension].index("\n}", start) + len("\n}")
    definition = updated = updates[dimension][start:end]
    for field in (
        "skybox", "cardinal_light", "bed_can_sleep", "bed_can_set_spawn",
        "straw_bed_can_sleep", "straw_bed_can_set_spawn",
    ):
        old_field = f"    pub {field}: String,\n"
        if updated.count(old_field) != 1:
            raise SystemExit(f"LevelDimensionType: expected one field line: {old_field!r}")
        updated = updated.replace(old_field, f"    pub {field}: &'static str,\n", 1)
    replace(dimension, definition, updated)

    dimension_signature = "fn level_dimension_type(&self, level: LevelRef) -> Option<LevelDimensionType>"
    for path in (VOCAB / "host/level_dimension.rs", GAME / "modding/native_level_world.rs"):
        replace(path, dimension_signature, "fn level_dimension_type(&self, level: LevelRef) -> Option<LevelDimensionType<'_>>")
    world = GAME / "modding/native_level_world.rs"
    for field in ("id", "infiniburn", "timelines"):
        replace(world, f"            {field}: facts.{field}.clone(),", f"            {field}: &facts.{field},")
    replace(world, "            default_clock: facts.default_clock.clone(),", "            default_clock: facts.default_clock.as_deref(),")
    for variant, name in (
        ("Skybox::None", "none"),
        ("Skybox::Overworld", "overworld"),
        ("Skybox::End", "end"),
        ("CardinalLight::Default", "default"),
        ("CardinalLight::Nether", "nether"),
    ):
        replace(world, f'                {variant} => "{name}".to_owned(),', f'                {variant} => "{name}",')
    for rule in ("bed_rule", "straw_bed_rule"):
        for field in ("can_sleep", "can_set_spawn"):
            prefix = f"            {rule.removesuffix('_rule')}_{field}: facts.{rule}.{field}.name()"
            replace(world, f"{prefix}.to_owned(),", f"{prefix},")

    replace(
        types,
        """/// A map's saved data (`MapItemSavedData`) as it stands: where it is, how far it reaches and
/// its pixels. The markers (players, banners, frames) are not part of it.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct MapState {
""",
        """/// A map's saved data (`MapItemSavedData`) as it stands: where it is, how far it reaches and
/// its pixels. The markers (players, banners, frames) are not part of it. The dimension and
/// colors are borrowed for the duration of [`LevelRef::visit_map_data`]'s visitor.
// no wire: borrows the map for a scoped visit.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct MapState<'a> {
""",
    )
    replace(types, "    /// The dimension it shows, `minecraft:overworld`.\n    pub dimension: String,", "    /// The dimension it shows, `minecraft:overworld`.\n    pub dimension: &'a str,")
    replace(types, "    /// 128 by 128 map colour ids, row by row (`x + z * 128`).\n    pub colors: Vec<u8>,", "    /// 128 by 128 map colour ids, row by row (`x + z * 128`).\n    pub colors: &'a [u8],")

    new_map_signature = """fn level_visit_map_data(
    &self,
    level: LevelRef,
    id: i32,
    visit: &mut dyn FnMut(MapState<'_>),
) -> Option<()>"""
    replace(
        VOCAB / "host/level_saved_data.rs",
        """    /// Maps belong to the world, not to a level. None on a client, for a stale handle and for
    /// a map id nothing ever made.
    #[access(WorldOnly)]
    fn level_map_data(&self, level: LevelRef, id: i32) -> Option<MapState>;
""",
        """    /// Maps belong to the world, not to a level. Calls `visit` once on `Some(())`, lending the
    /// dimension and pixels for the call only. To retain data after the visit, copy it inside `visit`.
    /// None on a client, for a stale handle and for a map id nothing ever made; none of these
    /// calls the visitor.
    #[access(WorldOnly)]
""" + indent(new_map_signature, "    ") + ";\n",
    )
    replace(
        refs,
        """    /// Map `id`'s saved data as it stands (`with_map`). None on a client, for a stale handle
    /// and for a map nothing made.
    pub(super) fn level_map_data(&self, level: LevelRef, id: i32) -> Option<MapState> {
        if !self.native_world_data_allowed(level) {
            return None;
        }
        self.with_map(id, |map| MapState {
            center: map.center,
            scale: map.scale,
            dimension: map.dimension.clone(),
            locked: map.locked,
            tracking: map.tracking,
            unlimited: map.unlimited,
            colors: map.colors.clone(),
        })
    }
""",
        """    /// Visit map `id`'s saved data as it stands (`with_map`). None on a client, for a stale
    /// handle and for a map nothing made.
    pub(super) """ + indent(new_map_signature, "    ").lstrip() + """ {
        if !self.native_world_data_allowed(level) {
            return None;
        }
        self.with_map(id, |map| {
            visit(MapState {
                center: map.center,
                scale: map.scale,
                dimension: &map.dimension,
                locked: map.locked,
                tracking: map.tracking,
                unlimited: map.unlimited,
                colors: &map.colors,
            });
        })
    }
""",
    )
    unwinding = ROOT / "crates/sunrise-mod-api-gen/unwinding-methods.txt"
    replace(unwinding, "level_saved_data::level_map_data\n", "")
    replace(unwinding, "level_saved_data::level_set_respawn_data\n", "level_saved_data::level_set_respawn_data\nlevel_saved_data::level_visit_map_data\n")

    hooks = VOCAB / "hooks_tests.rs"
    replace(hooks, "    let ty = level.dimension_type(host)?;", "    let ty: LevelDimensionType<'_> = level.dimension_type(host)?;")
    replace(
        hooks,
        "    let _: Option<MapState> = level.map_data(read, 0);",
        """    let _: Option<()> = level.visit_map_data(read, 0, &mut |map: MapState<'_>| {
        let _: &str = map.dimension;
        let _: &[u8] = map.colors;
    });""",
    )
    for level, value in (("level", "7"), ("reopened", "7"), ("reopened", "6")):
        call = '.block_property(game, [4, 64, 4], "moisture")'
        expected = f'\n                Some("{value}")\n'
        replace(
            refs,
            f"{level}\n                    {call}\n                    .as_deref(),{expected}",
            f"{level}{call},{expected}",
        )
    replace(
        GAME / "modding/native_server.rs",
        'assert_eq!(log.as_deref(), Some("y"), "placed on a top face, standing");',
        'assert_eq!(log, Some("y"), "placed on a top face, standing");',
    )
    replace(
        refs,
        "            let mut colors = level.map_data(game, id).unwrap().colors;",
        """            let mut colors = Vec::new();
            assert_eq!(
                level.visit_map_data(game, id, &mut |map| colors = map.colors.to_vec()),
                Some(())
            );""",
    )
    replace(
        refs,
        """            let read = level.map_data(game, id).unwrap();
            assert_eq!(read.colors[5 + 2 * 128], 34);
            assert_eq!((read.center, read.scale), ([0, 0], 0));
            assert_eq!(stale.map_data(game, id), None);
            assert!(!stale.set_map_data(game, id, &colors));
            assert_eq!(level.map_data(game, i32::MAX), None, "a map nothing made");""",
        """            assert_eq!(
                level.visit_map_data(game, id, &mut |map| {
                    assert_eq!(map.colors[5 + 2 * 128], 34);
                    assert_eq!((map.center, map.scale), ([0, 0], 0));
                    assert_eq!(map.dimension, "minecraft:overworld");
                }),
                Some(())
            );
            assert_eq!(
                stale.visit_map_data(game, id, &mut |_| panic!("visited a stale handle")),
                None
            );
            assert!(!stale.set_map_data(game, id, &colors));
            assert_eq!(
                level.visit_map_data(game, i32::MAX, &mut |_| panic!("visited a missing map")),
                None,
                "a map nothing made"
            );""",
    )
    replace(
        GAME / "network/remote/client_handles_tests.rs",
        "        let facts = level.dimension_type(game).unwrap();",
        "        let facts: mod_api::LevelDimensionType<'_> = level.dimension_type(game).unwrap();",
    )

    for path, source in updates.items():
        path.write_text(source, encoding="utf-8")


if __name__ == "__main__":
    main()
