use super::Game;
use super::api_values::{TryIntoApi, TryIntoEngine};
use mod_api::{AttachmentAccess, BlockPos, Direction};
use {
    sunrise_mod_runtime::attachments::{ATTACHMENTS_KEY, attachment_compound},
    sunrise_world::level::Ticks,
};

fn world_attachments(
    world: &mut sunrise_client::world::World,
    holder: mod_api::Holder,
) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
    use mod_api::{AttachmentError, Holder};
    if holder == Holder::Level {
        return Ok(&mut world.attachments);
    }
    if let Holder::BlockEntity([x, y, z]) = holder {
        let pos = (x, y, z);
        let chunk = world
            .chunks
            .get(&(x >> 4, z >> 4))
            .ok_or(AttachmentError::MissingHolder)?;
        attachment_compound(
            chunk
                .saved
                .as_ref()
                .and_then(|e| e.block_entity_sources.iter().find(|(p, _)| *p == pos))
                .and_then(|(_, tag)| tag.get(ATTACHMENTS_KEY)),
        )?;
        let map = world
            .block_entities
            .attachments_mut(pos)
            .ok_or(AttachmentError::MissingHolder)?;
        world.modified.insert((x >> 4, z >> 4));
        return Ok(map);
    }
    let Holder::Chunk([x, z]) = holder else {
        return Err(AttachmentError::MissingHolder);
    };
    let map = world.chunks.attachments_mut(&(x, z))?;
    world.modified.insert((x, z));
    Ok(map)
}

/// The entity families whose saved maps bind an owning lifetime
/// (`bind_saved_entity_attachments`), so a first store publishes index rows for them.
///
/// Every other family stays unindexed on a first store until its saved binding lands: the value
/// is kept and saved, but no query visits it. Cushions, hooks, TNT, fangs, crystals, falling
/// blocks, rockets, tridents, spawners, area-effect clouds, fence knots, the dragon itself and
/// `Other` (markers, displays) are unindexed today (#995).
fn saved_binding_target(target: super::commands::Target) -> bool {
    use super::commands::Target;
    matches!(
        target,
        Target::Mob(_)
            | Target::Item(_)
            | Target::Orb(_)
            | Target::Projectile(_)
            | Target::Stand(_)
            | Target::Frame(_)
            | Target::Painting(_)
            | Target::Boat(_)
            | Target::Cart(_)
            | Target::DragonPart(_)
            | Target::General(_)
    )
}

impl Game<'_> {
    /// Attachment membership uses physical presence (except removed mobs), not the
    /// command resolver's pending-removal masks. Limit resolution to this level.
    fn attachment_entity_exists(&self, id: u32) -> bool {
        use sunrise_client::join_seq::residency::table::EntityLookup;
        matches!(
            self.resident_entities
                .table
                .borrow_mut()
                .entity_lookup(&self.dim.key(), id, |at| {
                    use sunrise_client::join_seq::residency::Kind;
                    match at.owner.kind {
                        Kind::Hook | Kind::Player => self
                            .locate_specialized_resident_owner(at)
                            .map(|mut source| {
                                // Hooks were counted by physical presence, regardless of pending
                                // removals. Players were not, but must still validate their row.
                                source.value.1 = at.owner.kind == Kind::Hook;
                                source
                            }),
                        _ => self.locate_resident_attachment_owner(at),
                    }
                }),
            EntityLookup::Found {
                value: (_, true),
                ..
            }
        )
    }

    /// Object ownership for persistence cleanup, never a level-query membership set.
    pub(super) fn retained_entity_ids(&self) -> std::collections::HashSet<u32> {
        let mut retained = self.live_entity_ids();
        retained.extend(self.persist.off_level.keys().copied());
        retained
    }

    /// The attachments of entities that left the level, dropped: each tick
    /// while any entity has some. Only those entities are looked at, and
    /// nothing else of the entities' memory changes.
    pub(super) fn prune_entity_attachments(&mut self) {
        let alive = self.retained_entity_ids();
        let gone: Vec<u32> = self
            .persist
            .uuids
            .keys()
            .copied()
            .filter(|id| self.entity_attachments.contains_key(id) && !alive.contains(id))
            .collect();
        for id in gone {
            self.forget_entity_initialization(mod_api::Holder::Entity(id));
            self.entity_attachments.remove(&id);
        }
    }

    pub(super) fn native_authoritative(&self) -> bool {
        self.is_logical_server()
    }

    /// Access a holder's attachment map, to lend to a mod: a server's, whose
    /// synced changes are sent at the pass's end, or a client's copy of what
    /// its server synced.
    pub(super) fn attachment_map(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        if self.client_attachment_entity_exists(holder) {
            self.initialize_entity_on_access(holder);
        }
        if self.native_authoritative() {
            self.world.attachment_syncs.insert(holder);
            return self.attachment_map_quiet(holder, access);
        }
        let Some(remote) = self.remote.as_mut() else {
            return Err(mod_api::AttachmentError::MissingHolder);
        };
        remote.attachments.entry(holder).or_default();
        self.bind_remote_attachment(holder);
        let remote = self.remote.as_mut().unwrap();
        Ok(remote.attachments.get_mut(&holder).unwrap())
    }

    /// `attachment_map` without recording the holder for the sync. Inside a transaction, the
    /// map is kept as it was on its first access, so an abort puts it back.
    pub(super) fn attachment_map_quiet(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        if !self.native_authoritative() {
            return Err(mod_api::AttachmentError::MissingHolder);
        }
        self.journal_attachments(holder);
        self.attachment_slot(holder, access)
    }

    /// Bind first-store contents to their real owner, including writes made before
    /// insertion by EntityConstructing listeners. Never invent a temporary owner.
    pub(super) fn bind_resident_entity_attachments(
        &mut self,
        id: u32,
    ) -> Result<(), mod_api::AttachmentError> {
        use mod_api::AttachmentError;
        let id = std::num::NonZeroU32::new(id).ok_or(AttachmentError::MissingHolder)?;
        if matches!(
            self.with_resident_slot(&self.dim.key(), id, |slot| slot.is_bound()),
            super::entity_residency::SlotAccess::Done(true)
        ) {
            return Ok(());
        }
        if !matches!(
            self.server_resident_entity(id.get()),
            sunrise_client::join_seq::residency::table::EntityLookup::Found {
                value,
                ..
            } if saved_binding_target(value)
        ) {
            return Ok(());
        }
        if !self.entity_attachments.contains_key(&id.get()) {
            return Ok(());
        }
        // Cleanup owns per-level UUID metadata, while attachment maps are global.
        // Record the actual part identity so its level alone retires its map.
        if self.level_dragon_part(id.get()).is_some() {
            let uuid = self.uuid_for(id.get());
            self.persist.uuids.insert(id.get(), uuid);
        }
        let level = self
            .level(self.dim.id())
            .ok_or(AttachmentError::MissingHolder)?;
        let holder =
            mod_api::AttachmentHolder::Entity(mod_api::EntityRef::new(level.epoch(), id.get()));
        let index = self.attachment_indexes.entry(self.dim).or_default().clone();
        let mut map = self.entity_attachments.remove(&id.get()).unwrap();
        self.with_resident_slot(&self.dim.key(), id, |slot| {
            if !slot.is_bound() {
                slot.bind(&mut map, &index, holder);
            }
        });
        self.entity_attachments.insert(id.get(), map);
        Ok(())
    }

    /// Where `holder`'s attachment map lives now: an entity's by its object ID, a player's where
    /// their state is (swapped in for a guest's turn, or kept beside), the world's others.
    pub(super) fn attachment_slot(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        use mod_api::AttachmentError;
        if let mod_api::Holder::Entity(id) = holder {
            if self.entity_initialization.active != Some(holder) {
                use super::entity_residency::SlotAccess;
                use sunrise_client::join_seq::residency::Kind;
                let id = std::num::NonZeroU32::new(id).ok_or(AttachmentError::MissingHolder)?;
                match self.with_resident_slot(&self.dim.key(), id, |_| ()) {
                    SlotAccess::Done(()) => {}
                    SlotAccess::Taken => return Err(AttachmentError::HolderBusy),
                    SlotAccess::Missing => return Err(AttachmentError::MissingHolder),
                    SlotAccess::UnsupportedKind(kind) => {
                        let name = match kind {
                            Kind::Mob => "mob",
                            Kind::Item => "item",
                            Kind::Orb => "orb",
                            Kind::Projectile => "projectile",
                            Kind::Stand => "armorstand",
                            Kind::Frame => "itemframe",
                            Kind::Painting => "painting",
                            Kind::Boat => "boat",
                            Kind::Cart => "minecart",
                            Kind::Other => "other",
                            Kind::Fangs => "fangs",
                            Kind::Crystal => "crystal",
                            Kind::Dragon => "dragon",
                            Kind::Tnt => "tnt",
                            Kind::Falling => "falling",
                            Kind::Rocket => "rocket",
                            Kind::Trident => "trident",
                            Kind::Spawner => "spawner",
                            Kind::Cloud => "cloud",
                            Kind::Cushion => "cushion",
                            Kind::Knot => "knot",
                            Kind::Player => "player",
                            Kind::Hook => "hook",
                            Kind::General => "general",
                            Kind::DragonPart => "dragonpart",
                        };
                        return Err(AttachmentError::UnsupportedHolder(name));
                    }
                }
            }
            if self.entity_initialization.active != Some(holder)
                && !self.attachment_entity_exists(id)
            {
                return Err(AttachmentError::MissingHolder);
            }
            self.initialize_entity_on_access(holder);
            attachment_compound(
                self.persist
                    .raw
                    .get(&id)
                    .and_then(|tag| tag.get(ATTACHMENTS_KEY)),
            )?;
            if access == AttachmentAccess::Store
                && self
                    .persist
                    .removals
                    .contains(&super::commands::Target::Painting(id))
            {
                return Err(AttachmentError::MissingHolder);
            }
            if access == AttachmentAccess::Store
                && self.entity_initialization.active != Some(holder)
            {
                self.entity_attachments.entry(id).or_default();
                self.bind_resident_entity_attachments(id)?;
            }
            return Ok(self.entity_attachments.entry(id).or_default());
        }
        if let mod_api::Holder::Player(uuid) = holder {
            let cloning = self.cloning_guest(uuid).map(|guest| guest.conn);
            if self.entity_initialization.active != Some(holder)
                && cloning.is_none()
                && self.lan_acting_uuid != Some(uuid)
                && self
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.holder_guest(uuid, self.dim.id()))
                    .is_none()
            {
                return Err(AttachmentError::MissingHolder);
            }
            self.initialize_entity_on_access(holder);
            if self.lan_acting_uuid == Some(uuid) {
                if access == AttachmentAccess::Store
                    && let Some(conn) = self.lan_acting
                {
                    self.bind_player_attachments(conn, true);
                }
                return Ok(&mut self.player_state.attachments);
            }
            let conn = cloning
                .or_else(|| {
                    let host = self.lan.as_ref()?;
                    let guest = if self.entity_initialization.active == Some(holder) {
                        host.hub.guests.iter().find(|guest| guest.uuid == uuid)
                    } else {
                        host.hub.holder_guest(uuid, self.dim.id())
                    }?;
                    Some(guest.conn)
                })
                .ok_or(AttachmentError::MissingHolder)?;
            if self.lan_guest_swaps.contains(&conn) {
                return Err(AttachmentError::MissingHolder);
            }
            if access == AttachmentAccess::Store {
                self.bind_player_attachments(conn, true);
            }
            return Ok(self
                .lan
                .as_mut()
                .and_then(|host| host.hub.guests.attachments_mut(conn))
                .ok_or(AttachmentError::MissingHolder)?);
        }
        world_attachments(&mut self.world, holder)
    }
}

/// A level a both-side level hook reads, which knows the side it is on.
pub(super) trait SidedLevel: mod_api::LevelHost {
    fn client_side(&self) -> bool;
}

impl SidedLevel for super::GameLevel<'_> {
    fn client_side(&self) -> bool {
        sunrise_world::level::LevelAccess::is_client_side(self)
    }
}

impl<
    'a: 'b,
    'b,
    L: std::ops::Deref<Target = super::GameLevel<'a>>,
    T: std::ops::Deref<Target = Ticks>,
> SidedLevel for LevelTicks<'b, L, T>
{
    fn client_side(&self) -> bool {
        self.level.client_side()
    }
}

impl SidedLevel for super::native_hooks::HitLevel<'_> {
    fn client_side(&self) -> bool {
        self.0.client_side()
    }
}

pub(super) fn level_side(level: &impl SidedLevel) -> mod_api::LevelSide<'_> {
    if level.client_side() {
        mod_api::LevelSide::Client(level)
    } else {
        mod_api::LevelSide::Server(level)
    }
}

/// The prediction's block states and the live block entities it invalidated.
pub(in crate::game) trait PredictedLevel:
    sunrise_world::level::LevelAccess
{
    fn block_entity_removed(&self, pos: BlockPos) -> bool;
}

/// The level read by fire-odds listeners, including edits already made in a dry prediction.
pub(in crate::game) struct FireOddsLevel<'a> {
    pub(in crate::game) base: &'a super::GameLevel<'a>,
    pub(in crate::game) prediction: Option<&'a dyn PredictedLevel>,
}

impl FireOddsLevel<'_> {
    fn block_entity_removed(&self, [x, y, z]: [i32; 3]) -> bool {
        self.prediction
            .is_some_and(|level| level.block_entity_removed(BlockPos::new(x, y, z)))
    }

    fn block(&self, [x, y, z]: [i32; 3]) -> sunrise_world::registry::BlockStateId {
        let pos = BlockPos::new(x, y, z);
        match self.prediction {
            Some(level) => level.block_state(pos),
            None => sunrise_world::level::LevelAccess::block_state(self.base, pos),
        }
    }
}

impl SidedLevel for FireOddsLevel<'_> {
    fn client_side(&self) -> bool {
        self.base.client_side()
    }
}

// Prediction uses the same implementation for reads not affected by staged blocks.
macro_rules! base_level_reads {
    ($(fn $method:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(fn $method(&self, $($arg: $ty),*) -> $ret {
            mod_api::LevelHost::$method(self.base, $($arg),*)
        })*
    };
}

impl mod_api::LevelHost for FireOddsLevel<'_> {
    base_level_reads! {
        fn block_states() -> mod_api::BlockStates<'_>;
        fn container_size(at: mod_api::BlockSide) -> Option<usize>;
        fn container_stack(at: mod_api::BlockSide, slot: usize) -> Option<mod_api::ItemStack>;
        fn block_state_id(name: &str) -> Option<mod_api::BlockState>;
        fn block_state_default(block: mod_api::BlockKey) -> Option<mod_api::BlockState>;
        fn state_block(state: mod_api::BlockState) -> Option<&str>;
        fn state_string(state: mod_api::BlockState) -> Option<String>;
        fn min_y() -> i32;
        fn height() -> i32;
        fn block_in_tag(block: mod_api::BlockKey, tag: &str) -> Option<bool>;
        fn tags() -> &dyn mod_api::Tags;
        fn state_fluid_state(state: mod_api::BlockState) -> Option<mod_api::FluidState>;
        fn fluid_in_tag(fluid: &str, tag: &str) -> Option<bool>;
        fn state_with_property(state: mod_api::BlockState, key: &str, value: &str) -> Option<mod_api::BlockState>;
        fn brightness(layer: mod_api::LightLayer, pos: [i32; 3]) -> i32;
        fn raw_brightness(pos: [i32; 3], sky_darken: i32) -> i32;
        fn is_loaded(pos: [i32; 3]) -> bool;
        fn enabled_features() -> mod_api::FeatureFlags;
    }

    fn has_item_capability(&self, pos: [i32; 3], side: Option<mod_api::Direction>) -> Option<bool> {
        let level = self.base.api_level?;
        let at = mod_api::BlockSide { level, pos, side };
        let site = self.base.containers().capability_site(
            at,
            self.block(pos),
            self.block_entity_removed(pos),
        )?;
        Some(mod_api::__private::item_capability_present(self, site))
    }
    fn block_entity(&self, pos: [i32; 3]) -> Option<mod_api::BlockEntityRef> {
        if self.block_entity_removed(pos) {
            return None;
        }
        mod_api::LevelHost::block_entity(self.base, pos)
    }
    fn attachment_read_scope(&self) -> mod_api::AttachmentReadScope<'_> {
        mod_api::LevelHost::attachment_read_scope(self.base)
    }
    fn block_entity_part_snapshot(
        &self,
        entity: mod_api::BlockEntityRef,
        namespace: &str,
        name: &str,
    ) -> Result<mod_api::AttachmentBatch, mod_api::PartStateError> {
        use mod_api::__private::game::Fields;
        let fields = entity.fields();
        // The live reader still diagnoses handles from another level.
        if self.base.api_level == Some(fields.level) && self.block_entity_removed(fields.position) {
            return Err(mod_api::PartStateError::Stale);
        }
        mod_api::LevelHost::block_entity_part_snapshot(self.base, entity, namespace, name)
    }
    fn block_entity_data(&self, entity: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        use mod_api::__private::game::Fields;
        if self.block_entity_removed(entity.fields().position) {
            return None;
        }
        mod_api::LevelHost::block_entity_data(self.base, entity)
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        self.block(pos).try_into_api().ok()
    }
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        if !self.is_loaded(pos) {
            return None;
        }
        super::native_world_read::state_fluid(
            self.base.behaviours,
            self.base.registry,
            self.block(pos),
        )
    }
    fn collision_shape(&self, pos: [i32; 3]) -> Option<Vec<[f64; 6]>> {
        if !self.is_loaded(pos) {
            return None;
        }
        let [x, y, z] = pos;
        let state = self.block(pos);
        let binding = self.base.collision;
        let world = &*self.base.world;
        if !self.block_entity_removed(pos)
            && (world.block(x, y, z) == state || world.block_entities.contains_key(&(x, y, z)))
        {
            // Use the staged state but preserve the live entity's geometry when it survives.
            return Some(super::native_state_shapes::collision_boxes(
                binding, world, state, pos,
            ));
        }
        let mut scene = sunrise_client::collisions::LookupCollisions {
            block: &|x, y, z| self.block([x, y, z]),
            binding,
        };
        let shape = binding.collision_shape_with_context(
            &mut scene,
            state,
            BlockPos::new(x, y, z),
            &sunrise_world::collision_context::CollisionContext::empty(false),
        );
        Some(shape.map_or_else(
            |_| Vec::new(),
            |shape| {
                shape
                    .boxes()
                    .iter()
                    .map(|bounds| {
                        let min = bounds.min_position();
                        let max = bounds.max_position();
                        [min.x, min.y, min.z, max.x, max.y, max.z]
                    })
                    .collect()
            },
        ))
    }
    fn is_raining_at(&self, [x, y, z]: [i32; 3]) -> bool {
        if let Some(level) = self.prediction {
            self.is_loaded([x, y, z])
                && self
                    .base
                    .behaviours
                    .is_raining_at(level, BlockPos::new(x, y, z))
        } else {
            mod_api::LevelHost::is_raining_at(self.base, [x, y, z])
        }
    }
}

impl mod_api::LevelHost for super::GameLevel<'_> {
    fn block_states(&self) -> mod_api::BlockStates<'_> {
        mod_api::BlockStates::of(&self.tables)
    }
    fn block_face_sturdy(
        &self,
        pos: [i32; 3],
        face: mod_api::BlockFace,
        support: mod_api::SupportType,
    ) -> Option<bool> {
        if !self.native_authoritative {
            self.api_level?;
        }
        super::native_world_read::block_face_sturdy(
            self.world,
            self.behaviours,
            self.collision,
            pos,
            face,
            support,
        )
    }
    fn can_see_sky(&self, pos: [i32; 3]) -> Option<bool> {
        if !self.native_authoritative {
            self.api_level?;
        }
        Some(super::native_world_read::can_see_sky(self.world, pos))
    }
    fn has_item_capability(&self, pos: [i32; 3], side: Option<mod_api::Direction>) -> Option<bool> {
        let level = self.api_level?;
        let [x, y, z] = pos;
        let at = mod_api::BlockSide { level, pos, side };
        let site = self
            .containers()
            .capability_site(at, self.world.block(x, y, z), false)?;
        Some(mod_api::__private::item_capability_present(self, site))
    }
    fn block_entity(&self, pos: [i32; 3]) -> Option<mod_api::BlockEntityRef> {
        super::native_block_entity_refs::world_block_entity(self.world, self.api_level?, pos)
    }
    fn container_size(&self, at: mod_api::BlockSide) -> Option<usize> {
        if !self.native_authoritative
            || self.api_level != Some(at.level)
            || !self.world.is_loaded_at(at.pos[0], at.pos[1], at.pos[2])
        {
            return None;
        }
        self.containers().capability_size(at)
    }
    fn container_stack(&self, at: mod_api::BlockSide, slot: usize) -> Option<mod_api::ItemStack> {
        if !self.native_authoritative
            || self.api_level != Some(at.level)
            || !self.world.is_loaded_at(at.pos[0], at.pos[1], at.pos[2])
        {
            return None;
        }
        self.containers()
            .capability_stack(at, slot)?
            .try_into_api()
            .ok()
    }
    fn attachment_read_scope(&self) -> mod_api::AttachmentReadScope<'_> {
        mod_api::__private::game::attachment_read_scope(self.attachment_query_scopes)
    }
    fn block_entity_part_snapshot(
        &self,
        entity: mod_api::BlockEntityRef,
        namespace: &str,
        name: &str,
    ) -> Result<mod_api::AttachmentBatch, mod_api::PartStateError> {
        super::native_block_entity_refs::part_snapshot(
            self.world,
            self.api_level,
            self.native_authoritative,
            self.part_state_index,
            self.attachment_query_scopes,
            entity,
            (namespace, name),
        )
    }
    fn block_entity_data(&self, entity: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        use mod_api::__private::game::Fields;
        let fields = entity.fields();
        if self.api_level != Some(fields.level) {
            return None;
        }
        let [x, y, z] = fields.position;
        let key = (x, y, z);
        if self.world.block_entities.generation(&key) != Some(fields.generation) {
            return None;
        }
        let entity = self.world.block_entities.get(&key)?;
        let name = &self.facts.get(self.world.block(x, y, z)).name;
        super::native_block_entity_refs::save_without_metadata(
            self.world, key, entity, name, self.items,
        )
    }

    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        super::native_block_states::parse(self.block_registries, self.registry, name)?
            .try_into_api()
            .ok()
    }
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        super::api_values::default_state(block)?.try_into_api().ok()
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        sunrise_world::level::LevelAccess::block_state(self, BlockPos::new(pos[0], pos[1], pos[2]))
            .try_into_api()
            .ok()
    }
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        self.native_state_block(state.try_into_engine(&()).ok()?)
    }
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        self.registry
            .state(state.try_into_engine(&()).ok()?)
            .map(super::native_block_states::serialize)
    }
    fn min_y(&self) -> i32 {
        self.world.shape.min_y
    }
    fn height(&self) -> i32 {
        self.world.shape.height
    }
    fn block_in_tag(&self, block: mod_api::BlockKey, tag: &str) -> Option<bool> {
        if !self.native_authoritative {
            return None;
        }
        super::native_world_read::block_in_tag(block, tag)
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        self
    }
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        let [x, y, z] = pos;
        if !(self.world.shape.contains_y(y) && self.world.loaded(x, z)) {
            return None;
        }
        super::native_world_read::state_fluid(
            self.behaviours,
            self.registry,
            self.world.block(x, y, z),
        )
    }
    fn state_fluid_state(&self, state: mod_api::BlockState) -> Option<mod_api::FluidState> {
        super::native_world_read::state_fluid(
            self.behaviours,
            self.registry,
            state.try_into_engine(&()).ok()?,
        )
    }
    fn collision_shape(&self, pos: [i32; 3]) -> Option<Vec<[f64; 6]>> {
        let [x, y, z] = pos;
        if !(self.world.shape.contains_y(y) && self.world.loaded(x, z)) {
            return None;
        }
        Some(super::native_state_shapes::collision_boxes(
            self.collision,
            self.world,
            self.world.block(x, y, z),
            pos,
        ))
    }
    fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool> {
        self.native_authoritative
            .then(|| super::native_world_read::fluid_in_tag(self.items, fluid, tag))
    }
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        self.native_state_with_property(state.try_into_engine(&()).ok()?, key, value)?
            .try_into_api()
            .ok()
    }
    fn brightness(&self, layer: mod_api::LightLayer, pos: [i32; 3]) -> i32 {
        super::native_level_refs::world_brightness(self.world, layer, pos)
    }
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        sunrise_world::level::raw_brightness(
            self,
            BlockPos::new(pos[0], pos[1], pos[2]),
            sky_darken,
        )
    }
    fn is_loaded(&self, pos: [i32; 3]) -> bool {
        self.world.is_loaded_at(pos[0], pos[1], pos[2])
    }
    fn is_raining_at(&self, pos: [i32; 3]) -> bool {
        self.world.is_loaded_at(pos[0], pos[1], pos[2])
            && self
                .behaviours
                .is_raining_at(self, BlockPos::new(pos[0], pos[1], pos[2]))
    }
    fn enabled_features(&self) -> mod_api::FeatureFlags {
        mod_api::FeatureFlags::new(self.enabled_features.clone())
    }
}

impl mod_api::AttachmentHost for Game<'_> {
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), mod_api::AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, sync)
    }
    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        self.attachment_map(holder, access)
    }
}

impl mod_api::AttachmentHost for super::GameLevel<'_> {
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), mod_api::AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, sync)
    }
    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        _: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        if !self.native_authoritative {
            return Err(mod_api::AttachmentError::MissingHolder);
        }
        self.world.attachment_syncs.insert(holder);
        world_attachments(self.world, holder)
    }
}

impl mod_api::LevelRandomHost for super::GameLevel<'_> {
    fn container_signal(&mut self, pos: [i32; 3]) -> Option<i32> {
        if !self.native_authoritative || !self.world.is_loaded_at(pos[0], pos[1], pos[2]) {
            return None;
        }
        Some(super::machines::container_signal(
            self,
            BlockPos::new(pos[0], pos[1], pos[2]),
        ))
    }
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        self
    }
    fn lend_random(&mut self, draw: mod_api::RandomDraw<'_>) {
        if self.native_authoritative {
            super::api_values::with_live_level_random(&mut self.world.random, draw);
        }
    }
}

impl mod_api::LevelHost for super::native_hooks::HitLevel<'_> {
    fn block_entity_data(&self, _: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        None
    }

    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        super::native_block_states::parse(self.0.block_registries()?, self.0.registry(), name)?
            .try_into_api()
            .ok()
    }
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        self.0.block_registries()?;
        super::api_values::default_state(block)?.try_into_api().ok()
    }
    fn block_in_tag(&self, block: mod_api::BlockKey, tag: &str) -> Option<bool> {
        self.0.tags()?;
        super::native_world_read::block_in_tag(block, tag)
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        self
    }
    fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool> {
        Some(super::native_world_read::fluid_in_tag(
            self.0.tags()?,
            fluid,
            tag,
        ))
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        self.0.block(pos).try_into_api().ok()
    }
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        self.0
            .registry()
            .state(state.try_into_engine(&()).ok()?)
            .map(|s| &*s.name)
    }
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        self.0
            .registry()
            .state(state.try_into_engine(&()).ok()?)
            .map(super::native_block_states::serialize)
    }
    fn min_y(&self) -> i32 {
        self.0.shape().min_y
    }
    fn height(&self) -> i32 {
        self.0.shape().height
    }
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        super::native_hooks::with_property(
            self.0.registry(),
            state.try_into_engine(&()).ok()?,
            key,
            value,
        )?
        .try_into_api()
        .ok()
    }
}

impl mod_api::AttachmentHost for super::native_hooks::HitLevel<'_> {
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), mod_api::AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, sync)
    }
    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        let _ = access;
        self.0.attachments(holder)
    }
}

impl mod_api::LevelRandomHost for super::native_hooks::HitLevel<'_> {
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        self
    }
    fn lend_random(&mut self, draw: mod_api::RandomDraw<'_>) {
        super::api_values::with_live_level_random(self.0.random(), draw);
    }
}

impl mod_api::HitHost for super::native_hooks::HitLevel<'_> {
    fn play_sound_at(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: mod_api::SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        super::native_hooks::mod_sound_ok(sound, at, source.index(), volume, pitch)
            && self
                .0
                .hit_effects()
                .is_some_and(|hit| hit.play(sound, at, source, volume, pitch, None))
    }
    fn play_sound_from(
        &mut self,
        entity: mod_api::EntityRef,
        sound: &str,
        source: mod_api::SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        let Some(hit) = self.0.hit_effects() else {
            return false;
        };
        let Some((id, at, _)) = hit.entity(entity) else {
            return false;
        };
        super::native_hooks::mod_sound_ok(sound, at, source.index(), volume, pitch)
            && hit.play(sound, at, source, volume, pitch, Some(id))
    }
    fn entity_next_float(&mut self, entity: mod_api::EntityRef) -> Option<f32> {
        use sunrise_core::random::RandomSource;
        Some(self.0.hit_effects()?.entity(entity)?.2.next_float())
    }
    fn entity_next_double(&mut self, entity: mod_api::EntityRef) -> Option<f64> {
        use sunrise_core::random::RandomSource;
        Some(self.0.hit_effects()?.entity(entity)?.2.next_double())
    }
    fn entity_next_bool(&mut self, entity: mod_api::EntityRef) -> Option<bool> {
        use sunrise_core::random::RandomSource;
        Some(self.0.hit_effects()?.entity(entity)?.2.next_boolean())
    }
    fn entity_next_int(&mut self, entity: mod_api::EntityRef, bound: i32) -> Option<i32> {
        use sunrise_core::random::RandomSource;
        self.0
            .hit_effects()?
            .entity(entity)?
            .2
            .next_int_bound(bound)
            .ok()
    }
}

/// A level with the simulation's ticks beside it: `&mut Ticks` in a write or tick hook, `&Ticks`
/// where the listener only reads (placement and survival: [`LevelView`](super::native_hooks::LevelView)).
/// Reads need only a shared view of the ticks; scheduling and writes need them mutable.
pub(in crate::game) struct LevelTicks<'b, L, T = &'b mut Ticks> {
    level: L,
    ticks: T,
    unpack_container_loot: bool,
    /// The blocks a preview standing in for the level changed, which the reads see over it.
    preview: Option<&'b sunrise_world::level::PreviewBlocks>,
    /// The level's and the ticks' borrow, which the delegating reads below are bounded by.
    borrow: std::marker::PhantomData<&'b ()>,
}

impl<'b, L, T> LevelTicks<'b, L, T> {
    pub(in crate::game) fn new(level: L, ticks: T) -> Self {
        Self {
            level,
            ticks,
            unpack_container_loot: true,
            preview: None,
            borrow: std::marker::PhantomData,
        }
    }

    /// `place_liquid` reads stored slots whether it uses a direct or journalled host.
    pub(in crate::game) fn without_container_loot(mut self) -> Self {
        self.unpack_container_loot = false;
        self
    }

    /// This level as the preview that changed `preview` sees it (`None`: as it is).
    pub(in crate::game) fn previewing(
        mut self,
        preview: Option<&'b sunrise_world::level::PreviewBlocks>,
    ) -> Self {
        self.preview = preview;
        self
    }
    /// The block the preview put at `pos`, if it changed it.
    fn previewed(&self, pos: BlockPos) -> Option<sunrise_world::registry::BlockStateId> {
        self.preview?.get(&pos).copied()
    }
}

impl<
    'a: 'b,
    'b,
    L: std::ops::Deref<Target = super::GameLevel<'a>>,
    T: std::ops::Deref<Target = Ticks>,
> mod_api::LevelHost for LevelTicks<'b, L, T>
{
    fn block_states(&self) -> mod_api::BlockStates<'_> {
        mod_api::LevelHost::block_states(&*self.level)
    }
    fn block_face_sturdy(
        &self,
        pos: [i32; 3],
        face: mod_api::BlockFace,
        support: mod_api::SupportType,
    ) -> Option<bool> {
        mod_api::LevelHost::block_face_sturdy(&*self.level, pos, face, support)
    }
    fn can_see_sky(&self, pos: [i32; 3]) -> Option<bool> {
        mod_api::LevelHost::can_see_sky(&*self.level, pos)
    }
    fn has_item_capability(&self, pos: [i32; 3], side: Option<Direction>) -> Option<bool> {
        let level = self.level.api_level?;
        let [x, y, z] = pos;
        let preview = self.previewed(BlockPos::new(x, y, z));
        let state = preview.unwrap_or_else(|| self.level.world.block(x, y, z));
        let site = self.level.containers().capability_site(
            mod_api::BlockSide { level, pos, side },
            state,
            preview.is_some(),
        )?;
        Some(mod_api::__private::item_capability_present(self, site))
    }
    fn block_entity(&self, pos: [i32; 3]) -> Option<mod_api::BlockEntityRef> {
        // A block the preview changed has no block entity left.
        if self
            .previewed(BlockPos::new(pos[0], pos[1], pos[2]))
            .is_some()
        {
            return None;
        }
        mod_api::LevelHost::block_entity(&*self.level, pos)
    }
    fn container_size(&self, at: mod_api::BlockSide) -> Option<usize> {
        if self
            .previewed(BlockPos::new(at.pos[0], at.pos[1], at.pos[2]))
            .is_some()
        {
            return None;
        }
        mod_api::LevelHost::container_size(&*self.level, at)
    }
    fn container_stack(&self, at: mod_api::BlockSide, slot: usize) -> Option<mod_api::ItemStack> {
        if self
            .previewed(BlockPos::new(at.pos[0], at.pos[1], at.pos[2]))
            .is_some()
        {
            return None;
        }
        mod_api::LevelHost::container_stack(&*self.level, at, slot)
    }
    fn attachment_read_scope(&self) -> mod_api::AttachmentReadScope<'_> {
        mod_api::LevelHost::attachment_read_scope(&*self.level)
    }
    fn block_entity_part_snapshot(
        &self,
        entity: mod_api::BlockEntityRef,
        namespace: &str,
        name: &str,
    ) -> Result<mod_api::AttachmentBatch, mod_api::PartStateError> {
        mod_api::LevelHost::block_entity_part_snapshot(&*self.level, entity, namespace, name)
    }
    fn block_entity_data(&self, entity: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
        use mod_api::__private::game::Fields;
        let [x, y, z] = entity.fields().position;
        if self.previewed(BlockPos::new(x, y, z)).is_some() {
            return None;
        }
        mod_api::LevelHost::block_entity_data(&*self.level, entity)
    }
    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::block_state_id(&*self.level, name)
    }
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::block_state_default(&*self.level, block)
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        match self.previewed(BlockPos::new(pos[0], pos[1], pos[2])) {
            Some(state) => state.try_into_api().ok(),
            None => mod_api::LevelHost::block_state(&*self.level, pos),
        }
    }
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        mod_api::LevelHost::state_block(&*self.level, state)
    }
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        mod_api::LevelHost::state_string(&*self.level, state)
    }
    fn min_y(&self) -> i32 {
        mod_api::LevelHost::min_y(&*self.level)
    }
    fn height(&self) -> i32 {
        mod_api::LevelHost::height(&*self.level)
    }
    fn block_in_tag(&self, block: mod_api::BlockKey, tag: &str) -> Option<bool> {
        mod_api::LevelHost::block_in_tag(&*self.level, block, tag)
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        mod_api::LevelHost::tags(&*self.level)
    }
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        match self.previewed(BlockPos::new(pos[0], pos[1], pos[2])) {
            Some(state) => self.state_fluid_state(state.try_into_api().ok()?),
            None => mod_api::LevelHost::fluid_state(&*self.level, pos),
        }
    }
    fn state_fluid_state(&self, state: mod_api::BlockState) -> Option<mod_api::FluidState> {
        mod_api::LevelHost::state_fluid_state(&*self.level, state)
    }
    fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool> {
        mod_api::LevelHost::fluid_in_tag(&*self.level, fluid, tag)
    }
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::state_with_property(&*self.level, state, key, value)
    }
    fn brightness(&self, layer: mod_api::LightLayer, pos: [i32; 3]) -> i32 {
        mod_api::LevelHost::brightness(&*self.level, layer, pos)
    }
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        mod_api::LevelHost::raw_brightness(&*self.level, pos, sky_darken)
    }
    fn is_loaded(&self, pos: [i32; 3]) -> bool {
        mod_api::LevelHost::is_loaded(&*self.level, pos)
    }
    fn is_raining_at(&self, pos: [i32; 3]) -> bool {
        mod_api::LevelHost::is_raining_at(&*self.level, pos)
    }
    fn enabled_features(&self) -> mod_api::FeatureFlags {
        mod_api::LevelHost::enabled_features(&*self.level)
    }
    fn signal(&self, pos: [i32; 3], direction: Direction) -> Option<i32> {
        self.read_signals(|s| s.signal(BlockPos::new(pos[0], pos[1], pos[2]), direction))
    }
    fn direct_signal(&self, pos: [i32; 3], direction: Direction) -> Option<i32> {
        self.read_signals(|s| s.direct_signal(BlockPos::new(pos[0], pos[1], pos[2]), direction))
    }
    fn direct_signal_to(&self, pos: [i32; 3]) -> Option<i32> {
        self.read_signals(|s| s.direct_signal_to(BlockPos::new(pos[0], pos[1], pos[2])))
    }
    fn best_neighbor_signal(&self, pos: [i32; 3]) -> Option<i32> {
        self.read_signals(|s| s.best_neighbor_signal(BlockPos::new(pos[0], pos[1], pos[2])))
    }
    fn has_neighbor_signal(&self, pos: [i32; 3]) -> Option<bool> {
        self.read_signals(|s| s.has_neighbor_signal(BlockPos::new(pos[0], pos[1], pos[2])))
    }
    fn collision_shape(&self, pos: [i32; 3]) -> Option<Vec<[f64; 6]>> {
        let Some(state) = self.previewed(BlockPos::new(pos[0], pos[1], pos[2])) else {
            return mod_api::LevelHost::collision_shape(&*self.level, pos);
        };
        Some(super::native_state_shapes::collision_boxes(
            self.level.collision,
            self.level.world,
            state,
            pos,
        ))
    }
}

impl<'a: 'b, 'b, T: std::ops::Deref<Target = Ticks>> mod_api::LevelRandomHost
    for LevelTicks<'b, &mut super::GameLevel<'a>, T>
{
    fn container_signal(&mut self, pos: [i32; 3]) -> Option<i32> {
        if self.unpack_container_loot {
            return mod_api::LevelRandomHost::container_signal(&mut *self.level, pos);
        }
        let [x, y, z] = pos;
        (self.level.native_authoritative && self.level.world.is_loaded_at(x, y, z))
            .then(|| self.level.containers().stored_container_signal((x, y, z)))
    }
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        mod_api::LevelRandomHost::attachments(&mut *self.level)
    }
    fn lend_random(&mut self, draw: mod_api::RandomDraw<'_>) {
        mod_api::LevelRandomHost::lend_random(&mut *self.level, draw);
    }
}

impl<
    'a: 'b,
    'b,
    L: std::ops::Deref<Target = super::GameLevel<'a>>,
    T: std::ops::DerefMut<Target = Ticks>,
> mod_api::LevelTicksHost for LevelTicks<'b, L, T>
{
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: mod_api::BlockKey, delay: i32) -> bool {
        self.level
            .native_schedule_block_tick(&mut self.ticks, pos, block, delay)
    }
    fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool {
        self.level
            .native_schedule_fluid_tick(&mut self.ticks, pos, fluid, delay)
    }
}

impl<
    'a: 'b,
    'b,
    L: std::ops::Deref<Target = super::GameLevel<'a>>,
    T: std::ops::Deref<Target = Ticks>,
> LevelTicks<'b, L, T>
{
    /// A read of the level in place (over the preview's blocks) through the game's own
    /// `SignalGetter`, with these ticks' comparator outputs. A client's ticks don't hold the
    /// server's outputs: a read that needed one it lacks is unknown there, `None`.
    fn read_signals<R>(
        &self,
        read: impl FnOnce(&sunrise_world::level::Signals<'_, Self>) -> R,
    ) -> Option<R> {
        let behaviours = self.level.behaviours;
        let signals = if self.level.native_authoritative && self.preview.is_some() {
            behaviours.uncached_signals(self, &self.ticks)
        } else {
            behaviours.signals(self, &self.ticks)
        };
        let value = read(&signals);
        (!(self.level.client_side && signals.read_unknown_output())).then_some(value)
    }
}

/// The signal reads of the level in place, over the preview's blocks: a block the preview
/// changed has no block entity left, as `DryLevel` reads it.
impl<
    'a: 'b,
    'b,
    L: std::ops::Deref<Target = super::GameLevel<'a>>,
    T: std::ops::Deref<Target = Ticks>,
> sunrise_world::level::signal_access::SignalAccess for LevelTicks<'b, L, T>
{
    fn block_state(&self, pos: BlockPos) -> sunrise_world::registry::BlockStateId {
        self.previewed(pos)
            .unwrap_or_else(|| sunrise_world::level::LevelAccess::block_state(&*self.level, pos))
    }
    fn shulker_is_closed(&self, pos: BlockPos) -> Option<bool> {
        if self.previewed(pos).is_some() {
            return None;
        }
        sunrise_world::level::LevelAccess::shulker_is_closed(&*self.level, pos)
    }
    fn published_collision_shape(
        &self,
        pos: BlockPos,
        state: sunrise_world::registry::BlockStateId,
    ) -> Option<sunrise_core::geometry::VoxelShape> {
        if self.previewed(pos).is_some() {
            return None;
        }
        sunrise_world::level::LevelAccess::published_collision_shape(&*self.level, pos, state)
    }
    fn adjust_signal_source(
        &self,
        state: sunrise_world::registry::BlockStateId,
        vanilla: bool,
    ) -> bool {
        sunrise_world::level::LevelAccess::adjust_signal_source(&*self.level, state, vanilla)
    }
    fn has_weak_power_hook(&self) -> bool {
        sunrise_world::level::LevelAccess::has_weak_power_hook(&*self.level)
    }
    fn adjust_should_check_weak_power(
        &self,
        pos: BlockPos,
        state: sunrise_world::registry::BlockStateId,
        direction: Direction,
        vanilla: bool,
    ) -> bool {
        sunrise_world::level::LevelAccess::adjust_should_check_weak_power(
            &*self.level,
            pos,
            state,
            direction,
            vanilla,
        )
    }
    fn has_redstone_wire_connection_hook(&self) -> bool {
        sunrise_world::level::LevelAccess::has_redstone_wire_connection_hook(&*self.level)
    }
    fn adjust_should_redstone_wire_connect_to(
        &self,
        pos: BlockPos,
        state: sunrise_world::registry::BlockStateId,
        direction: Option<Direction>,
        vanilla: bool,
    ) -> bool {
        sunrise_world::level::LevelAccess::adjust_should_redstone_wire_connect_to(
            &*self.level,
            pos,
            state,
            direction,
            vanilla,
        )
    }
    fn adjust_signal(
        &self,
        pos: BlockPos,
        state: sunrise_world::registry::BlockStateId,
        direction: Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32 {
        sunrise_world::level::LevelAccess::adjust_signal(
            &*self.level,
            pos,
            state,
            direction,
            direct,
            vanilla,
        )
    }
    fn block_entity_signal(
        &self,
        pos: BlockPos,
        state: sunrise_world::registry::BlockStateId,
        direction: Direction,
    ) -> i32 {
        if self.previewed(pos).is_some() {
            return 0;
        }
        sunrise_world::level::LevelAccess::block_entity_signal(&*self.level, pos, state, direction)
    }
}

impl<'a: 'b, 'b, T: std::ops::DerefMut<Target = Ticks>> mod_api::LevelWriteHost
    for LevelTicks<'b, &mut super::GameLevel<'a>, T>
{
    fn set_block(
        &mut self,
        pos: [i32; 3],
        state: mod_api::BlockState,
        flags: mod_api::BlockUpdateFlags,
    ) -> bool {
        let Ok(state) = state.try_into_engine(&()) else {
            return false;
        };
        self.level
            .native_set_block(&mut self.ticks, pos, state, flags.bits())
    }

    fn remove_block(&mut self, pos: [i32; 3], moved: bool) -> bool {
        self.level.remove_block_write(&mut self.ticks, pos, moved)
    }

    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str, flags: i32) -> bool {
        self.level
            .native_set_block_property(&mut self.ticks, pos, key, value, flags)
    }

    fn play_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: usize,
        volume: f32,
        pitch: f32,
    ) -> bool {
        self.level
            .native_play_sound(sound, at, source, volume, pitch)
    }

    fn send_particles(
        &mut self,
        options: &mod_api::ParticleOptions,
        state: Option<mod_api::BlockState>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let Ok(state) = state.try_into_engine(&()) else {
            return false;
        };
        let Ok(options) = options.try_into_engine(self.level.items) else {
            return false;
        };
        self.level
            .native_send_particles(&options, state, at, spread, speed, count)
    }

    fn level_event(&mut self, id: i32, pos: [i32; 3], data: i32) -> bool {
        self.level.native_level_event(id, pos, data)
    }
}

super::native_tags::impl_tags!(
    super::GameLevel<'_>,
    level,
    level.native_authoritative.then_some(level.items)
);
super::native_tags::impl_tags!(super::native_hooks::HitLevel<'_>, level, level.0.tags());

#[cfg(test)]
mod attachment_membership_tests {
    use super::*;
    use crate::game::{commands::Target, native_entity_refs::tests::with_game};
    use sunrise_client::{join_seq::Joined, mobs::Mob};
    use sunrise_core::geometry::Vec3;

    fn old_membership(game: &Game<'_>, id: u32) -> bool {
        use crate::game::entity::{Entity as _, entity_lists_of, for_each_entity};
        let mut found = false;
        for_each_entity!(entity_lists_of!(game), entity => {
            found |= entity.id() == id && !entity.is_removed();
        });
        found || game.entity_ids_off_walk().any(|e| e == id) || game.level_dragon_part(id).is_some()
    }
    fn assert_membership(game: &mut Game<'_>, id: u32) {
        assert_eq!(
            game.attachment_entity_exists(id),
            old_membership(game, id),
            "id {id}"
        );
    }
    #[test]
    #[ignore = "daily"]
    fn attachment_off_walk_membership_matches_old_walk() {
        with_game(|game| {
            game.reset_entity_residency();
            macro_rules! summon {
                ($name:literal, $($field:ident).+) => {{
                    game.summon_with_nbt(concat!("minecraft:", $name), [0.5, 70.0, 0.5], "{}").unwrap();
                    game.lan_assign_ids();
                    let id = game$(.$field)+.last().unwrap().id;
                    assert_membership(game, id);
                }};
            }
            summon!("pig", mobs);
            summon!("item_frame", item_frames);
            summon!("painting", paintings);
            summon!("marker", others);
            summon!("leash_knot", others);
            summon!("area_effect_cloud", breath_clouds);
            summon!("end_crystal", crystals);
            summon!("cushion", cushions);
            game.add_fangs([0.5, 70.0, 0.5], 0.0, 0, 0);
            let id = game.fangs.list[0].id;
            assert_membership(game, id);
            game.add_ominous_item_spawner(
                sunrise_client::items::ItemStack::new("minecraft:stone", 1),
                [0.5, 70.0, 0.5],
                60,
            );
            let id = game.ominous_spawners[0].id;
            assert_membership(game, id);
            let fence = [0, 70, 0];
            game.tie_knot(fence);
            let mob = game.mobs[0].id;
            assert!(game.set_leash(
                Target::Mob(mob),
                sunrise_client::mobs::leash::Holder::Knot(fence),
                None
            ));
            let id = game.resident_entities.knots[0].id;
            assert_membership(game, id);
            // Also cover a player-tied natural knot, besides the explicitly summoned knot.
            let natural_fence = [3, 70, 3];
            assert!(game.set_leash(
                Target::Mob(mob),
                sunrise_client::mobs::leash::Holder::Player,
                None
            ));
            assert!(game.use_knot(natural_fence, false));
            let id = game.knot_ids[&natural_fence];
            assert_membership(game, id);
            let mut player =
                sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player.entity_id = 880;
            game.player = Some(player);
            game.bind_local_player_residency();
            let player_id = game.player.as_ref().unwrap().entity_id as u32;
            assert_membership(game, player_id);
            assert!(!old_membership(game, player_id));
            game.install_fishing_hook(crate::game::fishing::Hook::remote(
                900,
                Vec3::ZERO,
                [1, 2, 3, 4],
            ));
            assert_membership(game, 900);
            assert!(
                game.attachment_slot(mod_api::Holder::Entity(900), AttachmentAccess::Existing)
                    .is_ok()
            );
            game.persist.removals.push(Target::Hook(900));
            assert_membership(game, 900);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn attachment_guest_hook_membership_matches_old_walk() {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            for id in [12, 13] {
                assert_membership(game, id);
                assert!(
                    !old_membership(game, id),
                    "players are not entity attachment holders"
                );
            }
            game.as_guest(12, |game| {
                game.install_fishing_hook(crate::game::fishing::Hook::remote(
                    901,
                    Vec3::ZERO,
                    [5, 6, 7, 8],
                ));
                assert_membership(game, 901);
                assert!(
                    game.attachment_slot(mod_api::Holder::Entity(901), AttachmentAccess::Existing)
                        .is_ok()
                );
            })
            .unwrap();
            assert_membership(game, 901);
            assert!(
                game.attachment_slot(mod_api::Holder::Entity(901), AttachmentAccess::Existing)
                    .is_ok()
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn attachment_membership_matches_old_walk() {
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(410, kind, Vec3::ZERO, 0.0));
            let check = |game: &Game<'_>| {
                for id in [0, 410, 999] {
                    assert_eq!(
                        game.attachment_entity_exists(id),
                        old_membership(game, id),
                        "id {id}"
                    );
                }
            };
            check(game);
            game.paintings.push(sunrise_client::entities::Painting {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 420,
                pos: [0, 64, 0],
                facing: 2,
                variant: "minecraft:kebab".into(),
                width: 1,
                height: 1,
                check_interval: 0,
                ticks: 0,
            });
            game.persist.removals.push(Target::Painting(420));
            assert_eq!(
                game.attachment_entity_exists(420),
                old_membership(game, 420)
            );
            assert!(
                game.attachment_slot(mod_api::Holder::Entity(420), AttachmentAccess::Existing)
                    .is_ok()
            );
            assert!(
                game.attachment_slot(mod_api::Holder::Entity(420), AttachmentAccess::Store)
                    .is_err()
            );
            let join = game.mobs[0].join_seq();
            let taken = game.mobs.extract(join).unwrap();
            assert!(matches!(
                game.attachment_slot(mod_api::Holder::Entity(410), AttachmentAccess::Existing),
                Err(mod_api::AttachmentError::HolderBusy)
            ));
            game.mobs.put_back(0, taken);
            let mut dragon = crate::game::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 500;
            game.dragon.replace(dragon);
            for id in 500..=508 {
                assert_eq!(game.attachment_entity_exists(id), old_membership(game, id));
            }
            game.persist.removals.push(Target::Dragon(500));
            for id in 500..=508 {
                assert_eq!(game.attachment_entity_exists(id), old_membership(game, id));
            }
            game.persist.removals.push(Target::Mob(410));
            check(game);
            game.persist.destroying_vehicles.insert(Target::Mob(410));
            check(game);
            game.mobs.update(0, |mob| mob.removed = true);
            check(game);
            game.enter_level(crate::game::Dim::Nether).unwrap();
            check(game);
            game.enter_level(crate::game::Dim::Overworld).unwrap();
            let holder = mod_api::Holder::Entity(999);
            game.entity_initialization.active = Some(holder);
            assert!(
                game.attachment_slot(holder, AttachmentAccess::Existing)
                    .is_ok()
            );
            game.entity_initialization.active = None;
            assert!(
                game.attachment_slot(holder, AttachmentAccess::Existing)
                    .is_err()
            );
        });
    }
    #[test]
    #[ignore = "manual attachment scaling benchmark; needs SUNRISE_TEST_RESOURCES"]
    fn attachment_entity_read_bench() {
        with_game(|game| {
            game.reset_entity_residency();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            for population in [300, 3000, 10000] {
                while game.mobs.len() < population {
                    let id = game.mobs.len() as u32 + 100;
                    game.mobs.push(Mob::new(id, kind.clone(), Vec3::ZERO, 0.0));
                }
                let holder = mod_api::Holder::Entity(100);
                game.attachment_slot(holder, AttachmentAccess::Existing)
                    .unwrap();
                let start = std::time::Instant::now();
                for _ in 0..100_000 {
                    std::hint::black_box(
                        game.attachment_slot(holder, AttachmentAccess::Existing)
                            .unwrap(),
                    );
                }
                println!(
                    "attachment entity population={population} ns/read={:.2}",
                    start.elapsed().as_nanos() as f64 / 100_000.0
                );
            }
        });
    }
}
