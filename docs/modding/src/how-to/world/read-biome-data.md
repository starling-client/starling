# Read biome data

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
