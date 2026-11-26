//! `entity_entering_section` (`EntityEvent.EnteringSection`): vanilla tells it from
//! `Entity.setPosRaw`, whose `EntityInLevelCallback.onMove` (`PersistentEntitySectionManager`'s
//! callback on a server, `TransientEntitySectionManager`'s on a client) compares the section the
//! entity is filed under with the one its new position is in, and moves it at once when they
//! differ. Sunrise's kinds keep their positions in their own fields, so the hook is told from
//! the places that write them: [`EntitySections`] files each entity under its section, and each
//! position-write boundary hands it the entity's new position immediately after the write.
//!
//! The boundaries: an entity joining (the server's `native_joins_flush`, a client's
//! `AddEntity`) files it untold. The mob and item loops report each committed write while their
//! rows are resident again; minecarts, boats and the remaining server entities report after each
//! entity's own update. A client reports interpolation writes per entity. A teleport, a piston
//! push, a mod's position write, each move a server player sent and each packet that moves an
//! entity on a client are one each.
//! Every boundary that sees another section records a crossing from the section the entity was
//! filed under and tells it before the write returns, so two moves told apart are two crossings,
//! and a move out and back is two.
//!
//! A listener runs synchronously after the new section is filed. Crossings from nested writes
//! wait for the current event's listeners to return, then are told in write order before the
//! owning boundary releases its rows. Each crossing carries the placement of the move that filed
//! it: a read of that entity while it is told answers that placement, and a later move, or the
//! entity's removal, neither rewrites nor deletes a crossing already filed (the no-reentry rule
//! rules out telling it at its write). Listeners should defer moving or removing the crossing
//! entity itself until its write completes.
//!
//! Section entry order (`EntitySection` keeps its entities in the order they entered it, which
//! a chunk's save follows) does not depend on a listener: a server boundary that knows both
//! ends of the write compares their section coordinates (integers) and, only on a crossing,
//! tells the residency table (`EntityTable::entered_section`, one hash lookup per crossing).
//! A write inside one section costs the compare alone.
//!
//! With no listener a boundary is that compare and one subscription check, and nothing is
//! filed. With one, a boundary is a hash lookup and the reusable crossing record is told
//! without a per-frame scan.

#[path = "entity_sections/observer_cache.rs"]
pub(super) mod observer_cache;
#[path = "entity_sections/player.rs"]
mod player;
pub(super) use player::PlayerSectionObserver;

use super::Game;
use crate::mobs::living::section_writes::PositionWrite;
use mod_api::__private::game::BackendRefOf;
use mod_api::{DimensionKey, EntityRef, EntityResidency, WorldEpoch};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};

#[cfg(test)]
#[path = "entity_sections/tests.rs"]
mod tests;

/// `SectionPos.blockToSectionCoord(Mth.floor(v))`.
fn section_coord(v: f64) -> i32 {
    sunrise_core::mth::floor(v) >> 4
}

fn section_of([x, y, z]: [f64; 3]) -> [i32; 3] {
    [section_coord(x), section_coord(y), section_coord(z)]
}

/// Whether a write from `from` to `to` leaves the section it was in: six integer shifts and a
/// compare, no allocation and no lookup.
pub(super) fn crossed(from: [f64; 3], to: [f64; 3]) -> bool {
    section_of(from) != section_of(to)
}

/// A section change a boundary found: the entity, its old and its new section, and where it
/// was put by the move that filed it. A crossing queued behind a listener keeps its own
/// placement: a later move or removal of the entity neither rewrites nor deletes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Crossing {
    pub(super) id: u32,
    pub(super) old: [i32; 3],
    pub(super) new: [i32; 3],
    pub(super) at: [f64; 3],
}

/// Where a boundary is: the world open and the level the entity is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Scope {
    epoch: WorldEpoch,
    level: DimensionKey,
}

/// The section each entity is filed under, and the crossings not told yet.
#[derive(Default)]
pub(super) struct EntitySections {
    /// The world open the table was filled in: another one starts it over.
    epoch: Option<WorldEpoch>,
    /// Each entity's level and section.
    filed: HashMap<u32, (DimensionKey, [i32; 3])>,
    /// Not-yet-told crossings. Keep them in the table so a nested removal can forget
    /// its pending moves even while an earlier one is being told.
    queue: VecDeque<Crossing>,
    /// A telling is under way: crossings from nested writes join the end of the queue.
    busy: bool,
    /// The mobs' position updates during their family's tick; kept for its capacity.
    pub(super) ticked: Vec<PositionWrite>,
    /// Mobs that crossed into another section during their family's tick, in the order of the
    /// writes; told to the residency table once the pass has its rows back. Kept for its capacity.
    pub(super) entered: Vec<(u32, [f64; 3])>,
    /// A projectile row is lent during its individual tick. Position reads during its write
    /// boundary use this value until the row is restored.
    pub(super) write_position: Option<(u32, [f64; 3])>,
    /// Projectile ids seen by the current pass; kept to forget rows removed during settlement.
    pub(super) projectile_ids: Vec<u32>,
    /// Projectile ids resident after settlement; kept to avoid a fresh set per pass.
    pub(super) projectile_live: sunrise_client::fast_hash::FxHashSet<u32>,
}

impl EntitySections {
    /// The table for `scope`'s world: another world's entries are dropped.
    fn table(&mut self, scope: Scope) -> &mut HashMap<u32, (DimensionKey, [i32; 3])> {
        if self.epoch != Some(scope.epoch) {
            self.epoch = Some(scope.epoch);
            self.filed.clear();
            self.queue.clear();
        }
        &mut self.filed
    }

    /// `addEntity`: entity `id` is filed at `at`, untold. A new entity under an id seen before
    /// (a client's `AddEntity` replacing one) starts over too.
    pub(super) fn placed(&mut self, scope: Scope, id: u32, at: [f64; 3]) {
        self.table(scope).insert(id, (scope.level, section_of(at)));
    }

    /// `setPosRaw`: entity `id` is at `to` now. Filed under another section of the same level,
    /// that is a crossing. One not filed in this level yet (it changed level, or its join is
    /// still to be told) is filed where it was, `from` when the boundary knows it, so a move
    /// before its join is told is a crossing too; otherwise it is filed untold. Returns
    /// whether a crossing was queued.
    pub(super) fn moved(
        &mut self,
        scope: Scope,
        id: u32,
        from: Option<[f64; 3]>,
        to: [f64; 3],
    ) -> bool {
        let new = section_of(to);
        let old = match self.table(scope).entry(id) {
            Entry::Occupied(mut entry) => {
                let (level, section) = entry.get_mut();
                if *level != scope.level {
                    *level = scope.level;
                    *section = new;
                    return false;
                }
                std::mem::replace(section, new)
            }
            Entry::Vacant(entry) => {
                entry.insert((scope.level, new));
                match from {
                    Some(from) => section_of(from),
                    None => return false,
                }
            }
        };
        if old == new {
            return false;
        }
        self.queue.push_back(Crossing {
            id,
            old,
            new,
            at: to,
        });
        true
    }

    /// `id` is gone (removed, or respawned as a new entity under the same id): it is filed
    /// again where it next appears, untold.
    pub(super) fn forget(&mut self, id: u32) {
        // Crossings already filed stay queued: each was a committed move (`onMove` posts at
        // the filed move, before any later removal).
        self.filed.remove(&id);
    }

    /// Claim dispatch until the queue runs out; never re-enter a listener's dispatch.
    fn begin_telling(&mut self) -> bool {
        if self.busy || self.queue.is_empty() {
            return false;
        }
        self.busy = true;
        true
    }

    fn end_telling(&mut self) {
        self.busy = false;
    }
}

#[derive(Clone, Copy)]
struct SectionWriter {
    entity: EntityRef,
    residency: EntityResidency,
    position: [f64; 3],
    player: Option<PlayerMoveState>,
}

#[derive(Clone, Copy)]
struct PlayerMoveState {
    local_player: Option<mod_api::PlayerRef>,
    on_ground: bool,
    velocity: [f64; 3],
}

/// The Host used while the entity's world view is borrowed. It answers only
/// reads from the current write and immutable startup paths; other operations
/// retain their typed refusal.
struct LiveSectionHost {
    writer: Option<SectionWriter>,
    config_root: std::sync::Arc<std::path::Path>,
}

impl LiveSectionHost {
    fn new(writer: Option<SectionWriter>) -> Self {
        Self {
            writer,
            config_root: mod_api::__private::RegistrationData::resolved_config_dir(),
        }
    }
}

mod_api::impl_host_areas!(LiveSectionHost, except(config, datafix));
impl mod_api::prelude::ConfigHost for LiveSectionHost {
    fn config_dir(&self) -> &std::path::Path {
        &self.config_root
    }
}
impl mod_api::prelude::DatafixHost for LiveSectionHost {
    fn upgrade_nbt(
        &self,
        kind: mod_api::NbtDataType,
        data_version: i32,
        nbt: mod_api::Compound,
    ) -> Result<mod_api::Compound, mod_api::NbtUpgradeError> {
        super::Game::upgrade_nbt(kind, data_version, nbt)
    }
}
mod_api::impl_server_host_areas!(LiveSectionHost);
mod_api::impl_client_host_areas!(
    LiveSectionHost,
    except(client_render, menus, network, scoreboard, services)
);
impl mod_api::prelude::ScoreboardClientHost for LiveSectionHost {
    fn client_scoreboard(
        &self,
    ) -> Result<Option<&dyn mod_api::ClientScoreboard>, mod_api::ClientSessionReadError> {
        Err(mod_api::NoBackend::no_backend())
    }
}
mod_api::impl_backend_areas!(
    LiveSectionHost,
    partial(
        entity_position,
        entity_residency,
        entity_on_ground,
        entity_velocity
    )
);

impl mod_api::prelude::NetworkClientHost for LiveSectionHost {
    fn build_play_packet(
        &self,
        request: &mod_api::PlayPacketRequest,
    ) -> Result<mod_api::ServerboundPlayPacket, mod_api::PacketBuildError> {
        Game::build_play_packet(request)
    }

    fn container_click(
        &self,
        _: &mod_api::ContainerClickRequest,
    ) -> Result<mod_api::ServerboundPlayPacket, mod_api::PacketBuildError> {
        Err(mod_api::NoBackend::no_backend())
    }
}

impl mod_api::prelude::ClientRenderClientHost for LiveSectionHost {
    fn biome_tint(
        &self,
        _: [i32; 3],
        _: mod_api::ColorResolver,
    ) -> Result<u32, mod_api::LevelReadError> {
        Err(mod_api::LevelReadError::Unavailable)
    }
}

impl mod_api::prelude::MenusClientHost for LiveSectionHost {
    fn local_player(&self) -> Option<mod_api::PlayerRef> {
        self.writer?.player?.local_player
    }
}

impl mod_api::prelude::ServicesClientHost for LiveSectionHost {
    fn client_executor(&self) -> Option<mod_api::ClientExecutor> {
        None
    }
}

// The generated read backend forwards these snapshots and answers the
// standard no-backend result for reads that would need the borrowed world.
impl LiveSectionHost {
    fn entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let writer = self.writer.filter(|writer| writer.entity == entity)?;
        Some(writer.position)
    }

    fn entity_residency(&self, entity: EntityRef) -> EntityResidency {
        self.writer
            .filter(|writer| writer.entity == entity)
            .map_or(EntityResidency::Gone, |writer| writer.residency)
    }

    fn entity_on_ground(&self, entity: EntityRef) -> Option<bool> {
        let player = self
            .writer
            .filter(|writer| writer.entity == entity)?
            .player?;
        Some(player.on_ground)
    }

    fn entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let player = self
            .writer
            .filter(|writer| writer.entity == entity)?
            .player?;
        Some(player.velocity)
    }
}

impl mod_api::Backend for LiveSectionHost {}

impl mod_api::Host for LiveSectionHost {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}

impl mod_api::ServerHost for LiveSectionHost {}
impl mod_api::ClientHost for LiveSectionHost {}

fn tell_live(sections: &std::cell::RefCell<EntitySections>, writer: SectionWriter, server: bool) {
    if !sections.borrow_mut().begin_telling() {
        return;
    }
    for crossing in std::iter::from_fn(|| sections.borrow_mut().queue.pop_front()) {
        let entity = EntityRef::new(writer.entity.epoch(), crossing.id);
        // The crossing's own placement is what a read of that entity answers.
        let at = SectionWriter {
            entity,
            position: crossing.at,
            ..if entity == writer.entity {
                writer
            } else {
                SectionWriter {
                    residency: EntityResidency::Gone,
                    player: None,
                    ..writer
                }
            }
        };
        let mut host = LiveSectionHost::new(Some(at));
        let host = if server {
            mod_api::HostSide::Server(&mut host)
        } else {
            mod_api::HostSide::Client(&mut host)
        };
        mod_api::dispatch_entity_entering_section(
            host,
            entity,
            crossing.old,
            crossing.new,
            crossing.old[0] != crossing.new[0] || crossing.old[2] != crossing.new[2],
        );
    }
    sections.borrow_mut().end_telling();
}

/// Delivers a mob's captured teleport at the write boundary.  The observer
/// owns only the section table, so it can run while the mob's borrowed world
/// view is still live.
pub(crate) struct LiveSectionObserver {
    sections: std::rc::Rc<std::cell::RefCell<EntitySections>>,
    scope: Scope,
    current: Option<(u32, EntityResidency)>,
}

impl LiveSectionObserver {
    pub(crate) fn new(
        sections: std::rc::Rc<std::cell::RefCell<EntitySections>>,
        scope: Scope,
    ) -> Self {
        Self {
            sections,
            scope,
            current: None,
        }
    }

    pub(in crate::game) fn epoch(&self) -> WorldEpoch {
        self.scope.epoch
    }

    pub(in crate::game) fn set_residency(&mut self, id: u32, residency: EntityResidency) {
        self.current = Some((id, residency));
    }
}

impl crate::mobs::living::section_writes::WriteObserver for LiveSectionObserver {
    fn position_written(
        &mut self,
        id: u32,
        from: sunrise_core::geometry::Vec3,
        to: sunrise_core::geometry::Vec3,
    ) {
        let residency = self
            .current
            .filter(|(current_id, _)| *current_id == id)
            .map_or_else(
                || {
                    if id == 0 {
                        EntityResidency::Gone
                    } else {
                        EntityResidency::Busy
                    }
                },
                |(_, residency)| residency,
            );
        self.sections
            .borrow_mut()
            .moved(self.scope, id, Some(from.components()), to.components());
        tell_live(
            &self.sections,
            SectionWriter {
                entity: EntityRef::new(self.scope.epoch, id),
                residency,
                position: to.components(),
                player: None,
            },
            true,
        );
    }
}

impl Game<'_> {
    /// Where boundaries are now: None with no listener (nothing is filed) or no world open.
    pub(super) fn entity_section_scope(&self) -> Option<Scope> {
        if !mod_api::__private::listened(mod_api::HookId::EntityEnteringSection) {
            return None;
        }
        Some(Scope {
            epoch: self.native_lifecycle.epoch()?,
            level: self.native_level_key(),
        })
    }

    /// `Callback.onMove` for the residency table: a server entity's write from `from` to `to`
    /// that leaves its section puts it at the end of the section it enters, whether or not a
    /// hook listens. Cost: the integer compare of [`crossed`]; the table is touched only on a
    /// crossing.
    pub(super) fn entity_section_entered(&mut self, id: u32, from: [f64; 3], to: [f64; 3]) {
        if crossed(from, to) && self.is_logical_server() {
            self.resident_entities
                .table
                .get_mut()
                .entered_section(id, to);
        }
    }

    /// Entity `id` was put from `from` (when known) at `to`; tell it immediately after filing.
    pub(super) fn entity_section_moved(&mut self, id: u32, from: Option<[f64; 3]>, to: [f64; 3]) {
        self.pick_section_move(id, to);
        if let Some(from) = from {
            self.entity_section_entered(id, from, to);
        }
        if let Some(scope) = self.entity_section_scope() {
            self.native_lifecycle
                .entity_sections
                .moved(scope, id, from, to);
            self.tell_entity_sections();
        }
    }

    /// Entity `id`'s position was written: it is wherever it is now (any kind, either side).
    pub(super) fn entity_section_moved_id(&mut self, id: u32) {
        self.pick_section_sync_moved_id(id);
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        if let Some(at) = self.entity_position(EntityRef::new(scope.epoch, id)) {
            self.native_lifecycle
                .entity_sections
                .moved(scope, id, None, at);
            self.tell_entity_sections();
        }
    }

    /// Entity `id` is gone, or replaced under the same id: it starts over untold.
    pub(super) fn entity_section_forget(&mut self, id: u32) {
        self.pick_section_remove(id);
        self.native_lifecycle.entity_sections.forget(id);
        self.native_lifecycle
            .entity_sections
            .projectile_live
            .remove(&id);
    }

    /// Tell the writes from one lifted mob while its owning list is resident.
    ///
    /// A mob tick cannot borrow `Game` while its world view is live, so the
    /// caller restores the pass before entering here. The resident row is
    /// temporarily moved to each recorded write while its crossing is told;
    /// this gives listeners the same position they would see at the write and
    /// preserves a listener's own position change. Returns whether callbacks
    /// wrote the mob list, excluding the temporary position staging itself.
    pub(super) fn entity_sections_ticked_live(
        &mut self,
        moves: &mut Vec<PositionWrite>,
        mut hint: Option<usize>,
    ) -> bool {
        let Some(scope) = self.entity_section_scope() else {
            moves.clear();
            return false;
        };
        let mut written = false;
        for write in moves.drain(..) {
            if write.delivered {
                continue;
            }
            let Some(index) = hint
                .filter(|&index| {
                    self.mobs
                        .get(index)
                        .is_some_and(|mob| mob.join == write.join)
                })
                .or_else(|| self.mobs.locate(write.join))
            else {
                self.entity_section_forget(write.id);
                continue;
            };
            if self.mobs[index].removed {
                self.entity_section_forget(write.id);
                continue;
            }
            hint = Some(index);
            let sections = &mut self.native_lifecycle.entity_sections;
            let crossed = sections.moved(
                scope,
                write.id,
                write.from.map(|from| from.components()),
                write.to.components(),
            );
            if !crossed || sections.busy {
                continue;
            }
            let final_position = self.mobs[index].position;
            let staged = final_position != write.to;
            if staged {
                self.mobs.update(index, |mob| mob.position = write.to);
            }
            let staged_stamp = self.mobs.stamp();
            self.tell_entity_sections();
            written |= self.mobs.stamp() != staged_stamp;
            if staged {
                let restore = if self
                    .mobs
                    .get(index)
                    .is_some_and(|mob| mob.join == write.join)
                {
                    Some(index)
                } else {
                    self.mobs.locate(write.join)
                };
                if let Some(index) = restore {
                    hint = Some(index);
                    if !self.mobs[index].removed && self.mobs[index].position == write.to {
                        self.mobs.update(index, |mob| mob.position = final_position);
                    }
                }
            }
        }
        written
    }

    /// Entity `id`'s position was written from `from` to `to` (`setPosRaw`) inside a family tick
    /// or while a rider is carried by a vehicle teleport. File and tell it at this write.
    pub(super) fn entity_section_step(&mut self, id: u32, from: [f64; 3], to: [f64; 3]) {
        if crossed(from, to) {
            self.pick_section_move(id, to);
        }
        self.entity_section_entered(id, from, to);
        if id != 0
            && let Some(scope) = self.entity_section_scope()
        {
            self.native_lifecycle
                .entity_sections
                .moved(scope, id, Some(from), to);
            self.tell_entity_sections();
        }
    }

    /// Tell a write whose row is temporarily lent to a family pass. The callback sees the
    /// committed position without removing and reinserting the surrounding list tail.
    pub(super) fn entity_section_step_visible(
        &mut self,
        id: u32,
        from: [f64; 3],
        to: [f64; 3],
    ) -> bool {
        let scope = self.entity_section_scope();
        self.native_lifecycle.entity_sections.write_position = Some((id, to));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.entity_section_step(id, from, to);
        }));
        self.native_lifecycle.entity_sections.write_position = None;
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
        scope.is_none_or(|_| {
            self.native_lifecycle
                .entity_sections
                .filed
                .contains_key(&id)
        })
    }

    /// `Entity.teleport` placed as `/tp` places it ([`Game::place_teleport_in_level`]), then filed
    /// and told as one position update (`/tp`, a mob's ender pearl). Its old position, as the
    /// placement returns it.
    pub(super) fn place_teleport_filed(
        &mut self,
        t: super::commands::Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
        turn_riders: bool,
    ) -> Option<[f64; 3]> {
        let from = self.place_teleport_in_level(t, pos, rotation, relative, turn_riders);
        if let (Some(entity), Some(to)) = (self.network_entity_id(t), self.target_position(t)) {
            self.entity_section_moved(entity as u32, from, to);
        }
        from
    }

    /// An entity joined (`addEntity`): filed untold where it is now, which a join listener may
    /// have changed from `at`, where it joined.
    pub(super) fn entity_section_joined(&mut self, id: u32, at: [f64; 3]) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        let now = self.entity_position(EntityRef::new(scope.epoch, id));
        self.native_lifecycle
            .entity_sections
            .placed(scope, id, now.unwrap_or(at));
    }

    /// The projectiles tick ([`Game::tick_projectiles_now`]), and each one's move is filed.
    pub(super) fn tick_projectiles(&mut self) -> super::Result<()> {
        self.tick_projectiles_now()
    }

    /// A client's entities after its tick: each one where its interpolation or its own move
    /// put it (its live position, not the packet's), and this window's player.
    pub(super) fn client_entity_sections_stepped(&mut self) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        let own = self.own_entity_id();
        let mut sections = std::mem::take(&mut self.native_lifecycle.entity_sections);
        if let Some(r) = self.remote.as_ref() {
            for &id in r.entities.keys().filter(|&&id| id != own && id > 0) {
                if let Some(at) = self.entity_position(EntityRef::new(scope.epoch, id as u32)) {
                    sections.moved(scope, id as u32, None, at);
                }
            }
        }
        if let Some(player) = self.player.as_ref()
            && own > 0
        {
            let at = player.position;
            sections.moved(scope, own as u32, None, [at.x, at.y, at.z]);
        }
        self.native_lifecycle.entity_sections = sections;
        self.tell_entity_sections();
    }

    /// A client handles one packet from its server ([`Game::apply_packet`]). A packet that adds,
    /// moves or removes an entity is a position-update boundary. The add handler files an
    /// admitted entity untold; a move (`handleMoveEntity`, `handleEntityPositionSync`,
    /// `handleTeleportEntity`, `handleMovePlayer`'s `PlayerPosition`, a vehicle's) is told when
    /// it put the entity in another section, and `handleRespawn`'s new player starts over.
    /// An entity a move only starts interpolating moves at the client's entity tick.
    pub(in crate::game) fn handle_packet(
        &mut self,
        packet: sunrise_protocol::packets::play::Clientbound,
    ) -> super::Result<()> {
        use sunrise_protocol::packets::play::Clientbound as C;
        if self.entity_section_scope().is_none() {
            return self.apply_packet(packet);
        }
        if let C::RemoveEntities(p) = &packet {
            for &id in &p.ids {
                self.entity_section_forget(id as u32);
            }
        }
        let respawned = matches!(packet, C::Respawn(_));
        let result = self.apply_packet(packet);
        if respawned {
            self.entity_section_forget(self.own_entity_id() as u32);
        }
        result
    }

    /// Deliver all crossings while their owning rows are available. A nested write only
    /// enqueues; this dispatch picks it up after the current callback, with the full Host.
    pub(super) fn tell_entity_sections(&mut self) {
        if !self.native_lifecycle.entity_sections.begin_telling() {
            return;
        }
        while let Some(c) = self.native_lifecycle.entity_sections.queue.pop_front() {
            if let Some(epoch) = self.native_lifecycle.epoch() {
                let chunk_changed = c.old[0] != c.new[0] || c.old[2] != c.new[2];
                // Reads of the crossing entity answer where this move put it, not where a
                // later queued move has since put it.
                let lent = self
                    .native_lifecycle
                    .entity_sections
                    .write_position
                    .replace((c.id, c.at));
                self.native_entity_entering_section(
                    EntityRef::new(epoch, c.id),
                    c.old,
                    c.new,
                    chunk_changed,
                );
                self.native_lifecycle.entity_sections.write_position = lent;
            }
        }
        self.native_lifecycle.entity_sections.end_telling();
    }
}
