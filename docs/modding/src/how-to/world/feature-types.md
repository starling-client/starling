# Feature types

Register a `FeatureType` in `Hooks::register` to place features named by a configured feature's
`"type"` field. For example, `data/mymod/worldgen/configured_feature/tall_pillar.json` holding
`{"type": "mymod:pillar", "height": 4}` uses your registered `mymod:pillar` implementation:

```rust,ignore
{{#include ../../../examples/src/worldgen.rs:feature}}
```

This is vanilla's feature-type registry (`minecraft:worldgen/feature_type`, NeoForge's
`FEATURE_TYPE`); a configured feature is an entry of `minecraft:worldgen/feature`.

A bare name uses your mod's namespace; an explicit valid `namespace:path` keeps its namespace.
These stop startup with a message naming your mod: a duplicate ID, a built-in feature type ID,
a malformed or empty name, or an invalid config schema.

## Config

The second argument of `register_feature_type` is your config's schema: a flat record of
`DatapackField`s, the same one [datapack registries](../content-mods.md) use (`Bool`, `Int`,
`Float`, `Text`, `Identifier`, each with its bounds and an optional default). The game decodes
every configured feature of your type against it when the definition loads: an undeclared key, a
missing required key, a wrong type or a value out of bounds refuses that configured feature then,
naming it and the key, so a bad data pack never reaches generation. `type` is the game's own key.
`Reference` fields aren't accepted here; name another entry with `Identifier`.

`context.config()` is the decoded `DatapackEntry`: `config().get("height")` is a `DatapackValue`,
defaults filled in.

## Where your feature runs

A configured feature of your type places wherever the game places a feature, with no extra code:

| Route | What calls your `place` |
|---|---|
| Biome decoration | A placed feature in a biome's `features` list (or added by a biome modifier), on the worldgen workers, once per position its placement modifiers give |
| Nested features | A selector (`minecraft:random_selector`, `simple_random_selector`, `random_boolean_selector`) choosing it |
| Growth and bone meal | A sapling or grass whose configured feature it is |
| Commands | `/place feature mymod:tall_pillar` |
| Code | `LevelRef::place_feature(host, "mymod:tall_pillar", pos)` |

Vanilla's placement modifiers still choose the positions (`count`, `in_square`, `height_range`,
`biome` and the rest). Your feature runs at each position as soon as the last modifier gives it,
before the modifiers draw for the next position, in vanilla's depth-first order. A position given
twice is placed twice.

## Placing

`FeatureType::place(&self, context: &mut dyn FeatureContext, origin: [i32; 3])` returns:

- `Ok(true)` when the feature placed.
- `Ok(false)` when it didn't, for example a tree with no room. Blocks written before that are
  kept, as vanilla keeps them.
- `Err(FeatureError::new(cause))` for a failure. Pass the original error with
  `.map_err(FeatureError::new)?` (`BlockStateError` implements `std::error::Error`), or use
  `std::io::Error::other(message)` for a plain message. The game logs your mod, the feature type,
  the origin and the cause chain each time, then treats this invocation as unsuccessful.
  `FeatureError` is not a `Wire` value.

Writes go straight to the world. Block, tag and height reads in this invocation see the work
already done, as do later placements. An ordinary per-operation refusal (`false` from
`ensure_can_write`, `set_block` or `block_entity`, or `None` from a read) only answers that call;
your callback can return `Ok(false)`, report an `Err` or continue. Neither `Err` nor a refusal
disables a callback, and no outcome undoes completed writes or random draws.

A panic in your callback is logged and queued for an error toast naming your mod. It fails
that invocation without undoing completed writes or draws, and disables only that registered
callback for this session, not other types registered by your mod.

There is no `Host` in a feature, and a feature can't place another feature. Your code runs on the
thread its placement does: a worldgen worker for decoration (chunks generate on several at once),
the tick thread for growth, `/place feature` and `LevelRef::place_feature`. With no `Host` or
game state in reach, a worker never touches mod state through you. Your type must be
`Send + Sync`, stateless and deterministic: draw only from `context.random()`, the same stream
the modifiers before you drew from.

| `FeatureContext` access | Purpose |
|---|---|
| `config` | The configured feature's decoded settings |
| `random` | The caller's `RandomSource`: `next_int`, `next_int_bound`, `next_long`, `next_float`, `next_double`, `next_boolean`, `next_gaussian` |
| `block`, `block_in_tag` | Read current blocks and test membership in a resolved `TagKey` |
| `height` | Read the current heightmap |
| `ensure_can_write` | Check the region's write admission at a position without writing |
| `set_block` | Write a `BlockState` immediately with `BlockUpdateFlags`; `Feature.setBlock` uses `NEIGHBORS \| CLIENTS` |
| `block_entity` | Apply saved NBT to the current block's block entity |
| `state`, `state_with`, `is_air`, `metadata` | Resolve a registered `BlockKey` to a `BlockState`, and test states |
| `min_y`, `max_y` | The level's height bounds |
| `biome` | The biome at a position (`WorldGenLevel.getBiome`), or `None` where the level can't be read |

During FEATURES decoration, `WorldGenRegion.ensureCanWrite` allows the center chunk and its
adjacent chunks, including diagonals. If the center is upgrading, its generation height also
restricts writes throughout that region, regardless of the target chunk. Reads follow
`ChunkPyramid.GENERATION_PYRAMID`'s eight-chunk `STRUCTURE_STARTS` dependency radius: chunks
at Chebyshev distance 2..=8 log vanilla's unsafe-read warning and return resident blocks and
heights or an empty `STRUCTURE_STARTS` holder (`minecraft:air` inside build height,
`minecraft:void_air` outside, height `min_y`), while reads past radius 8 return `None`. The live
level allows the preflight check and enforces its own bounds when writing. `set_block` returns the
world's result: vanilla's live level returns `false` for an unchanged state, but an admitted write
in a generation region can return `true` without changing a block. A read the level can't answer
is `None`. These operations do not load chunks.

`block_entity` can update a block entity that was there before this invocation. The saved
`id` must fit the current block. Invalid NBT or a position the world won't let you change
returns `false`, including a full chunk that decoration only reads.

Resolve block states once per placement, or keep the `BlockKey`s from registration, rather than
resolving names for every block.

## Choosing a tree decorator's ground

`alter_ground` (NeoForge's `AlterGroundEvent`) lets a mod replace the ground provider of an
`AlterGroundDecorator`, such as the one that puts podzol under a spruce. To change only some
placements, wrap the `event.provider` you found and delegate the other positions to it with
`wrapped.state_at(level, pos)`.

A `GroundStateProvider::state_at` returns `Option<BlockState>`. `GroundLevel::block_state(pos)`
reads the region's canonical `BlockState`, and `GroundLevel::decorator_state(pos)` asks the
decorator's own provider for an `Option<BlockState>`. Compare or return those values directly;
for a fixed replacement, capture a state resolved through `host.block_state_id(name)` during
setup. There is no game `Host` in the provider. `None` tries the next position down; a state the
placing registry does not have stops the feature. Use `level.random()` for any draws so they stay
in order with the decorator's own.
