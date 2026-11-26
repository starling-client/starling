//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `block_states` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait BlockStatesHost {
    /// Side: both.
    /// Resolve a registered fluid identifier.
    fn fluid_key(&self, id: &str) -> Option<FluidKey>;
    /// Side: both.
    /// The state `name` names, read as `/setblock` reads a block
    /// (`BlockStateParser.parseForBlock`): `minecraft:oak_stairs[facing=east]`,
    /// `stone` for `minecraft:stone`, the properties left out at their
    /// default. Returns an owned block key and canonical local ordinal, never
    /// vanilla's numeric `Block.getId`. None for an unknown block,
    /// a property it doesn't have or a value it can't take, or block NBT
    /// (`{...}`); and, as `/setblock`'s argument, for anything after the
    /// block.
    fn block_state_id(&self, name: &str) -> Option<BlockState>;
    /// Side: both.
    /// The state `state` as `/setblock` takes it back
    /// (`BlockStateParser.serialize`): its block, then each property in
    /// name order, `minecraft:oak_stairs[facing=east,half=bottom,...]`, or
    /// the block alone for one with no properties. None for an unknown key
    /// or local ordinal; an invalid value never falls back to air.
    fn block_state_string(&self, state: BlockState) -> Option<String>;
    /// Side: both.
    /// The default state of the resolved `block` key:
    /// `Block.defaultBlockState`, the one a block takes when nothing else
    /// says. None for an unknown block.
    fn block_state_default(&self, block: BlockKey) -> Option<BlockState>;
    /// Side: both.
    /// `Item.getCraftingRemainder`: Sunrise currently stores the remainder's
    /// item id, so the result is a single plain stack.
    fn item_crafting_remainder(&self, stack: &ItemStack) -> Option<ItemStack>;
    /// Side: both.
    /// `Block.asItem`: the block's item identity, including air for a block with
    /// no item; None if either resolved identity is unavailable.
    fn block_item(&self, block: BlockKey) -> Option<ItemKey>;
    /// Side: both.
    /// The block `item` places (`BlockItem.getBlock`, as `Block.byItem` reads
    /// it): `minecraft:wheat_seeds` places `minecraft:wheat`, and a
    /// standing/wall item places its standing block, not the wall variant.
    /// None for an item that places no block, and for an unknown item.
    fn item_block(&self, item: ItemKey) -> Option<BlockKey>;
}
/// The handles' reads of the `block_states` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
#[table_backend("crate::game::native_block_tables::BlockTables")]
pub trait BlockStatesBackend {
    /// Side: both. The age read by `BlockStates::weathering_age`.
    #[forward_api_inputs]
    fn block_weathering_age(&self, state: BlockState) -> Option<WeatheringAge>;
    /// Side: both. The state read by `BlockStates::next_weathering_stage`.
    #[forward_api_inputs]
    fn block_next_weathering_stage(&self, state: BlockState) -> Option<BlockState>;
    /// Side: both. The state read by `BlockStates::previous_weathering_stage`.
    #[forward_api_inputs]
    fn block_previous_weathering_stage(&self, state: BlockState) -> Option<BlockState>;
    /// Side: both. The state read by `BlockStates::waxed`.
    #[forward_api_inputs]
    fn block_state_waxed(&self, state: BlockState) -> Option<BlockState>;
    /// Side: both. The state read by `BlockStates::unwaxed`.
    #[forward_api_inputs]
    fn block_state_unwaxed(&self, state: BlockState) -> Option<BlockState>;
    /// Side: both.
    /// Borrow the canonical block name through the block metadata resource.
    fn block_metadata_name(&self, block: BlockKey) -> Result<&str, BlockStateError>;
    /// Side: both.
    /// Borrow the serialized property value for this `BlockState` from immutable block metadata.
    #[forward_api_inputs]
    fn block_metadata_property(
        &self,
        state: BlockState,
        name: &str,
    ) -> Result<Option<&str>, BlockStateError>;
    /// Side: both.
    /// Visit each property of this `BlockState` with its serialized value, in name order, from
    /// immutable block metadata.
    #[forward_api_inputs]
    fn block_metadata_visit_properties(
        &self,
        state: BlockState,
        visit: &mut dyn FnMut(&str, &str),
    ) -> Result<(), BlockStateError>;
    /// Side: both.
    /// Return a `BlockState` with one property changed, without a world write.
    #[forward_api_inputs]
    #[forward_api_output]
    fn block_metadata_with(
        &self,
        state: BlockState,
        name: &str,
        value: &str,
    ) -> Result<BlockState, BlockStateError>;
    /// Side: both.
    /// Return a `BlockState` by applying partial constraints to the block's default.
    #[forward_api_output]
    fn block_metadata_of(
        &self,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<BlockState, BlockStateError>;
    /// Side: both.
    /// The key of the block registered as `id`, read by `BlockMetadata::key`.
    fn block_metadata_key(&self, id: &str) -> Option<BlockKey>;
    /// Side: both.
    /// The properties of `block`, read by `BlockMetadata::state_definition`.
    fn block_metadata_state_definition(
        &self,
        block: BlockKey,
    ) -> Result<StateProperties<'_>, BlockStateError>;
    /// Side: both.
    /// The owned metadata read by `BlockStates::metadata`.
    fn block_metadata_snapshot(&self) -> Option<std::sync::Arc<dyn BlockMetadata + Send + Sync>>;
    /// Side: both.
    /// `BlockState.canBeReplaced()`: whether placing a block where this state stands
    /// replaces it (air, grass, water, snow layers). None for an unknown state.
    fn block_state_replaceable(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// `BlockBehaviour.Properties.destroyTime`, as `BlockState.getDestroySpeed` reads it:
    /// stone 1.5, obsidian 50, bedrock -1 (it can't be broken). None for an unknown state.
    fn block_state_destroy_speed(&self, state: BlockState) -> Option<f32>;
    /// Side: both.
    /// `ItemStack.isCorrectToolForDrops(state)`: whether `stack`'s `minecraft:tool` component has a
    /// rule that sets `correct_for_drops` and names the block of `state`, and the first such rule
    /// says yes. A stack with no tool component, or no matching rule, is not the correct tool. Only
    /// the stack is asked: not whether the block requires a tool, the player, the position or any
    /// mod's hooks (`PlayerRef::has_correct_tool_for_drops` asks those). None for an unknown state.
    fn block_state_correct_tool_for_drops(
        &self,
        stack: &ItemStack,
        state: BlockState,
    ) -> Option<bool>;
    /// Side: both.
    /// `BlockState.requiresCorrectToolForDrops()`: whether the block drops its loot only when
    /// mined with a correct tool (stone and ores yes, dirt and logs no). None for an unknown state.
    fn block_state_requires_correct_tool(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// The explosion resistance the block of this state declares (`Block.getExplosionResistance`,
    /// `BlockBehaviour.Properties.explosionResistance`): stone 6, obsidian 1200, bedrock 3,600,000,
    /// water 100. Not what a blast meets at a position, which parts, fluids and the blast's own
    /// rules change (`LevelRef::explosion_resistance`). None for an unknown state. A Sunrise
    /// addition: vanilla only reads it per position.
    fn block_state_explosion_resistance(&self, state: BlockState) -> Option<f32>;
    /// Side: both.
    /// `BlockState.getSoundType()`: its identity, volume, pitch and the sound events for breaking,
    /// stepping on, placing, hitting and falling on it. The keys describe the base state,
    /// not the contextual answer of a block part or the events in the loaded sound resources.
    /// None for an unknown state.
    fn block_state_sound(&self, state: BlockState) -> Option<BlockSoundType>;
    /// Side: both.
    /// `BlockState.hasBlockEntity()`. None for an unknown state.
    fn block_state_has_block_entity(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// `BlockState.getMapColor(level, pos)` for a state whose colour does not depend on where it
    /// stands: its `MapColor.id`, 0 (`MapColor.NONE`) for no colour. Map items colour a column
    /// by it (`MapItem.update`). None for an unknown state.
    fn block_state_map_color(&self, state: BlockState) -> Option<u8>;
    /// Side: both.
    /// Decode a packed map color for [`BlockStates::map_color_rgba`].
    fn block_state_map_color_rgba(&self, packed: u8) -> Option<[u8; 4]>;
    /// Side: both.
    /// `BlockState.getLightEmission()`: the light level the state gives off, 0 to 15 (a torch 14,
    /// glowstone 15, stone 0). The state's own value: a block that declares `dynamic_light`
    /// answers per cell through its parts, which this does not ask. None for an unknown state.
    fn block_state_light_emission(&self, state: BlockState) -> Option<u8>;
    /// Side: both.
    /// The friction of the state's block (`BlockBehaviour.getFriction`, as
    /// `BlockState.getBlock().getFriction()` reads it): 0.6 for most blocks, 0.98 for ice, 0.8
    /// for slime. None for an unknown state.
    fn block_state_friction(&self, state: BlockState) -> Option<f32>;
    /// Side: both.
    /// The speed factor of the state's block (`Block.getSpeedFactor`, its
    /// `BlockBehaviour.Properties.speedFactor`): 1.0 for most blocks, 0.4 for soul sand and
    /// honey. The declared value, not what the block's parts answer in a level. None for an
    /// unknown state.
    fn block_state_speed_factor(&self, state: BlockState) -> Option<f32>;
    /// Side: both.
    /// The jump factor of the state's block (`Block.getJumpFactor`, its
    /// `BlockBehaviour.Properties.jumpFactor`): 1.0 for most blocks, 0.5 for honey. The
    /// declared value, not what the block's parts answer in a level. None for an unknown state.
    fn block_state_jump_factor(&self, state: BlockState) -> Option<f32>;
    /// Side: both.
    /// The state's collision boxes with an empty collision context, in block-local
    /// coordinates (`[min_x, min_y, min_z, max_x, max_y, max_z]`; a box may leave the cell, as a
    /// fence's reaches y 1.5), and whether they hold at every position ([`StateCollision`]).
    /// None for an unknown state.
    fn block_state_collision(&self, state: BlockState) -> Option<StateCollision>;
    /// Side: both.
    /// The bounce restitution and fall-distance reduction the state's block declares
    /// (`BlockBehaviour.Properties.bounceRestitution`, `fallDistanceReduction`): 0.75 and 0.5 for
    /// beds and the shelf mushroom, a bounce of 1.0 for slime, 0 for most blocks. The static
    /// properties only: neither a block part's `bounce_restitution` nor a custom `fallOn` (hay's
    /// 0.2 fall damage multiplier). None for an unknown state.
    fn block_state_landing(&self, state: BlockState) -> Option<BlockLanding>;
    /// Side: both.
    /// The state's walking path type (`WalkNodeEvaluator.getPathTypeFromState`) after the block
    /// parts' `isPathfindable` answers: `Blocked`, `Open`, `Fire`, `Damaging`, `Lava`, `Water`,
    /// `StickyHoney`, `PowderSnow`, `Fence`, doors, trapdoors, leaves. The state alone: neither the
    /// per-position `block_path_type` hook nor the neighbour rules (`checkNeighbourBlocks`:
    /// `FireInNeighbor`, `DamagingInNeighbor`) apply. None for an unknown state.
    fn block_state_path_type(&self, state: BlockState) -> Option<PathType>;
    /// Side: both.
    /// `BlockState.isSolidRender()`: whether the state is a full, occluding cube for rendering
    /// and light (stone yes; glass, leaves and slabs no). Not `isSolid` and not full collision.
    /// None for an unknown state.
    fn block_state_is_solid_render(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// `BlockState.isSolid()`, the cached `legacySolid`, read by `BlockStates::is_solid`.
    fn block_state_is_solid(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// The cached `isCollisionShapeFullBlock`, read by `BlockStates::collision_is_full_block`.
    fn block_state_collision_is_full_block(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// `PoiTypes.forState`: the point of interest type the state belongs to (`minecraft:home`
    /// for a bed's head, `minecraft:librarian` for a lectern, `minecraft:meeting` for a bell),
    /// the states mods add with `extend_poi_types` included. None for a state that is in no
    /// POI type and for an unknown state.
    fn block_state_poi_type(&self, state: BlockState) -> Option<Identifier>;
    /// Side: both.
    /// `BlockState.propagatesSkylightDown()`: whether sky light passes down through the state
    /// unchanged, the block's own answer included (glass, barriers and vegetation override the
    /// shape's). None for an unknown state.
    fn block_state_propagates_skylight_down(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// `BlockState.isAir()`: air, cave air and void air, by the block's own flag rather than
    /// the `#minecraft:air` tag a data pack may change. None for an unknown state.
    fn block_state_is_air(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// `BlockState.isRedstoneConductor` for a state alone. None if the state is unknown
    /// or the predicate needs a level and position.
    fn block_state_is_redstone_conductor(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// The read of `BlockStates::block_class`: `Ok(None)` for an unknown state.
    fn block_state_class(
        &self,
        state: BlockState,
        class: BlockClass,
    ) -> Result<Option<bool>, LevelReadError>;
    /// Side: both.
    /// The read of `BlockStates::is_liquid_block_container`.
    fn block_state_is_liquid_block_container(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// The read of `BlockStates::is_simple_waterlogged`.
    fn block_state_is_simple_waterlogged(&self, state: BlockState) -> Option<bool>;
    /// Side: both.
    /// The resolved reaction read by `BlockStates::piston_push_reaction`.
    #[forward_api_inputs]
    fn block_state_piston_push_reaction(
        &self,
        state: BlockState,
    ) -> Result<Option<PushReaction>, LevelReadError>;
    /// Side: both.
    /// The states of `block` (`StateDefinition.getPossibleStates`) in immutable local ordinal
    /// order, independent of world numeric IDs. None for unavailable metadata or an unknown block.
    fn block_states_of(&self, block: BlockKey) -> Option<BlockStateRange>;
    /// Side: both.
    /// The properties of a resolved block, borrowed from the tables.
    fn block_state_definition(&self, block: BlockKey) -> Option<StateProperties<'_>>;
    /// Side: both.
    /// Match against the existing command registry definitions without allocating on the call.
    fn block_state_matches(
        &self,
        state: BlockState,
        block: BlockKey,
        constraints: &[StateConstraint],
    ) -> Option<Result<bool, StateMatchError>>;
    /// Side: both.
    /// A registered fluid's world default and borrowed legal properties, or TankOnly.
    /// None for an unknown key or unavailable registry.
    fn fluid_state_definition(&self, fluid: FluidKey) -> Option<FluidDefinition<'_>>;
    /// Side: both.
    /// A registered fluid's level-independent type properties.
    /// None means the owning host has no resolved registry or `fluid` isn't in it.
    fn fluid_type_properties(&self, fluid: FluidKey) -> Option<FluidTypeProperties>;
    /// Side: both.
    /// `FluidState.createLegacyBlock`: the block state that holds `state` in the world, a water or
    /// lava block of the matching level (source, flowing amount, falling), a mod's world fluid's
    /// block, or air for the empty fluid. None for a state no fluid can be in (a tank-only
    /// fluid, an amount above 8, a zero amount for anything but the empty fluid, or a source
    /// that isn't full) and without a resolved registry.
    fn fluid_legacy_block(&self, state: FluidState) -> Option<BlockState>;
    /// Side: both.
    /// The outline of `state` as `BlockState.getShape` answers it with an empty getter at the
    /// origin (`EmptyBlockGetter.INSTANCE`, `BlockPos.ZERO`): its boxes, each `[min x, y, z,
    /// max x, y, z]` in the block's own space, with no random offset and none of a world's
    /// context (a state that reads its neighbours or block entity answers what its state alone
    /// gives). Empty for a state with no outline, such as air. None for an id no state has.
    fn block_state_outline(&self, state: BlockState)
    -> Result<Option<&[[f64; 6]]>, LevelReadError>;
    /// Side: both.
    /// The fluid state `state` holds (`BlockState.getFluidState`): water for a waterlogged state,
    /// the empty fluid for most. None for an id no state has.
    fn block_state_fluid_state(
        &self,
        state: BlockState,
    ) -> Result<Option<FluidState>, LevelReadError>;
    /// Side: both.
    /// Whether `fluid` and `other` are the same fluid (`Fluid.isSame`): a source and its
    /// flowing form are, so `minecraft:water` and `minecraft:flowing_water` are the same, water
    /// and lava not. False when either key has no world fluid.
    fn fluid_is_same(&self, fluid: FluidKey, other: FluidKey) -> Result<bool, LevelReadError>;
    /// Side: both.
    /// The bucket item that picks `fluid` up (`Fluid.getBucket`), `minecraft:water_bucket` for
    /// water and flowing water, and for a mod's fluid, tank-only or not, the `bucket_item` it
    /// names (a world fluid's generated `<id>_bucket` by default). None for `minecraft:empty`,
    /// whose bucket is air, for a key with no bucket, for a named item no item has, and for one
    /// of the game's own buckets that would not keep it: the empty bucket, another fluid's or a
    /// mob's.
    fn fluid_bucket(&self, fluid: FluidKey) -> Result<Option<ItemKey>, LevelReadError>;
    /// Side: both.
    /// The fluid a filled bucket the game registers holds (`BucketItem.content`), the inverse of
    /// [`fluid_bucket`](Self::fluid_bucket) for those buckets: `minecraft:water` for
    /// `minecraft:water_bucket`, `minecraft:lava` for `minecraft:lava_bucket`, a mod world
    /// fluid's source for its generated bucket. None for any other item, the empty bucket and an
    /// item a mod declares itself among them.
    fn bucket_fluid(&self, bucket: ItemKey) -> Result<Option<FluidKey>, LevelReadError>;
    /// Side: both.
    /// The sound event a bucket plays as it picks `fluid` up (`Fluid.getPickupSound`),
    /// `minecraft:item.bucket.fill_lava` for lava and flowing lava, `minecraft:item.bucket.fill`
    /// for water and a mod's fluid with a bucket. None for `minecraft:empty`, which has none,
    /// and for a tank-only fluid that names no bucket.
    fn fluid_pickup_sound(&self, fluid: FluidKey) -> Result<Option<&'static str>, LevelReadError>;
    /// Side: both.
    /// The sound event a bucket plays as it empties `fluid` (`BucketItem.playEmptySound`, the
    /// `BUCKET_EMPTY` sound of NeoForge's `FluidType`), `minecraft:item.bucket.empty_lava` for
    /// lava and flowing lava and `minecraft:item.bucket.empty` for water and a mod's fluid with
    /// a bucket. None for `minecraft:empty` and for a tank-only fluid that names no bucket.
    fn fluid_empty_sound(&self, fluid: FluidKey) -> Result<Option<&'static str>, LevelReadError>;
    /// Side: both.
    /// Validate partial property constraints in input order with indexed errors.
    /// An unavailable or tank-only definition returns NoWorldState.
    fn fluid_state_validate(
        &self,
        fluid: FluidKey,
        constraints: &[StateConstraint],
    ) -> Result<(), FluidStateError>;
}
