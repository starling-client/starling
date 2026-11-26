//! The `level_blocks` area: reading and writing the block at a position. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_blocks` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelBlocksBackend {
    /// Visit one resident block's read-only level and resolved capability inputs, or do
    /// nothing when the position cannot be resolved. The borrow ends before server defaults.
    // hidden: the typed capability methods own the query; only their engine adapter visits the site
    #[doc(hidden)]
    fn level_with_capability_site(
        &self,
        at: BlockSide,
        visit: &mut dyn FnMut(&dyn LevelHost, BlockCapabilitySite),
    ) -> Result<(), LevelReadError>;
    /// Side: both.
    /// Whether the supplied candidate state can survive at `pos` (`BlockState.canSurvive`),
    /// evaluated with the level's normal block rules and survival Parts. `state` need not be
    /// placed there. Vegetation checks the soil's `can_sustain_plant` Part before the plant's
    /// own `may_place_on` Part. This does not run placement, obstruction, permission, or
    /// `must_survive` checks. On a client, this is a prediction from its loaded local level.
    /// On a server, this reads any open level named by the handle, including one that is parked
    /// while another dimension is in play. On a client, it predicts only from its loaded local
    /// level. `pos` must be in bounds and loaded; unknown states, unavailable hosts or phases,
    /// and recursive queries return `Err(LevelReadError::Unavailable)`. A dead handle returns
    /// `Err(LevelReadError::Stale)`. It reads without loading, placing, scheduling ticks, or
    /// changing the level.
    #[access(WorldOnly)]
    #[game_method(native_level_can_survive)]
    fn level_can_survive(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
    ) -> Result<bool, LevelReadError>;
    /// Side: both.
    /// The fire odds of the supplied `state` at `pos`, asked from its side `face`, the one
    /// facing the fire or the air fire may take: `(ignite, burn)` as fire itself reads them
    /// (`FireBlock.getIgniteOdds` and `getBurnOdds`; NeoForge's
    /// `IBlockExtension.getFlammability` and `getFireSpreadSpeed`). The mods' `fire_odds`
    /// answers come first, then the world's `neoforge:flammable` data map, then vanilla's
    /// table (`FireBlock.bootStrap`). Ignite odds are at least 0 and burn odds 0 to 300; a
    /// waterlogged state is `(0, 0)`. `state` need not be placed there. `pos` must be in
    /// bounds and loaded; an unknown state, an unavailable host or phase and a query nested in
    /// another block query of the same state at the same position return
    /// `Err(LevelReadError::Unavailable)`, a dead handle `Err(LevelReadError::Stale)`. On a
    /// client this is a prediction from its loaded local level. It reads without loading,
    /// placing or changing the level.
    #[access(WorldOnly)]
    #[game_method(native_level_fire_odds)]
    fn level_fire_odds(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        face: Direction,
    ) -> Result<(i32, i32), LevelReadError>;
    /// Side: both.
    /// The contextual friction of `state` at `pos` (`IBlockExtension.getFriction`).
    /// The state need not occupy the position. `entity` is the optional live handle passed to the block's part. A stale entity
    /// is refused. Returns `Ok(None)` for a nested query that repeats
    /// an active read or exceeds the nesting bound. A stale level, unknown state, or
    /// unavailable host or phase returns a typed read error.
    /// The query never loads chunks or changes the world.
    fn level_friction(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        entity: Option<EntityRef>,
    ) -> Result<Option<f32>, LevelReadError>;
    /// Side: both.
    /// The enchanting power of the supplied `state` at `pos` (`IBlockStateExtension.getEnchantPowerBonus`).
    /// The state need not occupy that position; `table_pos` is context passed to the Part,
    /// not a block that the query looks up. The tag default is 1.0 for providers, 0.0 otherwise.
    /// Returns the Part's answer unchanged, even if it is non-finite. This read never loads
    /// chunks or applies transmitter, bookshelf offset or offer-cost rules.
    /// Returns `Stale` for a stale handle, or `Unavailable` for an unreadable view or an
    /// unknown state. If an enchanting-power provider for this state is already active
    /// on this thread in this game, returns `Unavailable` even at a different level or position.
    /// Delegation to another state is allowed.
    fn level_get_enchant_power_bonus(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        table_pos: [i32; 3],
    ) -> Result<f32, LevelReadError>;
    /// Side: both.
    /// How fast `tool` (None is an empty hand) mines the block at `pos`: the tool's own mining
    /// speed on that block (`ItemStack.getDestroySpeed`, 1.0 for no tool; for a block it has no
    /// rule for, the tool's default mining speed), before the player's enchantments, attributes, water, ground and
    /// `player_break_speed` hooks, which `player_break_speed` itself hears. None for a stale
    /// handle or an unloaded or out-of-height position. Reads no chunks.
    fn level_block_break_speed(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
    ) -> Option<f32>;
    /// Side: both.
    /// The block state ID at `pos` (`Level.getBlockState`), without formatting a string.
    /// None for a chunk that isn't loaded or a `y` outside the world. On a client, the
    /// state is the one the server sent for a chunk the client holds, in the level it shows
    /// or in one the server opened for viewing (`host.level`): a viewed level reads its own
    /// world, never the player's, and never loads a column.
    fn level_block_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockState>;
    /// Side: server.
    /// Preview a held tool's transformation without changing the block or stack. The existing
    /// block-tool hook fires with `simulated` true; listeners must not mutate the level in a
    /// simulation. The player must be live in the level in place, and the block must be loaded;
    /// parked levels cannot provide the hook's level context. A stale handle, invalid face,
    /// unavailable player or level, or no transformation returns None.
    fn level_block_transform(
        &self,
        level: LevelRef,
        player: PlayerRef,
        hand: Hand,
        stack: &ItemStack,
        pos: [i32; 3],
        hit: BlockHit,
    ) -> Option<BlockTransformPreview>;
    /// Side: both.
    /// The block at `pos` without its properties: `minecraft:oak_stairs`.
    /// None as for `block_state`. API 381.
    fn level_block(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockKey>;
    /// Side: both.
    /// Property `key` of the block state at `pos` (`BlockState.getValue`):
    /// `east` for `facing`. None for a block without it, or as for
    /// `block_state`. API 381.
    #[manual_handle]
    fn level_block_property(&self, level: LevelRef, pos: [i32; 3], key: &str) -> Option<&str>;
    /// Side: both.
    /// The mod-published connection group at `pos`, or `Ok(None)` when unset, unloaded, or out
    /// of height. Returns `Err(LevelReadError::Stale)` for a stale handle or closed level, and
    /// `Err(LevelReadError::Unavailable)` when the host is not readable in this phase.
    /// Connection groups are derived visual state: they are synced to Sunrise clients with chunk
    /// loads and updates, never saved to disk, and cleared when the block at `pos` is replaced by
    /// a different block (same-block state changes preserve the group).
    fn level_connection_group(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Result<Option<u64>, LevelReadError>;
}
/// The handles' writes of the `level_blocks` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelBlocksBackendMut {
    /// Side: server.
    /// Set property `key` to `value` on the block at `pos` through `Level.setBlock`,
    /// using the supplied update flags. Preserves the block entity. Returns `Ok(false)`
    /// for a missing property or value, an unchanged state or a refused position.
    /// A parked level cannot be written while a Host transaction is open.
    fn level_set_block_property(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: BlockUpdateFlags,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Drop one item as `DefaultDispenseItemBehavior.execute` does. Use the
    /// returned remainder in the dispenser's answer; the caller plays the answer's
    /// event and smoke. The detached stack keeps its actual count until one item
    /// is dropped; placement limits apply only to the final slot write. An enclosing
    /// transaction defers the drop in the dispenser's level until commit.
    /// A missing or mismatched dispenser returns `InvalidDispenser`.
    /// A parked level cannot be used while a Host transaction is open.
    #[forward_api_inputs]
    #[forward_api_output]
    fn level_dispense_default(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Option<ItemStack>, LevelWriteError>;
    /// Side: server.
    /// `DefaultDispenseItemBehavior.consumeWithRemainder`: one item of `stack` is used.
    /// If it was the last, the slot gets `remainder`; otherwise the remainder goes
    /// into the dispenser or out its front with a click and smoke. Returns the detached
    /// active-slot remainder immediately; inside a transaction, insertion into another
    /// slot is provisional and slot notices, overflow drops, click and smoke are published
    /// in the source level only when the enclosing transaction commits at the root.
    /// A missing or mismatched dispenser returns `InvalidDispenser`.
    /// A parked level cannot be used while a Host transaction is open.
    #[forward_api_inputs]
    #[forward_api_output]
    fn level_dispense_with_remainder(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        facing: Direction,
        stack: ItemStack,
        remainder: ItemStack,
    ) -> Result<Option<ItemStack>, LevelWriteError>;
    /// Side: server.
    /// Set a block (`Level.setBlock`). Stale/unavailable levels and invalid states return
    /// typed errors without mutation. Ok(false) means vanilla refused an unchanged,
    /// unloaded or out-of-height write. Notification follows the supplied update flags.
    /// Inside a transaction in this level, replacement effects wait for that level's
    /// outermost commit. The server uses the block entity as it was at replacement,
    /// including earlier edits. A direct write joins its resident row's drops before removing
    /// the row and notifying neighbors. A join listener can replace the same position again
    /// without dropping stacks that already belong to the first replacement's drops.
    /// The first write only removes the original row; it leaves the listener's replacement
    /// block entity, its contents and its saved data intact, even at the same block state.
    /// A new block entity does not inherit the original's saved tag, including its lock.
    /// A strict bulk edit at the same position does not change which row this write removes.
    /// Aborting restores the original state and undoes those edits.
    fn level_set_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        flags: BlockUpdateFlags,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// The state `state` takes at `pos` once it is fitted to the blocks around it
    /// (`Block.updateFromNeighbourShapes`): a fence gets its connections, a stair its corner, a
    /// water-logged block its fluid tick. Nothing is written, so a state placed by code is
    /// fitted before `level_set_block` writes it; the ticks the neighbours' shapes ask for
    /// are queued, as vanilla's do. Neighbours that are not loaded read as air. Stale or
    /// unavailable levels and invalid states return typed errors; a position outside the
    /// world or not loaded gives `state` back unchanged.
    fn level_update_from_neighbour_shapes(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
    ) -> Result<BlockState, LevelWriteError>;
    /// Side: server.
    /// Ask `Block.getStateForPlacement(BlockPlaceContext)` without placing the block.
    /// The placement-state parts see `block`'s default state, as in a real placement.
    /// A registered answer belonging to another block is returned as given; an unknown
    /// answer falls back to the block's own rule, as in a real placement.
    /// The context is taken as written, with no target or click re-resolution, except that a
    /// `PlacementSource::Directional` context's `replace_clicked` is worked out from the block
    /// at `hit_pos`, as `DirectionalPlaceContext.replacingClickedOnBlock()` is. Nothing is
    /// written, notified or loaded. No additional survival, obstruction or permission check
    /// from `BlockItem.place` runs.
    /// Only the level in play is queried. Returns `Err(PlacementStateError::Stale)` for a
    /// stale handle or closed level, and `Err(PlacementStateError::Unavailable)` for a parked
    /// level, an unavailable host or load phase, or a state that the API cannot represent.
    /// A directional question vanilla throws on is refused, not answered:
    /// `Err(PlacementStateError::NoVerticalLook)` when the block's own rule reads the
    /// placer's vertical look (pointed dripstone), and
    /// `Err(PlacementStateError::ReplacingClickedCycle)` when the clicked block's
    /// `canBeReplaced` asks `replacingClickedOnBlock` (a single slab or a snow layer clicked
    /// with its own item), which is settled before any Part is asked. A Part's answer
    /// replaces the block's rule, so a block whose Part answers is never `NoVerticalLook`.
    ///
    /// Returns `Ok(None)` for vanilla's `null`, a part's `Some(None)`, an unknown block
    /// or item key, an unloaded or out-of-bounds destination, or a non-finite hit or
    /// player rotation. `context.source` selects between player (`BlockPlaceContext`)
    /// and directional (`DirectionalPlaceContext`) placement rules. `None` in
    /// `context.item` means an empty hand.
    fn level_state_for_placement(
        &mut self,
        level: LevelRef,
        block: BlockKey,
        context: &PlacementContext,
    ) -> Result<Option<BlockState>, PlacementStateError>;
    /// Side: server.
    /// Write `state` at `pos` as a placement does and run `Block.setPlacedBy` for it, with no
    /// player: a bed places its head, a door its upper half, a tall plant its top. The write
    /// is `level_set_block`'s with `NEIGHBORS | CLIENTS` (plus `KNOWN_SHAPE` when
    /// `update_neighbour_shapes` is false; without it the neighbours' shapes update, so fences
    /// and walls connect). `setPlacedBy` gets no placer and no item. Nothing else of
    /// `BlockItem.place` runs: no survival or obstruction check, item placement sound, placement
    /// game event or advancement. A carrier-transfer fence can refuse a direct placement write
    /// (the origin, a companion or a callback write) with `Ok(false)`; neighbour updates follow
    /// the level's normal rules. A completed wither pattern in an open Host transaction returns
    /// `LevelWriteError::InTransaction`.
    fn level_place_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        update_neighbour_shapes: bool,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Run `Block.setPlacedBy` on the block currently at `pos`, without a placer. `stack` is
    /// a detached item value (`None` is the empty stack); the block may inspect its components.
    /// Write the initial state and load any block entity data before calling. Vanilla's part can
    /// write companion cells and produce block-specific effects, but does not load stack NBT.
    /// The block's subscribed `BlockPart::set_placed_by` runs after vanilla's work, with
    /// the same read-only stack and no placer. This does not notify the item's part.
    /// `Ok(false)` at an unloaded or out-of-height origin or on carrier-transfer refusal.
    /// The fence check covers the origin and cells the callback writes directly, including
    /// companions; neighbour updates follow the level's normal rules.
    /// Stale or unavailable levels and invalid stacks return typed errors before any effects.
    /// A placement that would summon a wither is refused while a Host transaction is open.
    #[read_stack]
    fn level_set_placed_by(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        stack: Option<&ItemStack>,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Remove a block in place (`Level.removeBlock`), preserving its fluid.
    /// In a Host transaction the removal is tentative and its effects wait for commit,
    /// with the same block-entity rules as [`LevelRef::set_block`].
    fn level_remove_block(&mut self, level: LevelRef, pos: [i32; 3], moved: bool) -> bool;
    /// Side: server.
    /// Destroy a block in the level in place (`Level.destroyBlock(pos, drop, breaker, maxUpdates)`),
    /// optionally dropping resources with `tool` (None is an empty hand) and `breaker` as the
    /// loot's `this` entity and the `minecraft:block_destroy` game event's source. With `collect`,
    /// return the dropped stacks without spawning their item entities; otherwise return an empty
    /// list after spawning them. The one level drop path runs loot, block-drop hooks and
    /// experience once, in vanilla's random order. `drop` false suppresses resources entirely.
    /// `Ok(None)` means nothing broke: air, a refused write, a stale or foreign `breaker`, a
    /// stale handle, a client, an unloaded/out-of-height position, or a parked level. In a
    /// transaction in the block's level, loot is prepared against the old block, the new state
    /// is immediately readable, and spawning and break effects are deferred to commit. Another
    /// level's transaction alone does not defer the operation.
    fn level_destroy_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        drop: bool,
        tool: Option<&ItemStack>,
        breaker: Option<EntityRef>,
        collect: bool,
    ) -> Result<Option<Vec<ItemStack>>, StackWriteError>;
    /// Side: server.
    /// Drop the resources of the block at `pos` without breaking it
    /// (`Block.dropResources(state, level, pos, blockEntity, entity, tool)`): what
    /// `destroy_block` drops, with `tool` (None is an empty hand) and `breaker` as the loot's
    /// `this` entity, the block entity's contribution (a shulker box's contents), the
    /// `block_drops` hook and the block's experience (`Block.spawnAfterBreak`), under the
    /// `doTileDrops` game rule. Unlike `destroy_block` it sends no break particles or sound
    /// (level event 2001), leaves the block in place and posts no `minecraft:block_destroy` game
    /// event: a mod that breaks many blocks at once sends the effect once, drops each block's
    /// resources, then removes the blocks and posts the game events itself. With `collect`,
    /// return the stacks without spawning their item entities; otherwise an empty list after
    /// spawning them. `Ok(None)` for air, a stale or foreign `breaker`, a stale handle, a
    /// client, an unloaded or out-of-height position, or a parked level. In a transaction in
    /// the block's level, the loot is rolled
    /// now and the spawns wait for commit, as `destroy_block`'s do.
    fn level_drop_resources(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
        breaker: Option<EntityRef>,
        collect: bool,
    ) -> Result<Option<Vec<ItemStack>>, StackWriteError>;
    /// Side: server.
    /// What the block at `pos` would drop (`Block.getDrops(state, level, pos, blockEntity,
    /// entity, tool)`), without breaking it or spawning anything: its loot table rolled with
    /// `tool` (None is an empty hand) and `entity` as the breaker, with what its block entity
    /// copies into the drops (a shulker box's contents, a banner's patterns, a pot's sherds).
    /// The roll is given the block's centre as ORIGIN and its block entity as BLOCK_ENTITY,
    /// saved as `BlockEntity.saveWithFullMetadata` gives it (stored components included), so
    /// a `minecraft:match_block` tests the block state, then its NBT and components against
    /// that entity. A `minecraft:location_check` tests this level at the centre moved by its
    /// offsets (`LocationCheck.test`): its biome, structure, smoke, light, block and fluid
    /// tests fail where that position isn't loaded (`LocationPredicate.matches`).
    /// The roll draws from the table's random sequence, else the level random, as vanilla's
    /// does, so it moves later draws: that is why this is a write. No `block_drops` event,
    /// item entity or experience: that is `destroy_block`. None for a refused write, a stale or
    /// parked handle, a client, an unloaded or out-of-height position, and a stale `entity`.
    fn level_block_drops(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
        entity: Option<EntityRef>,
    ) -> Result<Option<Vec<ItemStack>>, StackWriteError>;
    /// Side: both.
    /// Apply bone meal to the block at `pos` as a dispenser does (`BoneMealItem.growCrop`, then
    /// `growWaterPlant`), with no player and no item: the mods' `bonemeal` hook first, level event
    /// 1505 when something grew. True when something grew; false for a refused write, a stale or
    /// parked handle, a client, and a block that did not grow.
    fn level_bone_meal(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Set or clear the mod-published connection group at `pos` (any `u64`, including `0`; `None`
    /// clears). Connected-texture rules with `"connect_group": true` join two neighbours only when
    /// both hold the same group; blocks with no group join nothing.
    ///
    /// Returns `Ok(true)` if the stored group changed, or `Ok(false)` if it was already `group`,
    /// `pos` is in an unloaded chunk or outside the level's vertical bounds, or the cell is
    /// currently locked by an in-flight carrier transfer fence. Refuses with
    /// `Err(LevelWriteError::Stale)` for a stale handle and `Err(LevelWriteError::Unavailable)`
    /// on a client, parked level, or outside the world phase.
    ///
    /// Connection groups are derived visual state and **never saved to disk**. A negotiated
    /// client receives the cell's current authoritative block state with its group, including
    /// in viewed levels. If only the group would be sent, a current block update goes first in
    /// the same bundle, only to negotiated clients. To keep block-entity-backed groups across
    /// reloads, republish them from `block_entity_on_load` (when `HostSide::Server`). Otherwise,
    /// record loaded chunks in `chunk_load` and publish on the next server tick. Changing a
    /// block's state properties on the same block (or weathering a copper chest / copper golem
    /// statue) preserves the group; replacing it with a different block (or air) clears it
    /// automatically.
    ///
    /// Participates in open `Host` transactions: tentative writes are visible immediately,
    /// synced on root commit, and restored silently on abort.
    fn level_set_connection_group(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        group: Option<u64>,
    ) -> Result<bool, LevelWriteError>;
}
