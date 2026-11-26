# Placing a structure template

`level.place_template` puts a structure template into a server level (`LevelRef`, active or parked)
through the game's own placement, the one `/place template` and a structure block's Load take.
Blocks, block-entity data and the template's own entities arrive as vanilla places them: states are
mirrored then turned, a block that carries a block entity keeps its data, loot tables get the seed a
placement gives them, and the template's entities are spawned where the turn puts them.

```rust,ignore
let placed = level.place_template(
    host,
    &StructureTemplateSource::id("mymod:hut"),
    [120, 64, -40], // at
    [120, 64, -40], // pivot: what /place template uses
    StructureRotation::None,
    StructureMirror::None,
);
if let Err(reason) = placed {
    // Nothing was placed; `reason` is a typed `StructureTemplateError`.
}
```

Side: server. The call acts on `level` (active or parked), refuses a client with
`StructureTemplateError::Unavailable`, a stale handle with `StructureTemplateError::Stale`, and a
parked level inside an open Host transaction with `StructureTemplateError::InTransaction`.

## Where the template comes from

`StructureTemplateSource` is one of two things:

- `StructureTemplateSource::id("mymod:hut")`: a template of the game's, found as
  `StructureTemplateManager.get` finds it, the way `level.visit_structure_template` finds it: the
  world's saved structures first, then the game's and the mods' own.
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

`level.visit_structure_template` is the read side of the same thing: it lends each transformed
block as a `StructureTemplateBlockView<'_>` in vanilla placement order (stopping early on
`ControlFlow::Break(())`) and returns the rotated size `[i32; 3]`. A mod that wants to place blocks
its own way, one at a time or through `level.edit_blocks`, uses `level.visit_structure_template`; a
mod that wants vanilla's block entities and entities uses `level.place_template`.

## Refusals

`Err(StructureTemplateError)` names why nothing was read or placed, and nothing of the template
reaches the level:

| Variant | What was wrong |
|---|---|
| `Stale` | the level handle is stale or the level is gone |
| `Unavailable` | the call ran on a client, wrong phase/host, or structure resources are missing |
| `InTransaction` | a parked level cannot place a template while a Host transaction is open |
| `InvalidId` | the id doesn't parse, or has a `.` or `..` part |
| `NotFound` | nothing of that id is in the world, the game or the mods |
| `InvalidNbt` | the bytes are not a gzipped or plain structure file, or they are past vanilla's limits |
| `EmptyTemplate` | the template has no palette, a side below 1, or places no blocks or entities |
| `UnloadedChunk { chunk }` | chunk `[cx, cz]` in the transformed footprint is not loaded (`PlaceCommand.checkLoaded`) |
| `InvalidState` | a block state in the template cannot be mapped to the API block state table |
| `UnsupportedTransform` | the rotation, mirror, or template source variant is unsupported by this build |

Nothing here panics on what a mod supplies: an unreadable file, a rotation this build does not know and
an id with a `..` in it all come back as a typed `StructureTemplateError`.

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
