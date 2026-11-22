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
/// The server host's methods of the `block_entities` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait BlockEntitiesServerHost {
    /// Side: server.
    /// Sends clients the block at `pos` and its block entity's update tag
    /// (`block_entity_update_tag`) again, as `Level.sendBlockUpdated`
    /// does: with the level's other block changes, to the players who see its
    /// chunk, once however often it is called before then. Changes made in a
    /// block entity's tick go out at the start of the next tick, as vanilla's
    /// do (`ServerLevel.tick` broadcasts chunks before it ticks block
    /// entities). Call it when what clients show changed (a machine started),
    /// not for every tick's progress: a menu's data slots carry that. Nothing
    /// happens for an unloaded block. It also sends the one-shot fields of
    /// `BlockEntityView::set_client_update_fields`.
    fn block_entity_sync(&mut self, pos: [i32; 3]);
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
    /// Replace internal storage under the existing transaction; ITEMS routing is separate.
    /// A committed change marks the chunk unsaved but sends no notification. Call
    /// set_changed explicitly to notify. Removed, replaced or reloaded holders return
    /// BlockEntityItemError::Stale.
    fn block_entity_set_item_slot(
        &mut self,
        entity: BlockEntityRef,
        tx: &Transaction,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<bool, BlockEntityItemError>;
}
