//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `block_entities` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait BlockEntitiesHost {
    /// Side: both.
    /// Find a block entity in a loaded chunk of an open level, or on a client in
    /// the level it shows. Tick and menu callbacks keep the entity resident and readable until removal.
    fn block_entity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef>;
    /// Side: both.
    /// The handle [`BlockEntitiesHost::block_entity`] gives, and also for a block entity
    /// temporarily taken out by an engine operation. Its identity and attachments stay in place,
    /// so its Part state (`BlockEntityPartState`) can still check its handle even though
    /// data reads find nothing until the row is put back.
    fn block_entity_identity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef>;
}
/// The handles' reads of the `block_entities` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait BlockEntitiesBackend {
    /// Side: both.
    /// The block entity's type (`BlockEntity.getType`).
    fn block_entity_kind(&self, entity: BlockEntityRef) -> Option<BlockEntityTypeKey>;
    /// Side: both.
    /// The block entity's position.
    fn block_entity_position(&self, entity: BlockEntityRef) -> Option<[i32; 3]>;
    /// Side: both.
    /// The block entity's level.
    fn block_entity_level(&self, entity: BlockEntityRef) -> Option<LevelRef>;
    /// Side: server.
    /// The block entities of the named open server level, active or parked, whose positions lie
    /// in the box from `min` (inclusive) to `max` (exclusive) on each axis and within the level's
    /// build height (`LevelHeightAccessor.isInsideBuildHeight`), in no order. Only chunks
    /// already loaded are read (`ServerChunkCache.getChunkNow`), from their live entries
    /// (`LevelChunk.getBlockEntities`): nothing is loaded or generated, and saved entries not
    /// yet made live are not listed. A moving piston's (`minecraft:piston`) is listed, and one taken
    /// out for its tick is still listed with its own type. Empty for a box
    /// with no block entity, including one of zero volume, one outside the build height and one
    /// that meets no loaded chunk; any corners are valid, `i32::MIN` and `i32::MAX` included,
    /// and there is no cap on the chunks a box covers. None for reversed bounds (`min` above
    /// `max` on an axis), a stale handle, or on a client.
    fn level_block_entities(
        &self,
        level: LevelRef,
        min: [i32; 3],
        max: [i32; 3],
    ) -> Option<Vec<BlockEntityAt>>;
    /// Side: both.
    /// An owned snapshot of the data saved without identity or position metadata
    /// (`BlockEntity.saveWithoutMetadata`). The save payload is assembled on request.
    /// A mod holder without declared inventory slots returns saved `Items` as opaque
    /// NBT in its original list order, even if entries look like canonical stacks.
    fn block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound>;
    /// Side: both.
    /// Borrows the vanilla lock's item predicate for the lifetime of the Host borrow.
    /// `None` for an absent lock or an unavailable block entity; a stored empty predicate
    /// remains present. Use `set_lock` to replace or clear the whole predicate.
    fn block_entity_lock(&self, entity: BlockEntityRef) -> Option<&Compound>;
    /// Side: server.
    /// The custom name (`Nameable.getCustomName`) as plain text. None when the block entity
    /// has none or isn't nameable (chests, barrels, furnaces, hoppers, brewing stands,
    /// banners, beacons, enchanting tables, command blocks...), or for a stale handle or a
    /// client.
    fn block_entity_name(&self, entity: BlockEntityRef) -> Option<String>;
    /// Side: server.
    /// The players whose open menu holds this block entity's container (for openers'
    /// containers, `ContainerOpenersCounter.getEntitiesWithContainerOpen`'s players), from the menus the opener counts read
    /// (`tick_mod_container_viewers`, `ChestBlockEntity$1.isOwnContainer`). Guests in
    /// its level, not spectators nor fake players, then the acting player for a menu
    /// their action opened, which the hub records when the action ends. None for a
    /// stale handle or a client.
    fn block_entity_viewers(&self, entity: BlockEntityRef) -> Option<Vec<PlayerRef>>;
    /// Side: both.
    /// A snapshot of a mod block entity's menu data slots, on the server or in the
    /// client's copy. None for a stale/missing/vanilla entity. Item storage is separate.
    fn block_entity_data_slots(&self, entity: BlockEntityRef) -> Option<Vec<i32>>;
    /// Side: both.
    /// Read a configured mod holder's declared internal item slot, without automation policy.
    /// A read cannot roll a loot table still waiting in the container: on the server, call
    /// `unpack_loot_table` first, as vanilla's `getItem` unpacks before it reads.
    /// Removed, replaced or reloaded holders return BlockEntityItemError::Stale.
    fn block_entity_item_slot(
        &self,
        entity: BlockEntityRef,
        slot: u32,
    ) -> Result<Option<ItemStack>, BlockEntityItemError>;
}
/// The handles' writes of the `block_entities` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait BlockEntitiesBackendMut {
    /// Side: server.
    /// `ServerLevel.sendBlockUpdated`: send the holder's block and update tag
    /// (including one-shot client update fields) on its level's next block broadcast.
    /// Requests before that broadcast coalesce. Call when the display changes, not
    /// for progress carried by a menu's data slots. Works during the holder's own tick
    /// and in parked levels. Returns `Stale` after replacement or unload, or
    /// `Unavailable` when server world access is unavailable.
    fn block_entity_sync(&mut self, entity: BlockEntityRef) -> Result<(), LevelWriteError>;
    /// Side: server.
    /// Change the lock through the same storage used by container opening and saving.
    /// The lock belongs to this block entity, not to the position or its previous occupant.
    fn block_entity_set_lock(
        &mut self,
        entity: BlockEntityRef,
        lock: Option<Compound>,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Merges `data` into the block entity's saved data and loads it, as `/data merge block`
    /// does (`BlockDataAccessor.setData`: `loadWithComponents`, `setChanged`, then
    /// `sendBlockUpdated`, so comparators re-read it and clients get the update). `id`, `x`,
    /// `y` and `z` in `data` are ignored. Ok(true) when data changed; Ok(false) for data
    /// nested too deep, no change, or data the entity cannot load. Stale or unwritable
    /// handles return BlockEntityError::Stale. Inside a transaction an
    /// abort puts the block entity, its name, `lock`, loot table roll and attachments back.
    fn block_entity_merge_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Replaces the saved payload, retaining identity and position metadata.
    /// Uses the same transactional load and client update path as `merge_data`.
    /// Ok(false) for unsupported data or no change; Stale for an invalidated handle.
    fn block_entity_replace_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// `RandomizableContainer.setLootTable(table, seed)` (NeoForge mods call the same vanilla
    /// method) on a mod block entity whose block declares container slots: the table and seed
    /// are kept with it and saved as its `LootTable` and `LootTableSeed`, which `/data` and
    /// `data` show, and nothing is rolled yet. The first time a player is admitted to its
    /// menu (a spectator is refused while a table waits, as
    /// `RandomizableContainerBlockEntity.canOpen` refuses one), a hopper or other container
    /// access reads it, [`Self::block_entity_unpack_loot_table`] runs, or the block is broken
    /// (`RandomizableContainer.unpackLootTable`), the table rolls once into its empty slots
    /// with `LootTable.fill`'s slot shuffle, from `RandomSource.create(seed)`, the table's
    /// random sequence for a seed of 0, else the level random, with the container's centre
    /// as `ORIGIN` and the opening player as `THIS_ENTITY` and for its luck; then the table
    /// is gone. Stacks already in the container keep their slots: the fill shuffles and
    /// fills only the empty ones, as vanilla's does. Two worlds rolling the same table and
    /// seed into the same contents fill the same slots. A saved container with a table loads
    /// the table, not its `Items` (`tryLoadLootTable`). Ok(false) for a block entity with no
    /// declared slots, a table that is not an identifier, or the table and seed it already
    /// waits with (`chests/x` is `minecraft:chests/x`); Stale for an invalidated or
    /// unwritable handle. Inside a transaction an abort puts the previous table back.
    fn block_entity_set_loot_table(
        &mut self,
        entity: BlockEntityRef,
        table: &str,
        seed: i64,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// `RandomizableContainer.unpackLootTable(null)`, what vanilla's `getItem`, `setItem`,
    /// `removeItem` and `isEmpty` run before they touch a container's slots: a table still
    /// waiting (from [`Self::block_entity_set_loot_table`], a save or a structure) rolls into
    /// the container's empty slots now, with no player, as
    /// `block_entity_set_loot_table` describes. Reads through `&dyn Host`
    /// (`item_slot`) and `set_item_slot` don't roll it: call this first when the mod reads or
    /// writes the slots of a container that may still hold a table. Ok(true) when a table
    /// rolled, Ok(false) when none waited; Stale for an invalidated or unwritable handle.
    /// Inside a transaction an abort puts the table and the slots back.
    fn block_entity_unpack_loot_table(
        &mut self,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// [`Self::block_entity_merge_data`] without the send: the data is merged and loaded,
    /// then `BlockEntity.setChanged` marks the chunk unsaved and notifies comparators, and no
    /// `Level.sendBlockUpdated` follows. Clients keep what they last got until
    /// `block_entity_sync`, so a machine whose state changes every tick sends when it
    /// chooses, as a vanilla or NeoForge block entity calling `setChanged()` and
    /// `Level.sendBlockUpdated` itself does. A block entity whose own `setChanged` sends is
    /// still sent: a vanilla shelf's (`ShelfBlockEntity.setChanged`) calls
    /// `sendBlockUpdated` itself. Same results, errors and transaction rules as
    /// `merge_data`.
    fn block_entity_merge_data_quiet(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// [`Self::block_entity_replace_data`] without the send, as
    /// [`Self::block_entity_merge_data_quiet`] is to `merge_data`: `setChanged` only, the
    /// client update left to `block_entity_sync`.
    fn block_entity_replace_data_quiet(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// `BlockEntity.setChanged`: mark the chunk and notify comparators immediately, even
    /// inside a transaction and for equal contents. Aborting storage does not undo an issued
    /// notification. Returns Stale for an invalidated handle or a level not writable here.
    fn block_entity_set_changed(
        &mut self,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Watch one source position in this observer's level. The observer's BlockParts
    /// receive the same contents_changed notice with observer = Some(self). Repeated
    /// registration is idempotent; a self-watch never duplicates its own notice.
    /// Registration changes join the transaction and abort restores them. Removal,
    /// replacement and chunk unload discard watches; loading must register them again.
    /// The source need not be loaded; watches follow the position, not its occupant.
    /// No part subscribed to this moment returns NotSubscribed; an invalid observer
    /// or a level not writable here returns Stale. Available during the observer's tick.
    fn block_entity_watch_contents(
        &mut self,
        entity: BlockEntityRef,
        source: BlockPos,
        enabled: bool,
    ) -> Result<(), ContentsWatchError>;
    /// Side: server.
    /// Sets the custom name, or clears it with None, as an anvil-renamed item placed with
    /// `components` sets `CustomName`. Plain text only. Clients get a block entity update.
    /// Ok(false) for an entity that is not nameable; Stale for an invalidated or
    /// unwritable handle. Inside a transaction an abort puts the name back.
    fn block_entity_set_name(
        &mut self,
        entity: BlockEntityRef,
        name: Option<&str>,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Publishes the shapes this block entity's block has in place of its declared ones, or
    /// clears them with None, so the declared shapes apply again. The block must declare
    /// `shapes_from_block_entity`. Physics, selection, ray casts, support, the floor a mob's
    /// path stands on, its move control's jump, an evoker's fangs, a panicking mob's search for
    /// water and an iron golem's spawn check read the published value at the position
    /// and never call into the mod, so a port of NeoForge's `getShape` publishes wherever
    /// that state changes: it is published when it changes, not asked for per query.
    ///
    /// The server applies it at once, for the queries that follow in the same tick; a client
    /// applies it when the block entity's update arrives, which may be a tick later, as any
    /// block change. The game saves it with the block entity, restores it when the chunk
    /// loads and sends it with the block entity to clients; it is dropped when the block
    /// entity is removed or its block changes. An entity the new shape now overlaps is
    /// treated as vanilla treats a block that appears inside it, with no push-out. Ok(false)
    /// when nothing changed. Fluids read the collision shape too: water and lava pass between
    /// two cells by their collision shapes at their positions. A publish, a clear and an abort
    /// that puts the old shapes back each notify the block's neighbours, so a fluid beside it
    /// reads the new shape; an unchanged value notifies nothing. The checks are those of
    /// `BlockEntityShapes`; a shape that fails one is refused by name.
    fn block_entity_set_shapes(
        &mut self,
        entity: BlockEntityRef,
        shapes: Option<BlockEntityShapes>,
    ) -> Result<bool, BlockEntityShapesError>;
    /// Side: server.
    /// Set a mod block entity's transient menu data slot in place, joining the current
    /// transaction. These slots are not serialized, and writing them does not mark the
    /// chunk unsaved or issue any notification; keep persistent values in the block
    /// entity's NBT and call `set_changed` explicitly when persisted state changes.
    /// `Ok(false)` for a vanilla entity, invalid slot or no change. Does not resize slots.
    fn block_entity_set_data_slot(
        &mut self,
        entity: BlockEntityRef,
        slot: u32,
        value: i32,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Sizes a mod block entity's transient menu data explicitly, with no 256-slot cap,
    /// joining the current transaction. Writing transient menu data does not mark the
    /// chunk unsaved or issue any notification; call `set_changed` explicitly when
    /// persisted state changes. Removed, replaced or reloaded holders return
    /// `BlockEntityDataError::Stale`.
    fn block_entity_set_data_slots(
        &mut self,
        entity: BlockEntityRef,
        values: Vec<i32>,
    ) -> Result<bool, BlockEntityDataError>;
    /// Side: server.
    /// Replace internal storage under the innermost transaction; ITEMS routing is separate.
    /// A mismatched `tx` returns `BlockEntityItemError::InvalidContext`. A committed change marks
    /// the chunk unsaved but sends no notification. Call set_changed explicitly to notify.
    /// Removed, replaced or reloaded holders return `BlockEntityItemError::Stale`.
    fn block_entity_set_item_slot(
        &mut self,
        entity: BlockEntityRef,
        tx: &TransactionContext,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<bool, BlockEntityItemError>;
    /// Side: server.
    /// Removes this block entity from its level without changing its block or state and
    /// without running `pre_remove_side_effects` (`Level.removeBlockEntity`,
    /// `LevelChunk.removeBlockEntity`, `BlockEntity.setRemoved`).
    ///
    /// Accepts any resident block entity (vanilla or mod) in a writable server level.
    /// Unregisters its game-event listener (sculk sensors, calibrated sensors, shriekers,
    /// catalysts), runs its `setRemoved` effects (a jukebox stops playing; a beacon plays
    /// its deactivation sound), retires its ticker, attachments, custom name, lock, and
    /// contents watches, and notifies neighbours if a mod block entity's published shapes
    /// disappeared. Sends no network packet and does not mark the chunk unsaved.
    ///
    /// Change the block after removing if it still carries a block entity type; otherwise
    /// a fresh, empty entity is created on the next access or reload.
    ///
    /// Inside a transaction, tentative reads immediately see the entity as gone and this
    /// handle as stale; `setRemoved` effects (listener unregister, jukebox stop, beacon
    /// sound, neighbour updates) are deferred to root commit and never happen on abort,
    /// which restores the entity, generation, attachments, custom name, lock, watches,
    /// and published shapes. A subsequent `set_block` at the same position in the same
    /// transaction does not run `pre_remove_side_effects` for the already-removed entity.
    ///
    /// Returns `Err(BlockEntityError::Stale)` when this handle is stale, unloaded,
    /// temporarily taken out by an engine operation, on a client, or in a level not
    /// writable here.
    fn block_entity_remove(&mut self, entity: BlockEntityRef) -> Result<(), BlockEntityError>;
}
