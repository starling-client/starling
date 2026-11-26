//! A server's levels, one per dimension (`MinecraftServer.levels`).
//!
//! The game's tick code reads one level: the one in place, held in `Game`'s
//! own fields (the world, its streaming and save, block ticks, entities).
//! A server (the dedicated server and single player's server thread) keeps
//! the other dimensions' levels here and ticks each one with players or forced
//! chunks by swapping it into place (`enter_level`), the way `as_guest` swaps a player
//! in, so the tick code runs unchanged, one level at a time.
//!
//! Shared by every level, as vanilla's derived levels share the
//! Overworld's level data: the game time, the day time and the weather parameters
//! (not the rain and thunder interpolants), the game rules, the scoreboard,
//! the maps and the entities waiting to arrive
//! through a portal.

use super::commands::Target;
use super::entity_residency::{define_resident_access, resident_owners};
use super::{
    Dim, ExperienceOrb, FallingBlock, Game, ItemEntity, Mob, boats, commands,
    creaking::CreakingState, dragon, fireworks, lightning, machines, minecarts,
    ominous_item_spawner, persist, raids::Raiding, sculk::SculkState, tnt, trident,
};
use mod_api::DimensionKey;
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use sunrise_client::join_seq::{
    Joined,
    residency::{
        Journal, Kind, Owner,
        table::{EntityLookup, Location, SourceEntity},
    },
};
use sunrise_client::spans::time as span;
use sunrise_client::{
    Result,
    block_entity_anim::BlockEntityAnims,
    random_tick_sections::RandomTickSections,
    save::SaveDir,
    streaming::Streaming,
    tick_chunk_membership::TickChunkMembership,
    ticket_levels::{ChunkTickets, PortalTicket},
    vanillagen::VanillaTerrain,
    world::World,
};
use sunrise_world::level::{LevelEvent, Ticks};
#[path = "levels/starling.rs"]
mod starling;
#[allow(unused_imports)]
pub use self::starling::*;

/// A borrowed counting view: no resident-table scans and no allocation for gauges.
pub(super) struct EntityCountView<'a> {
    pub(super) fixed: [(&'static str, usize); 19],
    pub(super) mobs: &'a [Mob],
    pub(super) projectiles: &'a [sunrise_client::projectiles::Projectile],
    pub(super) general: &'a super::general::GeneralRows,
    pub(super) others: &'a [commands::OtherEntity],
}
impl EntityCountView<'_> {
    pub(super) fn total(&self) -> usize {
        self.fixed.iter().map(|(_, n)| n).sum::<usize>()
            + self.projectiles.len()
            + self.mobs.len()
            + self.general.len()
            + self.others.len()
    }
    pub(super) fn registry_counts(&self) -> Vec<(String, u32)> {
        let mut counts = std::collections::BTreeMap::<String, u32>::new();
        for &(id, n) in &self.fixed {
            if n > 0 {
                *counts.entry(id.to_owned()).or_default() += n as u32;
            }
        }
        for projectile in self.projectiles {
            *counts.entry(projectile.kind.id().to_owned()).or_default() += 1;
        }
        for mob in self.mobs.iter().filter(|mob| !mob.removed) {
            *counts.entry(mob.kind.id.clone()).or_default() += 1;
        }
        for row in self.general.iter() {
            *counts
                .entry(
                    super::general::general_type_catalog()
                        .key(row.type_slot)
                        .to_owned(),
                )
                .or_default() += 1;
        }
        for row in self.others {
            *counts.entry(row.kind.clone()).or_default() += 1;
        }
        counts.into_iter().collect()
    }
}

// Both Game and Parked own these typed families. Keep their instrumentation in one list.
macro_rules! entity_count_view {
    ($level:expr, $projectiles:expr, $general:expr, $knots:expr) => {
        crate::game::levels::EntityCountView {
            fixed: [
                ("minecraft:item", $level.dropped.len()),
                ("minecraft:experience_orb", $level.orbs.len()),
                ("minecraft:minecart", $level.carts.len()),
                ("minecraft:boat", $level.boats.len()),
                ("minecraft:falling_block", $level.falling.len()),
                ("minecraft:painting", $level.paintings.len()),
                ("minecraft:item_frame", $level.item_frames.len()),
                ("minecraft:armor_stand", $level.armor_stands.len()),
                ("minecraft:trident", $level.tridents.len()),
                ("minecraft:firework_rocket", $level.rockets.len()),
                ("minecraft:tnt", $level.tnts.len()),
                ("minecraft:end_crystal", $level.crystals.len()),
                ("minecraft:leash_knot", $knots.len()),
                (
                    "minecraft:ominous_item_spawner",
                    $level.ominous_spawners.len(),
                ),
                ("minecraft:cushion", $level.cushions.len()),
                ("minecraft:area_effect_cloud", $level.breath_clouds.len()),
                ("minecraft:lightning_bolt", $level.bolts.len()),
                ("minecraft:evoker_fangs", $level.fangs.perf_count()),
                // Multipart hitboxes are not independently admitted world entities.
                (
                    "minecraft:ender_dragon",
                    usize::from($level.dragon.as_ref().is_some()),
                ),
            ],
            mobs: &$level.mobs,
            projectiles: $projectiles,
            general: $general,
            others: &$level.others,
        }
    };
}
pub(super) use entity_count_view;

/// Vanilla 26.3 only services connections while the server is paused (escape
/// pause or `pause-when-empty-seconds`), so parked-level eviction is skipped
/// then, while still running during `/tick freeze` (`paused == false`).
fn should_run_idle_maintenance(paused: bool) -> bool {
    !paused
}

/// A level that is not in place: every per-level field of `Game`.
pub(super) struct Parked {
    pub(super) world: World,
    pub(super) streaming: Streaming,
    locator: Option<Arc<VanillaTerrain>>,
    pub(super) save: Arc<SaveDir>,
    block_ticks: Ticks,
    tick_chunks: TickChunkMembership,
    tickets: ChunkTickets,
    random_tick_sections: RandomTickSections,
    level_events: Vec<LevelEvent>,
    /// Entity portal processes and cooldowns belong to this level's entities.
    pub(super) portal_states: HashMap<u32, sunrise_client::nether_portal::PortalState>,
    falling: sunrise_client::join_seq::JoinList<FallingBlock, false>,
    paintings: sunrise_client::join_seq::JoinList<sunrise_client::entities::Painting>,
    item_frames: sunrise_client::join_seq::JoinList<sunrise_client::entities::ItemFrame>,
    armor_stands: sunrise_client::join_seq::JoinList<sunrise_client::entities::ArmorStand>,
    cushions: sunrise_client::join_seq::JoinList<sunrise_client::entities::Cushion>,
    others: sunrise_client::join_seq::ResidentList<commands::OtherEntity>,
    fangs: super::fangs::FangState,
    pub(super) boats: sunrise_client::join_seq::JoinList<boats::Boat, false>,
    pub(super) carts: sunrise_client::join_seq::JoinList<minecarts::Minecart, false>,
    jukebox_sounds: HashMap<(i32, i32, i32), u64>,
    bolts: sunrise_client::join_seq::ResidentList<lightning::Bolt>,
    crystals: sunrise_client::join_seq::JoinList<dragon::Crystal>,
    dragon: super::native_entity_refs::dragon_parts_table::DragonSlot,
    breath_clouds: sunrise_client::join_seq::ResidentList<dragon::Breath>,
    tridents: sunrise_client::join_seq::JoinList<trident::ThrownTrident>,
    rockets: sunrise_client::join_seq::JoinList<fireworks::Rocket>,
    ominous_spawners:
        sunrise_client::join_seq::ResidentList<ominous_item_spawner::OminousItemSpawner>,
    tnts: sunrise_client::join_seq::JoinList<tnt::PrimedTnt>,
    general: super::general::GeneralStore,
    hoppers: HashMap<(i32, i32, i32), machines::HopperState>,
    pub(super) mobs: sunrise_client::join_seq::JoinList<Mob>,
    /// The entity ids of the level's fence knots (`knot_ids`).
    knot_ids: HashMap<[i32; 3], u32>,
    broken_knots: Vec<[i32; 3]>,
    knots: sunrise_client::join_seq::JoinList<super::leads::Knot>,
    projectiles: sunrise_client::join_seq::JoinList<sunrise_client::projectiles::Projectile, false>,
    dropped: sunrise_client::join_seq::JoinList<ItemEntity>,
    pub(super) orbs: sunrise_client::join_seq::JoinList<ExperienceOrb>,
    persist: persist::EntityMemory,
    lectern_pulses: Vec<((i32, i32, i32), i32)>,
    lid_openers: HashMap<(i32, i32, i32), i32>,
    lan_events: Vec<super::QueuedLevelEvent>,
    lan_block_events: Vec<([i32; 3], i32, i32, u32)>,
    lan_entity_events: Vec<(i32, i8)>,
    lan_critical_hits: Vec<(Option<u32>, i32, bool)>,
    /// The hub's tracked entities of this level (`LanHost::states`), as its last tick described
    /// them: what its players' moves are checked against before this tick describes them again.
    lan_states: sunrise_client::lan::entity::States,
    /// Only the per-level parts of these are used (`swap_level`).
    anims: BlockEntityAnims,
    pub(super) raiding: Raiding,
    sculk: SculkState,
    creaking: CreakingState,
    chunk_spawn_enemies: bool,
    /// Natural spawning's candidate cache: each level keeps its own players' squares.
    spawn_scratch: super::natural_spawning::SpawnScratch,
    /// The level's own world border (`ServerLevel.getWorldBorder`).
    pub(in crate::game) border: sunrise_client::worldborder::WorldBorder,
}

impl Parked {
    /// The exact body restored by a cross-level transfer during natural-spawn admission.
    pub(super) fn natural_spawn_mob(
        &self,
        id: u32,
        join: sunrise_client::join_seq::JoinSeq,
    ) -> Option<(&Mob, Option<mod_api::EntityTags<'_>>)> {
        let mob = if join.is_joined() {
            &self.mobs[self.mobs.locate(join)?]
        } else {
            self.persist.off_level.get(&id)?.as_mob()?
        };
        let tags = match self.persist.raw.get(&id).and_then(|tag| tag.get("Tags")) {
            Some(sunrise_core::nbt::Tag::List(tags)) => tags.as_slice(),
            _ => &[],
        };
        Some((
            mob,
            Some(mod_api::__private::game::borrow_entity_tags(tags)),
        ))
    }

    /// Whether `obstructs` returns `true` for any live `blocksBuilding` entity in this parked
    /// level (`EntityGetter.isUnobstructed`), without allocating.
    pub(super) fn any_building_blocker(
        &self,
        mut obstructs: impl FnMut(sunrise_core::geometry::Aabb) -> bool,
    ) -> bool {
        if self
            .mobs
            .iter()
            .any(|m| !m.removed && obstructs(m.bounds()))
            || self
                .dragon
                .as_ref()
                .is_some_and(|dragon| obstructs(dragon.body_bounds()))
        {
            return true;
        }
        let entities = crate::game::entity::EntityLists {
            mobs: &self.mobs,
            dropped: &self.dropped,
            boats: &self.boats,
            carts: &self.carts,
            armor_stands: &self.armor_stands,
            tnts: &self.tnts,
            falling: &self.falling,
            orbs: &self.orbs,
            projectiles: &self.projectiles,
            rockets: &self.rockets,
            tridents: &self.tridents,
            general: &self.general.rows,
        };
        entities
            .non_mob_building_blocker_boxes(
                self.carts.iter().map(minecarts::Minecart::bounds),
                &self.crystals,
            )
            .any(obstructs)
    }

    resident_owners!(
        define_resident_access,
        self,
        projectiles,
        knots,
        general.rows
    );
}

impl Game<'_> {
    /// Resolve through the exact owner, never by trying dimension swaps.
    pub(super) fn locate_server_resident_owner(
        &self,
        at: Location,
    ) -> Option<SourceEntity<(Target, bool)>> {
        if matches!(
            at.owner.kind,
            sunrise_client::join_seq::residency::Kind::Player
                | sunrise_client::join_seq::residency::Kind::Hook
        ) {
            return self
                .locate_specialized_resident_owner(at)
                .map(|mut source| {
                    source.value.1 &=
                        self.resident_source_target_visible(at.owner.dimension, source.value.0);
                    source
                });
        }
        if at.owner.dimension == self.dim.key() {
            // An open transaction's removal reads as gone already.
            return self.locate_resident_owner(at).map(|mut source| {
                source.value.1 &= !self.removal_pending(&source.value.0);
                source
            });
        }
        let dim = match at.owner.dimension {
            DimensionKey::OVERWORLD => Dim::Overworld,
            DimensionKey::NETHER => Dim::Nether,
            DimensionKey::END => Dim::End,
            key => Dim::Extra(key),
        };
        // The transactions stay with the game when a level is parked: its pending removals too.
        self.levels
            .parked
            .get(&dim)?
            .locate_resident_owner(at)
            .map(|mut source| {
                source.value.1 &= !self.removal_pending_in(dim, &source.value.0);
                source
            })
    }

    /// Whether `target` counts as removed in the level in place: a mod removed it, or an open
    /// transaction did and its root commit has not run the removal yet.
    pub(in crate::game) fn target_removed(&self, target: &Target) -> bool {
        self.persist.removals.contains(target)
            || self.persist.destroying_vehicles.contains(target)
            || self.removal_pending(target)
    }

    pub(in crate::game) fn resident_source_target_visible(
        &self,
        dimension: DimensionKey,
        target: Target,
    ) -> bool {
        let player = match target {
            Target::Guest(conn) => Some(conn),
            Target::Player => self.lan_acting,
            _ => None,
        };
        if player
            .and_then(|conn| self.command_guest(conn))
            .is_some_and(|guest| !guest.body_present())
        {
            return false;
        }
        if dimension == self.dim.key() {
            return !self.target_removed(&target);
        }
        let dim = match dimension {
            DimensionKey::OVERWORLD => Dim::Overworld,
            DimensionKey::NETHER => Dim::Nether,
            DimensionKey::END => Dim::End,
            key => Dim::Extra(key),
        };
        self.levels.parked.get(&dim).is_none_or(|parked| {
            !parked.persist.removals.contains(&target)
                && !parked.persist.destroying_vehicles.contains(&target)
                && !self.removal_pending_in(dim, &target)
        })
    }

    /// Visit exactly the actual physical lifetime slot. Resolution releases the
    /// table and its journal before lending storage; no lift, swap or replacement
    /// occurs, including when the visitor unwinds. Logical visibility is a caller
    /// policy, just as for the underlying owner resolver. The visitor must not
    /// call entity lookup or recursively enter this seam.
    pub(super) fn with_resident_slot<R>(
        &mut self,
        dimension: &mod_api::DimensionKey,
        id: std::num::NonZeroU32,
        visit: impl FnOnce(&mut sunrise_mod_runtime::attachments::AttachmentLifetime) -> R,
    ) -> super::entity_residency::SlotAccess<R> {
        use super::entity_residency::SlotAccess;
        let resolved = {
            self.resident_entities
                .table
                .borrow_mut()
                .entity_lookup(dimension, id.get(), |at| {
                    self.locate_server_resident_owner(at)
                })
        };
        let location = match resolved {
            EntityLookup::Found { location, .. } => location,
            EntityLookup::Taken => return SlotAccess::Taken,
            EntityLookup::Missing => return SlotAccess::Missing,
        };
        debug_assert!(self.resident_entities.table.try_borrow_mut().is_ok());
        if matches!(
            location.owner.kind,
            sunrise_client::join_seq::residency::Kind::Player
                | sunrise_client::join_seq::residency::Kind::Hook
        ) {
            if *dimension != self.dim.key() {
                let dim = match *dimension {
                    DimensionKey::OVERWORLD => Dim::Overworld,
                    DimensionKey::NETHER => Dim::Nether,
                    DimensionKey::END => Dim::End,
                    key => Dim::Extra(key),
                };
                if !self.levels.parked.contains_key(&dim) {
                    return SlotAccess::Missing;
                }
            }
            return self.with_specialized_resident_slot(location, visit);
        }
        if *dimension == self.dim.key() {
            self.with_resident_owner_slot(location, visit)
        } else {
            let dim = match *dimension {
                mod_api::DimensionKey::OVERWORLD => Dim::Overworld,
                mod_api::DimensionKey::NETHER => Dim::Nether,
                mod_api::DimensionKey::END => Dim::End,
                key => Dim::Extra(key),
            };
            self.levels
                .parked
                .get_mut(&dim)
                .map_or(SlotAccess::Missing, |parked| {
                    parked.with_resident_owner_slot(location, visit)
                })
        }
    }

    /// Whether the living entity `id` resident in a parked level is alive (`Entity.isAlive`):
    /// a remembered target stays remembered while its holder is in another level.
    pub(in crate::game) fn parked_living_alive(&self, id: u32) -> bool {
        let EntityLookup::Found { location, value } = self.server_resident_entity(id) else {
            return false;
        };
        let Some(parked) = (self.levels.parked.iter())
            .find(|(dim, _)| dim.key() == location.owner.dimension)
            .map(|(_, parked)| parked)
        else {
            return false;
        };
        match value {
            Target::Mob(_) => {
                (parked.mobs.locate(location.join)).is_some_and(|i| parked.mobs[i].alive())
            }
            Target::Stand(_) => (parked.armor_stands.locate(location.join))
                .is_some_and(|i| parked.armor_stands[i].health > 0.0),
            Target::Dragon(id) => {
                (parked.dragon.as_ref()).is_some_and(|d| d.id == id && d.health > 0.0)
            }
            _ => false,
        }
    }

    pub(in crate::game) fn server_resident_entity(&self, id: u32) -> EntityLookup<Target> {
        let found = self
            .resident_entities
            .table
            .borrow_mut()
            .server_entity_lookup(id, |at| self.locate_server_resident_owner(at));
        match found {
            EntityLookup::Found {
                location,
                value: (value, true),
            } => EntityLookup::Found { location, value },
            EntityLookup::Found {
                value: (_, false), ..
            }
            | EntityLookup::Missing => EntityLookup::Missing,
            EntityLookup::Taken => EntityLookup::Taken,
        }
    }
    /// `ServerLevel.getEntity(UUID)`: search this level's admitted entities, not unloaded
    /// saves or tracking entries. A departure can overlap a replacement with the same UUID.
    pub(super) fn level_target_by_uuid(&self, dim: Dim, uuid: [i32; 4]) -> Option<Target> {
        let dimension = dim.key();
        let mut table = self.resident_entities.table.borrow_mut();
        let mut after = None;
        loop {
            let id = table.level_uuid_candidate(dimension, uuid, after)?;
            if let EntityLookup::Found {
                value: (target, true),
                ..
            } = table.entity_lookup(&dimension, id.get(), |at| {
                self.locate_server_resident_owner(at)
            }) && self.resident_source_target_visible(dimension, target)
            {
                return Some(target);
            }
            after = Some(id);
        }
    }

    /// `PersistentEntitySectionManager.isLoaded(UUID)` for the destination level in place,
    /// including the in-place player and any guests in this level, independently of command visibility.
    pub(in crate::game) fn is_loaded_uuid_in_level(&self, uuid: [i32; 4]) -> bool {
        if self.level_target_by_uuid(self.dim, uuid).is_some() {
            return true;
        }
        if self.resident_entities.local_player.location().is_none()
            && self.lan_acting.is_none()
            && self.player.is_some()
            && self.level.player_uuid == Some(uuid)
        {
            return true;
        }
        self.lan.iter().flat_map(|h| h.hub.guests.iter()).any(|g| {
            g.placed
                && !g.released
                && Dim::from_id(&g.dimension) == self.dim
                && sunrise_client::lan::playerdata::uuid_ints(g.uuid) == uuid
        })
    }

    /// Find the canonical level without swapping or probing physical families.
    pub(super) fn resident_target_dimension(&self, target: Target) -> Option<Dim> {
        let id = self.resident_target_id(target)?;
        let found = match self.server_resident_entity(id) {
            EntityLookup::Found { location, value }
                if value == target
                    || matches!(
                        (value, target),
                        (Target::Player, Target::Guest(_)) | (Target::Guest(_), Target::Player)
                    ) =>
            {
                location
            }
            _ => return None,
        };
        Some(match found.owner.dimension {
            DimensionKey::OVERWORLD => Dim::Overworld,
            DimensionKey::NETHER => Dim::Nether,
            DimensionKey::END => Dim::End,
            key => Dim::Extra(key),
        })
    }

    /// A new epoch invalidates every old producer before rebinding retained levels.
    pub(super) fn reset_entity_residency(&mut self) {
        let table = self.resident_entities.table.get_mut();
        table.reset();
        let journal = table.producer();
        self.bind_resident_owners(&journal, self.dim.key(), self.is_logical_server());
        self.bind_specialized_resident_owners(&journal);
        for (dim, parked) in &mut self.levels.parked {
            parked.bind_resident_owners(&journal, dim.key(), true);
        }
    }
}

/// Which passes of the server tick the level in place runs: the first
/// reads the network (`Hub::poll`), the last ends the tick for every player
/// (`Hub::end_tick`). A tick with one level is both. `running` is the
/// tick rate manager's answer for this server tick, asked once before the
/// levels (`MinecraftServer.tickServer`); `None` outside `tick_server`.
#[derive(Clone, Copy)]
pub(super) struct Pass {
    pub first: bool,
    pub last: bool,
    pub running: Option<bool>,
    /// Set once this level's player packets were handled before block ticks.
    pub packets_drained: bool,
}

impl Default for Pass {
    fn default() -> Self {
        Self {
            first: true,
            last: true,
            running: None,
            packets_drained: false,
        }
    }
}

pub(in crate::game) enum ServerTickPhase {
    Level(usize, Dim),
    IdleLevel(Dim),
    Connections,
}

/// `MinecraftServer.tickChildren`: visit levels in server order, then tick connections.
/// Only the active subset needs its chunks and entities simulated.
pub(in crate::game) fn server_tick_phases<'a>(
    dimensions: &'a [Dim],
    active: &'a [Dim],
) -> impl Iterator<Item = ServerTickPhase> + 'a {
    let mut active = active.iter().copied().enumerate().peekable();
    dimensions
        .iter()
        .copied()
        .map(move |dim| match active.next_if(|(_, next)| *next == dim) {
            Some((index, _)) => ServerTickPhase::Level(index, dim),
            None => ServerTickPhase::IdleLevel(dim),
        })
        .chain(std::iter::once(ServerTickPhase::Connections))
}

/// How long a server tick took and what it spent the time in.
pub(super) struct TickTiming {
    pub wall_ms: f32,
    pub cpu_ms: Option<f32>,
    /// Spans recorded inside the tick only, in observation order.
    pub raw: Vec<sunrise_client::spans::Span>,
}

/// The levels not in place, by dimension.
#[derive(Default)]
pub(super) struct Levels {
    pub(super) parked: HashMap<Dim, Parked>,
    /// Loaded server levels in creation order (`MinecraftServer.levels`'s
    /// insertion-ordered `LinkedHashMap`), shared by tick, save, close and Host
    /// enumeration regardless of which level is swapped into `Game`.
    order: Vec<Dim>,
    /// A tick holds its eligible levels across swaps and callbacks; reuse the allocation.
    tick_order: Vec<Dim>,
    /// Whether the level held in `Game`'s own fields has been closed by server
    /// shutdown (`close_loaded_levels_for_stop`).
    pub(in crate::game) active_closed: bool,
    /// The levels a mod asked to have saved (`level_request_save`), each once, saved at the start
    /// of the next server tick.
    requested_saves: Vec<Dim>,
    /// The levels mods opened while the server runs, and the requests waiting to be applied.
    pub(super) runtime: super::runtime_levels::RuntimeTable,
    /// The entities that travelled to another level lately: the id each had, to the id it
    /// arrived with and when. A kill credit recorded under the old id follows it there.
    travels: HashMap<u32, (u32, i64)>,
}

impl Levels {
    /// Note that the entity `from` arrived in another level as `to` at `now`. A kill credit
    /// lasts a hundred ticks, so a travel older than that names nothing still credited.
    pub(in crate::game) fn note_travel(&mut self, from: u32, to: u32, now: i64) {
        self.travels.retain(|_, &mut (_, at)| now - at <= 100);
        self.travels.insert(from, (to, now));
    }

    /// The id the entity `from` arrived with in another level, if it travelled lately.
    pub(in crate::game) fn travelled(&self, from: u32) -> Option<u32> {
        self.travels.get(&from).map(|&(to, _)| to)
    }

    /// Queue `dim` to be saved at the start of the next server tick; a level already queued is
    /// not queued twice.
    pub(in crate::game) fn request_save(&mut self, dim: Dim) {
        if !self.requested_saves.contains(&dim) {
            self.requested_saves.push(dim);
        }
    }

    /// Whether a save of `dim` is still waiting for a tick that admits
    /// saves (the #2488 gate holds it back).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::game) fn save_pending(&self, dim: Dim) -> bool {
        self.requested_saves.contains(&dim)
    }

    /// Drop a save of `dim` still waiting for a tick: its level is gone.
    pub(super) fn cancel_requested_save(&mut self, dim: Dim) {
        self.requested_saves.retain(|saved| *saved != dim);
    }

    pub(in crate::game) fn has_loaded_levels(&self) -> bool {
        !self.order.is_empty()
    }

    /// Clear every parked level and dimensions recorded as heard open.
    pub(super) fn clear(&mut self) {
        self.parked.clear();
        self.order.clear();
        self.tick_order.clear();
        self.active_closed = false;
        self.requested_saves.clear();
        self.runtime.clear();
    }

    pub(super) fn abort_parked_generation(&mut self) {
        for level in self.parked.values_mut() {
            level.streaming.abort_generation();
        }
    }

    /// A parked level, without opening or loading it.
    pub(super) fn parked_world(&self, dim: Dim) -> Option<&World> {
        self.parked.get(&dim).map(|level| &level.world)
    }

    /// The world of the level not in place whose dimension is `dimension`
    /// (`minecraft:the_nether`), found without collecting the parked levels.
    pub(super) fn parked_world_of(&self, dimension: &str) -> Option<&World> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &level.world)
    }

    /// Block ticks belonging to an open level that is not currently in place.
    pub(in crate::game) fn parked_block_ticks_of(&self, dimension: &str) -> Option<&Ticks> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &level.block_ticks)
    }

    pub(in crate::game) fn parked_lid_openers_of(
        &self,
        dimension: &str,
    ) -> Option<&HashMap<(i32, i32, i32), i32>> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &level.lid_openers)
    }

    /// The world of the level not in place whose dimension is `dimension`, to
    /// change.
    pub(super) fn parked_world_mut_of(&mut self, dimension: &str) -> Option<&mut World> {
        self.parked
            .iter_mut()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &mut level.world)
    }

    /// The streaming of the level not in place whose dimension is `dimension`.
    pub(super) fn parked_streaming_of(&self, dimension: &str) -> Option<&Streaming> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &level.streaming)
    }

    /// A parked level's existing generator query worker, without switching levels.
    pub(super) fn parked_locator_of(&self, dimension: &str) -> Option<Arc<VanillaTerrain>> {
        self.parked_of(dimension)?.locator.clone()
    }

    /// The streaming of the level not in place whose dimension is `dimension`, to change.
    pub(super) fn parked_streaming_of_mut(&mut self, dimension: &str) -> Option<&mut Streaming> {
        self.parked
            .iter_mut()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &mut level.streaming)
    }

    /// `ServerChunkCache.tick` of the parked levels that `tick` names: their `UNKNOWN` tickets
    /// purged (`TicketStorage.purgeStaleTickets`) when `purge`, then their recent FULL lookups
    /// forgotten (`clearCache`).
    pub(super) fn tick_parked_chunk_caches(&mut self, purge: bool, tick: impl Fn(Dim) -> bool) {
        for (&dim, level) in &mut self.parked {
            if tick(dim) {
                if purge {
                    level.streaming.purge_unknown_tickets();
                }
                level.streaming.clear_full_lookups();
            }
        }
    }

    /// `ServerLevel.setChunkForced`: force a column in an already open level without changing the acting
    /// player's level; the same queue and load barrier serve the active level.
    pub(super) fn force_parked_chunk(
        &mut self,
        dimension: &str,
        x: i32,
        z: i32,
        forced: bool,
    ) -> Option<bool> {
        let level = self.parked_mut_of(dimension)?;
        Some(
            level
                .streaming
                .set_forced_loaded(&mut level.world, x, z, forced),
        )
    }

    pub(super) fn parked_anims(&self, dim: Dim) -> Option<&BlockEntityAnims> {
        self.parked.get(&dim).map(|level| &level.anims)
    }

    /// The worlds of the levels not in place.
    /// A parked level's world border.
    pub(super) fn parked_border(
        &self,
        dim: Dim,
    ) -> Option<&sunrise_client::worldborder::WorldBorder> {
        self.parked.get(&dim).map(|level| &level.border)
    }

    pub(super) fn install_profession_save_snapshots(
        &self,
        table: Arc<sunrise_client::mobs::villager::ResolvedProfessions>,
    ) {
        for level in self.parked.values() {
            level.save.install_profession_snapshot(table.clone());
        }
    }

    /// The generators of the parked levels, for the world snapshots their workers read.
    pub(in crate::game) fn parked_terrains(&self) -> impl Iterator<Item = &Arc<VanillaTerrain>> {
        self.parked
            .values()
            .filter_map(|level| level.locator.as_ref())
    }

    pub(super) fn parked_worlds(&self) -> impl Iterator<Item = &World> {
        self.parked.values().map(|level| &level.world)
    }

    /// The streamers of the levels not in place.
    pub(super) fn parked_streamings(
        &self,
    ) -> impl Iterator<Item = &sunrise_client::streaming::Streaming> {
        self.parked.values().map(|level| &level.streaming)
    }

    /// The level not in place, in `dim`.
    pub(super) fn parked_mut(&mut self, dim: Dim) -> Option<&mut Parked> {
        self.parked.get_mut(&dim)
    }

    /// A parked level by its exact dimension identity, without swapping it in.
    pub(super) fn parked_of(&self, dimension: &str) -> Option<&Parked> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| level)
    }
    pub(super) fn parked_mut_of(&mut self, dimension: &str) -> Option<&mut Parked> {
        self.parked
            .iter_mut()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| level)
    }

    /// Whether a parked level's general rows hold any of these UUIDs (#792's
    /// dimension-transfer check: a destination may not already hold a row
    /// of the travelling tree's UUIDs).
    pub(in crate::game) fn parked_general_holds_any_uuid(
        &self,
        dim: Dim,
        uuids: &std::collections::HashSet<[i32; 4]>,
    ) -> bool {
        self.parked.get(&dim).is_some_and(|parked| {
            parked
                .general
                .rows
                .iter()
                .any(|row| uuids.contains(&row.uuid))
        })
    }

    /// The entities of a parked level, counted as `Game::entity_count` counts the one in place.
    pub(super) fn parked_entity_count(&self, dim: Dim) -> Option<usize> {
        let l = self.parked.get(&dim)?;
        Some(
            l.mobs.iter().filter(|m| !m.removed).count()
                + l.dropped.len()
                + l.orbs.len()
                + l.projectiles.len()
                + l.carts.len()
                + l.boats.len()
                + l.falling.len()
                + l.paintings.len()
                + l.item_frames.len()
                + l.armor_stands.len()
                + l.cushions.len()
                + l.tridents.len()
                + l.rockets.len()
                + l.tnts.len(),
        )
    }

    /// The mobs of a parked level.
    pub(in crate::game) fn parked_mobs(&self, dim: Dim) -> Option<&[Mob]> {
        self.parked.get(&dim).map(|level| &*level.mobs)
    }

    pub(super) fn parked_perf_views(
        &self,
    ) -> impl Iterator<Item = (Dim, &World, EntityCountView<'_>)> {
        self.parked.iter().map(|(&dim, level)| {
            (
                dim,
                &level.world,
                entity_count_view!(level, &level.projectiles, &level.general.rows, &level.knots),
            )
        })
    }

    /// A parked level's loose entities, for `sunrise-report`: items, experience orbs,
    /// projectiles, minecarts and boats.
    pub(super) fn parked_entity_counts(&self, dim: Dim) -> Option<[usize; 5]> {
        self.parked.get(&dim).map(|l| {
            [
                l.dropped.len(),
                l.orbs.len(),
                l.projectiles.len(),
                l.carts.len(),
                l.boats.len(),
            ]
        })
    }

    /// The levels that are not in place, by dimension.
    pub(super) fn parked_dims(&self) -> Vec<Dim> {
        self.parked.keys().copied().collect()
    }
}

impl Game<'_> {
    /// A new level for `dim`, opened from the save (`MinecraftServer.createLevels`).
    pub(super) fn open_level(&self, dim: Dim) -> Result<Parked> {
        // A level a mod opened has the seed and generator it asked for.
        let (generate, seed) = match self.runtime_terrain(dim) {
            Some((generate, seed)) => (Some(generate), seed),
            None => (None, self.level.seed),
        };
        let biomes = self
            .world_biomes
            .as_ref()
            .unwrap_or_else(|| self.res.biomes());
        let overrides = match &self.world_tables {
            Some(tables) => tables.biomes.overrides.clone(),
            None => {
                let base_conditions = self.worldgen_base_conditions()?;
                let mut enabled: Vec<_> = self.fns.enabled_packs_highest_first().collect();
                enabled.reverse();
                let conditions = super::load_conditions::worldgen_load_conditions(
                    &base_conditions,
                    &self.options.resources,
                    &enabled,
                );
                super::world_loading::worldgen_overrides_of(&self.fns.data, &conditions)?.0
            }
        };
        let (streaming, save, locator, generator_type) = super::world_loading::make_streaming(
            &self.res,
            &self.pack,
            &self.options,
            &self.settings,
            &self.world_root,
            dim,
            self.level_shapes.get(dim.id()),
            seed,
            &self.level,
            biomes.clone(),
            &overrides,
            generate,
            self.save.chunk_reports().clone(),
            self.save.save_failure_reporter().clone(),
        )?;
        self.bind_chunk_disk_worker(&save, dim);
        save.install_profession_snapshot(self.stations().professions.clone());
        if let Some(terrain) = &locator {
            terrain.set_mob_maps(self.mob_map_snapshot());
            terrain.set_biome_keys(&self.biome_key_snapshot());
        }
        let mut world = Game::authoritative_world_for_mods(&self.res, seed);
        world.attachments = save.load_attachments();
        world.shape = self.level_shapes.get(dim.id());
        world.weather.copy_parameters_from(self.world.weather);
        // ServerLevel.prepareWeather seeds a new level's current strengths from
        // WeatherData, never from another level's interpolants.
        if world.shape.can_have_weather(dim == Dim::End) && world.weather.raining {
            world.weather.rain_level.1 = 1.0;
            if world.weather.thundering {
                world.weather.thunder_level.1 = 1.0;
            }
        }
        world.generator_type = Some(generator_type);
        world.fast_lava = self.dimension_type_of(dim).fast_lava;
        world.infiniburn = Some(self.resolve_level_infiniburn(self.dimension_type_of(dim)));
        world.golem_heads = Some(self.resolve_golem_heads());
        Ok(Parked {
            world,
            streaming,
            locator,
            save,
            block_ticks: Ticks::new(seed as i32),
            tick_chunks: Default::default(),
            tickets: Default::default(),
            random_tick_sections: Default::default(),
            level_events: Vec::new(),
            portal_states: HashMap::new(),
            falling: Default::default(),
            paintings: Default::default(),
            item_frames: Default::default(),
            armor_stands: Default::default(),
            cushions: Default::default(),
            others: Default::default(),
            fangs: Default::default(),
            boats: Default::default(),
            carts: Default::default(),
            jukebox_sounds: HashMap::new(),
            bolts: Default::default(),
            crystals: Default::default(),
            dragon: Default::default(),
            breath_clouds: Default::default(),
            tridents: Default::default(),
            rockets: Default::default(),
            ominous_spawners: Default::default(),
            tnts: Default::default(),
            general: Default::default(),
            hoppers: HashMap::new(),
            mobs: Default::default(),
            knot_ids: HashMap::new(),
            broken_knots: Vec::new(),
            knots: Default::default(),
            projectiles: Default::default(),
            dropped: Default::default(),
            orbs: Default::default(),
            persist: Default::default(),
            lectern_pulses: Vec::new(),
            lid_openers: HashMap::new(),
            lan_events: Vec::new(),
            lan_block_events: Vec::new(),
            lan_entity_events: Vec::new(),
            lan_critical_hits: Vec::new(),
            lan_states: Default::default(),
            anims: Default::default(),
            raiding: Default::default(),
            sculk: Default::default(),
            creaking: Default::default(),
            chunk_spawn_enemies: true,
            spawn_scratch: Default::default(),
            border: self.load_border(dim),
        })
    }

    /// Every loaded server level in `MinecraftServer.levels` creation order.
    pub(in crate::game) fn loaded_level_order(&self) -> Vec<Dim> {
        self.level_creation_order().into_owned()
    }

    fn level_creation_order(&self) -> Cow<'_, [Dim]> {
        #[cfg(test)]
        if self.levels.order.len()
            != self.levels.parked.len() + usize::from(!self.levels.active_closed)
        {
            // Isolated Game fixtures can populate level storage without running createLevels.
            let mut dims = self.levels.order.clone();
            let mut unannounced: Vec<_> = (!self.levels.active_closed)
                .then_some(self.dim)
                .into_iter()
                .chain(self.levels.parked.keys().copied())
                .filter(|dim| !dims.contains(dim))
                .collect();
            unannounced.sort_by_key(|dim| dim_fallback_key(*dim));
            dims.extend(unannounced);
            return Cow::Owned(dims);
        }
        Cow::Borrowed(&self.levels.order)
    }

    /// `MinecraftServer.createLevels`: open every configured level from the
    /// `LEVEL_STEM` registry in order before `prepare_levels`, which restores
    /// and waits for their saved tickets.
    pub(super) fn open_configured_levels(&mut self, configured: &[Dim]) -> Result<()> {
        if self.options.execution_role.is_client() {
            return Ok(());
        }
        let starting_dim = self.dim;
        // World preparation can build the owner's starting level before the
        // Overworld. Keep it out of both the current and parked slots until
        // its registry turn, so earlier `level_load` listeners cannot see it.
        let mut starting_level = if starting_dim != Dim::Overworld {
            let mut overworld = self.open_level(Dim::Overworld)?;
            self.replace_level(Dim::Overworld, &mut overworld);
            self.apply_settings();
            self.read_raids();
            Some(overworld)
        } else {
            None
        };
        self.native_level_opened(Dim::Overworld);
        self.initialize_overworld_spawn()?;
        for dim in configured
            .iter()
            .copied()
            .filter(|&dim| dim != Dim::Overworld)
        {
            if dim == starting_dim
                && let Some(level) = starting_level.take()
            {
                self.levels.parked.insert(dim, level);
                self.enter_level(dim)?;
            } else {
                let mut level = self.open_level(dim)?;
                let left = self.replace_level(dim, &mut level);
                self.levels.parked.insert(left, level);
                self.apply_settings();
                self.read_raids();
            }
            self.native_level_opened(dim);
        }
        // Preparing the owner's level does not add it to createLevels. The
        // player dimension was resolved against the configured stems before loading.
        self.enter_level(starting_dim)?;
        Ok(())
    }

    /// `MinecraftServer.prepareLevels`: after all configured levels are
    /// published, activate their saved tickets, then wait for the chunks they
    /// hold (`ChunkLoadCounter`). The first tick finds them ticking and unpacks
    /// their saved block and fluid ticks at the saved game time
    /// (`startTickingChunk`), not at whichever tick they finish streaming in.
    pub(super) fn prepare_levels(&mut self) -> Result<()> {
        if self.options.execution_role.is_client() {
            return Ok(());
        }
        let back = self.dim;
        let dims = self.loaded_level_order();
        for &dim in &dims {
            self.enter_level(dim)?;
            Self::load_saved_level_tickets(&self.save, &mut self.streaming, &mut self.tickets);
            self.validate_mod_chunk_tickets(dim);
        }
        self.startup_ticket_restore_pending = false;
        let deadline = Instant::now() + Duration::from_secs(600);
        for dim in dims {
            self.enter_level(dim)?;
            self.wait_for_ticket_chunks(deadline);
        }
        self.enter_level(back)?;
        self.update_mob_spawning_flags();
        Ok(())
    }

    /// `MinecraftServer.updateMobSpawningFlags`: refresh each open chunk source.
    /// Sunrise's levels share one rule set, so the updated value is the same for all.
    pub(super) fn update_mob_spawning_flags(&mut self) {
        use sunrise_client::gamerules::vanilla::{SPAWN_MOBS, SPAWN_MONSTERS};

        let rules = &self.ui.rules;
        let spawn_enemies = rules.bool_at(SPAWN_MOBS) && rules.bool_at(SPAWN_MONSTERS);
        self.chunk_spawn_enemies = spawn_enemies;
        for level in self.levels.parked.values_mut() {
            level.chunk_spawn_enemies = spawn_enemies;
        }
    }

    /// `TicketStorage.activateAllDeactivatedTickets`: restore portal and
    /// command tickets, and stage mod tickets for their controllers to validate.
    fn load_saved_level_tickets(
        save: &SaveDir,
        streaming: &mut Streaming,
        tickets: &mut ChunkTickets,
    ) {
        tickets.restore_portal(save.load_portal_tickets());
        streaming.load_forced(
            save.load_forced_chunks(),
            save.load_mod_tickets(mod_api::__private::game::ticket_controller_slot_by_id),
        );
    }

    /// A level that opens after startup restores its saved command and portal
    /// tickets at once. Mod tickets still wait for validation after `level_load`.
    pub(super) fn open_runtime_level(&self, dim: Dim) -> Result<Parked> {
        let mut level = self.open_level(dim)?;
        Self::load_saved_level_tickets(&level.save, &mut level.streaming, &mut level.tickets);
        Ok(level)
    }

    /// Stream the level in place until each chunk its tickets make block
    /// ticking is a ticking chunk (lit, with its neighbours loaded).
    fn wait_for_ticket_chunks(&mut self, deadline: Instant) {
        // Forced chunks are checked two chunks out (their ticket block ticks one).
        // A `PORTAL` ticket's chunks are streamed as an area of their own (the
        // network tick does it, which hasn't run yet), whose chunks out to its
        // radius can become ticking chunks: those are waited for.
        let portal = self.portal_ticket_areas();
        for area in &portal {
            if !self.streaming.extra.contains(area) {
                self.streaming.extra.push(*area);
            }
        }
        loop {
            self.update_chunk_tickets();
            let levels = self.tickets.levels();
            let streaming = &self.streaming;
            let ready = streaming
                .forced
                .iter()
                .map(|&c| (c, 2))
                .chain(portal.iter().copied())
                .all(|((cx, cz), reach)| {
                    (cz - reach..=cz + reach).all(|z| {
                        (cx - reach..=cx + reach).all(|x| {
                            !levels.loaded_ticking(x, z) || super::ticking_chunk(&self.world, x, z)
                        })
                    })
                });
            if ready {
                return;
            }
            if Instant::now() >= deadline {
                log::warn!("Sunrise: the chunks of the saved tickets did not load");
                return;
            }
            self.update_world();
            self.streaming.wait(Duration::from_millis(5));
        }
    }

    /// Exchange the level in place with `p`.
    fn swap_level(&mut self, p: &mut Parked) {
        use std::mem::swap;
        swap(&mut self.world, &mut p.world);
        // The carrier first-use gate belongs to the world, not the
        // level: a level swap keeps it.
        self.world.carrier_data_existed_at_open = p.world.carrier_data_existed_at_open;
        swap(&mut self.streaming, &mut p.streaming);
        let mut locator = self.end.locator();
        swap(&mut locator, &mut p.locator);
        self.end.set_locator(locator);
        self.share_spawn_conditions();
        swap(&mut self.save, &mut p.save);
        swap(&mut self.block_ticks, &mut p.block_ticks);
        swap(&mut self.tick_chunks, &mut p.tick_chunks);
        swap(&mut self.tickets, &mut p.tickets);
        swap(&mut self.random_tick_sections, &mut p.random_tick_sections);
        swap(&mut self.level_events, &mut p.level_events);
        swap(&mut self.portals.entities, &mut p.portal_states);
        swap(&mut self.falling, &mut p.falling);
        swap(&mut self.paintings, &mut p.paintings);
        swap(&mut self.item_frames, &mut p.item_frames);
        swap(&mut self.armor_stands, &mut p.armor_stands);
        swap(&mut self.cushions, &mut p.cushions);
        swap(&mut self.others, &mut p.others);
        swap(&mut self.fangs, &mut p.fangs);
        swap(&mut self.boats, &mut p.boats);
        swap(&mut self.carts, &mut p.carts);
        swap(&mut self.jukebox_sounds, &mut p.jukebox_sounds);
        swap(&mut self.bolts, &mut p.bolts);
        swap(&mut self.crystals, &mut p.crystals);
        swap(&mut self.dragon, &mut p.dragon);
        swap(&mut self.breath_clouds, &mut p.breath_clouds);
        swap(&mut self.tridents, &mut p.tridents);
        swap(&mut self.rockets, &mut p.rockets);
        swap(&mut self.ominous_spawners, &mut p.ominous_spawners);
        swap(&mut self.tnts, &mut p.tnts);
        swap(&mut self.hoppers, &mut p.hoppers);
        swap(&mut self.mobs, &mut p.mobs);
        swap(&mut self.knot_ids, &mut p.knot_ids);
        swap(&mut self.broken_knots, &mut p.broken_knots);
        swap(&mut self.resident_entities.knots, &mut p.knots);
        swap(&mut self.resident_entities.general, &mut p.general);
        swap(&mut self.creatures.projectiles, &mut p.projectiles);
        swap(&mut self.dropped, &mut p.dropped);
        swap(&mut self.orbs, &mut p.orbs);
        swap(&mut self.persist, &mut p.persist);
        swap(&mut self.lectern_pulses, &mut p.lectern_pulses);
        swap(&mut self.lid_openers, &mut p.lid_openers);
        swap(&mut self.lan_events, &mut p.lan_events);
        swap(&mut self.lan_block_events, &mut p.lan_block_events);
        swap(&mut self.lan_entity_events, &mut p.lan_entity_events);
        swap(&mut self.lan_critical_hits, &mut p.lan_critical_hits);
        let (a, b) = (&mut self.be_anims, &mut p.anims);
        swap(&mut a.chests, &mut b.chests);
        swap(&mut a.books, &mut b.books);
        swap(&mut a.shulkers, &mut b.shulkers);
        swap(&mut a.bells, &mut b.bells);
        swap(&mut a.conduits, &mut b.conduits);
        swap(&mut a.skulls, &mut b.skulls);
        swap(&mut a.pots, &mut b.pots);
        swap(&mut a.gateways, &mut b.gateways);
        swap(&mut a.gateway_servers, &mut b.gateway_servers);
        swap(&mut a.conduit_servers, &mut b.conduit_servers);
        self.raiding.swap_level(&mut p.raiding);
        self.sculk.swap_level(&mut p.sculk);
        self.creaking.swap_level(&mut p.creaking);
        swap(&mut self.chunk_spawn_enemies, &mut p.chunk_spawn_enemies);
        swap(&mut self.spawn_scratch, &mut p.spawn_scratch);
        swap(&mut self.border, &mut p.border);
        // What only the client draws starts over.
        self.clear_client_level_particles();
    }

    /// Swap the level in place without publishing the one just left.
    fn replace_level(&mut self, target: Dim, p: &mut Parked) -> Dim {
        p.world.game_time = self.world.game_time;
        // The level's day time is its default clock's; a level without one follows the
        // Overworld's.
        let clock = self.level_clock_name(target);
        p.world.day_time = self.switch_clock(&clock);
        p.world.end_clock = self.world.end_clock;
        p.world.weather.copy_parameters_from(self.world.weather);
        // The game rules and the difficulty are the server's, not a
        // level's: natural regeneration holds in every dimension.
        p.world.health_rules = self.world.health_rules;
        // The sculk tables' watched blocks are built once per game.
        p.world.watched = std::mem::take(&mut self.world.watched);
        self.swap_level(p);
        // The tracked entities go with their level: a level's moves, read before its tick
        // describes its entities again, would otherwise meet the last level's (a move
        // missed the boat under a player while anyone was in another dimension).
        if let Some(h) = self.lan.as_mut() {
            std::mem::swap(&mut h.states, &mut p.lan_states);
        }
        std::mem::replace(&mut self.dim, target)
    }

    /// Put `target`'s level in place, opening it first if it is not loaded.
    /// The level that was in place is kept with the others; the shared
    /// clock and weather parameters go with the swap.
    pub(super) fn enter_level(&mut self, target: Dim) -> Result<()> {
        // A client holds no levels.
        if self.options.execution_role.is_client() {
            return Ok(());
        }
        if target != self.dim {
            let (mut p, opened) = match self.levels.parked.remove(&target) {
                Some(p) => (p, false),
                None => (self.open_runtime_level(target)?, true),
            };
            let left = self.replace_level(target, &mut p);
            self.levels.parked.insert(left, p);
            if opened {
                // The new streaming's radius and meshing options.
                self.apply_settings();
                // ServerLevel's constructor reads the level's raids.
                self.read_raids();
                self.native_level_opened(target);
                self.validate_mod_chunk_tickets(target);
            }
        }
        if let Some(h) = self.lan.as_mut() {
            h.hub.set_dimension(target.id());
        }
        self.sync_hub_clocks();
        Ok(())
    }

    /// Follow the direct mount of a travelling player through complete source objects.
    /// A later player in the same travelling stack finds no source root: the first
    /// arrival already moved it, keeping every id that the guests' mounts name.
    pub(super) fn parked_vehicle_root(&self, from: Dim, mut id: u32) -> Option<u32> {
        let p = self.levels.parked.get(&from)?;
        for _ in 0..=p.mobs.len() + p.persist.off_level.len() + 1 {
            let mob = p
                .mobs
                .iter()
                .find(|m| m.id == id)
                .or_else(|| {
                    p.persist
                        .off_level
                        .get(&id)
                        .and_then(persist::OffLevel::as_mob)
                })
                .filter(|m| !m.removed);
            if let Some(parent) = mob.and_then(|m| m.extra.mount) {
                id = parent;
                continue;
            }
            let present = mob.is_some()
                || p.boats.iter().any(|b| b.id == id)
                || p.carts.iter().any(|c| c.id == id);
            let removed = p.persist.removals.iter().chain(p.persist.destroying_vehicles.iter()).any(|target| matches!(
                target, commands::Target::Mob(n) | commands::Target::Boat(n) | commands::Target::Cart(n) if *n == id
            ));
            return (present && !removed).then_some(id);
        }
        None
    }

    /// Move the source tree's object memory and take its Mob descendants in
    /// canonical postorder. The caller owns the extracted root and admits it last.
    /// Transfer removal preserves lifetimes; residency is decided in the destination.
    pub(super) fn move_entity_memory(&mut self, from: Dim, root: u32) -> Vec<Mob> {
        let Some(p) = self.levels.parked.get_mut(&from) else {
            return Vec::new();
        };
        let mut edges: Vec<_> = p
            .mobs
            .iter()
            .chain(
                p.persist
                    .off_level
                    .values()
                    .filter_map(persist::OffLevel::as_mob),
            )
            .filter(|mob| !mob.removed)
            .map(|mob| (mob.extra.mount, mob.extra.vehicle_order, mob.id))
            .collect();
        edges.sort_by_key(|&(_, order, id)| (order, id));
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for (parent, _, id) in edges {
            if let Some(parent) = parent {
                children.entry(parent).or_default().push(id);
            }
        }
        let mut pending = vec![(root, false)];
        let mut seen = std::collections::HashSet::new();
        let mut ordered = Vec::new();
        while let Some((id, done)) = pending.pop() {
            if done {
                ordered.push(id);
            } else if seen.insert(id) {
                pending.push((id, true));
                if let Some(riders) = children.get(&id) {
                    pending.extend(riders.iter().rev().map(|&rider| (rider, false)));
                }
            }
        }
        let mut moved = Vec::new();
        for id in ordered {
            p.persist.move_object_to(id, &mut self.persist);
            if id == root {
                continue;
            }
            if let Some(state) = p.portal_states.remove(&id) {
                self.portals.entities.insert(id, state);
            }
            let mob = if let Some(index) = p.mobs.iter().position(|m| m.id == id) {
                Some(p.mobs.remove_for_transfer(index))
            } else {
                // Boats and carts are only roots, which the caller takes.
                match p.persist.off_level.remove(&id) {
                    Some(persist::OffLevel::Mob(mob)) => Some(mob),
                    _ => None,
                }
            };
            if let Some(mut mob) = mob {
                mob.join = sunrise_client::join_seq::JoinSeq::UNJOINED;
                moved.push(mob);
            }
        }
        moved
    }

    /// An already-owned body changes level, never construction identity.
    /// `PersistentEntitySectionManager.addEntityUuid` decides its residency:
    /// a body whose UUID an earlier arrival took is retained off-level.
    pub(super) fn admit_transferred_object(&mut self, mut object: persist::OffLevel) {
        let id = match &mut object {
            persist::OffLevel::Mob(mob) => {
                self.rebind_transferred_entity(mob.id, mob);
                mob.id
            }
            persist::OffLevel::Boat(boat) => {
                self.rebind_transferred_entity(boat.id, boat);
                boat.id
            }
            persist::OffLevel::Cart(cart) => {
                self.rebind_transferred_entity(cart.id, cart);
                cart.id
            }
        };
        self.journal_arrival(id);
        let level = self.dim;
        self.entity_attachments.move_collider(id, level);
        if self.uuid_admission_refused(self.persist.uuids.get(&id).copied()) {
            self.persist.off_level.insert(id, object);
            return;
        }
        match object {
            persist::OffLevel::Mob(mob) => self.mobs.push(mob),
            persist::OffLevel::Boat(boat) => self.boats.push(boat),
            persist::OffLevel::Cart(cart) => self.carts.push(cart),
        }
    }

    /// Far terrain for a client in `dim`: the level in place may be another
    /// one, the server ticking its levels in turn.
    pub(super) fn lod_source_in(
        &self,
        dim: Dim,
    ) -> Option<Arc<dyn sunrise_client::streaming::LodSource>> {
        if dim == self.dim {
            return Some(self.streaming.lod_source());
        }
        let parked = self.levels.parked.get(&dim)?;
        Some(parked.streaming.lod_source())
    }

    /// Put `dim`'s level in place for a command whose source is there
    /// (`execute in`, `execute at` a player in another level), opening it if
    /// nobody is in it; returns the level to put back afterwards. Only a
    /// server holds other levels.
    pub(super) fn enter_command_level(&mut self, dim: Dim) -> Option<Dim> {
        if self.options.execution_role.is_client() || dim == self.dim {
            return None;
        }
        let back = self.dim;
        match self.enter_level(dim) {
            Ok(()) => Some(back),
            Err(e) => {
                log::error!("could not open {}: {e}", dim.id());
                None
            }
        }
    }

    /// Run an action with `dim` in place, then restore the caller's level.
    /// Transactions and entity admission consult the level in place during the action.
    pub(super) fn with_level_in_place<R>(
        &mut self,
        dim: Dim,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if dim == self.dim {
            return Some(f(self));
        }
        let back = self.enter_command_level(dim)?;
        let result = f(self);
        self.enter_command_level(back);
        Some(result)
    }

    /// One server tick (`MinecraftServer.tickServer`): each level with
    /// players or forced chunks in turn (`tickChildren`), its chunks streamed
    /// and loaded and its world ticked. Shared command functions run before any level;
    /// the first active level also advances shared time and handles autosaves.
    /// Connection player ticks follow all levels. While `paused`, the network runs
    /// but player bodies do not.
    /// `ServerTickStart` and `ServerTickEnd` bracket the whole tick.
    pub(super) fn tick_server(&mut self, paused: bool) -> Result<()> {
        self.with_residency_tick(|game| game.tick_server_body(paused))
    }

    fn tick_server_body(&mut self, paused: bool) -> Result<()> {
        // A panic something caught left a game that carried on: its record ends with the tick.
        self.interrupted_turns.clear();
        // `tickRateManager.tick()` once per server tick, before any level
        // (each `ServerLevel.tick` only reads `runsNormally`), and the native
        // `ServerTickStart` before that level work (NeoForge's
        // `ServerTickEvent.Pre`). A paused pass and a held server tick nothing.
        let running = if !paused && !self.agent.hold_server {
            self.clock.check_sprint_completion();
            if let Some(report) = self.clock.take_sprint_report() {
                // ServerTickRateManager.finishTickSprint: the server's sendSuccess.
                self.console_broadcast = Some(true);
                self.sprint_report(&report);
                self.console_broadcast = None;
                self.send_admin_feedback();
                self.admin_feedback.clear();
                // Then onTickRateChanged, its mods outside the server's broadcast.
                self.on_tick_rate_changed();
            }
            Some(self.clock.tick_world_runs())
        } else {
            None
        };
        let server = self.serving.is_some() || self.options.dedicated.is_some();
        if server {
            self.run_requested_saves();
        }
        // A config edited while the server is paused (empty) still reloads.
        // The parts outside every level get spans of their own, so that a slow
        // tick's line and `sunrise-report` say when a mod's tick hook took it.
        if server {
            let _span = sunrise_client::spans::guard("mod_configs_poll");
            self.native_configs_poll();
        }
        if let Some(running) = running
            && server
        {
            let _span = sunrise_client::spans::guard("mods_server_tick_start");
            self.native_lifecycle_server_tick_start(running);
        }
        // `MinecraftServer.tickChildren`: command functions precede even the first idle level.
        if running == Some(true) {
            span("tick_functions", || self.tick_functions());
        }
        let observe_level =
            running.is_some() && (self.serving.is_some() || self.options.dedicated.is_some());
        let dimensions = self.level_creation_order().to_vec();
        let mut order = std::mem::take(&mut self.levels.tick_order);
        self.select_tick_levels(&mut order);
        let n = order.len();
        let mut result = Ok(());
        for phase in server_tick_phases(&dimensions, &order) {
            if let ServerTickPhase::IdleLevel(dim) = phase {
                if running == Some(true) {
                    self.tick_level_environment(dim);
                }
                continue;
            }
            let ServerTickPhase::Level(k, dim) = phase else {
                if running.is_some() && self.lan.is_some() {
                    let holding = sunrise_client::lan::Holding::start();
                    result = sunrise_client::spans::time("lan_guest_connections", || {
                        self.tick_guest_connections(true)
                    });
                    if result.is_ok()
                        && let Some(host) = self.lan.as_mut()
                    {
                        sunrise_client::spans::time("lan_end_tick", || host.hub.end_tick());
                    }
                    drop(holding);
                    if let Some(host) = self.lan.as_ref() {
                        sunrise_client::lan::flush_all(host.hub.guests.iter().map(|g| &g.sender));
                    }
                }
                break;
            };
            // Each active level's share of the tick: `sunrise-report`'s `level_*` spans.
            sunrise_threads::work::set_dim(match dim {
                Dim::Overworld => 1,
                Dim::Nether => 2,
                Dim::End => 3,
                Dim::Extra(_) => 0,
            });
            let _level_span = sunrise_client::spans::guard(match dim {
                Dim::Overworld => "level_overworld",
                Dim::Nether => "level_the_nether",
                Dim::End => "level_the_end",
                Dim::Extra(_) => "level_extra",
            });
            if let Err(e) = self.enter_level(dim) {
                result = Err(e);
                break;
            }
            self.level_pass = Pass {
                first: k == 0,
                last: k + 1 == n,
                running,
                packets_drained: false,
            };
            if observe_level && let Some(level) = self.level(dim.id()) {
                self.native_level_tick_pre(level);
            }
            // ServerChunkCache.tick's purge, before the update that may unload the columns.
            if running == Some(true) {
                self.streaming.purge_unknown_tickets();
            }
            sunrise_client::spans::time("dedicated_streaming", || {
                self.update_dedicated_streaming();
            });
            self.tick_spawn_tickets(running == Some(true), true);
            sunrise_client::spans::time("update_world", || self.update_world());
            // The end of ServerChunkCache.tick, after its unloads: `clearCache`. A paused or held
            // server ticks no level.
            if running.is_some() {
                self.streaming.clear_full_lookups();
            }
            let tick_span = sunrise_client::spans::guard("level_tick");
            // This level's moving colliders, solid to its movers for this tick only.
            let colliders = self.placed_moving_colliders();
            let parts = self.collision_parts();
            let history = self
                .collider_history(&colliders)
                .map(sunrise_client_render::moving_colliders::HistoryScope::enter);
            result = {
                let _scope = sunrise_client_render::moving_colliders::Scope::enter_level(
                    self.collider_level(),
                    colliders.clone(),
                );
                sunrise_client::block_contact::in_level(
                    self.tick,
                    (self.serving.is_some() || self.options.dedicated.is_some())
                        .then_some(dim.id()),
                    || {
                        if paused {
                            self.tick_lan(true);
                            Ok(())
                        } else if k == 0 {
                            self.tick_server_first(running.unwrap_or(false))
                        } else {
                            let running = running.unwrap_or_else(|| self.clock.world_runs());
                            if running {
                                self.tick_level_environment(dim);
                            }
                            // ClientLevel.tick: the sky flash fades in a frozen level too.
                            self.tick_weather_effects();
                            self.tick_world(running)
                        }
                    },
                )
            };
            drop(history);
            // Then the bodies standing on a collider that moved are carried with it.
            if result.is_ok() && !paused {
                sunrise_client::block_contact::in_level(
                    self.tick,
                    (self.serving.is_some() || self.options.dedicated.is_some())
                        .then_some(dim.id()),
                    || self.carry_pass(&colliders),
                );
            }
            drop(parts);
            drop(tick_span);
            self.drain_entity_inside();
            if result.is_err() {
                break;
            }
            if observe_level && let Some(level) = self.level(dim.id()) {
                self.native_level_tick_post(level);
            }
            #[cfg(feature = "state-digest")]
            self.state_digest_tick(dim, running);
            sunrise_client::spans::time("server_debug", || self.publish_server_debug(dim));
            // An entity that crossed into a level that already ticked this
            // server tick is in it when the tick ends (`addDuringTeleport`).
            if running == Some(true) {
                let _span = sunrise_client::spans::guard("late_portal_arrivals");
                let mut moved = false;
                for &earlier in &order[..k] {
                    if self.has_portal_arrivals(earlier) && self.enter_level(earlier).is_ok() {
                        self.land_portal_arrivals();
                        moved = true;
                    }
                }
                if moved && k + 1 < n {
                    result = self.enter_level(dim);
                    if result.is_err() {
                        break;
                    }
                }
            }
        }
        // Vanilla ticks the chunk source of every level, ticked here or not: the levels left
        // parked purge their tickets and forget their recent lookups too.
        if running.is_some() {
            self.levels
                .tick_parked_chunk_caches(running == Some(true), |dim| !order.contains(&dim));
        }
        // Levels held only by loading tickets: stream them without running the world tick.
        if result.is_ok() {
            result = self.run_view_passes(&order, running == Some(true));
        }
        // Parked levels that did not tick or run a view pass still advance any light
        // work scheduled on them directly (for example via `LevelRef::relight_chunks`).
        if result.is_ok() {
            self.update_parked_light(&order);
        }
        if let Some(&first) = order.first()
            && self.dim != first
        {
            let _ = self.enter_level(first);
        }
        self.levels.tick_order = order;
        self.level_pass = Pass::default();
        if result.is_err() {
            self.native_lifecycle.cancel_server_tick();
        }
        result?;
        // The levels mods asked to open or close: now that no level is in the middle of a tick.
        self.apply_runtime_levels();
        let evicted = if should_run_idle_maintenance(paused) {
            let _span = sunrise_client::spans::guard("evict_idle_levels");
            self.evict_idle_levels()
        } else {
            Ok(())
        };
        if let Err(error) = evicted {
            self.native_lifecycle.cancel_server_tick();
            return Err(error);
        }
        // `ServerTickEnd`: an errored tick or an idle-eviction failure (at
        // the `?` above) never reaches this site.
        {
            let _span = sunrise_client::spans::guard("mods_server_tick_end");
            self.native_lifecycle_server_tick_end(paused);
        }
        Ok(())
    }

    /// `tick_server` as a server thread runs it, timed whenever spans are on.
    pub(super) fn tick_server_timed(&mut self, paused: bool) -> Result<()> {
        self.tick_server_spans(paused, |_| {}).map(|_| ())
    }

    /// Return the time in `tick_server` itself, and hand its detailed timing to `record`
    /// while spans are on. The recording and thread-work bookkeeping are outside that time.
    pub(super) fn tick_server_spans(
        &mut self,
        paused: bool,
        record: impl FnOnce(TickTiming),
    ) -> Result<Duration> {
        let started = Instant::now();
        sunrise_threads::work::tick_start();
        let result = self.tick_server_recorded(paused, record);
        sunrise_threads::work::tick_end(started.elapsed().as_nanos() as u64);
        result
    }

    fn tick_server_recorded(
        &mut self,
        paused: bool,
        record: impl FnOnce(TickTiming),
    ) -> Result<Duration> {
        if self.options.spikes.is_none() && !sunrise_client::spans::recording_here() {
            let started = Instant::now();
            self.tick_server(paused)?;
            return Ok(started.elapsed());
        }
        // Discard between-tick work (such as the chunk pump) before timing this tick.
        sunrise_client::spans::take_raw();
        let cpu = sunrise_client::spans::thread_cpu();
        let started = Instant::now();
        let result = self.tick_server(paused);
        let method = started.elapsed();
        let wall = method.as_secs_f32() * 1000.0;
        let cpu = sunrise_client::spans::thread_cpu()
            .zip(cpu)
            .map(|(end, start)| (end - start).as_secs_f32() * 1000.0);
        // Taken every tick: nothing else takes this thread's spans.
        let raw = sunrise_client::spans::take_raw();
        // The spans go to the slow ring; whose packets or events took the tick stays in the owner's log.
        let notes = sunrise_client::spans::take_notes();
        if let Some(threshold) = self.options.spikes
            && wall > threshold
            && !notes.is_empty()
        {
            log::info!(
                "Sunrise: {wall:.1} ms tick was busy with: {}",
                slowest_steps(&notes)
            );
        }
        record(TickTiming {
            wall_ms: wall,
            cpu_ms: cpu,
            raw,
        });
        result.map(|()| method)
    }

    /// The end of `MinecraftServer.runServer`'s loop for a server thread:
    /// the next tick starts one tick length after this one, or straight away
    /// while the sprint runs (`checkShouldSprintThisTick`). A server more
    /// than two seconds behind skips the ticks it missed: that returns how far
    /// behind it was, for the overload warning, and how long the thread blocked on
    /// workers or slept (`MinecraftServer.waitForTasks`'s idle time).
    pub(super) fn wait_for_next_server_tick(
        &mut self,
        next: &mut Instant,
        paused: bool,
    ) -> (Option<Duration>, Duration) {
        let now = Instant::now();
        let mut idle = Duration::ZERO;
        if !paused && self.clock.sprinting() {
            *next = now;
            return (None, idle);
        }
        *next += self.clock.tick_duration();
        if *next > now {
            // The chunks loading (the players', force-loaded ones, a
            // `/pregen` task) keep the workers fed until the next tick.
            idle = self.pump_chunks(*next, paused);
            let now = Instant::now();
            if *next > now {
                std::thread::sleep(*next - now);
                idle += now.elapsed();
            }
        } else if now - *next > Duration::from_secs(2) {
            let behind = now - *next;
            *next = now;
            return (Some(behind), idle);
        }
        (None, idle)
    }

    /// The loaded levels this server tick runs, in `MinecraftServer.levels`
    /// creation order. A level with no work can be skipped; the Overworld
    /// supplies the idle pass when none qualify.
    fn select_tick_levels(&self, dims: &mut Vec<Dim>) {
        dims.clear();
        let order = self.level_creation_order();
        dims.extend(
            order
                .iter()
                .copied()
                .filter(|&dim| self.should_tick_level(dim)),
        );
        if dims.is_empty() {
            dims.push(Dim::Overworld);
        }
    }

    #[cfg(test)]
    pub(super) fn levels_to_tick(&self) -> Vec<Dim> {
        let mut dims = Vec::new();
        self.select_tick_levels(&mut dims);
        dims
    }

    fn should_tick_level(&self, dim: Dim) -> bool {
        // `/forceload`ed chunks, or a native mod's tickets.
        let streaming = if dim == self.dim {
            Some(&self.streaming)
        } else {
            self.levels.parked.get(&dim).map(|p| &p.streaming)
        };
        streaming.is_some_and(|s| !s.forced.is_empty())
            || self.lan.as_ref().is_some_and(|h| {
                h.hub.guests.iter().any(|g| {
                    g.dimension == dim.id() && g.participates_in_player_simulation()
                })
            })
            // A viewer count not yet told to 0 (`tick_mod_container_viewers`).
            || self.mod_container_viewers.keys().any(|(k, _)| *k == dim.id())
            || self.player_bed_levels().any(|b| b == dim)
            || self.game_tests.has_tests_in(dim)
            || self.pregen_level() == Some(dim)
            // An arrival or its portal ticket keeps the destination level active.
            || self.has_portal_arrivals(dim)
            || self.server_trips.iter().any(|trip| trip.spawn_destination_areas(dim).next().is_some())
            || !self.portal_tickets_in(dim).is_empty()
            // A viewed `Ticking` area (#3716), for its chunks only.
            || self.viewed_level(dim) == Some(true)
    }

    /// A loading-only ticket needs chunk work, not an active simulation.
    pub(super) fn has_spawn_tickets(&self, dim: Dim) -> bool {
        if self.lan.as_ref().is_some_and(|host| {
            host.hub
                .lan
                .configuration_tasks
                .loading_spawns(dim.id())
                .next()
                .is_some()
        }) {
            return true;
        }
        if dim == self.dim {
            self.tickets.spawn_areas().next().is_some()
        } else {
            self.levels
                .parked
                .get(&dim)
                .is_some_and(|p| p.tickets.spawn_areas().next().is_some())
        }
    }

    /// The portal tickets of `dim`'s level (`placePortalTicket`, or saved).
    pub(super) fn portal_tickets_in(&self, dim: Dim) -> &[PortalTicket] {
        if dim == self.dim {
            self.tickets.portal_tickets()
        } else {
            self.levels
                .parked
                .get(&dim)
                .map_or(&[], |p| p.tickets.portal_tickets())
        }
    }

    /// The areas the level in place's portal tickets keep streamed: around each
    /// exit its block-ticking chunks and their neighbours, the ring vanilla
    /// keeps `FULL` (level 33), so the outer block-ticking ring is a ticking
    /// chunk too; and at least the chunks its expiry waits for
    /// (`portal_ticket_can_expire`).
    pub(super) fn portal_ticket_areas(&self) -> Vec<((i32, i32), i32)> {
        let block_ticking = i32::from(sunrise_client::ticket_levels::BLOCK_TICKING);
        self.tickets
            .portal_tickets()
            .iter()
            .map(|t| (t.chunk, (block_ticking - t.level).max(1)))
            .collect()
    }

    /// Advance light jobs queued on parked levels that neither ticked nor ran a view pass
    /// this server tick, without switching `self.dim` or running chunk generation/unloads.
    pub(super) fn update_parked_light(&mut self, ticked: &[Dim]) {
        let viewed_active = self.lan.as_ref().is_some_and(|h| h.hub.viewed.active());
        let pending: Vec<Dim> = self
            .levels
            .parked
            .iter()
            .filter_map(|(&dim, parked)| {
                (!ticked.contains(&dim)
                    && !(viewed_active && self.needs_view_pass(dim))
                    && !parked.streaming.paused
                    && parked.streaming.has_pending_light(&parked.world))
                .then_some(dim)
            })
            .collect();
        for dim in pending {
            self.ask_edited_light_emission(dim);
            if let Some(parked) = self.levels.parked.get_mut(&dim) {
                parked.streaming.update_light(&mut parked.world);
            }
        }
    }

    /// Whether saving is currently disabled (`/save-off`, `ServerLevel.noSave`).
    fn saving_disabled(&self) -> bool {
        self.server_lists
            .as_ref()
            .is_some_and(|lists| !lists.lock().unwrap().autosave)
    }

    /// Evict unticketed chunks and entities from idle parked levels without closing the
    /// levels themselves (`MinecraftServer.levels`, `ChunkMap.processUnloads`).
    pub(super) fn evict_idle_levels(&mut self) -> Result<()> {
        // Runtime level opens/closes are applied outside this pass. Read each parked level
        // in creation order, after previous unload listeners have had a chance to activate it.
        for index in 0..self.levels.order.len() {
            if self.saving_disabled() {
                break;
            }
            let dim = self.levels.order[index];
            if dim == Dim::Overworld
                || !self.levels.parked.get(&dim).is_some_and(|p| {
                    (!p.world.chunks.is_empty() || !p.persist.uuids.is_empty())
                        && !p.streaming.has_unknown_tickets()
                })
                || self.should_tick_level(dim)
                || self.needs_view_pass(dim)
                || self.has_spawn_tickets(dim)
                || self.has_async_chunk_requests(dim)
                || self.vehicle_waits_in(dim)
            {
                continue;
            }
            self.evict_idle_level_chunks(dim)?;
        }
        Ok(())
    }

    /// Evict non-forced chunks and entities from an idle parked level while
    /// keeping the level open in `self.levels.parked`.
    pub(super) fn evict_idle_level_chunks(&mut self, dim: Dim) -> Result<()> {
        if self.saving_disabled() {
            return Ok(());
        }
        let back = self.dim;
        self.enter_level(dim)?;
        let prev_radius = self.streaming.radius;
        let prev_paused = self.streaming.paused;
        let prev_extra = std::mem::take(&mut self.streaming.extra);
        self.streaming.radius = -1;
        self.streaming.paused = false;
        self.streaming.force_unload_sweep();
        let evicted = self.update_world_with_eviction(true);
        self.rebuild_chunk_tickets(false);
        self.streaming.radius = prev_radius;
        self.streaming.paused = prev_paused;
        self.streaming.extra = prev_extra;
        self.enter_level(back)?;
        evicted
    }

    /// Save the parked level `dim` and drop it on explicit runtime-level
    /// unload: the mods hear its chunks and the level unload.
    pub(super) fn close_parked_level(&mut self, dim: Dim) -> Result<()> {
        debug_assert!(
            !dim.is_builtin(),
            "built-in levels stay open until server shutdown"
        );
        let back = self.dim;
        self.enter_level(dim)?;
        // Vanilla saves the level's chunks before their unload hooks.
        self.native_chunks_loaded();
        self.abort_abandoned_transactions(0);
        // Light still queued or in flight would be saved with the new blocks under the old
        // light arrays marked on: a column it can reach is saved unlit and relit on reopen.
        for key in self.streaming.columns_with_outstanding_light(&self.world) {
            if let Some(chunk) = self.world.chunks.get_mut(&key) {
                Arc::make_mut(chunk).lit = false;
                // A close that fails leaves the level parked: it relights.
                self.world.dirty_light.insert(key);
            }
        }
        if let Err(why) = self.save_level(true) {
            // Not saved: the level stays parked, and the level in place is the one before.
            self.enter_level(back)?;
            return Err(why);
        }
        let chunks: Vec<_> = self.world.chunks.values().cloned().collect();
        self.native_chunks_unloaded(&chunks);
        // Save and unload listeners can leave frames open; undo them before dropping the level.
        self.abort_abandoned_transactions(0);
        // Its entities' attachments are saved with them; they leave memory too.
        for id in self.persist.uuids.keys() {
            self.entity_attachments.remove(id);
            self.entity_attachments.remove_collider(*id);
        }
        self.enter_level(back)?;
        self.native_level_closed(dim);
        self.levels.parked.remove(&dim);
        Ok(())
    }

    /// The mods hear `dim`'s level open (`LevelEvent.Load`), on a server.
    pub(super) fn native_level_opened(&mut self, dim: Dim) {
        if dim == self.dim {
            self.levels.active_closed = false;
        }
        if self.remote.is_some()
            || self.options.execution_role.is_client()
            || self.levels.order.contains(&dim)
        {
            return;
        }
        let journal = self.resident_entities.table.borrow().producer();
        if dim == self.dim {
            self.bind_resident_owners(&journal, dim.key(), true);
        } else if let Some(parked) = self.levels.parked.get_mut(&dim) {
            parked.bind_resident_owners(&journal, dim.key(), true);
        }
        self.bind_dimension_attachments(dim);
        self.levels.order.push(dim);
        self.init_custom_spawners(dim);
        self.native_level_load(dim.id());
    }

    /// The mods hear `dim`'s level close (`LevelEvent.Unload`), once for
    /// each open they heard.
    pub(super) fn native_level_closed(&mut self, dim: Dim) {
        self.remove_level_fake_players(dim.id());
        if let Some(i) = self.levels.order.iter().position(|d| *d == dim) {
            self.native_level_unload(dim.id());
            self.levels.order.remove(i);
        }
        if dim == self.dim {
            self.world.water_tickets.clear();
        } else if let Some(world) = self.levels.parked_world_mut_of(dim.id()) {
            world.water_tickets.clear();
        }
        self.creatures.custom_spawners.remove(&dim);
        self.attachment_indexes.remove(&dim);
        self.passages.retire_level(dim.key());
        self.clear_async_chunk_requests_for_dim(dim);
        self.close_saved_chunk_readers_for_dim(dim);
        self.close_detached_generators_for_dim(dim);
    }

    /// `MinecraftServer.stopServer`'s `level.close()` loop: close every loaded
    /// level in creation order, firing `LevelEvent.Unload` only after that
    /// level's own close (`save.flush()`) completes without error. A failure
    /// closing one level is logged and skips its unload event while continuing
    /// to close the remaining levels.
    pub(in crate::game) fn close_loaded_levels_for_stop(&mut self) {
        let dims = self.loaded_level_order();
        for dim in dims {
            if dim == self.dim {
                if self.levels.active_closed {
                    continue;
                }
                match self.save.flush() {
                    Ok(()) => {
                        self.streaming.abort_generation();
                        self.native_level_closed(dim);
                        self.levels.active_closed = true;
                    }
                    Err(error) => {
                        log::error!("Exception closing the level {}: {error}", dim.id());
                    }
                }
            } else if let Some(parked) = self.levels.parked.get_mut(&dim) {
                match parked.save.flush() {
                    Ok(()) => {
                        parked.streaming.abort_generation();
                        self.native_level_closed(dim);
                        self.levels.parked.remove(&dim);
                    }
                    Err(error) => {
                        log::error!("Exception closing the level {}: {error}", dim.id());
                    }
                }
            }
        }
    }

    /// Run the requested level save passes. `level_save` fires after a pass even if an
    /// individual record failed and was reported. A failed awaited flush ends the pass with
    /// an error and no hook. Reopen levels as needed, then restore the active one.
    pub(super) fn run_requested_saves(&mut self) {
        if Self::admit_entity_save().is_err() {
            return;
        }
        let requested = std::mem::take(&mut self.levels.requested_saves);
        if requested.is_empty() || self.remote.is_some() {
            return;
        }
        let back = self.dim;
        for dim in requested {
            if let Err(e) = self.enter_level(dim).and_then(|()| self.save_level(true)) {
                log::error!("saving {} on a mod's request failed: {e}", dim.id());
            }
        }
        if let Err(e) = self.enter_level(back) {
            log::error!(
                "returning to {} after a requested save failed: {e}",
                back.id()
            );
        }
    }

    /// Save every loaded level in creation order (`MinecraftServer.saveAllChunks`),
    /// restoring whichever level was in place afterwards.
    pub(super) fn save_all_levels(&mut self, wait: bool) -> Result<()> {
        let back = self.dim;
        let dims = self.loaded_level_order();
        let mut saved = Ok(());
        for dim in dims {
            if let Err(error) = self.enter_level(dim).and_then(|()| self.save_level(wait)) {
                saved = Err(error);
                break;
            }
        }
        let restored = if self.dim != back {
            self.enter_level(back)
        } else {
            Ok(())
        };
        saved.and(restored)
    }
}

#[cfg(test)]
fn dim_fallback_key(dim: Dim) -> (u8, &'static str) {
    match dim {
        Dim::Overworld => (0, ""),
        Dim::Nether => (1, ""),
        Dim::End => (2, ""),
        Dim::Extra(_) => (3, dim.id()),
    }
}

/// The steps a slow tick noted as its slowest (`spans::note`), whose packets or which player's event.
fn slowest_steps(notes: &[(f32, String)]) -> String {
    let list: Vec<String> = notes
        .iter()
        .map(|(ms, what)| format!("{what} {ms:.1}"))
        .collect();
    list.join(", ")
}

impl Game<'_> {
    /// `Level.getChunk(x, z)` (`ServerChunkCache.getChunk(x, z, FULL, true)`) on the server
    /// thread: column `chunk` of the level in place (`parked: None`) or of the parked level
    /// `parked`, without switching levels, loaded or generated to FULL (resident and lit) under
    /// an `UNKNOWN` ticket (`Streaming::begin_full_column`). It blocks until then, with no
    /// timeout, as vanilla's `managedBlock`. Each column the wait brings in is announced while
    /// it is resident, before this returns: its `dynamic_light` cells asked (the light job
    /// waits for them) and `chunk_load` fired, at the same point as vanilla's FULL task fires
    /// NeoForge's `ChunkEvent.Load` inside `getChunk`. `Ok` once FULL; `Failed` when a
    /// generation the column needs panicked, before or during the wait
    /// (`Streaming::full_column_failed`); `Unloaded` when `begin_full_column` refuses, when
    /// the level is gone, and when generation is aborted during the wait.
    pub(in crate::game) fn load_full_chunk(
        &mut self,
        parked: Option<mod_api::LevelRef>,
        chunk: (i32, i32),
    ) -> std::result::Result<(), mod_api::ChunkLoadError> {
        use mod_api::ChunkLoadError;
        use sunrise_client::streaming::{FullColumn, full_column_ready};
        let id = parked.map(|level| level.dimension_id());
        let Some((streaming, world)) = self.full_load_level(id) else {
            return Err(ChunkLoadError::Unloaded);
        };
        match streaming.begin_full_column(world, chunk) {
            FullColumn::Ready => return Ok(()),
            FullColumn::Refused => return Err(ChunkLoadError::Unloaded),
            FullColumn::Failed => return Err(ChunkLoadError::Failed),
            FullColumn::Pending => {}
        }
        loop {
            let Some((streaming, world)) = self.full_load_level(id) else {
                return Err(ChunkLoadError::Unloaded);
            };
            let alive = streaming.pump_column(world);
            self.announce_synchronous_loads(parked);
            let Some((streaming, world)) = self.full_load_level(id) else {
                return Err(ChunkLoadError::Unloaded);
            };
            let failed = streaming.full_column_failed(chunk);
            if !alive || failed || full_column_ready(world, chunk) {
                return if streaming.end_full_column(world, chunk) {
                    Ok(())
                } else if failed {
                    Err(ChunkLoadError::Failed)
                } else {
                    Err(ChunkLoadError::Unloaded)
                };
            }
            streaming.wait(Duration::from_millis(5));
        }
    }

    /// The streaming and world of the level in place, or of the parked level `parked`.
    fn full_load_level(
        &mut self,
        parked: Option<&str>,
    ) -> Option<(&mut sunrise_client::streaming::Streaming, &mut World)> {
        match parked {
            None => Some((&mut self.streaming, &mut self.world)),
            Some(id) => {
                let level = self.levels.parked_mut_of(id)?;
                Some((&mut level.streaming, &mut level.world))
            }
        }
    }

    /// The columns a synchronous load brought into the level, told to the mods as the level's
    /// own update tells them (`native_chunks_loaded`), their `dynamic_light` cells asked first.
    fn announce_synchronous_loads(&mut self, parked: Option<mod_api::LevelRef>) {
        let Some(level) = parked else {
            self.ask_loaded_light_emission(0);
            self.native_chunks_loaded();
            return;
        };
        let id = level.dimension_id();
        let Some(world) = self.levels.parked_world_mut_of(id) else {
            return;
        };
        let loaded = std::mem::take(&mut world.loaded_chunks);
        if self.pack.context.light.any_dynamic() {
            let dimension = Self::light_emission_dimension(level);
            for &(x, z, _) in &loaded {
                self.ask_level_chunk_light_emission(Some(id), dimension, (x, z));
            }
        }
        for (x, z, new) in loaded {
            self.native_chunk_load(id, [x, z], new);
        }
    }
}

impl Game<'_> {
    /// Rebind only an already-owning transfer, before installing it in this level.
    /// Memory still belongs to the parked source until the caller moves it.
    pub(super) fn rebind_transferred_entity(
        &mut self,
        id: u32,
        entity: &mut impl sunrise_mod_runtime::attachments::HasAttachmentLifetime,
    ) {
        let slot = entity.attachment_lifetime();
        if !slot.is_bound() {
            return;
        }
        let Some(level) = self.level(self.dim.id()) else {
            return;
        };
        let Some(map) = self.entity_attachments.get_mut(&id) else {
            return;
        };
        slot.bind(
            map,
            self.attachment_indexes.entry(self.dim).or_default(),
            mod_api::AttachmentHolder::Entity(mod_api::EntityRef::new(level.epoch(), id)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirty_test_chunk(game: &mut Game<'_>) {
        let chunk = sunrise_client::chunk::Chunk::with_layout(0, 0, game.world.layout());
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.world.modified.insert((0, 0));
    }

    fn queue_unencodable_entity_record(save: &SaveDir) {
        use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};
        let invalid =
            (0..=DISK_WRITE_NESTING_GUARD).fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
        save.save_entities_later(vec![((2, 0), vec![invalid])])
            .unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn save_all_succeeds_after_reported_failures_in_multiple_levels() {
        use crate::game::native_entity_refs::tests::with_game;
        use sunrise_client::save::{Level, LevelShape};
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                let root = tempfile::tempdir().unwrap();
                game.world_root = root.path().to_path_buf();
                game.save = Arc::new(SaveDir::new(
                    game.world_root.clone(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.level.name = "saved despite two refused records".into();
                let mut folders = Vec::new();
                for dim in [Dim::Nether, Dim::End, Dim::Overworld] {
                    game.enter_level(dim).unwrap();
                    dirty_test_chunk(game);
                    folders.push((dim, game.save.root.clone(), game.save.shape));
                    if dim != Dim::End {
                        queue_unencodable_entity_record(&game.save);
                    }
                }
                assert_eq!(
                    game.save_all_reply().translation_key(),
                    Some("commands.save.success")
                );
                assert_eq!(game.save.record_save_failures(), 2);
                assert_eq!(game.dim, Dim::Overworld);
                assert_eq!(
                    Level::read(root.path()).unwrap().name,
                    "saved despite two refused records"
                );
                for (_, folder, shape) in folders {
                    assert!(
                        SaveDir::new(folder.clone(), shape, None)
                            .raw_chunk(0, 0)
                            .is_some(),
                        "{} was not written",
                        folder.display()
                    );
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_idle_level_closes_after_reporting_a_record_failure() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::{RuntimeGenerator, RuntimeLevel};
        use sunrise_client::save::LevelShape;
        crate::game::in_own_process(
            "game::levels::tests::an_idle_level_closes_after_reporting_a_record_failure",
            || {
                mod_platform::testing::with_mods(Vec::new().into(), || {
                    with_game(|game| {
                        let root = tempfile::tempdir().unwrap();
                        game.world_root = root.path().to_path_buf();
                        game.save = Arc::new(SaveDir::new(
                            game.world_root.clone(),
                            LevelShape::OVERWORLD,
                            Some(Default::default()),
                        ));
                        // Built-in levels unload chunks without closing; exercise the save
                        // on close of a custom level instead.
                        let id = "test_idle:record_failure";
                        let flat =
                            RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Flat, 1);
                        game.runtime_level_open(id, flat).unwrap();
                        game.apply_runtime_levels();
                        assert!(game.runtime_level_is_open(id));
                        let custom = Dim::extra(id).unwrap();
                        game.enter_level(custom).unwrap();
                        dirty_test_chunk(game);
                        queue_unencodable_entity_record(&game.save);
                        let disk = SaveDir::new(game.save.root.clone(), game.save.shape, None);
                        game.enter_level(Dim::Overworld).unwrap();
                        game.close_parked_level(custom).unwrap();
                        assert_eq!(game.dim, Dim::Overworld);
                        assert!(!game.levels.parked.contains_key(&custom));
                        assert_eq!(game.save.record_save_failures(), 1);
                        assert!(disk.raw_chunk(0, 0).is_some());
                    });
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn idle_eviction_keeps_the_refused_column_and_retires_its_neighbor() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use mod_api::{ChunkUnloadEvent, EntitiesUnloadEvent, Hooks, ModEntry, ServerHost};
        use std::sync::Mutex;
        use sunrise_client::{chunk::Chunk, items::ItemStack};
        use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};

        if run_fresh_cases(
            "game::levels::tests::idle_eviction_keeps_the_refused_column_and_retires_its_neighbor",
            "SUNRISE_IDLE_EVICTION_WRITE_CASE",
            &["refused-entities"],
        ) {
            return;
        }

        #[derive(Debug, PartialEq)]
        enum Notice {
            Chunk([i32; 2]),
            Entities([i32; 2], usize),
        }
        struct Log(Arc<Mutex<Vec<Notice>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Log {
            fn chunk_unload(&self, _: &mut dyn ServerHost, event: &mut ChunkUnloadEvent<'_>) {
                if event.dimension == "minecraft:the_nether" {
                    self.0.lock().unwrap().push(Notice::Chunk(event.chunk));
                }
            }
            fn entities_unload(&self, _: &mut dyn ServerHost, event: &mut EntitiesUnloadEvent<'_>) {
                if event.dimension == "minecraft:the_nether" {
                    self.0
                        .lock()
                        .unwrap()
                        .push(Notice::Entities(event.chunk, event.count));
                }
            }
        }
        let notices = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "idle_eviction",
                Box::new(Log(notices.clone())),
            )]
            .into(),
            || {
                with_game(|game| {
                    let root = tempfile::tempdir().unwrap();
                    game.world_root = root.path().to_owned();
                    game.enter_level(Dim::Nether).unwrap();
                    for (x, item) in [(0, "minecraft:diamond"), (1, "minecraft:gold_ingot")] {
                        game.world.chunks.insert(
                            (x, 0),
                            Arc::new(Chunk::with_layout(x, 0, game.world.layout())),
                        );
                        game.world.modified.insert((x, 0));
                        game.spawn_item(
                            ItemStack {
                                item: item.into(),
                                count: 1,
                                damage: 0,
                                extra: None,
                            },
                            [f64::from(x * 16) + 0.5, 64.0, 0.5],
                            [0.0; 3],
                            0,
                        );
                    }
                    let previous = Tag::compound([("marker", Tag::Int(47))]);
                    game.save
                        .save_entities(vec![((0, 0), vec![previous.clone()])])
                        .unwrap();
                    let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                        .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
                    game.spawn_loaded_entity((0, 0), invalid, None);
                    let disk = SaveDir::new(game.save.root.clone(), game.save.shape, None);
                    game.enter_level(Dim::Overworld).unwrap();
                    game.evict_idle_level_chunks(Dim::Nether).unwrap();
                    game.enter_level(Dim::Nether).unwrap();
                    assert!(game.world.chunks.contains_key(&(0, 0)));
                    assert!(game.world.modified.contains(&(0, 0)));
                    assert!(!game.world.chunks.contains_key(&(1, 0)));
                    assert!(!game.world.modified.contains(&(1, 0)));
                    assert_eq!(game.dropped.len(), 1);
                    assert_eq!(game.dropped[0].stack.item.as_ref(), "minecraft:diamond");
                    assert_eq!(disk.load_entities(0, 0).unwrap(), [previous]);
                    let neighbor = disk.load_entities(1, 0).unwrap();
                    assert_eq!(neighbor.len(), 1);
                    assert_eq!(
                        neighbor[0].get("Item").and_then(|item| item.get("id")),
                        Some(&Tag::String("minecraft:gold_ingot".into()))
                    );
                    assert!(disk.raw_chunk(1, 0).is_some());
                })
            },
        );
        assert_eq!(
            *notices.lock().unwrap(),
            [Notice::Chunk([1, 0]), Notice::Entities([1, 0], 1)]
        );
    }

    #[test]
    fn levels_clear_retires_the_level_order() {
        let mut levels = Levels::default();
        levels.order.push(Dim::Nether);
        assert!(!levels.order.is_empty());
        levels.clear();
        assert!(levels.order.is_empty());
        assert!(levels.parked.is_empty());
    }

    #[test]
    fn idle_maintenance_respects_pause() {
        // Server pause skips parked-level maintenance; unpaused ticks (including
        // `/tick freeze`, where `paused == false`) still run it.
        assert!(!should_run_idle_maintenance(true));
        assert!(should_run_idle_maintenance(false));
    }

    /// A game on the prepared resources in `SUNRISE_TEST_RESOURCES`, with no
    /// world opened and commands allowed.
    fn prepared_game<'a>(
        resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
    ) -> Game<'a> {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
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
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        // The game keeps its resources for the whole test.
        let resources = &*resources.insert(loaded.resources);
        let mut game = Game::new(
            resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game
    }

    /// `WorldBorderCommand.setSize` from `/worldborder add` with a time: the
    /// new change lasts the ticks still running plus the argument
    /// (`getLerpTime() + time`), where `/worldborder set` replaces it.
    #[test]
    #[ignore = "daily"]
    fn worldborder_add_extends_a_change_in_flight() {
        let mut resources = None;
        let mut game = prepared_game(&mut resources);
        assert!(game.run_command("worldborder set 100 1000"));
        assert_eq!(game.border.remaining_ticks(), 1000);
        // Part way through the change, where the size is between its ends.
        for _ in 0..100 {
            game.border.tick().unwrap();
        }
        assert_eq!(game.border.remaining_ticks(), 900);
        let from = game.border.size();
        assert!(from < 59_999_968.0 && from > 100.0);
        // The new change starts from that size and lasts the ticks still
        // running plus the argument, not just the argument.
        assert!(game.run_command("worldborder add -50 5s"));
        assert_eq!(game.border.target_size(), from - 50.0);
        assert_eq!(game.border.remaining_ticks(), 1000);
        // `set` starts over from the size the border has now.
        assert!(game.run_command("worldborder set 200 40"));
        assert_eq!(game.border.target_size(), 200.0);
        assert_eq!(game.border.remaining_ticks(), 40);
    }

    /// `ServerLevel.getWorldBorder`: `/worldborder` in one level leaves the others' alone.
    #[test]
    #[ignore = "daily"]
    fn each_level_keeps_its_own_world_border() {
        let mut resources = None;
        let mut game = prepared_game(&mut resources);
        assert!(game.run_command("worldborder set 1000"));
        assert!(game.run_command("execute in minecraft:the_nether run worldborder set 100"));
        assert_eq!(game.dim, Dim::Overworld);
        assert_eq!(game.border.size(), 1000.0);
        let nether = game.levels.parked_border(Dim::Nether).unwrap();
        assert_eq!(nether.size(), 100.0);
    }

    struct LogCapture;
    static LOGGED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    impl log::Log for LogCapture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            LOGGED.lock().unwrap().push(record.args().to_string());
        }
        fn flush(&self) {}
    }

    /// 300 slow ticks in ten seconds log no Sunrise slow-tick line, and the ring holds the last 256.
    #[test]
    #[ignore = "daily"]
    fn slow_ticks_enter_the_ring_and_not_the_log() {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        use sunrise_client::server_perf::{ServerPerf, TickDetail, WorldCounts};
        log::set_logger(&LogCapture).expect("no other logger in this test run");
        log::set_max_level(log::LevelFilter::Info);
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
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        options.spikes = Some(0.0);
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        let mut perf = ServerPerf::new(0.0, 0.0);
        for i in 0..300 {
            let mut detail = None;
            let started = Instant::now();
            game.tick_server_spans(false, |timing| {
                detail = Some(TickDetail {
                    cpu_ms: timing.cpu_ms,
                    spans: timing.raw,
                });
            })
            .unwrap();
            let wall_ms = started.elapsed().as_secs_f32() * 1000.0;
            perf.tick(
                f64::from(i) / 30.0,
                wall_ms,
                detail,
                WorldCounts::default,
                Vec::new,
            );
        }
        let kept: Vec<u64> = perf.slow_records().map(|r| r.counter).collect();
        assert_eq!(kept, (45..=300).collect::<Vec<u64>>());
        let logged = LOGGED.lock().unwrap();
        assert!(
            logged.iter().all(|line| !line.contains("slow tick")),
            "{logged:?}"
        );
    }

    /// A game on a scratch world folder, the way `dedicated.rs` opens one
    /// (`load_world` with an absolute folder).
    fn open_game<'a>(
        resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
        root: &std::path::Path,
        home: &std::path::Path,
        world: &std::path::Path,
    ) -> Game<'a> {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            Vec::new(),
            None,
            root,
            &settings,
            Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        // The game keeps a reference to its resources for the whole test.
        let resources = resources.insert(loaded.resources);
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.to_path_buf();
        options.game_dir = Some(home.to_path_buf());
        // A fixed name: no generated sunrise-player.txt in the working directory.
        options.player_name = Some("H260".into());
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            resources,
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
        game.build_registries();
        game.set_allow_commands(true);
        game.native_lifecycle_open_started();
        let folder = world.to_string_lossy().into_owned();
        game.load_world(
            crate::game::world_loading::WorldRequest::new(
                &folder,
                Some("h260"),
                Some(260),
                true,
                None,
                std::sync::Arc::new(crate::game::api_values::game_rule_registry()),
            )
            .unwrap(),
        )
        .unwrap();
        game
    }

    #[test]
    #[ignore = "daily"]
    fn late_chunk_saves_use_their_opened_levels_disk_context() {
        use mod_api::{
            ChunkDataSaveEvent, ChunkDiskContext, ChunkDiskHost, ChunkDiskKind, ChunkDiskPhase,
            Hooks, Identifier, ModEntry,
        };
        use std::path::PathBuf;
        use std::sync::Mutex;
        use sunrise_client::chunk::Chunk;

        struct ObserveSaves(Arc<Mutex<Vec<ChunkDiskContext>>>);
        #[mod_api::__subscriptions]
        impl Hooks for ObserveSaves {
            fn chunk_data_save(&self, host: &dyn ChunkDiskHost, _: &mut ChunkDataSaveEvent<'_>) {
                self.0.lock().unwrap().push(host.job().clone());
            }
        }

        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        // Opening a world publishes the server's worldgen tables. Isolate that
        // publication from the other tests, even though the mod list is scoped.
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::levels::tests::late_chunk_saves_use_their_opened_levels_disk_context",
            "SUNRISE_CHUNK_DISK_CONTEXT_TEST_CHILD",
            &["1"],
        ) {
            return;
        }
        // The fresh child runs in its own scratch home.
        let home = std::env::current_dir().unwrap();
        let world = home.join("world");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "level_disk_context",
            Box::new(ObserveSaves(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let game = open_game(&mut resources, &root, &home, &world);
            let epoch = game.native_lifecycle.epoch().unwrap();

            // The streamer can send a late chunk straight to its writer, even
            // while its level is parked and the Overworld is in place.
            game.streaming
                .save_finished(Chunk::with_layout(2, -3, game.world.layout()));
            game.save.flush().unwrap();
            let nether = &game.levels.parked[&Dim::Nether];
            nether
                .streaming
                .save_finished(Chunk::with_layout(-4, 5, nether.world.layout()));
            nether.save.flush().unwrap();

            assert_eq!(
                seen.lock().unwrap().as_slice(),
                [
                    ChunkDiskContext::new(
                        epoch,
                        Identifier::parse("minecraft:overworld").unwrap(),
                        [2, -3],
                        ChunkDiskKind::Full,
                        ChunkDiskPhase::Save,
                    ),
                    ChunkDiskContext::new(
                        epoch,
                        Identifier::parse("minecraft:the_nether").unwrap(),
                        [-4, 5],
                        ChunkDiskKind::Full,
                        ChunkDiskPhase::Save,
                    ),
                ],
            );
        });
    }

    /// `MinecraftServer.prepareLevels` activates each level's saved tickets and
    /// waits for their ticking chunks without a player visiting either level.
    #[test]
    #[ignore = "daily"]
    fn built_in_levels_restore_saved_tickets_before_the_first_tick() {
        use mod_api::{
            Hooks, LevelRef, ModEntry, Registrar, RegistrationContext, ServerHost,
            TicketController, TicketValidation,
        };
        use std::sync::{
            OnceLock,
            atomic::{AtomicBool, Ordering},
        };
        use sunrise_client::mod_chunk_tickets::{ModTicket, ModTickets, TicketOwner};
        use sunrise_client::save::dimension_dir;

        const OWNER: [i32; 3] = [2, 64, 3];
        struct Quarry {
            saw_end: Arc<AtomicBool>,
            controller: Arc<OnceLock<TicketController>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Quarry {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                let saw_end = Arc::clone(&self.saw_end);
                let controller = registrar.register_ticket_controller(
                    "quarry",
                    Some(Arc::new(
                        move |host: &mut dyn ServerHost,
                              level: LevelRef,
                              validation: &mut dyn TicketValidation| {
                            if level.dimension_id() == "minecraft:the_nether"
                                && host.level("minecraft:the_end").is_some()
                            {
                                saw_end.store(true, Ordering::Relaxed);
                            } else {
                                validation.remove_block_tickets(OWNER);
                            }
                        },
                    )),
                );
                let _ = self.controller.set(controller);
            }
        }

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = tempfile::tempdir().unwrap();
        let world = home.path().join("world");
        let saw_end = Arc::new(AtomicBool::new(false));
        let controller_cell = Arc::new(OnceLock::new());
        let mods = vec![ModEntry::new(
            "startup_tickets",
            Box::new(Quarry {
                saw_end: saw_end.clone(),
                controller: controller_cell.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            {
                let mut resources = None;
                let mut game = open_game(&mut resources, &root, home.path(), &world);
                game.save_game_metadata().unwrap();
            }
            let nether = Dim::Nether;
            let end = Dim::End;
            let commanded_chunk = (4, -5);
            let controller = *controller_cell.get().unwrap();
            let controller_slot =
                mod_api::__private::game::ticket_controller_slot(controller).unwrap();
            let ticket = ModTicket {
                controller: controller_slot,
                owner: TicketOwner::Block(OWNER),
                chunk: (12, -13),
                natural_spawning: false,
            };
            SaveDir::new(dimension_dir(&world, end.id()), end.shape(), None)
                .save_chunk_tickets(&[commanded_chunk].into(), &[])
                .unwrap();
            let mut tickets = ModTickets::default();
            tickets.insert(ticket.clone());
            SaveDir::new(dimension_dir(&world, nether.id()), nether.shape(), None)
                .save_mod_tickets(&tickets, mod_api::__private::game::ticket_controller_id)
                .unwrap();

            let mut resources = None;
            let mut game = open_game(&mut resources, &root, home.path(), &world);
            assert!(!saw_end.load(Ordering::Relaxed), "validation has not run");
            assert!(game.levels.parked[&end].streaming.commanded().is_empty());
            // An early final save can run after the world is adopted but
            // before ticket activation. The originals on disk must survive.
            game.save_all_levels(false).unwrap();
            game.prepare_levels().unwrap();
            assert!(
                saw_end.load(Ordering::Relaxed),
                "Nether validator sees the End"
            );
            assert_eq!(game.dim, Dim::Overworld);
            let nether_level = &game.levels.parked[&nether];
            assert_eq!(
                nether_level
                    .streaming
                    .mod_tickets()
                    .of_controller(controller_slot)
                    .next(),
                Some(&ticket)
            );
            let end_level = &game.levels.parked[&end];
            assert!(end_level.streaming.commanded().contains(&commanded_chunk));
            let ticking = |game: &Game<'_>, dim, (x, z)| {
                let world = if dim == game.dim {
                    &game.world
                } else {
                    &game.levels.parked[&dim].world
                };
                crate::game::ticking_chunk(world, x, z)
            };
            assert!(ticking(&game, nether, ticket.chunk));
            assert!(ticking(&game, end, commanded_chunk));
            game.tick_server(false).unwrap();
            assert!(ticking(&game, nether, ticket.chunk));
            assert!(ticking(&game, end, commanded_chunk));
        });
    }

    /// `WorldDimensions.BUILTIN_ORDER` is independent of a saved player's dimension.
    #[test]
    #[ignore = "daily"]
    fn built_in_level_load_order_ignores_the_saved_player_dimension() {
        use mod_api::{Hooks, LevelLoadEvent, ModEntry, ServerHost};
        use std::sync::{Arc, Mutex};

        const BUILT_INS: [&str; 3] = [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ];
        struct Observer(Arc<Mutex<Vec<(String, [bool; 3])>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn level_load(&self, host: &mut dyn ServerHost, event: &mut LevelLoadEvent<'_>) {
                assert_eq!(host.dimension(), event.dimension);
                let available = BUILT_INS.map(|id| host.level(id).is_some());
                self.0
                    .lock()
                    .unwrap()
                    .push((event.dimension.to_owned(), available));
            }
        }

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = tempfile::tempdir().unwrap();
        let world = home.path().join("world");
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, home.path(), &world);
            game.level.dimension = "minecraft:the_nether".into();
            game.level.position = Some([0.0, 64.0, 0.0]);
            game.level.write(&world, game.res.items()).unwrap();
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "startup_order",
            Box::new(Observer(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let game = open_game(&mut resources, &root, home.path(), &world);
            assert_eq!(game.dim, Dim::Nether);
        });
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), BUILT_INS.len());
        assert_eq!(seen[0], (BUILT_INS[0].into(), [true, false, false]));
        let mut loaded = [true, false, false];
        let mut names: Vec<&str> = vec![seen[0].0.as_str()];
        for (dim, available) in &seen[1..] {
            let idx = BUILT_INS
                .iter()
                .position(|&id| id == dim)
                .expect("built-in dimension");
            assert!(!loaded[idx], "duplicate level_load for {dim}");
            loaded[idx] = true;
            assert_eq!(*available, loaded);
            names.push(dim.as_str());
        }
        names.sort_unstable();
        let mut expected = BUILT_INS;
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    /// Vanilla 26.3 (`MinecraftServer.createLevels` / `MinecraftServer.levels` /
    /// `ServerLevel.noSave` / `ChunkMap.processUnloads`): built-in levels open
    /// once at startup, stay open and addressable across idle ticks while
    /// unloading released chunks (except during server pause or `/save-off`,
    /// but still unloading during `/tick freeze`), keep ticketed chunks loaded,
    /// do not fire a second `level_load` on a later first visit, and fire
    /// `level_unload` once on normal server shutdown.
    #[test]
    #[ignore = "daily"]
    fn built_in_levels_stay_open_across_idle_ticks_until_shutdown() {
        use crate::game::{
            lan::LanSettings, lan_actions::GuestCommand, native_entity_refs::tests::run_fresh_cases,
        };
        use mod_api::{Hooks, LevelLoadEvent, LevelUnloadEvent, ModEntry, ServerHost};
        use std::sync::{Arc, Mutex};
        use sunrise_client::{
            lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
            server::user_lists::{BanList, IpBanList, OpList, WhiteList},
        };

        if run_fresh_cases(
            "game::levels::tests::built_in_levels_stay_open_across_idle_ticks_until_shutdown",
            "SUNRISE_BUILT_IN_IDLE_LIFETIME_CASE",
            &["built-ins"],
        ) {
            return;
        }

        const BUILT_INS: [&str; 3] = [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ];
        struct Observer {
            loads: Arc<Mutex<Vec<String>>>,
            unloads: Arc<Mutex<Vec<String>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn level_load(&self, _: &mut dyn ServerHost, event: &mut LevelLoadEvent<'_>) {
                self.loads.lock().unwrap().push(event.dimension.to_owned());
            }
            fn level_unload(&self, _: &mut dyn ServerHost, event: &mut LevelUnloadEvent<'_>) {
                self.unloads
                    .lock()
                    .unwrap()
                    .push(event.dimension.to_owned());
            }
        }

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = tempfile::tempdir().unwrap();
        let world = home.path().join("world");
        let loads = Arc::new(Mutex::new(Vec::new()));
        let unloads = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "builtin_lifetime",
            Box::new(Observer {
                loads: loads.clone(),
                unloads: unloads.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, home.path(), &world);
            let initial_loads = loads.lock().unwrap().clone();
            assert_eq!(
                initial_loads.first().map(String::as_str),
                Some("minecraft:overworld")
            );
            let mut sorted_loads: Vec<&str> = initial_loads.iter().map(String::as_str).collect();
            sorted_loads.sort_unstable();
            let mut sorted_builtins = BUILT_INS;
            sorted_builtins.sort_unstable();
            assert_eq!(sorted_loads, sorted_builtins);
            assert!(unloads.lock().unwrap().is_empty());

            // Configure as a dedicated server with real commands (`/save-off`,
            // `/save-on`, `/tick freeze`, `/forceload`) and a local placed guest
            // (no network listener) so ticks, visits and shutdown run normally.
            game.options.execution_role =
                crate::game::execution_role::ExecutionRole::DedicatedServer;
            game.options.dedicated = Some(Vec::new());
            game.settings.render_distance = 2;
            game.settings.simulation_distance = 2;
            game.streaming.radius = -1;
            game.server_lists = Some(Arc::new(Mutex::new(crate::game::dedicated::Lists {
                ops: OpList::new(home.path().join("ops.json")),
                white: WhiteList::new(home.path().join("whitelist.json")),
                bans: BanList::new(home.path().join("banned-players.json")),
                ip_bans: IpBanList::new(home.path().join("banned-ips.json")),
                use_whitelist: false,
                enforce_whitelist: false,
                op_level: 4,
                player_idle_timeout: 0,
                stop: false,
                autosave: true,
            })));
            game.ui.dispatcher = crate::game::chat::SessionDispatcher(Arc::new(
                sunrise_client::command::vanilla::build_for(false, true),
            ));
            game.lan_host(
                LanSettings {
                    port: 0,
                    guest_commands: true,
                    force_game_mode: false,
                },
                NetConfig {
                    listen: false,
                    announce: false,
                    ..NetConfig::lan(0, "builtin-lifetime".into(), 2)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: true,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: Dim::Overworld.id().into(),
                    allow_flight: false,
                    seed: game.level.seed,
                    max_players: 2,
                    integrated: false,
                    levels: game.level_shapes.clone(),
                    level_seeds: game.levels.runtime.seeds(),
                },
            )
            .unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            let joined = Joined {
                conn: 1,
                uuid: 1,
                name: "Visitor".into(),
                properties: Vec::new(),
                info: sunrise_protocol::packets::configuration::ClientInformation::new(2),
                brand: "sunrise".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(std::sync::atomic::AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            };
            game.next_entity += 1;
            let mut guest = Guest::new(joined, game.next_entity as i32, [0.5, 80.0, 0.5], 0);
            guest.placed = true;
            guest.loaded = true;
            guest.dimension = Dim::Overworld.id().into();
            // Creative: no fall or suffocation where the teleports below land.
            guest.game_mode = sunrise_client::lan::guest::CREATIVE;
            guest.motion = Some(Box::new(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(0.5, 80.0, 0.5),
                )
                .unwrap(),
            ));
            guest.max_unacknowledged = 1000;
            game.lan.as_mut().unwrap().hub.guests.push(guest);

            game.prepare_levels().unwrap();
            game.native_lifecycle_world_ready();

            // Force one chunk in the Nether (to release later) and one in the End
            // (to keep ticketed), then drive real server ticks until both load.
            assert!(game.run_command("execute in minecraft:the_nether run forceload add 0 0"));
            assert!(game.run_command("execute in minecraft:the_end run forceload add 0 0"));
            let mut loaded = false;
            for _ in 0..2000 {
                game.tick_server(false).unwrap();
                let nether_ready = game.levels.parked.get(&Dim::Nether).is_some_and(|p| {
                    p.world.chunks.contains_key(&(0, 0)) && p.world.entities_pending.is_empty()
                });
                let end_ready = game.levels.parked.get(&Dim::End).is_some_and(|p| {
                    p.world.chunks.contains_key(&(0, 0)) && p.world.entities_pending.is_empty()
                });
                if nether_ready && end_ready {
                    loaded = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(loaded, "forced chunks in Nether and End did not load");

            // Release the Nether ticket while keeping the End chunk ticketed.
            assert!(game.run_command("execute in minecraft:the_nether run forceload remove 0 0"));

            // Server pause (`paused = true`) must not drain the parked Nether chunk.
            for _ in 0..16 {
                game.tick_server(true).unwrap();
            }
            assert!(
                game.levels.parked[&Dim::Nether]
                    .world
                    .chunks
                    .contains_key(&(0, 0)),
                "paused server must keep parked Nether chunk resident"
            );

            // `/save-off` must keep the parked Nether chunk resident until `/save-on`.
            assert!(game.run_command("save-off"));
            for _ in 0..16 {
                game.tick_server(false).unwrap();
            }
            assert!(
                game.levels.parked[&Dim::Nether]
                    .world
                    .chunks
                    .contains_key(&(0, 0)),
                "save-off must keep parked Nether chunk resident"
            );
            assert!(game.run_command("save-on"));

            // `/tick freeze` still allows unticketed chunks to unload. Eligibility,
            // not a dimension idle timer, controls the sweep.
            assert!(game.run_command("tick freeze"));
            game.tick_server(false).unwrap();
            assert!(game.levels.parked.contains_key(&Dim::Nether));
            assert!(game.levels.parked.contains_key(&Dim::End));
            assert!(
                game.levels.parked[&Dim::Nether].world.chunks.is_empty(),
                "released Nether chunk unloaded while parked under /tick freeze"
            );
            assert!(
                !game.run_command_quietly("execute in minecraft:the_nether if loaded 0 64 0"),
                "released Nether chunk reports unloaded"
            );
            assert!(
                game.run_command_quietly("execute in minecraft:the_end if loaded 0 64 0"),
                "ticketed End chunk stays loaded"
            );
            assert!(
                game.run_command("execute in minecraft:the_nether run worldborder get"),
                "idle built-in Nether stays addressable"
            );
            assert!(
                unloads.lock().unwrap().is_empty(),
                "built-in levels stay open after their idle chunk sweep"
            );
            assert!(game.run_command("tick unfreeze"));

            // The guest's first visits to the Nether and the End after the idle
            // period cross levels as a player's own `/execute in ... run tp`
            // does (`ServerPlayer.teleport`): no second `level_load`.
            for dim in [Dim::Nether, Dim::End, Dim::Overworld] {
                assert!(game.run_guest_command(GuestCommand {
                    conn: 1,
                    command: format!("execute in {} run tp @s 0.5 100 0.5", dim.id()),
                    permission: 4,
                    integrated: false,
                    quiet: false,
                }));
                let mut arrived = false;
                for _ in 0..2000 {
                    game.tick_server(false).unwrap();
                    let guest = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
                    if guest.dimension == dim.id() && !game.on_server_trip(1) {
                        arrived = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(arrived, "the guest never arrived in {dim:?}");
            }
            assert_eq!(*loads.lock().unwrap(), initial_loads);
            assert!(unloads.lock().unwrap().is_empty());

            // Normal server shutdown saves worlds and fires `level_unload` once each,
            // in the same creation order as `level_load`.
            game.run_server_lifecycle(true, |_| {
                Ok(crate::game::server_lifecycle::ServerRunOutcome::Orderly)
            })
            .unwrap();
            assert_eq!(*unloads.lock().unwrap(), initial_loads);
        });
    }

    /// Expected values from the official 26.3 dedicated server restarted on the
    /// same world: each level's `world_border` SavedData is read when the level
    /// is made, so the Overworld's saved border is in place for the first
    /// command, and a first `/worldborder set` is not overwritten later.
    #[test]
    #[ignore = "daily"]
    fn a_reopened_world_has_each_levels_saved_border_from_the_start() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("h260");
        let world = home.join("saves/h260");
        // A new world each run: a border saved by the last one would already be 50.
        let _ = std::fs::remove_dir_all(&home);
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // A server ticks before it saves.
            game.tick_world(true).unwrap();
            assert!(game.run_command("worldborder set 50"));
            assert!(game.run_command("execute in minecraft:the_nether run worldborder set 100"));
            game.save_game().unwrap();
        }
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        let mut failures = Vec::new();
        if game.border.size() != 50.0 {
            failures.push(format!("in place after the open: {}", game.border.size()));
        }
        assert!(game.run_command("worldborder set 70"));
        game.tick_world(true).unwrap();
        assert!(game.run_command("execute in minecraft:the_nether run worldborder get"));
        assert!(game.run_command("execute in minecraft:the_end run worldborder get"));
        assert_eq!(game.dim, Dim::Overworld);
        let sizes = (
            game.border.size(),
            game.levels.parked_border(Dim::Nether).map(|b| b.size()),
            game.levels.parked_border(Dim::End).map(|b| b.size()),
        );
        if sizes != (70.0, Some(100.0), Some(59_999_968.0)) {
            failures.push(format!("after the first set and a tick: {sizes:?}"));
        }
        assert!(failures.is_empty(), "reopened borders: {failures:#?}");
    }

    /// Added controls (H260, after the baseline): a new world, and a read and a
    /// set before the first tick that survive a tick, a save and another reopen.
    #[test]
    #[ignore = "daily"]
    fn reopened_borders_keep_through_a_tick_and_a_save_and_a_new_world_has_defaults() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
            .join("h260-controls");
        // A new world each run, as the first open expects.
        let _ = std::fs::remove_dir_all(&home);
        let world = home.join("saves/h260");
        let sizes = |game: &mut Game<'_>| {
            assert!(game.run_command("execute in minecraft:overworld run worldborder get"));
            assert!(game.run_command("execute in minecraft:the_nether run worldborder get"));
            assert!(game.run_command("execute in minecraft:the_end run worldborder get"));
            let parked = |d| game.levels.parked_border(d).map(|b| b.size());
            let size = |d: Dim| {
                if game.dim == d {
                    Some(game.border.size())
                } else {
                    parked(d)
                }
            };
            (size(Dim::Overworld), size(Dim::Nether), size(Dim::End))
        };
        const DEFAULT: f64 = 59_999_968.0;
        let mut failures = Vec::new();
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // A new world: every level at the default.
            if game.border.size() != DEFAULT {
                failures.push(format!("new world in place: {}", game.border.size()));
            }
            game.tick_world(true).unwrap();
            let got = sizes(&mut game);
            if got != (Some(DEFAULT), Some(DEFAULT), Some(DEFAULT)) {
                failures.push(format!("new world: {got:?}"));
            }
            assert!(game.run_command("worldborder set 50"));
            assert!(game.run_command("execute in minecraft:the_nether run worldborder set 100"));
            game.save_game().unwrap();
        }
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // Before the first tick: the read, then the set.
            if game.border.size() != 50.0 {
                failures.push(format!("first read before a tick: {}", game.border.size()));
            }
            assert!(game.run_command("worldborder set 70"));
            if game.border.size() != 70.0 {
                failures.push(format!("first set before a tick: {}", game.border.size()));
            }
            game.tick_world(true).unwrap();
            let got = sizes(&mut game);
            if got != (Some(70.0), Some(100.0), Some(DEFAULT)) {
                failures.push(format!("after a tick: {got:?}"));
            }
            game.save_game().unwrap();
        }
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            let got = sizes(&mut game);
            if got != (Some(70.0), Some(100.0), Some(DEFAULT)) {
                failures.push(format!("after the save and another reopen: {got:?}"));
            }
        }
        assert!(failures.is_empty(), "reopened borders: {failures:#?}");
    }

    /// `ImposterProtoChunk(…, false)`: a new chunk's features read a full
    /// neighbour but their ordinary `setBlockState` writes do not change it,
    /// while `BulkSectionAccess` (ores) writes its sections. Expected values
    /// from the official 26.3 server on the constructed 1.20.4 world
    /// (`H077_OLD_WORLD`: full old chunks, their partial ring removed), where
    /// chunks 13..14 at z 0 generate next to the full chunk (12, -1).
    #[test]
    #[ignore = "needs SUNRISE_TEST_RESOURCES, a scratch SUNRISE_HOME and the constructed H077_OLD_WORLD"]
    fn features_write_a_full_neighbour_only_through_its_sections() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("h078");
        let world = home.join("saves/h078");
        let source = std::path::PathBuf::from(std::env::var_os("H077_OLD_WORLD").unwrap());
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else if entry.file_name() != "session.lock" {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy(&source, &world);
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        // The world's level.dat has cheats off: the open read it after open_game's setting.
        game.set_allow_commands(true);
        assert!(game.run_command("forceload add 208 0 239 15"));
        let mut loaded = false;
        // As the dedicated server's loop: a tick, then the chunk streaming.
        for _ in 0..2000 {
            game.tick_world(true).unwrap();
            game.update_world();
            std::thread::sleep(std::time::Duration::from_millis(10));
            if game.run_command_quietly("execute if loaded 215 64 8") {
                loaded = true;
                break;
            }
        }
        assert!(loaded, "the new chunk (13, 0) was not generated");
        game.save_game().unwrap();
        let mut failures = Vec::new();
        for (p, want) in [
            // A spruce tree and a mushroom of the new chunks: ordinary writes.
            ([207, 68, -10], "minecraft:air"),
            ([207, 69, -6], "minecraft:air"),
            ([202, 69, -5], "minecraft:air"),
            // A gold ore blob: section writes.
            ([207, -32, -12], "minecraft:deepslate_gold_ore"),
        ] {
            let got = game.block_name(p);
            if &*got != want {
                failures.push(format!("{p:?}: {got}, expected {want}"));
            }
        }
        assert!(
            failures.is_empty(),
            "the full chunk (12, -1): {failures:#?}"
        );
    }

    /// `BlendingData.pack` of an old chunk whose border columns a new chunk's
    /// generation calculated (`getOrUpdateBlendingData`), as the official 26.3
    /// server saves it: a constructed 1.20.4 world (full old chunks, their
    /// partial ring removed, `H077_OLD_WORLD`) where chunk (13, 0) is new.
    #[test]
    #[ignore = "needs SUNRISE_TEST_RESOURCES, a scratch SUNRISE_HOME and the constructed H077_OLD_WORLD"]
    fn an_old_chunk_saves_the_heights_its_new_neighbour_calculated() {
        use sunrise_core::nbt::Tag;
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("h077");
        let world = home.join("saves/h077");
        let source = std::path::PathBuf::from(std::env::var_os("H077_OLD_WORLD").unwrap());
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else if entry.file_name() != "session.lock" {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy(&source, &world);
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // The world's level.dat has cheats off: the open read it after open_game's setting.
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 208 0 239 15"));
            let mut loaded = false;
            // As the dedicated server's loop: a tick, then the chunk streaming.
            for _ in 0..2000 {
                game.tick_world(true).unwrap();
                game.update_world();
                std::thread::sleep(std::time::Duration::from_millis(10));
                if game.run_command_quietly("execute if loaded 215 64 8") {
                    loaded = true;
                    break;
                }
            }
            assert!(loaded, "the new chunk (13, 0) was not generated");
            game.save_game().unwrap();
        }
        let save = sunrise_client::save::SaveDir::new(
            sunrise_client::save::dimension_dir(&world, "minecraft:overworld"),
            sunrise_client::chunk::LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        let blending = |x, z| {
            save.raw_chunk(x, z)
                .and_then(|r| r.tag.get("blending_data").cloned())
        };
        let heights = |tag: &Tag| match tag.get("heights") {
            Some(Tag::List(list)) => Some(
                list.iter()
                    .map(|v| match v {
                        Tag::Float(bits) => *bits,
                        other => panic!("height {other:?}"),
                    })
                    .collect::<Vec<u32>>(),
            ),
            _ => None,
        };
        let sections = |tag: &Tag| {
            (
                tag.get("min_section").cloned(),
                tag.get("max_section").cloned(),
            )
        };
        let mut failures = Vec::new();
        let mut want = vec![f32::MAX.to_bits(); 12];
        want.extend([70.0f32, 70.0, 69.0, 68.0].map(f32::to_bits));
        match blending(12, 0) {
            Some(tag) => {
                if heights(&tag) != Some(want.clone()) {
                    failures.push(format!("(12, 0) heights {:?}", heights(&tag)));
                }
                if sections(&tag) != (Some(Tag::Int(-4)), Some(Tag::Int(20))) {
                    failures.push(format!("(12, 0) sections {:?}", sections(&tag)));
                }
            }
            None => failures.push("(12, 0) has no blending_data".into()),
        }
        // No calculation reached these: the compound stays, without heights.
        for (x, z) in [(11, 0), (0, 0)] {
            match blending(x, z) {
                Some(tag) => {
                    if heights(&tag).is_some()
                        || sections(&tag) != (Some(Tag::Int(-4)), Some(Tag::Int(20)))
                    {
                        failures.push(format!("({x}, {z}) {tag:?}"));
                    }
                }
                None => failures.push(format!("({x}, {z}) has no blending_data")),
            }
        }
        assert!(failures.is_empty(), "saved blending data: {failures:#?}");
    }

    /// Added control (H077, after the baseline): the exact cache state the save
    /// reads. After the generation of (13, 0), the Overworld folder's lookup
    /// holds the calculated data of the edge chunk (12, 0), with the official
    /// server's heights; an old neighbour without new sides holds no height,
    /// and a far chunk no entry.
    #[test]
    #[ignore = "needs SUNRISE_TEST_RESOURCES, a scratch SUNRISE_HOME and the constructed H077_OLD_WORLD"]
    fn the_overworld_cache_holds_the_edge_chunks_calculated_heights() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let home =
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("h077-cache");
        let world = home.join("saves/h077");
        let source = std::path::PathBuf::from(std::env::var_os("H077_OLD_WORLD").unwrap());
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else if entry.file_name() != "session.lock" {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy(&source, &world);
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        game.set_allow_commands(true);
        assert!(game.run_command("forceload add 208 0 239 15"));
        let mut loaded = false;
        for _ in 0..2000 {
            game.tick_world(true).unwrap();
            game.update_world();
            std::thread::sleep(std::time::Duration::from_millis(10));
            if game.run_command_quietly("execute if loaded 215 64 8") {
                loaded = true;
                break;
            }
        }
        assert!(loaded, "the new chunk (13, 0) was not generated");
        assert_eq!(game.dim, Dim::Overworld);
        let mut want = vec![f32::MAX.to_bits(); 12];
        want.extend([70.0f32, 70.0, 69.0, 68.0].map(f32::to_bits));
        let edge = game
            .save
            .calculated_blending(12, 0)
            .expect("(12, 0) was calculated");
        let (min, max, heights) = edge.pack();
        assert_eq!((min, max), (-4, 20));
        assert_eq!(heights.map(|h| h.map(f32::to_bits).to_vec()), Some(want));
        // (11, 0) borders no new chunk: the generation's calculation adds no height
        // to it (it may have an entry), so it packs without heights, as vanilla's.
        let inner = game.save.calculated_blending(11, 0);
        assert!(
            inner.as_ref().is_none_or(|data| data.pack().2.is_none()),
            "{inner:?}"
        );
        // (0, 0) is far from the new chunks: nothing calculated it.
        assert!(game.save.calculated_blending(0, 0).is_none());
    }
}

#[cfg(test)]
mod request_save_tests {
    use super::{Dim, Levels};

    #[test]
    #[ignore = "daily"]
    fn requested_save_waits_without_consuming_the_queue_while_lifted() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.levels.request_save(Dim::Overworld);
            let pass = game.dropped.lift();
            game.run_requested_saves();
            assert_eq!(game.levels.requested_saves, [Dim::Overworld]);
            drop(pass);
            game.run_requested_saves();
            assert!(game.levels.requested_saves.is_empty());
        });
    }

    /// A level asked for twice is queued once, and clearing the levels drops the queue.
    #[test]
    fn a_level_asked_for_twice_is_saved_once() {
        let mut levels = Levels::default();
        levels.request_save(Dim::Nether);
        levels.request_save(Dim::Nether);
        levels.request_save(Dim::End);
        assert_eq!(levels.requested_saves, [Dim::Nether, Dim::End]);
        levels.clear();
        assert!(levels.requested_saves.is_empty());
    }
}

impl Game<'_> {
    /// `Game::tick` as a server runs it for its first level: no window lead, which only the window's `Game` has.
    fn tick_server_first(&mut self, running: bool) -> Result<()> {
        self.expire_client_cracks();
        if self.agent.hold_server {
            return Ok(());
        }
        self.abort_open_transactions();
        self.update_interaction_ranges();
        self.tick_first_level(running)
    }

    /// `ServerLevel.tick`: advance the world border, then the natural weather cycle.
    fn tick_level_environment(&mut self, dim: Dim) {
        self.tick_worldborder(dim);
        self.tick_level_weather(dim);
    }

    /// A server's tick of its first active level, with the shared clock and autosave.
    pub(super) fn tick_first_level(&mut self, running: bool) -> Result<()> {
        // Only a server ticks its own world from here: its players are guests, swapped in for
        // their turns (`as_guest`), and the window's player is a client of it.
        self.ticks += 1;
        self.repro_tick();
        span("mods_inventory_ticks", || self.native_inventory_ticks());
        if running {
            self.tick_level_environment(self.dim);
            self.world.game_time += 1;
        } else if self.clock.still() {
            // Under `SUNRISE_STILL` a frozen world's rain and thunder sit at their levels, as a
            // shader pack's smoothed values do, so `/weather rain` shows full rain without
            // stepping the world (and its mobs) through the fade.
            self.world.weather.settle_levels();
        }
        // ClientLevel.tick: the sky flash fades in a frozen level too.
        self.tick_weather_effects();
        let _ = span("player_autosaves", || self.finish_player_autosaves(false));
        // MinecraftServer.tickServer's countdown, then autoSave: three
        // hundred seconds of ticks at the rate the server runs at now.
        self.ticks_until_autosave -= 1;
        self.autosaving = self.ticks_until_autosave <= 0;
        if let Some(serving) = &self.serving {
            serving.set_saving(self.autosaving);
        }
        if self.autosaving {
            span("autosave", || self.save_game_with(false))?;
            self.ticks_until_autosave = self.next_autosave_interval();
            if let Some(serving) = &self.serving {
                serving.set_saving(false);
            }
        }
        if running {
            self.advance_clock();
            // `ServerLevel.tickTime`, once the clock has moved on.
            span("scheduled_functions", || self.run_scheduled_functions());
        }
        span("tick_command_extras", || self.tick_command_extras());
        self.tick_world(running)
    }
}
