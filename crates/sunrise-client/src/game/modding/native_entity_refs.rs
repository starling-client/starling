//! Entity and level handles for native server mods (docs/modding/src/how-to/native-mods.md,
//! "Entity and level handles", #7387): `entity(id)` turns an event's
//! entity id into an `EntityRef`, and each `native_entity_*` helper reads the
//! entity a handle names. They read where the world is simulated: on a client,
//! with no world open, or for a handle of an earlier open, they return `None`,
//! except the reads marked side `both` (`entity`, `level`, `entity_position`,
//! `entity_rotation`, `entity_velocity`), which on a client answer from its own
//! copy: the local player and the entities the server told it of, in the level
//! it shows. The entities are those of the level in place (the one a hook runs
//! in) and the server's players, whatever their level.
#[path = "native_entity_refs/death_loot.rs"]
mod death_loot;
#[path = "native_entity_refs/dragon_parts_table.rs"]
pub(in crate::game) mod dragon_parts_table;
#[path = "native_entity_refs/dragon_query.rs"]
mod dragon_query;
#[cfg(test)]
#[path = "native_entity_refs/level_uuid_tests.rs"]
mod level_uuid_tests;
#[cfg(test)]
#[path = "native_entity_refs/roll_loot_tests.rs"]
mod roll_loot_tests;
#[path = "native_entity_refs/save_lending.rs"]
mod save_lending;
use super::{
    Game,
    commands::Target,
    entity::Entity,
    pick_entity::{
        aabb_extents,
        pick_sections::{PickKind, PickMember},
    },
    remote::Remote,
    text_displays::{RemoteTextDisplay, Shows},
    transactions::After,
};
use core::ops::ControlFlow;
use mod_api::{
    BlockFace, BlockShapeKind, BlockUpdateFlags, ClipBlock, ClipError, ClipFluid, ClipHit,
    ClipOutcome, CollisionContext, DimensionKey, EntityRayHit, EntityReadError, EntityRef,
    LevelReadError, LevelRef, LevelWriteError, NeighbourUpdate,
};
use sunrise_client::join_seq::{
    JoinList, JoinSeq, Joined,
    residency::table::{EntityLookup, Location, SectionSearch},
};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_items::items::ItemStack;
use sunrise_world::registry::BlockStateId;

/// The engine's direction of a mod API block face.
pub(super) fn block_face_direction(face: BlockFace) -> sunrise_core::geometry::Direction {
    use sunrise_core::geometry::Direction;
    match face {
        BlockFace::Down => Direction::Down,
        BlockFace::Up => Direction::Up,
        BlockFace::North => Direction::North,
        BlockFace::South => Direction::South,
        BlockFace::West => Direction::West,
        BlockFace::East => Direction::East,
        _ => unreachable!("BlockFace: unhandled variant"),
    }
}

/// The mod API face of an engine direction.
fn api_face(direction: sunrise_core::geometry::Direction) -> BlockFace {
    use sunrise_core::geometry::Direction;
    match direction {
        Direction::Down => BlockFace::Down,
        Direction::Up => BlockFace::Up,
        Direction::North => BlockFace::North,
        Direction::South => BlockFace::South,
        Direction::West => BlockFace::West,
        Direction::East => BlockFace::East,
    }
}

/// Vanilla's miss result of a ray from `from` to `to` (`BlockHitResult.miss`): the end point, its
/// floored block and the face nearest the way back to `from`.
pub(super) fn miss_hit(from: [f64; 3], to: [f64; 3]) -> ClipHit {
    let [fx, fy, fz] = from;
    let [x, y, z] = to;
    ClipHit {
        miss: true,
        block: [x.floor() as i32, y.floor() as i32, z.floor() as i32],
        face: api_face(sunrise_core::geometry::Direction::approximate_nearest(
            Vec3::new(fx - x, fy - y, fz - z),
        )),
        location: to,
        inside: false,
        world_border: false,
    }
}

/// Where a level ray ended.
pub(super) enum ClipRay {
    /// A hit, or vanilla's miss result when `miss` is set.
    Reached(ClipHit),
    /// The first cell the ray entered inside a column that is not loaded.
    Unloaded([i32; 3]),
}

/// `Visibility.isAccessible`: a column is tracked once it is resident. A level
/// with no resident columns has no tracked entity sections.
pub(super) fn entity_section_accessible(
    chunks: &sunrise_client::chunk_map::ChunkMap,
    [x, _, z]: [f64; 3],
) -> bool {
    x.is_finite()
        && z.is_finite()
        && chunks.contains_key(&(
            sunrise_core::mth::floor(x) >> 4,
            sunrise_core::mth::floor(z) >> 4,
        ))
}

/// `ProjectileUtil.getEntityHitResult`'s test on a candidate: grow its box and clip it
/// against the segment. A segment starting inside the grown box finds no entry (`AABB.clip`).
fn clip_entity_ray(
    start: Vec3,
    end: Vec3,
    padding: Vec3,
    entity: EntityRef,
    [lo, hi]: [[f64; 3]; 2],
) -> Option<(EntityRayHit, f64)> {
    let grown = Aabb::new(
        Vec3::new(lo[0], lo[1], lo[2]),
        Vec3::new(hi[0], hi[1], hi[2]),
    )
    .inflate(padding);
    let location = grown.clip(start, end)?.components();
    let from = start.components();
    let distance_squared = (0..3).map(|i| (location[i] - from[i]).powi(2)).sum::<f64>();
    Some((
        EntityRayHit {
            entity,
            location,
            distance: distance_squared.sqrt(),
        },
        distance_squared,
    ))
}

fn list_query_body<T: Entity + Joined, const DIRECT: bool>(
    list: &JoinList<T, DIRECT>,
    join: JoinSeq,
) -> Option<Aabb> {
    let entity = list.get(list.locate(join)?)?;
    (!entity.is_removed()).then(|| entity.bounds())
}

impl Game<'_> {
    /// The window's live level and network id for a handle in its current world generation.
    pub(super) fn native_client_entity_context(
        &self,
        entity: EntityRef,
    ) -> Result<(&Remote, i32), EntityReadError> {
        let remote = self
            .remote
            .as_ref()
            .filter(|remote| remote.has_level())
            .ok_or(EntityReadError::Unavailable)?;
        let epoch = self
            .native_lifecycle
            .epoch()
            .ok_or(EntityReadError::Unavailable)?;
        if epoch != entity.epoch() {
            return Err(EntityReadError::Stale);
        }
        Ok((remote, entity.id() as i32))
    }

    /// An entity tracked by the remote client but absent from local entity lists.
    pub(super) fn native_client_tracked(&self, entity: EntityRef) -> Option<(&str, [f64; 3])> {
        let (remote, id) = self.native_client_entity_context(entity).ok()?;
        let (kind, position) = remote.tracked_entity(id)?;
        if Shows::from_kind(kind).is_some() {
            let display = self.live_client_display(remote, id)?;
            return Some((kind, display.position().to_array()));
        }
        Some((kind, position))
    }

    pub(super) fn native_client_display(&self, entity: EntityRef) -> Option<&RemoteTextDisplay> {
        let (remote, id) = self.native_client_entity_context(entity).ok()?;
        Shows::from_kind(remote.tracked(id)?.0)?;
        self.live_client_display(remote, id)
    }

    fn live_client_display(&self, remote: &Remote, id: i32) -> Option<&RemoteTextDisplay> {
        if !remote.has_level() {
            return None;
        }
        self.text_displays.get(&id)
    }

    /// What a handle names now, or None for a stale or foreign one.
    pub(super) fn native_entity_target(&self, entity: EntityRef) -> Option<Target> {
        self.native_entity_target_with_location(entity)
            .map(|(target, _)| target)
    }

    /// The server entity read boundary, distinguishing absent handles from parked bodies.
    pub(super) fn native_server_entity_target(
        &self,
        entity: EntityRef,
    ) -> Result<Target, EntityReadError> {
        self.resolve_server_entity_in(entity, self.dim.key())
            .map(|(target, _)| target)
    }

    /// Resolve the handle and return its resident location, if admitted to the level.
    pub(super) fn native_entity_target_with_location(
        &self,
        entity: EntityRef,
    ) -> Option<(Target, Option<Location>)> {
        self.resolve_server_entity_in(entity, self.dim.key()).ok()
    }

    /// [`Self::native_entity_target_with_location`] for an entity resident in the open server
    /// level `dimension`, the one in place or a parked one.
    fn resolve_server_entity_in(
        &self,
        entity: EntityRef,
        dimension: DimensionKey,
    ) -> Result<(Target, Option<Location>), EntityReadError> {
        if !self.is_logical_server() {
            return Err(EntityReadError::Unavailable);
        }
        if self.native_lifecycle.epoch() != Some(entity.epoch())
            || self.held_out.contains(&entity.id())
        {
            return Err(EntityReadError::Stale);
        }
        let EntityLookup::Found { location, value } = self.server_resident_entity(entity.id())
        else {
            // A lifecycle event lends the body without admitting it to the level.
            let conn = self
                .player_state
                .cloning_players
                .iter()
                .rev()
                .find(|player| player.entity == Some(entity.id()))
                .ok_or(EntityReadError::Stale)?
                .conn;
            return if self.lan_acting == Some(conn) {
                Ok((Target::Player, None))
            } else if self.lan_guest_swaps.contains(&conn) {
                Err(EntityReadError::Unavailable)
            } else {
                Ok((Target::Guest(conn), None))
            };
        };
        // Players are server-wide records. Ordinary body operations retain
        // their existing current-level domain; source membership is global.
        if !matches!(value, Target::Player | Target::Guest(_))
            && location.owner.dimension != dimension
        {
            return Err(EntityReadError::Unavailable);
        }
        Ok((value, Some(location)))
    }
    /// What a handle names on a client: the local player, or an entity the
    /// server told this client of (`ClientLevel.getEntity`, whose entities are
    /// the ones the server tracks to it). None for a stale handle, with no
    /// login, or for an entity the client doesn't have.
    pub(super) fn native_client_entity_target(&self, entity: EntityRef) -> Option<Target> {
        self.native_client_entity_body_target(entity).ok()
    }

    /// Resolve a projected body without mistaking packet-only tracking for a retired handle.
    fn native_client_entity_body_target(
        &self,
        entity: EntityRef,
    ) -> Result<Target, EntityReadError> {
        let (remote, id) = self.native_client_entity_context(entity)?;
        if id == remote.entity_id {
            return self
                .player
                .as_ref()
                .map(|_| Target::Player)
                .ok_or(EntityReadError::Unavailable);
        }
        // The server tracks the dragon alone; its parts are the client's own copies of it.
        if self.resident_dragon_part(entity.id()).is_some() {
            return Ok(Target::DragonPart(entity.id()));
        }
        let (kind, _) = remote.tracked_entity(id).ok_or(EntityReadError::Stale)?;
        if Shows::from_kind(kind).is_some() {
            // Displays have their own client bodies; never resolve one through another store.
            return Err(EntityReadError::Unavailable);
        }
        match self.level_entity(id) {
            Some(Target::Mob(mob)) if !self.mobs.iter().any(|m| m.id == mob && !m.removed) => {
                Err(EntityReadError::Unavailable)
            }
            Some(target) => Ok(target),
            None => Err(EntityReadError::Unavailable),
        }
    }
    /// What a handle names for an optional body read that works on both sides.
    pub(super) fn native_entity_read_target(&self, entity: EntityRef) -> Option<Target> {
        self.native_entity_visit_target(entity).ok()
    }
    /// Resolve the resident server body or the client projection, separating a stale
    /// handle from a body or world that the current view cannot read.
    pub(super) fn native_entity_visit_target(
        &self,
        entity: EntityRef,
    ) -> Result<Target, EntityReadError> {
        if self.is_logical_server() {
            self.native_server_entity_target(entity)
        } else {
            self.native_client_entity_body_target(entity)
        }
    }
    /// Whether a world position sits in an accessible entity section.
    pub(in crate::game) fn entity_position_accessible(&self, pos: [f64; 3]) -> bool {
        entity_section_accessible(&self.world.chunks, pos)
    }

    /// Whether `target` is discoverable through `LevelEntityGetter`
    /// (`PersistentEntitySectionManager.visibleEntityStorage` /
    /// `EntitySectionStorage.getEntities`): players are always ticking
    /// (`Player.isAlwaysTicking`), while other entities require an accessible
    /// section.
    pub(in crate::game) fn entity_target_accessible(&self, target: Target) -> bool {
        match target {
            Target::Player | Target::Guest(_) => true,
            Target::DragonPart(_) => self.dragon.as_ref().is_some_and(|d| {
                self.entity_position_accessible([d.position.x, d.position.y, d.position.z])
            }),
            _ => self
                .target_position(target)
                .is_some_and(|pos| self.entity_position_accessible(pos)),
        }
    }

    /// The entity `id` (the id events carry) as a handle for the `entity_*`
    /// methods: None with no world open, or for no such entity in the accessible
    /// sections of the level in place or among the server's players. On a client (API
    /// @API:client_entity_refs@), any entity the server shows this window,
    /// this window's own player included. API 373.
    pub(super) fn entity(&self, id: u32) -> Option<EntityRef> {
        let entity = EntityRef::new(self.native_lifecycle.epoch()?, id);
        let found = if self.is_logical_server() {
            self.native_entity_target(entity)
                .is_some_and(|target| self.entity_target_accessible(target))
        } else {
            self.native_client_entity_target(entity).is_some()
                || self.native_client_tracked(entity).is_some()
        };
        found.then_some(entity)
    }
    /// `ServerLevel.getEntity(UUID)` for [`LevelRef::entity_by_uuid`]: the level's UUID index,
    /// in place or parked, then the same resolution and visibility as any handle of that level.
    pub(super) fn level_entity_by_uuid(
        &self,
        level: LevelRef,
        uuid: mod_api::EntityUuid,
    ) -> Option<EntityRef> {
        self.native_level_world(level)?;
        let dimension = level.dimension();
        let uuid = sunrise_client::lan::playerdata::uuid_ints(uuid.0);
        let mut after = None;
        loop {
            let id = self
                .resident_entities
                .table
                .borrow_mut()
                .level_uuid_candidate(dimension, uuid, after)?;
            let entity = EntityRef::new(level.epoch(), id.get());
            // The level in place asks that the entity's section is accessible, as `entity` does;
            // a parked level's sections are not read here.
            if let Ok((target, _)) = self.resolve_server_entity_in(entity, dimension)
                && self.resident_source_target_visible(dimension, target)
                && (dimension != self.dim.key() || self.entity_target_accessible(target))
            {
                return Some(entity);
            }
            // A queued departure can overlap a replacement with the same UUID.
            after = Some(id);
        }
    }
    /// Whether the entity is made of parts that are entities of their own
    /// (`Entity.isMultipartEntity`): the dragon, not one of its parts.
    pub(super) fn entity_is_multipart(&self, entity: EntityRef) -> Option<bool> {
        Some(matches!(
            self.native_entity_read_target(entity)?,
            Target::Dragon(_)
        ))
    }
    /// `Entity.getPickResult` of the entity, on the side that holds it.
    pub(super) fn entity_pick_result(&self, entity: EntityRef) -> Option<ItemStack> {
        self.native_entity_read_target(entity)?;
        self.pick_result_of(i32::try_from(entity.id()).ok()?)
    }
    /// `IEntityExtension.getParts` in `EnderDragon.getSubEntities` order.
    pub(super) fn entity_visit_parts(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        let Target::Dragon(id) = self.native_entity_visit_target(entity)? else {
            return Ok(ControlFlow::Continue(()));
        };
        // Project the eight part handles together before calling the visitor on any of them.
        let mut parts = [entity; 8];
        for (offset, part) in (1..).zip(&mut parts) {
            *part = id
                .checked_add(offset)
                .and_then(|id| self.entity(id))
                .ok_or(EntityReadError::Unavailable)?;
        }
        Ok(parts.into_iter().try_for_each(visit))
    }
    /// The level `dimension` (`minecraft:the_nether`) as a handle: on a client,
    /// the level it shows, whatever the server calls it, or one the server opened
    /// for viewing (`client_level_key`). None with no world open, or for a
    /// dimension the server hasn't loaded. API 373.
    pub(in crate::game) fn level(&self, dimension: &str) -> Option<LevelRef> {
        if !self.is_logical_server() {
            return self.client_level_by_name(dimension);
        }
        let epoch = self.native_lifecycle.epoch()?;
        let dim = (!self.levels.active_closed)
            .then_some(self.dim)
            .into_iter()
            .chain(self.levels.parked_dims())
            .find(|d| d.id() == dimension)?;
        Some(self.level_ref(epoch, dim.key()))
    }
    /// The client level in place as a handle: the level this window shows and
    /// ticks. None before login, while reconfiguring, or with no world open.
    pub(in crate::game) fn native_client_level(&self) -> Option<LevelRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let remote = self.remote.as_ref().filter(|r| r.has_level())?;
        Some(LevelRef::new(epoch, remote.level_key))
    }
    /// The current world on its owning side, as a handle for a native query.
    pub(super) fn native_current_level(&self) -> Option<LevelRef> {
        if self.is_logical_server() {
            return Some(self.level_ref(self.native_lifecycle.epoch()?, self.dim.key()));
        }
        self.native_client_level()
    }
    /// The name of a dimension's key.
    pub(in crate::game) fn dimension_key_id(&self, key: DimensionKey) -> Option<&str> {
        mod_api::__private::dimension_id(key)
    }
    /// The key of the level in place: on a client the level it shows, which
    /// `Dim` may have no name for.
    pub(super) fn native_level_key(&self) -> DimensionKey {
        self.remote
            .as_ref()
            .map_or_else(|| self.dim.key(), |r| r.level_key)
    }
    /// Loaded server levels in `MinecraftServer.getAllLevels` dimension order.
    pub(super) fn native_visit_levels(
        &self,
        visit: &mut dyn FnMut(LevelRef) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let Some(epoch) = self
            .native_lifecycle
            .epoch()
            .filter(|_| self.is_logical_server())
        else {
            return ControlFlow::Continue(());
        };
        self.loaded_level_order()
            .into_iter()
            .map(|dim| self.level_ref(epoch, dim.key()))
            .try_for_each(visit)
    }
    fn level_entity_candidates(&self, level: LevelRef) -> impl Iterator<Item = EntityRef> + '_ {
        let mut after = None;
        std::iter::from_fn(move || {
            // Copy the key and release the residency borrow before resolution or a visitor.
            let key = self
                .resident_entities
                .table
                .borrow_mut()
                .level_entity_candidate_after(level.dimension(), after)?;
            after = Some(key);
            Some(key.1.get())
        })
        .filter(move |id| {
            !self
                .persist
                .removals
                .iter()
                .chain(self.persist.destroying_vehicles.iter())
                .any(|target| super::native_entities::target_id(*target) == Some(*id))
        })
        .map(move |id| EntityRef::new(level.epoch(), id))
    }

    /// Visit lookup members of the active server level, without spatial filtering.
    pub(super) fn level_visit_all_entities(
        &self,
        level: LevelRef,
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        if !self.is_logical_server() {
            return Err(LevelReadError::Unavailable);
        }
        if !self.level_handle_current(level) {
            return Err(LevelReadError::Stale);
        }
        if level.dimension() != self.dim.key() {
            return Err(LevelReadError::Unavailable);
        }

        for entity in self.level_entity_candidates(level) {
            if self
                .native_entity_target(entity)
                .is_some_and(|target| self.entity_target_accessible(target))
                && visit(entity).is_break()
            {
                return Ok(ControlFlow::Break(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    }
    /// Engine callers that mutate after querying need a snapshot of the matching handles.
    pub(super) fn level_entities_in(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Option<Vec<EntityRef>> {
        let mut entities = Vec::new();
        let _ = self
            .level_visit_entities_in(level, min, max, &mut |entity| {
                entities.push(entity);
                ControlFlow::Continue(())
            })
            .ok()?;
        Some(entities)
    }

    /// Visit intersecting, readable entities in `Level.getEntities` query order.
    pub(super) fn level_visit_entities_in(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        if !self.level_handle_current(level) {
            return Err(LevelReadError::Stale);
        }
        if !min.iter().chain(&max).all(|v| v.is_finite()) {
            return Err(LevelReadError::Unavailable);
        }
        let server = self.is_logical_server();
        if (server && level.dimension() != self.dim.key())
            || (!server && self.native_client_level() != Some(level))
        {
            return Err(LevelReadError::Unavailable);
        }
        let bounds = Aabb::new(
            Vec3::new(min[0], min[1], min[2]),
            Vec3::new(max[0], max[1], max[2]),
        );
        let search = SectionSearch::new(bounds);
        if server {
            self.file_server_section_members(level);
            let mut after = None;
            loop {
                let candidate = self
                    .resident_entities
                    .table
                    .borrow()
                    .level_section_candidate_after(level.dimension(), search, after);
                let Some(key) = candidate else { break };
                after = Some(key);
                let (_, (_, id)) = key;
                let entity = EntityRef::new(level.epoch(), id.get());
                if let Some((target, Some(at))) = self.native_entity_target_with_location(entity)
                    && self.entity_target_accessible(target)
                    && self
                        .query_body(target, at.join, true)
                        .is_some_and(|body| bounds.intersects(body))
                    && visit(entity).is_break()
                {
                    return Ok(ControlFlow::Break(()));
                }
            }
        } else {
            for member in self.client_section_candidates(search) {
                let entity = EntityRef::new(level.epoch(), member.id);
                if self
                    .client_query_body(entity, member)
                    .is_some_and(|body| bounds.intersects(body))
                    && visit(entity).is_break()
                {
                    return Ok(ControlFlow::Break(()));
                }
            }
        }
        Ok(self.visit_dragon_query_parts(bounds, &mut |id| {
            let entity = EntityRef::new(level.epoch(), id);
            // Resolution ends before mod code runs: the visitor can make its own handle reads.
            let accessible = if self.is_logical_server() {
                self.native_entity_target(entity)
                    .is_some_and(|target| self.entity_target_accessible(target))
            } else {
                self.native_client_entity_target(entity).is_some()
            };
            if accessible {
                visit(entity)
            } else {
                ControlFlow::Continue(())
            }
        }))
    }

    /// File the rows admitted since the last query. One walk over the families files every
    /// member they hold at its position; only the rows no family walk reaches resolve one by one.
    fn file_server_section_members(&self, level: LevelRef) {
        let table = &self.resident_entities.table;
        if !table
            .borrow_mut()
            .has_unfiled_section_members(level.dimension())
        {
            return;
        }
        {
            let mut table = table.borrow_mut();
            crate::game::pick_entity::pick_sections::visit_section_members(
                self,
                &mut |_, _, id, position| {
                    if let Some(id) = std::num::NonZeroU32::new(id) {
                        table.file_section_member(id, position);
                    }
                },
            );
        }
        let mut after = None;
        loop {
            let next = table
                .borrow_mut()
                .unfiled_section_candidate_after(level.dimension(), after);
            let Some(id) = next else { break };
            after = Some(id);
            let entity = EntityRef::new(level.epoch(), id.get());
            if let Some(target) = self.native_entity_target(entity)
                && let Some(position) = self.target_position(target)
            {
                table.borrow_mut().file_section_member(id, position);
            }
        }
    }

    fn client_query_body(&self, entity: EntityRef, member: PickMember) -> Option<Aabb> {
        match member.kind {
            PickKind::RemoteDisplay => {
                let p = self.native_client_display(entity)?.position();
                let p = Vec3::new(p.x, p.y, p.z);
                Some(Aabb::new(p, p))
            }
            // These are indexed only for picking; they were not readable box-query bodies.
            PickKind::Cushion
            | PickKind::Interaction
            | PickKind::RemoteGeneral
            | PickKind::RemoteKnot
            | PickKind::LocalKnot
            | PickKind::Crystal => None,
            PickKind::LocalPlayer if self.mode_id() != 3 => {
                self.player.as_ref().map(|p| p.bounds())
            }
            PickKind::LocalPlayer => None,
            _ => {
                let remote = self.remote.as_ref()?;
                remote.tracked(member.id as i32)?;
                let EntityLookup::Found { location, value } =
                    self.server_resident_entity(member.id)
                else {
                    return None;
                };
                if location.owner.dimension != self.dim.key() {
                    return None;
                }
                self.query_body(value, location.join, false)
            }
        }
    }

    fn query_body(&self, target: Target, join: JoinSeq, server: bool) -> Option<Aabb> {
        match target {
            Target::Player if self.mode_id() != 3 => {
                let guest = self.native_target_guest(target)?;
                if !guest.participates_in_player_simulation() || !guest.placed {
                    return None;
                }
                self.player.as_ref().map(|p| p.bounds())
            }
            Target::Guest(conn) => self
                .command_guest(conn)
                .filter(|guest| {
                    guest.participates_in_player_simulation()
                        && guest.placed
                        && guest.game_mode != 3
                })
                .map(|guest| guest.bounds()),
            Target::Other(_) if server => self
                .others
                .get(self.others.locate(join)?)?
                .display_marker_or_interaction_bounds(),
            Target::Hook(id) if server => self
                .fishing_hooks_here()
                .find_map(|(hook, _)| (hook.id() == id).then(|| hook.bounds())),
            Target::Dragon(_) => self.dragon.as_ref().map(|d| d.body_bounds()),
            Target::Frame(_) => self
                .item_frames
                .get(self.item_frames.locate(join)?)
                .map(|f| aabb_extents(f.bounds())),
            Target::Painting(_) => self
                .paintings
                .get(self.paintings.locate(join)?)
                .map(|p| aabb_extents(p.bounds())),
            Target::Mob(_) => list_query_body(&self.mobs, join),
            Target::Item(_) => list_query_body(&self.dropped, join),
            Target::Orb(_) => list_query_body(&self.orbs, join),
            Target::Boat(_) => list_query_body(&self.boats, join),
            Target::Cart(_) => list_query_body(&self.carts, join),
            Target::Stand(_) => list_query_body(&self.armor_stands, join),
            Target::Tnt(_) => list_query_body(&self.tnts, join),
            Target::Falling(_) => list_query_body(&self.falling, join),
            Target::Projectile(_) => list_query_body(&self.creatures.projectiles, join),
            Target::Rocket(_) => list_query_body(&self.rockets, join),
            Target::Trident(_) => list_query_body(&self.tridents, join),
            Target::General(_) => list_query_body(&self.resident_entities.general.rows, join),
            _ => None,
        }
    }
    /// Every geometric hit, stably sorted nearest first. None for a refused walk.
    pub(super) fn level_ray_trace_entities(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        margin: f64,
        exclude: Option<EntityRef>,
    ) -> Option<Vec<EntityRayHit>> {
        let mut hits = Vec::new();
        self.visit_entity_ray_hits(
            level,
            from,
            to,
            margin,
            exclude,
            |_| true,
            |hit, _| hits.push(hit),
        )
        .ok()?;
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        Some(hits)
    }

    pub(super) fn level_ray_trace_nearest_entity(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        margin: f64,
        exclude: Option<EntityRef>,
        accept: &mut dyn FnMut(EntityRef) -> bool,
    ) -> Result<Option<EntityRayHit>, LevelReadError> {
        let mut nearest = None;
        self.visit_entity_ray_hits(
            level,
            from,
            to,
            margin,
            exclude,
            accept,
            |hit, distance_squared| {
                if nearest.is_none_or(|(_, best)| distance_squared < best) {
                    nearest = Some((hit, distance_squared));
                }
            },
        )?;
        Ok(nearest.map(|(hit, _)| hit))
    }

    fn visit_entity_ray_hits(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        margin: f64,
        exclude: Option<EntityRef>,
        mut accept: impl FnMut(EntityRef) -> bool,
        mut visit: impl FnMut(EntityRayHit, f64),
    ) -> Result<(), LevelReadError> {
        if !(margin.is_finite() && from.iter().chain(&to).all(|v| v.is_finite())) {
            return Err(LevelReadError::Unavailable);
        }
        // Every box the inflated or deflated test can hit meets the segment's own box
        // padded by `margin.abs()` (including over-deflated boxes swapped by `Aabb::new`).
        let pad = margin.abs();
        let min = std::array::from_fn(|i| from[i].min(to[i]) - pad);
        let max = std::array::from_fn(|i| from[i].max(to[i]) + pad);
        let start = Vec3::new(from[0], from[1], from[2]);
        let end = Vec3::new(to[0], to[1], to[2]);
        let padding = Vec3::new(margin, margin, margin);
        let _ = self.level_visit_entities_in(level, min, max, &mut |entity| {
            // Match `ProjectileUtil.getEntityHitResult`: filter candidates before reading and
            // clipping their bounding boxes. The walk releases any residency borrow before
            // calling mod code.
            if Some(entity) != exclude
                && accept(entity)
                && let Some(bounds) = self.entity_bounding_box(entity)
                && let Some((hit, distance_squared)) =
                    clip_entity_ray(start, end, padding, entity, bounds)
            {
                visit(hit, distance_squared);
            }
            ControlFlow::Continue(())
        })?;
        Ok(())
    }
    /// What the block at `pos` would drop (`Block.getDrops`) with `tool` and `entity` as the
    /// breaker, without breaking it: the level's `block_drops_of`, which draws from the random
    /// as vanilla's roll does. None for a refused write, a stale or parked handle, a client, an
    /// unloaded or out-of-height position, and an `entity` that names nothing.
    pub(super) fn level_block_drops(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
        entity: Option<EntityRef>,
    ) -> Option<Vec<ItemStack>> {
        if self.entity_ids_exhausted() {
            return None;
        }
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return None;
        }
        if let Some(entity) = entity {
            self.native_entity_target(entity)?;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        Some(self.with_level(|_, level, _| level.block_drops_of(at, state, tool, entity)))
    }
    /// One roll of the world's loot table `table` (`LootTable.getRandomItems`) with `origin` as
    /// ORIGIN, `tool` as TOOL and `entity` as THIS_ENTITY when given: the context
    /// `/loot … fish <table> <pos> <tool>` gives. It draws from the table's random sequence,
    /// else the level random, and the stacks come split to their stack sizes. None for a
    /// missing table, a stale handle or one of a parked level, a client, and an `entity` that
    /// names nothing.
    pub(super) fn level_roll_loot(
        &mut self,
        level: LevelRef,
        table: &str,
        origin: [f64; 3],
        tool: Option<&ItemStack>,
        entity: Option<EntityRef>,
    ) -> Option<Vec<ItemStack>> {
        if !self.is_logical_server()
            || !self.level_handle_current(level)
            || self.dim.key() != level.dimension()
            || !world_items!(self).has_loot_table(table)
        {
            return None;
        }
        let this = match entity {
            Some(entity) => {
                let target = self.native_entity_target(entity)?;
                Some(self.target_loot_entity(target))
            }
            None => None,
        };
        self.roll_table_at(table, Some(origin), tool, this.as_ref())
    }
    /// Set a block through Level.setBlock. Typed errors distinguish stale/unavailable
    /// levels; Ok(false) preserves vanilla's position/no-change refusal.
    pub(super) fn level_set_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        flags: BlockUpdateFlags,
    ) -> Result<bool, mod_api::LevelWriteError> {
        if !self.native_level_write_position(level, pos)? {
            return Ok(false);
        }
        if let Some(mut host) = super::journal_level_write::JournalLevelWrite::new(self, level) {
            return Ok(host.write_block(pos, state, flags.bits()));
        }
        Ok(self.native_set_block(level.dimension_id(), pos, state, flags.bits()))
    }
    /// Set or clear the mod-published connection group at `pos`.
    /// Returns `Ok(true)` if changed, `Ok(false)` if unchanged, unloaded, out of height, or
    /// fenced, and `Err(LevelWriteError::Stale | Unavailable)` for stale or non-writable levels.
    pub(super) fn level_set_connection_group(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        group: Option<u64>,
    ) -> Result<bool, mod_api::LevelWriteError> {
        if !self.native_level_write_position(level, pos)? {
            return Ok(false);
        }
        Ok(self.journal_set_connection_group(pos, group))
    }

    pub(super) fn level_set_block_property(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: BlockUpdateFlags,
    ) -> Result<bool, LevelWriteError> {
        self.with_server_level_mut(
            level,
            LevelWriteError::Unavailable,
            LevelWriteError::Stale,
            Some(LevelWriteError::InTransaction),
            |game| {
                if !game.native_level_write_position(level, pos)? {
                    return Ok(false);
                }
                let [x, y, z] = pos;
                let state = game.world.block(x, y, z);
                let Some(id) = super::native_hooks::property_change(
                    game.pack.assets.registry,
                    state,
                    key,
                    value,
                ) else {
                    return Ok(false);
                };
                game.level_set_block(level, pos, id, flags)
            },
        )
    }

    /// `Block.updateFromNeighbourShapes` for `state` at `pos`, nothing written: the state
    /// fitted to the loaded blocks around it. A position that is not loaded or outside the
    /// world gives `state` back.
    pub(super) fn level_update_from_neighbour_shapes(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Result<BlockStateId, mod_api::LevelWriteError> {
        if !self.native_level_write_position(level, pos)? {
            return Ok(state);
        }
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let fitted = self
            .with_level(|b, level, ticks| b.update_from_neighbour_shapes(level, ticks, at, state));
        let _ = self.process_level_events();
        Ok(fitted)
    }
    /// Remove a block in place (`Level.removeBlock`), preserving its fluid.
    pub(super) fn level_remove_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        moved: bool,
    ) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        if let Some(mut host) = super::journal_level_write::JournalLevelWrite::new(self, level) {
            return mod_api::LevelWriteHost::remove_block(&mut host, pos, moved);
        }
        let (state, flags) = self.level_removal_write(pos, moved);
        let changed = self.set_block_live(pos, state, flags, false);
        let _ = self.process_level_events();
        changed
    }
    /// Tell the neighbours of `pos` that a block changed there, as vanilla's own writes do:
    /// `Neighbours` (`Level.updateNeighborsAt`: a lever's or a button's own block, and the one
    /// it is attached to, when the signal it gives changes), `NeighboursExcept(direction)`
    /// (`updateNeighborsAtExceptFromFacing`: a diode's front) or `OutputSignal`
    /// (`updateNeighbourForOutputSignal`: the comparators beside it, when its comparator output
    /// changes without its state changing). `source` is the state whose block vanilla passes as
    /// the one that changed. False, with nothing done, off the level in place, for a position
    /// that isn't loaded, or for a `source` that is no block state.
    pub(super) fn level_update_neighbors(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: BlockStateId,
        update: NeighbourUpdate,
    ) -> Result<bool, mod_api::LevelWriteError> {
        if !self.native_level_write_position(level, pos)? {
            return Ok(false);
        }
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| {
            b.neighbour_update(level, ticks, at, b.block(source), update)
        });
        let _ = self.process_level_events();
        Ok(true)
    }
    /// Tell the block at `pos` that `source`'s block beside it changed (`Level.neighborChanged`),
    /// as a diode tells the block in front of it before updating that block's neighbours. False,
    /// with nothing done, as for `update_neighbors`: off the level in place, for a position that
    /// isn't loaded, or for a `source` that is no block state.
    pub(super) fn level_neighbor_changed(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: BlockStateId,
    ) -> Result<bool, mod_api::LevelWriteError> {
        if !self.native_level_write_position(level, pos)? {
            return Ok(false);
        }
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.neighbor_changed_at(level, ticks, at, b.block(source)));
        let _ = self.process_level_events();
        Ok(true)
    }
    /// Destroy a block in the level in place (`Level.destroyBlock(pos, drop, breaker, maxUpdates)`),
    /// optionally dropping resources with `tool` (None is an empty hand) and `breaker` as the
    /// loot's `this` entity and the `minecraft:block_destroy` game event's source. With `collect`,
    /// return the dropped stacks without spawning their item entities; otherwise return an empty
    /// list after spawning them. The one level drop path runs loot, block-drop hooks and
    /// experience once, in vanilla's random order. `drop` false suppresses resources entirely.
    /// None means nothing broke: air, a refused write, a stale or foreign `breaker`, a stale
    /// handle, a client, an unloaded/out-of-height position, or a parked level.
    pub(super) fn level_destroy_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        drop: bool,
        tool: Option<&ItemStack>,
        breaker: Option<EntityRef>,
        collect: bool,
    ) -> Option<Vec<ItemStack>> {
        if !self.native_level_write_position(level, pos).ok()? {
            return None;
        }
        if let Some(entity) = breaker
            && (self.native_entity_target(entity).is_none() || !self.native_entity_here(entity))
        {
            return None;
        }
        let [x, y, z] = pos;
        let at = sunrise_core::position::BlockPos::new(x, y, z);
        if let Some(mut write) = super::journal_level_write::JournalLevelWrite::new(self, level) {
            return write.destroy_block(pos, drop, tool, breaker, collect);
        }
        let (changed, drops) = self.with_level(|b, level, ticks| {
            level.drop_tool = Some(((pos[0], pos[1], pos[2]), tool.cloned()));
            level.drop_entity = breaker;
            level.drop_breaker_at = breaker.map(|_| at);
            level.drop_into = collect.then(Vec::new);
            let changed = b.destroy_block(level, ticks, at, drop);
            (changed, level.drop_into.take().unwrap_or_default())
        });
        let destroyed_state = if changed
            && let Some(&sunrise_world::level::LevelEvent::GameEvent {
                pos: event_pos,
                event: "minecraft:block_destroy",
                state: Some(state),
            }) = self.level_events.last()
            && event_pos == at
        {
            self.level_events.pop();
            Some(state)
        } else {
            None
        };
        let _ = self.process_level_events();
        if let Some(state) = destroyed_state {
            self.post_destroy_block_game_event(pos, state, breaker);
        }
        changed.then_some(drops)
    }

    /// `Block.dropResources` of the block at `pos`, the block left in place: the loot with
    /// `tool` and `breaker`, the block entity's contribution, the `block_drops` hook and the
    /// experience, under `doTileDrops`, as `level_destroy_block` drops them, without the break
    /// particles, the removal or the `block_destroy` game event. With `collect`, the stacks
    /// come back instead of spawning. None for air and where `level_destroy_block` refuses.
    pub(super) fn level_drop_resources(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
        breaker: Option<EntityRef>,
        collect: bool,
    ) -> Option<Vec<ItemStack>> {
        if !self.native_level_write_position(level, pos).ok()? {
            return None;
        }
        if let Some(entity) = breaker
            && (self.native_entity_target(entity).is_none() || !self.native_entity_here(entity))
        {
            return None;
        }
        let [x, y, z] = pos;
        let state = self.world.block(x, y, z);
        if self.res.behaviours().is_air(state) {
            return None;
        }
        if let Some(mut write) = super::journal_level_write::JournalLevelWrite::new(self, level) {
            return Some(write.drop_resources(pos, state, tool, breaker, collect));
        }
        let at = sunrise_core::position::BlockPos::new(x, y, z);
        let drops = self.with_level(|b, level, ticks| {
            level.drop_tool = Some(((x, y, z), tool.cloned()));
            level.drop_entity = breaker;
            level.drop_breaker_at = breaker.map(|_| at);
            level.drop_into = collect.then(Vec::new);
            b.drop_resources(level, ticks, at, state);
            level.drop_into.take().unwrap_or_default()
        });
        let _ = self.process_level_events();
        Some(drops)
    }

    pub(in crate::game) fn post_destroy_block_game_event(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        breaker: Option<EntityRef>,
    ) {
        use crate::game::api_values::TryIntoApi as _;
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        if let Ok(api_state) = state.try_into_api() {
            let context = mod_api::GameEventContext {
                source_entity: breaker,
                affected_state: Some(api_state),
            };
            if self.post_contextual_game_event("minecraft:block_destroy", at, context) {
                return;
            }
        }
        // If a deferred transaction's breaker left before commit, still emit block_destroy without a source.
        self.post_block_game_event("minecraft:block_destroy", pos, None, Some(state));
    }

    /// Send `ServerLevel.destroyBlockProgress` in this level: stages 0..9 show a crack,
    /// any other stage removes it. A machine uses a stable negative breaker id.
    pub(super) fn level_set_break_progress(
        &mut self,
        level: LevelRef,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        self.after_commit(After::BreakProgress(breaker, pos, progress));
        true
    }
    /// `Level.sendBlockUpdated`: queue the block at `pos` to be sent to the clients again.
    pub(super) fn level_send_block_updated(&mut self, level: LevelRef, pos: [i32; 3]) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        self.notify_block_clients((pos[0], pos[1], pos[2]));
        true
    }
    /// `LevelRenderer.destroyBlockProgress` for this client's own view only.
    pub(super) fn level_set_client_break_progress(
        &mut self,
        level: LevelRef,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> bool {
        if self.is_logical_server() || self.native_level_read_world(level).is_none() {
            return false;
        }
        self.destroy_block_progress(breaker, pos, progress);
        true
    }
    /// `ItemStack.getDestroySpeed` of `tool` on the block at `pos`.
    pub(super) fn level_block_break_speed(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
    ) -> Option<f32> {
        let world = self.native_level_read_world(level)?;
        if !world.loaded(pos[0], pos[2]) || !world.shape.contains_y(pos[1]) {
            return None;
        }
        let state = world.block(pos[0], pos[1], pos[2]);
        let name = &self.res.facts().get(state).name;
        Some(self.items().mining_speed(tool, name))
    }
    /// Apply bone meal to the block at `pos` as a dispenser does (`BoneMealItem.growCrop`, then
    /// `growWaterPlant`): the mods' `bonemeal` hook first, level event 1505 when something grew.
    /// False for a refused write, a stale or parked handle, a client, and a block that did not
    /// grow.
    pub(super) fn level_bone_meal(&mut self, level: LevelRef, pos: [i32; 3]) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let mut stack = sunrise_client::items::ItemStack::new("minecraft:bone_meal", 1);
        let ask = self.bonemeal(at, false, &mut stack);
        let grown = self.with_level(|b, level, ticks| {
            ask.crop
                .unwrap_or_else(|| b.grow_crop(level, ticks, at, ask.check))
                || b.grow_water_plant(level, ticks, at, None)
        });
        if grown {
            self.level_events
                .push(sunrise_world::level::LevelEvent::LevelEvent {
                    id: 1505,
                    pos: at,
                    data: 15,
                    source: None,
                });
        }
        let _ = self.process_level_events();
        grown
    }
    /// Set the biome of the cell holding `pos` to `biome` (`/fillbiome` for one cell). False for a
    /// refused write, a stale or parked handle, a client, an unknown biome and an unchanged cell.
    pub(super) fn level_set_biome(&mut self, level: LevelRef, pos: [i32; 3], biome: &str) -> bool {
        if !self.native_level_write_allowed(level, pos) || self.dim.key() != level.dimension() {
            return false;
        }
        self.set_biome_at(pos, biome)
    }
    /// One residency boundary for state-bearing block writes; position refusal is vanilla.
    pub(super) fn native_level_write_position(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Result<bool, mod_api::LevelWriteError> {
        let world = self.native_level_world_result(level)?;
        if self.dim.key() != level.dimension() {
            return Err(mod_api::LevelWriteError::Unavailable);
        }
        Ok(!self.world.carrier_fences.locks_cell(&pos)
            && world.loaded(pos[0], pos[2])
            && world.shape.contains_y(pos[1]))
    }
    pub(crate) fn native_level_write_allowed(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        // A carrier transfer fence holds this cell: every mod write
        // through the level gate refuses while the operation runs.
        !self.world.carrier_fences.locks_cell(&pos)
            && self.is_logical_server()
            && self.level_handle_current(level)
            && self.world.loaded(pos[0], pos[2])
            && self.world.shape.contains_y(pos[1])
    }
    /// The supplied state's block-local outline, collision or visual boxes at a level
    /// position with an empty context. The world is never modified. Returns None
    /// for a stale or unavailable level. Unloaded neighbors
    /// read as air; collision errors return no boxes, like `visit_block_collision`.
    pub(super) fn level_visit_block_shape(
        &self,
        level: LevelRef,
        state: BlockStateId,
        pos: [i32; 3],
        kind: BlockShapeKind,
        visit: &mut dyn FnMut([f64; 6]) -> std::ops::ControlFlow<()>,
    ) -> Option<std::ops::ControlFlow<()>> {
        if !self.is_logical_server() {
            return (self.native_client_level() == Some(level)).then(|| {
                self.visit_state_shape(&self.world, &self.be_anims, state, pos, kind, visit)
            });
        }
        if !self.level_handle_current(level) {
            return None;
        }
        let (world, anims) = if self.dim.key() == level.dimension() {
            (&self.world, &self.be_anims)
        } else {
            let dim = self
                .levels
                .parked_dims()
                .into_iter()
                .find(|dim| dim.key() == level.dimension())?;
            (
                self.levels.parked_world(dim)?,
                self.levels.parked_anims(dim)?,
            )
        };
        Some(self.visit_state_shape(world, anims, state, pos, kind, visit))
    }

    #[cfg(test)]
    pub(super) fn collect_level_block_shape(
        &self,
        level: LevelRef,
        state: BlockStateId,
        pos: [i32; 3],
        kind: BlockShapeKind,
    ) -> Option<Vec<[f64; 6]>> {
        let mut boxes = Vec::new();
        let _ = self.level_visit_block_shape(level, state, pos, kind, &mut |bounds| {
            boxes.push(bounds);
            std::ops::ControlFlow::Continue(())
        })?;
        Some(boxes)
    }

    /// The local face support in an open server level or the current client level,
    /// including shapes published by local block entities. None for unavailable
    /// levels, unloaded or out-of-height blocks, or a shape query error.
    pub(super) fn level_block_face_sturdy(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        face: BlockFace,
        support: mod_api::SupportType,
    ) -> Option<bool> {
        super::native_world_read::block_face_sturdy(
            self.native_level_read_world(level)?,
            self.res.behaviours(),
            self.res.collision(),
            pos,
            face,
            support,
        )
    }

    /// `LevelRef::clip`: a ray against collider, outline or visual shapes and fluids
    /// (`BlockGetter.clip` / `BlockGetter.clipWithInteractionOverride`) in the resident world
    /// `native_level_view_world` reads. See [`Self::level_clip_ray`].
    pub(super) fn level_clip(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        blocks: ClipBlock,
        fluids: ClipFluid,
        context: CollisionContext,
    ) -> Result<ClipOutcome, ClipError> {
        Ok(
            match self.level_clip_ray(level, from, to, blocks, fluids, context)? {
                ClipRay::Reached(hit) if hit.miss => ClipOutcome::Miss,
                ClipRay::Reached(hit) => ClipOutcome::Hit(hit),
                ClipRay::Unloaded(at) => ClipOutcome::Unloaded { at },
            },
        )
    }

    /// Run a ray with no traversal cell cap or length limit. Cells above or below build height
    /// read as air and nothing is loaded; the ray stops at the first cell inside a column that
    /// is not loaded. Shapes answer for `context`: the empty context, or the entity's own
    /// (`CollisionContext.of(entity)`, with its sneaking, held item and powder-snow facts), so
    /// scaffolding, powder snow and a mod `BlockPart::collision_shape` see that entity.
    /// Collider mode uses `SceneCollisions`'s full registry/context shapes, dynamic ones
    /// included; outline mode errors where dynamic outline geometry has no implementation;
    /// visual mode reads `StateFacts::visual_shape`. Fluid modes need the tag tables mods read
    /// (`native_tag_items`).
    ///
    /// `StaleLevel` for an unavailable level or entity handle, for a shape that cannot be
    /// answered and for fluid modes before the tag tables exist; `NonFiniteRay` for a NaN or
    /// infinite endpoint.
    pub(super) fn level_clip_ray(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        blocks: ClipBlock,
        fluids: ClipFluid,
        context: CollisionContext,
    ) -> Result<ClipRay, ClipError> {
        use sunrise_world::collision_context::CollisionContext as ShapeContext;
        if !from.iter().chain(&to).all(|v| v.is_finite()) {
            return Err(ClipError::NonFiniteRay);
        }
        let world = self
            .native_level_view_world(level)
            .ok_or(ClipError::StaleLevel)?;
        match context {
            CollisionContext::Empty => self.clip_in_world(
                world,
                (from, to),
                (blocks, fluids),
                ShapeContext::empty(false),
                None,
            ),
            CollisionContext::Entity(entity) => self
                .with_clip_entity(entity, |shape_context, subject| {
                    self.clip_in_world(world, (from, to), (blocks, fluids), shape_context, subject)
                })
                .ok_or(ClipError::StaleLevel)?,
            _ => Err(ClipError::StaleLevel),
        }
    }

    /// Hand `f` the collision context and powder-snow facts of the entity a handle names, as its
    /// own movement asks for them: the player's with its held item, a simulated entity's
    /// `support_collision`, and for an entity that is only a client's tracked copy a plain
    /// body context at its position. None for a stale handle.
    fn with_clip_entity<R>(
        &self,
        entity: EntityRef,
        f: impl FnOnce(
            sunrise_world::collision_context::CollisionContext<'_>,
            Option<sunrise_client::collisions::PowderSnowSubject<'_>>,
        ) -> R,
    ) -> Option<R> {
        use super::entity::with_entity;
        let target = self.native_entity_read_target(entity)?;
        if let Some(body) = self.native_player_body(target) {
            let context = sunrise_client::motion::player_context(
                entity.id(),
                body.position.y,
                body.shift_key_down,
                body.main_hand.as_deref(),
            );
            return Some(f(context, Some(body.powder_snow_subject())));
        }
        if let Some(handle) = self.native_walk_entity(entity.id()) {
            let tags = &self.items().entity_type_tags;
            return Some(with_entity!(self, handle, e => {
                let (context, subject) =
                    super::entity::Entity::support_collision(e, tags);
                f(context, subject)
            }));
        }
        let [x, y, z] = self.entity_position(entity)?;
        Some(f(
            sunrise_client::motion::BodyMotion::entity_context(entity.id(), Vec3::new(x, y, z)),
            None,
        ))
    }

    /// The ray itself, over `world` with `context` and the colliding entity's `subject`.
    fn clip_in_world(
        &self,
        world: &sunrise_client::world::World,
        (from, to): ([f64; 3], [f64; 3]),
        (blocks, fluids): (ClipBlock, ClipFluid),
        context: sunrise_world::collision_context::CollisionContext<'_>,
        subject: Option<sunrise_client::collisions::PowderSnowSubject<'_>>,
    ) -> Result<ClipRay, ClipError> {
        use sunrise_client::{collisions::SceneCollisions, motion::clip_world_with_fluids};
        use sunrise_world::world_ray::{RayBlockMode, RayError, RayFluidMode};
        let rays = super::explosion::ColliderRays {
            scene: SceneCollisions {
                world,
                binding: self.collision,
                subject,
            },
            context,
            clip_data: Some(super::explosion::ClipData {
                facts: self.res.facts(),
                behaviours: self.res.behaviours(),
                tables: &self.pack.context.tables,
                registry: self.pack.assets.registry,
            }),
            report_unloaded: true,
        };
        let mode = match blocks {
            ClipBlock::Collider => RayBlockMode::Collider,
            ClipBlock::Outline => RayBlockMode::Outline,
            ClipBlock::Visual => RayBlockMode::Visual,
            _ => unreachable!("ClipBlock: unhandled variant"),
        };
        let fluid = match fluids {
            ClipFluid::None => RayFluidMode::None,
            ClipFluid::SourceOnly => RayFluidMode::SourceOnly,
            ClipFluid::Any => RayFluidMode::Any,
            ClipFluid::Water => RayFluidMode::Water,
            _ => unreachable!("ClipFluid: unhandled variant"),
        };
        let tags = if fluid == RayFluidMode::None {
            None
        } else {
            Some(self.native_tag_items().ok_or(ClipError::StaleLevel)?)
        };
        let result = clip_world_with_fluids(
            rays,
            world,
            Vec3::new(from[0], from[1], from[2]),
            Vec3::new(to[0], to[1], to[2]),
            mode,
            fluid,
            usize::MAX,
            || {
                let fluid_resources = &self.res.motion().fluids;
                sunrise_client::motion::FluidClip {
                    states: fluid_resources.states(),
                    items: tags.expect("fluid clip closure only runs for non-None fluid mode"),
                    shapes: &fluid_resources.shapes,
                }
            },
        );
        let result = match result {
            Ok(result) => result,
            Err(RayError::World(error)) => {
                return match error
                    .get_ref()
                    .and_then(|source| source.downcast_ref::<super::explosion::UnloadedColumn>())
                {
                    Some(super::explosion::UnloadedColumn(at)) => {
                        Ok(ClipRay::Unloaded([at.x, at.y, at.z]))
                    }
                    None => Err(ClipError::StaleLevel),
                };
            }
            Err(RayError::Traversal(_)) => return Err(ClipError::NonFiniteRay),
        };
        if result.miss {
            return Ok(ClipRay::Reached(miss_hit(from, to)));
        }
        let h = result.hit;
        Ok(ClipRay::Reached(ClipHit {
            miss: false,
            block: [h.block.x, h.block.y, h.block.z],
            face: api_face(h.direction),
            location: [h.location.x, h.location.y, h.location.z],
            inside: h.inside,
            world_border: false,
        }))
    }

    /// The entity's position (`Entity.position`): its feet, the middle of
    /// its box for a painting or an item frame. Client displays use their committed tick pose.
    pub(super) fn entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        if self.native_lifecycle.epoch() == Some(entity.epoch())
            && let Some((id, position)) = self.native_lifecycle.entity_sections.write_position
            && id == entity.id()
        {
            return Some(position);
        }
        match self.native_entity_read_target(entity) {
            Some(target) => self.target_position(target),
            None if self.is_logical_server() => self
                .native_orb_ref(entity)
                .ok()
                .map(|orb| orb.position.components()),
            None => self
                .native_client_tracked(entity)
                .map(|(_, position)| position),
        }
    }
    /// The block the entity's position is in (`Entity.blockPosition`). API
    /// 373.
    pub(super) fn entity_block_position(&self, entity: EntityRef) -> Option<[i32; 3]> {
        self.entity_position(entity)
            .map(|p| p.map(|v| v.floor() as i32))
    }
    /// The entity's yaw and pitch in degrees (`Entity.getYRot`, `getXRot`); 0
    /// for the kinds that don't turn. API 373.
    pub(super) fn entity_rotation(&self, entity: EntityRef) -> Option<[f32; 2]> {
        Some(self.target_rotation(self.native_entity_read_target(entity)?))
    }

    pub(super) fn native_client_general(
        &self,
        entity: EntityRef,
    ) -> Option<&super::remote::RemoteGeneral> {
        let (remote, id) = self.native_client_entity_context(entity).ok()?;
        remote.general.rows.get(&id)
    }
    /// The entity's type (`Entity.getType`): `minecraft:zombie`. API
    /// 373.
    pub(super) fn entity_kind(&self, entity: EntityRef) -> Option<mod_api::EntityTypeKey> {
        match self.native_entity_read_target(entity) {
            Some(target) => self.native_entity_kind_key(target),
            None => {
                if self.native_lifecycle.epoch() == Some(entity.epoch())
                    && let Some((_, mob)) = self.mob_in_other_level(entity.id())
                {
                    return mob.kind.type_key;
                }
                mod_api::__private::resolved_entity_type_key(self.native_client_tracked(entity)?.0)
            }
        }
    }
    /// A mob of this game that is resident in a loaded level other than the
    /// one in place, as a kill credit can name one (`getEntityInAnyDimension`):
    /// its identity reads (`entity_kind`, `entity_uuid`, `entity_level`)
    /// answer from there. None on a client or for anything else.
    pub(super) fn mob_in_other_level(&self, id: u32) -> Option<(super::Dim, &super::Mob)> {
        use sunrise_client::join_seq::residency::table::EntityLookup;
        if !self.is_logical_server() {
            return None;
        }
        let EntityLookup::Found {
            location,
            value: Target::Mob(_),
        } = self.server_resident_entity(id)
        else {
            return None;
        };
        let (dim, parked) = self
            .levels
            .parked
            .iter()
            .find(|(dim, _)| dim.key() == location.owner.dimension)?;
        let mob = parked.mobs.get(parked.mobs.locate(location.join)?)?;
        Some((*dim, mob))
    }
    /// The entity's UUID (`Entity.getUUID`), the one it is saved with; an
    /// entity that has none yet gets it now, as a new vanilla entity does.
    /// On a client, the one the server added it with (a player's is its
    /// profile's). API 373.
    pub(super) fn entity_uuid(&mut self, entity: EntityRef) -> Option<u128> {
        if let Some(r) = &self.remote {
            let id = i32::try_from(entity.id()).ok()?;
            if self.native_entity_read_target(entity).is_none()
                && self.native_client_tracked(entity).is_none()
            {
                return None;
            }
            if let Some(words) =
                self.known_resident_entity_uuid_at(self.native_level_key(), entity.id())
            {
                return Some(uuid_from_words(words));
            }
            if let Some((uuid, _)) = r.level_player(id) {
                return Some(uuid);
            }
            return self
                .persist
                .uuids
                .get(&entity.id())
                .copied()
                .map(uuid_from_words);
        }
        if self.native_lifecycle.epoch() == Some(entity.epoch())
            && self.mob_in_other_level(entity.id()).is_some()
        {
            return self
                .resident_entities
                .table
                .borrow_mut()
                .resident_uuid(entity.id())
                .map(uuid_from_words);
        }
        let words = match self.native_entity_target(entity)? {
            Target::Player => {
                let dimension = self.current_player_resident_location()?.owner.dimension;
                return self
                    .known_resident_entity_uuid_at(dimension, entity.id())
                    .map(uuid_from_words);
            }
            Target::Guest(conn) => return self.command_guest(conn).map(|g| g.uuid),
            _ => self.uuid_for(entity.id()),
        };
        Some(uuid_from_words(words))
    }
    /// The level the entity is in (`Entity.level`): on a client, the
    /// client's level. API 373.
    pub(super) fn entity_level(&self, entity: EntityRef) -> Option<LevelRef> {
        let key = match self.native_entity_read_target(entity) {
            Some(t) if self.is_logical_server() => self.target_level(t).key(),
            Some(_) => self.native_level_key(),
            None if self.native_lifecycle.epoch() == Some(entity.epoch()) => {
                match self.mob_in_other_level(entity.id()) {
                    Some((dim, _)) => dim.key(),
                    None => self
                        .native_client_tracked(entity)
                        .map(|_| self.native_level_key())?,
                }
            }
            None => return None,
        };
        Some(self.level_ref(entity.epoch(), key))
    }
}

/// A saved UUID's four ints (`UUIDUtil.uuidFromIntArray`) as one number.
fn uuid_from_words(words: [i32; 4]) -> u128 {
    words
        .iter()
        .fold(0, |uuid, w| (uuid << 32) | u128::from(*w as u32))
}

#[cfg(test)]
mod general_handle_tests {
    use super::tests::with_game;
    use crate::game::commands::Target;
    use crate::game::general::GeneralRow;
    use mod_api::{EntityRef, WorldEpoch};
    use sunrise_core::geometry::Vec3;

    fn row(id: u32, uuid: [i32; 4], fall: f64) -> GeneralRow {
        GeneralRow {
            id,
            uuid,
            join: sunrise_client::join_seq::JoinSeq::UNJOINED,
            type_slot: 0,
            position: Vec3::new(0.5, 64.0, 0.5),
            previous: Vec3::new(0.5, 64.0, 0.5),
            velocity: Vec3::ZERO,
            rotation: [0.0, 0.0],
            previous_rotation: [0.0, 0.0],
            owner: None,
            fall_distance: fall,
            custom_name: None,
            custom_name_visible: false,
            base: Default::default(),
            on_ground: false,
            tick_count: 0,
            no_physics: false,
            horizontal_collision: false,
            vertical_collision: false,
            vertical_collision_below: false,
            fluids: Default::default(),
            under_water: false,
            payload_version: 1,
            payload_bytes: Vec::new(),
            mount: None,
            vehicle_order: 0,
            boarding_cooldown: 0,
            attachment_lifetime: Default::default(),
        }
    }

    /// #792's stale-handle row, as the id counter really behaves: a removed
    /// row's handle refuses, and the next row never reuses the id — ids come
    /// from the game's one monotonic counter (`take_entity_id`,
    /// game/network/lan_entities.rs:25: `next_entity += 1`, never decreased,
    /// and `EntityRef`'s contract says ids are never reused while the world
    /// is open), so the old handle has nothing to latch onto. Red without
    /// the resident lookup refusing a gone id: the old handle would resolve.
    #[test]
    #[ignore = "daily"]
    fn removed_general_row_refuses_its_handle_and_the_next_row_takes_a_new_id() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let uuid = [21, 22, 23, 24];
            game.resident_entities.general.rows.push(row(42, uuid, 7.0));
            let handle = game.entity(42).expect("the row became a handle");
            assert_eq!(handle.id(), 42);
            assert!(game.native_entity_target(handle).is_some());

            // The row leaves; the handle refuses.
            game.resident_entities
                .general
                .rows
                .retain(|row| row.id != 42);
            assert!(game.entity(42).is_none());
            assert!(game.native_entity_target(handle).is_none());

            // The next row is id 43 — the counter moved on, the same UUID
            // notwithstanding — and the old handle still refuses.
            game.resident_entities.general.rows.push(row(43, uuid, 7.0));
            let fresh = game.entity(43).expect("the next row became a handle");
            assert_ne!(fresh.id(), handle.id());
            assert!(
                game.native_entity_target(handle).is_none(),
                "the old handle resolved the next row"
            );
            let target = game
                .native_entity_target(fresh)
                .expect("the next row's handle resolves");
            assert!(matches!(target, Target::General(43)));
            let next = game
                .resident_entities
                .general
                .rows
                .iter()
                .find(|row| row.id == 43)
                .expect("the next row is in its list");
            assert_eq!(next.fall_distance, 7.0);
        });
    }

    /// A handle from an earlier epoch of the world — what a reload's restore
    /// of the same UUID leaves in a mod's hands — refuses by the epoch check
    /// alone, before any row is consulted.
    #[test]
    #[ignore = "daily"]
    fn general_handle_of_an_earlier_epoch_refuses_by_the_epoch_check() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            game.resident_entities
                .general
                .rows
                .push(row(42, [21, 22, 23, 24], 7.0));
            let handle = game.entity(42).expect("the row became a handle");
            assert!(game.native_entity_target(handle).is_some());
            // A reload opens a new epoch; a handle minted against the old one
            // refuses even though a row of the same UUID and id is resident.
            let stale = EntityRef::new(WorldEpoch::new(handle.epoch().get() + 1), handle.id());
            assert!(game.native_entity_target(stale).is_none());
        });
    }
}

#[cfg(test)]
mod ray_tests {
    use super::*;

    fn entity(id: u32) -> EntityRef {
        EntityRef::new(mod_api::WorldEpoch::new(1), id)
    }

    /// A unit box with its lowest corner at `x` on the x axis, around the ray along x.
    fn at(x: f64) -> [[f64; 3]; 2] {
        [[x, -0.5, -0.5], [x + 1.0, 0.5, 0.5]]
    }

    #[test]
    fn clipping_reports_the_entry_and_distance_of_each_hit() {
        let to = Vec3::new(10.0, 0.0, 0.0);
        for (id, x, squared) in [(1, 6.0, 36.0), (2, 2.0, 4.0)] {
            assert_eq!(
                clip_entity_ray(Vec3::ZERO, to, Vec3::ZERO, entity(id), at(x)),
                Some((
                    EntityRayHit {
                        entity: entity(id),
                        location: [x, 0.0, 0.0],
                        distance: x,
                    },
                    squared,
                )),
            );
        }
        assert_eq!(
            clip_entity_ray(Vec3::ZERO, to, Vec3::ZERO, entity(3), at(20.0)),
            None,
            "the box past the segment's end is not hit"
        );
    }

    #[test]
    fn the_margin_grows_every_box_and_a_start_inside_one_misses_it() {
        // The box is above the ray: missed without a margin, hit with one.
        let above = [[3.0, 0.8, -0.5], [4.0, 1.8, 0.5]];
        let to = Vec3::new(10.0, 0.0, 0.0);
        assert!(clip_entity_ray(Vec3::ZERO, to, Vec3::ZERO, entity(1), above).is_none());
        let (grown, _) =
            clip_entity_ray(Vec3::ZERO, to, Vec3::new(1.0, 1.0, 1.0), entity(1), above)
                .expect("the ray enters the grown box");
        assert_eq!(
            grown.location,
            [2.0, 0.0, 0.0],
            "entered at the grown box's face"
        );
        // `AABB.clip` from inside finds no entry, as for a projectile still in its shooter.
        assert!(
            clip_entity_ray(Vec3::new(0.5, 0.0, 0.0), to, Vec3::ZERO, entity(1), at(0.0)).is_none()
        );
        // A negative margin deflates via `AABB.inflate`, and over-deflation swaps min/max.
        let grazing_from = Vec3::new(0.0, 0.375, 0.0);
        let grazing_to = Vec3::new(10.0, 0.375, 0.0);
        assert!(
            clip_entity_ray(grazing_from, grazing_to, Vec3::ZERO, entity(1), at(3.0)).is_some()
        );
        assert!(
            clip_entity_ray(
                grazing_from,
                grazing_to,
                Vec3::new(-0.25, -0.25, -0.25),
                entity(1),
                at(3.0),
            )
            .is_none()
        );
        let over_from = Vec3::new(0.0, 0.625, 0.0);
        let over_to = Vec3::new(10.0, 0.625, 0.0);
        assert!(clip_entity_ray(over_from, over_to, Vec3::ZERO, entity(1), at(3.0)).is_none());
        let (over, _) = clip_entity_ray(
            over_from,
            over_to,
            Vec3::new(-1.25, -1.25, -1.25),
            entity(1),
            at(3.0),
        )
        .expect("over-deflation swaps min/max like vanilla AABB");
        assert_eq!(over.location, [2.75, 0.625, 0.0]);
    }
}

#[cfg(test)]
pub(super) mod tests {
    /// Install an empty column only where a fixture needs existing bodies to be tracked.
    pub(in crate::game) fn fixture_resident_column(
        world: &mut sunrise_client::world::World,
        key: (i32, i32),
    ) {
        if !world.chunks.contains_key(&key) {
            let chunk = sunrise_client::chunk::Chunk::with_layout(key.0, key.1, world.layout());
            world.chunks.insert(key, std::sync::Arc::new(chunk));
        }
    }

    /// Run `test` on a game-sized stack, preserving libtest's thread name and panic payload.
    pub(in crate::game) fn run_with_game_stack(test: impl FnOnce() + Send + 'static) {
        let mut worker =
            std::thread::Builder::new().stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES);
        if let Some(name) = std::thread::current().name() {
            worker = worker.name(name.to_owned());
        }
        if let Err(payload) = worker.spawn(test).unwrap().join() {
            std::panic::resume_unwind(payload);
        }
    }

    /// Resolve a fixture after registration or a late Parts reinstall from its loaded table.
    pub(in crate::game) fn resolve_parts(game: &super::Game<'_>) -> Result<(), String> {
        crate::game::api_values::resolve_block_definitions(game.pack.assets.registry)?;
        mod_api::__private::resolve_items(game.items().registry_order.iter().map(|id| &**id))
            .map_err(|errors| format!("fixture item keys refused: {errors:?}"))?;
        game.resolve_parts().map_err(|error| error.to_string())
    }
    use crate::game::api_values::TryIntoApi;
    use core::ops::ControlFlow;
    use mod_api::__private::game::*;
    use mod_api::{EntityRef, LevelReadError, LevelRef};

    /// Collect only in fixtures that compare the whole result of an entity visit.
    pub(in crate::game) fn collect_entity_visit(
        enumerate: impl FnOnce(
            &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
        ) -> Result<ControlFlow<()>, mod_api::EntityReadError>,
    ) -> Result<Vec<EntityRef>, mod_api::EntityReadError> {
        let mut entities = Vec::new();
        let complete = enumerate(&mut |entity| {
            entities.push(entity);
            ControlFlow::Continue(())
        })?;
        assert_eq!(complete, ControlFlow::Continue(()));
        Ok(entities)
    }

    // Keep the handle and direct Game routes separate when collecting for assertions.
    pub(in crate::game) fn collect_all_entities(
        host: &dyn mod_api::Host,
        level: LevelRef,
    ) -> Result<Vec<EntityRef>, LevelReadError> {
        let mut entities = Vec::new();
        let _ = level.visit_all_entities(host, &mut |entity| {
            entities.push(entity);
            ControlFlow::Continue(())
        })?;
        Ok(entities)
    }

    pub(in crate::game) fn collect_entities_in(
        host: &dyn mod_api::Host,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Result<Vec<EntityRef>, LevelReadError> {
        let mut entities = Vec::new();
        let _ = level.visit_entities_in(host, min, max, &mut |entity| {
            entities.push(entity);
            ControlFlow::Continue(())
        })?;
        Ok(entities)
    }

    pub(in crate::game) fn collect_game_all_entities(
        game: &super::Game<'_>,
        level: LevelRef,
    ) -> Result<Vec<EntityRef>, LevelReadError> {
        let mut entities = Vec::new();
        let _ = game.level_visit_all_entities(level, &mut |entity| {
            entities.push(entity);
            ControlFlow::Continue(())
        })?;
        Ok(entities)
    }

    pub(in crate::game) fn collect_game_entities_in(
        game: &super::Game<'_>,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Result<Vec<EntityRef>, LevelReadError> {
        let mut entities = Vec::new();
        let _ = game.level_visit_entities_in(level, min, max, &mut |entity| {
            entities.push(entity);
            ControlFlow::Continue(())
        })?;
        Ok(entities)
    }

    /// Isolate declarations and frozen registries without changing another test's resources.
    pub(in crate::game) fn run_fresh_cases(name: &str, flag: &str, cases: &[&str]) -> bool {
        run_fresh_cases_at_once(name, flag, cases, 1)
    }

    /// [`run_fresh_cases`] with up to `at_once` children running together, for a test that runs
    /// alone in its job: each child still has its own process and folders.
    pub(in crate::game) fn run_fresh_cases_at_once(
        name: &str,
        flag: &str,
        cases: &[&str],
        at_once: usize,
    ) -> bool {
        run_fresh_cases_with_env(name, flag, cases, at_once, &[])
    }

    /// Set extra variables only on the isolated test processes.
    pub(in crate::game) fn run_fresh_cases_with_env(
        name: &str,
        flag: &str,
        cases: &[&str],
        at_once: usize,
        env: &[(&str, &str)],
    ) -> bool {
        if std::env::var_os(flag).is_some() {
            return false;
        }
        let resources = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").expect(
            "SUNRISE_TEST_RESOURCES must name the prepared resources (.local/sunrise-client/<version>)",
        ))
        .canonicalize()
        .expect("SUNRISE_TEST_RESOURCES names a folder that does not exist");
        let scratch_owner = tempfile::tempdir().unwrap();
        let target = scratch_owner.path();
        let run = |case: &str| {
            let scratch = target.join(format!("fresh-case-{case}"));
            std::fs::create_dir(&scratch).unwrap();
            let mut command = mod_platform::testing::fresh_case_command(name, flag, case);
            command
                .current_dir(&scratch)
                .envs(env.iter().copied())
                .env("SUNRISE_TEST_RESOURCES", &resources)
                .env("SUNRISE_NO_AUDIO", "1")
                .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1");
            for key in [
                "SUNRISE_HOME",
                "SUNRISE_LAUNCHER_HOME",
                "XDG_DATA_HOME",
                "XDG_CONFIG_HOME",
                "XDG_RUNTIME_DIR",
                "SUNRISE_LAUNCHER_TRASH",
                "SUNRISE_LAUNCHER_CLIPBOARD",
                "SUNRISE_LAUNCHER_DESKTOP",
            ] {
                let home = scratch.join(key);
                std::fs::create_dir(&home).unwrap();
                command.env(key, home);
            }
            let output = output_within(&mut command, name, case);
            std::fs::remove_dir_all(scratch).unwrap();
            output
        };
        for group in cases.chunks(at_once.max(1)) {
            let run = &run;
            let outputs: Vec<_> = std::thread::scope(|scope| {
                let running: Vec<_> = group
                    .iter()
                    .map(|&case| scope.spawn(move || run(case)))
                    .collect();
                running
                    .into_iter()
                    .map(|child| child.join().unwrap())
                    .collect()
            });
            for (case, output) in group.iter().zip(outputs) {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // The child's log, captured with this test's own output.
                print!("{stdout}");
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
                assert!(output.status.success(), "{name}: {case}");
                assert!(
                    ran_one_test(&stdout),
                    "{name}: {case}: the child matched no test of that name"
                );
            }
        }
        true
    }

    /// The longest a fresh child may run: a hang in its startup or shutdown fails the parent
    /// with the child killed, instead of waiting for the whole test run's own limit.
    const FRESH_CASE_LIMIT: std::time::Duration = std::time::Duration::from_secs(900);

    /// `command`'s output, killing the child if it outlives [`FRESH_CASE_LIMIT`].
    fn output_within(
        command: &mut std::process::Command,
        name: &str,
        case: &str,
    ) -> std::process::Output {
        use std::io::Read;
        use std::process::Stdio;
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let out = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            bytes
        });
        let err = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        });
        let deadline = std::time::Instant::now() + FRESH_CASE_LIMIT;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{name}: {case}: the child did not finish within {FRESH_CASE_LIMIT:?}");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        std::process::Output {
            status,
            stdout: out.join().unwrap(),
            stderr: err.join().unwrap(),
        }
    }

    /// Whether libtest's summary says exactly one test passed: `--exact` with a name no test
    /// has runs nothing and still exits 0.
    fn ran_one_test(stdout: &str) -> bool {
        stdout
            .lines()
            .any(|line| line.starts_with("test result: ok. 1 passed;"))
    }

    /// A child run with a name no test has fails the parent instead of passing vacuously.
    #[test]
    #[ignore = "daily"]
    #[should_panic(expected = "matched no test of that name")]
    fn a_fresh_case_whose_name_matches_no_test_fails() {
        run_fresh_cases(
            "game::native_entity_refs::tests::no_test_has_this_name",
            "SUNRISE_NO_MATCH_CHILD",
            &["missing"],
        );
    }

    /// The parent runs and checks an isolated child; the child loads the Part fixtures' types.
    pub(in crate::game) fn declared_part_root(name: &str) -> Option<std::path::PathBuf> {
        if run_fresh_cases(name, "SUNRISE_DECLARED_PART_GAME_CHILD", &["valid"]) {
            return None;
        }
        let _mods = mod_platform::__private::part_tests::game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = super::super::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        // game_thread installs a small synthetic key table for API-only tests.
        // This fixture loads a real resource graph: resolve its complete keys first.
        let (registry, _) = sunrise_world::level::data::load_registry_with_flags(&root).unwrap();
        let ids: std::collections::HashSet<_> = (0..registry.len())
            .map(|i| {
                registry
                    .state(sunrise_world::registry::BlockStateId(i as u32))
                    .unwrap()
                    .name
                    .clone()
            })
            .collect();
        crate::game::api_values::register_game_keys(ids);
        Some(root)
    }

    pub(in crate::game) fn prepare_entity_ticking_neighborhood(
        game: &mut super::Game<'_>,
        center: sunrise_core::position::ChunkPos,
    ) {
        for z in center.z - 2..=center.z + 2 {
            for x in center.x - 2..=center.x + 2 {
                if !game.world.chunks.contains_key(&(x, z)) {
                    let mut chunk = sunrise_client::chunk::Chunk::empty(x, z);
                    // A ticking column is lit (`ChunkMap.prepareTickingChunk`); `forceload` waits for it.
                    chunk.lit = true;
                    game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
                    game.block_ticks
                        .add_empty_chunk(sunrise_core::position::ChunkPos::new(x, z))
                        .unwrap();
                }
            }
        }
    }

    /// Bind the fixture body's canonical identity before configuring its acting context.
    pub(in crate::game) fn bind_local_player(game: &mut super::Game<'_>, id: u32, uuid: u128) {
        game.next_entity = game.next_entity.max(id);
        game.player.as_mut().expect("fixture player body").entity_id =
            i32::try_from(id).expect("fixture entity ID fits the protocol");
        game.level.player_uuid = Some(sunrise_client::lan::playerdata::uuid_ints(uuid));
        game.bind_local_player_residency();
        game.lan_acting_entity = Some(id);
        game.lan_acting_uuid = Some(uuid);
    }

    /// Install the server record backing a fixture's already configured acting body.
    pub(in crate::game) fn bind_acting_guest(game: &mut super::Game<'_>) {
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use sunrise_protocol::packets::configuration::ClientInformation;
        let conn = game.lan_acting.unwrap();
        let id = game.lan_acting_entity.unwrap();
        let uuid = game.lan_acting_uuid.unwrap();
        let name = game.lan_acting_name.clone().unwrap();
        let at = game.player.as_ref().unwrap().position;
        game.next_entity = game.next_entity.max(id);
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            },
            NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes.clone(),
                level_seeds: game.levels.runtime.seeds(),
            },
        )
        .unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid,
                name,
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            id as i32,
            [at.x, at.y, at.z],
            10,
        );
        guest.placed = true;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        game.reset_entity_residency();
    }

    pub(in crate::game) fn with_game(test: impl FnOnce(&mut super::Game<'_>)) {
        with_game_at(fixture_root(), test);
    }

    /// [`with_game`] with column (0, 0) and its neighbours loaded and ticking: entities are
    /// found in resident columns only (`Visibility.isAccessible`), so a fixture that pushes a
    /// body at the origin starts here.
    pub(in crate::game) fn with_resident_game(test: impl FnOnce(&mut super::Game<'_>)) {
        with_game(|game| {
            prepare_entity_ticking_neighborhood(game, sunrise_core::position::ChunkPos::new(0, 0));
            test(game);
        });
    }

    pub(in crate::game) fn fixture_root() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap())
    }

    pub(in crate::game) fn with_game_at(
        root: std::path::PathBuf,
        test: impl FnOnce(&mut super::Game<'_>),
    ) {
        with_game_role(
            root,
            super::super::execution_role::ExecutionRole::IntegratedServer,
            test,
        );
    }

    /// Keep a key that a test mod's `register` declared. The game fixture registers the
    /// installed mods again (`register_game_keys`), and a key's slot is stable for the process,
    /// so the repeat must declare the same key.
    pub(in crate::game) fn record_key<K: Copy + PartialEq + std::fmt::Debug>(
        slot: &std::sync::OnceLock<K>,
        key: K,
    ) {
        assert_eq!(
            *slot.get_or_init(|| key),
            key,
            "a repeat registration declared another key"
        );
    }

    /// Create a hidden desktop window during winit's initial `Resumed` callback.
    /// Keep the returned event loop alive while the fixture uses the window.
    pub(in crate::game) fn test_window(
        size: [u32; 2],
    ) -> (
        winit::event_loop::EventLoop<()>,
        std::sync::Arc<winit::window::Window>,
    ) {
        use winit::application::ApplicationHandler;
        use winit::dpi::PhysicalSize;
        use winit::event::WindowEvent;
        use winit::event_loop::{ActiveEventLoop, EventLoop};
        use winit::platform::pump_events::EventLoopExtPumpEvents;
        use winit::window::{Window, WindowId};

        struct HiddenWindow {
            size: [u32; 2],
            window: Option<Window>,
        }

        impl ApplicationHandler for HiddenWindow {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                let [width, height] = self.size;
                self.window = Some(
                    event_loop
                        .create_window(
                            Window::default_attributes()
                                .with_visible(false)
                                .with_inner_size(PhysicalSize::new(width, height)),
                        )
                        .unwrap(),
                );
            }

            fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        }

        let mut builder = EventLoop::builder();
        #[cfg(target_os = "linux")]
        {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11().with_any_thread(true);
        }
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        }
        let mut event_loop = builder.build().unwrap();
        let mut app = HiddenWindow { size, window: None };
        event_loop.pump_app_events(Some(std::time::Duration::ZERO), &mut app);
        let window = app
            .window
            .expect("winit did not resume to create the fixture window");
        (event_loop, std::sync::Arc::new(window))
    }

    pub(in crate::game) fn with_client_game(test: impl FnOnce(&mut super::Game<'_>)) {
        with_game_role(
            fixture_root(),
            super::super::execution_role::ExecutionRole::Client,
            test,
        );
    }

    pub(in crate::game) fn with_game_role(
        root: std::path::PathBuf,
        role: super::super::execution_role::ExecutionRole,
        test: impl FnOnce(&mut super::Game<'_>),
    ) {
        with_game_role_and_facts(root, role, None, test);
    }

    /// Configure the fixture's owned gameplay facts before constructing the game.
    pub(in crate::game) fn with_game_role_and_facts(
        root: std::path::PathBuf,
        role: super::super::execution_role::ExecutionRole,
        configure: Option<
            fn(&sunrise_world::registry::BlockRegistry, &mut sunrise_client::blocks::BlockFacts),
        >,
        test: impl FnOnce(&mut super::Game<'_>),
    ) {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use mod_api::Authority;

        // Parts registration replaces its metadata snapshot. Finish registration before
        // loading the resource graph, whose registry retains one immutable worker snapshot.
        let (registry, _) = sunrise_world::level::data::load_registry_with_flags(&root).unwrap();
        let ids: std::collections::HashSet<_> = (0..registry.len())
            .map(|i| {
                registry
                    .state(sunrise_world::registry::BlockStateId(i as u32))
                    .unwrap()
                    .name
                    .clone()
            })
            .collect();
        crate::game::api_values::register_game_keys(ids);
        // Parts registration also clears particle keys. Restore the load-owned
        // providers as content_root does, rather than relying on a prior test.
        super::super::mods::resolve_particle_providers(
            &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
        )
        .unwrap();
        let saves = tempfile::tempdir().unwrap();
        let settings = Settings::default();
        let mut loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            role,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        if let Some(configure) = configure {
            configure(
                loaded.pack.assets.registry,
                &mut std::sync::Arc::get_mut(&mut loaded.resources)
                    .unwrap()
                    .core
                    .facts,
            );
        }
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = role;
        options.game_dir = Some(saves.path().to_path_buf());
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.reset_entity_residency();
        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));

        // A game that opened its world resolves its parts; the harness stops
        // short of the open, so do it here for the by-state part table.
        resolve_parts(&game).unwrap();
        game.install_root_block_transformers();
        // The title's save folder is under the working directory: give each test its own, so a
        // test that saves leaves the checkout clean and parallel tests don't share files.
        game.save = std::sync::Arc::new(sunrise_client::save::SaveDir::new(
            saves.path().to_path_buf(),
            sunrise_client::save::LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        test(&mut game);
    }

    #[test]
    #[ignore = "daily"]
    fn typed_uuid_and_nbt_reads_follow_the_entity_level() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            // with_game prepares lifecycle, not world-open command registries.
            // execute in and summon must resolve real dimension and entity keys.
            game.build_registries();
            let words = [7, 0, 0, 3];
            let uuid = (7u128 << 96) | 3;
            // Harness commands use the console boundary; setting dedicated
            // options alone does not grant the command parser operator rights.
            assert!(game.as_console(|game| game.run_command(
                "execute in minecraft:the_nether run summon cow 8.5 70 8.5 {UUID:[I;7,0,0,3],NoAI:1b}"
            )));
            let original = game.dim;
            game.enter_command_level(super::super::Dim::Nether).unwrap();
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let id = *game
                .persist
                .uuids
                .iter()
                .find(|(_, value)| **value == words)
                .unwrap()
                .0;
            let entity = game.entity(id).unwrap();
            assert_eq!(game.entity_uuid(entity), Some(uuid));
            let nbt = game.entity_nbt(entity).unwrap();
            assert_eq!(
                nbt.get(&"NoAI".into()),
                Some(&sunrise_core::nbt::Tag::Byte(1))
            );
            assert!(!nbt.contains_key(&"id".into()));
            game.enter_command_level(original).unwrap();
            assert_eq!(game.entity_nbt(entity), None);
            game.enter_command_level(super::super::Dim::Nether).unwrap();
            assert_eq!(game.entity_uuid(entity), Some(uuid));
            assert_eq!(game.entity_nbt(entity), Some(nbt));
            game.enter_command_level(original).unwrap();
            assert_eq!(game.dim, original);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_allows_weak_power_in_real_game_level() {
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        use sunrise_core::position::BlockPos;

        mod_platform::__private::part_tests::weak_power_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let source = registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:lever".into(),
                    properties: [("face", "wall"), ("facing", "east"), ("powered", "true")]
                        .into_iter()
                        .map(|(key, value)| (key.into(), value.into()))
                        .collect(),
                })
                .unwrap();
            let pos = BlockPos::new(5, 64, 5);
            game.world.set(5, 64, 5, glass).unwrap();
            game.world.set(6, 64, 5, source).unwrap();
            let signal = game.with_level(|b, level, ticks| {
                b.best_neighbor_signal(level, ticks, BlockPos::new(pos.x - 1, pos.y, pos.z))
            });
            assert_eq!(signal, 15);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_placement_view_reads_redstone_and_collision_shapes() {
        use mod_api::{Direction, LevelHost};
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let lever = registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:lever".into(),
                    properties: [("face", "wall"), ("facing", "east"), ("powered", "true")]
                        .into_iter()
                        .map(|(key, value)| (key.into(), value.into()))
                        .collect(),
                })
                .unwrap();
            game.world.set(5, 64, 5, glass).unwrap();
            game.world.set(6, 64, 5, lever).unwrap();
            game.world.set(9, 64, 5, stone).unwrap();
            game.with_level(|_, level, ticks| {
                let view = level.view(ticks);
                // The glass has a powered lever on its east side; the stone nothing.
                assert_eq!(view.has_neighbor_signal([5, 64, 5]), Some(true));
                assert_eq!(view.best_neighbor_signal([5, 64, 5]), Some(15));
                assert_eq!(view.signal([5, 64, 5], Direction::East), Some(0));
                assert_eq!(view.direct_signal_to([5, 64, 5]), Some(15));
                assert_eq!(view.has_neighbor_signal([9, 64, 5]), Some(false));
                // The same reads from the level itself have no ticks to ask.
                assert_eq!(LevelHost::has_neighbor_signal(&*level, [5, 64, 5]), None);
                // A full block's collision box, none for air, nothing for an unloaded position.
                for host in [&view as &dyn LevelHost, &*level] {
                    assert_eq!(
                        host.collision_shape([9, 64, 5])
                            .map(|shape| shape.boxes().collect::<Vec<_>>()),
                        Some(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
                    );
                    assert!(host.collision_shape([9, 70, 5]).unwrap().is_empty());
                    assert!(host.collision_shape([160, 64, 5]).is_none());
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_soil_part_answers_for_the_plants_above_it_in_a_real_game_level() {
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        use sunrise_core::position::BlockPos;

        // The fixture's parts: glass sustains any plant, grass sustains none.
        mod_platform::__private::part_tests::sustain_plant_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let dandelion = state("minecraft:dandelion");
            let survives = |game: &mut super::super::Game<'_>, soil: &str| {
                game.world.set(5, 64, 5, state(soil)).unwrap();
                game.with_level(|b, level, ticks| {
                    b.can_survive(level, ticks, BlockPos::new(5, 65, 5), dandelion)
                })
            };
            // Allow: vanilla refuses a dandelion on glass; the soil's part lets it stand.
            assert!(survives(game, "minecraft:glass"));
            // Deny: vanilla lets a dandelion stand on grass; the soil's part refuses it.
            assert!(!survives(game, "minecraft:grass_block"));
            // Default: soils with no part answer as vanilla's plant rule does.
            assert!(survives(game, "minecraft:dirt"));
            assert!(!survives(game, "minecraft:stone"));
            // A non-authoritative copy of the level asks the part too (the client side itself is
            // covered by `client_placement_prediction_asks_placement_and_survival_parts`).
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            game.options.dedicated = None;
            assert!(survives(game, "minecraft:glass"));
            assert!(!survives(game, "minecraft:grass_block"));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_part_decides_whether_the_tool_harvests() {
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, motion::PlayerMotion};
        use sunrise_core::{geometry::Vec3, position::ChunkPos};

        // The fixture's parts: stone is harvested by any tool, dirt by none.
        mod_platform::__private::part_tests::harvest_game_thread();
        with_game(|game| {
            let scratch = std::env::temp_dir().join(format!(
                "sunrise-harvest-part-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&scratch).unwrap();
            game.world_root = scratch.clone();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 62.0, 4.5)).unwrap());
            game.player.as_mut().unwrap().ground.on_ground = true;
            game.creative = false;
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("gamemode survival"));
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let pos = [5, 63, 4];
            // The dig's progress and the drops of a break by bare hand: (block, its hardness, what
            // vanilla drops by hand, and what the drop is). Vanilla's own rule: a block that
            // requires a tool drops nothing by hand, and the dig is 3.33 times slower for it.
            for (block, hardness, part, by_hand, drop) in [
                // Stone needs a pickaxe; the part harvests it anyway.
                (
                    "minecraft:stone",
                    1.5_f32,
                    Some(true),
                    false,
                    "minecraft:cobblestone",
                ),
                // Dirt needs none; the part refuses it.
                ("minecraft:dirt", 0.5, Some(false), true, "minecraft:dirt"),
                // No part: vanilla's rule alone, both ways.
                (
                    "minecraft:cobblestone",
                    2.0,
                    None,
                    false,
                    "minecraft:cobblestone",
                ),
                ("minecraft:grass_block", 0.6, None, true, "minecraft:dirt"),
            ] {
                let state = registry.default_state(block).unwrap();
                let harvest = !part.unwrap_or(by_hand);
                game.world.set(pos[0], pos[1], pos[2], state).unwrap();
                let divisor = if harvest { 30.0 } else { 100.0 };
                assert_eq!(
                    game.destroy_progress(pos),
                    1.0 / hardness / divisor,
                    "{block}"
                );
                game.dropped.clear();
                game.break_block(pos).unwrap();
                let dropped = game.dropped.iter().any(|item| &*item.stack.item == drop);
                assert_eq!(dropped, harvest, "{block}: drops {drop}");
            }
            std::fs::remove_dir_all(scratch).unwrap();
        });
    }

    /// A world with block `support` at (5, 64, 5) and a powered lever on its east face.
    fn lever_on(game: &mut super::super::Game<'_>, support: &str) {
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;

        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let support = registry.default_state(support).unwrap();
        let lever = registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:lever".into(),
                properties: [("face", "wall"), ("facing", "east"), ("powered", "true")]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            })
            .unwrap();
        game.world.set(5, 64, 5, support).unwrap();
        game.world.set(6, 64, 5, lever).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_handle_reads_redstone_as_signal_getter_does() {
        use sunrise_core::geometry::Direction::{East, North, West};

        with_game(|game| {
            lever_on(game, "minecraft:stone");
            let level = game.level(game.dim.id()).unwrap();
            let (stone, lever, beside, far) = ([5, 64, 5], [6, 64, 5], [4, 64, 5], [5, 64, 9]);
            // A powered lever: weak 15 every way, strong 15 only into the block it hangs on,
            // which lies the way it faces from that block (LeverBlock.getDirectSignal).
            assert_eq!(level.signal(game, lever, North), Some(15));
            assert_eq!(level.direct_signal(game, lever, East), Some(15));
            assert_eq!(level.direct_signal(game, lever, North), Some(0));
            // Stone conducts the strong power to the block west of it.
            assert_eq!(level.direct_signal_to(game, stone), Some(15));
            assert_eq!(level.signal(game, stone, West), Some(15));
            assert_eq!(level.best_neighbor_signal(game, beside), Some(15));
            assert!(level.has_neighbor_signal(game, beside));
            // Nothing gives strong power into `beside`, and nothing reaches `far`.
            assert_eq!(level.direct_signal_to(game, beside), Some(0));
            assert_eq!(level.best_neighbor_signal(game, far), Some(0));
            assert!(!level.has_neighbor_signal(game, far));
        });
        // Glass is no conductor: the lever's power stops at it.
        with_game(|game| {
            lever_on(game, "minecraft:glass");
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(level.best_neighbor_signal(game, [4, 64, 5]), Some(0));
            assert_eq!(level.signal(game, [5, 64, 5], West), Some(0));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_bone_meals_a_block_and_sets_a_biome_cell() {
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let farmland = registry.default_state("minecraft:farmland").unwrap();
            let wheat = registry.default_state("minecraft:wheat").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(0, 63, 0, farmland).unwrap();
            game.world.set(0, 64, 0, wheat).unwrap();
            game.world.set(5, 64, 5, stone).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            assert!(game.level_bone_meal(level, [0, 64, 0]));
            assert_ne!(game.world.block(0, 64, 0), wheat, "the crop grew");
            assert!(!game.level_bone_meal(level, [5, 64, 5]), "stone");
            assert!(!game.level_bone_meal(level, [100_000, 64, 0]), "unloaded");

            let current = usize::from(game.world.chunks[&(0, 0)].biome(0, 64, 0));
            let target = usize::from(current == 0);
            let name = world_biomes!(game).list[target].name.clone();
            assert!(game.level_set_biome(level, [0, 64, 0], &name));
            assert_eq!(
                usize::from(game.world.chunks[&(0, 0)].biome(3, 67, 3)),
                target,
                "the whole 4x4x4 cell changed"
            );
            assert!(game.world.modified.contains(&(0, 0)));
            assert!(
                !game.level_set_biome(level, [0, 64, 0], &name),
                "already set"
            );
            assert!(!game.level_set_biome(level, [0, 64, 0], "minecraft:nope"));
            assert!(!game.level_set_biome(level, [100_000, 64, 0], &name));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_handle_updates_neighbours_as_vanillas_writes_do() {
        use mod_api::NeighbourUpdate::{Neighbours, NeighboursExcept, OutputSignal};
        use sunrise_core::geometry::Direction::East;

        let lit = |game: &super::super::Game<'_>, [x, y, z]: [i32; 3]| {
            let state = game.world.block(x, y, z);
            game.block_state_property(state, "lit") == Some("true")
        };
        let set = |game: &mut super::super::Game<'_>, [x, y, z]: [i32; 3], name: &str| {
            let state = game.pack.assets.registry.default_state(name).unwrap();
            game.world.set(x, y, z, state).unwrap();
        };
        // The powered lever at (6, 64, 5) on stone; unlit lamps beside it, east and above,
        // placed without updates, so they don't know it powers them yet.
        let (lever, east, above) = ([6, 64, 5], [7, 64, 5], [6, 65, 5]);
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            let level = game.level(game.dim.id()).unwrap();
            let state = game.world.block(6, 64, 5);
            set(game, east, "minecraft:redstone_lamp");
            set(game, above, "minecraft:redstone_lamp");
            // `updateNeighborsAtExceptFromFacing(lever, East)`: every neighbour but the east one.
            assert_eq!(
                level.update_neighbors(
                    game,
                    lever,
                    state.try_into_api().unwrap(),
                    NeighboursExcept(East)
                ),
                Ok(true)
            );
            assert!(lit(game, above) && !lit(game, east));
            // `updateNeighborsAt(lever)`: the east lamp hears it too.
            assert_eq!(
                level.update_neighbors(game, lever, state.try_into_api().unwrap(), Neighbours),
                Ok(true)
            );
            assert!(lit(game, east));
        });
        // `Level.neighborChanged(east, lever)`: only that lamp hears.
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            let level = game.level(game.dim.id()).unwrap();
            let state = game.world.block(6, 64, 5);
            set(game, east, "minecraft:redstone_lamp");
            set(game, above, "minecraft:redstone_lamp");
            assert_eq!(
                level.neighbor_changed(game, east, state.try_into_api().unwrap()),
                Ok(true)
            );
            assert!(lit(game, east) && !lit(game, above));
        });
        // A repeater in front of the lever, reading it, schedules its own tick when told (a
        // diode's `checkTickOnNeighbor`), through the level's queue; told again while that tick
        // is pending, it schedules no second one (`willTickThisTick` / `hasScheduledTick`).
        // Hooks that run inside the update get level hosts, not a `ServerHost`, so a mod
        // can't call `update_neighbors` from within one: re-entry goes through the queue.
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let level = game.level(game.dim.id()).unwrap();
            set(game, [7, 63, 5], "minecraft:stone");
            let repeater = game
                .pack
                .assets
                .registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:repeater".into(),
                    properties: [
                        ("facing", "west"),
                        ("delay", "1"),
                        ("locked", "false"),
                        ("powered", "false"),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
                })
                .unwrap();
            game.world.set(7, 64, 5, repeater).unwrap();
            let block = game.res.behaviours().block(repeater);
            let at = sunrise_core::position::BlockPos::new(7, 64, 5);
            let state = game.world.block(6, 64, 5);
            assert!(!game.block_ticks.has_block_tick(at, block));
            assert_eq!(
                level.update_neighbors(game, lever, state.try_into_api().unwrap(), Neighbours),
                Ok(true)
            );
            assert!(game.block_ticks.has_block_tick(at, block));
            assert_eq!(
                level.update_neighbors(game, lever, state.try_into_api().unwrap(), Neighbours),
                Ok(true)
            );
            assert!(game.block_ticks.has_block_tick(at, block));
            assert_eq!(
                game.world.block(7, 64, 5),
                repeater,
                "unpowered until its tick runs"
            );
        });
        // `updateNeighbourForOutputSignal(cake)`: the comparator reading the cake, which
        // outputs 14 when whole while the comparator still reads 0, schedules its tick.
        with_game(|game| {
            lever_on(game, "minecraft:stone");
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let level = game.level(game.dim.id()).unwrap();
            // Both stand on stone, as they must to survive a neighbour update.
            set(game, [10, 63, 5], "minecraft:stone");
            set(game, [11, 63, 5], "minecraft:stone");
            set(game, [10, 64, 5], "minecraft:cake");
            let comparator = game
                .pack
                .assets
                .registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:comparator".into(),
                    properties: [
                        ("facing", "west"),
                        ("mode", "compare"),
                        ("powered", "false"),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
                })
                .unwrap();
            game.world.set(11, 64, 5, comparator).unwrap();
            let block = game.res.behaviours().block(comparator);
            let at = sunrise_core::position::BlockPos::new(11, 64, 5);
            assert!(!game.block_ticks.has_block_tick(at, block));
            let cake = game.world.block(10, 64, 5);
            assert_eq!(
                level.update_neighbors(
                    game,
                    [10, 64, 5],
                    cake.try_into_api().unwrap(),
                    OutputSignal
                ),
                Ok(true)
            );
            assert!(game.block_ticks.has_block_tick(at, block));
            // Off the level in place: refused, nothing done.
            assert_eq!(
                level.update_neighbors(
                    game,
                    [10, 64, 500_000],
                    cake.try_into_api().unwrap(),
                    OutputSignal
                ),
                Ok(false)
            );
            // A source that is no block state: refused, not a panic.
            let known: mod_api::BlockState = cake.try_into_api().unwrap();
            let unknown: mod_api::BlockState =
                mod_api::__private::block_state_from_ordinal(known.block(), u16::MAX);
            assert_eq!(
                level.update_neighbors(game, [10, 64, 5], unknown, Neighbours),
                Err(mod_api::LevelWriteError::InvalidState(
                    mod_api::BlockStateError::UnknownState
                ))
            );
            assert_eq!(
                level.neighbor_changed(game, [11, 64, 5], unknown),
                Err(mod_api::LevelWriteError::InvalidState(
                    mod_api::BlockStateError::UnknownState
                ))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn neighbor_changed_reads_its_inputs_and_its_tick_fires_at_its_delay() {
        use mod_platform::__private::part_tests::{
            SignalRead, signal_reader_game_thread, signal_reads,
        };

        // The fixture's part on glass reads its inputs in `neighbor_changed` and schedules
        // its own tick 2 ticks later, as a diode does.
        signal_reader_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let lever = |powered: &str| {
                registry
                    .resolve(&sunrise_world::chunk::BlockState {
                        name: "minecraft:lever".into(),
                        properties: [("face", "wall"), ("facing", "east"), ("powered", powered)]
                            .into_iter()
                            .map(|(key, value)| (key.into(), value.into()))
                            .collect(),
                    })
                    .unwrap()
            };
            // A block-ticking chunk: lit, its neighbours loaded, with its tick list, as
            // loading gives one.
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = sunrise_client::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            // A lever on the east face of stone, and glass north of the lever: the lever's
            // weak power reaches it from the south.
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(5, 64, 5, stone).unwrap();
            game.world.set(6, 64, 4, glass).unwrap();
            game.world.set(6, 64, 5, lever("false")).unwrap();
            let dimension = game.dim.id().to_owned();
            let start = game.world.game_time;
            assert!(signal_reads().is_empty());

            // Pulling the lever updates its neighbours (`Block.UPDATE_ALL`), the glass among them.
            assert!(game.native_set_block(&dimension, [6, 64, 5], lever("true"), 3));
            assert_eq!(
                signal_reads(),
                [SignalRead::Changed([6, 64, 4], true, 15, true)]
            );
            // The tick it scheduled runs through the level's own queue, 2 ticks later, and
            // reads the lever south of it from `tick_block` (`LevelWriteHost: LevelTicksHost`).
            for (elapsed, reads) in [(1, vec![]), (2, vec![SignalRead::Ticked([6, 64, 4], 15)])] {
                game.world.game_time = start + elapsed;
                game.tick_scheduled().unwrap();
                assert_eq!(signal_reads(), reads, "after {elapsed} ticks");
            }
        });
    }

    /// `RedstoneLampBlock.neighborChanged` sets its own block in the call
    /// (`level.setBlock(pos, state.cycle(LIT), 2)`): a part's write in `neighbor_changed`
    /// lands within the lever's update, and its own neighbour updates run next, depth first,
    /// before the lever's next neighbour hears (`CollectingNeighborUpdater`).
    #[test]
    #[ignore = "daily"]
    fn neighbor_changed_writes_the_level_and_its_updates_run_next() {
        use mod_platform::__private::part_tests::{neighbor_writer_game_thread, neighbor_writes};

        // The fixture installs its mod for the process: run it in a child of its own.
        if run_fresh_cases(
            "game::native_entity_refs::tests::neighbor_changed_writes_the_level_and_its_updates_run_next",
            "SUNRISE_NEIGHBOR_WRITER_CHILD",
            &["lever"],
        ) {
            return;
        }
        neighbor_writer_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let lit = registry
                .default_state("minecraft:white_stained_glass")
                .unwrap();
            let lever = |powered: &str| {
                registry
                    .resolve(&sunrise_world::chunk::BlockState {
                        name: "minecraft:lever".into(),
                        properties: [("face", "wall"), ("facing", "east"), ("powered", powered)]
                            .into_iter()
                            .map(|(key, value)| (key.into(), value.into()))
                            .collect(),
                    })
                    .unwrap()
            };
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = sunrise_client::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            // A lever on the east face of stone. `NeighborUpdater.UPDATE_ORDER` is west, east,
            // down, up, north, south: glass `a` east of the lever hears second (west is the
            // stone), glass `b` above it fourth. Glass `c` east of `a` hears only `a`'s write.
            let (a, b, c) = ([7, 64, 5], [6, 65, 5], [8, 64, 5]);
            game.world
                .set(5, 64, 5, registry.default_state("minecraft:stone").unwrap())
                .unwrap();
            game.world.set(6, 64, 5, lever("false")).unwrap();
            for [x, y, z] in [a, b, c] {
                game.world.set(x, y, z, glass).unwrap();
            }
            let dimension = game.dim.id().to_owned();
            let key = |state: sunrise_world::registry::BlockStateId| {
                use crate::game::api_values::TryIntoApi;
                let state: mod_api::BlockState = state.try_into_api().unwrap();
                state.block()
            };
            assert!(neighbor_writes().is_empty());

            assert!(game.native_set_block(&dimension, [6, 64, 5], lever("true"), 3));
            assert_eq!(
                neighbor_writes(),
                [
                    (a, key(lever("true"))),
                    // `Level.setBlock` names the old block as the source (`oldState.getBlock()`).
                    (c, key(glass)),
                    (b, key(lever("true"))),
                ]
            );
            // Both glasses the lever reached changed in the same call, with no tick run.
            for [x, y, z] in [a, b] {
                assert_eq!(game.world.block(x, y, z), lit);
            }
            assert_eq!(game.world.block(c[0], c[1], c[2]), glass);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_level_handle_reads_redstone_through_should_check_weak_power() {
        use sunrise_core::geometry::Direction::West;

        // The fixture's part lets glass conduct (`should_check_weak_power`).
        mod_platform::__private::part_tests::weak_power_game_thread();
        with_game(|game| {
            lever_on(game, "minecraft:glass");
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(level.signal(game, [5, 64, 5], West), Some(15));
            assert_eq!(level.best_neighbor_signal(game, [4, 64, 5]), Some(15));
            // Strong power into the glass is its own; the part changes only what it conducts.
            assert_eq!(level.direct_signal_to(game, [5, 64, 5]), Some(15));
            assert_eq!(level.direct_signal_to(game, [4, 64, 5]), Some(0));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_answers_is_pathfindable_once_per_state_and_type() {
        use mod_api::entity::mobs::PathComputationType::{self, Air, Land, Water};

        // Before any part: every state answers as the vanilla export says.
        with_game(|game| {
            let data = &game.res.rest().mob_data;
            for (i, vanilla) in data.states.iter().enumerate() {
                for (kind, bit) in [(Land, 1), (Water, 2), (Air, 4)] {
                    let state = sunrise_world::registry::BlockStateId(i as u32);
                    assert_eq!(data.pathfindable(state, kind), vanilla.flags & bit != 0);
                }
            }
        });
        mod_platform::__private::part_tests::pathfindable_game_thread();
        with_game(|game| {
            let registry = game.pack.assets.registry;
            let asks = |name: &str, kind: PathComputationType| {
                let state = registry.default_state(name).unwrap();
                game.res.rest().mob_data.pathfindable(state, kind)
            };
            // Glass: land and air answered, water left to the block (vanilla: none of them).
            assert!(asks("minecraft:glass", Land));
            assert!(asks("minecraft:glass", Air));
            assert!(!asks("minecraft:glass", Water));
            // Air: land refused, water allowed, air left to the block.
            assert!(!asks("minecraft:air", Land));
            assert!(asks("minecraft:air", Water));
            assert!(asks("minecraft:air", Air));
            // A block with no part answers as before.
            assert!(!asks("minecraft:stone", Land));
            // A reload without the mod's answers takes them back.
            game.res.mob_data().resolve_parts(
                game.res.facts(),
                &sunrise_client::mobs::data::PartAnswers::default(),
            );
            assert!(!asks("minecraft:glass", Land));
            assert!(asks("minecraft:air", Land));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_asks_neighbours_to_update_when_its_block_is_removed() {
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        use sunrise_core::position::BlockPos;

        mod_platform::__private::part_tests::removal_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let lamp_state = |lit: &str| {
                registry
                    .resolve(&sunrise_world::chunk::BlockState {
                        name: "minecraft:redstone_lamp".into(),
                        properties: [("lit".into(), lit.into())].into_iter().collect(),
                    })
                    .unwrap()
            };
            let power = registry.default_state("minecraft:redstone_block").unwrap();
            // An unlit lamp beside a redstone block lights when told a neighbour changed, and
            // set into the world it hears nothing: the proof the update reached it. Moved by a
            // piston, a removal tells no neighbour itself, so only the part's request can.
            let lamp = BlockPos::new(6, 64, 5);
            for (state, heard) in [(stone, false), (glass, true)] {
                game.world.set(5, 64, 5, state).unwrap();
                game.world.set(6, 64, 5, lamp_state("false")).unwrap();
                game.world.set(7, 64, 5, power).unwrap();
                let lit = game.with_level(|b, level, ticks| {
                    // Flags 82: how a piston pushes a block out (`PistonBaseBlock.moveBlocks`).
                    use sunrise_world::level::flags;
                    let flags = flags::UPDATE_MOVE_BY_PISTON
                        | flags::UPDATE_KNOWN_SHAPE
                        | flags::UPDATE_CLIENTS;
                    b.set_block(
                        level,
                        ticks,
                        BlockPos::new(5, 64, 5),
                        registry.default_state("minecraft:air").unwrap(),
                        flags,
                    );
                    sunrise_world::level::LevelAccess::block_state(level, lamp)
                        == lamp_state("true")
                });
                assert_eq!(lit, heard);
            }
        });
    }

    /// A plain piston pushing east at (5, `y`, 5) with `block` at (6, `y`, 5) and `above` on top
    /// of it, powered from (4, `y`, 5): whether the block above moved with the one below. Moved,
    /// its old cell is air once the push started. Each scenario has its own `y`, so none meets
    /// the moving blocks of another.
    fn piston_pulls_above(game: &mut super::Game<'_>, y: i32, block: &str, above: &str) -> bool {
        use sunrise_core::position::BlockPos;
        let registry = game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        let air = state("minecraft:air");
        game.with_level(|b, level, ticks| {
            let piston = |extended: &str| {
                registry
                    .resolve(&sunrise_world::chunk::BlockState {
                        name: "minecraft:piston".into(),
                        properties: [
                            ("facing".into(), "east".into()),
                            ("extended".into(), extended.into()),
                        ]
                        .into_iter()
                        .collect(),
                    })
                    .unwrap()
            };
            let mut place = |pos: [i32; 3], s| {
                let flags = sunrise_world::level::flags::UPDATE_ALL;
                b.set_block(
                    level,
                    ticks,
                    BlockPos::new(pos[0], pos[1], pos[2]),
                    s,
                    flags,
                );
            };
            place([5, y, 5], piston("false"));
            place([6, y, 5], state(block));
            place([6, y + 1, 5], state(above));
            place([4, y, 5], state("minecraft:redstone_block"));
            b.run_block_events(level, ticks);
            assert_eq!(
                sunrise_world::level::LevelAccess::block_state(level, BlockPos::new(5, y, 5)),
                piston("true"),
                "{block}: the piston extended"
            );
            sunrise_world::level::LevelAccess::block_state(level, BlockPos::new(6, y + 1, 5)) == air
        })
    }

    #[test]
    #[ignore = "daily"]
    fn block_parts_decide_what_a_piston_pulls_along() {
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;

        mod_platform::__private::part_tests::sticky_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            // The block below, the block on top of it, and whether the top one moves.
            for ((block, above, pulled), y) in [
                // No part: vanilla's slime and honey, and nothing else sticks.
                ("minecraft:slime_block", "minecraft:stone", true),
                ("minecraft:honey_block", "minecraft:stone", true),
                ("minecraft:slime_block", "minecraft:honey_block", false),
                ("minecraft:stone", "minecraft:stone", false),
                // A sticky part pulls whatever its `can_stick_to` allows: diamond all of it,
                // gold only gold.
                ("minecraft:diamond_block", "minecraft:stone", true),
                ("minecraft:diamond_block", "minecraft:honey_block", true),
                ("minecraft:gold_block", "minecraft:gold_block", true),
                ("minecraft:gold_block", "minecraft:stone", false),
                ("minecraft:gold_block", "minecraft:honey_block", false),
                // Either side may refuse: honey is held back by gold's rule though it is sticky.
                ("minecraft:honey_block", "minecraft:gold_block", false),
                // A slime part is kept apart from honey as slime is.
                ("minecraft:emerald_block", "minecraft:honey_block", false),
                ("minecraft:emerald_block", "minecraft:stone", true),
            ]
            .into_iter()
            .zip((8..).step_by(4))
            {
                assert_eq!(
                    piston_pulls_above(game, y, block, above),
                    pulled,
                    "{above} on {block}"
                );
            }
        });
    }

    /// The restitution an entity landing on a block reads is asked of the block's parts in the
    /// entity's level, on the side that ticks it, with the entity and the block's position; a
    /// block without a part is not asked, and a non-finite answer is ignored.
    #[test]
    #[ignore = "daily"]
    fn a_block_part_changes_the_bounce_restitution_an_entity_reads() {
        crate::game::in_own_process(
            "game::native_entity_refs::tests::a_block_part_changes_the_bounce_restitution_an_entity_reads",
            block_part_bounce_restitution_in_own_process,
        );
    }

    fn block_part_bounce_restitution_in_own_process() {
        use sunrise_client::block_contact::{Contact, bounce_restitution};
        use sunrise_core::position::BlockPos;

        let heard = mod_platform::__private::part_tests::restitution_game_thread();
        crate::game::native_hooks::install_block_contact_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let (glass, stone, slime) = (
                state("minecraft:glass"),
                state("minecraft:stone"),
                state("minecraft:slime_block"),
            );
            let pos = BlockPos::new(3, 64, 5);
            let mut read = |block, own, client_side| {
                game.world.set(pos.x, pos.y, pos.z, block).unwrap();
                let contact = Contact {
                    entity: 7,
                    kind: "minecraft:pig",
                    state: block,
                    position: pos,
                };
                use sunrise_world::native_load_context::{
                    DataLoadOrigin, LoadContext, LoadInputs, LoopSide,
                };
                let context = LoadContext::new(
                    DataLoadOrigin::ServerDataLoad,
                    std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
                    [],
                )
                .captured_loop_context(if client_side {
                    LoopSide::Client
                } else {
                    LoopSide::Server
                });
                bounce_restitution(
                    sunrise_client::motion::FrictionLevel {
                        world: &game.world,
                        registry,
                        context: &context,
                        side: &game_side!(game),
                        coefficient: &|id| {
                            game.res
                                .facts()
                                .states
                                .get(id.0 as usize)
                                .map(|p| p.friction)
                        },
                    },
                    contact,
                    own,
                )
            };
            // Glass's part answers whatever the block's own value: 0.5 on a server, 0.25 on a
            // client predicting its own bounce.
            assert_eq!(read(glass, 0.0, false), 0.5);
            assert_eq!(read(glass, 1.0, true), 0.25);
            // A block with no part keeps its own: slime bounces fully.
            assert_eq!(read(slime, 1.0, false), 1.0);
            // Stone's part answers not a number, which is ignored.
            assert_eq!(read(stone, 0.0, false), 0.0);
            let pig = |client| (client, 7, "minecraft:pig".to_owned(), pos, true);
            assert_eq!(*heard.lock().unwrap(), [pig(false), pig(true)]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_connects_wire_in_a_real_game_level() {
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        use sunrise_core::{geometry::Direction, position::BlockPos};

        mod_platform::__private::part_tests::redstone_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let wire = registry.default_state("minecraft:redstone_wire").unwrap();
            let glass = registry.default_state("minecraft:glass").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let pos = BlockPos::new(5, 64, 5);
            let neighbour = BlockPos::new(6, 64, 5);
            game.world.set(5, 63, 5, stone).unwrap();
            game.world.set(6, 64, 5, glass).unwrap();
            let connected = game.with_level(|b, level, ticks| {
                b.update_shape(level, ticks, wire, pos, Direction::East, neighbour, glass)
            });
            assert_eq!(
                registry.state(connected).unwrap().properties["east"],
                "side"
            );
            game.world.set(6, 64, 5, stone).unwrap();
            let vanilla = game.with_level(|b, level, ticks| {
                b.update_shape(level, ticks, wire, pos, Direction::East, neighbour, stone)
            });
            assert_eq!(registry.state(vanilla).unwrap().properties["east"], "none");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_flips_fence_wall_and_pane_connections_in_a_real_game_level() {
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        use sunrise_core::{geometry::Direction, position::BlockPos};

        mod_platform::__private::part_tests::connection_game_thread();
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let stone = state("minecraft:stone");
            let torch = state("minecraft:torch");
            let pos = BlockPos::new(5, 64, 5);
            let neighbour = BlockPos::new(6, 64, 5);
            let side = |game: &mut crate::game::Game<'_>, block, beside| {
                game.world.set(6, 64, 5, beside).unwrap();
                let shaped = game.with_level(|b, level, ticks| {
                    b.update_shape(level, ticks, block, pos, Direction::East, neighbour, beside)
                });
                registry.state(shaped).unwrap().properties["east"].clone()
            };
            game.world.set(5, 63, 5, stone).unwrap();
            // Vanilla joins each to a sturdy face and not to a torch. The part on
            // the oak fence, cobblestone wall and glass pane flips both answers;
            // the spruce fence, brick wall and iron bars have none.
            for (block, with_part, yes, no) in [
                ("oak_fence", true, "true", "false"),
                ("spruce_fence", false, "true", "false"),
                ("cobblestone_wall", true, "low", "none"),
                ("brick_wall", false, "low", "none"),
                ("glass_pane", true, "true", "false"),
                ("iron_bars", false, "true", "false"),
            ] {
                let block_state = state(&format!("minecraft:{block}"));
                let (sturdy, not) = if with_part { (no, yes) } else { (yes, no) };
                assert_eq!(side(game, block_state, stone), sturdy, "{block} by stone");
                assert_eq!(side(game, block_state, torch), not, "{block} by a torch");
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn side_guards_agree_across_integrated_dedicated_and_client() {
        use super::super::execution_role::ExecutionRole;
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        with_game(|game| {
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            let level = game.level(game.dim.id()).unwrap();
            // The integrated server thread (game/world/integrated.rs sets the role and
            // `dedicated` on the serving Game) and a dedicated server simulate;
            // a client never does. A remote client runs with the `Client` role
            // (connecting changes `remote`, not the role), and the guards read
            // only the role, so it answers like the title-screen game here.
            for (role, dedicated) in [
                (ExecutionRole::IntegratedServer, true),
                (ExecutionRole::DedicatedServer, true),
            ] {
                game.options.execution_role = role;
                game.options.dedicated = dedicated.then(Vec::new);
                assert!(game.native_simulates());
                assert!(game.level(game.dim.id()).is_some());
                assert!(game.level_in_place(level));
                assert!(game.native_level_write_allowed(level, [-32, 64, 48]));
            }
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            assert!(!game.native_simulates());
            assert!(game.level(game.dim.id()).is_none());
            assert!(!game.level_in_place(level));
            assert!(!game.native_level_write_allowed(level, [-32, 64, 48]));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn chunk_handles_resolve_loaded_parked_and_stale_levels() {
        use mod_api::{Authority, ChunkStatus};
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            let chunk = game.chunk(level, [-2, 3]).unwrap();
            assert_eq!(chunk.pos(), [-2, 3]);
            assert_eq!(chunk.level(), level);
            assert!(chunk.loaded(game));
            assert_eq!(chunk.status(game), Some(ChunkStatus::Full));
            assert!(game.chunk(level, [99, 99]).is_none());
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(chunk.status(game), Some(ChunkStatus::Full));
            assert_eq!(game.chunk(level, [-2, 3]), Some(chunk));
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            game.world.chunks.remove(&(-2, 3));
            assert!(!chunk.loaded(game));
            assert_eq!(chunk.status(game), None);
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            game.native_lifecycle
                .begin_open(Some(Authority::IntegratedServer));
            assert!(game.chunk(level, [-2, 3]).is_none());
            assert!(!chunk.loaded(game));
            assert_eq!(chunk.status(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_property_write_changes_only_the_addressed_level() {
        use mod_api::{BlockUpdateFlags, LevelWriteError};
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        use sunrise_world::chunk::BlockState;

        if run_fresh_cases(
            "game::native_entity_refs::tests::block_property_write_changes_only_the_addressed_level",
            "SUNRISE_BLOCK_PROPERTY_ROUTE_CHILD",
            &["parked"],
        ) {
            return;
        }
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let registry = game.pack.assets.registry;
            let lamp = |lit: &str| {
                registry
                    .resolve(&BlockState {
                        name: "minecraft:redstone_lamp".into(),
                        properties: [("lit".into(), lit.into())].into(),
                    })
                    .unwrap()
            };
            let unlit = lamp("false");
            let lit = lamp("true");
            let [x, y, z] = [2, 65, 3];
            let pos = [x, y, z];
            let flags = BlockUpdateFlags::CLIENTS | BlockUpdateFlags::KNOWN_SHAPE;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.set(x, y, z, unlit).unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            assert_eq!(
                level.set_block_property(game, pos, "lit", "false", flags),
                Ok(false)
            );

            let nether = super::super::Dim::Nether;
            game.enter_command_level(nether).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.set(x, y, z, unlit).unwrap();

            game.storage_scope(|game, _| {
                assert_eq!(
                    level.set_block_property(game, pos, "lit", "true", flags),
                    Err(LevelWriteError::InTransaction)
                );
                ((), false)
            });
            assert_eq!(
                game.native_level_world(level).unwrap().block(x, y, z),
                unlit
            );

            assert_eq!(
                level.set_block_property(game, pos, "lit", "true", flags),
                Ok(true)
            );
            assert_eq!(game.dim, nether);
            assert_eq!(game.world.block(x, y, z), unlit);
            assert_eq!(game.native_level_world(level).unwrap().block(x, y, z), lit);
            assert_eq!(
                level.set_block_property(game, pos, "lit", "true", flags),
                Ok(false)
            );
            assert_eq!(game.dim, nether);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_reads_and_writes_use_dimension_build_height() {
        use mod_api::BlockUpdateFlags;
        use std::sync::Arc;
        use sunrise_client::chunk::{Chunk, LevelShape};
        with_game(|game| {
            game.world.shape = LevelShape::new(0, 128, 128, true, false).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 127, 0, stone).unwrap();
            game.world.set(0, 128, 0, stone).unwrap();
            assert!(game.level_is_loaded(level, [0, 127, 0]));
            assert_eq!(game.level_block_state(level, [0, 127, 0]), Some(stone));
            assert_eq!(
                game.level_block(level, [0, 127, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:stone")
            );
            assert!(game.native_level_write_allowed(level, [0, 127, 0]));
            assert!(!game.level_is_loaded(level, [0, 128, 0]));
            assert_eq!(game.level_block_state(level, [0, 128, 0]), None);
            assert_eq!(game.level_block(level, [0, 128, 0]), None);
            assert!(!game.level_is_loaded(level, [0, -1, 0]));
            assert_eq!(game.level_block_state(level, [0, -1, 0]), None);
            assert_eq!(
                game.level_set_block(level, [0, 128, 0], stone, BlockUpdateFlags::NEIGHBORS),
                Ok(false)
            );
            assert!(
                game.level_destroy_block(level, [0, 128, 0], false, None, None, false)
                    .is_none()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn equipping_mobs_by_hand_keeps_one_complete_stack() {
        use sunrise_client::{items::ItemStack, mobs::Mob};
        use sunrise_core::geometry::Vec3;
        with_game(|game| {
            game.creative = false;
            for (id, kind, item) in [
                (12, "minecraft:horse", "minecraft:iron_horse_armor"),
                (13, "minecraft:llama", "minecraft:red_carpet"),
                (14, "minecraft:happy_ghast", "minecraft:red_harness"),
                (15, "minecraft:nautilus", "minecraft:iron_nautilus_armor"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mob = Mob::new(id, kind, Vec3::ZERO, 0.0);
                mob.extra.tame = true;
                let mut held = ItemStack::new(item, 3);
                held.damage = 7;
                held.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("kept".into()));
                game.inventory[game.selected] = Some(held.clone());
                let index = game.mobs.len();
                game.mobs.push(mob);
                let used = match id {
                    12 => game.interact_mount(index, item),
                    13 => game.interact_llama(index, item),
                    14 => game.interact_happy_ghast(index, item),
                    _ => game.interact_nautilus(index, item),
                };
                assert!(used, "{item}");
                assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 2);
                held.count = 1;
                assert_eq!(game.mobs[index].equipment_stacks()[6], Some(held), "{item}");
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn sulfur_cube_absorbs_one_complete_stack_by_hand_and_pickup() {
        use sunrise_client::{items::ItemStack, mobs::Mob};
        use sunrise_core::geometry::Vec3;
        with_game(|game| {
            game.creative = false;
            let kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            let item = kind.tempt.first().expect("a swallowable item").to_string();
            let mut held = ItemStack::new(&item, 3);
            held.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("kept".into()));
            game.inventory[game.selected] = Some(held.clone());
            game.mobs.push(Mob::new(12, kind.clone(), Vec3::ZERO, 0.0));
            assert!(game.interact_sulfur_cube(0, &item));
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 2);
            held.count = 1;
            assert_eq!(game.mobs[0].equipment_stacks()[6], Some(held.clone()));
            game.mobs.clear();
            game.mobs.push(Mob::new(13, kind, Vec3::ZERO, 0.0));
            game.creatures.ticked.insert(13);
            game.spawn_item(
                ItemStack {
                    count: 3,
                    ..held.clone()
                },
                [0.0, 0.0, 0.0],
                [0.0; 3],
                0,
            );
            game.sulfur_cube_pickups();
            assert_eq!(game.mobs[0].equipment_stacks()[6], Some(held));
            assert_eq!(game.dropped[0].stack.count, 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn shearing_llama_and_happy_ghast_returns_full_body_stacks() {
        use sunrise_client::{items::ItemStack, mobs::Mob};
        use sunrise_core::geometry::Vec3;
        with_game(|game| {
            for (id, kind, item) in [
                (12, "minecraft:llama", "minecraft:red_carpet"),
                (13, "minecraft:happy_ghast", "minecraft:red_harness"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mob = Mob::new(id, kind, Vec3::ZERO, 0.0);
                mob.extra.tame = true;
                let mut stack = ItemStack::new(item, 3);
                stack.damage = 7;
                stack.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("kept".into()));
                mob.extra.set_body_stack(Some(stack.clone()));
                let index = game.mobs.len();
                game.mobs.push(mob);
                game.inventory[game.selected] = Some(ItemStack::new("minecraft:shears", 1));
                let used = if item.ends_with("carpet") {
                    game.interact_llama(index, "minecraft:shears")
                } else {
                    game.interact_happy_ghast(index, "minecraft:shears")
                };
                assert!(used);
                assert!(game.dropped.iter().any(|drop| drop.stack == stack));
                assert!(game.mobs[index].extra.body_id().is_none());
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn acting_guest_handles_resolve_during_guest_turns() {
        use super::super::lan::LanSettings;
        use mod_api::{EquipmentSlot, GameMode};
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_client::{
            items::ItemStack,
            lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
        };
        use sunrise_protocol::packets::configuration::ClientInformation;

        with_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes.clone(),
                level_seeds: game.levels.runtime.seeds(),
            };
            game.lan_host(settings, net, config).unwrap();
            let host = game.lan.as_mut().unwrap();
            for id in [12, 13] {
                let (tx, _rx) = std::sync::mpsc::channel();
                let mut guest = Guest::new(
                    Joined {
                        conn: id,
                        uuid: id as u128,
                        name: format!("guest{id}"),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: String::new(),
                        sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                        latency: Arc::new(AtomicI32::new(0)),
                        address: "127.0.0.1:1".parse().unwrap(),
                        expected: false,
                    },
                    id as i32,
                    [3.0, 64.0, 5.0],
                    10,
                );
                guest.placed = true;
                if id == 12 {
                    // A player loaded from a file saved with a walking speed.
                    guest.abilities.walking_speed = 0.25;
                    guest.loaded_body = Some(sunrise_client::lan::guest::SavedBody {
                        fire_ticks: 0,
                        air: 300,
                        fall_distance: 0.0,
                        velocity: [0.0; 3],
                        food_exhaustion: 0.0,
                        food_tick_timer: 0,
                        ticks_frozen: 0,
                        invulnerable: false,
                        invulnerable_time: 0,
                    });
                }
                guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 1));
                host.hub.guests.push(guest);
            }
            let motion = sunrise_client::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(3.0, 64.0, 5.0),
            )
            .unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.motion = Some(Box::new(motion)))
                .unwrap();
            let player = game.player(12).unwrap();
            player
                .with_inventory(game, |view| {
                    let mut slots = view.iter();
                    let (slot, stack) = slots.next().unwrap();
                    let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
                    assert_eq!((slot, stack.item(), stack.count()), (0, stone, 1));
                    assert!(slots.next().is_none());
                })
                .unwrap();
            let guest_living = game.living(player.fields().entity).unwrap();
            let host: &dyn mod_api::Host = game;
            assert_eq!(
                guest_living.attribute_base(host, "minecraft:max_health"),
                Some(20.0)
            );
            assert_eq!(
                guest_living.attribute_value(host, "minecraft:max_health"),
                Some(20.0)
            );
            assert_eq!(guest_living.max_health(host), Some(20.0));
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.motion = None)
                .unwrap();
            // Body and saddle are stored on a guest like on a mob.
            let living = game.living(player.fields().entity).unwrap();
            let saddle = ItemStack::new("minecraft:saddle", 1);
            assert!(game.living_set_equipment(living, EquipmentSlot::Saddle, Some(saddle.clone())));
            assert!(game.living_set_equipment(
                living,
                EquipmentSlot::Body,
                Some(ItemStack::new("minecraft:iron_horse_armor", 1))
            ));
            assert_eq!(
                game.living_equipment_stack(living, EquipmentSlot::Saddle)
                    .map(|stack| stack.project())
                    .as_deref(),
                Some(&saddle)
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0]
                    .equipment
                    .get(sunrise_items::equipment::BODY)
                    .map(|s| &*s.item),
                Some("minecraft:iron_horse_armor")
            );
            assert!(game.living_set_equipment(living, EquipmentSlot::Saddle, None));
            assert!(
                game.living_equipment_stack(living, EquipmentSlot::Saddle)
                    .is_none()
            );
            // The host's own previous mode, which no guest's change may touch.
            game.previous_mode = Some(sunrise_client::command::args::GameMode::Adventure);
            assert_eq!(game.player_previous_game_mode(player), Some(None));
            assert!(!game.player_set_game_mode(player, GameMode::Survival));
            assert_eq!(game.player_previous_game_mode(player), Some(None));
            assert!(game.player_set_game_mode(player, GameMode::Creative));
            assert_eq!(game.player_game_mode(player), Some(GameMode::Creative));
            assert_eq!(
                game.player_previous_game_mode(player),
                Some(Some(GameMode::Survival))
            );
            let abilities = game.player_abilities(player).unwrap();
            assert!(abilities.invulnerable && abilities.may_fly && abilities.instabuild);
            assert!(game.player_set_game_mode(player, GameMode::Survival));
            assert_eq!(game.player_game_mode(player), Some(GameMode::Survival));
            let abilities = game.player_abilities(player).unwrap();
            assert!(!abilities.invulnerable && !abilities.may_fly && !abilities.instabuild);
            // The Abilities setters store both floats as supplied; a mode change only
            // sets the flags again.
            let mut written = abilities;
            written.may_build = false;
            written.walk_speed = 0.2;
            written.fly_speed = 0.1;
            assert!(game.player_set_abilities(player, written));
            assert_eq!(game.player_abilities(player), Some(written));
            assert_eq!(game.player_may_build(player), Some(false));
            written.walk_speed = f32::NAN;
            written.fly_speed = f32::INFINITY;
            assert!(game.player_set_abilities(player, written));
            let stored = game.player_abilities(player).unwrap();
            assert!(stored.walk_speed.is_nan());
            assert_eq!(stored.fly_speed, f32::INFINITY);
            assert!(game.player_set_game_mode(player, GameMode::Creative));
            let abilities = game.player_abilities(player).unwrap();
            assert!(abilities.may_build && abilities.instabuild && abilities.may_fly);
            assert!(abilities.walk_speed.is_nan());
            assert_eq!(abilities.fly_speed, f32::INFINITY);
            assert!(game.player_set_game_mode(player, GameMode::Survival));
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(mod_api::WorldEpoch::new(0), player.fields().entity.id()),
                player.uuid(),
            );
            assert!(!game.player_set_game_mode(stale, GameMode::Creative));
            let other = game
                .native_lifecycle
                .epoch()
                .map(|epoch| mod_api::EntityRef::new(epoch, 13))
                .unwrap();
            game.as_guest(12, |game| {
                let player = game.player(12).unwrap();
                let entity = game.entity(12).unwrap();
                assert_eq!(player.fields().entity, entity);
                assert_eq!(entity.position(game), Some([3.0, 64.0, 5.0]));
                assert_eq!(entity.uuid(game), Some(12));
                let living = game.living(entity).unwrap();
                assert_eq!(
                    living
                        .with_equipment(game, EquipmentSlot::MainHand, |stack| stack
                            .item()
                            .id(game))
                        .unwrap(),
                    Some("minecraft:stone")
                );
                assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
                // The loaded walking speed is the movement speed's base.
                assert_eq!(
                    game.player
                        .as_mut()
                        .unwrap()
                        .attribute("minecraft:movement_speed")
                        .unwrap(),
                    f64::from(0.25f32)
                );
                // In the player's own turn the write reaches the body as well.
                let mut a = game.player_abilities(player).unwrap();
                a.may_fly = true;
                a.walk_speed = 0.3;
                assert!(game.player_set_abilities(player, a));
                assert_eq!(game.player_abilities(player), Some(a));
                assert!(game.player.as_ref().unwrap().may_fly);
                assert_eq!(other.position(game), Some([3.0, 64.0, 5.0]));
                assert!(game.living(other).is_some());
            })
            .unwrap();
            // A mode change in a turn, then a write in the same turn: the write wins.
            game.as_guest(12, |game| {
                let player = game.player(12).unwrap();
                // The guest's own previous mode is swapped in for the turn.
                assert_eq!(
                    game.player_previous_game_mode(player),
                    Some(Some(GameMode::Creative))
                );
                assert_eq!(game.player_is_secondary_use_active(player), Some(false));
                game.player.as_mut().unwrap().shift_key_down = true;
                assert_eq!(game.player_is_secondary_use_active(player), Some(true));
                game.player.as_mut().unwrap().shift_key_down = false;
                assert!(game.player_set_game_mode(player, GameMode::Creative));
                assert_eq!(
                    game.player_previous_game_mode(player),
                    Some(Some(GameMode::Survival))
                );
                let mut a = game.player_abilities(player).unwrap();
                assert!(a.instabuild && a.may_fly);
                a.fly_speed = 0.4;
                a.walk_speed = 0.3;
                assert!(game.player_set_abilities(player, a));
            })
            .unwrap();
            let kept = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
            assert_eq!(kept.game_mode, 1);
            assert_eq!(kept.previous_game_mode, Some(0));
            assert_eq!(
                game.previous_mode,
                Some(sunrise_client::command::args::GameMode::Adventure)
            );
            assert!(kept.abilities.instabuild);
            assert_eq!(kept.abilities.flying_speed, 0.4);
            assert_eq!(kept.abilities.walking_speed, 0.3);
            // The turn's write is stored in the guest's record.
            let stored = game.lan.as_ref().unwrap().hub.guest(12).unwrap().abilities;
            assert!(stored.may_fly && stored.walking_speed == 0.3);
            game.with_guest_swap(12, game.dim, |game| {
                assert!(game.entity(12).is_some());
                let other = game.entity(13).unwrap();
                assert_eq!(other.position(game), Some([3.0, 64.0, 5.0]));
                assert_eq!(other.uuid(game), Some(13));
                assert_eq!(
                    other.kind(game).and_then(|kind| kind.id(game)).as_deref(),
                    Some("minecraft:player")
                );
                let living = game.living(other).unwrap();
                assert_eq!(
                    living
                        .with_equipment(game, EquipmentSlot::MainHand, |stack| stack
                            .item()
                            .id(game))
                        .unwrap(),
                    Some("minecraft:stone")
                );
                let player = game.player(13).unwrap();
                assert_eq!(player.fields().entity, other);
                assert_eq!(player.game_mode(game), Some(GameMode::Survival));
                assert!(game.player_set_game_mode(player, GameMode::Creative));
                assert_eq!(player.game_mode(game), Some(GameMode::Creative));
                assert_eq!(game.lan_acting, Some(12));
                let acting = game.player(12).unwrap();
                assert!(game.player_set_game_mode(acting, GameMode::Adventure));
                assert_eq!(game.player_game_mode(acting), Some(GameMode::Adventure));
                game.with_guest_swap(13, game.dim, |game| {
                    assert!(
                        game.with_guest_swap(12, game.dim, |_| panic!("active guest re-entered"))
                            .is_none()
                    );
                    assert_eq!(game.lan_acting, Some(13));
                    assert!(game.entity(12).is_none());
                    assert!(game.player(12).is_none());
                    assert!(
                        game.living(mod_api::EntityRef::new(other.epoch(), 12))
                            .is_none()
                    );
                    assert!(game.entity(13).is_some());
                })
                .unwrap();
                assert_eq!(game.lan_acting, Some(12));
            })
            .unwrap();
            assert!(game.lan_guest_swaps.is_empty());
            // A player saved in creative (game mode 1, mayfly and instabuild)
            // joins again: without force they stay creative; forced into the
            // default mode (survival) they get its abilities.
            let root = std::env::temp_dir().join(format!("abilities-force-{}", std::process::id()));
            game.world_root = root.clone();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |g| {
                    g.game_mode = 1;
                    g.abilities.update_for_mode(1);
                    assert!(g.abilities.may_fly && g.abilities.instabuild);
                    sunrise_client::lan::playerdata::save(g, &root, &Default::default());
                })
                .unwrap();
            let joined = |game: &mut super::super::Game<'_>, force| {
                let host = game.lan.as_mut().unwrap();
                host.hub.config.default_game_mode = 0;
                host.hub.config.force_game_mode = force;
                host.hub
                    .update_guest(13, |g| {
                        g.game_mode = 0;
                        g.abilities = Default::default();
                    })
                    .unwrap();
                game.lan_guest_joined(13, &[], [0.0, 64.0, 0.0], None);
                let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
                (
                    g.game_mode,
                    g.abilities.may_fly,
                    g.abilities.instabuild,
                    g.abilities.may_build,
                )
            };
            assert_eq!(joined(game, false), (1, true, true, true));
            assert_eq!(joined(game, true), (0, false, false, true));
            // A remote guest can mount through the same handle path as the acting player.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(sunrise_client::mobs::Mob::new(
                21,
                kind,
                sunrise_core::geometry::Vec3::ZERO,
                0.0,
            ));
            let guest = game.entity(12).unwrap();
            let vehicle = game.entity(21).unwrap();
            assert!(game.entity_start_riding(guest, vehicle, true).is_ok());
            assert_eq!(game.command_guest(12).unwrap().vehicle, Some(21));
            assert!(game.entity_stop_riding(guest));
            assert_eq!(game.command_guest(12).unwrap().vehicle, None);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.dimension = "minecraft:the_nether".into())
                .unwrap();
            assert!(game.entity_start_riding(guest, vehicle, true).is_err());
            assert!(game.entity_start_riding(vehicle, guest, true).is_err());
            let _ = std::fs::remove_dir_all(root);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn entities_in_returns_readable_handles_and_intersects_boxes() {
        use mod_api::{LevelRef, WorldEpoch};
        use sunrise_client::entities::ArmorStand;
        use sunrise_core::geometry::Vec3;
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            game.armor_stands
                .push(ArmorStand::new(0, Vec3::new(0.5, 0.0, 0.0), 0.0));
            assert!(game.entity(0).is_none());
            game.armor_stands
                .push(ArmorStand::new(12, Vec3::new(1.0, 0.0, 0.0), 0.0));
            let found =
                collect_game_entities_in(game, level, [0.0, 0.0, -1.0], [0.8, 1.0, 1.0]).unwrap();
            assert_eq!(found.len(), 1);
            assert_eq!(game.entity_position(found[0]), Some([1.0, 0.0, 0.0]));
            assert_eq!(
                game.entity_kind(found[0])
                    .and_then(|kind| kind.id(game))
                    .as_deref(),
                Some("minecraft:armor_stand")
            );
            assert!(
                collect_game_entities_in(game, level, [0.0, 0.0, -1.0], [0.75, 1.0, 1.0])
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                collect_game_entities_in(
                    game,
                    LevelRef::new(WorldEpoch::new(0), game.dim.key()),
                    [0.0; 3],
                    [2.0; 3]
                ),
                Err(LevelReadError::Stale)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn connectionless_guest_has_no_transport() {
        use mod_api::HostTransaction;
        use std::sync::{Arc, atomic::Ordering};
        use sunrise_client::{
            lan::{
                Joined, Lan, NetConfig, PlayerSender,
                chunks::ChunkEncoder,
                conn::{Holding, flush_all},
                data::SyncData,
                guest::Guest,
                hub::{Hub, HubConfig},
            },
            motion::PlayerMotion,
        };
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::{
            configuration::ClientInformation,
            play::{Clientbound, Distance},
        };
        with_game(|game| {
            let sync = Arc::new(SyncData::empty());
            let net = NetConfig {
                announce: false,
                ..NetConfig::lan(0, "test".into(), 2)
            };
            let lan = Lan::start_with(sync.clone(), net).unwrap();
            let real_id = lan.shared.next_id().unwrap();
            let fake_id = lan.shared.next_id().unwrap();
            assert_ne!(real_id, fake_id);
            let encoder = ChunkEncoder::new(
                [] as [(Arc<str>, bool); 0],
                &sunrise_protocol::registry::Builtin::default(),
                &[],
                &[],
                Default::default(),
            );
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 10,
                simulation_distance: 10,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 20,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            };
            let mut hub = Hub::new(
                lan,
                sync,
                encoder,
                config.clone(),
                std::sync::Arc::new(
                    crate::game::test_server_load()
                        .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
                ),
            );
            let motion = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            let fake = Guest::connectionless(fake_id, 2, "fake".into(), 2, [0.0; 3], motion);
            assert!(fake.placed && fake.motion.is_some());
            assert!(fake.loaded && fake.client_loaded());
            assert!(!fake.participates_online() && !fake.participates_in_player_simulation());
            assert_eq!((fake.game_mode, fake.permission, fake.input), (0, 0, 0));
            assert!(fake.vehicle.is_none() && fake.bed.is_none() && fake.menu.is_none());
            let (tx, rx) = std::sync::mpsc::channel();
            let mut real = Guest::new(
                Joined {
                    conn: real_id,
                    uuid: 1,
                    name: "real".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::default(),
                    address: ([127, 0, 0, 1], 1).into(),
                    expected: false,
                },
                1,
                [0.0; 3],
                10,
            );
            real.placed = true;
            hub.guests.push(real);
            hub.guests.push(fake);
            assert!(hub.here(&hub.guests[0]));
            assert!(!hub.here(&hub.guests[1]));
            let packet = Clientbound::SetSimulationDistance(Distance { distance: 8 });
            let hold = Holding::start();
            hub.send_to(fake_id, &packet);
            hub.broadcast(&packet);
            hub.guests[1].sender.send_many([vec![1, 2, 3]]);
            flush_all(hub.guests.iter().map(|g| &g.sender));
            drop(hold);
            assert_eq!(hub.guests[1].sender.pending.load(Ordering::Relaxed), 0);
            assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).is_ok());
            game.lan_host(
                super::super::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::lan(0, "swap".into(), 2)
                },
                config,
            )
            .unwrap();
            let host = game.lan.as_mut().unwrap();
            host.hub.guests = std::mem::take(&mut hub.guests);
            {
                let value = [1600.0, 64.0, 0.0];
                host.hub.guests.update(1, |guest| guest.position = value);
            }
            game.lan_update_streaming();
            assert!(
                game.streaming
                    .extra
                    .iter()
                    .any(|(chunk, _)| *chunk == (0, 0))
            );
            assert!(
                !game
                    .streaming
                    .extra
                    .iter()
                    .any(|(chunk, _)| *chunk == (100, 0))
            );
            game.next_entity = 100;
            game.lan.as_ref().unwrap().hub.lan.shared.next_id();
            game.lan.as_ref().unwrap().hub.lan.shared.next_id();
            let level = game.level(game.dim.id()).unwrap();
            let role = game.options.execution_role;
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            assert!(game.fake_player(level, 1234, "machine").is_none());
            game.options.execution_role = role;
            let made = game.fake_player(level, 1234, "machine").unwrap();
            assert_eq!(game.fake_player(level, 1234, "machine"), Some(made));
            assert!(game.fake_player(level, 1234, "other").is_none());
            assert!(game.fake_player(level, 1, "real").is_none());
            assert_eq!(game.player_is_fake(made), Some(true));
            assert!(game.player(1234).is_none());
            let mut abilities = game.player_abilities(made).unwrap();
            abilities.may_fly = true;
            assert!(game.player_set_abilities(made, abilities));
            assert!(game.player_abilities(made).unwrap().may_fly);
            assert!(game.player_set_food_level(made, 10));
            assert!(game.player_feed(made, 3, 0.0));
            assert_eq!(game.player_food(made).unwrap().level, 13);
            assert!(game.player_set_experience_levels(made, 7));
            assert_eq!(game.player_experience(made).unwrap().level, 7);
            assert!(game.player_set_item_cooldown(made, "minecraft:stone", 20));
            assert_eq!(
                game.player_has_item_cooldown(made, "minecraft:stone"),
                Some(true)
            );
            assert!(game.player_remove_fake(made));
            assert!(!game.player_remove_fake(made));
            assert_eq!(game.player_is_fake(made), None);
            let replacement = game.fake_player(level, 1234, "machine").unwrap();
            assert_ne!(made, replacement);
            let saved_root = game.world_root.clone();
            let scratch = std::env::current_dir()
                .unwrap()
                .join("target/fake-player-save-test");
            let _ = std::fs::remove_dir_all(&scratch);
            game.world_root = scratch.clone();
            let host = game.lan.as_mut().unwrap();
            let i = host.hub.guests.iter().position(|g| g.uuid == 1234).unwrap();
            let mut actor = host.hub.guests.remove(i);
            game.save_guest(&mut actor);
            game.lan.as_mut().unwrap().hub.guests.insert(i, actor);
            assert!(!scratch.exists());
            game.world_root = saved_root;
            let conn = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .find(|g| g.uuid == 1234)
                .unwrap()
                .conn;
            game.as_guest(conn, |game| {
                assert_eq!(game.player_permission_level(replacement), Some(0));
                assert!(!game.player_remove_fake(replacement));
                assert!(game.fake_player(level, 4321, "other").is_none());
            })
            .unwrap();
            assert!(game.player_remove_fake(replacement));
            let overworld_actor = game.fake_player(level, 2345, "parked").unwrap();
            let attachment = sunrise_mod_runtime::attachments::AttachmentType {
                namespace: "test",
                name: "identity",
                default: || 0i32,
                codec: None,
                copy_on_death: false,
                sync_codec: None,
            };
            let holder = sunrise_mod_runtime::attachments::Holder::Player(2345);
            game.options.dedicated = Some(Vec::new());
            let real_holder = sunrise_mod_runtime::attachments::Holder::Player(1);
            attachment
                .set(
                    game.attachment_map_quiet(real_holder, mod_api::AttachmentAccess::Existing)
                        .unwrap(),
                    sunrise_mod_runtime::attachments::Holder::Level,
                    17,
                )
                .unwrap();
            attachment
                .set(
                    game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                        .unwrap(),
                    sunrise_mod_runtime::attachments::Holder::Level,
                    3,
                )
                .unwrap();
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            let nether = game.level("minecraft:the_nether").unwrap();
            assert!(game.fake_player(level, 3456, "other").is_none());
            let nether_actor = game.fake_player(nether, 2345, "parked").unwrap();
            attachment
                .set(
                    game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                        .unwrap(),
                    sunrise_mod_runtime::attachments::Holder::Level,
                    8,
                )
                .unwrap();
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        sunrise_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                8
            );
            assert_ne!(overworld_actor, nether_actor);
            let host: &mut dyn mod_api::ServerHost = game;
            let aborted: Result<(), mod_api::TransactionError<()>> = host.transaction(|host, _| {
                attachment.set(host, holder, 99).unwrap();
                Err(())
            });
            assert!(aborted.is_err());
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        sunrise_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                8
            );
            let place = mod_api::ItemPlace::PlayerSlot {
                player: 2345,
                slot: 0,
            };
            let sticks = mod_api::ItemResource::of(&crate::game::api_values::test_snapshot(
                &sunrise_client::items::ItemStack::new("minecraft:stick", 1),
            ));
            let host: &mut dyn mod_api::ServerHost = game;
            let rolled_back: Result<(), mod_api::TransactionError<()>> =
                host.transaction(|host, tx| {
                    assert_eq!(place.insert(host, tx, &sticks, 1), Ok(1));
                    Err(())
                });
            assert!(rolled_back.is_err());
            assert!(place.with_stack(game, |_| ()).is_none());
            assert_eq!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id as u32 == overworld_actor.entity().id())
                    .unwrap()
                    .inventory[0],
                None
            );
            let host: &mut dyn mod_api::ServerHost = game;
            assert_eq!(
                host.transaction(|host, tx| place.insert(host, tx, &sticks, 1)),
                Ok(1)
            );
            assert_eq!(place.with_stack(game, |stack| stack.count()).unwrap(), 1);
            assert_eq!(game.fake_player(nether, 2345, "parked"), Some(nether_actor));
            assert!(game.player_set_experience_levels(nether_actor, 8));
            assert!(game.player_set_experience_levels(overworld_actor, 3));
            assert_eq!(game.player_experience(nether_actor).unwrap().level, 8);
            assert_eq!(game.player_experience(overworld_actor).unwrap().level, 3);
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        sunrise_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                3
            );
            assert!(game.player_remove_fake(overworld_actor));
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(real_holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        sunrise_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                17
            );
            game.enter_command_level(super::super::Dim::Nether).unwrap();
            assert_eq!(
                attachment
                    .get(
                        game.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
                            .unwrap(),
                        sunrise_mod_runtime::attachments::Holder::Level
                    )
                    .unwrap(),
                8
            );
            assert!(game.player_remove_fake(nether_actor));
            game.enter_command_level(super::super::Dim::Overworld)
                .unwrap();
            game.options.dedicated = None;
            let players = game.native_server_players();
            assert_eq!(players.len(), 1);
            assert_eq!(players[0].uuid(), 1);
            // A fake record in the Nether does not keep that level ticking.
            {
                let value = "minecraft:the_nether".into();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(1, |guest| guest.dimension = value);
            }
            let ticking = game.levels_to_tick();
            assert!(!ticking.contains(&super::super::Dim::Nether));
            {
                let value = "minecraft:the_nether".into();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            assert!(game.levels_to_tick().contains(&super::super::Dim::Nether));
            {
                let value = "minecraft:overworld".into();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            game.lan_acting = Some(real_id);
            let selector = sunrise_client::command::args::Selector {
                kind: 'a',
                limit: usize::MAX,
                ..Default::default()
            };
            assert_eq!(game.select(&selector).len(), 1);
            let named = sunrise_client::command::args::Selector {
                player_name: Some("fake".into()),
                ..Default::default()
            };
            assert!(game.select(&named).is_empty());
            assert!(game.run_command_quietly("list"));
            assert!(!game.run_command_quietly("tp fake 0 64 0"));
            game.lan_acting = None;
            for (conn, learns) in [(fake_id, false), (real_id, true)] {
                game.as_guest(conn, |game| {
                    let mut book =
                        sunrise_client::items::ItemStack::new("minecraft:knowledge_book", 1);
                    book.data_mut().set_other(
                        "minecraft:recipes",
                        Some(sunrise_core::nbt::Tag::List(vec![
                            sunrise_core::nbt::Tag::String("minecraft:stick".into()),
                        ])),
                    );
                    let slot = game.held_slot();
                    *game.stack_in_mut(slot) = Some(book.clone());
                    assert!(game.use_knowledge_book(&book));
                    assert!(game.stack_in(slot).is_none());
                    assert_eq!(game.level.recipe_book.knows("minecraft:stick"), learns);
                })
                .unwrap();
            }
            game.guest_packets
                .push(Clientbound::SetSimulationDistance(Distance { distance: 4 }));
            game.as_guest(fake_id, |game| {
                assert!(game.lan_acting_fake);
                assert!(game.player.as_ref().is_some_and(|p| p.client_loaded));
                assert!(game.player_view().is_none());
                let mut visited = 0;
                game.for_each_player(|_| visited += 1);
                assert_eq!(visited, 0);
                let before = game.stats.get("custom", "minecraft:jump");
                game.award_stat("custom", "minecraft:jump", 1);
                assert_eq!(game.stats.get("custom", "minecraft:jump"), before);
                game.trigger("minecraft:tick", |_, _| panic!("fake criterion evaluated"));
                assert_eq!(game.award_recipes(&["minecraft:test".into()]), 0);
                let source = sunrise_client::command::args::Selector {
                    kind: 's',
                    limit: 1,
                    ..Default::default()
                };
                assert_eq!(
                    game.select(&source),
                    vec![super::super::commands::Target::Player]
                );
                let named = sunrise_client::command::args::Selector {
                    player_name: Some("fake".into()),
                    ..Default::default()
                };
                assert!(game.select(&named).is_empty());
                let uuid = sunrise_client::command::args::Selector {
                    uuid: Some(2),
                    ..Default::default()
                };
                assert!(game.select(&uuid).is_empty());
                game.guest_packets
                    .push(Clientbound::SetSimulationDistance(Distance {
                        distance: 12,
                    }));
                game.guest_entity_events.push(3);
                game.guest_cooldowns.push(("minecraft:test".into(), 20));
            })
            .unwrap();
            assert_eq!(game.guest_packets.len(), 1);
            assert!(game.guest_entity_events.is_empty() && game.guest_cooldowns.is_empty());
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[1]
                    .sender
                    .pending
                    .load(Ordering::Relaxed),
                0
            );
            assert!(rx.try_recv().is_err());
            game.with_guest_swap(real_id, game.dim, |game| {
                assert!(!game.lan_acting_fake);
                game.guest_cooldowns.push(("minecraft:real".into(), 7));
                game.guest_player_sounds
                    .push(("outer".into(), [0.0; 3], 1.0, 1.0));
                game.as_guest(fake_id, |game| {
                    assert!(game.lan_acting_fake);
                    game.guest_cooldowns.push(("minecraft:fake".into(), 20));
                })
                .unwrap();
                assert!(!game.lan_acting_fake);
                assert_eq!(game.guest_cooldowns.len(), 1);
                assert_eq!(&*game.guest_cooldowns[0].0, "minecraft:real");
                assert_eq!(game.guest_player_sounds[0].0, "outer");
                game.guest_player_sounds.clear();
            })
            .unwrap();
            let out = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
            let sunrise_client::lan::Out::Packet(body) = out else {
                panic!("expected cooldown packet")
            };
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            let packet = Clientbound::decode(id, reader.rest()).unwrap();
            let Clientbound::Cooldown(cooldown) = packet else {
                panic!("expected cooldown")
            };
            assert_eq!(cooldown.group, "minecraft:real");
            assert_eq!(cooldown.duration, 7);
            assert!(rx.try_recv().is_err());

            // A real turn nested in a fake must not receive the fake's cooldown.
            game.with_guest_swap(fake_id, game.dim, |game| {
                assert!(game.lan_acting_fake);
                game.guest_cooldowns
                    .push(("minecraft:fake_outer".into(), 8));
                game.guest_player_sounds
                    .push(("fake_outer".into(), [0.0; 3], 1.0, 1.0));
                game.as_guest(real_id, |game| {
                    assert!(!game.lan_acting_fake);
                    let before = game.stats.get("custom", "minecraft:jump");
                    game.award_stat("custom", "minecraft:jump", 1);
                    assert_eq!(game.stats.get("custom", "minecraft:jump"), before + 1);
                    game.guest_cooldowns
                        .push(("minecraft:real_inner".into(), 9));
                })
                .unwrap();
                assert!(game.lan_acting_fake);
                assert_eq!(&*game.guest_cooldowns[0].0, "minecraft:fake_outer");
                assert_eq!(game.guest_player_sounds[0].0, "fake_outer");
                game.guest_player_sounds.clear();
            })
            .unwrap();
            let sunrise_client::lan::Out::Packet(body) =
                rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
            else {
                panic!("expected cooldown")
            };
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            let Clientbound::Cooldown(cooldown) = Clientbound::decode(id, reader.rest()).unwrap()
            else {
                panic!("expected cooldown")
            };
            assert_eq!(cooldown.group, "minecraft:real_inner");
            assert!(rx.try_recv().is_err());

            // Two real nested turns deliver each queue to its own connection.
            let (tx2, rx2) = std::sync::mpsc::channel();
            let mut second = Guest::new(
                Joined {
                    conn: 3,
                    uuid: 3,
                    name: "second".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx2, Arc::default()),
                    latency: Arc::default(),
                    address: ([127, 0, 0, 1], 3).into(),
                    expected: false,
                },
                3,
                [0.0; 3],
                10,
            );
            second.placed = true;
            game.lan.as_mut().unwrap().hub.guests.push(second);
            game.with_guest_swap(real_id, game.dim, |game| {
                game.guest_cooldowns
                    .push(("minecraft:real_outer".into(), 10));
                game.guest_player_sounds
                    .push(("real_outer".into(), [0.0; 3], 1.0, 1.0));
                game.as_guest(3, |game| {
                    assert_eq!(game.lan_acting, Some(3));
                    assert!(
                        game.as_guest(real_id, |_| panic!("active guest re-entered"))
                            .is_none()
                    );
                    assert_eq!(game.lan_acting, Some(3));
                    game.guest_cooldowns
                        .push(("minecraft:real_second".into(), 11))
                })
                .unwrap();
                assert_eq!(&*game.guest_cooldowns[0].0, "minecraft:real_outer");
                assert_eq!(game.guest_player_sounds[0].0, "real_outer");
                game.guest_player_sounds.clear();
            })
            .unwrap();
            for (receiver, expected) in [
                (&rx, "minecraft:real_outer"),
                (&rx2, "minecraft:real_second"),
            ] {
                let sunrise_client::lan::Out::Packet(body) = receiver
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap()
                else {
                    panic!("expected cooldown")
                };
                let mut reader = sunrise_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                let Clientbound::Cooldown(cooldown) =
                    Clientbound::decode(id, reader.rest()).unwrap()
                else {
                    panic!("expected cooldown")
                };
                assert_eq!(cooldown.group, expected);
                assert!(receiver.try_recv().is_err());
            }
            let fake = game.fake_player(level, 5678, "joiner").unwrap();
            game.enter_command_level(super::super::Dim::Nether).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            let other_fake = game.fake_player(nether, 5678, "joiner").unwrap();
            game.enter_command_level(super::super::Dim::Overworld)
                .unwrap();
            let saved_root = game.world_root.clone();
            let scratch = std::env::current_dir()
                .unwrap()
                .join("target/fake-login-save-test");
            let _ = std::fs::remove_dir_all(&scratch);
            game.world_root = scratch.clone();
            let shared = &game.lan.as_ref().unwrap().hub.lan.shared;
            let conn = shared.next_id().unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            shared
                .events
                .send(sunrise_client::lan::NetEvent::Joined(
                    Box::new(Joined {
                        conn,
                        uuid: 5678,
                        name: "joiner".into(),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: String::new(),
                        sender: PlayerSender::new(tx, Arc::default()),
                        latency: Arc::default(),
                        address: ([127, 0, 0, 1], 1).into(),
                        expected: false,
                    }),
                    None,
                ))
                .unwrap();
            use mod_api;
            struct Logout(std::sync::Arc<std::sync::atomic::AtomicUsize>);
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for Logout {
                fn player_logged_out(
                    &self,
                    _: &mut dyn mod_api::ServerHost,
                    _: &mut mod_api::PlayerLoggedOutEvent<'_>,
                ) {
                    self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let logouts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "logout-test",
                Box::new(Logout(logouts.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                game.native_player_logged_out(0, "control");
                assert_eq!(logouts.swap(0, std::sync::atomic::Ordering::Relaxed), 1);
                game.tick_lan_packets();
                assert_eq!(logouts.load(std::sync::atomic::Ordering::Relaxed), 0);
            });
            assert_eq!(game.player_is_fake(fake), None);
            assert_eq!(game.player_is_fake(other_fake), None);
            assert!(
                !sunrise_client::lan::playerdata::path(&scratch, 5678).exists(),
                "fake login replacement saved player data"
            );
            game.world_root = saved_root;
            let matching: Vec<_> = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .filter(|g| g.uuid == 5678)
                .collect();
            assert_eq!(matching.len(), 1);
            assert!(matching[0].participates_online());
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "logout-shutdown-test",
                Box::new(Logout(logouts.clone())),
            )]
            .into();
            let real_count = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .filter(|g| g.placed && g.participates_online())
                .count();
            mod_platform::testing::with_mods(mods, || game.native_players_leaving());
            assert_eq!(logouts.load(Ordering::Relaxed), real_count);
            game.lan.as_mut().unwrap().play.entry(fake_id).or_default();
            game.award_leave_game_to_all();
            let host = game.lan.as_mut().unwrap();
            assert!(!host.play.get(&fake_id).unwrap().left_game);
            assert!(host.play.get(&real_id).unwrap().left_game);
            host.hub.ticks = 2401;
            assert_eq!(game.ban_ip_target("fake"), None);
            assert_eq!(game.ban_ip_target("real").as_deref(), Some("127.0.0.1"));
            game.kick_idle(1);
            let host = game.lan.as_ref().unwrap();
            assert!(host.hub.guest(fake_id).unwrap().left_reason.is_empty());
            assert!(!host.hub.guest(real_id).unwrap().left_reason.is_empty());
            let old = game.fake_player(level, 6789, "reopen").unwrap();
            game.native_lifecycle_world_closed();
            game.native_lifecycle_open_started();
            let reopened = game.level(game.dim.id()).unwrap();
            assert_ne!(level.epoch(), reopened.epoch());
            assert_eq!(game.player_is_fake(old), None);
            let fresh = game.fake_player(reopened, 6789, "reopen").unwrap();
            assert_ne!(fresh, old);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_reads_whether_a_block_conducts_redstone() {
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            for (name, conducts) in [
                ("minecraft:stone", true),
                ("minecraft:glass", false),
                ("minecraft:stone_slab", false),
            ] {
                let state = game.pack.assets.registry.default_state(name).unwrap();
                game.world.set(0, 64, 0, state).unwrap();
                assert_eq!(
                    level.is_redstone_conductor(game, [0, 64, 0]),
                    Some(conducts),
                    "{name}"
                );
            }
            // An unloaded position is unknown, not a guess.
            assert_eq!(level.is_redstone_conductor(game, [160, 64, 0]), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_sturdy_uses_gameplay_support() {
        use mod_api::SupportType as Support;
        use mod_api::{BlockFace as Face, LevelRef, WorldEpoch};
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let slab = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone_slab")
                .unwrap();
            game.world.set(0, 64, 0, slab).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Down, Support::Full),
                Some(true)
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::North, Support::Full),
                Some(false)
            );
            let wall = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cobblestone_wall")
                .unwrap();
            game.world.set(0, 64, 0, wall).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Center),
                Some(true)
            );
            assert_eq!(
                level.block_face_sturdy(game, [16, 64, 0], Face::Down, Support::Full),
                None
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 10000, 0], Face::Down, Support::Full),
                None
            );
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Center),
                Some(true)
            );
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            let shulker = game
                .pack
                .assets
                .registry
                .default_state("minecraft:shulker_box")
                .unwrap();
            game.world.set(0, 64, 0, shulker).unwrap();
            use sunrise_client::block_entity_anim::{Shulker, ShulkerStatus};
            // The early guest packet pass changes opener status before entity
            // hooks run, and before tick_block_entities synchronizes animations.
            // Exercise that producer with both an absent and a closed snapshot.
            for had_snapshot in [false, true] {
                game.be_anims.shulkers.remove(&[0, 64, 0]);
                game.world.shulkers.remove(&[0, 64, 0]);
                if had_snapshot {
                    game.world.shulkers.insert([0, 64, 0], Shulker::default());
                }
                for support in [Support::Full, Support::Center, Support::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                        Some(true),
                    );
                }
                assert!(game.container_openers_changed((0, 64, 0), true));
                assert_eq!(
                    game.be_anims.shulkers[&[0, 64, 0]].status,
                    ShulkerStatus::Opening
                );
                for parked in [false, true] {
                    if parked {
                        assert!(
                            game.enter_command_level(super::super::Dim::Nether)
                                .is_some()
                        );
                    }
                    for support in [Support::Full, Support::Center, Support::Rigid] {
                        assert_eq!(
                            level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                            Some(false),
                        );
                        assert_eq!(
                            level.block_face_sturdy(game, [0, 64, 0], Face::Down, support),
                            Some(true),
                        );
                    }
                }
                assert!(
                    game.enter_command_level(super::super::Dim::Overworld)
                        .is_some()
                );
                assert!(game.container_openers_changed((0, 64, 0), false));
                assert_eq!(
                    game.world.shulkers[&[0, 64, 0]].status,
                    ShulkerStatus::Closing
                );
                for support in [Support::Full, Support::Center, Support::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                        Some(false),
                    );
                }
            }
            game.be_anims.shulkers.remove(&[0, 64, 0]);
            for status in [
                ShulkerStatus::Closed,
                ShulkerStatus::Opening,
                ShulkerStatus::Opened,
                ShulkerStatus::Closing,
            ] {
                game.world.shulkers.insert(
                    [0, 64, 0],
                    Shulker {
                        status,
                        ..Default::default()
                    },
                );
                for support in [Support::Full, Support::Center, Support::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Up, support),
                        Some(status == ShulkerStatus::Closed),
                    );
                    assert_eq!(
                        level.block_face_sturdy(game, [0, 64, 0], Face::Down, support),
                        Some(true),
                    );
                }
            }
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(false),
            );
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            game.world.shulkers.remove(&[0, 64, 0]);
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(true),
            );
            let moving = game
                .pack
                .assets
                .registry
                .default_state("minecraft:moving_piston")
                .unwrap();
            game.world.set(0, 64, 0, moving).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(false),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.moving_pistons.push((
                [0, 64, 0],
                sunrise_world::piston_shape::MovingPistonState {
                    moved_state: stone,
                    direction: sunrise_core::geometry::Direction::East,
                    extending: true,
                    source: false,
                    progress: 1.0,
                    previous_progress: 1.0,
                },
            ));
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(true),
            );
            game.world.moving_pistons[0].1.progress = 0.5;
            game.world.moving_pistons[0].1.previous_progress = 0.5;
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                Some(false),
            );
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(
                stale.block_face_sturdy(game, [0, 64, 0], Face::Down, Support::Full),
                None
            );
        });
    }

    /// The hit of a ray that found one; any other outcome fails the test.
    fn hit_of(outcome: Result<mod_api::ClipOutcome, mod_api::ClipError>) -> mod_api::ClipHit {
        match outcome {
            Ok(mod_api::ClipOutcome::Hit(hit)) => hit,
            other => panic!("expected a hit, got {other:?}"),
        }
    }

    #[test]
    #[ignore = "daily"]
    fn level_clip_reads_parked_world_and_stops_at_unloaded_columns() {
        use mod_api::{
            BlockFace as Face, ClipBlock as Block, ClipError, ClipFluid as Fluid, ClipOutcome,
            CollisionContext as Context,
        };
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 64, 0, stone).unwrap();
            let from = [0.5, 64.5, 0.5];
            let to = [2.5, 64.5, 0.5];
            for mode in [Block::Collider, Block::Outline] {
                let hit = hit_of(level.clip(game, from, to, mode, Fluid::None, Context::Empty));
                assert!(!hit.miss);
                assert_eq!(hit.block, [1, 64, 0]);
                assert_eq!(hit.face, Face::West);
                assert_eq!(hit.location, [1.0, 64.5, 0.5]);
                assert!(!hit.inside && !hit.world_border);
            }
            let chunks_before_gap = game.world.chunks.len();
            // Crossing into a missing column stops at its first cell: neither a miss nor air,
            // and nothing is loaded.
            assert_eq!(
                level.clip(
                    game,
                    [15.5, 64.5, 0.5],
                    [17.5, 64.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                    Context::Empty,
                ),
                Ok(ClipOutcome::Unloaded { at: [16, 64, 0] })
            );
            // A ray that starts inside a missing column stops at its first cell too.
            assert_eq!(
                level.clip(
                    game,
                    [17.5, 64.5, 0.5],
                    [15.5, 64.5, 0.5],
                    Block::Outline,
                    Fluid::None,
                    Context::Empty,
                ),
                Ok(ClipOutcome::Unloaded { at: [17, 64, 0] })
            );
            assert_eq!(
                level.line_of_sight(game, [15.5, 64.5, 0.5], [17.5, 64.5, 0.5]),
                None,
                "an unloaded column answers neither way"
            );
            // Cells out of build height are air whatever the column below holds, so a ray there
            // is a miss, even across a missing column.
            for (from, to, mode) in [
                ([0.5, 325.5, 0.5], [2.5, 325.5, 0.5], Block::Collider),
                ([15.5, 325.5, 0.5], [17.5, 325.5, 0.5], Block::Collider),
                ([0.5, -70.5, 0.5], [0.5, -66.5, 0.5], Block::Outline),
                ([15.5, -70.5, 0.5], [17.5, -70.5, 0.5], Block::Outline),
            ] {
                assert_eq!(
                    level.clip(game, from, to, mode, Fluid::None, Context::Empty),
                    Ok(ClipOutcome::Miss),
                    "{from:?} to {to:?}"
                );
            }
            assert!(!game.world.chunks.contains_key(&(1, 0)));
            assert_eq!(game.world.chunks.len(), chunks_before_gap);
            // A straight ray through 300 loaded columns has no length cap: it hits a block more
            // than 4096 cells away, and misses the row above it.
            for cx in 1..=300 {
                game.world
                    .chunks
                    .insert((cx, 0), Arc::new(Chunk::empty(cx, 0)));
            }
            game.world.set(4805, 64, 0, stone).unwrap();
            let chunks_with_far = game.world.chunks.len();
            for mode in [Block::Collider, Block::Outline] {
                let far_hit = hit_of(level.clip(
                    game,
                    [15.5, 64.5, 0.5],
                    [4810.5, 64.5, 0.5],
                    mode,
                    Fluid::None,
                    Context::Empty,
                ));
                assert!(!far_hit.miss);
                assert_eq!(far_hit.block, [4805, 64, 0]);
                assert_eq!(far_hit.face, Face::West);
                assert_eq!(far_hit.location, [4805.0, 64.5, 0.5]);
                assert!(!far_hit.inside && !far_hit.world_border);
            }
            // Independent opposite hit face and a miss beyond 4096.
            let east_hit = hit_of(level.clip(
                game,
                [4810.5, 64.5, 0.5],
                [4800.5, 64.5, 0.5],
                Block::Collider,
                Fluid::None,
                Context::Empty,
            ));
            assert_eq!(east_hit.block, [4805, 64, 0]);
            assert_eq!(east_hit.face, Face::East);
            assert_eq!(east_hit.location, [4806.0, 64.5, 0.5]);
            assert_eq!(
                level.clip(
                    game,
                    [15.5, 65.5, 0.5],
                    [4810.5, 65.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                    Context::Empty,
                ),
                Ok(ClipOutcome::Miss)
            );
            // `line_of_sight` delegates to the same segment clip with no 128-block entity cap.
            assert_eq!(
                level.line_of_sight(game, [15.5, 64.5, 0.5], [4810.5, 64.5, 0.5]),
                Some(false)
            );
            assert_eq!(
                level.line_of_sight(game, [15.5, 65.5, 0.5], [4810.5, 65.5, 0.5]),
                Some(true)
            );
            assert_eq!(game.world.chunks.len(), chunks_with_far);
            assert_eq!(
                level.clip(
                    game,
                    [0.5, 67.5, 0.5],
                    [2.5, 67.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                    Context::Empty,
                ),
                Ok(ClipOutcome::Miss)
            );
            let inside = hit_of(level.clip(
                game,
                [1.5, 64.5, 0.5],
                [2.5, 64.5, 0.5],
                Block::Collider,
                Fluid::None,
                Context::Empty,
            ));
            assert!(inside.inside && !inside.miss);
            for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                for (from, to) in [
                    ([bad, 64.0, 0.0], to),
                    (from, [0.0, bad, 0.0]),
                    ([0.0, 64.0, bad], [bad, bad, bad]),
                ] {
                    assert_eq!(
                        level.clip(game, from, to, Block::Collider, Fluid::None, Context::Empty),
                        Err(ClipError::NonFiniteRay)
                    );
                }
            }
            let stale = mod_api::LevelRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                level.dimension(),
            );
            assert_eq!(
                stale.clip(game, from, to, Block::Collider, Fluid::None, Context::Empty),
                Err(ClipError::StaleLevel)
            );
            use sunrise_world::world_ray::RayWorld;
            let mut ordinary = super::super::explosion::ColliderRays {
                scene: sunrise_client::collisions::SceneCollisions {
                    world: &game.world,
                    binding: game.collision,
                    subject: None,
                },
                context: sunrise_world::collision_context::CollisionContext::empty(false),
                clip_data: None,
                report_unloaded: false,
            };
            assert!(
                ordinary
                    .interaction_shape(
                        sunrise_world::registry::BlockStateId(u32::MAX),
                        sunrise_core::position::BlockPos::new(0, 0, 0)
                    )
                    .unwrap()
                    .is_empty()
            );
            // Dynamic collision shapes (bamboo) work on the collider branch; outline reports
            // an actual unsupported error instead of fabricating a miss.
            let bamboo = game
                .pack
                .assets
                .registry
                .default_state("minecraft:bamboo")
                .unwrap();
            game.world.set(1, 64, 2, bamboo).unwrap();
            let bx = 1.3833333402872086;
            let bz = 2.3166666701436043;
            let bamboo_hit = hit_of(level.clip(
                game,
                [bx, 66.0, bz],
                [bx, 62.0, bz],
                Block::Collider,
                Fluid::None,
                Context::Empty,
            ));
            assert!(!bamboo_hit.miss);
            assert_eq!(bamboo_hit.block, [1, 64, 2]);
            assert_eq!(bamboo_hit.face, Face::Up);
            assert_eq!(bamboo_hit.location, [bx, 65.0, bz]);
            assert_eq!(
                level.clip(
                    game,
                    [bx, 66.0, bz],
                    [bx, 62.0, bz],
                    Block::Outline,
                    Fluid::None,
                    Context::Empty,
                ),
                Err(ClipError::StaleLevel)
            );
            let plant = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dandelion")
                .unwrap();
            game.world.set(4, 64, 4, plant).unwrap();
            let facts = game.res.facts().get(plant);
            assert!(facts.outline_offset);
            let offset = game.pack.context.tables.offset(plant, 4, 4).unwrap();
            assert_ne!(offset[0], 0.0);
            let b = facts.outline[0];
            let y = 64.0 + offset[1] + (b[1] + b[4]) / 2.0;
            let z = 4.0 + offset[2] + (b[2] + b[5]) / 2.0;
            let plant_hit = hit_of(level.clip(
                game,
                [3.0, y, z],
                [5.0, y, z],
                Block::Outline,
                Fluid::None,
                Context::Empty,
            ));
            assert_eq!(plant_hit.block, [4, 64, 4]);
            assert!((plant_hit.location[0] - (4.0 + offset[0] + b[0])).abs() < 1e-9);
            let cauldron = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cauldron")
                .unwrap();
            game.world.set(0, 64, 2, cauldron).unwrap();
            let bowl = hit_of(level.clip(
                game,
                [0.5, 65.27, 0.5],
                [0.5, 64.85, 3.5],
                Block::Outline,
                Fluid::None,
                Context::Empty,
            ));
            assert!(!bowl.miss);
            assert_eq!(bowl.block, [0, 64, 2]);
            assert_eq!(bowl.face, Face::Up);
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                hit_of(level.clip(game, from, to, Block::Collider, Fluid::None, Context::Empty))
                    .block,
                [1, 64, 0]
            );
            assert_eq!(
                hit_of(level.clip(
                    game,
                    [15.5, 64.5, 0.5],
                    [4810.5, 64.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                    Context::Empty,
                ))
                .block,
                [4805, 64, 0]
            );
        });
    }

    /// `CollisionContext.empty()` against `CollisionContext.of(entity)`: scaffolding is solid to
    /// a ray with no entity and to one an entity above it asks for, and open to one an entity
    /// below it asks for (`ScaffoldingBlock.getCollisionShape` with `isAbove`).
    #[test]
    #[ignore = "daily"]
    fn level_clip_answers_shapes_for_the_entity_in_its_context() {
        use mod_api::{
            ClipBlock as Block, ClipFluid as Fluid, ClipOutcome, CollisionContext as Context,
        };
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, mobs::Mob};
        use sunrise_core::geometry::Vec3;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let scaffolding = game
                .pack
                .assets
                .registry
                .default_state("minecraft:scaffolding")
                .unwrap();
            game.world.set(1, 64, 0, scaffolding).unwrap();
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie.clone(), Vec3::new(1.5, 60.0, 0.5), 0.0));
            game.mobs
                .push(Mob::new(13, zombie, Vec3::new(1.5, 70.0, 0.5), 0.0));
            let (below, above) = (game.entity(12).unwrap(), game.entity(13).unwrap());
            let (from, to) = ([1.5, 66.0, 0.5], [1.5, 64.5, 0.5]);
            let clip = |game: &super::Game<'_>, context| {
                level.clip(game, from, to, Block::Collider, Fluid::None, context)
            };
            let empty = hit_of(clip(game, Context::Empty));
            assert_eq!(empty.block, [1, 64, 0]);
            assert_eq!(empty.location, [1.5, 65.0, 0.5]);
            let from_above = hit_of(clip(game, Context::Entity(above)));
            assert_eq!(from_above.location, empty.location);
            assert_eq!(clip(game, Context::Entity(below)), Ok(ClipOutcome::Miss));
            let gone = mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                above.id(),
            );
            assert_eq!(
                clip(game, Context::Entity(gone)),
                Err(mod_api::ClipError::StaleLevel)
            );
        });
    }

    /// `ClipContext.Block.VISUAL` (`BlockState.getVisualShape`): a ray passes glass, iron
    /// bars and powder snow, which a collider ray stops on (powder snow has no empty-context
    /// collision either), and stops on soul sand, mud, a snow layer and a fence at their
    /// visual top, not their collision top.
    #[test]
    #[ignore = "daily"]
    fn level_clip_visual_passes_see_through_blocks_and_stops_on_visual_shapes() {
        use mod_api::{
            BlockFace as Face, ClipBlock as Block, ClipFluid as Fluid, ClipOutcome,
            CollisionContext as Context,
        };
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let place = |game: &mut super::Game<'_>, block: &str| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let state = game.pack.assets.registry.default_state(block).unwrap();
                game.world.set(1, 64, 0, state).unwrap();
            };
            let (from, to) = ([0.5, 64.5, 0.5], [2.5, 64.5, 0.5]);
            for (block, collider) in [
                ("minecraft:glass", Some([1.0, 64.5, 0.5])),
                ("minecraft:iron_bars", Some([1.4375, 64.5, 0.5])),
                ("minecraft:powder_snow", None),
            ] {
                place(game, block);
                assert_eq!(
                    level.clip(game, from, to, Block::Visual, Fluid::None, Context::Empty),
                    Ok(ClipOutcome::Miss),
                    "{block}"
                );
                let hit = level.clip(game, from, to, Block::Collider, Fluid::None, Context::Empty);
                let location = match hit {
                    Ok(ClipOutcome::Hit(hit)) => Some(hit.location),
                    Ok(ClipOutcome::Miss) => None,
                    other => panic!("{block}: {other:?}"),
                };
                assert_eq!(location, collider, "{block}");
            }
            let (from, to) = ([1.5, 66.0, 0.5], [1.5, 63.5, 0.5]);
            for (block, visual, collider) in [
                ("minecraft:soul_sand", 65.0, Some(64.875)),
                ("minecraft:mud", 65.0, Some(64.875)),
                ("minecraft:snow", 64.125, None),
                ("minecraft:oak_fence", 65.0, Some(65.5)),
            ] {
                place(game, block);
                let hit =
                    hit_of(level.clip(game, from, to, Block::Visual, Fluid::None, Context::Empty));
                assert!(!hit.miss, "{block}");
                assert_eq!(hit.block, [1, 64, 0], "{block}");
                assert_eq!(hit.face, Face::Up, "{block}");
                assert_eq!(hit.location, [1.5, visual, 0.5], "{block}");
                assert!(!hit.inside && !hit.world_border, "{block}");
                let hit = level.clip(game, from, to, Block::Collider, Fluid::None, Context::Empty);
                let top = match hit {
                    Ok(ClipOutcome::Hit(hit)) => Some(hit.location[1]),
                    Ok(ClipOutcome::Miss) => None,
                    other => panic!("{block}: {other:?}"),
                };
                assert_eq!(top, collider, "{block}");
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_clip_fluid_modes_shapes_and_solid_on_tie() {
        use mod_api::{
            BlockFace as Face, ClipBlock as Block, ClipFluid as Fluid, CollisionContext as Context,
        };
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        with_game(|game| {
            // Fluid clips read the tag tables once the world has built them (`native_tag_items`).
            let root = tempfile::tempdir().unwrap();
            game.session_lock =
                Some(sunrise_client::save::SessionLock::acquire(root.path()).unwrap());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world
                .chunks
                .insert((0, -1), Arc::new(Chunk::empty(0, -1)));
            let level = game.level(game.dim.id()).unwrap();
            let stone = game.block_state_id("minecraft:stone").unwrap();
            let water_src = game.block_state_id("minecraft:water[level=0]").unwrap();
            let water_flow3 = game.block_state_id("minecraft:water[level=5]").unwrap();
            let water_flow4 = game.block_state_id("minecraft:water[level=4]").unwrap();
            let lava_src = game.block_state_id("minecraft:lava[level=0]").unwrap();
            let lava_flow6 = game.block_state_id("minecraft:lava[level=2]").unwrap();
            let slab_bot = game
                .block_state_id("minecraft:oak_slab[type=bottom,waterlogged=true]")
                .unwrap();
            let slab_top = game
                .block_state_id("minecraft:oak_slab[type=top,waterlogged=true]")
                .unwrap();
            let h8 = f64::from(8.0_f32 / 9.0);
            let h6 = f64::from(6.0_f32 / 9.0);
            let h3 = f64::from(3.0_f32 / 9.0);

            // Matrix over water/lava (source vs flowing) and waterlogged slab with stone below.
            let cells = [
                (0, water_src, h8, [false, true, true, true]),
                (1, water_flow3, h3, [false, false, true, true]),
                (2, lava_src, h8, [false, true, true, false]),
                (3, lava_flow6, h6, [false, false, true, false]),
                (4, slab_bot, h8, [false, true, true, true]),
            ];
            for &(x, state, _, _) in &cells {
                game.world.set(x, 63, 0, stone).unwrap();
                game.world.set(x, 64, 0, state).unwrap();
            }
            // Flowing water under same fluid lifts its shape height to 1.0.
            game.world.set(5, 64, 0, water_flow4).unwrap();
            game.world.set(5, 65, 0, water_src).unwrap();
            // Solid-on-tie comparison: plain water vs waterlogged top slab.
            game.world.set(6, 64, 0, water_src).unwrap();
            game.world.set(7, 64, 0, slab_top).unwrap();

            let modes = [Fluid::None, Fluid::SourceOnly, Fluid::Any, Fluid::Water];
            for &(x, state, height, stops) in &cells {
                let cx = f64::from(x) + 0.5;
                for (mode, expect_fluid) in modes.into_iter().zip(stops) {
                    let hit = hit_of(level.clip(
                        game,
                        [cx, 65.5, 0.5],
                        [cx, 62.5, 0.5],
                        Block::Outline,
                        mode,
                        Context::Empty,
                    ));
                    assert!(!hit.miss, "x={x} mode={mode:?}");
                    if expect_fluid {
                        assert_eq!(hit.block, [x, 64, 0]);
                        assert_eq!(hit.face, Face::Up);
                        assert_eq!(hit.location[1], 64.0 + height);
                    } else if state == slab_bot {
                        assert_eq!(hit.block, [x, 64, 0]);
                        assert_eq!(hit.location[1], 64.5);
                    } else {
                        assert_eq!(hit.block, [x, 63, 0]);
                        assert_eq!(hit.location[1], 64.0);
                    }
                }
            }

            let deep = hit_of(level.clip(
                game,
                [5.5, 64.95, -0.5],
                [5.5, 64.95, 0.5],
                Block::Outline,
                Fluid::Any,
                Context::Empty,
            ));
            assert!(!deep.miss);
            assert_eq!((deep.block, deep.face), ([5, 64, 0], Face::North));

            let plain = hit_of(level.clip(
                game,
                [6.5, 64.0 + h8 + 0.5, -0.5],
                [6.5, 64.0 + h8 - 0.5, 0.5],
                Block::Outline,
                Fluid::Any,
                Context::Empty,
            ));
            assert_eq!((plain.block, plain.face), ([6, 64, 0], Face::Up));
            let tied = hit_of(level.clip(
                game,
                [7.5, 64.0 + h8 + 0.5, -0.5],
                [7.5, 64.0 + h8 - 0.5, 0.5],
                Block::Outline,
                Fluid::Any,
                Context::Empty,
            ));
            assert_eq!((tied.block, tied.face), ([7, 64, 0], Face::North));

            // Tagging a non-water-named fluid into minecraft:water makes ClipFluid::Water hit it.
            let mut tagged = game.items().clone();
            tagged
                .fluid_tags
                .entry("minecraft:water".into())
                .or_default()
                .insert("minecraft:lava".into());
            game.server_items = Some(Arc::new(tagged));
            let tagged_hit = hit_of(level.clip(
                game,
                [2.5, 65.5, 0.5],
                [2.5, 62.5, 0.5],
                Block::Outline,
                Fluid::Water,
                Context::Empty,
            ));
            assert_eq!((tagged_hit.block, tagged_hit.face), ([2, 64, 0], Face::Up));
            assert_eq!(tagged_hit.location[1], 64.0 + h8);
            game.server_items = None;
            assert_eq!(game.world.chunks.len(), 2, "clip loads no chunks");
            game.session_lock = None;
        });
    }

    #[test]
    #[ignore = "daily"]
    fn living_attributes_answer_for_mobs_stands_and_the_dragon() {
        use sunrise_client::{entities::ArmorStand, mobs::Mob};
        use sunrise_core::geometry::Vec3;
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 14;
            game.dragon.replace(dragon);
            for (id, max) in [(12, 20.0), (13, 20.0), (14, 200.0)] {
                let living = game.living(game.entity(id).unwrap()).unwrap();
                assert_eq!(
                    living.attribute_base(game, "minecraft:max_health"),
                    Some(max)
                );
                assert_eq!(
                    living.attribute_value(game, "minecraft:max_health"),
                    Some(max)
                );
                assert_eq!(living.attribute_value(game, "missing:attribute"), None);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_shapes_use_empty_context() {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use mod_api::{Authority, BlockShapeKind};
        use std::{sync::Arc, time::Instant};
        use sunrise_client::chunk::Chunk;

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            super::super::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::super::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        {
            let host: &dyn mod_api::ServerHost = &game;
            assert_eq!(
                host.levels()
                    .visit_levels(&mut |_| panic!("level before world open")),
                Ok(ControlFlow::Continue(()))
            );
        }
        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));

        let level = game.level("minecraft:overworld").unwrap();
        assert!(
            game.enter_command_level(super::super::Dim::Nether)
                .is_some()
        );
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 0, stone).unwrap();
        let nether = game.level("minecraft:the_nether").unwrap();
        let host: &dyn mod_api::ServerHost = &game;
        let levels = host.levels();
        let mut visited = Vec::new();
        assert_eq!(
            levels.visit_levels(&mut |level| {
                visited.push(level);
                ControlFlow::Continue(())
            }),
            Ok(ControlFlow::Continue(()))
        );
        // The parked Overworld precedes the active Nether.
        assert_eq!(visited, [level, nether]);
        visited.clear();
        assert_eq!(
            levels.visit_levels(&mut |level| {
                visited.push(level);
                ControlFlow::Break(())
            }),
            Ok(ControlFlow::Break(()))
        );
        assert_eq!(visited, [level]);
        assert!(
            game.enter_command_level(super::super::Dim::Overworld)
                .is_some()
        );
        let cube = vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
        assert_eq!(
            game.collect_level_block_shape(nether, stone, [0, 64, 0], BlockShapeKind::Outline),
            Some(cube.clone())
        );
        assert_eq!(
            game.collect_level_block_shape(nether, stone, [0, 64, 0], BlockShapeKind::Collision),
            Some(cube)
        );
        let shape = |game: &mut Game<'_>, name: &str, kind| {
            let state = game.pack.assets.registry.default_state(name).unwrap();
            game.world.set(0, 64, 0, state).unwrap();
            game.collect_level_block_shape(level, state, [0, 64, 0], kind)
                .unwrap()
        };
        game.inventory[game.selected] = Some(sunrise_client::items::ItemStack {
            item: "minecraft:light".into(),
            count: 1,
            damage: 0,
            extra: None,
        });
        assert!(shape(&mut game, "minecraft:light", BlockShapeKind::Outline).is_empty());
        assert_eq!(
            game.outline_boxes([0, 64, 0]),
            vec![[0.0, 64.0, 0.0, 1.0, 65.0, 1.0]]
        );
        game.inventory[game.selected].as_mut().unwrap().item = "minecraft:scaffolding".into();
        let scaffolding = shape(&mut game, "minecraft:scaffolding", BlockShapeKind::Outline);
        assert!(!scaffolding.is_empty());
        assert_ne!(scaffolding, vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]);
        assert_eq!(
            game.outline_boxes([0, 64, 0]),
            vec![[0.0, 64.0, 0.0, 1.0, 65.0, 1.0]]
        );
        assert_eq!(
            shape(&mut game, "minecraft:stone_slab", BlockShapeKind::Outline),
            vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
        );
        assert_eq!(
            shape(&mut game, "minecraft:stone_slab", BlockShapeKind::Collision),
            vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
        );
        let contains = |boxes: &[[f64; 6]], p: [f64; 3]| {
            boxes
                .iter()
                .any(|b| (0..3).all(|i| b[i] < p[i] && p[i] < b[i + 3]))
        };
        let stair = shape(&mut game, "minecraft:oak_stairs", BlockShapeKind::Outline);
        assert!(contains(&stair, [0.25, 0.25, 0.25]));
        let upper = [
            [0.25, 0.75, 0.25],
            [0.75, 0.75, 0.25],
            [0.25, 0.75, 0.75],
            [0.75, 0.75, 0.75],
        ];
        assert_eq!(
            upper.into_iter().filter(|p| contains(&stair, *p)).count(),
            2
        );
        let fence = shape(&mut game, "minecraft:oak_fence", BlockShapeKind::Outline);
        assert!(contains(&fence, [0.5, 0.5, 0.5]));
        assert!(!contains(&fence, [0.1, 0.5, 0.1]));
        let lectern = shape(&mut game, "minecraft:lectern", BlockShapeKind::Collision);
        assert!(contains(&lectern, [0.5, 0.5, 0.5]));
        assert!(!contains(&lectern, [0.1, 0.5, 0.1]));
    }

    #[test]
    fn uuid_words_are_most_significant_first() {
        let words = [0x0102_0304, -1, 0, 0x7fff_ffff];
        assert_eq!(
            super::uuid_from_words(words),
            0x0102_0304_ffff_ffff_0000_0000_7fff_ffff
        );
    }
    /// `LevelRef::drop_resources` (#5526) drops what `destroy_block` drops, without breaking
    /// the block: a stone gives its cobblestone and stays, a chest gives itself and keeps its
    /// contents, nothing drops under `doTileDrops` false, and air drops nothing. In a transaction the spawns wait for commit.
    #[test]
    #[ignore = "daily"]
    fn drop_resources_drops_without_breaking() {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use mod_api::{Authority, BlockUpdateFlags, HostTransaction};
        use std::{sync::Arc, time::Instant};
        use sunrise_client::{chunk::Chunk, items::ItemStack};

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            super::super::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::super::execution_role::ExecutionRole::IntegratedServer;
        options.dedicated = Some(Vec::new());
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));

        let level = game.level("minecraft:overworld").unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        let flags = BlockUpdateFlags::NEIGHBORS;
        let count = |game: &Game<'_>, item: &str| {
            game.dropped
                .iter()
                .filter(|dropped| dropped.stack.item.as_ref() == item)
                .map(|dropped| dropped.stack.count)
                .sum::<u32>()
        };
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        let collected = game
            .level_drop_resources(level, [0, 64, 0], None, None, true)
            .unwrap();
        assert!(
            collected
                .iter()
                .any(|stack| stack.item.as_ref() == "minecraft:cobblestone")
        );
        assert!(game.dropped.is_empty(), "collected, not spawned");
        assert_eq!(game.world.block(0, 64, 0), stone, "the block stays");
        assert_eq!(
            game.level_drop_resources(level, [0, 64, 0], None, None, false),
            Some(Vec::new())
        );
        assert_eq!(count(&game, "minecraft:cobblestone"), 1);
        assert_eq!(game.world.block(0, 64, 0), stone);
        game.dropped.clear();

        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        let Some(super::super::BlockEntity::Chest(items)) =
            game.world.block_entities.get_mut(&(0, 64, 0))
        else {
            panic!("chest block entity missing")
        };
        items[0] = Some(ItemStack {
            item: "minecraft:diamond".into(),
            count: 3,
            damage: 0,
            extra: None,
        });
        assert!(
            game.level_drop_resources(level, [0, 64, 0], None, None, false)
                .is_some()
        );
        // The chest's own loot only: its contents spill when the block is removed
        // (`Containers.dropContents`), which this doesn't do.
        assert_eq!(count(&game, "minecraft:chest"), 1);
        assert_eq!(count(&game, "minecraft:diamond"), 0);
        assert_eq!(game.world.block(0, 64, 0), chest);
        let Some(super::super::BlockEntity::Chest(items)) =
            game.world.block_entities.get(&(0, 64, 0))
        else {
            panic!("chest block entity missing")
        };
        assert_eq!(items[0].as_ref().map(|stack| stack.count), Some(3));
        game.dropped.clear();

        // Inside a transaction: rolled now, spawned at commit.
        let host: &mut dyn mod_api::ServerHost = &mut game;
        host.transaction(|host, _| {
            assert!(
                level
                    .drop_resources(host, [0, 64, 0], None, None, false)
                    .unwrap()
                    .is_some()
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(count(&game, "minecraft:chest"), 1);
        assert_eq!(game.world.block(0, 64, 0), chest);
        game.dropped.clear();

        assert!(game.ui.rules.set(
            "block_drops",
            sunrise_client_core::gamerules::Value::Bool(false)
        ));
        assert_eq!(
            game.level_drop_resources(level, [0, 64, 0], None, None, true),
            Some(Vec::new())
        );
        assert!(game.dropped.is_empty());

        assert_eq!(
            game.level_set_block(level, [0, 64, 0], air, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_drop_resources(level, [0, 64, 0], None, None, true),
            None
        );
    }

    #[test]
    #[ignore = "daily"]
    fn level_block_writes_validate_and_change_world() {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use mod_api::{Authority, BlockUpdateFlags, HostTransaction};
        use std::{sync::Arc, time::Instant};
        use sunrise_client::{chunk::Chunk, items::ItemStack};

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            super::super::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::super::execution_role::ExecutionRole::IntegratedServer;
        options.dedicated = Some(Vec::new());
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));

        let level = game.level("minecraft:overworld").unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        // Exercise the generated typed boundary, not only the native engine-input helpers.
        use crate::game::api_values::TryIntoApi;
        let api_stone: mod_api::BlockState = stone.try_into_api().unwrap();
        let invalid: mod_api::BlockState =
            mod_api::__private::block_state_from_ordinal(api_stone.block(), u16::MAX);
        assert_eq!(
            level.set_block(&mut game, [0, 64, 0], invalid, BlockUpdateFlags::NONE),
            Err(mod_api::LevelWriteError::InvalidState(
                mod_api::BlockStateError::UnknownState
            ))
        );
        assert_eq!(game.world.block(0, 64, 0), air);
        game.world.block_log = Some(Vec::new());
        assert_eq!(
            level.update_neighbors(
                &mut game,
                [0, 64, 0],
                invalid,
                mod_api::NeighbourUpdate::Neighbours
            ),
            Err(mod_api::LevelWriteError::InvalidState(
                mod_api::BlockStateError::UnknownState
            ))
        );
        assert_eq!(
            level.neighbor_changed(&mut game, [0, 64, 0], invalid),
            Err(mod_api::LevelWriteError::InvalidState(
                mod_api::BlockStateError::UnknownState
            ))
        );
        assert_eq!(game.world.block(0, 64, 0), air);
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        // Flag-aware writes log only client updates; ordinary World::set still logs by default.
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], stone, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        assert_eq!(
            game.level_set_block(level, [3, 64, 0], stone, BlockUpdateFlags::CLIENTS),
            Ok(true)
        );
        assert_eq!(game.world.block_log.as_ref().unwrap(), &vec![(3, 64, 0)]);
        assert_eq!(
            game.level_set_block(
                level,
                [4, 64, 0],
                stone,
                BlockUpdateFlags::CLIENTS | BlockUpdateFlags::INVISIBLE
            ),
            Ok(true)
        );
        assert_eq!(
            game.world.block_log.as_ref().unwrap(),
            &vec![(3, 64, 0), (4, 64, 0)]
        );
        // `sendBlockUpdated` queues the block again without writing; out of height it queues nothing.
        game.world.modified.clear();
        let dirty_before = game.world.modified.clone();
        let block_before = game.world.block(5, 64, 0);
        assert!(game.level_send_block_updated(level, [5, 64, 0]));
        assert!(game.level_send_block_updated(level, [5, 64, 0]));
        assert_eq!(game.world.modified.clone(), dirty_before);
        assert_eq!(game.world.block(5, 64, 0), block_before);
        let host: &mut dyn mod_api::ServerHost = &mut game;
        let aborted: Result<(), mod_api::TransactionError<()>> = host.transaction(|host, _| {
            assert!(level.send_block_updated(host, [6, 64, 0]));
            Err(())
        });
        assert!(aborted.is_err());
        assert!(game.world.block_log.as_ref().unwrap().contains(&(6, 64, 0)));
        assert!(!game.level_send_block_updated(level, [5, 10_000, 0]));
        assert_eq!(
            game.world.block_log.as_ref().unwrap(),
            &vec![(3, 64, 0), (4, 64, 0), (5, 64, 0), (6, 64, 0)]
        );
        // A bare hand mines at the base speed; an unloaded position has none.
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], None),
            Some(1.0)
        );
        assert_eq!(
            game.level_block_break_speed(level, [3, 10_000, 0], None),
            None
        );
        let mut pickaxe = ItemStack::new("minecraft:iron_pickaxe", 1);
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], Some(&pickaxe)),
            Some(6.0)
        );
        pickaxe.enchant("minecraft:efficiency", 5);
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], Some(&pickaxe)),
            Some(6.0)
        );
        let mut custom_tool = ItemStack::new("minecraft:stick", 1);
        let components = sunrise_client::item_data::command_components(
            r#"[tool={rules:[{blocks:"stone",speed:7.0f}],default_mining_speed:2.0f}]"#,
        )
        .unwrap();
        (custom_tool.damage, custom_tool.extra) =
            sunrise_client::item_data::decode_components(Some(&components));
        assert_eq!(
            game.level_block_break_speed(level, [3, 64, 0], Some(&custom_tool)),
            Some(7.0)
        );
        assert_eq!(
            game.level_block_break_speed(level, [5, 64, 0], Some(&custom_tool)),
            Some(2.0)
        );
        // Server-side game: the client-only crack is refused.
        assert!(!game.level_set_client_break_progress(level, 1, [3, 64, 0], 4));
        game.world.block_log = None;
        let waterlogged = game
            .pack
            .assets
            .registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:oak_slab".into(),
                properties: [
                    ("type".into(), "bottom".into()),
                    ("waterlogged".into(), "true".into()),
                ]
                .into_iter()
                .collect(),
            })
            .unwrap();
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        for moved in [false, true] {
            game.world.set(5, 64, 0, waterlogged).unwrap();
            assert!(game.level_remove_block(level, [5, 64, 0], moved));
            assert_eq!(game.world.block(5, 64, 0), water);
            game.world.set(5, 64, 0, waterlogged).unwrap();
            let removed = game.with_level(|_, level, ticks| {
                let mut host = super::super::mod_api_host::LevelTicks::new(level, ticks);
                mod_api::LevelWriteHost::remove_block(&mut host, [5, 64, 0], moved)
            });
            assert!(removed);
            assert_eq!(game.world.block(5, 64, 0), water);
        }
        let flags = BlockUpdateFlags::NEIGHBORS;
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        assert_eq!(game.world.block(0, 64, 0), stone);
        assert_eq!(
            game.level_set_block(level, [16, 64, 0], stone, flags),
            Ok(false)
        );
        assert_eq!(
            game.level_set_block(level, [0, 10000, 0], stone, flags),
            Ok(false)
        );
        let stale = mod_api::LevelRef::new(
            mod_api::WorldEpoch::new(level.epoch().get() + 1),
            level.dimension(),
        );
        assert_eq!(
            game.level_set_block(stale, [0, 64, 0], stone, flags),
            Err(mod_api::LevelWriteError::Stale)
        );
        assert!(
            game.level_destroy_block(stale, [0, 64, 0], true, None, None, false)
                .is_none()
        );
        let fill_chest = |game: &mut Game<'_>| {
            let Some(super::super::BlockEntity::Chest(items)) =
                game.world.block_entities.get_mut(&(0, 64, 0))
            else {
                panic!("chest block entity missing")
            };
            items[0] = Some(sunrise_client::items::ItemStack {
                item: "minecraft:diamond".into(),
                count: 3,
                damage: 0,
                extra: None,
            });
        };
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], air, flags),
            Ok(true)
        );
        assert_eq!(
            game.dropped
                .iter()
                .filter(|item| item.stack.item.as_ref() == "minecraft:diamond")
                .map(|item| item.stack.count)
                .sum::<u32>(),
            3
        );
        game.dropped.clear();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        assert_eq!(
            game.level_set_block(
                level,
                [0, 64, 0],
                air,
                BlockUpdateFlags::SKIP_BLOCK_ENTITY_SIDE_EFFECTS
            ),
            Ok(true)
        );
        assert!(!game.world.block_entities.contains_key(&(0, 64, 0)));
        assert!(game.dropped.is_empty());
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], false, None, None, false)
                .is_some()
        );
        assert!(game.dropped.is_empty());
        assert_eq!(game.world.block(0, 64, 0), air);
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], true, None, None, false)
                .is_some()
        );
        assert!(
            game.dropped
                .iter()
                .any(|item| item.stack.item.as_ref() == "minecraft:cobblestone")
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], true, None, None, false)
                .is_none()
        );
        game.dropped.clear();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        let host: &mut dyn mod_api::ServerHost = &mut game;
        let rolled_back: Result<(), mod_api::TransactionError<()>> = host.transaction(|host, _| {
            assert!(
                level
                    .destroy_block(host, [0, 64, 0], true, None, None, false)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                level.block_state(host, [0, 64, 0]),
                Some(air.try_into_api().unwrap())
            );
            Err(())
        });
        assert_eq!(rolled_back, Err(mod_api::TransactionError::Aborted(())));
        assert_eq!(game.world.block(0, 64, 0), chest);
        assert!(game.dropped.is_empty());
        let Some(super::super::BlockEntity::Chest(restored)) =
            game.world.block_entities.get(&(0, 64, 0))
        else {
            panic!("transaction did not restore the chest");
        };
        assert_eq!(restored[0].as_ref().unwrap().count, 3);
        let host: &mut dyn mod_api::ServerHost = &mut game;
        host.transaction(|host, _| {
            assert!(
                level
                    .destroy_block(host, [0, 64, 0], true, None, None, false)
                    .unwrap()
                    .is_some()
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(game.world.block(0, 64, 0), air);
        assert_eq!(
            game.dropped
                .iter()
                .filter(|item| item.stack.item.as_ref() == "minecraft:chest")
                .map(|item| item.stack.count)
                .sum::<u32>(),
            1
        );
        assert_eq!(
            game.dropped
                .iter()
                .filter(|item| item.stack.item.as_ref() == "minecraft:diamond")
                .map(|item| item.stack.count)
                .sum::<u32>(),
            3
        );
        game.dropped.clear();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Ok(true)
        );
        let host: &mut dyn mod_api::ServerHost = &mut game;
        let collected = host
            .transaction(|host, _| {
                Ok::<_, ()>(
                    level
                        .destroy_block(host, [0, 64, 0], true, None, None, true)
                        .unwrap()
                        .unwrap(),
                )
            })
            .unwrap();
        assert_eq!(collected.len(), 1);
        assert!(game.dropped.is_empty());
        assert_eq!(game.world.block(0, 64, 0), air);
        let copper = game
            .pack
            .assets
            .registry
            .default_state("minecraft:copper_chest")
            .unwrap();
        let exposed = game
            .pack
            .assets
            .registry
            .default_state("minecraft:exposed_copper_chest")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], copper, flags),
            Ok(true)
        );
        fill_chest(&mut game);
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], exposed, flags),
            Ok(true)
        );
        let Some(super::super::BlockEntity::Chest(items)) =
            game.world.block_entities.get(&(0, 64, 0))
        else {
            panic!("copper chest entity missing")
        };
        assert_eq!(items[0].as_ref().unwrap().count, 3);
        assert!(game.dropped.is_empty());
        let sensor = game
            .pack
            .assets
            .registry
            .default_state("minecraft:sculk_sensor")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], sensor, flags),
            Ok(true)
        );
        assert!(matches!(
            game.world.block_entities.get(&(0, 64, 0)),
            Some(super::super::BlockEntity::Sculk(_))
        ));
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        assert!(matches!(
            game.world.block_entities.get(&(0, 64, 0)),
            Some(super::super::BlockEntity::Chest(_))
        ));
        assert!(
            game.enter_command_level(super::super::Dim::Nether)
                .is_some()
        );
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], stone, flags),
            Err(mod_api::LevelWriteError::Unavailable)
        );
        assert!(
            game.level_destroy_block(level, [0, 64, 0], true, None, None, false)
                .is_none()
        );
    }

    #[test]
    #[ignore = "daily"]
    fn destroy_block_passes_breaker_to_loot_and_game_event() {
        crate::game::in_own_process(
            "game::native_entity_refs::tests::destroy_block_passes_breaker_to_loot_and_game_event",
            || {
                use mod_api::{BlockUpdateFlags, EntityRef, Flow, Hooks, ModEntry, ServerHost};
                use std::sync::{Arc, Mutex};
                use sunrise_client::{chunk::Chunk, mobs::Mob};
                use sunrise_client_core::who::EventSource;
                use sunrise_core::geometry::Vec3;

                struct Observer {
                    drop_breakers: Arc<Mutex<Vec<Option<EntityRef>>>>,
                    destroy_causes: Arc<Mutex<Vec<Option<u32>>>>,
                }
                #[mod_api::__subscriptions]
                impl Hooks for Observer {
                    fn block_drops(
                        &self,
                        _: &dyn mod_api::LevelHost,
                        event: &mut mod_api::BlockDropsEvent<'_>,
                    ) -> Flow {
                        self.drop_breakers
                            .lock()
                            .unwrap()
                            .push(event.breaker_entity);
                        Flow::Continue
                    }
                    fn vanilla_game_event(
                        &self,
                        _: &mut dyn ServerHost,
                        event: &mut mod_api::VanillaGameEventEvent<'_>,
                    ) -> Flow {
                        if event.event == "minecraft:block_destroy" {
                            self.destroy_causes.lock().unwrap().push(event.cause);
                        }
                        Flow::Continue
                    }
                }

                let drop_breakers = Arc::new(Mutex::new(Vec::new()));
                let destroy_causes = Arc::new(Mutex::new(Vec::new()));
                mod_platform::testing::with_mods(
                    vec![ModEntry::new(
                        "destroy_breaker_observer",
                        Box::new(Observer {
                            drop_breakers: drop_breakers.clone(),
                            destroy_causes: destroy_causes.clone(),
                        }),
                    )]
                    .into(),
                    || {
                        with_game(|game| {
                            game.options.dedicated = Some(Vec::new());
                            game.sculk_prepare();
                            game.streaming
                                .insert_remote(&mut game.world, Chunk::empty(0, 0));
                            let level = game.level(game.dim.id()).unwrap();
                            let reg = game.pack.assets.registry;
                            let [chorus_flower, sensor, air] = [
                                "minecraft:chorus_flower",
                                "minecraft:sculk_sensor",
                                "minecraft:air",
                            ]
                            .map(|name| reg.default_state(name).unwrap());
                            let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
                            let pig_kind =
                                game.res.mob_types().get("minecraft:pig").unwrap().clone();
                            game.mobs
                                .push(Mob::new(42, pig_kind, Vec3::new(0.5, 64.0, 2.5), 0.0));
                            let pig = game.entity(42).unwrap();
                            assert_eq!(
                                game.level_set_block(level, [2, 64, 0], sensor, flags),
                                Ok(true)
                            );
                            game.sculk_sync_index();

                            let sensor_vibration = |game: &mut super::Game<'_>| {
                                let Some(super::super::BlockEntity::Sculk(sculk)) =
                                    game.world.block_entities.get_mut(&(2, 64, 0))
                                else {
                                    panic!("sculk sensor missing");
                                };
                                sculk
                                    .vibration
                                    .selector
                                    .current
                                    .take()
                                    .map(|(info, _)| info)
                            };

                            // Without a breaker: chorus_flower's `entity_properties` on `this` fails
                            // (no drop), block_drops sees None, and sculk/game_event see no source.
                            assert_eq!(
                                game.level_set_block(level, [0, 64, 0], chorus_flower, flags),
                                Ok(true)
                            );
                            let _ = sensor_vibration(game);
                            assert_eq!(
                                game.level_destroy_block(
                                    level,
                                    [0, 64, 0],
                                    true,
                                    None,
                                    None,
                                    false
                                ),
                                Some(Vec::new())
                            );
                            assert!(game.dropped.is_empty());
                            let info = sensor_vibration(game).expect("sculk sensor heard break");
                            assert_eq!(info.source, None);

                            // Stale breaker refuses before breaking the block.
                            assert_eq!(
                                game.level_set_block(level, [0, 64, 0], chorus_flower, flags),
                                Ok(true)
                            );
                            let stale = EntityRef::new(
                                mod_api::WorldEpoch::new(pig.epoch().get() + 1),
                                pig.id(),
                            );
                            assert!(
                                game.level_destroy_block(
                                    level,
                                    [0, 64, 0],
                                    true,
                                    None,
                                    Some(stale),
                                    false
                                )
                                .is_none()
                            );
                            assert_eq!(game.world.block(0, 64, 0), chorus_flower);

                            // With a breaker outside a transaction: chorus_flower drops, hooks & sculk see pig.
                            let _ = sensor_vibration(game);
                            assert_eq!(
                                game.level_destroy_block(
                                    level,
                                    [0, 64, 0],
                                    true,
                                    None,
                                    Some(pig),
                                    false
                                ),
                                Some(Vec::new())
                            );
                            assert_eq!(game.world.block(0, 64, 0), air);
                            assert!(
                                game.dropped
                                    .iter()
                                    .any(|item| item.stack.item.as_ref()
                                        == "minecraft:chorus_flower")
                            );
                            let info = sensor_vibration(game).expect("sculk sensor heard break");
                            assert_eq!(info.source, Some(EventSource::Mob(42)));

                            // Same with a breaker inside a committed transaction.
                            game.dropped.clear();
                            assert_eq!(
                                game.level_set_block(level, [0, 64, 0], chorus_flower, flags),
                                Ok(true)
                            );
                            let _ = sensor_vibration(game);
                            let ok: Result<(), ()> = game.storage_transaction(|game, _| {
                                assert_eq!(
                                    game.level_destroy_block(
                                        level,
                                        [0, 64, 0],
                                        true,
                                        None,
                                        Some(pig),
                                        false
                                    ),
                                    Some(Vec::new())
                                );
                                assert!(game.dropped.is_empty());
                                Ok(())
                            });
                            assert!(ok.is_ok());
                            assert_eq!(game.world.block(0, 64, 0), air);
                            assert!(
                                game.dropped
                                    .iter()
                                    .any(|item| item.stack.item.as_ref()
                                        == "minecraft:chorus_flower")
                            );
                            let info = sensor_vibration(game).expect("sculk sensor heard break");
                            assert_eq!(info.source, Some(EventSource::Mob(42)));

                            assert_eq!(
                                *drop_breakers.lock().unwrap(),
                                vec![None, Some(pig), Some(pig)]
                            );
                            assert_eq!(
                                *destroy_causes.lock().unwrap(),
                                vec![None, Some(42), Some(42)]
                            );
                        });
                    },
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn client_box_queries_use_live_display_positions_and_section_order() {
        use super::Game;
        use mod_api::Host;
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;
        use sunrise_core::nbt::Compound;
        use sunrise_protocol::{
            connection::Pipe,
            entity_data::Value,
            packets::play::{self, Clientbound as C},
        };

        const CASE: &str = "SUNRISE_DISPLAY_BOX_QUERY_CHILD";
        if run_fresh_cases(
            "game::native_entity_refs::tests::client_box_queries_use_live_display_positions_and_section_order",
            CASE,
            &["client", "server"],
        ) {
            return;
        }

        const LO: [f64; 3] = [13.0, 65.0, 2.0];
        const HI: [f64; 3] = [19.0, 66.0, 4.0];
        const ADMISSIONS: [(i32, &str, [f64; 3]); 4] = [
            (31, "minecraft:item_display", [17.0, 65.5, 3.0]),
            (92, "minecraft:block_display", [15.5, 65.5, 3.0]),
            (55, "minecraft:pig", [14.5, 65.5, 3.0]),
            (11, "minecraft:text_display", [13.5, 65.5, 3.0]),
        ];
        let chunks = |game: &mut Game<'_>| {
            for x in 0..=1 {
                game.world
                    .chunks
                    .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
            }
        };
        let ids = |host: &dyn Host, level, min, max| {
            collect_entities_in(host, level, min, max)
                .unwrap()
                .into_iter()
                .map(|e| e.id())
                .collect::<Vec<_>>()
        };
        match std::env::var(CASE).unwrap().as_str() {
            "client" => with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "DisplayBoxTest").unwrap();
                let remote = game.remote.as_mut().unwrap();
                remote.logged_in = true;
                remote.entity_id = 7;
                chunks(game);
                for (id, name, position) in ADMISSIONS {
                    let kind = game
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:entity_type")
                        .unwrap()
                        .id(name)
                        .unwrap();
                    game.handle_packet(C::AddEntity(play::AddEntity {
                        id,
                        uuid: id as u128,
                        kind,
                        position,
                        velocity: [0.0; 3],
                        yaw: 0,
                        pitch: 0,
                        head_yaw: 0,
                        data: 0,
                    }))
                    .unwrap();
                }
                let level = game.level("minecraft:overworld").unwrap();
                assert_eq!(ids(game, level, HI, LO), [92, 55, 11, 31]);
                let mut seen = Vec::new();
                assert_eq!(
                    level.visit_entities_in(game, LO, HI, &mut |entity| {
                        seen.push(entity.id());
                        let (_, kind, at) = ADMISSIONS
                            .into_iter()
                            .find(|(id, _, _)| *id as u32 == entity.id())
                            .unwrap();
                        assert_eq!(
                            entity.kind(game).and_then(|k| k.id(game)).as_deref(),
                            Some(kind)
                        );
                        assert_eq!(entity.position(game), Some(at));
                        if entity.id() != 55 {
                            assert_eq!(entity.bounding_box(game), Some([at; 2]));
                        }
                        ControlFlow::Continue(())
                    }),
                    Ok(ControlFlow::Continue(()))
                );
                assert_eq!(seen, [92, 55, 11, 31]);

                let block = game.entity(92).unwrap();
                game.text_displays.get_mut(&92).unwrap().metadata(&[
                    (10, 1, Value::Int(3)),
                    (20, 2, Value::Float(4.0)),
                    (21, 2, Value::Float(4.0)),
                ]);
                game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
                    id: 92,
                    path: play::PositionPath::Linear([18.5, 65.5, 3.0]),
                    yaw: 0.0,
                    pitch: 0.0,
                    on_ground: false,
                }))
                .unwrap();
                assert_eq!(block.position(game), Some([15.5, 65.5, 3.0]));
                assert_eq!(block.bounding_box(game), Some([[15.5, 65.5, 3.0]; 2]));
                assert_eq!(ids(game, level, LO, HI), [92, 55, 11, 31]);

                game.tick_text_displays(true);
                assert_eq!(ids(game, level, LO, HI), [55, 11, 31, 92]);
                let tight_lo = [16.25, 65.0, 2.5];
                let tight_hi = [16.75, 66.0, 3.5];
                let mut seen = Vec::new();
                let stopped = level.visit_entities_in(game, LO, HI, &mut |entity| {
                    seen.push(entity.id());
                    assert_eq!(ids(game, level, tight_lo, tight_hi), [92]);
                    ControlFlow::Break(())
                });
                assert_eq!(stopped, Ok(ControlFlow::Break(())));
                assert_eq!(seen, [55]);
                assert_eq!(block.position(game), Some([16.5, 65.5, 3.0]));
                assert_eq!(block.bounding_box(game), Some([[16.5, 65.5, 3.0]; 2]));
                assert!(ids(game, level, [16.5, 65.0, 2.5], tight_hi).is_empty());
                let culling_only_lo = [16.5, 67.0, 2.5];
                let culling_only_hi = [16.75, 68.0, 3.5];
                assert!(ids(game, level, culling_only_lo, culling_only_hi).is_empty());
                let (lo, hi) = game.text_displays[&92].interpolated_bounding_box(1.0);
                let culling = super::aabb_extents((lo.to_array(), hi.to_array()));
                let culling_only = super::aabb_extents((culling_only_lo, culling_only_hi));
                assert!(culling.intersects(culling_only));
                let mut picked = Vec::new();
                game.for_each_pickable_in_bounds(super::aabb_extents((LO, HI)), |id, _, _| {
                    picked.push(id)
                });
                assert_eq!(picked, [55]);

                game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![92] }))
                    .unwrap();
                assert!(game.entity(92).is_none());
                assert_eq!(block.position(game), None);
                assert_eq!(block.bounding_box(game), None);
                assert_eq!(ids(game, level, LO, HI), [55, 11, 31]);
            }),
            "server" => with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                chunks(game);
                let level = game.level("minecraft:overworld").unwrap();
                let [item, block, pig, text] = ADMISSIONS.map(|(_, kind, at)| {
                    game.level_summon(level, kind, at, Compound::new())
                        .unwrap()
                        .unwrap()
                        .id()
                });
                assert_eq!(ids(game, level, LO, HI), [block, pig, text, item]);
                for (id, (_, _, at)) in [item, block, pig, text].into_iter().zip(ADMISSIONS) {
                    if id != pig {
                        let display = game.entity(id).unwrap();
                        assert_eq!(display.position(game), Some(at));
                        assert_eq!(display.bounding_box(game), Some([at; 2]));
                    }
                }
                let at = [16.5, 65.5, 3.0];
                assert_eq!(
                    game.place_teleport_filed(
                        super::Target::Other(block),
                        at,
                        None,
                        [false; 3],
                        false
                    ),
                    Some(ADMISSIONS[1].2)
                );
                assert_eq!(ids(game, level, LO, HI), [pig, text, item, block]);
                assert_eq!(
                    game.entity(block).unwrap().bounding_box(game),
                    Some([at; 2])
                );
                // `/data merge` moves it through `Entity.load`: the old box loses it and the
                // new one, queried for the first time, finds it.
                let far_lo = [39.5, 65.0, 2.0];
                let far_hi = [41.5, 66.0, 4.0];
                assert!(ids(game, level, far_lo, far_hi).is_empty());
                let target = super::Target::Other(block);
                let mut tag = game.command_target_nbt(target).unwrap();
                let pos = sunrise_core::nbt::Tag::List(
                    [40.5, 65.5, 3.0]
                        .map(|v: f64| sunrise_core::nbt::Tag::Double(v.to_bits()))
                        .to_vec(),
                );
                if let sunrise_core::nbt::Tag::Compound(c) = &mut tag {
                    c.insert("Pos".into(), pos);
                }
                game.apply_entity_nbt(target, tag).unwrap();
                assert_eq!(ids(game, level, LO, HI), [pig, text, item]);
                assert_eq!(ids(game, level, far_lo, far_hi), [block]);
            }),
            _ => unreachable!("unexpected display query case"),
        }
    }

    /// Crossings in one normal client tick land in `ClientLevel.tickEntities` order whichever
    /// Sunrise loop moved them and whether or not a read-only tick listener splits the displays
    /// off; a tracked player is filed before its `player_tick_post`; a rider snapped with its
    /// display is filed at its seat before it ticks.
    #[test]
    #[ignore = "daily"]
    fn client_crossings_land_in_tick_order_and_before_hooks_read_them() {
        use super::Game;
        use mod_api::{Hooks, Host, HostSide, ModEntry, PlayerTickPostEvent};
        use std::sync::{Arc, Mutex};
        use sunrise_client::chunk::Chunk;
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::{
            connection::Pipe,
            entity_data::Value,
            packets::play::{self, Clientbound as C},
        };

        const CASE: &str = "SUNRISE_CLIENT_CROSSING_CHILD";
        if run_fresh_cases(
            "game::native_entity_refs::tests::client_crossings_land_in_tick_order_and_before_hooks_read_them",
            CASE,
            &["order", "order_listened", "tracked_player", "rider"],
        ) {
            return;
        }

        /// Asks for each block display's tick alone, reading nothing; answers the box around
        /// player 18 from its `player_tick_post`.
        struct Probe(Arc<Mutex<Vec<Vec<u32>>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Probe {
            fn player_tick_post(&self, host: HostSide<'_>, event: &mut PlayerTickPostEvent) {
                let HostSide::Client(host) = host else { return };
                if event.player.entity().id() != 18 {
                    return;
                }
                let level = host.level("minecraft:overworld").unwrap();
                let found = collect_entities_in(&*host, level, [30.0, 0.0, -0.5], [31.0, 1.8, 0.5])
                    .unwrap()
                    .into_iter()
                    .map(|e| e.id())
                    .collect();
                self.0.lock().unwrap().push(found);
            }
        }

        let ids = |host: &dyn Host, min, max| {
            let level = host.level("minecraft:overworld").unwrap();
            collect_entities_in(host, level, min, max)
                .unwrap()
                .into_iter()
                .map(|e| e.id())
                .collect::<Vec<_>>()
        };
        let connect = |game: &mut Game<'_>| {
            let (client, server) = Pipe::pair();
            std::mem::forget(server);
            game.connect_local(client, "CrossingTest").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            for x in -1..=4 {
                game.world
                    .chunks
                    .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
            }
        };
        let add = |game: &mut Game<'_>, id: i32, name: &str, position: [f64; 3]| {
            let kind = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id(name)
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id,
                uuid: id as u128,
                kind,
                position,
                velocity: [0.0; 3],
                yaw: 0,
                pitch: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
        };
        let sync = |game: &mut Game<'_>, id: i32, to: [f64; 3]| {
            game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
                id,
                path: play::PositionPath::Linear(to),
                yaw: 0.0,
                pitch: 0.0,
                on_ground: false,
            }))
            .unwrap();
        };
        let heard = Arc::new(Mutex::new(Vec::new()));
        let case = std::env::var(CASE).unwrap();
        let mods = if case == "order" {
            Vec::new()
        } else {
            vec![ModEntry::new("probe", Box::new(Probe(heard.clone())))]
        };
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                connect(game);
                match case.as_str() {
                    "order" | "order_listened" => {
                        // The pig joined first, so vanilla moves it first: both enter section 1
                        // on this tick, the pig ahead of the display the display loop moved.
                        add(game, 70, "minecraft:pig", [14.5, 65.5, 3.0]);
                        add(game, 71, "minecraft:block_display", [13.5, 65.5, 3.0]);
                        game.text_displays.get_mut(&71).unwrap().metadata(&[(
                            10,
                            1,
                            Value::Int(3),
                        )]);
                        sync(game, 70, [20.5, 65.5, 3.0]);
                        sync(game, 71, [22.5, 65.5, 3.0]);
                        game.tick_remote_entities(true);
                        assert_eq!(
                            game.entity(71).unwrap().position(game),
                            Some([16.5, 65.5, 3.0])
                        );
                        assert_eq!(ids(game, [16.0, 65.0, 2.5], [17.0, 66.0, 3.5]), [70, 71]);
                    }
                    "tracked_player" => {
                        let player = game
                            .remote
                            .as_ref()
                            .unwrap()
                            .builtin
                            .get("minecraft:entity_type")
                            .unwrap()
                            .id("minecraft:player")
                            .unwrap();
                        game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                            actions: 1,
                            entries: vec![play::PlayerInfo {
                                uuid: 9,
                                name: Some("Player18".into()),
                                ..Default::default()
                            }],
                        }))
                        .unwrap();
                        game.handle_packet(C::AddEntity(play::AddEntity {
                            id: 18,
                            uuid: 9,
                            kind: player,
                            position: [0.5, 0.0, 0.0],
                            velocity: [0.0; 3],
                            pitch: 0,
                            yaw: 0,
                            head_yaw: 0,
                            data: 0,
                        }))
                        .unwrap();
                        // A tracked player's interpolation carries it halfway on this step, to
                        // x 30.5: section 1, beyond the padded range of a box around its new
                        // position.
                        sync(game, 18, [60.5, 0.0, 0.0]);
                        game.tick_remote_entities(true);
                        assert_eq!(*heard.lock().unwrap(), [vec![18]]);
                    }
                    "rider" => {
                        let mut local =
                            sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO)
                                .unwrap();
                        local.entity_id = 7;
                        local.position = Vec3::new(15.5, 65.5, 3.0);
                        game.player = Some(local);
                        game.bind_local_player_residency();
                        add(game, 92, "minecraft:block_display", [15.5, 65.5, 3.0]);
                        game.handle_packet(C::SetPassengers(play::SetPassengers {
                            vehicle: 92,
                            passengers: vec![7],
                        }))
                        .unwrap();
                        let near =
                            |game: &Game<'_>| ids(game, [39.5, 60.0, 2.0], [41.5, 70.0, 4.0]);
                        assert!(near(game).is_empty());
                        // No teleport duration: the display snaps, and its rider with it.
                        sync(game, 92, [40.5, 65.5, 3.0]);
                        let seat = game.player.as_ref().unwrap().position;
                        use sunrise_client::join_seq::residency::table::SectionKey;
                        assert_eq!(
                            SectionKey::at(seat.components()),
                            SectionKey::at([40.5, 65.5, 3.0])
                        );
                        assert_eq!(near(game), [92, 7]);
                    }
                    _ => unreachable!("unexpected crossing case"),
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_client_finds_the_entities_it_tracks_and_its_own_player_in_a_box() {
        if run_fresh_cases(
            "game::native_entity_refs::tests::a_client_finds_the_entities_it_tracks_and_its_own_player_in_a_box",
            "SUNRISE_CLIENT_ENTITY_BOX_QUERY_CHILD",
            &["client"],
        ) {
            return;
        }
        use super::super::execution_role::ExecutionRole;
        use mod_api::{Host, LevelRef, WorldEpoch};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::connection::Pipe;
        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EntitiesInTest").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            let mut local =
                sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            local.entity_id = 7;
            game.player = Some(local);
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            game.pick_section_sync_id(15);
            game.spawn_entity(16, "minecraft:pig", [40.0, 0.0, 0.0], [0.0; 3]);
            game.pick_section_sync_id(16);
            game.bind_local_player_residency();
            let level = game.level("minecraft:overworld").unwrap();
            let ids = |host: &dyn Host, level: LevelRef, min, max| {
                collect_entities_in(host, level, min, max)
                    .map(|found| found.into_iter().map(|e| e.id()).collect::<Vec<_>>())
            };
            // The tracked pig, then the window's own player; the other pig is far away.
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Ok(vec![15, 7])
            );
            assert_eq!(
                ids(game, level, [39.0, -1.0, -1.0], [41.0, 2.0, 1.0]),
                Ok(vec![16])
            );
            // The pig's box ends at x 2.45.
            assert_eq!(
                ids(game, level, [2.5, 0.0, -1.0], [4.0, 1.0, 1.0]),
                Ok(vec![])
            );
            game.set_local_mode(sunrise_client::command::args::GameMode::Spectator);
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Ok(vec![15])
            );
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(
                ids(game, nether, [-1.0; 3], [3.0; 3]),
                Err(LevelReadError::Unavailable)
            );
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(
                ids(game, stale, [-1.0; 3], [3.0; 3]),
                Err(LevelReadError::Stale)
            );
            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(
                ids(game, level, [-1.0; 3], [3.0; 3]),
                Err(LevelReadError::Unavailable)
            );
        });
    }

    /// A client's `visit_entities_in` visits another player standing in the box (added through the
    /// real `AddEntity` packet), after the entities the server tracks and before the window's
    /// own player; a player outside the box, and one the server removed, are not listed.
    #[test]
    #[ignore = "daily"]
    fn a_client_lists_another_player_in_the_box_after_the_tracked_entities() {
        if run_fresh_cases(
            "game::native_entity_refs::tests::a_client_lists_another_player_in_the_box_after_the_tracked_entities",
            "SUNRISE_CLIENT_ENTITY_BOX_QUERY_CHILD",
            &["client"],
        ) {
            return;
        }
        use super::super::execution_role::ExecutionRole;
        use mod_api::{Host, LevelRef};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::{
            connection::Pipe,
            packets::play::{
                AddEntity, Clientbound as C, PlayerInfo, PlayerInfoUpdate, RemoveEntities,
            },
        };
        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EntitiesInTest").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            let player = remote
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id("minecraft:player")
                .unwrap();
            let mut local =
                sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            local.entity_id = 7;
            game.player = Some(local);
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            game.pick_section_sync_id(15);
            let add_player = |game: &mut crate::game::Game<'_>, id, uuid, x| {
                game.handle_packet(C::PlayerInfoUpdate(PlayerInfoUpdate {
                    actions: 1,
                    entries: vec![PlayerInfo {
                        uuid,
                        name: Some(format!("Player{id}")),
                        ..Default::default()
                    }],
                }))
                .unwrap();
                game.handle_packet(C::AddEntity(AddEntity {
                    id,
                    uuid,
                    kind: player,
                    position: [x, 0.0, 0.0],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                }))
                .unwrap();
            };
            // 18 stands in the box, 19 far outside it, 20 in it until the server removes it.
            add_player(game, 18, 9, 1.0);
            add_player(game, 19, 10, 40.0);
            add_player(game, 20, 11, 0.5);
            game.bind_local_player_residency();
            assert!(
                [18, 19, 20]
                    .iter()
                    .all(|id| game.mobs.iter().any(|m| m.id == *id))
            );
            let level = game.level("minecraft:overworld").unwrap();
            let ids = |host: &dyn Host, level: LevelRef, min, max| {
                collect_entities_in(host, level, min, max)
                    .map(|found| found.into_iter().map(|e| e.id()).collect::<Vec<_>>())
            };
            // Section entry order: the pig, then the other players, then the window's own.
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Ok(vec![15, 18, 20, 7])
            );
            // A box around only the far player finds it, and the near box did not.
            assert_eq!(
                ids(game, level, [39.0, -1.0, -1.0], [41.0, 2.0, 1.0]),
                Ok(vec![19])
            );
            game.handle_packet(C::RemoveEntities(RemoveEntities { ids: vec![20] }))
                .unwrap();
            assert_eq!(
                ids(game, level, [-1.0, -1.0, -1.0], [3.0, 2.0, 1.0]),
                Ok(vec![15, 18, 7])
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_client_refuses_a_box_with_a_bound_that_is_not_finite_and_swaps_an_inverted_one() {
        if run_fresh_cases(
            "game::native_entity_refs::tests::a_client_refuses_a_box_with_a_bound_that_is_not_finite_and_swaps_an_inverted_one",
            "SUNRISE_CLIENT_ENTITY_BOX_QUERY_CHILD",
            &["client"],
        ) {
            return;
        }
        use super::super::execution_role::ExecutionRole;
        use mod_api::{Host, LevelRef};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::connection::Pipe;
        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EntitiesInTest").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            let mut local =
                sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            local.entity_id = 7;
            game.player = Some(local);
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            game.pick_section_sync_id(15);
            game.bind_local_player_residency();
            let level = game.level("minecraft:overworld").unwrap();
            let ids = |host: &dyn Host, level: LevelRef, min, max| {
                collect_entities_in(host, level, min, max)
                    .map(|found| found.into_iter().map(|e| e.id()).collect::<Vec<_>>())
            };
            // Mod code chooses the bounds: one that is not finite is refused, never panics,
            // and nothing is walked by the box's size.
            for (min, max) in [
                ([f64::NAN, -1.0, -1.0], [3.0, 2.0, 1.0]),
                ([-1.0, -1.0, -1.0], [3.0, f64::NAN, 1.0]),
                ([f64::NEG_INFINITY, -1.0, -1.0], [3.0, 2.0, 1.0]),
                ([-1.0, -1.0, -1.0], [3.0, 2.0, f64::INFINITY]),
                ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]),
            ] {
                assert_eq!(
                    ids(game, level, min, max),
                    Err(LevelReadError::Unavailable),
                    "{min:?} {max:?}"
                );
            }
            // A minimum above its maximum on one axis is the same box swapped (vanilla's
            // AABB): the tracked pig, then the window's own player.
            assert_eq!(
                ids(game, level, [3.0, -1.0, -1.0], [-1.0, 2.0, 1.0]),
                Ok(vec![15, 7])
            );
            assert_eq!(
                ids(game, level, [3.0, 2.0, 1.0], [-1.0, -1.0, -1.0]),
                Ok(vec![15, 7])
            );
        });
    }
}

#[cfg(test)]
#[path = "native_entity_refs/restitution_replay_tests.rs"]
mod restitution_replay_tests;
