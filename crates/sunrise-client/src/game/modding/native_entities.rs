//! Entities for native server mods (docs/modding/src/how-to/native-mods.md, "Entities"): spawn
//! any entity from its compound as `/summon` does through [`LevelRef::summon`], and move, show or remove
//! the entities Sunrise keeps without simulating them (displays, markers,
//! interactions); `native_display_refs.rs` poses displays.
#[cfg(test)]
use super::commands::{data::summon_position_ok, place_summoned};
use super::{Game, commands::Target};
use mod_api::{EntityRef, LevelRef, LevelWriteError};
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Compound;
#[cfg(test)]
use sunrise_core::nbt::Tag;

impl super::commands::OtherEntity {
    /// Physical query bounds, not display rendering/culling dimensions.
    pub(in crate::game) fn display_marker_or_interaction_bounds(
        &self,
    ) -> Option<sunrise_core::geometry::Aabb> {
        let dimensions = self.vanilla_dimensions()?;
        let vanilla = if let Some(settings) = self.interaction {
            let mut interaction = super::interactions::RemoteInteraction::new([
                self.position.x,
                self.position.y,
                self.position.z,
            ]);
            interaction.width = settings.width;
            interaction.height = settings.height;
            interaction.response = settings.response;
            interaction.bounds()
        } else {
            // Display and Marker have zero-sized physical dimensions.
            sunrise_core::geometry::Aabb::new(self.position, self.position)
        };
        Some(sunrise_client::pose::SizeAnswer::sized_box(
            self.size_answer,
            dimensions,
            self.position,
            vanilla,
        ))
    }

    /// `Entity.getDimensions(pose)` of a display, a marker or an interaction this list holds,
    /// the kinds whose box follows their size here: an interaction's
    /// `EntityDimensions.scalable(width, height)`, none for a display or a marker. None for
    /// every other kind the list keeps without simulating it (a leash knot, an eye of ender),
    /// which isn't asked.
    pub(in crate::game) fn vanilla_dimensions(&self) -> Option<[f32; 3]> {
        if !matches!(
            self.kind.as_str(),
            "minecraft:block_display"
                | "minecraft:item_display"
                | "minecraft:text_display"
                | "minecraft:marker"
                | "minecraft:interaction"
        ) {
            return None;
        }
        Some(match self.interaction {
            Some(settings) => [settings.width, settings.height, settings.height * 0.85],
            None => [0.0; 3],
        })
    }
}

/// The id of a summoned stack's root: its command target's, else `first`,
/// the first id handed out, when an entity no command target stands for
/// (`live`) holds it.
fn root_id(target: Option<Target>, first: u32, live: impl Fn(u32) -> bool) -> Option<u32> {
    match target {
        Some(t) => target_id(t),
        None => live(first).then_some(first),
    }
}

/// The entity id a summoned stack's root is known by.
pub(super) fn target_id(t: Target) -> Option<u32> {
    match t {
        Target::Mob(id)
        | Target::Item(id)
        | Target::Orb(id)
        | Target::Projectile(id)
        | Target::Stand(id)
        | Target::Frame(id)
        | Target::Painting(id)
        | Target::Boat(id)
        | Target::Cart(id)
        | Target::Other(id)
        | Target::Fangs(id)
        | Target::Crystal(id)
        | Target::Dragon(id)
        | Target::DragonPart(id)
        | Target::Tnt(id)
        | Target::Cushion(id)
        | Target::Falling(id)
        | Target::Rocket(id)
        | Target::Trident(id)
        | Target::Spawner(id)
        | Target::Cloud(id)
        | Target::Bolt(id)
        | Target::Hook(id)
        | Target::Knot(id)
        | Target::General(id) => Some(id),
        Target::Player | Target::Guest(_) => None,
    }
}

impl Game<'_> {
    /// Spawns an entity of `kind` (`minecraft:item_display`) at `pos` in the
    /// server level `level` names from `nbt` (its compound, as `/summon
    /// <kind> <pos> <nbt>` takes it; passengers included): its [`EntityRef`].
    /// Routes parked dimensions by handle and refuses with
    /// [`LevelWriteError::InTransaction`] when summoning into a parked level
    /// while a Host transaction is open. Returns `Ok(None)` for a position
    /// outside `/summon`'s bounds, a chunk that isn't loaded, a kind Sunrise
    /// can't load, one a mod kept out (`native_entity_join_level`), or with
    /// entity ids exhausted. Inside a transaction the handle comes back at
    /// once: a join a mod cancels at the root commit leaves its handle stale.
    pub(super) fn level_summon(
        &mut self,
        level: LevelRef,
        kind: &str,
        pos: [f64; 3],
        nbt: Compound,
    ) -> Result<Option<EntityRef>, LevelWriteError> {
        self.with_server_level_mut(
            level,
            LevelWriteError::Unavailable,
            LevelWriteError::Stale,
            Some(LevelWriteError::InTransaction),
            |game| {
                let epoch = game
                    .native_lifecycle
                    .epoch()
                    .ok_or(LevelWriteError::Unavailable)?;
                if game.entity_ids_exhausted() {
                    return Ok(None);
                }
                let chunk = ((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4);
                if !game.world.chunks.contains_key(&chunk) {
                    return Ok(None);
                }
                let first = game.next_entity + 1;
                let deferred = (game.tnts.len(), game.falling.len(), game.rockets.len());
                let Ok(target) = game.summon_from_compound(kind, pos, nbt) else {
                    return Ok(None);
                };
                // These roots take their id as they load, and a rocket has no command
                // Target variant. Select the newly appended root, never an older waiting entity.
                let root = match kind {
                    "minecraft:tnt" if deferred.0 < game.tnts.len() => {
                        Some(super::entity_residency::assign_resident_id!(
                            game.tnts,
                            deferred.0,
                            e,
                            &mut game.next_entity,
                            None
                        ))
                    }
                    "minecraft:falling_block" if deferred.1 < game.falling.len() => {
                        Some(super::entity_residency::assign_resident_id!(
                            game.falling,
                            deferred.1,
                            f,
                            &mut game.next_entity,
                            Some(&f.attachment_lifetime)
                        ))
                    }
                    "minecraft:firework_rocket" if deferred.2 < game.rockets.len() => {
                        Some(super::entity_residency::assign_resident_id!(
                            game.rockets,
                            deferred.2,
                            e,
                            &mut game.next_entity,
                            None
                        ))
                    }
                    _ => None,
                };
                // Otherwise the root is the first entity spawned, also for kinds no
                // command target stands for (experience orbs, end crystals, cushions). An id handed out to an
                // entity a mod then kept out is not given back, so the root must be
                // there.
                let id = root.or_else(|| {
                    root_id(target, first, |id| {
                        game.orbs.iter().any(|o| o.id == id)
                            || game.crystals.iter().any(|c| c.id == id)
                            || game.cushions.iter().any(|c| c.id == id)
                    })
                });
                Ok(id.map(|id| EntityRef::new(epoch, id)))
            },
        )
    }
}

/// The entities the walk does not hold: Sunrise keeps these in lists of their own.
#[derive(Clone, Copy)]
enum Apart {
    Crystal(usize),
    Cushion(usize),
    Painting(usize),
    Frame(usize),
    Other(usize),
    Dragon,
}

/// One entity a mod is told of: in the walk, or apart from it.
#[derive(Clone, Copy)]
enum Slot {
    Walk(super::entity::EntityHandle),
    Apart(Apart),
}

/// Entities joining and leaving the level (`native_entity_join_level`,
/// `native_entity_leave_level`), for every kind in the entity walk and the
/// families kept apart from it. Kept out of `native_hooks.rs`, whose lines may not hold macros.
impl Game<'_> {
    /// Whether an entity may have joined since the last announcement. A join
    /// in another level may cause one harmless walk of this level.
    fn joins_pending(&self) -> bool {
        sunrise_client::join_seq::JoinSeq::latest() > self.persist.joined_seq
    }
    /// Tells `native_entity_join_level` for every still-unannounced entity,
    /// of any kind, in insertion order. A mod that cancels takes
    /// the entity back out here,
    /// before it ticks again, and `/summon` of it fails (`joins_canceled`).
    /// `loaded_from_disk` is for the entities that came with a chunk. Returns
    /// whether `watch` (an id, or 0) is still in the level. Walks the lists
    /// with static dispatch and allocates nothing but the kind name of an
    /// entity it announces; nothing runs when no mod is loaded, on a client
    /// of a server (which tells nothing), or when nothing is new.
    /// Inside a transaction open in this level, construction is still told now, and the join
    /// at its root commit (`transactions/entities.rs`), with this call's `loaded_from_disk`, in
    /// insertion order and before the transaction's removals take effect; the entity is kept
    /// until then. The same holds for a call a listener makes while that commit is told: its
    /// joins are told after the batch being told. A `through` is the commit's own flush of one
    /// batch, never held.
    pub(super) fn native_joins_flush(&mut self, loaded_from_disk: bool, watch: u32) -> bool {
        self.flush_joins(loaded_from_disk, watch, &[], None)
    }
    /// `addFreshEntity` at a call site that has a specific new row. It does not wait
    /// for an unrelated item still being decided by `item_toss`. The level's journal
    /// can hold the join, but construction still happens at the call.
    pub(super) fn announce_fresh_join(
        &mut self,
        handle: super::entity::EntityHandle,
        id: u32,
    ) -> bool {
        if self.remote.is_some() || mod_api::__private::mods().is_empty() {
            return true;
        }
        if self.hold_announcement(super::transactions::Announcement::FreshJoin { handle, id }) {
            let kind = self
                .slot_kind_and_position(Slot::Walk(handle))
                .0
                .into_owned();
            self.construct_joining(id, &kind);
            return true;
        }
        self.admit_fresh_join(handle, id)
    }

    /// Admit one fresh row now, either synchronously or when its journal is told.
    pub(in crate::game) fn admit_fresh_join(
        &mut self,
        handle: super::entity::EntityHandle,
        id: u32,
    ) -> bool {
        if self.join_was_announced(id) {
            return true;
        }
        let Some(handle) = self.refresh_entity_handle(id, handle) else {
            return false;
        };
        use sunrise_client::join_seq::Joined as _;
        let seq = super::entity::with_entity!(self, handle, e => e.join_seq());
        if seq > self.persist.joined_seq {
            self.persist.admitted_joins.push((seq, id));
        }
        self.native_lifecycle.admitting_joins.push(id);
        let kept = self.admit_join(Slot::Walk(handle), id, false);
        self.native_lifecycle.admitting_joins.pop();
        kept
    }

    /// [`Self::native_joins_flush`], never announcing the entities `removed` names and, with a
    /// `through`, none that joined after it: a root commit's, for the entities its transaction
    /// added and removed, each held flush keeping its own `loaded_from_disk`.
    pub(in crate::game) fn flush_joins(
        &mut self,
        loaded_from_disk: bool,
        watch: u32,
        removed: &[u32],
        through: Option<sunrise_client::join_seq::JoinSeq>,
    ) -> bool {
        if self.remote.is_some() || mod_api::__private::mods().is_empty() || !self.joins_pending() {
            return true;
        }
        self.assign_pending_join_ids();
        // Read before this call's own announcement is held: it reaches the entities to construct.
        let held_before = self.held_joins_through();
        if through.is_none()
            && self.hold_announcement(super::transactions::Announcement::Joins {
                loaded_from_disk,
                through: sunrise_client::join_seq::JoinSeq::latest(),
            })
        {
            // What an earlier held flush of this level told is constructed already; with a toss
            // still asked it may have stopped short, so then everything since the mark is looked at.
            let mut mark = self.persist.joined_seq;
            if self.native_lifecycle.held_joins.is_empty() {
                mark = mark.max(held_before);
            }
            while let Some((slot, id, seq)) = self.next_unannounced(mark, &[], None) {
                mark = seq;
                let kind = self.slot_kind_and_position(slot).0.into_owned();
                self.construct_joining(id, &kind);
            }
            return true;
        }
        let latest = sunrise_client::join_seq::JoinSeq::latest();
        let observed = through.map_or(latest, |through| through.min(latest));
        let mut kept = true;
        while let Some((slot, id, seq)) =
            self.next_unannounced(self.persist.joined_seq, removed, through)
        {
            self.persist.joined_seq = seq;
            if !self.admit_join(slot, id, loaded_from_disk) && id == watch {
                kept = false;
            }
        }
        // A held toss is not past yet: the mark stays before it.
        if self.native_lifecycle.held_joins.is_empty() {
            self.persist.joined_seq = self.persist.joined_seq.max(observed);
        }
        self.persist
            .admitted_joins
            .retain(|&(seq, _)| seq > self.persist.joined_seq);
        kept
    }

    /// The shared `PersistentEntitySectionManager.addEntity` boundary, for a fresh
    /// row or for a row waiting in a batch. NeoForge asks `EntityJoinLevelEvent`
    /// before filing the entity in its section.
    fn admit_join(&mut self, slot: Slot, id: u32, loaded_from_disk: bool) -> bool {
        let (kind, at) = self.slot_kind_and_position(slot);
        let kind = kind.into_owned();
        let mut canceled = false;
        let dimension = self.dim.id();
        let admission = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.construct_joining(id, &kind);
            // No lifecycle epoch means no mod is loaded to hear it.
            if let Some(epoch) = self.native_lifecycle.epoch() {
                self.native_entity_join_level(
                    dimension,
                    &kind,
                    [at.x, at.y, at.z],
                    loaded_from_disk,
                    mod_api::EntityRef::new(epoch, id),
                    &mut canceled,
                );
            }
            #[cfg(test)]
            {
                use super::pending_births::escaping_fault::{self, At};
                escaping_fault::reach(At::Admission, id);
            }
        }));
        let spawn = self
            .native_lifecycle
            .pending_item_spawns
            .iter()
            .position(|entry| entry.0 == id)
            .map(|index| self.native_lifecycle.pending_item_spawns.remove(index));
        if canceled || admission.is_err() {
            // A listener may have written other rows before unwinding. Retire this
            // admission only, and do not use an index that may now name another row.
            if let Some(slot) = self.refresh_join_slot(id, slot) {
                let retained = admission.is_ok()
                    && matches!(slot, Slot::Walk(super::entity::EntityHandle::Mob(index))
                        if self.detach_natural_candidate(id, index));
                if !retained {
                    self.discard_slot(slot, id);
                }
            }
            self.forget_entity(id);
            if let Err(payload) = admission {
                std::panic::resume_unwind(payload);
            }
            self.persist.joins_canceled += 1;
            return false;
        }
        if self.refresh_join_slot(id, slot).is_some() {
            // A teleported candidate already went through admission in its destination.
            self.remember_join(id, kind);
            // `addEntity` files it under its section, untold.
            self.entity_section_joined(id, [at.x, at.y, at.z]);
            if let Some((id, item, count, position)) = spawn {
                self.native_item_spawn(id, &item, count, position);
            }
        }
        true
    }

    /// `Entity.<init>` comes before `addFreshEntity`: an entity that started on a path that
    /// doesn't tell it (`entity_initialization.rs`) is told here.
    fn construct_joining(&mut self, id: u32, kind: &str) {
        self.initialize_entity(mod_api::Holder::Entity(id), kind);
        // Constructor writes were canonical but unbound before insertion.
        // The real lifetime is now resident, before join listeners query it.
        if self
            .entity_attachments
            .get(&id)
            .is_some_and(|map| map.has_index_entries())
            && let Err(error) = self.bind_resident_entity_attachments(id)
        {
            log::warn!("constructor attachment binding failed for entity {id}: {error:?}");
        }
    }

    fn assign_pending_join_ids(&mut self) {
        self.assign_dragon_and_crystal_ids();
        use super::entity_residency::assign_resident_ids;
        assign_resident_ids!(self.tnts, t, &mut self.next_entity, None);
        assign_resident_ids!(
            self.falling,
            f,
            &mut self.next_entity,
            Some(&f.attachment_lifetime)
        );
        assign_resident_ids!(
            self.creatures.projectiles,
            p,
            &mut self.next_entity,
            Some(&p.attachment_lifetime)
        );
        assign_resident_ids!(self.tridents, t, &mut self.next_entity, None);
        assign_resident_ids!(self.rockets, r, &mut self.next_entity, None);
    }
    /// The entity inserted earliest after `mark` (and, with a `through`, no later than it), but
    /// the `skipped`.
    fn next_unannounced(
        &self,
        mark: sunrise_client::join_seq::JoinSeq,
        skipped: &[u32],
        through: Option<sunrise_client::join_seq::JoinSeq>,
    ) -> Option<(Slot, u32, sunrise_client::join_seq::JoinSeq)> {
        use super::entity::Entity;
        use sunrise_client::join_seq::{JoinSeq, Joined};
        let mut best: Option<(Slot, u32, JoinSeq)> = None;
        let mut offer = |slot: Slot, id: u32, seq: JoinSeq| {
            if skipped.contains(&id)
                || self
                    .persist
                    .removals
                    .iter()
                    .chain(self.persist.destroying_vehicles.iter())
                    .any(|target| target_id(*target) == Some(id))
            {
                return;
            }
            if seq > mark
                && through.is_none_or(|through| seq <= through)
                && best.is_none_or(|(_, _, previous)| seq < previous)
                && !self.native_lifecycle.admitting_joins.contains(&id)
                && !self.join_was_announced(id)
            {
                best = Some((slot, id, seq));
            }
        };
        for handle in self.entity_handles() {
            let (id, seq) = super::entity::with_entity!(self, handle, e => (e.id(), e.join_seq()));
            offer(Slot::Walk(handle), id, seq);
        }
        for (i, c) in self.crystals.iter().enumerate() {
            offer(Slot::Apart(Apart::Crystal(i)), c.id, c.join);
        }
        for (i, c) in self.cushions.iter().enumerate() {
            offer(Slot::Apart(Apart::Cushion(i)), c.id, c.join);
        }
        for (i, p) in self.paintings.iter().enumerate() {
            offer(Slot::Apart(Apart::Painting(i)), p.id, p.join);
        }
        for (i, f) in self.item_frames.iter().enumerate() {
            offer(Slot::Apart(Apart::Frame(i)), f.id, f.join);
        }
        if let Some(d) = self.dragon.as_ref() {
            offer(Slot::Apart(Apart::Dragon), d.id, d.join);
        }
        for (i, other) in self
            .others
            .iter()
            .enumerate()
            .filter(|(_, e)| !super::general::quarantined_kind(&e.kind))
        {
            offer(Slot::Apart(Apart::Other(i)), other.id, other.join);
        }
        // A toss still being asked (`item_toss`) ends the flush: it and what joined after wait.
        best.filter(|&(_, id, _)| !self.native_lifecycle.held_joins.contains(&id))
    }

    fn join_was_announced(&self, id: u32) -> bool {
        self.persist.live.binary_search_by_key(&id, |l| l.0).is_ok()
            || self
                .persist
                .admitted_joins
                .iter()
                .any(|&(_, admitted)| admitted == id)
    }

    fn remember_join(&mut self, id: u32, kind: String) {
        let index = self
            .persist
            .live
            .binary_search_by_key(&id, |entry| entry.0)
            .unwrap_or_else(|i| i);
        self.persist.live.insert(index, (id, kind, true));
    }
    /// The type id and position a mod is told of.
    fn slot_kind_and_position(&self, slot: Slot) -> (std::borrow::Cow<'_, str>, Vec3) {
        use super::entity::Entity;
        let block = |p: [i32; 3]| {
            Vec3::new(
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            )
        };
        match slot {
            Slot::Walk(h) => {
                super::entity::with_entity!(self, h, e => (e.native_type(), e.position()))
            }
            Slot::Apart(Apart::Crystal(i)) => {
                ("minecraft:end_crystal".into(), self.crystals[i].position)
            }
            Slot::Apart(Apart::Cushion(i)) => {
                ("minecraft:cushion".into(), self.cushions[i].position)
            }
            Slot::Apart(Apart::Painting(i)) => {
                ("minecraft:painting".into(), block(self.paintings[i].pos))
            }
            Slot::Apart(Apart::Frame(i)) => {
                let f = &self.item_frames[i];
                let kind = if f.glow {
                    "minecraft:glow_item_frame"
                } else {
                    "minecraft:item_frame"
                };
                (kind.into(), block(f.pos))
            }
            Slot::Apart(Apart::Other(i)) => {
                (self.others[i].kind.as_str().into(), self.others[i].position)
            }
            Slot::Apart(Apart::Dragon) => (
                "minecraft:ender_dragon".into(),
                self.dragon.as_ref().map_or(Vec3::ZERO, |d| d.position),
            ),
        }
    }
    fn refresh_join_slot(&self, id: u32, slot: Slot) -> Option<Slot> {
        let apart = match slot {
            Slot::Walk(handle) => return self.refresh_entity_handle(id, handle).map(Slot::Walk),
            Slot::Apart(Apart::Crystal(_)) => self
                .crystals
                .iter()
                .position(|e| e.id == id)
                .map(Apart::Crystal),
            Slot::Apart(Apart::Cushion(_)) => self
                .cushions
                .iter()
                .position(|e| e.id == id)
                .map(Apart::Cushion),
            Slot::Apart(Apart::Painting(_)) => self
                .paintings
                .iter()
                .position(|e| e.id == id)
                .map(Apart::Painting),
            Slot::Apart(Apart::Frame(_)) => self
                .item_frames
                .iter()
                .position(|e| e.id == id)
                .map(Apart::Frame),
            Slot::Apart(Apart::Other(_)) => self
                .others
                .iter()
                .position(|e| e.id == id)
                .map(Apart::Other),
            Slot::Apart(Apart::Dragon) => self
                .dragon
                .as_ref()
                .filter(|e| e.id == id)
                .map(|_| Apart::Dragon),
        };
        apart.map(Slot::Apart)
    }
    /// Takes the entity out of the level, before it ever ticked.
    fn discard_slot(&mut self, slot: Slot, id: u32) {
        match slot {
            // A listener can cause another join or a removal; resolve by id
            // rather than discarding whatever now occupies the old index.
            Slot::Walk(hint) => {
                if let Some(handle) = self.refresh_entity_handle(id, hint) {
                    self.discard_entity(handle);
                }
            }
            Slot::Apart(Apart::Crystal(i)) => drop(self.crystals.remove(i)),
            Slot::Apart(Apart::Cushion(i)) => drop(self.cushions.remove(i)),
            Slot::Apart(Apart::Painting(i)) => drop(self.paintings.remove(i)),
            Slot::Apart(Apart::Frame(i)) => drop(self.item_frames.remove(i)),
            Slot::Apart(Apart::Dragon) => self.discard_resident_dragon(),
            Slot::Apart(Apart::Other(i)) => drop(self.others.remove(i)),
        }
    }
    /// Entities that were moved, not added (a portal's arrival keeps being
    /// silent): everything now in the level counts as announced, and will
    /// be told when it leaves.
    pub(super) fn native_joins_skip(&mut self) {
        let observed = sunrise_client::join_seq::JoinSeq::latest();
        if self.remote.is_none() && !mod_api::__private::mods().is_empty() {
            self.assign_pending_join_ids();
            while let Some((slot, id, seq)) =
                self.next_unannounced(self.persist.joined_seq, &[], None)
            {
                self.persist.joined_seq = seq;
                let kind = self.slot_kind_and_position(slot).0.into_owned();
                self.remember_join(id, kind);
            }
        }
        self.persist.joined_seq = self.persist.joined_seq.max(observed);
        self.persist
            .admitted_joins
            .retain(|&(seq, _)| seq > self.persist.joined_seq);
    }
    /// An entity taken out of the level without leaving it (an item through
    /// an End gateway): forgotten, not told of.
    pub(super) fn native_leave_skip(&mut self, id: u32) {
        self.entity_section_forget(id);
        if let Ok(i) = self.persist.live.binary_search_by_key(&id, |l| l.0) {
            self.persist.live.remove(i);
        }
    }
    /// Entity `id` has left the level: told once, if mods were told it joined.
    pub(super) fn native_entity_left(&mut self, id: u32) {
        if self.remote.is_some()
            || self.hold_announcement(super::transactions::Announcement::Left(id))
        {
            return;
        }
        self.entity_left_now(id);
    }
    /// [`Self::native_entity_left`] told now, though a transaction or a commit holds the rest:
    /// the root commit's own telling.
    pub(in crate::game) fn entity_left_now(&mut self, id: u32) {
        self.entity_section_forget(id);
        if let Ok(i) = self.persist.live.binary_search_by_key(&id, |l| l.0) {
            let (id, kind, _) = self.persist.live.remove(i);
            let dimension = self.dim.id();
            self.native_entity_leave_level(dimension, id, &kind);
        }
    }
    /// Tells `native_entity_leave_level` for every announced entity that is
    /// no longer in any list (died, despawned, picked up, burnt, blown up, gone
    /// through a portal, unloaded), lowest id first. Marks the entities still
    /// there in the registry and tells the rest: no allocation, and nothing
    /// runs when no mod is loaded or none was announced.
    pub(super) fn native_leaves_flush(&mut self) {
        self.native_removals_flush();
        if self.persist.live.is_empty()
            || self.remote.is_some()
            || self.hold_announcement(super::transactions::Announcement::Leaves)
        {
            return;
        }
        self.leaves_now();
    }
    /// [`Self::native_leaves_flush`] past its guards and told now, though a transaction or a
    /// commit holds the rest: the root commit's own telling.
    pub(in crate::game) fn leaves_now(&mut self) {
        use super::entity::Entity;
        for l in &mut self.persist.live {
            l.2 = false;
        }
        let live = &mut self.persist.live;
        let mut seen = |id: u32| {
            if let Ok(i) = live.binary_search_by_key(&id, |l| l.0) {
                live[i].2 = true;
            }
        };
        super::entity::for_each_entity!(super::entity::entity_lists_of!(self), e => {
            seen(e.id());
        });
        self.crystals.iter().for_each(|c| seen(c.id));
        self.cushions.iter().for_each(|c| seen(c.id));
        self.paintings.iter().for_each(|p| seen(p.id));
        self.item_frames.iter().for_each(|f| seen(f.id));
        self.others.iter().for_each(|e| seen(e.id));
        if let Some(d) = self.dragon.as_ref() {
            seen(d.id);
        }
        while let Some(i) = self.persist.live.iter().position(|l| !l.2) {
            let (id, kind, _) = self.persist.live.remove(i);
            self.entity_section_forget(id);
            let dimension = self.dim.id();
            self.native_entity_leave_level(dimension, id, &kind);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::collect_entities_in;
    use mod_api::ServerHost;

    #[test]
    #[ignore = "daily"]
    fn saved_display_and_utility_entities_join_once_and_are_queryable() {
        use mod_api::{EntityJoinLevelEvent, Flow, Hooks};
        use std::sync::{Arc, Mutex};
        let joins = Arc::new(Mutex::new(Vec::new()));
        struct Observer(Arc<Mutex<Vec<(String, bool)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                host: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                let entity = host.entity(event.subject.id()).expect("joining handle");
                assert_eq!(
                    entity.kind(host).and_then(|kind| kind.id(host)).as_deref(),
                    Some(event.kind)
                );
                assert_eq!(entity.position(host), Some(event.pos));
                let level = host.level(event.dimension).unwrap();
                if event.kind != "minecraft:eye_of_ender" {
                    assert!(
                        collect_entities_in(host, level, [4.0, 63.0, 4.0], [6.0, 66.0, 6.0])
                            .unwrap()
                            .contains(&entity)
                    );
                }
                self.0
                    .lock()
                    .unwrap()
                    .push((event.kind.to_owned(), event.loaded_from_disk));
                Flow::Continue
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "saved-display-joins",
            Box::new(Observer(joins.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                game.native_joins_skip();
                let kinds = [
                    "minecraft:block_display",
                    "minecraft:item_display",
                    "minecraft:text_display",
                    "minecraft:marker",
                    "minecraft:interaction",
                ];
                for kind in kinds {
                    game.summon_with_nbt(kind, [5.0, 64.0, 5.0], "{}").unwrap();
                }
                game.native_joins_flush(false, 0);
                assert_eq!(
                    *joins.lock().unwrap(),
                    kinds.map(|kind| (kind.to_owned(), false))
                );
                let level = game.level("minecraft:overworld").unwrap();
                let handles =
                    collect_entities_in(game, level, [4.0, 63.0, 4.0], [6.0, 66.0, 6.0]).unwrap();
                assert_eq!(handles.len(), 5);
                let mut chunks =
                    game.entity_chunks(&std::collections::HashSet::from([(0, 0)]), true);
                assert_eq!(chunks.len(), 1);
                let (_, saved) = chunks.pop().unwrap();
                assert_eq!(saved.len(), 5);
                game.native_leaves_flush();
                assert!(
                    collect_entities_in(game, level, [4.0, 63.0, 4.0], [6.0, 66.0, 6.0])
                        .unwrap()
                        .is_empty()
                );
                game.world.queue_entity_nbt((0, 0), saved);
                game.spawn_loaded_entities();
                game.native_joins_flush(true, 0);
                game.native_leaves_flush();
                assert_eq!(
                    *joins.lock().unwrap(),
                    kinds
                        .into_iter()
                        .map(|k| (k.to_owned(), false))
                        .chain(kinds.into_iter().map(|k| (k.to_owned(), true)))
                        .collect::<Vec<_>>()
                );
                let reloaded =
                    collect_entities_in(game, level, [4.0, 63.0, 4.0], [6.0, 66.0, 6.0]).unwrap();
                assert_eq!(reloaded.len(), 5);
                assert!(handles.iter().all(|e| e.kind(game).is_none()));
                assert!(
                    reloaded
                        .iter()
                        .all(|e| e.position(game) == Some([5.0, 64.0, 5.0]))
                );
                assert_eq!(
                    game.persist.live.len(),
                    5,
                    "resident utilities must not spuriously leave"
                );
                // The join moment covers the fallback store, not a fixed list of kinds.
                game.summon_with_nbt("minecraft:eye_of_ender", [5.0, 64.0, 5.0], "{}")
                    .unwrap();
                game.native_joins_flush(false, 0);
                assert_eq!(joins.lock().unwrap().len(), 11);
                assert_eq!(
                    joins.lock().unwrap().last(),
                    Some(&("minecraft:eye_of_ender".to_owned(), false))
                );
                // A quarantined missing-mod record shares storage, not level membership.
                game.next_entity += 1;
                let mut record = Tag::compound([(
                    "Pos",
                    Tag::List(
                        [5.0_f64, 64.0, 5.0]
                            .map(|n| Tag::Double(n.to_bits()))
                            .to_vec(),
                    ),
                )]);
                let opaque = game
                    .other_from_nbt(&mut record, game.next_entity, "missing:object")
                    .unwrap();
                game.others.push(opaque);
                game.native_joins_flush(true, 0);
                assert_eq!(
                    joins.lock().unwrap().len(),
                    11,
                    "opaque saved data must not join"
                );
            });
        });
    }

    // NBT loads assign ids; reinstall an unbound payload as a pending owner.
    macro_rules! pending {
        ($list:expr) => {{
            let mut owner = $list.pop().unwrap();
            owner.id = 0;
            $list.push(owner);
        }};
    }

    #[test]
    #[ignore = "daily"]
    fn pending_ids_keep_counter_order_and_assign_zero_only_once() {
        super::super::native_entity_refs::tests::with_game(|game| {
            for (kind, nbt) in [
                ("minecraft:tnt", "{fuse:80s}"),
                (
                    "minecraft:falling_block",
                    "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
                ),
                ("minecraft:arrow", "{}"),
                ("minecraft:trident", "{}"),
                ("minecraft:firework_rocket", "{LifeTime:20}"),
            ] {
                game.summon_with_nbt(kind, [1.0, 70.0, 1.0], nbt).unwrap();
            }
            pending!(game.tnts);
            pending!(game.falling);
            pending!(game.creatures.projectiles);
            pending!(game.tridents);
            pending!(game.rockets);
            let base = game.next_entity;
            let joined = game.persist.joined_seq;
            let announced = game.persist.live.clone();
            game.assign_pending_join_ids();
            let ids = [
                game.tnts[0].id,
                game.falling[0].id,
                game.creatures.projectiles[0].id,
                game.tridents[0].id,
                game.rockets[0].id,
            ];
            assert_eq!(ids, [base + 1, base + 2, base + 3, base + 4, base + 5]);
            let mut identities = super::super::entity::taken_pass_tests::SlotIdentities::default();
            identities.observe(
                &mut game.falling,
                game.native_lifecycle.epoch().unwrap(),
                |f| f.id,
            );
            game.assign_pending_join_ids();
            identities.observe(
                &mut game.falling,
                game.native_lifecycle.epoch().unwrap(),
                |f| f.id,
            );
            assert_eq!(game.next_entity, base + 5);
            assert_eq!(game.persist.joined_seq, joined);
            assert_eq!(game.persist.live, announced);
            assert_eq!(
                [
                    game.tnts[0].id,
                    game.falling[0].id,
                    game.creatures.projectiles[0].id,
                    game.tridents[0].id,
                    game.rockets[0].id
                ],
                ids
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn native_roots_keep_the_loaded_id_and_do_not_allocate_twice() {
        use sunrise_client::chunk::Chunk;
        for (kind, nbt) in [
            ("minecraft:tnt", "{fuse:80s}"),
            (
                "minecraft:falling_block",
                "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
            ),
            ("minecraft:firework_rocket", "{LifeTime:20}"),
        ] {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
                game.summon_with_nbt(kind, [2.0, 70.0, 1.0], nbt).unwrap();
                match kind {
                    "minecraft:tnt" => pending!(game.tnts),
                    "minecraft:falling_block" => pending!(game.falling),
                    _ => pending!(game.rockets),
                }
                let base = game.next_entity;
                let (Tag::Compound(tag), _) = sunrise_core::nbt::parse_snbt(nbt).unwrap() else {
                    unreachable!()
                };
                let level = game.level(game.dim.id()).unwrap();
                let entity = game
                    .level_summon(level, kind, [1.0, 70.0, 1.0], tag)
                    .unwrap()
                    .unwrap();
                let id = entity.id();
                assert_eq!(id, base + 1);
                assert_eq!(game.next_entity, base + 1);
                assert_eq!(game.entity(id), Some(entity));
                let waiting = match kind {
                    "minecraft:tnt" => game.tnts[0].id,
                    "minecraft:falling_block" => game.falling[0].id,
                    _ => game.rockets[0].id,
                };
                assert_eq!(waiting, 0);
                if kind == "minecraft:falling_block" {
                    let (Tag::Compound(unknown_state), _) = sunrise_core::nbt::parse_snbt(
                        "{BlockState:{Name:\"test:missing\"},Time:1}",
                    )
                    .unwrap() else {
                        unreachable!()
                    };
                    let fallback = game
                        .level_summon(level, kind, [1.0, 70.0, 1.0], unknown_state)
                        .unwrap()
                        .unwrap()
                        .id();
                    assert_eq!(fallback, base + 2);
                    assert_eq!(game.next_entity, base + 2);
                    assert_eq!(game.falling.len(), 3);
                    assert_eq!(game.falling[0].id, 0);
                    assert_eq!(game.falling[1].id, id);
                    assert_eq!(game.falling[2].id, fallback);
                    assert_eq!(
                        game.falling[2].state,
                        game.pack
                            .assets
                            .registry
                            .default_state("minecraft:sand")
                            .unwrap()
                    );
                }
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn portal_ids_follow_only_the_existing_moving_entity_filters() {
        super::super::native_entity_refs::tests::with_game(|game| {
            for _ in 0..3 {
                game.summon_with_nbt("minecraft:trident", [1.0, 70.0, 1.0], "{}")
                    .unwrap();
                game.summon_with_nbt(
                    "minecraft:firework_rocket",
                    [1.0, 70.0, 1.0],
                    "{LifeTime:20}",
                )
                .unwrap();
            }
            for t in &mut game.tridents {
                t.id = 0;
                t.in_ground = true;
                t.previous = t.position;
            }
            game.tridents[0].in_ground = false;
            game.tridents[2].previous.x -= 1.0;
            for r in &mut game.rockets {
                r.id = 0;
                r.attached = false;
                r.attached_to = None;
            }
            game.rockets[1].attached_to = Some(123);
            game.rockets[2].attached = true;
            let base = game.next_entity;
            game.find_ridden_vehicle_portals(&[999]);
            assert_eq!(game.next_entity, base);
            game.find_entity_portals();
            assert_eq!(
                game.tridents.iter().map(|t| t.id).collect::<Vec<_>>(),
                [base + 1, 0, base + 2]
            );
            assert_eq!(
                game.rockets.iter().map(|r| r.id).collect::<Vec<_>>(),
                [base + 3, 0, 0]
            );
            game.find_entity_portals();
            assert_eq!(game.next_entity, base + 3);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn pending_assignment_refuses_a_bound_zero_owner_before_consuming_an_id() {
        use std::sync::{Arc, Mutex};
        use sunrise_mod_runtime::attachments::{
            AttachmentType, Attachments, Holder, index::AttachmentIndex,
        };
        static VALUE: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "pending_id",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };
        for (kind, nbt) in [
            (
                "minecraft:falling_block",
                "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
            ),
            ("minecraft:arrow", "{}"),
        ] {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.summon_with_nbt(kind, [1.0, 70.0, 1.0], nbt).unwrap();
                let mut values = Attachments::default();
                VALUE.set(&mut values, Holder::Entity(0), 7).unwrap();
                let index = Arc::new(Mutex::new(AttachmentIndex::default()));
                let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 0);
                macro_rules! bind_zero {
                    ($list:expr) => {{
                        let mut owner = $list.pop().unwrap();
                        owner.id = 0;
                        owner.attachment_lifetime.bind(&mut values, &index, holder);
                        $list.push(owner);
                    }};
                }
                if kind == "minecraft:arrow" {
                    bind_zero!(game.creatures.projectiles);
                } else {
                    bind_zero!(game.falling);
                }
                let before = game.next_entity;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.assign_pending_join_ids()
                }));
                assert!(result.is_err());
                assert_eq!(game.next_entity, before);
                assert!(game.falling.iter().all(|owner| owner.id == 0));
                assert!(game.creatures.projectiles.iter().all(|owner| owner.id == 0));
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn joins_follow_insertion_order_and_canceled_entities_never_leave() {
        use mod_api::{self, EntityJoinLevelEvent, EntityLeaveLevelEvent, Flow, Hooks};
        use std::sync::{Arc, Mutex};
        use sunrise_client::mobs::Mob;
        use sunrise_core::geometry::Vec3;

        type Joins = Arc<Mutex<Vec<(u32, String, bool)>>>;
        struct Observer {
            joins: Joins,
            leaves: Arc<Mutex<Vec<u32>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                self.joins.lock().unwrap().push((
                    event.subject.id(),
                    event.kind.into(),
                    event.loaded_from_disk,
                ));
                if event.kind == "minecraft:sheep" {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn entity_leave_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityLeaveLevelEvent<'_>,
            ) {
                self.leaves.lock().unwrap().push(event.entity);
            }
        }
        let joins = Joins::default();
        let leaves = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "join-order-observer",
            Box::new(Observer {
                joins: joins.clone(),
                leaves: leaves.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.native_joins_skip();
                let base = game.next_entity;
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                let sheep = game.res.mob_types().get("minecraft:sheep").unwrap().clone();
                let child = Mob::new(base + 1, cow.clone(), Vec3::ZERO, 0.0);
                game.next_entity += 2;
                game.mobs.push(Mob::new(base + 2, cow, Vec3::ZERO, 0.0));
                game.native_joins_flush(false, 0);
                game.mobs.push(child);
                game.native_joins_flush(false, 0);
                game.native_joins_flush(false, 0);
                assert_eq!(
                    *joins.lock().unwrap(),
                    vec![
                        (base + 2, "minecraft:cow".into(), false),
                        (base + 1, "minecraft:cow".into(), false)
                    ]
                );
                game.next_entity += 1;
                game.mobs.push(Mob::new(base + 3, sheep, Vec3::ZERO, 0.0));
                assert!(!game.native_joins_flush(true, base + 3));
                assert!(!game.mobs.iter().any(|mob| mob.id == base + 3));
                game.native_entity_left(base + 3);
                assert!(leaves.lock().unwrap().is_empty());
                assert_eq!(
                    joins.lock().unwrap().last(),
                    Some(&(base + 3, "minecraft:sheep".into(), true))
                );

                game.crystals.push(super::super::dragon::Crystal::new(
                    base + 4,
                    Vec3::ZERO,
                    true,
                    0,
                ));
                game.cushions.push(sunrise_client::entities::Cushion {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: base + 5,
                    position: Vec3::ZERO,
                    pos: [0; 3],
                    yaw: 0.0,
                    color: 0,
                    check_interval: 0,
                    custom_name: None,
                });
                game.paintings.push(sunrise_client::entities::Painting {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: base + 6,
                    pos: [0; 3],
                    facing: 0,
                    variant: "minecraft:kebab".into(),
                    asset: None,
                    width: 1,
                    height: 1,
                    check_interval: 0,
                    ticks: 0,
                });
                game.item_frames.push(sunrise_client::entities::ItemFrame {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: base + 7,
                    pos: [0; 3],
                    facing: 0,
                    glow: false,
                    item: None,
                    rotation: 0,
                    drop_chance: 1.0,
                    fixed: false,
                    invisible: false,
                    custom_name: None,
                    random: sunrise_core::random::LegacyRandom::new(0),
                    check_interval: 0,
                    ticks: 0,
                });
                let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
                dragon.id = base + 8;
                dragon.join = sunrise_client::join_seq::JoinSeq::next();
                game.dragon.replace(dragon);
                game.next_entity = base + 8;
                game.native_joins_flush(true, 0);
                game.native_joins_flush(true, 0);
                let seen = joins.lock().unwrap();
                assert_eq!(
                    &seen[3..],
                    &[
                        (base + 4, "minecraft:end_crystal".into(), true),
                        (base + 5, "minecraft:cushion".into(), true),
                        (base + 6, "minecraft:painting".into(), true),
                        (base + 7, "minecraft:item_frame".into(), true),
                        (base + 8, "minecraft:ender_dragon".into(), true)
                    ]
                );
                drop(seen);
                game.tnts.push(super::super::tnt::PrimedTnt {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    id: 0,
                    position: Vec3::ZERO,
                    previous: Vec3::ZERO,
                    velocity: Vec3::ZERO,
                    fuse: 80,
                    ticks: 0,
                    ground: Default::default(),
                    in_water: false,
                    stuck: Vec3::ZERO,
                    fall_distance: 0.0,
                    owner: None,
                    guest: None,
                    no_gravity: false,
                    power: 4.0,
                    used_portal: false,
                    block_state: None,
                });
                game.native_joins_flush(false, 0);
                game.native_joins_flush(false, 0);
                assert_eq!(game.tnts.last().unwrap().id, base + 9);
                assert_eq!(
                    joins.lock().unwrap().last(),
                    Some(&(base + 9, "minecraft:tnt".into(), false))
                );
                assert_eq!(joins.lock().unwrap().len(), 9);
            });
        });
    }

    #[test]
    fn a_passenger_without_a_position_stands_where_the_stack_is_summoned() {
        let mut rider = Compound::new();
        rider.insert("id".into(), Tag::String("minecraft:pig".into()));
        let mut root = Compound::new();
        root.insert("id".into(), Tag::String("minecraft:pig".into()));
        root.insert("Passengers".into(), Tag::List(vec![Tag::Compound(rider)]));
        let mut tag = Tag::Compound(root);
        place_summoned(&mut tag, [1.5, 70.0, -2.5]);
        let pos = Tag::List(
            [1.5, 70.0, -2.5]
                .map(|v: f64| Tag::Double(v.to_bits()))
                .to_vec(),
        );
        let Tag::Compound(root) = &tag else { panic!() };
        assert_eq!(root.get(&"Pos".into()), Some(&pos));
        let Some(Tag::List(riders)) = root.get(&"Passengers".into()) else {
            panic!("no passengers");
        };
        let Tag::Compound(rider) = &riders[0] else {
            panic!()
        };
        assert_eq!(rider.get(&"Pos".into()), Some(&pos));
    }

    #[test]
    fn an_orb_root_has_an_id_only_while_it_is_in_the_level() {
        // Spawned: the orb holds the first id.
        assert_eq!(root_id(None, 36, |id| id == 36), Some(36));
        // A mod kept it out: id 36 was handed out, but no entity holds it.
        assert_eq!(root_id(None, 36, |_| false), None);
        // A command target answers for itself.
        assert_eq!(root_id(Some(Target::Mob(40)), 36, |_| false), Some(40));
        assert_eq!(root_id(Some(Target::Player), 36, |_| true), None);
    }

    #[test]
    fn summon_positions_are_summons() {
        assert!(summon_position_ok([0.5, 64.0, 0.5]));
        assert!(summon_position_ok([-3.0e7, 64.0, 0.5]));
        assert!(!summon_position_ok([3.0e7, 64.0, 0.5]));
        assert!(!summon_position_ok([0.5, f64::NAN, 0.5]));
    }

    #[test]
    #[ignore = "daily"]
    fn receiver_summon_refuses_a_tree_with_a_resident_passenger_uuid() {
        use sunrise_client::chunk::Chunk;
        use sunrise_core::nbt::parse_snbt;

        super::super::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let compound = |s| match parse_snbt(s).unwrap().0 {
                Tag::Compound(c) => c,
                _ => panic!("expected a compound"),
            };
            let existing = level
                .summon(
                    game,
                    "minecraft:pig",
                    [1.5, 70.0, 1.5],
                    compound("{UUID:[I;22,33,44,55]}"),
                )
                .unwrap()
                .unwrap();
            let next = game.next_entity;
            let tree = compound(
                r#"{UUID:[I;11,22,33,44],Passengers:[{id:"minecraft:pig",UUID:[I;22,33,44,55]}]}"#,
            );
            assert_eq!(
                level.summon(game, "minecraft:pig", [2.5, 70.0, 1.5], tree),
                Ok(None)
            );
            assert_eq!(
                game.mobs.iter().map(|mob| mob.id).collect::<Vec<_>>(),
                [existing.id()]
            );
            assert_eq!(
                game.next_entity, next,
                "not even the root entered construction"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn summon_into_parked_level_uses_level_handle_and_refuses_in_transaction() {
        use sunrise_client::chunk::Chunk;

        super::super::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
            let overworld = game.level("minecraft:overworld").unwrap();
            let nether = super::super::Dim::Nether;
            game.enter_command_level(nether).unwrap();
            game.world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));

            game.storage_scope(|game, _| {
                assert_eq!(
                    overworld.summon(game, "minecraft:pig", [1.5, 70.0, 1.5], Compound::new()),
                    Err(LevelWriteError::InTransaction)
                );
                ((), false)
            });

            let pig = overworld
                .summon(game, "minecraft:pig", [1.5, 70.0, 1.5], Compound::new())
                .unwrap()
                .expect("summoned in parked overworld");
            assert_eq!(game.dim, nether);
            assert!(game.mobs.is_empty());
            assert!(
                game.levels
                    .parked_mobs(super::super::Dim::Overworld)
                    .unwrap()
                    .iter()
                    .any(|mob| mob.id == pig.id())
            );

            let stale_epoch = mod_api::WorldEpoch::new(
                game.native_lifecycle.epoch().unwrap().get().wrapping_add(1),
            );
            let stale = LevelRef::new(stale_epoch, mod_api::DimensionKey::OVERWORLD);
            assert_eq!(
                stale.summon(game, "minecraft:pig", [1.5, 70.0, 1.5], Compound::new()),
                Err(LevelWriteError::Stale)
            );

            game.enter_command_level(super::super::Dim::Overworld)
                .unwrap();
            assert_eq!(game.entity(pig.id()), Some(pig));
        });
    }
}
