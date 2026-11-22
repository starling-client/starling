//! Block entity handles: storage writes join the journal; explicit world notifications happen
//! immediately and survive abort. Raw mod storage writes and restores synthesize no notice.
use super::Game;
use mod_api::__private::game::*;
use mod_api::{
    BlockEntityDataError, BlockEntityError, BlockEntityItemError, BlockEntityRef,
    BlockEntityShapes, BlockEntityShapesError, BlockEntityTypeKey, LevelRef, PlayerRef,
    Transaction,
};
use sunrise_client::{
    block_entity_nbt,
    containers::{BlockEntity, ModBlockEntity},
    save,
    world::World,
};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_core::position::BlockPos;
use sunrise_items::items::ItemStack;
use sunrise_world::registry::BlockStateId;

impl Game<'_> {
    fn native_block_entity_target(&self, entity: BlockEntityRef) -> Option<(&World, &BlockEntity)> {
        let world = self.native_level_read_world(entity.fields().level)?;
        Some((world, resident_block_entity(world, entity)?))
    }

    /// Find a block entity in a loaded chunk of an open level, or on a client in
    /// the level it shows.
    pub(super) fn block_entity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
        let world = self.native_level_read_world(level)?;
        world_block_entity(world, level, pos)
    }

    /// `block_entity`, also for a block entity temporarily taken out by an engine operation:
    /// its generation stays in place while it is out.
    pub(super) fn block_entity_identity(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<BlockEntityRef> {
        let world = self.native_level_read_world(level)?;
        if !world.loaded(pos[0], pos[2]) {
            return None;
        }
        let generation = world.block_entities.generation(&(pos[0], pos[1], pos[2]))?;
        Some(mod_api::__private::game::block_entity_ref(
            level, pos, generation,
        ))
    }

    /// The data of the block entity at `pos`, as saved without identity or position
    /// metadata, for the hook that hears its block's removal: the entity is gone from
    /// the world by then, so it is read first. None for a block without one, or while
    /// no mod listens.
    pub(in crate::game) fn removed_block_entity_data(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<Compound> {
        if !mod_api::__private::listened(mod_api::HookId::PlayerDestroyedBlock) {
            return None;
        }
        let entity = loaded_block_entity(&self.world, pos)?;
        let block = &self.pack.assets.registry.state(state)?.name;
        save_without_metadata(
            &self.world,
            (pos[0], pos[1], pos[2]),
            entity,
            block,
            self.res.items(),
        )
    }

    /// The saved data of `entity` at `key` without identity or position metadata, with its
    /// `CustomName`, for a block `block`: what a handle to it reads.
    pub(in crate::game) fn saved_block_entity_data(
        &self,
        key: (i32, i32, i32),
        entity: &BlockEntity,
        block: &str,
    ) -> Option<Compound> {
        save_without_metadata(&self.world, key, entity, block, self.res.items())
    }

    /// Opens a block entity's menu for a connected player (`ServerPlayer.openMenu`).
    /// Returns Stale for an invalidated entity; Ok(false) for a wrong-level player,
    /// non-provider, cancelled open, or an unconnected player such as a fake player.
    pub(super) fn player_open_menu(
        &mut self,
        player: PlayerRef,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError> {
        self.native_block_entity_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        if self.player_handle_read(player).is_none() || !self.level_in_place(entity.fields().level)
        {
            return Ok(false);
        }
        let Some(conn) = self.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .iter()
                .find(|guest| {
                    guest.uuid == player.fields().uuid
                        && guest.placed
                        && guest.dimension == entity.fields().level.dimension_id()
                })
                .map(|guest| guest.conn)
        }) else {
            return Ok(false);
        };
        if self.menu_click_in_progress(conn) {
            return Ok(false);
        }
        let [x, y, z] = entity.fields().position;
        match self.open_block_menu(conn, (x, y, z)) {
            super::lan_actions::MenuOpen::Stale => Err(BlockEntityError::Stale),
            opened => Ok(opened == super::lan_actions::MenuOpen::Opened),
        }
    }

    /// The block entity's type (`BlockEntity.getType`).
    pub(super) fn block_entity_kind(&self, entity: BlockEntityRef) -> Option<BlockEntityTypeKey> {
        let (world, block_entity) = self.native_block_entity_target(entity)?;
        let [x, y, z] = entity.fields().position;
        let block = &self.pack.assets.registry.state(world.block(x, y, z))?.name;
        mod_api::__private::block_entity_type_key(save::block_entity_id(block_entity, block))
    }

    /// The block entity's position.
    pub(super) fn block_entity_position(&self, entity: BlockEntityRef) -> Option<[i32; 3]> {
        self.native_block_entity_target(entity)?;
        Some(entity.fields().position)
    }

    /// The block entity's level.
    pub(super) fn block_entity_level(&self, entity: BlockEntityRef) -> Option<LevelRef> {
        self.native_block_entity_target(entity)?;
        Some(entity.fields().level)
    }

    /// The data saved without identity or position metadata
    /// (`BlockEntity.saveWithoutMetadata`).
    pub(super) fn block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound> {
        let world = self.native_level_read_world(entity.fields().level)?;
        world_block_entity_data(world, self.pack.assets.registry, self.res.items(), entity)
    }

    /// The resident lock, independent of the block entity's synthesized save payload.
    pub(super) fn block_entity_lock(&self, entity: BlockEntityRef) -> Option<&Compound> {
        let (world, _) = self.native_block_entity_target(entity)?;
        let [x, y, z] = entity.fields().position;
        match world.block_lock((x, y, z))? {
            Tag::Compound(lock) => Some(lock),
            _ => None,
        }
    }

    /// Change the lock through the same storage used by container opening and saving.
    pub(super) fn block_entity_set_lock(
        &mut self,
        entity: BlockEntityRef,
        lock: Option<Compound>,
    ) -> Result<bool, BlockEntityError> {
        let (key, block) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        if matches!((self.world.block_lock(key), &lock), (None, None))
            || matches!((self.world.block_lock(key), &lock), (Some(Tag::Compound(old)), Some(new)) if old == new)
        {
            return Ok(false);
        }
        let Some(mut tag) = self.world.block_entities.get(&key).and_then(|current| {
            save_with_metadata(&self.world, key, current, &block, self.res.items())
        }) else {
            return Ok(false);
        };
        match lock {
            Some(lock) => {
                tag.insert("lock".into(), Tag::Compound(lock));
            }
            None => {
                tag.remove(&"lock".into());
            }
        }
        self.journal_block_entity(key);
        self.world.set_block_lock(key, &Tag::Compound(tag));
        self.block_entity_updated(key);
        self.container_changed(key);
        Ok(true)
    }

    /// A loaded block entity of the server level in place, when a write may change it.
    pub(super) fn native_block_entity_write_key(
        &self,
        entity: BlockEntityRef,
    ) -> Option<(i32, i32, i32)> {
        if !self.level_in_place(entity.fields().level) {
            return None;
        }
        let pos = entity.fields().position;
        self.native_block_entity_target(entity)?;
        Some((pos[0], pos[1], pos[2]))
    }

    /// The block entity's key and block name when a write may change it.
    fn native_block_entity_write_target(
        &self,
        entity: BlockEntityRef,
    ) -> Option<((i32, i32, i32), String)> {
        let key = self.native_block_entity_write_key(entity)?;
        let block = self
            .pack
            .assets
            .registry
            .state(self.world.block(key.0, key.1, key.2))?
            .name
            .clone();
        Some((key, block))
    }

    /// A snapshot of a mod block entity's menu data slots, on the server or in the
    /// client's copy. None for a stale/missing/vanilla entity. Item storage is separate.
    pub(super) fn block_entity_data_slots(&self, entity: BlockEntityRef) -> Option<Vec<i32>> {
        let (_, BlockEntity::Mod(block_entity)) = self.native_block_entity_target(entity)? else {
            return None;
        };
        Some(block_entity.data.clone())
    }

    /// Set a mod block entity's menu data slot silently in the current transaction.
    /// Ok(false) for a vanilla entity, invalid slot or equal value; Stale for an invalidated
    /// or unwritable handle. Notification is explicit; this does not resize slots.
    pub(super) fn block_entity_set_data_slot(
        &mut self,
        entity: BlockEntityRef,
        slot: u32,
        value: i32,
    ) -> Result<bool, BlockEntityError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityError::Stale)?;
        let Some(block_entity) = self.mod_block_entity_write_target(key) else {
            return Ok(false);
        };
        let index = slot as usize;
        if block_entity
            .data
            .get(index)
            .is_none_or(|current| *current == value)
        {
            return Ok(false);
        }
        self.write_mod_block_entity(key, |block_entity| block_entity.data[index] = value);
        Ok(true)
    }

    /// Sizes a mod block entity's transient menu data using the same silent
    /// write and journal as scalar data-slot writes.
    pub(super) fn block_entity_set_data_slots(
        &mut self,
        entity: BlockEntityRef,
        values: Vec<i32>,
    ) -> Result<bool, BlockEntityDataError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityDataError::Stale)?;
        let block_entity = self
            .mod_block_entity_write_target(key)
            .ok_or(BlockEntityDataError::Unavailable)?;
        if block_entity.data == values {
            return Ok(false);
        }
        self.write_mod_block_entity(key, |block_entity| block_entity.data = values);
        Ok(true)
    }

    fn mod_block_entity_write_target(&self, key: (i32, i32, i32)) -> Option<&ModBlockEntity> {
        let BlockEntity::Mod(block_entity) = self.world.block_entities.get(&key)? else {
            return None;
        };
        Some(block_entity)
    }

    // The target was resolved above, and journaling invokes no listener that could replace it.
    fn write_mod_block_entity(
        &mut self,
        key: (i32, i32, i32),
        edit: impl FnOnce(&mut ModBlockEntity),
    ) {
        self.journal_block_entity(key);
        self.edit_mod_block_entity(key, edit);
    }

    /// `edit` on a resolved mod holder; the caller journaled the storage. Notification is the
    /// mod's explicit responsibility, including any notice it wants while restoring storage.
    fn edit_mod_block_entity(
        &mut self,
        key: (i32, i32, i32),
        edit: impl FnOnce(&mut ModBlockEntity),
    ) {
        let BlockEntity::Mod(block_entity) = self.world.block_entities.get_mut(&key).unwrap()
        else {
            unreachable!("resolved mod holder replaced before its write")
        };
        edit(block_entity);
    }

    /// Read a configured mod holder's declared internal item slot, without automation policy.
    pub(super) fn block_entity_item_slot(
        &self,
        entity: BlockEntityRef,
        slot: u32,
    ) -> Result<Option<ItemStack>, BlockEntityItemError> {
        let (world, BlockEntity::Mod(block_entity)) = self
            .native_block_entity_target(entity)
            .ok_or(BlockEntityItemError::Stale)?
        else {
            return Err(BlockEntityItemError::Unavailable);
        };
        let [x, y, z] = entity.fields().position;
        mod_item_slots(world, (x, y, z), slot)?;
        Ok(block_entity.items.get(slot as usize).cloned().flatten())
    }

    /// Replace internal storage under the existing transaction; external ITEMS routing is separate.
    pub(super) fn block_entity_set_item_slot(
        &mut self,
        entity: BlockEntityRef,
        tx: &Transaction,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<bool, BlockEntityItemError> {
        if !self.transaction_holds(tx) {
            return Ok(false);
        }
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityItemError::Stale)?;
        let block_entity = self
            .mod_block_entity_write_target(key)
            .ok_or(BlockEntityItemError::Unavailable)?;
        let slots = mod_item_slots(&self.world, key, slot)?;
        if let Some(stack) = &stack {
            if stack.count == 0
                || stack.item.as_ref() == "minecraft:air"
                || self.items().get(&stack.item).is_none()
            {
                return Err(BlockEntityItemError::InvalidStack);
            }
            let limit = self.stack_max_size(stack);
            if stack.count > limit {
                return Err(BlockEntityItemError::StackTooLarge {
                    count: stack.count,
                    limit,
                });
            }
        }
        let index = slot as usize;
        if block_entity.items.get(index).and_then(Option::as_ref) == stack.as_ref()
            && !block_entity.has_opaque_item_slot(index)
        {
            return Ok(false);
        }
        // A slot of a list already its full length: the journal keeps that slot, not the
        // entity's whole saved data. A list still to grow is kept whole, length and all.
        if index < block_entity.items.len() {
            self.journal_block_entity_slot(key, index);
            self.edit_mod_block_entity(key, |block_entity| {
                *block_entity.item_slot_mut(index).unwrap() = stack;
            });
        } else {
            self.write_mod_block_entity(key, |block_entity| {
                block_entity.items.resize(slots, None);
                *block_entity.item_slot_mut(index).unwrap() = stack;
            });
        }
        self.journal_chunk_save((key.0 >> 4, key.2 >> 4));
        Ok(true)
    }

    /// Merges `data` into the block entity's saved data and loads it, as `/data merge block`
    /// does (`BlockDataAccessor.setData`: `loadWithComponents`, `setChanged`, then
    /// `sendBlockUpdated`, so comparators re-read it and clients get the update). `id`, `x`,
    /// `y` and `z` in `data` are ignored. Ok(true) when data changed; Ok(false) for data
    /// nested too deep, no change, or data the entity cannot load. Stale or unwritable
    /// handles return BlockEntityError::Stale. Inside a transaction an
    /// abort puts the block entity, its name, `lock`, loot table roll and attachments back.
    pub(super) fn block_entity_merge_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError> {
        self.write_block_entity_data(entity, data, false)
    }

    /// Replaces the saved payload, retaining identity and position metadata.
    /// Uses the same transactional load and client update path as `merge_data`.
    /// Ok(false) for unsupported data or no change; Stale for an invalidated handle.
    pub(super) fn block_entity_replace_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError> {
        self.write_block_entity_data(entity, data, true)
    }

    fn write_block_entity_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
        replace: bool,
    ) -> Result<bool, BlockEntityError> {
        let (key, block) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        let nbt = Tag::Compound(mod_payload(data));
        if sunrise_client::nbt_path::too_deep(&nbt, 0) {
            return Ok(false);
        }
        let Some(old) = self
            .world
            .block_entities
            .get(&key)
            .and_then(|current| {
                save_with_metadata(&self.world, key, current, &block, self.res.items())
            })
            .map(Tag::Compound)
        else {
            return Ok(false);
        };
        let merged = loaded_payload(&old, &nbt, replace);
        if merged == old {
            return Ok(false);
        }
        self.journal_block_entity(key);
        let cleared = matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Mod(current)) if current.shapes_cleared()
        );
        if !self.load_block_entity_data(key, &merged) {
            return Ok(false);
        }
        if cleared && let Some(BlockEntity::Mod(loaded)) = self.world.block_entities.get_mut(&key) {
            loaded.keep_shapes_cleared(true);
        }
        self.block_entity_updated(key);
        self.container_changed(key);
        Ok(true)
    }

    /// `BlockEntity.setChanged`: notify at once, including inside a transaction. An abort
    /// restores storage, not notifications already delivered to observers or comparators.
    pub(super) fn block_entity_set_changed(
        &mut self,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityError::Stale)?;
        self.container_changed(key);
        Ok(true)
    }

    /// Register a generation-bound observer in its level's keyed contents index.
    pub(super) fn block_entity_watch_contents(
        &mut self,
        entity: BlockEntityRef,
        source: mod_api::BlockPos,
        enabled: bool,
    ) -> Result<(), mod_api::ContentsWatchError> {
        let fields = entity.fields();
        if !self.level_in_place(fields.level)
            || self.block_entity_identity(fields.level, fields.position) != Some(entity)
        {
            return Err(mod_api::ContentsWatchError::Stale);
        }
        let [x, y, z] = fields.position;
        if !crate::game::api_values::block_state::block_listens(
            mod_api::HookId::ContentsChanged,
            self.world.block(x, y, z),
        ) {
            return Err(mod_api::ContentsWatchError::NotSubscribed);
        }
        self.journal_block_entity((x, y, z));
        let observer = self.world.block_entities.identity((x, y, z)).unwrap();
        self.world.block_entities.contents_watches.set(
            observer,
            (source.x, source.y, source.z),
            enabled,
        );
        Ok(())
    }

    /// Publishes the shapes the block entity's block has in place of its declared ones, or
    /// clears them with None. Applied at once, to the queries that follow, and the block's
    /// neighbours are notified; clients get them with the block entity update. Inside a
    /// transaction an abort puts the old ones back.
    pub(super) fn block_entity_set_shapes(
        &mut self,
        entity: BlockEntityRef,
        shapes: Option<BlockEntityShapes>,
    ) -> Result<bool, BlockEntityShapesError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityShapesError::Stale)?;
        let state = self.world.block(key.0, key.1, key.2);
        if !self.collision.publishes_shapes(state) {
            return Err(BlockEntityShapesError::NotDeclared);
        }
        let shapes = shapes
            .map(sunrise_client::containers::PublishedShapes::new)
            .transpose()?
            .map(std::sync::Arc::new);
        let Some(current) = self.mod_block_entity_write_target(key) else {
            return Err(BlockEntityShapesError::Stale);
        };
        if current.shapes() == shapes.as_deref() {
            return Ok(false);
        }
        if cfg!(debug_assertions)
            && shapes
                .as_ref()
                .is_some_and(|shapes| shapes.reaches_past_one_cell())
        {
            static WARNED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
            let block = self
                .pack
                .assets
                .registry
                .state(state)
                .map(|s| s.name.clone());
            if let (Some(block), Ok(mut warned)) = (block, WARNED.lock())
                && !warned.contains(&block)
            {
                log::warn!(
                    "{block}: a published shape has a box beyond one block of the block; collision never finds it"
                );
                warned.push(block);
            }
        }
        self.write_mod_block_entity(key, |block_entity| {
            block_entity.set_shapes(shapes);
        });
        self.block_entity_updated(key);
        // Fluids and other neighbours read the shape: tell them it changed (the shulker lid's
        // `doNeighborUpdates`, without its shape updates).
        let at = BlockPos::new(key.0, key.1, key.2);
        self.with_level(|b, level, ticks| {
            let block = b.block(state);
            b.update_neighbors_at(level, ticks, at, block);
        });
        Ok(true)
    }

    /// The custom name (`Nameable.getCustomName`) as plain text. None when the block entity
    /// has none or isn't nameable (chests, barrels, furnaces, hoppers, brewing stands,
    /// banners, beacons, enchanting tables, command blocks...), or for a stale handle or a
    /// client.
    pub(super) fn block_entity_name(&self, entity: BlockEntityRef) -> Option<String> {
        let (world, block_entity) = self.native_block_entity_target(entity)?;
        let [x, y, z] = entity.fields().position;
        match block_entity {
            BlockEntity::Banner(banner) => banner.name.clone(),
            _ => world.block_names.get(&(x, y, z)).map(|name| match name {
                Tag::String(text) => text.to_string_lossy(),
                component => sunrise_client::component::name_text(component).plain(),
            }),
        }
    }

    /// The players whose open menu holds this block entity's container (for openers'
    /// containers, `ContainerOpenersCounter.getEntitiesWithContainerOpen`'s players), from the menus the opener counts read
    /// (`tick_mod_container_viewers`, `ChestBlockEntity$1.isOwnContainer`). Guests in
    /// its level, not spectators nor fake players, then the acting player for a menu
    /// their action opened, which the hub records when the action ends. None for a
    /// stale handle or a client.
    pub(super) fn block_entity_viewers(&self, entity: BlockEntityRef) -> Option<Vec<PlayerRef>> {
        if !self.is_logical_server() {
            return None;
        }
        self.native_block_entity_target(entity)?;
        let epoch = self.native_lifecycle.epoch()?;
        let fields = entity.fields();
        let level = fields.level;
        let [x, y, z] = fields.position;
        let key = (x, y, z);
        let mut viewers: Vec<PlayerRef> = self
            .lan
            .iter()
            .flat_map(|host| host.hub.guests.simulation_iter().map(move |g| (host, g)))
            .filter(|(host, g)| {
                // The acting guest's mode is live in `Game` during its turn; the hub
                // hears of a change only in the put-back at the end of `guest_turn`.
                let spectator = if self.lan_acting_uuid == Some(g.uuid) {
                    self.spectator()
                } else {
                    g.spectator()
                };
                g.participates_online()
                    && g.placed
                    && !spectator
                    && g.dimension == level.dimension_id()
                    && host.menus.get(&g.conn).is_some_and(|menus| {
                        menus.menu_holds(g.menu.as_ref(), key, fields.generation)
                    })
            })
            .filter_map(|(_, g)| {
                let id = u32::try_from(g.entity_id).ok()?;
                Some(player_ref(mod_api::EntityRef::new(epoch, id), g.uuid))
            })
            .collect();
        if self.level_in_place(level)
            && !self.lan_acting_fake
            && !self.spectator()
            && self
                .container
                .is_some_and(|open| open == key || self.chest_other_half(open) == Some(key))
            && let Some(player) = self.native_acting_player()
            && !viewers.contains(&player)
        {
            viewers.push(player);
        }
        Some(viewers)
    }

    /// Sets the custom name, or clears it with None, as an anvil-renamed item placed with
    /// `components` sets `CustomName`. Plain text only. Clients get a block entity update.
    /// Ok(false) for an entity that is not nameable; Stale for an invalidated or
    /// unwritable handle. Inside a transaction an abort puts the name back.
    pub(super) fn block_entity_set_name(
        &mut self,
        entity: BlockEntityRef,
        name: Option<&str>,
    ) -> Result<bool, BlockEntityError> {
        let (key, block) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        let nameable = self
            .world
            .block_entities
            .get(&key)
            .is_some_and(|current| nameable(key, current, &block, self.res.items()));
        if !nameable {
            return Ok(false);
        }
        self.journal_block_entity(key);
        match self.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Banner(banner)) => banner.name = name.map(str::to_owned),
            _ => {
                match name {
                    Some(name) => self.world.block_names.insert(key, Tag::String(name.into())),
                    None => self.world.block_names.remove(&key),
                };
            }
        }
        self.block_entity_updated(key);
        Ok(true)
    }
}

/// The compound a merge (or, with `replace`, a replace) of `payload` loads over the block entity
/// saved as `old`. A replace keeps the identity, position and published shapes, which are not
/// the payload's to replace.
fn loaded_payload(old: &Tag, payload: &Tag, replace: bool) -> Tag {
    let base = if replace {
        let mut metadata = Compound::new();
        if let Tag::Compound(fields) = old {
            for key in ["id", "x", "y", "z", sunrise_client::containers::SHAPES_KEY] {
                if let Some(value) = fields.get(&key.into()) {
                    metadata.insert(key.into(), value.clone());
                }
            }
        }
        Tag::Compound(metadata)
    } else {
        old.clone()
    };
    sunrise_client::data_cmd::merged(&base, payload)
}

/// A mod's data payload as the block entity takes it: without the fields the engine owns, which
/// are its identity and position, and its published shapes (`set_shapes` alone writes those).
pub(in crate::game) fn mod_payload(mut data: Compound) -> Compound {
    for field in ["id", "x", "y", "z", sunrise_client::containers::SHAPES_KEY] {
        data.remove(&field.into());
    }
    data
}

#[cfg(test)]
mod payload_tests {
    use super::*;

    #[test]
    fn merge_and_replace_keep_the_published_shapes_whatever_the_payload_carries() {
        use sunrise_client::containers::SHAPES_KEY;
        let shapes = Tag::Compound(Compound::from([(
            "collision".into(),
            Tag::LongArray(vec![1, 2, 3, 4, 5, 6]),
        )]));
        let old = Tag::Compound(Compound::from([
            ("id".into(), Tag::String("test:bus".into())),
            ("x".into(), Tag::Int(1)),
            ("progress".into(), Tag::Int(7)),
            ("kept".into(), Tag::Int(9)),
            (SHAPES_KEY.into(), shapes.clone()),
        ]));
        // A payload that carries the key, to overwrite it or to remove it.
        let colliding = || {
            mod_payload(Compound::from([
                ("progress".into(), Tag::Int(8)),
                ("id".into(), Tag::String("other:type".into())),
                (SHAPES_KEY.into(), Tag::Compound(Compound::new())),
            ]))
        };
        for replace in [false, true] {
            let loaded = loaded_payload(&old, &Tag::Compound(colliding()), replace);
            assert_eq!(loaded.get(SHAPES_KEY), Some(&shapes), "replace: {replace}");
            // The ordinary field applies, and the identity stays.
            assert_eq!(loaded.get("progress"), Some(&Tag::Int(8)));
            assert_eq!(loaded.get("id"), Some(&Tag::String("test:bus".into())));
            // A merge keeps the fields the payload omits; a replace drops them.
            assert_eq!(loaded.get("kept").is_some(), !replace);
        }
    }
}

/// The block entity as `/data get block` shows it (`saveWithFullMetadata`), with the custom
/// name Sunrise keeps beside the model.
fn save_with_metadata(
    world: &World,
    pos: (i32, i32, i32),
    entity: &BlockEntity,
    block: &str,
    items: &crate::items::Items,
) -> Option<Compound> {
    let pending = world
        .chunks
        .get(&(pos.0 >> 4, pos.2 >> 4))
        .and_then(|chunk| chunk.saved.as_ref())
        .and_then(|saved| saved.pending_loot.get(&pos));
    let Tag::Compound(data) = save::block_entity_encode::data(
        pos,
        entity,
        block,
        save::block_entity_encode::DataFields {
            lock: world.block_lock(pos),
            custom_name: world.block_names.get(&pos),
            pending,
            ..Default::default()
        },
        items,
    ) else {
        return None;
    };
    Some(data)
}

pub(super) fn save_without_metadata(
    world: &World,
    pos: (i32, i32, i32),
    entity: &BlockEntity,
    block: &str,
    items: &crate::items::Items,
) -> Option<Compound> {
    save_with_metadata(world, pos, entity, block, items).map(block_entity_payload)
}

/// The saved block entity's payload, with its world identity and position removed.
pub(super) fn block_entity_payload(mut data: Compound) -> Compound {
    for key in ["id", "x", "y", "z"] {
        data.remove(&key.into());
    }
    data
}

/// Whether the block entity keeps a `CustomName` (`Nameable`): its saved form does.
fn nameable(
    pos: (i32, i32, i32),
    entity: &BlockEntity,
    block: &str,
    items: &crate::items::Items,
) -> bool {
    let mut tag = save::block_entity_encode::encode(pos, entity, block, items);
    if let Tag::Compound(map) = &mut tag {
        map.insert("CustomName".into(), Tag::String("x".into()));
    }
    block_entity_nbt::canonical(&tag).is_some_and(|tag| tag.get("CustomName").is_some())
}

fn mod_item_slots(
    world: &World,
    key: (i32, i32, i32),
    slot: u32,
) -> Result<usize, BlockEntityItemError> {
    let slots = world
        .block_entities
        .mod_container(world.block(key.0, key.1, key.2).0)
        .ok_or(BlockEntityItemError::Unavailable)?
        .slots;
    if slot as usize >= slots {
        return Err(BlockEntityItemError::SlotOutOfBounds {
            slot,
            slots: slots as u32,
        });
    }
    Ok(slots)
}

pub(super) fn part_snapshot(
    world: &World,
    level: Option<LevelRef>,
    authoritative: bool,
    index: Option<
        &std::sync::Arc<
            std::sync::Mutex<
                sunrise_mod_runtime::attachments::index::AttachmentIndex<mod_api::AttachmentHolder>,
            >,
        >,
    >,
    scopes: &sunrise_mod_runtime::attachments::AttachmentQueryScopes,
    entity: BlockEntityRef,
    (namespace, name): (&str, &str),
) -> Result<mod_api::AttachmentBatch, mod_api::PartStateError> {
    use sunrise_mod_runtime::attachments::AttachmentBatch;

    let fields = entity.fields();
    if level != Some(fields.level) {
        return Err(mod_api::PartStateError::OtherLevel);
    }
    let key = (fields.position[0], fields.position[1], fields.position[2]);
    if !world.is_loaded_at(key.0, key.1, key.2)
        || world.block_entities.generation(&key) != Some(fields.generation)
    {
        return Err(mod_api::PartStateError::Stale);
    }
    mod_api::__private::validate_attachment_use(namespace, name, false).map_err(|error| {
        mod_api::PartStateError::Attachment {
            namespace: "",
            name: "",
            error,
        }
    })?;
    let batch = if authoritative {
        index.map_or_else(AttachmentBatch::empty, |index| {
            AttachmentBatch::new(index.lock().unwrap().snapshot_named(namespace, name))
        })
    } else {
        AttachmentBatch::empty()
    };
    Ok(batch.in_scope(scopes).into())
}

/// The handle of the loaded block entity at `pos`, without loading a chunk or taking one
/// temporarily absent from the resident table.
pub(super) fn world_block_entity(
    world: &World,
    level: LevelRef,
    pos: [i32; 3],
) -> Option<BlockEntityRef> {
    loaded_block_entity(world, pos)?;
    block_entity_handle(world, level, pos)
}

/// Construct a handle after the caller has checked that the entry is resident.
pub(super) fn block_entity_handle(
    world: &World,
    level: LevelRef,
    pos: [i32; 3],
) -> Option<BlockEntityRef> {
    let [x, y, z] = pos;
    let generation = world.block_entities.generation(&(x, y, z))?;
    Some(mod_api::__private::game::block_entity_ref(
        level, pos, generation,
    ))
}

/// Read a resident entity's saved data after the caller has resolved its level to `world`.
pub(super) fn world_block_entity_data(
    world: &World,
    registry: &sunrise_world::registry::BlockRegistry,
    items: &crate::items::Items,
    entity: BlockEntityRef,
) -> Option<Compound> {
    let block_entity = resident_block_entity(world, entity)?;
    let [x, y, z] = entity.fields().position;
    let block = &registry.state(world.block(x, y, z))?.name;
    save_without_metadata(world, (x, y, z), block_entity, block, items)
}

fn resident_block_entity(world: &World, entity: BlockEntityRef) -> Option<&BlockEntity> {
    let fields = entity.fields();
    let [x, y, z] = fields.position;
    if world.block_entities.generation(&(x, y, z)) != Some(fields.generation) {
        return None;
    }
    loaded_block_entity(world, fields.position)
}

fn loaded_block_entity(world: &World, [x, y, z]: [i32; 3]) -> Option<&BlockEntity> {
    if !world.loaded(x, z) || !world.shape.contains_y(y) {
        return None;
    }
    world.block_entities.get(&(x, y, z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::HostTransaction;
    use mod_api::ServerHost;
    use std::sync::Arc;
    use sunrise_client::{chunk::Chunk, containers::ModBlockEntityFields, lan::menu::Slots};
    use sunrise_world::registry::BlockStateId;

    #[test]
    fn saved_data_omits_metadata_but_keeps_contents() {
        let world = World::new(BlockStateId(0), 0);
        let entity = BlockEntity::Chest(vec![None; 27]);
        let data = save_without_metadata(
            &world,
            (1, 64, 2),
            &entity,
            "minecraft:chest",
            &Default::default(),
        )
        .unwrap();
        for key in ["id", "x", "y", "z"] {
            assert!(!data.contains_key(&key.into()));
        }
        assert!(data.contains_key(&"Items".into()));
    }

    /// What a reader of a vanilla machine's progress (a crafting CPU's estimate, a progress
    /// line) needs is in `BlockEntity.saveWithoutMetadata`, which `block_entity_data` returns:
    /// a furnace's cooking and burning timers and the fuel's speed, a brewing stand's brew
    /// timer counting down, its total, its fuel and its bottles.
    #[test]
    fn saved_data_carries_vanilla_machine_progress() {
        use sunrise_client::{
            brewing::BrewingStand,
            containers::{Furnace, FurnaceKind},
        };
        let world = World::new(BlockStateId(0), 0);
        let int = |data: &Compound, key: &str| match data.get(&key.into()) {
            Some(Tag::Int(value)) => *value,
            other => panic!("{key} is {other:?}"),
        };
        let float = |data: &Compound, key: &str| match data.get(&key.into()) {
            Some(Tag::Float(bits)) => f32::from_bits(*bits),
            other => panic!("{key} is {other:?}"),
        };
        let slots = |data: &Compound| match data.get(&"Items".into()) {
            Some(Tag::List(items)) => items
                .iter()
                .map(|item| match item.get("Slot") {
                    Some(Tag::Byte(slot)) => *slot,
                    other => panic!("slot is {other:?}"),
                })
                .collect::<Vec<_>>(),
            other => panic!("Items is {other:?}"),
        };

        let furnace = BlockEntity::Furnace(Furnace {
            kind: FurnaceKind::BlastFurnace,
            items: [Some(ItemStack::new("minecraft:raw_iron", 3)), None, None],
            lit_remaining: 750,
            lit_total: 800,
            cooking_timer: 40,
            cooking_total: 100,
            speed: 2.0,
            recipes_used: Default::default(),
        });
        let data = save_without_metadata(
            &world,
            (0, 64, 0),
            &furnace,
            "minecraft:blast_furnace",
            &Default::default(),
        )
        .unwrap();
        assert_eq!(int(&data, "cooking_time_spent"), 40);
        assert_eq!(int(&data, "cooking_total_time"), 100);
        assert_eq!(int(&data, "lit_time_remaining"), 750);
        assert_eq!(int(&data, "lit_total_time"), 800);
        assert_eq!(float(&data, "speed_multiplier"), 2.0);
        assert_eq!(slots(&data), vec![0], "the input slot is held");

        let mut stand = BrewingStand::new();
        stand.brew_time = 150;
        stand.total_brew_time = 400;
        stand.fuel = 12;
        stand.items[0] = Some(ItemStack::new("minecraft:potion", 1));
        stand.items[2] = Some(ItemStack::new("minecraft:potion", 1));
        stand.items[3] = Some(ItemStack::new("minecraft:nether_wart", 1));
        let stand = BlockEntity::Brewing(Box::new(stand));
        let data = save_without_metadata(
            &world,
            (1, 64, 0),
            &stand,
            "minecraft:brewing_stand",
            &Default::default(),
        )
        .unwrap();
        assert_eq!(int(&data, "BrewTime"), 150);
        assert_eq!(int(&data, "total_brew_time"), 400);
        assert_eq!(int(&data, "Fuel"), 12);
        assert_eq!(float(&data, "speed_multiplier"), 1.0);
        assert_eq!(
            slots(&data),
            vec![0, 2, 3],
            "two bottles and the ingredient"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn retained_handles_reject_replacements_and_rollback_restores_identity() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        if run_fresh_cases(
            "game::native_block_entity_refs::tests::retained_handles_reject_replacements_and_rollback_restores_identity",
            "SUNRISE_RETAINED_HANDLES_CHILD",
            &["holder"],
        ) {
            return;
        }
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let pos = (1, 64, 1);
            let holder = || {
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "test:holder".into(),
                    ModBlockEntityFields {
                        nbt: compound("{count:3}"),
                        ..Default::default()
                    },
                )))
            };
            game.world.block_entities.insert(pos, holder());
            let original = game.block_entity(level, [1, 64, 1]).unwrap();
            game.with_level(|_, host, ticks| {
                let wrapped = super::super::mod_api_host::LevelTicks::new(&*host, ticks);
                assert_eq!(
                    mod_api::LevelHost::block_entity(&wrapped, [1, 64, 1]),
                    mod_api::LevelHost::block_entity(&*host, [1, 64, 1]),
                );
                assert_eq!(
                    mod_api::LevelHost::block_entity(&wrapped, [1, 64, 1]),
                    Some(original),
                );
                assert_eq!(
                    mod_api::LevelHost::block_entity(&wrapped, [1, 400, 1]),
                    None,
                );
                // The ticks wrapper hands the same reads through, saved data included.
                let data = mod_api::LevelHost::block_entity_data(&*host, original);
                assert_eq!(
                    data.as_ref().and_then(|data| data.get(&"count".into())),
                    Some(&mod_api::Tag::Int(3))
                );
                assert_eq!(
                    mod_api::LevelHost::block_entity_data(&wrapped, original),
                    data,
                );
            });
            assert_eq!(original.set_data_slots(game, vec![7]), Ok(true));
            let mut tentative = None;
            let _: Result<(), ()> = game.transaction(|game, _| {
                game.journal_block_entity(pos);
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Chest(vec![]));
                assert_eq!(original.data_slots(game), None);
                tentative = game.block_entity(level, [1, 64, 1]);
                Err(())
            });
            assert_eq!(original.data_slots(game), Some(vec![7]));
            assert_eq!(tentative.unwrap().data(game), None);
            let (lease, taken) = game.world.block_entities.take(&pos).unwrap();
            assert_eq!(original.data_slots(game), None);
            assert!(game.world.block_entities.put(lease, taken));
            assert_eq!(original.data_slots(game), Some(vec![7]));
            game.world.block_entities.remove(&pos);
            game.world.block_entities.insert(pos, holder());
            let replacement = game.block_entity(level, [1, 64, 1]).unwrap();
            assert_ne!(original, replacement);
            let stale = Err(BlockEntityError::Stale);
            assert_eq!(original.set_lock(game, None), stale);
            assert_eq!(original.set_name(game, Some("stale")), stale);
            assert_eq!(original.merge_data(game, Compound::new()), stale);
            assert_eq!(original.replace_data(game, Compound::new()), stale);
            assert_eq!(original.set_changed(game), stale);
            assert_eq!(original.set_data_slot(game, 0, 99), stale);
            assert_eq!(
                original.set_data_slots(game, vec![99]),
                Err(BlockEntityDataError::Stale)
            );
            assert_eq!(
                original.item_slot(game, 0),
                Err(BlockEntityItemError::Stale)
            );
            assert_eq!(replacement.data_slots(game), Some(vec![]));
            assert_eq!(replacement.set_data_slots(game, vec![8]), Ok(true));
            game.world.block_entities.load_chunk(vec![(pos, holder())]);
            assert_eq!(
                replacement.set_data_slots(game, vec![99]),
                Err(BlockEntityDataError::Stale)
            );
        });
    }

    #[test]
    fn lookup_requires_loaded_chunk_and_tracks_removal_and_replacement() {
        let mut world = World::new(BlockStateId(0), 0);
        let pos = [-1, 64, 0];
        let key = (-1, 64, 0);
        world.block_entities.insert(key, BlockEntity::Chest(vec![]));
        assert!(loaded_block_entity(&world, pos).is_none());
        world.chunks.insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
        assert!(matches!(
            loaded_block_entity(&world, pos),
            Some(BlockEntity::Chest(_))
        ));
        world.block_entities.remove(&key);
        assert!(loaded_block_entity(&world, pos).is_none());
        world
            .block_entities
            .insert(key, BlockEntity::DaylightDetector);
        assert!(matches!(
            loaded_block_entity(&world, pos),
            Some(BlockEntity::DaylightDetector)
        ));
        let (lease, detached) = world.block_entities.take(&key).unwrap();
        assert!(loaded_block_entity(&world, pos).is_none());
        assert!(world.block_entities.put(lease, detached));
        assert!(matches!(
            loaded_block_entity(&world, pos),
            Some(BlockEntity::DaylightDetector)
        ));
        let outside_pos = [-1, world.shape.max_y(), 0];
        world
            .block_entities
            .insert((-1, outside_pos[1], 0), BlockEntity::DaylightDetector);
        assert!(loaded_block_entity(&world, outside_pos).is_none());
        let below_pos = [-1, world.shape.min_y - 1, 0];
        world
            .block_entities
            .insert((-1, below_pos[1], 0), BlockEntity::DaylightDetector);
        assert!(loaded_block_entity(&world, below_pos).is_none());
    }

    fn compound(snbt: &str) -> Compound {
        match sunrise_core::nbt::parse_snbt(snbt).unwrap().0 {
            Tag::Compound(c) => c,
            _ => panic!("not a compound"),
        }
    }

    #[test]
    #[ignore = "daily"]
    fn player_opens_provider_and_refuses_non_provider() {
        use mod_api::*;
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_block_entity_refs::tests::player_opens_provider_and_refuses_non_provider",
            "SUNRISE_BE_MENU_CASE",
            &[
                "provider",
                "replacement",
                "existing",
                "close",
                "close_reentry",
            ],
        ) {
            return;
        }
        struct ReplaceDuringOpen;
        impl ReplaceDuringOpen {
            fn replace(host: &mut dyn ServerHost) {
                let level = host.level("minecraft:overworld").unwrap();
                for block in ["minecraft:stone", "minecraft:chest"] {
                    let state = host.block_state_id(block).unwrap();
                    assert_eq!(
                        level.set_block(host, [0, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                        Ok(true)
                    );
                }
            }
        }
        #[mod_api::__subscriptions]
        impl Hooks for ReplaceDuringOpen {
            fn inventory_block_start(
                &self,
                host: &mut dyn ServerHost,
                event: &mut InventoryBlockStartEvent<'_>,
            ) -> Flow {
                if event.pos == [0, 64, 0]
                    && !std::env::var("SUNRISE_BE_MENU_CASE")
                        .unwrap()
                        .starts_with("close")
                {
                    Self::replace(host);
                }
                Flow::Continue
            }
            fn player_container_close(
                &self,
                host: &mut dyn ServerHost,
                _event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                let case = std::env::var("SUNRISE_BE_MENU_CASE").unwrap();
                if case.starts_with("close") {
                    Self::replace(host);
                    if case == "close_reentry" {
                        let level = host.level("minecraft:overworld").unwrap();
                        let other = host.block_entity(level, [5, 64, 0]).unwrap();
                        let player = host.player(12).unwrap();
                        assert!(player.open_menu(host, other).unwrap());
                    }
                }
            }
        }
        let case = std::env::var("SUNRISE_BE_MENU_CASE").unwrap();
        let replacing = case != "provider";
        if replacing {
            let mods: Mods = vec![ModEntry::new("replace", Box::new(ReplaceDuringOpen))].into();
            mod_platform::__private::install(mods.clone());
            crate::game::api_values::install_parts(&mods).unwrap();
        }
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            for (x, name) in [
                (0, "minecraft:chest"),
                (2, "minecraft:furnace"),
                (4, "minecraft:oak_sign"),
            ] {
                let state = game.pack.assets.registry.default_state(name).unwrap();
                assert_eq!(
                    game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
            }
            let player = game.player(12).unwrap();
            let chest = game.block_entity(level, [0, 64, 0]).unwrap();
            if replacing {
                use sunrise_client::lan::{Out, PlayerSender};
                use sunrise_protocol::{buf::Reader, ids::play::clientbound};
                let (tx, rx) = std::sync::mpsc::channel();
                game.lan.as_mut().unwrap().hub.update_guest(12, |guest| {
                    guest.sender = PlayerSender::new(tx, Arc::default());
                });
                let previous = if case != "replacement" {
                    let state = game.world.block(0, 64, 0);
                    assert_eq!(
                        game.level_set_block(level, [5, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                        Ok(true)
                    );
                    let other = game.block_entity(level, [5, 64, 0]).unwrap();
                    assert!(player.open_menu(game, other).unwrap());
                    let host = game.lan.as_ref().unwrap();
                    Some(host.hub.guest(12).unwrap().menu.as_ref().unwrap().window)
                } else {
                    None
                };
                while rx.try_recv().is_ok() {}
                assert_eq!(player.open_menu(game, chest), Err(BlockEntityError::Stale));
                let replacement = game.block_entity(level, [0, 64, 0]).unwrap();
                assert_ne!(chest, replacement);
                assert_eq!(chest.position(game), None);
                let menu = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(12)
                    .unwrap()
                    .menu
                    .as_ref();
                assert_eq!(menu.is_some(), case == "close_reentry");
                if let Some(menu) = menu {
                    assert!(matches!(menu.slots, Slots::Block((5, 64, 0))));
                }
                let mut opens = Vec::new();
                let mut closes = Vec::new();
                for out in rx.try_iter() {
                    if let Out::Packet(packet) = out {
                        let mut reader = Reader::new(&packet);
                        match reader.varint().unwrap() {
                            clientbound::OPEN_SCREEN => opens.push(reader.varint().unwrap()),
                            clientbound::CONTAINER_CLOSE => closes.push(reader.varint().unwrap()),
                            _ => {}
                        }
                    }
                }
                assert_eq!(
                    opens,
                    menu.map(|menu| menu.window).into_iter().collect::<Vec<_>>()
                );
                let expected_closes = if case == "close_reentry" {
                    Vec::new()
                } else {
                    previous.into_iter().collect()
                };
                assert_eq!(closes, expected_closes);
                return;
            }
            let furnace = game.block_entity(level, [2, 64, 0]).unwrap();
            let sign = game.block_entity(level, [4, 64, 0]).unwrap();
            use super::super::lan_actions::MenuOpen;
            assert!(!player.open_menu(game, sign).unwrap());
            assert_eq!(game.open_block_menu(999, (0, 64, 0)), MenuOpen::Refused);
            game.world
                .block_entities
                .insert((0, 64, 0), BlockEntity::Chest(vec![None; 2]));
            assert!(!player.open_menu(game, chest).unwrap());
            game.world
                .block_entities
                .insert((0, 64, 0), BlockEntity::Chest(vec![None; 27]));
            let dimension = game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                std::mem::replace(&mut guest.dimension, "minecraft:the_nether".to_owned())
            });
            assert!(!player.open_menu(game, chest).unwrap());
            {
                let value = dimension;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            let host = game.lan.take();
            assert!(!player.open_menu(game, chest).unwrap());
            assert_eq!(game.open_block_menu(12, (0, 64, 0)), MenuOpen::Refused);
            game.lan = host;
            let mut guest = game.lan.as_mut().unwrap().hub.guests.remove(0);
            guest.kind = sunrise_client::lan::guest::GuestKind::Fake;
            game.lan.as_mut().unwrap().hub.guests.insert(0, guest);
            // Fake players have no client menu. This is a refusal, not a hook cancellation.
            assert_eq!(game.open_block_menu(12, (0, 64, 0)), MenuOpen::Refused);
            assert!(!player.open_menu(game, chest).unwrap());
            let mut guest = game.lan.as_mut().unwrap().hub.guests.remove(0);
            guest.kind = sunrise_client::lan::guest::GuestKind::Network;
            game.lan.as_mut().unwrap().hub.guests.insert(0, guest);
            assert!(player.open_menu(game, chest).unwrap());
            assert_eq!(chest.lock(game), None);
            assert!(chest.set_lock(game, Some(Compound::new())).unwrap());
            assert_eq!(chest.lock(game), Some(&Compound::new()));
            let stick_lock = compound("{items:\"minecraft:stick\"}");
            assert!(chest.set_lock(game, Some(stick_lock.clone())).unwrap());
            let lock = chest.lock(game).unwrap();
            assert_eq!(lock, &stick_lock);
            let Some(Tag::Compound(resident)) = game.world.block_lock((0, 64, 0)) else {
                panic!("the lock was not stored with the block entity");
            };
            assert!(std::ptr::eq(lock, resident));
            assert!(!player.open_menu(game, chest).unwrap());
            {
                let value = Some(sunrise_client::items::ItemStack::new("minecraft:stick", 1));
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.inventory[0] = value);
            }
            assert!(player.open_menu(game, chest).unwrap());
            {
                let value = None;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.inventory[0] = value);
            }
            assert!(chest.set_lock(game, None).unwrap());
            assert_eq!(chest.lock(game), None);
            assert!(player.open_menu(game, chest).unwrap());
            assert!(player.open_menu(game, furnace).unwrap());

            // Both halves of a double chest participate, whichever half is opened.
            use sunrise_world::chunk::BlockState;
            for (x, kind) in [(6, "left"), (7, "right")] {
                let state = game
                    .pack
                    .assets
                    .registry
                    .resolve(&BlockState {
                        name: "minecraft:chest".into(),
                        properties: [
                            ("facing".into(), "north".into()),
                            ("type".into(), kind.into()),
                            ("waterlogged".into(), "false".into()),
                        ]
                        .into_iter()
                        .collect(),
                    })
                    .unwrap();
                assert_eq!(
                    game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
            }
            let left = game.block_entity(level, [6, 64, 0]).unwrap();
            let right = game.block_entity(level, [7, 64, 0]).unwrap();
            assert!(game.chest_partner((6, 64, 0)).is_some());
            for half in [left, right] {
                assert!(
                    half.set_lock(game, Some(compound("{items:\"minecraft:stick\"}")))
                        .unwrap()
                );
                assert!(!player.open_menu(game, left).unwrap());
                assert!(!player.open_menu(game, right).unwrap());
                assert!(half.set_lock(game, None).unwrap());
                assert!(player.open_menu(game, left).unwrap());
            }
            game.world.block_entities.remove(&(2, 64, 0));
            assert_eq!(
                player.open_menu(game, furnace),
                Err(BlockEntityError::Stale)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn viewers_follow_open_menus_per_level_and_leave_out_fake_players() {
        use super::super::Dim;
        use mod_api::{BlockUpdateFlags, EntityRef, Host};
        use sunrise_world::chunk::BlockState;
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = |game: &mut Game<'_>, level, x: i32, kind: &str| {
                let state = game
                    .pack
                    .assets
                    .registry
                    .resolve(&BlockState {
                        name: "minecraft:chest".into(),
                        properties: [
                            ("facing".into(), "north".into()),
                            ("type".into(), kind.into()),
                            ("waterlogged".into(), "false".into()),
                        ]
                        .into_iter()
                        .collect(),
                    })
                    .unwrap();
                assert_eq!(
                    game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                game.block_entity(level, [x, 64, 0]).unwrap()
            };
            let single = chest(game, level, 0, "single");
            let left = chest(game, level, 6, "left");
            let right = chest(game, level, 7, "right");
            let epoch = game.native_lifecycle.epoch().unwrap();
            // with_guests: uuids and entity ids 12 and 13.
            let guest12 = player_ref(EntityRef::new(epoch, 12), 12);
            let guest13 = player_ref(EntityRef::new(epoch, 13), 13);
            let host: &dyn Host = game;
            assert_eq!(single.viewers(host), Some(vec![]));

            // Two players open the chest, then close it one by one.
            assert!(guest12.open_menu(game, single).unwrap());
            assert!(guest13.open_menu(game, single).unwrap());
            assert_eq!(single.viewers(game), Some(vec![guest12, guest13]));
            game.guest_close_all(12);
            assert_eq!(single.viewers(game), Some(vec![guest13]));
            game.guest_close_all(13);
            assert_eq!(single.viewers(game), Some(vec![]));

            // A menu the acting player's action opened counts before the hub records it,
            // and only once when the hub already has it.
            game.as_guest(13, |game| {
                game.container = Some((0, 64, 0));
                assert_eq!(single.viewers(game), Some(vec![guest13]));
                game.container = None;
            })
            .unwrap();
            assert!(guest12.open_menu(game, single).unwrap());
            game.as_guest(12, |game| {
                game.container = Some((0, 64, 0));
                assert_eq!(single.viewers(game), Some(vec![guest12]));
                game.container = None;
            })
            .unwrap();
            game.guest_close_all(12);

            // A guest that turns spectator during its own turn stops being a viewer then,
            // though the hub still has the old mode until the turn ends.
            assert!(guest12.open_menu(game, single).unwrap());
            game.as_guest(12, |game| {
                assert!(game.set_game_mode(sunrise_client::command::args::GameMode::Spectator));
                assert_eq!(single.viewers(game), Some(vec![]));
            })
            .unwrap();
            game.guest_close_all(12);
            game.as_guest(12, |game| {
                assert!(game.set_game_mode(sunrise_client::command::args::GameMode::Survival));
            })
            .unwrap();

            // A double chest's menu holds both halves, whichever was opened.
            assert!(guest12.open_menu(game, right).unwrap());
            assert_eq!(left.viewers(game), Some(vec![guest12]));
            assert_eq!(right.viewers(game), Some(vec![guest12]));
            assert_eq!(single.viewers(game), Some(vec![]));

            // A fake player holding the same menu is not a viewer.
            assert!(guest13.open_menu(game, left).unwrap());
            let mut fake = game.lan.as_mut().unwrap().hub.guests.remove(1);
            fake.kind = sunrise_client::lan::guest::GuestKind::Fake;
            game.lan.as_mut().unwrap().hub.guests.insert(1, fake);
            assert_eq!(left.viewers(game), Some(vec![guest12]));

            // The same position in another level has none of these viewers.
            let nether = LevelRef::new(epoch, mod_api::DimensionKey::NETHER);
            game.enter_level(Dim::Nether).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let nether_right = chest(game, nether, 7, "single");
            assert_eq!(nether_right.viewers(game), Some(vec![]));
            assert_eq!(right.viewers(game), Some(vec![guest12]), "parked level");
            game.enter_level(Dim::Overworld).unwrap();

            // A stale handle reads None.
            game.world.block_entities.remove(&(7, 64, 0));
            assert_eq!(right.viewers(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn writes_merge_data_name_and_roll_back_with_a_transaction() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game_at};
        use mod_api::BlockUpdateFlags;
        if run_fresh_cases(
            "game::native_block_entity_refs::tests::writes_merge_data_name_and_roll_back_with_a_transaction",
            "SUNRISE_VANILLA_BE_REGISTRY_CHILD",
            &["vanilla"],
        ) {
            return;
        }
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = super::super::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS;
            let states = ["minecraft:chest", "minecraft:daylight_detector"];
            for (i, name) in states.into_iter().enumerate() {
                let state = game.pack.assets.registry.default_state(name).unwrap();
                assert_eq!(
                    game.level_set_block(level, [i as i32 * 2, 64, 0], state, flags),
                    Ok(true)
                );
            }
            let chest = game.block_entity(level, [0, 64, 0]).unwrap();
            let sensor = game.block_entity(level, [2, 64, 0]).unwrap();
            let stick = "{Items:[{Slot:3b,id:\"minecraft:stick\",count:3}]}";
            let slot3 = |game: &Game<'_>| match game.world.block_entities.get(&(0, 64, 0)) {
                Some(BlockEntity::Chest(items)) => items[3].as_ref().map(|s| s.count),
                _ => panic!("chest block entity missing"),
            };

            // A merge loads the data, ignores the identity keys and keeps the rest.
            assert!(
                chest
                    .merge_data(
                        game,
                        compound(&format!("{{id:\"minecraft:furnace\",x:9,{}", &stick[1..]))
                    )
                    .unwrap()
            );
            assert_eq!(slot3(game), Some(3));
            assert_eq!(
                chest.kind(game).and_then(|kind| kind.id(game)),
                Some("minecraft:chest")
            );
            assert_eq!(chest.position(game), Some([0, 64, 0]));
            assert!(
                !chest.merge_data(game, compound(stick)).unwrap(),
                "no change"
            );
            assert!(chest.merge_data(game, compound("{Items:[]}")).unwrap());
            assert_eq!(slot3(game), None);

            // Names: a chest has one, a daylight detector does not.
            assert_eq!(chest.name(game), None);
            assert!(chest.set_name(game, Some("Loot")).unwrap());
            assert_eq!(chest.name(game), Some("Loot".to_owned()));
            let data = chest.data(game).unwrap();
            assert_eq!(
                data.get(&"CustomName".into()),
                Some(&Tag::String("Loot".into()))
            );
            assert!(!sensor.set_name(game, Some("Nope")).unwrap());
            assert_eq!(sensor.name(game), None);
            assert!(chest.set_name(game, None).unwrap());
            assert_eq!(chest.name(game), None);

            // An unrelated API merge must preserve the loaded rich name, not
            // remove it through the plain-text name accessor's projection.
            let styled = "{text:\"Styled\",color:\"aqua\",extra:[{text:\" chest\",bold:1b}]}";
            let name = Tag::Compound(compound(styled));
            assert!(game.load_block_entity_nbt([0, 64, 0], &format!("{{CustomName:{styled}}}")));
            assert_eq!(game.world.block_names.get(&(0, 64, 0)), Some(&name));
            assert!(chest.merge_data(game, compound(stick)).unwrap());
            assert_eq!(slot3(game), Some(3));
            assert_eq!(game.world.block_names.get(&(0, 64, 0)), Some(&name));
            assert_eq!(
                chest.data(game).unwrap().get(&"CustomName".into()),
                Some(&name)
            );
            assert!(chest.replace_data(game, compound("{}")).unwrap());
            assert_eq!((slot3(game), chest.name(game)), (None, None));

            // An abort puts back data and name; a commit keeps them.
            let host: &mut dyn ServerHost = game;
            let aborted: Result<(), ()> = host.transaction(|host, _| {
                assert!(chest.merge_data(host, compound(stick)).unwrap());
                assert!(chest.set_name(host, Some("Gone")).unwrap());
                assert!(chest.set_changed(host).unwrap());
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!((slot3(game), chest.name(game)), (None, None));
            assert!(game.transactions.is_empty());
            let host: &mut dyn ServerHost = game;
            let committed: Result<(), ()> = host.transaction(|host, _| {
                assert!(chest.merge_data(host, compound(stick)).unwrap());
                assert!(chest.set_name(host, Some("Kept")).unwrap());
                Ok(())
            });
            assert_eq!(committed, Ok(()));
            assert_eq!(
                (slot3(game), chest.name(game)),
                (Some(3), Some("Kept".to_owned()))
            );

            // Replacement uses the same journal and restores the original payload on abort.
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, _| {
                assert!(chest.replace_data(host, compound("{}")).unwrap());
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(
                (slot3(game), chest.name(game)),
                (Some(3), Some("Kept".to_owned()))
            );

            // Merge preserves omitted data; replacement clears the name and items.
            assert!(
                chest
                    .merge_data(game, compound("{CustomName:\"Changed\"}"))
                    .unwrap()
            );
            assert_eq!(slot3(game), Some(3));
            assert_eq!(chest.name(game), Some("Changed".to_owned()));
            game.world.block_log = Some(Vec::new());
            assert!(chest.replace_data(game, compound("{}")).unwrap());
            assert_eq!((slot3(game), chest.name(game)), (None, None));
            assert_eq!(game.world.block_log, Some(vec![(0, 64, 0)]));

            // Clients are told: the block log names the position, and the chunk is dirty.
            game.world.block_log = Some(Vec::new());
            game.world.modified.clear();
            assert!(chest.set_name(game, Some("Told")).unwrap());
            assert_eq!(game.world.block_log, Some(vec![(0, 64, 0)]));
            assert!(game.world.modified.contains(&(0, 0)));
            game.world.modified.clear();
            assert!(chest.set_changed(game).unwrap());
            assert!(game.world.modified.contains(&(0, 0)));

            // A removed block entity is a stale handle.
            game.world.block_entities.remove(&(0, 64, 0));
            let stale = Err(BlockEntityError::Stale);
            assert_eq!(chest.replace_data(game, compound("{}")), stale);
            assert_eq!(chest.set_changed(game), stale);
            assert_eq!(chest.merge_data(game, compound(stick)), stale);
            assert_eq!(chest.set_name(game, Some("x")), stale);
            assert_eq!(chest.name(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_abort_puts_back_the_lock_and_the_loot_table_roll() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::BlockUpdateFlags;
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game.pack.assets.registry.default_state("minecraft:chest");
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest.unwrap(), flags),
                Ok(true)
            );
            let chest = game.block_entity(level, [0, 64, 0]).unwrap();
            let key = (0, 64, 0);
            let lock = compound("{lock:{items:\"minecraft:stick\"}}");
            let table = compound("{LootTable:\"minecraft:chests/simple_dungeon\"}");
            let before = game.world.block_entity_extras(key);

            let host: &mut dyn ServerHost = game;
            let aborted: Result<(), ()> = host.transaction(|host, _| {
                assert!(
                    chest
                        .set_lock(host, Some(compound("{items:\"minecraft:stick\"}")))
                        .unwrap()
                );
                assert!(chest.merge_data(host, table.clone()).unwrap());
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(game.world.block_entity_extras(key), before);
            assert!(game.world.block_lock(key).is_none());

            let host: &mut dyn ServerHost = game;
            let committed: Result<(), ()> = host.transaction(|host, _| {
                assert!(chest.merge_data(host, lock.clone()).unwrap());
                assert!(chest.merge_data(host, table.clone()).unwrap());
                Ok(())
            });
            assert_eq!(committed, Ok(()));
            assert!(game.world.block_lock(key).is_some());
            assert_ne!(game.world.block_entity_extras(key), before);
        });
    }

    /// `LevelWriteHost::set_block` in a level hook: the block entity goes with
    /// its block and comes with the new one, as the Host's write does.
    #[test]
    #[ignore = "daily"]
    fn a_level_hook_set_block_replaces_whole_states_and_their_block_entities() {
        use super::super::mod_api_host::LevelTicks;
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::{BlockUpdateFlags, LevelWriteHost};
        use sunrise_world::chunk::BlockState;

        fn write(
            game: &mut Game<'_>,
            pos: [i32; 3],
            state: mod_api::BlockState,
            flags: BlockUpdateFlags,
        ) -> bool {
            game.with_level(|_, level, ticks| {
                LevelWriteHost::set_block(&mut LevelTicks::new(level, ticks), pos, state, flags)
            })
        }
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str, age: Option<&str>| {
                registry
                    .resolve(&BlockState {
                        name: name.into(),
                        properties: age
                            .map(|age| ("age".into(), age.into()))
                            .into_iter()
                            .collect(),
                    })
                    .unwrap()
            };
            let stone = state("minecraft:stone", None);
            let chest = state("minecraft:chest", None);
            let young = state("minecraft:wheat", Some("0"));
            let grown = state("minecraft:wheat", Some("7"));
            use crate::game::api_values::TryIntoApi;
            let api_stone = stone.try_into_api().unwrap();
            let api_chest = chest.try_into_api().unwrap();
            let api_young = young.try_into_api().unwrap();
            let api_grown = grown.try_into_api().unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::KNOWN_SHAPE;
            let key = (0, 64, 0);

            assert!(write(game, [0, 64, 0], api_stone, flags));
            assert!(game.world.block_entities.get(&key).is_none());
            assert!(write(game, [0, 64, 0], api_chest, flags));
            assert!(matches!(
                game.world.block_entities.get(&key),
                Some(BlockEntity::Chest(_))
            ));
            assert!(write(game, [0, 64, 0], api_stone, flags));
            assert_eq!(game.world.block(0, 64, 0), stone);
            assert!(game.world.block_entities.get(&key).is_none());

            // A crop's whole-state growth: no block entity, and a repeat changes nothing.
            assert!(write(game, [0, 64, 0], api_young, flags));
            assert!(write(game, [0, 64, 0], api_grown, flags));
            assert_eq!(game.world.block(0, 64, 0), grown);
            assert!(game.world.block_entities.get(&key).is_none());
            assert!(!write(game, [0, 64, 0], api_grown, flags));

            // Unloaded, outside the build height and invalid: refused, nothing loaded.
            let chunks = game.world.chunks.len();
            assert!(!write(game, [16, 64, 0], api_stone, flags));
            assert!(!write(game, [0, 10000, 0], api_stone, flags));
            assert!(!write(
                game,
                [0, 64, 0],
                mod_api::__private::block_state_from_ordinal(api_stone.block(), u16::MAX),
                flags
            ));
            assert_eq!(game.world.chunks.len(), chunks);
            assert_eq!(game.world.block(0, 64, 0), grown);

            // The same Host write still creates the block entity.
            assert!(game.native_set_block("minecraft:overworld", [1, 64, 0], chest, flags.bits()));
            assert!(matches!(
                game.world.block_entities.get(&(1, 64, 0)),
                Some(BlockEntity::Chest(_))
            ));

            // A client's copy of the level is not the server's.
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            game.options.dedicated = None;
            assert!(!write(game, [0, 64, 0], api_stone, flags));
            assert_eq!(game.world.block(0, 64, 0), grown);
        });
    }
}
