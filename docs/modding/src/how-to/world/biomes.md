# Biomes

Use biome content, region declarations and level handles to control biome generation, query biomes and edit biome cells.

| I want to | Use | Side |
|---|---|---|
| Add a biome | `data/<ns>/worldgen/biome/<name>.json` in a content mod or data pack ([content mods](../content-mods.md)) | Content |
| Add my features or spawns to vanilla's biomes | `Registrar::declare_biome_modifier` or a file in `data/<ns>/neoforge/biome_modifier/` ([content mods](../content-mods.md)) | Content |
| Give my dimension its own biomes | `DimensionSettings::biome_source`: `Fixed`, `Checkerboard`, `MultiNoise` by preset, `End`, or `Points` for an explicit climate-point layout ([dimension shape](dimension-shape.md)) | Both |
| Put my biomes among the Overworld's or the Nether's | `Registrar::declare_biome_region` with a weighted `BiomeRegion` ([biome regions](../content-mods/biome-regions.md)) | Both |
| Know the biome at a position | `LevelRef::biome_at` on a server level, `LevelRef::client_biome` on a client | Server, client |
| Check a biome's tag membership | [`Host::level_biome_in_tag`](#check-biome-tags) | Both |
| Find the nearest biome or tag | `LevelRef::find_closest_biome` with a `BiomeSearch` | Server |
| Change a biome | `LevelRef::set_biome`, one 4x4x4 cell, as `/fillbiome` does | Server |
| Run bone meal at a position | `LevelRef::bone_meal` | Server |
| Place a feature (a tree, a patch, an ore vein) | `LevelRef::place_feature`, as `/place feature` does ([placing a template](placing-a-template.md#placing-a-feature)) | Server |
| Add a feature type of my own (my tree's shape) | `Registrar::register_feature_type`, named by a configured feature's `"type"` ([feature types](feature-types.md)) | Both |
| Read a position's blended biome tint | [`ClientHost::biome_tint(pos, resolver)`](#read-blended-biome-tints-on-the-client), or `BlockAndTintGetter::block_tint(pos, resolver)` in `BlockTintSource` callbacks | Client |
| Tint foliage, grass or water by biome | The `register_color_resolvers` hook for new colour resolvers, and the `biome_colors` hook to change a biome's own colours | Client |

Code and file modifiers support features, spawns, spawn costs, carvers, climate and effects. They apply additions,
then removals, then modifications when the world loads, after its data packs. Mods cannot choose explicit phases
or supply custom biome-modifier serializers ([content mods](../content-mods.md)). Colour-resolver handles
refer to the slots of [successfully registered resolvers](../../concepts/load-phases.md#registration-and-setup);
a failed callback does not consume a slot.

## Adopt installed biome regions

From the client's world list, open Edit World and confirm adoption to save the installed mods' biome regions for that world. Completed chunks keep their biomes and blocks. New chunks use the adopted layout,
and chunks still being generated may finish with it. See [biome regions](../content-mods/biome-regions.md).

## Check biome tags

Call `host.level_biome_in_tag(level, biome, "minecraft:is_forest")` to check a biome
by ID without loading chunks. The live tables include vanilla, data-pack and mod tags.
Pass the namespaced tag with or without a leading `#`.

- `Some(true)`: the biome belongs to the tag.
- `Some(false)`: the biome is known but is not in the tag, or the tag is undefined.
- `None`: the biome, level or live biome-tag tables are unavailable.

## Read and write limits

- `biome_at` never loads or generates a chunk: an unloaded column answers `None`.
- `find_closest_biome` follows vanilla's sampled spiral (`ServerLevel.findClosestBiome3d`), so it is not a search through cells a mod edited; a negative radius or nonpositive step returns `None`.
- `set_biome` refuses a client, an unknown biome and an unchanged cell, and returns false rather than an error.
- **Biome writes in transactions:** reads inside a transaction see tentative `LevelRef::set_biome` cells, and an abort restores them without publishing changes. Saving, remeshing and resending wait for the outermost commit, once per column. An inner commit does not publish; its cells still roll back if the outer transaction aborts. Outside a transaction these effects run immediately.

## Read biome data

Read the level's biome table without loading chunks. A stale level or unknown biome returns `None`.

```rust,ignore
use mod_api::{Host, LevelRef};

fn inspect(host: &dyn Host, level: LevelRef) {
    if let Some(effects) = host.level_biome_effects(level, "minecraft:plains") {
        let water_rgb = effects.water;
        // Sky and fog preserve omitted declarations as None. Water fog instead
        // uses 0x050533 when omitted. These are not dimension-resolved colours.
        let water_fog_rgb = effects.water_fog;
    }
    if let Some(categories) = host.level_biome_spawners(level, "minecraft:plains") {
        for category in categories {
            // Categories are sorted by name; entries retain biome-file order.
            let entries = category.entries;
        }
    }
}
```

The table includes data packs and biome modifiers. Spawners describe the biome's own natural spawn lists, not a position-specific potential-spawn query.

## Read blended biome tints on the client

Grass, foliage and dry-foliage colormaps are client-only resources. In a client hook in the World
phase, use `ClientHost::biome_tint(pos, resolver)` to query a block position in the current client
level, without looking up a block state or model.

```rust,ignore
use mod_api::{ClientHost, ColorResolver, LevelReadError};

fn grass_tint(host: &dyn ClientHost, pos: [i32; 3]) -> Result<u32, LevelReadError> {
    host.biome_tint(pos, ColorResolver::Grass)
}
```

Pass `ColorResolver::Grass`, `Foliage`, `DryFoliage`, `Water` or a resolver registered in
`register_color_resolvers`. The result is opaque ARGB (`0xFF00_0000 | rgb`), averaged over the
client's biome blend radius, as in vanilla's `BiomeColors.getAverageGrassColor` and related
methods. It uses the mesher's biome colours, including updates from `refresh_biome_colors`.
The host returns `LevelReadError::Unavailable` outside the World phase or when it has no client
level to read.

In a `BlockTintSource` callback (`color_in_world`, `color_as_terrain_particle`), make the same
query on the meshing region passed to the callback with `BlockAndTintGetter::block_tint`:

```rust,ignore
use mod_api::{BlockAndTintGetter, ColorResolver};

fn sample_in_tint_source(level: &dyn BlockAndTintGetter, pos: [i32; 3]) {
    let _grass_argb = level.block_tint(pos, ColorResolver::Grass);
    let _foliage_argb = level.block_tint(pos, ColorResolver::Foliage);
    let _dry_foliage_argb = level.block_tint(pos, ColorResolver::DryFoliage);
    let _water_argb = level.block_tint(pos, ColorResolver::Water);
}
```

To match the tint drawn on a **specific face** of a block or fluid, use
[`ClientHost::block_tint_color(state, tint_index, pos)`](../client/model-drawing.md#copycat-faces)
with that face's tint index. It reads the same current biome colours, including after a refresh,
but its `0xRRGGBB` result may differ because the face can use a different tint source or a pack's
block or fluid colormap.
