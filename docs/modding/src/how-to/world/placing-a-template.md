# Placing a structure template

`host.place_template` puts a structure template into the level through the game's own placement, the
one `/place template` and a structure block's Load take. Blocks, block-entity data and the template's
own entities arrive as vanilla places them: states are mirrored then turned, a block that carries a
block entity keeps its data, loot tables get the seed a placement gives them, and the template's
entities are spawned where the turn puts them.

```rust,ignore
let placed = host.place_template(
    &StructureTemplateSource::id("mymod:hut"),
    [120, 64, -40], // at
    [120, 64, -40], // pivot: what /place template uses
    StructureRotation::None,
    StructureMirror::None,
);
if let Err(reason) = placed {
    // Nothing was placed; `reason` says why.
}
```

Side: server. The call acts on the level the server is ticking, as `edit_blocks` and `native_set_block`
do, and refuses a client.

## Where the template comes from

`StructureTemplateSource` is one of two things:

- `StructureTemplateSource::id("mymod:hut")`: a template of the game's, found as
  `StructureTemplateManager.get` finds it, the way `structure_template` finds it: the world's saved
  structures first, then the game's and the mods' own.
- `StructureTemplateSource::nbt(bytes)`: vanilla structure NBT the mod carries, the bytes of a
  `structures/<namespace>/<path>.nbt` file, gzipped as the game saves one or plain. The game's own
  loader reads it, within vanilla's limits for a template file (a `structure/*.nbt` read under
  `NbtAccounter.unlimitedHeap`, the byte cap as the only guard).

## What the placement does

- `at` is the template's corner, `rotation` and `mirror` turn it, and `pivot` is the level position the
  turn and the mirroring go around. `/place template` passes `at` twice, which turns the template about
  its own origin; that is the placement to copy when the mod has no opinion.
- The palette is the one the placement picks at `at`, as `/place template` picks it.
- No processor runs, there is no `integrity` and no `strict`: everything in the palette places, and the
  neighbours of a replaced block are not told. Loot seeds come from the clock, as `/place template`
  runs without a `seed` argument.
- The chunks the turned template covers all have to be loaded. The call checks the box first and places
  nothing at all rather than half a structure.

`structure_template` is the read side of the same thing: it answers the transformed blocks for a
preview, and `place_template` places them. A mod that wants to place blocks its own way, one at a time
or through `edit_blocks`, keeps using `structure_template`; a mod that wants vanilla's block entities
and entities uses this call.

## Refusals

`Err(String)` says why nothing was placed, and nothing of the template reaches the level:

| Reason | What was wrong |
|---|---|
| not a template id | the id doesn't parse, or has a `.` or `..` part |
| the game has no structure template | nothing of that id is the world's, the game's or the mods' |
| the structure NBT does not decompress / does not read | the bytes are not a gzipped or plain structure file, or they are past vanilla's limits |
| the template places nothing | it has no palette, or a side of its size is below 1 |
| a client of a server places nothing | the call ran on a client |
| chunk (x, z) of the template's box is not loaded | the footprint reaches a chunk the level has not loaded |
| the game placed nothing | the game has no structure resources, or the palette holds no block and there is no entity |

Nothing here panics on what a mod supplies: an unreadable file, a rotation this build does not know and
an id with a `..` in it all come back as a reason.

## Placing a feature

`level.place_feature(host, "minecraft:oak", pos)` places a `minecraft:worldgen/feature` by id, as
`/place feature` does: `Feature.place` with the level's random and chunk generator, after the
command's check that the 3x3 chunks around `pos` are loaded. The blocks it sets reach players as
any block change does. The answer is the feature's own report: `Ok(false)` means it reported
failure, and blocks it set before that are kept, as the command keeps them.

```rust,ignore
match level.place_feature(host, "mymod:ender_tree", pos) {
    Ok(true) => { /* placed */ }
    Ok(false) => { /* it reported failure: a tree with no room */ }
    Err(refusal) => { /* nothing changed; `refusal` names why */ }
}
```

Side: server, on the level being ticked. Each refusal changes nothing: `PlaceFeatureRefusal::Stale`,
`Client`, `NotInPlace` (another level than the one in place), `NotLoaded` (a chunk of the ring is not
loaded, or `pos` is out of height) and `UnknownFeature` (the id has no namespace or names no
feature). Each is checked before the feature runs.

`level.place_placed_feature(host, "minecraft:oak_checked", pos)` places a
`minecraft:worldgen/placed_feature` the way direct `PlacedFeature.place` does: its placement
modifiers run in order from `pos`, depth first, and its feature runs at every position they give.
There is no top feature, so a position reaching `minecraft:biome` refuses with
`PlaceFeatureRefusal::BiomeFilter`, not a biome membership check. If an earlier modifier leaves
no positions, the result is `Ok(false)` instead. Earlier modifiers draw the level random even
when the biome filter refuses. This call's feature has not run when its filter refuses, since
all its positions must first pass the filter. A nested placed feature may still have written
before an inner refusal; there is no rollback.

`Ok(true)` means at least one feature invocation reported success; `Ok(false)` means no positions
remained or every invocation reported failure. Blocks set before failure are kept. Other refusals
are checked before placement, as for `place_feature`, with `UnknownFeature` for an id the
placed-feature registry lacks.
