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
    /// The contextual friction of `state` at `pos` (`IBlockExtension.getFriction`).
    /// The state need not occupy the position. `entity` is the optional id and type
    /// passed to the block's part. Returns `Ok(None)` for a nested query that repeats
    /// an active read or exceeds the nesting bound. A stale level, unknown state, or
    /// unavailable host or phase returns a typed read error.
    /// The query never loads chunks or changes the world.
    fn level_friction(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        entity: Option<(u32, &str)>,
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
    fn level_block_property(&self, level: LevelRef, pos: [i32; 3], key: &str) -> Option<String>;
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
    /// The context is taken as written, with no target or click re-resolution. Nothing is
    /// written, notified or loaded. No additional survival, obstruction or permission check
    /// from `BlockItem.place` runs.
    /// Only the level in play is queried. Returns `Err(LevelReadError::Stale)` for a
    /// stale handle or closed level, and `Err(LevelReadError::Unavailable)` for a parked
    /// level, an unavailable host or load phase, or a state that the API cannot represent.
    ///
    /// Returns `Ok(None)` for vanilla's `null`, a part's `Some(None)`, an unknown block
    /// or item key, an unloaded or out-of-bounds destination, or a non-finite hit or
    /// rotation. `None` in `context.item` means an empty hand.
    fn level_state_for_placement(
        &mut self,
        level: LevelRef,
        block: BlockKey,
        context: &PlacementContext,
    ) -> Result<Option<BlockState>, LevelReadError>;
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
    /// Destroy a block in the level in place (`Level.destroyBlock`), optionally dropping
    /// resources with `tool` (None is an empty hand). With `collect`, return the dropped
    /// stacks without spawning their item entities; otherwise return an empty list after
    /// spawning them. The one level drop path runs loot, block-drop hooks and experience
    /// once, in vanilla's random order. `drop` false suppresses resources entirely.
    /// `Ok(None)` means nothing broke: air, a refused write, a stale handle, a client, an
    /// unloaded/out-of-height position, a parked level, or an open Host transaction.
    fn level_destroy_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        drop: bool,
        tool: Option<&ItemStack>,
        collect: bool,
    ) -> Result<Option<Vec<ItemStack>>, StackWriteError>;
    /// Side: server.
    /// What the block at `pos` would drop (`Block.getDrops(state, level, pos, blockEntity,
    /// entity, tool)`), without breaking it or spawning anything: its loot table rolled with
    /// `tool` (None is an empty hand) and `entity` as the breaker, with what its block entity
    /// copies into the drops (a shulker box's contents, a banner's patterns, a pot's sherds).
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
