# Block-state facts

A `BlockState` is a `Copy` value pairing a `BlockKey` with a block-local canonical ordinal, not a
numeric engine state ID or a live object. Use `state.block()` and `state.ordinal()` to read the
pair without a host. Pass `BlockState` directly to metadata and fact queries.

Read facts without a world position on either side through `host.block_states()` (import
`HostBlockStates`). Inside a level callback with no `Host` (such as `BlockPart::random_tick`),
call `level.block_states()` on the view: it is the same service over the game's loaded tables.
Basic fact queries, matching and `definition` allocate nothing; `definition` borrows the property data.

With `let metadata = host.block_states()`, `state.property(&metadata, "facing")` borrows the
serialized value from immutable metadata. `state.with(&metadata, "facing", "east")` returns a
new state without writing to the world. To start from a block's default and apply partial
property constraints, use `BlockState::of(&metadata, key, constraints)`. A missing property on a
known state reads as `Ok(None)`; unavailable metadata, invalid states and refused edits return
`BlockStateError`. To read a state's whole property set, as a tint source forwarding it does,
`state.visit_properties(&metadata, &mut |name, value| ...)` calls back with each property's name
and serialized value, in name order, as borrowed `&str`s (nothing is allocated). A block with no
properties calls nothing; metadata that cannot enumerate returns `Unavailable`. For a known local ordinal, use the
[block's state range](../../concepts/registry-keys.md#block-states).

| Method | Vanilla |
|---|---|
| `replaceable(state) -> Option<bool>` | `BlockState.canBeReplaced()` |
| `destroy_speed(state) -> Option<f32>` | `BlockBehaviour.Properties.destroyTime` (`getDestroySpeed`): stone 1.5, -1 unbreakable |
| `requires_correct_tool(state) -> Option<bool>` | `BlockState.requiresCorrectToolForDrops()`: stone and ores true, dirt and logs false; which tool is correct for a state is `BlockStates::is_correct_tool_for_drops` |
| `sound(state) -> Option<BlockSoundType>` | `BlockState.getSoundType()`: sound type identity, base volume and pitch, and the validated break, step, place, hit and fall event keys |
| `has_block_entity(state) -> Option<bool>` | `BlockState.hasBlockEntity()` |
| `light_emission(state) -> Option<u8>` | `BlockState.getLightEmission()`: 0 to 15; a block with `dynamic_light` answers per cell through its parts, which this does not ask |
| `friction(state) -> Option<f32>` | `BlockBehaviour.getFriction()` of the block: 0.6 usually, 0.98 ice, 0.8 slime |
| `speed_factor(state) -> Option<f32>` | `Block.getSpeedFactor()`: 1.0 usually, 0.4 soul sand and honey; the declared value, which a block's `block_speed_factor` part can change where an entity stands |
| `jump_factor(state) -> Option<f32>` | `Block.getJumpFactor()`: 1.0 usually, 0.5 honey; the declared value, which a block's `block_jump_factor` part can change |
| `collision(state) -> Option<StateCollision>` | `BlockState.getCollisionShape(level, pos, CollisionContext.empty())`: `Fixed` boxes the same at every position, `Contextual` boxes an entity's context can change (scaffolding, powder snow, liquids), or `Positional` (bamboo and dripstone offsets, shulker boxes, moving pistons, published shapes, parts that answer collision): read those per position |
| `landing(state) -> Option<BlockLanding>` | `BlockBehaviour.Properties.bounceRestitution` and `fallDistanceReduction`: 0.75 and 0.5 for beds and the shelf mushroom, a bounce of 1.0 for slime, 0 usually; not a part's `bounce_restitution` or a custom `fallOn` (hay) |
| `path_type(state) -> Option<PathType>` | `WalkNodeEvaluator.getPathTypeFromState` after parts' `isPathfindable`: the state alone, without the `block_path_type` hook or neighbour rules |
| `is_solid_render(state) -> Option<bool>` | `BlockState.isSolidRender()`: a full, occluding cube (stone yes; glass, leaves, slabs no) |
| `is_solid(state) -> Option<bool>` | `BlockState.isSolid()`, the cached `legacySolid`: `forceSolidOn` (cobweb, signs) and `forceSolidOff` (ladder) first; then a dynamic shape (scaffolding) or an empty collision shape (torches) is not solid, and a collision shape large or high enough is (leaves and slabs yes) |
| `collision_is_full_block(state) -> Option<bool>` | `BlockState.isCollisionShapeFullBlock` at the origin with an empty getter, as vanilla caches it (stone and leaves yes, slabs and fences no); `None` for a dynamic shape (shulker boxes, moving pistons, bamboo, scaffolding), which vanilla never caches |
| `poi_type(state) -> Option<Identifier>` | `PoiTypes.forState`: the point of interest type (`minecraft:librarian` for a lectern, `minecraft:meeting` for a bell), mods' `extend_poi_types` states included |
| `propagates_skylight_down(state) -> Option<bool>` | `BlockState.propagatesSkylightDown()`: sky light passes down unchanged, the block's own answer included (glass and air yes, stone no) |
| `is_air(state) -> Option<bool>` | `BlockState.isAir()`: air, cave air and void air, by the block's flag rather than the `#minecraft:air` tag |
| `is_redstone_conductor(state) -> Option<bool>` | `BlockState.isRedstoneConductor` when it depends only on the block state; `None` when a position is needed |
| `map_color(state) -> Option<u8>` | `BlockState.getMapColor`: the `MapColor.id` a map item colours a column by, 0 for none |
| `map_color_rgba(packed) -> Option<[u8; 4]>` | `MapColor.getColorFromPackedId`: decode a map pixel to `[red, green, blue, alpha]`; ID 0 and undefined IDs decode to transparent RGBA; `None` means no game backend |
| `block_class(state, class: BlockClass) -> Result<Option<bool>, LevelReadError>` | Java `instanceof` on the block's vanilla class, subclasses included: `Stair`, `Entity` (`EntityBlock`), `IronBars`, `StainedGlassPane`, `TrapDoor`, `Bonemealable` (`BonemealableBlock`), `Slab`, `Wall`, `Fence`. See below |
| `is_liquid_block_container(state) -> Option<bool>` | `instanceof LiquidBlockContainer`: the waterloggable blocks and kelp, seagrass |
| `is_simple_waterlogged(state) -> Option<bool>` | `instanceof SimpleWaterloggedBlock`; a container that is not one holds water without a `waterlogged` property |
| `piston_push_reaction(state) -> Result<Option<PushReaction>, LevelReadError>` | `BlockState.getPistonPushReaction()`: the resolved reaction used by pistons, including Part answers. See [piston push reactions](piston-push-reactions.md) |
| `states_of(block) -> Option<BlockStateRange>` | `StateDefinition.getPossibleStates`, as owned values in canonical local-ordinal order |
| `definition(block) -> Option<StateProperties<'_>>` | Borrowed property names and legal serialized values |
| `matches(state, block, constraints)` | Partial property match with typed invalid-property and invalid-value errors |

`map_color` returns an unshaded palette ID. Pass a packed map pixel to `map_color_rgba`, or pack an ID with
`id * 4 + shade` (0–3). The shade scales each RGB channel by 180, 220, 255 or 135 out of 255, rounding down.

Use `StateConstraint` for each partial constraint; `StateMatchError` identifies an invalid property or value by index.

`definition(block)` returns `None` for an unknown block or unavailable tables. A known block without
properties returns an empty `StateProperties<'_>` view. Use `properties.iter()` to read each
`StateProperty<'_>` in property-name order. It is `Copy`; its `name: &str` and
`values: &[String]` borrow the loaded table, keeping its value order. The view is tied
to the host (or level view), not the temporary service returned by `block_states()`.
For example, to check whether `facing` accepts `north` without copying the definition:

```rust,ignore
let allows_north = host.block_states().definition(block).is_some_and(|properties| {
    properties.iter().any(|p| p.name == "facing" && p.values.iter().any(|v| v == "north"))
});
```

Copy a property's name with `.to_owned()` and values with `.to_vec()` only if you need them
after that borrow ends.

## Block metadata off the host's thread

`BlockMetadata` is the block registry's metadata a thread can read without a host, as vanilla
reads `BuiltInRegistries.BLOCK` from any thread: `key("minecraft:deepslate_bricks")` resolves a
full identifier to its `BlockKey` (`BuiltInRegistries.BLOCK.getValue`), `default_state(key)` is
`Block.defaultBlockState()`, `state_definition(key)` borrows its properties and legal values
(`StateDefinition`), and `name`, `property`, `with` and `of` read and edit states as above. A chunk
generator's worker gets it from `ChunkAccess::metadata`, so a palette resolves its ids there.
`host.block_states().metadata()` returns the same tables as an owned
`Arc<dyn BlockMetadata + Send + Sync>`: take it on the server or client thread and move it to a
thread of your own, such as one rendering a region to an image. The tables never change once the
registry is loaded.

The conductor query uses the same exported predicate as redstone, not render solidity. It returns
`None` for shulker boxes because their collision bounds depend on the live block entity. To query a
placed block, use `LevelRef::is_redstone_conductor(host, pos)` (server side, `None` for an unloaded
position).

Option-only queries return `None` for an unknown state or block, including an invalid local ordinal, and on a host with no game behind it (a mod's test double). Queries returning `Result<Option<_>, LevelReadError>` distinguish unknown states (`Ok(None)`) from unavailable metadata (`Err(LevelReadError::Unavailable)`). The ordinal order is defined by the complete canonical property constraints, not by the engine's numeric enumeration.

## Spawn rule

A declared block's `spawn_rule` property (`default`, `always`, `never` or `ocelot_or_parrot`; see
[content mods](../content-mods.md)) sets the block's mob spawn surface flags, which natural spawning reads as
`BlockBehaviour.Properties.isValidSpawn` does. The default rule asks for a sturdy top face and light emission below
14 from the block's own light, which `light_emission` above reports without the cell's dynamic answer.

## Block sounds

The facts query does not run the block's parts or check loaded sound resources. Read an event by
name with `sound.step.as_str()`, or borrow the key for a known action with `sound.event(cause)`.

To change a contextual sound through `BlockPart::sound_type`, parse its event key once during
registration with `SoundEventKey::parse("my_mod:block.machine.step")`, handle `None` as invalid
syntax, and keep the key on the part. For example, on `BlockSoundCause::Step`, assign
`event.sound.step = self.step.clone()`. Both `block.stone.step` and `:block.stone.step` resolve to
`minecraft:block.stone.step`. The game reads the event for that action and applies its usual
volume and pitch factors. A solid bucket uses the item's placement event with those same levels.
Block break and placement play in the Blocks category even if their event is from another namespace.
An event missing from the active sound resources plays nothing; it never causes the block to play
its old sound instead.

Changing an event key does not change `sound.identity`: a horse chooses its hoof sound from the
sound type's identity, not from the step or placement events. When constructing a custom
`BlockSoundType` literal, set `identity: BlockSoundTypeIdentity::Other`.

## Contextual friction

`host.block_states().friction(state)` is the static coefficient. To ask the state's friction parts,
use `level.friction(host, pos, state, entity)` on either side. Pass `None` if no entity is asking,
or `Some(entity)` with the caller's live `EntityRef`. The queried state need not be the block at
`pos`. The read never loads chunks or writes blocks. It returns `Ok(None)` for a repeated active
query or a query beyond the nesting bound, and a read error for a stale or unavailable level
or entity, or an unknown state.

Inside `BlockPart::friction`, match `LevelSide` to borrow the read-only level. `view.entity_kind(entity)` reads the type
key for the entity lent in `event.entity`, without looking up other entities. Use
`view.block_entity(pos)` to get a resident block entity's handle, then
`view.block_entity_data(handle)` to read its saved data without identity or position metadata.
On the client this reads the local copy, not unsynced server data; synchronize any data that
friction needs on both sides. For a block that takes its material's friction, pass that
material to `view.friction(pos, material, event.entity)` at the **same position** and keep the
original coefficient if the query returns `None`. Use the same rule on both sides, including
client prediction and replay.

A part's answer is used as given, as vanilla uses what `getFriction` returns: any finite value,
above 1 or below 0 included. A non-finite answer is refused and the state's own coefficient is
used.

## Contextual light emission

`level.get_light_emission(host, pos, state)` asks how much light the supplied state emits in
an open server level or the current client level. The position supplies context: the state need
not be placed there, and the query doesn't load its chunk. A block declaring `dynamic_light`
can answer through its [Part](../../concepts/parts/block-parts.md#light-emission); a static state
or a Part that declines uses the declared emission. The query returns the raw value, including
`Ok(0)` for zero emission. `LevelReadError::Stale` means a stale level handle;
`Unavailable` means the level, state or view is unavailable, or a light-emission
provider is already answering for this state. That refusal applies even at another level or
position; a provider can still ask another state. The read neither publishes light nor requests a relight.

`host.block_states().light_emission(state)` reads only the declared value. To read the light
already propagated to a position, use `level.brightness(host, layer, pos)` instead.

## Contextual enchanting power

`level.get_enchant_power_bonus(host, pos, state, table_pos)` reads the effective power of
`state` on either side. To ask the placed block, get its state from `level.block_state(host, pos)`
first. `table_pos` is context for the [Part](../../concepts/parts/block-parts.md#enchanting-power),
not a block the read validates. The default is 1.0 for a provider-tagged block and 0.0 otherwise.
The read preserves the Part's answer, including signed fractions, NaN and infinity. Apply
your own scan and transmitter checks if summing powers for a table; the read doesn't round or
cap them. It doesn't load chunks or write to the world. A stale handle or unavailable view
or state refuses the query. A nested enchanting-power query for an active provider state
is also refused, even at another level or position; it can still ask a different state.

## Where an enchanting table finds its bookshelves

A table scans vanilla's 32 positions (the ring two blocks out, two blocks high). A mod adds
positions with `registrar.add_enchanting_table_offsets(&[[3, 0, 0], ...])` in `register`; they
follow vanilla's in mod load order, and one already scanned is dropped. The list is fixed at
startup, so a mod whose offsets come from its settings needs a restart to change them; `/reload`
doesn't re-run registration. Each added position adds one `nextInt(16)` draw to every table's
particle tick on the client, and nothing to the server's offers.

At each scanned position, `is_valid_book_shelf(level, table_pos, offset, valid)` decides whether
it counts (`EnchantingTableBlock.isValidBookShelf`). `valid` starts at vanilla's answer: the block
there has a nonzero enchanting power and the block halfway to the table (x and z halved, rounded
toward zero) is in `#minecraft:enchantment_power_transmitter`. A position set to valid adds its
block's enchanting power to the offers, so a block with none still adds 0. The client's
glyphs ask it too, only where vanilla does (after that position's `nextInt(16)` draw is 0), so a
mod that changes it belongs on both sides.

## Block classes

`block_class(state, class)` answers what a mod would ask with `instanceof StairBlock`, `EntityBlock`,
`IronBarsBlock`, `StainedGlassPaneBlock`, `TrapDoorBlock`, `BonemealableBlock`, `SlabBlock`, `WallBlock` or
`FenceBlock`. It reads the engine's own class of the
block, so a subclass counts (a waxed copper stair is a `Stair`) and a mod block declared on a vanilla
behaviour template belongs to that template's classes. It never reads tags: a pack that adds a block
to `minecraft:stairs` does not make it a `Stair`. An unknown state answers `Ok(None)`; a host with no game
behind it answers `Err(Unavailable)`. Plain glass panes are `IronBars` and not
`StainedGlassPane`; vanilla 26.3 has no separate glass-pane class, so tell a plain pane from other bars by its
block key. Classes are added when a consumer needs one.

## Copper weathering and wax

- `weathering_age(state) -> Option<WeatheringAge>` reads `WeatheringCopper.getAge`:
  `Unaffected`, `Exposed`, `Weathered` or `Oxidized`. Waxed blocks and blocks
  without vanilla's weathering behavior have no age.
- `next_weathering_stage(state)` and `previous_weathering_stage(state)` read
  `WeatheringCopper.getNext` and `getPrevious`. At either end of the chain, the
  missing neighbor is `None`.
- `waxed(state)` and `unwaxed(state)` read the corresponding twin from
  `HoneycombItem`'s waxables or inverse map. If `unwaxed(state)` has a result,
  the input is waxed. Use the unwaxed result to read the underlying age.

Pass a `BlockState` directly to each query. The relations return `BlockState` values
and keep the input's compatible properties (such as stair facing or waterlogging).
They use the host's loaded [block data maps](../items/data-maps.md) where present,
falling back to the vanilla relations; server data maps aren't synced to remote clients.
No call changes the world. These reads return `None` until the corresponding tables
are published.

## Fluid-state facts

Import `HostFluidStates` to reach `host.fluid_states()` on either owning side.
`definition(key)` returns `Some(FluidDefinition::World { default, properties })` for a world fluid,
`Some(FluidDefinition::TankOnly)` for a registered tank-only fluid, and `None` for an unknown key or
without a loaded registry. The typed default and borrowed `StateProperties<'_>` come from the loaded
preparation. A world fluid without properties (such as `minecraft:empty`) still returns `World`,
with an empty view.
These metadata reads are available in setup, reload and world callbacks after the fluid keys resolve,
not during registration or configuration.

`validate(key, constraints)` checks partial `StateConstraint` values in input order without matching
any particular state. It returns `Result<(), FluidStateError>`: `UnknownProperty(index)`,
`InvalidValue(index)` or `Duplicate(index)` identifies the first offending input.
`NoWorldState` means the fluid is tank-only or the host has no definition; use `definition` to distinguish them.

`type_properties(key)` returns a `FluidTypeProperties` record, or `None` without a loaded registry or for
a key it doesn't hold; `host.fluid_key(name)` is `None` for an unknown name. The record is the same in every
level: `description_id`, `light_level`, `fall_distance_modifier`, and whether the fluid can push entities, be
swum in, drown, extinguish, hydrate and carry boats. Flowing forms answer as their source. `light_level` is 15 for
lava, 0 for water and the empty fluid, and what a mod fluid's file states. The empty fluid and
tank-only fluids do nothing to an entity and have a fall modifier of 1. Anything that depends on
a level, such as lava's current or source conversion, isn't in the record.
The record also carries three `Fluid` facts: `explosion_resistance` (100 for water, lava and a mod's
world fluid, 0 for the empty or a tank-only fluid), `randomly_ticks` (lava only) and `drip_particle`
(`minecraft:dripping_water` or `minecraft:dripping_lava`, none for the rest).
Last come `FluidType`'s `temperature`, `density` and `viscosity`. Water is 300, 1000 and 1000, lava 1300, 3000 and 6000,
and the empty fluid 0, 0 and 0. A mod's fluid answers what its `data/<id>/sunrise/fluid/` file states, and 300, 1000
and 1000 for a key it leaves out ([content mods](../content-mods.md)).

`legacy_block(state)` is `FluidState.createLegacyBlock`: it returns the canonical `BlockState` of the
water, lava or mod-fluid block that holds a fluid state (a source is level 0, a flowing amount is
`8 - amount`, falling adds 8), or the air state for the empty fluid. Pass the result directly to other
block-state queries. It is `None` without a resolved registry or for a state no fluid can be in: a
tank-only fluid, an amount above 8, a zero amount for anything but the empty fluid, or a source that
isn't full.

What depends on a position is on `LevelRef`, not in this service: `fluid_tick_delay(host, fluid, pos)`
(water 5, lava 30 and 10 in the Nether, a mod fluid's as its `fluid_rules` parts change it there),
`fluid_height`, `fluid_height_for_camera` and `fluid_can_be_replaced_with(host, pos, fluid, direction)`.
`fluid_tick(server, pos)` runs a fluid's flow step immediately; `schedule_fluid_tick` queues one.

`fluid_passage` closes one side of a cell to a flowing fluid. It is asked on the server wherever a fluid decides
where to go: the step down, the choice of sides and every step of the search for the way down, so a closed side
changes which sides the fluid picks and not only whether one cell fills. It is asked only after the game's own
answer was yes, so it can close a side and never open a wall; the answer is not kept, and it reschedules no fluid tick
(the neighbour updates of the block change that moved your answer already do). `fluid_rules` changes how a mod's fluid
flows everywhere it is; `fluid_passage` answers for one step of one position, for any fluid.

`FluidState::matches(constraints)` is a pure local check of carried vanilla fields. Unknown properties
or noncanonical serialized values return false. A falling source remains a source; a flowing state
of level 8 remains flowing. Empty constraints match any state. Validate separately for indexed errors.
Neither matching nor validation allocates, and reading the definition lends its properties without copying.
Unresolved saved tank identities remain `FluidIdentity::Unresolved(handle)`, not world states or tank-only keys.

## Slowing or boosting entities on a block

Attach a `BlockPart` to change how fast, or how high, entities move on your blocks, as a block that overrides NeoForge's `Block.getSpeedFactor()` and `Block.getJumpFactor()` does. Both Part methods get the loop `context` of the side that ticks the entity (no `Host`) and an event with the block `state` and the `factor` to edit, which starts at the block's own value (honey is 0.4 speed and 0.5 jump, soul sand 0.4 speed, every other block 1.0).

```rust,ignore
struct Mud;

impl mod_api::BlockPart for Mud {
    fn block_speed_factor(&self, _: &dyn mod_api::LoopContext, event: &mut mod_api::BlockSpeedFactorEvent) {
        event.factor = 0.25;
    }
    fn block_jump_factor(&self, _: &dyn mod_api::LoopContext, event: &mut mod_api::BlockJumpFactorEvent) {
        event.factor = 0.5;
    }
}

// In `register`: let mud = r.block_key("mymod:mud"); r.block_part(mud, Mud);
```

The entity reads the block at its feet and, when that gives 1.0 (and, for speed, the block is not water or a bubble column), the block below it. A part's factor is the block's factor in that rule, so 1.0 from your part lets the block below decide, exactly as for a vanilla block.

The player, every mob's ground and jump path, boats and minecarts read the factors where vanilla does. Both hooks fire on both sides and on the client's replayed ticks: register the mod on both and keep each method a pure function of its inputs, or prediction and the server disagree. To affect entities on every block, use `living_speed` and `living_jump_power` instead.

See [the hook notes](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/block_factors.rs) for the cost and the details.
