# Structure types

Register a `StructureType` in `Hooks::register` to generate structures named by a data pack's
`"type"` field. For example, a structure JSON with `"type": "mymod:tower"` uses your registered
`mymod:tower` implementation:

```rust,ignore
{{#include ../../../examples/src/worldgen.rs:structure}}
```

A bare name uses your mod's namespace; an explicit valid `namespace:path` keeps its namespace.
A duplicate ID, a built-in structure type ID, or a malformed or empty name stops startup with a
message naming your mod.

## Generating a start

`StructureType::find_generation_point(&self, context: &mut dyn GenerationContext)` returns
`Some(FoundStructure { position, pool_pieces, pieces })` for a start, or `None` to skip the
chunk. Each of `pieces` is an `Arc<dyn StructurePiece>`; `pool_pieces` are pool element pieces
your type laid out itself (below). The start needs at least one piece, counting the jigsaw pieces
below. `FoundStructure` implements `Default`, so `..Default::default()` leaves out what you
don't use.

World generation has no `Host`. Starts run on worker threads, and pieces place per chunk. Your
`StructureType` must be `Send + Sync`, stateless and deterministic: the same seed produces the
same pieces, even when workers generate neighbouring chunks concurrently.

| `GenerationContext` access | Purpose |
|---|---|
| `seed`, `chunk`, `sea_level`, `min_y`, `max_y` | World and chunk coordinates and bounds |
| `random` | `&mut dyn RandomSource` ([Randomness](../../concepts/randomness.md)) seeded with vanilla's `setLargeFeatureSeed` for the chunk |
| `base_height`, `base_columns`, `noise_column` | Height and base-column queries |
| `noise_biome`, `column_has_valid_biome` | Registered `BiomeKey` identities and the structure's valid-biome test |
| `metadata` | Borrowed immutable block metadata |
| `config` | The structure JSON converted to a shared `Arc<Tag>` |
| `assemble_jigsaw` | Lay out pieces from the data pack's template pools (below) |
| `template_pool`, `elements` | Read a template pool, and its elements' jigsaws and boxes, to lay pieces out yourself (below) |
| `attach_jigsaw` | Place one candidate element on one jigsaw of a laid-out piece, with vanilla's checks (below) |

Config conversion runs when definitions load or reload, not per start. Nested JSON nulls are
refused at load. The engine handles structure-set spacing, `biomes` and `terrain_adaptation`.

Both contexts are traits, and `random` hands you a `&mut dyn RandomSource`: see
[Randomness](../../concepts/randomness.md) for its draws.

## Laying out from template pools

`context.assemble_jigsaw(&settings, position)` runs vanilla's `JigsawPlacement.addPieces` from the
data pack's template pools, as a NeoForge structure type calls it from its own
`findGenerationPoint`. Build the settings with `JigsawSettings::new(start_pool, max_depth)`, then
set the fields your type decodes from its config:

| Field | Vanilla setting |
|---|---|
| `start_jigsaw_name` | `start_jigsaw_name` |
| `use_expansion_hack` | `use_expansion_hack` |
| `project_start_to_heightmap` | `project_start_to_heightmap` |
| `max_distance_from_center` | `max_distance_from_center` (`JigsawReach { horizontal, vertical }`) |
| `pool_aliases` | `pool_aliases` (`PoolAliasBinding::Direct`, `Random`, `RandomGroup`) |
| `dimension_padding` | `dimension_padding` (`DimensionPadding { bottom, top }`) |
| `liquid_settings` | `liquid_settings` |

Your type chooses its own depth and reach: the limits `minecraft:jigsaw`'s codec puts on `size`
(0 to 20) and `max_distance_from_center` (1 to 128 across) belong to that type only. Every
placement check of the layout is vanilla's. Aliases are drawn from the world seed at `position`.
A depth of 0 or below places no piece, as in vanilla.

The call draws from `context.random()` and returns a `JigsawLayout` with the start position and
every piece's box, or `None` when no start fits there. The game keeps the laid-out pieces itself:
they join your start, ahead of your own pieces, when `find_generation_point` returns `Some`, and
they save, load and place as `minecraft:jigsaw` pieces do. Return `layout.position` as the
start's position to match vanilla's `JigsawStructure`.

### Laying out by hand

A type with its own layout rules reads the pools itself. `context.template_pool(&name)` returns
the data pack's `TemplatePool`, or `None` when the data pack has none by that name; aliases are
not applied. It offers vanilla's `StructureTemplatePool` reads: `templates`,
`shuffled_templates`, `random_template`, `fallback`, `size` and `max_size`. Each element is a
`PoolElementRef`, whatever its type; `template_location` names a single element's template
(`getTemplateLocation`). `context.elements()` gives any element's jigsaws
(`getShuffledJigsawBlocks`) and box, beside the context's random to shuffle the jigsaws with. `JigsawBlock::can_attach` is vanilla's
`JigsawBlock.canAttach`. Draw from `context.random()` in vanilla's order when you mean to match
it.

Hand the pieces back as `FoundStructure::pool_pieces`, each a `PoolPiece::new(element, position,
ground_level_delta, rotation, bounding_box, liquid)`, with `junctions` set when terrain should
blend towards them. The game keeps them as it keeps `assemble_jigsaw`'s pieces: after those and
before your own `pieces`, saved, loaded and placed as `minecraft:jigsaw` pieces are.

### Attaching children with vanilla's checks

`context.attach_jigsaw(&mut source, &jigsaw, &candidate, &mut space, use_expansion_hack)` is
the inner loop of vanilla's `JigsawPlacement.Placer.tryPlacingChildren` for one candidate: its
rotations in shuffled order, its jigsaws that `can_attach` to `jigsaw`, the child's height
(from the jigsaws when both pieces are rigid, from the surface otherwise), the expansion hack,
and the check that the child's box fits the free space. On the first placement that fits, the
box leaves `space`, `source` gets the junction toward the child, and the child comes back as a
`PoolPiece` with the junction toward `source` and `source`'s liquid settings. It returns `None`
when nothing fits, and `None` without drawing for the empty element.

A `JigsawSpace` is vanilla's free-space shape: `JigsawSpace::new(container)` is the whole box,
`remove` takes a placed box out and `fits` tests one. Everything around the call is your type's,
as it is `tryPlacingChildren`'s: which pieces to visit and in which order, which candidates to
try (the pool's `shuffled_templates`, none at the last depth, then its fallback's, up to the
first empty element) and which space each child goes in (a new space over `source`'s box when
the attached piece would start inside it, otherwise the space `source` was placed in). A rule
of your own, such as a cap on how often a piece appears, skips the candidates it refuses:

```rust,ignore
let (elements, random) = context.elements();
let jigsaws = elements.jigsaws(&piece.element, piece.position, piece.rotation, random);
'jigsaws: for jigsaw in jigsaws {
    let Some(pool) = context.template_pool(&jigsaw.pool) else { continue };
    for candidate in pool.shuffled_templates(context.random()) {
        if candidate.is_empty() {
            break;
        }
        if candidate.template_location() == Some(&capped) && placed >= cap {
            continue;
        }
        if let Some(child) = context.attach_jigsaw(&mut piece, &jigsaw, candidate, &mut space, false) {
            placed += 1;
            children.push(child);
            continue 'jigsaws;
        }
    }
}
```

## Pool element types

Register a `PoolElementType` with `r.register_pool_element_type("corridor", MyCorridor)` (NeoForge's
`STRUCTURE_POOL_ELEMENT` registry). A template pool entry whose `element` has
`"element_type": "mymod:corridor"` then decodes through `PoolElementType::decode`, which gets the
`element` value as NBT. The ID rules are those of structure types. The element works in every
structure that draws from that pool, `minecraft:jigsaw` included.

`decode` returns an `Arc<dyn PoolElement>` or a `StructureError`. An error leaves the whole pool
out and is logged; every structure that uses the pool then stops generating. To wrap another
element, decode it with `context.element(&tag)`, which accepts any element type, and keep the tag
to save it back.

| `PoolElement` method | Vanilla's `StructurePoolElement` |
|---|---|
| `shuffled_jigsaws` | `getShuffledJigsawBlocks`; shuffle with the random it gets |
| `bounding_box` | `getBoundingBox` |
| `place` | `place`, through the chunk's `PieceContext` and an `ElementPlacement` |
| `projection` | `getProjection`: the element's own |
| `ground_level_delta` | `getGroundLevelDelta`, default 1 |
| `save` | The codec's fields; the game adds `element_type` |

Elements are decoded at load and used on worker threads: keep them immutable. A list element's
projection replaces its children's, a mod element's included, as `ListPoolElement` sets it on each
child; `place` reads the projection in effect from `ElementPlacement::projection`.

## Structure processor types

Register a `StructureProcessorType` with
`r.register_structure_processor_type("weather", MyWeatherType)` (NeoForge's
`STRUCTURE_PROCESSOR` registry). A processor list entry with `"processor_type": "mymod:weather"`
decodes through `StructureProcessorType::decode`, which receives the entry's config as NBT and a
`ProcessorDecodeContext` for resolving block states and nested processors or processor lists once
at load. The processor runs inside vanilla and mod structures alike.

| `StructureProcessor` method | Vanilla's `StructureProcessor` |
|---|---|
| `process_block` | `processBlock`: inspect or alter one block (return `Ok(None)` to omit it) |
| `finalize_processing` | `finalizeProcessing`: inspect the piece's processed block list and write neighbouring blocks in the chunk |
| `evaluates_entire_piece_state` | `evaluatesEntirePieceState`: `true` when `finalize_processing` needs every block of the template before clipping to the chunk box |

## Placement modifier types

Register a `PlacementModifierType` with
`r.register_placement_modifier_type("below_sea", MyBelowSeaType)` (NeoForge's
`PLACEMENT_MODIFIER_TYPE` registry). A placed feature's `"placement"` entry with
`"type": "mymod:below_sea"` decodes through `PlacementModifierType::decode`, which receives the
entry's config as NBT and a `PlacementModifierDecodeContext` for resolving block states and nested
modifiers once at load.

`PlacementModifier::modify(&self, context, random, origin, output)` matches vanilla's
`PlacementModifier.modify`: pass zero, one, or many positions to `output`, in order, for the next
modifier in the chain, and return `output`'s error if it gives one (`output(pos)?`). Both
structure processors and placement modifiers run on worker threads and must be `Send + Sync`,
stateless, and deterministic.

## Placing pieces

Implement `StructurePiece` (`Debug + Send + Sync`) with these methods:

| Method | Contract |
|---|---|
| `bounding_box` | Return the piece's world-space `BoundingBox`. |
| `post_process` | Place blocks and entities through `&mut dyn PieceContext`; return `Result<(), StructureError>`. |
| `add_additional_save_data` | Write your piece's fields into its saved `Compound`. |
| `beardifier` | Optional `Option<PieceBeardifier>`, default `None`: override the terrain box, adjustment and ground-level delta. |

After the start's pieces place, `StructureType::after_place` runs once per chunk with every
piece's current bounding box. Its default implementation does nothing. To report a placement or
loading failure, return `Err(StructureError::new(cause))`. Pass the original error with
`.map_err(StructureError::new)?` (`BlockStateError` implements `std::error::Error`), or use
`std::io::Error::other(message)` for a plain message. `std::error::Error::source` exposes the
original cause; `StructureError` is not a `Wire` value.

Construct an override with `PieceBeardifier::new(bounds, TerrainAdjustment::BeardThin, 2)`.
The bounds are independent of the placement box; the ground is `bounds.min[1] + ground_level_delta`.
The rules are `None`, `Bury`, `BeardThin`, `BeardBox` and `Encapsulate`. An answer with
adjustment `None` disables only that piece. No answer inherits the structure's setting.
An override works even when the structure's `terrain_adaptation` is `"none"`.
Unordered bounds are refused with a warning and add no terrain adjustment.
Both `bounding_box` and `beardifier` are queried once when a piece is generated or loaded, and
their answers are cached on the engine piece; terrain generation and chunk placement never call
the mod for them per chunk.

Use `PieceContext` to work directly within the chunk:

| Method | Behaviour |
|---|---|
| `chunk`, `chunk_box` | Identify the chunk and writable box. |
| `set_block` | Write a `BlockState` immediately; positions outside `chunk_box` return `false` silently, and invalid states or world write failures log the position and cause and return `false`. |
| `block`, `height` | Read current blocks and heightmap heights. |
| `state`, `state_with` | Resolve a registered `BlockKey` to an API-owned `BlockState`. |
| `random` | Draw from the piece's `RandomSource`. |
| `metadata` | Borrow block metadata; `metadata().name(state.block())` returns the canonical name. |
| `solid_render` | Check whether a state renders as a solid block. |
| `entity` | Queue saved entity NBT, including its `id`, `Pos` and other fields; a world write failure logs the position (when present) and cause without failing the piece. |
| `block_entity` | Place block entity NBT at a position; a world write failure logs the position and cause without failing the piece. |
| `place_feature`, `place_placed_feature` | Place a configured feature (as `FeaturePoolElement` does) or a placed feature, by ID, drawing from the piece's random. An ID the data pack lacks answers `Ok(false)`. |
| `place_element` | Place any pool element in this chunk, drawing from the piece's random. |

## Errors and panics

- **No start.** Returning `None` from `find_generation_point` simply skips that chunk.
- **Per-operation refusals.** Clipping outside `chunk_box` and unchanged states return `false`
  quietly on `set_block`; an invalid state or world failure on `set_block`, `entity` or
  `block_entity` logs the position and cause without failing the piece.
- **Callback errors (`Err(StructureError)`).** Returning `Err` from `post_process` or
  `after_place` logs your mod, the structure type, the chunk and the full native cause chain each
  time and ends only that invocation without undoing completed writes or random draws: later
  intersecting pieces, `after_place` and later chunks still run. Returning `Err` from `load_piece`
  logs the same cause chain and omits only that saved piece without disabling later loads.
- **Panics.** Each operation (`find_generation_point`, `load_piece`, `bounding_box`,
  `beardifier`, `add_additional_save_data`, `post_process`, `after_place`) is isolated under its
  own callback owner for that registration. A panic is logged and queued for an error toast naming
  your mod, keeps completed writes and draws, and disables only that operation for that
  registration for the session — other operations on the same type and other structure types stay
  active. A panic in `bounding_box` or `beardifier` skips only that piece; a panic in
  `add_additional_save_data` discards the partial saved tag.

Entities remain in the generating chunk until it joins the world. Keep the `"sunrise:finalize"`
byte carried by structure-template entities to request `Mob.finalizeSpawn` with the `STRUCTURE`
reason when the game loads the entity.

Resolve block states once during piece construction or loading. Retain those states for
`post_process` rather than resolving names for every placed block.

## Saving and loading

The engine saves the structure type ID and the fields `BB`, `O` and `GD`, then calls
`add_additional_save_data`. On chunk load, your type's
`load_piece(metadata, tag) -> Result<Arc<dyn StructurePiece>, StructureError>` rebuilds the piece
from that data; the engine does not regenerate it from the seed. Return `StructureError` for
invalid saved block names, properties or piece data. The loader receives the same retained
metadata snapshot that worker contexts borrow.

- Every piece saves with its structure type's ID. There is no separate `STRUCTURE_PIECE` registry.
  If your type uses several piece kinds, save a discriminator in `add_additional_save_data` and
  read it in `load_piece`.
- The engine writes `O` as `-1` and `GD` as `0`. Save your own orientation or depth fields if you
  need them; do not interpret these defaults as your piece's orientation or depth.
- Save canonical block names and properties, never process-local ordinals.

Mod pieces participate in start caching, cross-chunk placement, terrain blending through the
beardifier, and `/locate structure`. A data pack structure whose `"type"` has no registered
implementation places nothing.

## Rule tests and position rule tests

Register custom `minecraft:rule` and feature replacement predicates (`RULE_TEST`) and rule
position predicates (`POS_RULE_TEST`) in `Hooks::register` with
`r.register_rule_test_type("name", MyRuleTestType)` and
`r.register_pos_rule_test_type("name", MyPosRuleTestType)`.

- A bare type name uses the mod's namespace and an explicit valid `namespace:path` keeps its
  namespace. Types decode once when a processor list or configured feature loads; bind block IDs, block states, block tags and nested tests through
  `RuleTestDecodeContext` / `PosRuleTestDecodeContext` at load time so `test` does no
  per-block lookups or allocations.
- Returning `Err(StructureError)` during `decode` (or referencing an unknown type, block or
  tag) leaves that processor list and every template pool naming it out of the load.
- `RuleTest::test` and `PosRuleTest::test` run concurrently on worldgen worker threads and
  must not panic; a panic aborts that structure's placement with nothing substituted.
