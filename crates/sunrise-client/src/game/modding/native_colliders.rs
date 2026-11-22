//! Moving colliders for native mods (`EntityRef::set_collider`): the boxes an entity carries
//! that every other entity collides with, kept on the server, saved with the entity, and sent
//! to the Sunrise clients that track it.
//!
//! The server keeps a [`ColliderEntry`] by entity id beside the entity's attachments, and a
//! client keeps what its tracking server sent in `Remote::colliders`. A level with no colliders
//! holds empty maps, so nothing else in the game pays for them.
use super::{
    Dim, Game,
    commands::Target,
    remote_vehicles::{PassengerSeat, passenger_root},
};
use mod_api::{Collider, ColliderError, EntityRef};
use std::collections::{HashMap, HashSet};
use sunrise_client::{
    colliders::{ColliderState, valid_rotation},
    fast_hash::FxHashMap,
};
use sunrise_core::{
    geometry::Vec3,
    nbt::{Compound, Tag},
};
use sunrise_mod_runtime::attachments::{Attachments, Holder};
use sunrise_protocol::collider_sync::{self, Message};

/// The save key of an entity's collider, beside its attachments.
pub(super) const SAVE_KEY: &str = "SunriseCollider";

/// A server entity's collider, with the form the save writes.
pub(in crate::game) struct ColliderEntry {
    pub state: ColliderState,
    saved: Compound,
}

/// The per-entity maps the game keeps by entity id: attachments, and colliders. It reads as
/// the attachments map, which is what most of the game wants; the colliders are beside it so
/// that what saves, loads and forgets an entity's attachments handles its collider too.
#[derive(Default)]
pub(in crate::game) struct EntityMaps {
    attachments: FxHashMap<u32, Attachments>,
    colliders: FxHashMap<u32, ColliderEntry>,
    /// The ids of the colliders each level holds, so a level's tick places only its own.
    by_level: HashMap<Dim, HashSet<u32>>,
}

impl std::ops::Deref for EntityMaps {
    type Target = FxHashMap<u32, Attachments>;
    fn deref(&self) -> &Self::Target {
        &self.attachments
    }
}

impl std::ops::DerefMut for EntityMaps {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.attachments
    }
}

impl EntityMaps {
    pub fn collider(&self, id: u32) -> Option<&ColliderEntry> {
        self.colliders.get(&id)
    }

    pub fn collider_mut(&mut self, id: u32) -> Option<&mut ColliderEntry> {
        self.colliders.get_mut(&id)
    }

    /// Gives the entity `id`, which stands in `level`, this collider.
    pub fn insert_collider(&mut self, id: u32, level: Dim, entry: ColliderEntry) {
        if self.colliders.insert(id, entry).is_some() {
            self.unindex(id);
        }
        self.by_level.entry(level).or_default().insert(id);
    }

    /// Takes the collider off the entity, wherever it stands.
    pub fn remove_collider(&mut self, id: u32) -> bool {
        let had = self.colliders.remove(&id).is_some();
        if had {
            self.unindex(id);
        }
        had
    }

    /// The entity `id` arrived in `level` with its collider.
    pub fn move_collider(&mut self, id: u32, level: Dim) {
        if self.colliders.contains_key(&id) {
            self.unindex(id);
            self.by_level.entry(level).or_default().insert(id);
        }
    }

    fn unindex(&mut self, id: u32) {
        self.by_level.retain(|_, ids| {
            ids.remove(&id);
            !ids.is_empty()
        });
    }

    /// The ids of the colliders in `level`.
    pub fn colliders_in(&self, level: Dim) -> impl Iterator<Item = u32> + '_ {
        self.by_level.get(&level).into_iter().flatten().copied()
    }

    /// Writes the entity's collider into its saved compound.
    pub(super) fn write_collider(&self, root: &mut Compound, id: u32) {
        match self.colliders.get(&id) {
            Some(entry) => {
                root.insert(SAVE_KEY.into(), Tag::Compound(entry.saved.clone()));
            }
            None => {
                root.remove(&SAVE_KEY.into());
            }
        }
    }
}

/// What a placed collider was made from: when all of these are as they were, so is it.
#[derive(PartialEq)]
struct PlacementKey {
    id: u32,
    serial: u64,
    origin: [u64; 3],
    /// The entities of its vehicle stack.
    stack: Vec<u32>,
}

/// The vehicle stack of `id` in a riding graph (passenger to vehicle): the root and everything
/// whose way up reaches it, `id` included. The walks are bounded by the graph's size, so a
/// cycle in what a server sent ends them.
fn stack_in_graph(vehicles: &HashMap<u32, PassengerSeat>, id: u32) -> Vec<u32> {
    let root_of = |at| passenger_root(vehicles, at).unwrap_or_else(|last| last);
    let root = root_of(id);
    let mut stack = vec![root];
    stack.extend(
        vehicles
            .keys()
            .copied()
            .filter(|&rider| root_of(rider) == root),
    );
    if !stack.contains(&id) {
        stack.push(id);
    }
    stack
}

fn origin_bits(origin: Vec3) -> [u64; 3] {
    [origin.x.to_bits(), origin.y.to_bits(), origin.z.to_bits()]
}

struct Placement {
    keys: Vec<PlacementKey>,
    prepared: sunrise_client_render::moving_colliders::Prepared,
}

/// The recent frames of each level's colliders, on a server, for the move check of a player a
/// collider carries.
#[derive(Default)]
pub(in crate::game) struct Histories(
    HashMap<Dim, std::rc::Rc<std::cell::RefCell<sunrise_client_render::moving_colliders::History>>>,
);

/// The placements of each level's colliders, kept from tick to tick.
#[derive(Default)]
pub(in crate::game) struct Placements(HashMap<Dim, Placement>);

/// What the server sends about one entity's collider this tick.
pub(in crate::game) struct ColliderChange {
    entity: u32,
    messages: Vec<Message>,
}

impl Game<'_> {
    /// The entity id a handle names when it may carry a collider: a server entity that is
    /// not a player.
    fn collider_holder(&self, entity: EntityRef) -> Result<u32, ColliderError> {
        if !self.is_logical_server() {
            return Err(ColliderError::StaleEntity);
        }
        match self.native_entity_target(entity) {
            None | Some(Target::Player | Target::Guest(_)) => Err(ColliderError::StaleEntity),
            Some(_) => Ok(entity.id()),
        }
    }

    pub(super) fn entity_collider(&self, entity: EntityRef) -> Option<Collider> {
        let id = self.collider_holder(entity).ok()?;
        self.entity_attachments
            .collider(id)
            .map(|entry| entry.state.to_api())
    }

    pub(super) fn entity_set_collider(
        &mut self,
        entity: EntityRef,
        collider: Option<Collider>,
    ) -> Result<(), ColliderError> {
        let id = self.collider_holder(entity)?;
        let Some(collider) = collider else {
            if self.entity_attachments.remove_collider(id) {
                self.collider_changes.push(ColliderChange {
                    entity: id,
                    messages: vec![Message::Clear { entity: id }],
                });
            }
            return Ok(());
        };
        let state = ColliderState::from_api(&collider)?;
        let messages = match self.entity_attachments.collider(id) {
            Some(old) => {
                let mut messages = collider_sync::changes(id, old.state.wire(), state.wire());
                if old.state.rotation() != state.rotation() {
                    messages.push(Message::Rotation {
                        entity: id,
                        rotation: state.rotation(),
                    });
                }
                messages
            }
            None => state.messages(id),
        };
        let saved = state.to_nbt();
        let level = self.dim;
        self.entity_attachments
            .insert_collider(id, level, ColliderEntry { state, saved });
        if !messages.is_empty() {
            self.collider_changes.push(ColliderChange {
                entity: id,
                messages,
            });
        }
        Ok(())
    }

    pub(super) fn entity_set_collider_transform(
        &mut self,
        entity: EntityRef,
        rotation: [f32; 4],
    ) -> Result<(), ColliderError> {
        let id = self.collider_holder(entity)?;
        if !valid_rotation(rotation) {
            return Err(ColliderError::NonFinite { index: None });
        }
        let Some(entry) = self.entity_attachments.collider_mut(id) else {
            return Err(ColliderError::NoCollider);
        };
        if entry.state.rotation() == rotation {
            return Ok(());
        }
        entry.state.set_rotation(rotation);
        entry.saved = entry.state.to_nbt();
        self.collider_changes.push(ColliderChange {
            entity: id,
            messages: vec![Message::Rotation {
                entity: id,
                rotation,
            }],
        });
        Ok(())
    }

    /// Reads the collider a saved entity carried. A malformed one is dropped with a log line.
    pub(super) fn load_saved_collider(&mut self, id: u32, tag: &Tag) {
        let Some(saved) = tag.get(SAVE_KEY) else {
            return;
        };
        match (saved, ColliderState::from_nbt(saved)) {
            (Tag::Compound(saved), Some(state)) => {
                let level = self.dim;
                self.entity_attachments.insert_collider(
                    id,
                    level,
                    ColliderEntry {
                        state,
                        saved: saved.clone(),
                    },
                );
            }
            _ => log::warn!("Sunrise: dropped a malformed saved collider of entity {id}"),
        }
    }

    /// Where the collider entity `id` is, in the level in place: the server's entity, or a
    /// client's copy of what its server shows it.
    pub(in crate::game) fn collider_origin(&self, id: u32) -> Option<Vec3> {
        let at = if self.is_logical_server() {
            self.target_position(self.level_entity(id as i32)?)?
        } else {
            let epoch = self.native_lifecycle.epoch()?;
            self.native_client_tracked(EntityRef::new(epoch, id))?.1
        };
        Some(Vec3::new(at[0], at[1], at[2]))
    }

    /// Every entity in the vehicle stack the entity `id` is in: its root vehicle and each
    /// passenger at any depth, `id` and the vehicles under it included
    /// (`Entity.isPassengerOfSameVehicle`). On a client, the riding graph its server told.
    pub(super) fn vehicle_stack(&self, id: u32) -> Vec<u32> {
        if self.is_logical_server() {
            let Some(epoch) = self.native_lifecycle.epoch() else {
                return vec![id];
            };
            let entity = EntityRef::new(epoch, id);
            let root = self.entity_root_vehicle(entity).unwrap_or(entity);
            let mut stack = vec![root.id()];
            stack.extend(
                self.entity_indirect_passengers(root)
                    .unwrap_or_default()
                    .iter()
                    .map(|rider| rider.id()),
            );
            if !stack.contains(&id) {
                stack.push(id);
            }
            return stack;
        }
        let Some(remote) = &self.remote else {
            return vec![id];
        };
        stack_in_graph(&remote.passenger_vehicles, id)
    }

    /// The frame the collider of entity `id` has now: where its entity stands and its rotation.
    pub(in crate::game) fn collider_now_frame(
        &self,
        id: u32,
    ) -> Option<sunrise_core::geometry::oriented::Frame> {
        let state = if self.is_logical_server() {
            self.entity_attachments.collider(id).map(|e| &e.state)?
        } else {
            self.remote.as_ref()?.colliders.get(&id)?
        };
        Some(state.frame(self.collider_origin(id)?))
    }

    /// Calls `f` with each collider of the level in place, in an order that is the same from
    /// one call to the next while none is added or removed; stops and answers false when `f`
    /// does.
    fn each_collider(&self, mut f: impl FnMut(u32, &ColliderState) -> bool) -> bool {
        if self.is_logical_server() {
            for id in self.entity_attachments.colliders_in(self.dim) {
                if let Some(entry) = self.entity_attachments.collider(id)
                    && !f(id, &entry.state)
                {
                    return false;
                }
            }
        } else if let Some(remote) = &self.remote {
            for (&id, state) in &remote.colliders {
                if !f(id, state) {
                    return false;
                }
            }
        }
        true
    }

    /// Whether the entity `id` is in no vehicle stack but its own: it rides nothing and
    /// nothing rides it. Asked without building the stack.
    fn is_alone(&self, id: u32) -> bool {
        if self.is_logical_server() {
            let Some(epoch) = self.native_lifecycle.epoch() else {
                return true;
            };
            let entity = EntityRef::new(epoch, id);
            self.entity_is_passenger(entity) == Some(false)
                && self.entity_is_vehicle(entity) == Some(false)
        } else {
            self.remote
                .as_ref()
                .is_none_or(|r| r.passenger_vehicle(id).is_none() && !r.has_passengers(id))
        }
    }

    /// Whether `kept` is still the vehicle stack of the entity `id`.
    fn stack_is(&self, id: u32, kept: &[u32]) -> bool {
        if kept == [id] {
            self.is_alone(id)
        } else {
            let mut now = self.vehicle_stack(id);
            let mut kept = kept.to_vec();
            now.sort_unstable();
            kept.sort_unstable();
            now == kept
        }
    }

    /// The colliders of the level in place, where they stand now, for the movers of its tick
    /// (`sunrise_client_render::moving_colliders::in_level`): a collider is solid at the
    /// position it had when its level's tick began, and moves after it. A level with none
    /// answers an empty one and pays nothing more. The level's placement is kept while no
    /// collider, place or vehicle stack of it changed; a tick where none did builds nothing
    /// (a collider that rides, or is ridden, has its stack worked out each tick).
    pub(in crate::game) fn placed_moving_colliders(
        &mut self,
    ) -> sunrise_client_render::moving_colliders::Prepared {
        use sunrise_client_render::moving_colliders::{Placed, Prepared};
        let level = self.dim;
        let mut any = false;
        self.each_collider(|_, _| {
            any = true;
            false
        });
        if !any {
            self.collider_placements.0.remove(&level);
            return Prepared::empty();
        }
        if let Some(kept) = self.collider_placements.0.get(&level) {
            let mut at = 0;
            let same = self.each_collider(|id, state| {
                let key = kept.keys.get(at);
                at += 1;
                let Some(origin) = self.collider_origin(id) else {
                    return false;
                };
                key.is_some_and(|key| {
                    key.id == id
                        && key.serial == state.serial()
                        && key.origin == origin_bits(origin)
                        && self.stack_is(id, &key.stack)
                })
            });
            if same && at == kept.keys.len() {
                return kept.prepared.clone();
            }
        }
        let mut keys = Vec::new();
        let mut placed = Vec::new();
        self.each_collider(|id, state| {
            if let Some(origin) = self.collider_origin(id) {
                let stack = self.vehicle_stack(id);
                keys.push(PlacementKey {
                    id,
                    serial: state.serial(),
                    origin: origin_bits(origin),
                    stack: stack.clone(),
                });
                placed.push(Placed {
                    id,
                    collider: state.oriented(origin),
                    exempt: stack,
                });
            }
            true
        });
        let prepared = Prepared::new(placed);
        self.collider_placements.0.insert(
            level,
            Placement {
                keys,
                prepared: prepared.clone(),
            },
        );
        prepared
    }

    /// The history the level keeps of its colliders, if it has any.
    #[cfg(test)]
    pub(in crate::game) fn level_collider_history(
        &self,
    ) -> Option<std::rc::Rc<std::cell::RefCell<sunrise_client_render::moving_colliders::History>>>
    {
        self.collider_histories.0.get(&self.dim).cloned()
    }

    /// The level in place, including the world lifetime and runtime level generation.
    pub(in crate::game) fn collider_level(&self) -> Option<mod_api::LevelRef> {
        if self.is_logical_server() {
            Some(self.level_ref(self.native_lifecycle.epoch()?, self.dim.key()))
        } else {
            self.native_client_level()
        }
    }

    /// Runs `f` with this level's moving colliders and history. If the level already
    /// has a snapshot, keep it. Otherwise use its own shapes and history, not those
    /// of a caller ticking another level; a client calling between ticks scopes its
    /// collision parts too. Nothing is recorded in the history here.
    pub(in crate::game) fn in_this_levels_colliders<R>(
        &mut self,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        use sunrise_client_render::moving_colliders::{HistoryScope, Scope};

        let level = self.collider_level();
        if level.is_some_and(Scope::covers) {
            return f(self);
        }
        let placed = self.placed_moving_colliders();
        let history = self
            .is_logical_server()
            .then(|| self.collider_histories.0.get(&self.dim).cloned())
            .flatten();
        let _parts = (!self.is_logical_server())
            .then(|| self.collision_parts())
            .flatten();
        let _history = HistoryScope::replace(history);
        let _scope = Scope::enter_level(level, placed);
        f(self)
    }

    /// Records the level's colliders as its tick begins into the level's history and answers it,
    /// for a server; a client keeps none. A level with no collider keeps nothing.
    pub(in crate::game) fn collider_history(
        &mut self,
        placed: &sunrise_client_render::moving_colliders::Prepared,
    ) -> Option<std::rc::Rc<std::cell::RefCell<sunrise_client_render::moving_colliders::History>>>
    {
        if !self.is_logical_server() {
            return None;
        }
        let level = self.dim;
        if placed.is_empty() {
            self.collider_histories.0.remove(&level);
            return None;
        }
        let history = self.collider_histories.0.entry(level).or_default().clone();
        history.borrow_mut().record(placed);
        Some(history)
    }

    /// Sends what changed this tick to the clients that track each entity and have the
    /// channel, except those whose whole collider `fresh` is about to send.
    pub(super) fn flush_collider_sync(&mut self, fresh: &HashSet<(u128, u32)>) {
        let changes = std::mem::take(&mut self.collider_changes);
        for change in changes {
            let senders = self.lan.as_ref().map_or_else(Vec::new, |host| {
                host.hub
                    .guests
                    .iter()
                    .filter(|g| {
                        host.hub.here(g)
                            && g.placed
                            && g.negotiated_collider_sync()
                            && g.tracker.is_tracking(change.entity as i32)
                            && !fresh.contains(&(g.uuid, change.entity))
                    })
                    .map(|g| g.sender.clone())
                    .collect()
            });
            for message in &change.messages {
                self.send_collider_sync(&senders, message);
            }
        }
    }

    /// The whole collider of the entity `id` to one player who started tracking it.
    pub(super) fn send_collider_snapshot(
        &mut self,
        sender: &sunrise_client::lan::conn::PlayerSender,
        id: u32,
    ) {
        let Some(entry) = self.entity_attachments.collider(id) else {
            return;
        };
        let messages = entry.state.messages(id);
        for message in &messages {
            self.send_collider_sync(std::slice::from_ref(sender), message);
        }
    }

    fn send_collider_sync(
        &mut self,
        senders: &[sunrise_client::lan::conn::PlayerSender],
        message: &Message,
    ) {
        use mod_api::{PayloadBytes, PayloadId, PayloadSendError};
        use sunrise_client::raw_payloads::Endpoint;
        let body = collider_sync::encode(message);
        for sender in senders {
            let sent = self.queue_current_payload(
                Endpoint::server(sender.clone()),
                &PayloadId(collider_sync::CHANNEL.into()),
                PayloadBytes(body.clone()),
            );
            match sent {
                Ok(_) => {}
                // A peer without the channel, or between epochs, is sent nothing.
                Err(
                    PayloadSendError::UnsupportedPeer(_)
                    | PayloadSendError::NotNegotiated
                    | PayloadSendError::WrongProtocol(_)
                    | PayloadSendError::StaleEpoch
                    | PayloadSendError::Disconnected,
                ) => {}
                // The replica would miss the change: close, as a full queue always did.
                Err(PayloadSendError::Backpressure) => sender.close(),
                Err(error) => log::warn!("Sunrise: collider sync not sent: {error:?}"),
            }
        }
    }

    /// A `sunrise:collider_sync` payload the server sent the window.
    pub(super) fn remote_collider_sync(&mut self, data: &[u8]) {
        let message = match collider_sync::decode(data) {
            Ok(message) => message,
            Err(e) => return log::debug!("Sunrise: a bad collider sync: {e}"),
        };
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        let entity = match message {
            Message::Boxes { entity, .. }
            | Message::Patch { entity, .. }
            | Message::Rotation { entity, .. }
            | Message::Clear { entity } => entity,
        };
        let mut held = remote.colliders.remove(&entity);
        ColliderState::apply(&mut held, &message);
        if let Some(state) = held {
            remote.colliders.insert(entity, state);
        }
    }

    /// A client forgets the collider of an entity the server stopped showing it.
    pub(super) fn forget_client_collider(&mut self, id: i32) {
        if let Some(remote) = self.remote.as_mut() {
            remote.colliders.remove(&(id as u32));
        }
    }

    /// The holders a flush sends whole collider snapshots for: the entities players started
    /// to track. Marks the pairs so the change flush skips them.
    pub(super) fn fresh_collider_pairs(snapshots: &[(u128, Holder)]) -> HashSet<(u128, u32)> {
        snapshots
            .iter()
            .filter_map(|(player, holder)| match holder {
                Holder::Entity(id) => Some((*player, *id)),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a client works out from the riding its server told: the root, every rider at any
    /// depth, siblings included.
    #[test]
    fn a_stack_in_a_riding_graph_holds_root_and_every_rider() {
        // 1 rides 2 rides 3; 10 and 11 ride 12; 20 and 21 ride each other (a cycle).
        let graph = [(1, 2), (2, 3), (10, 12), (11, 12), (20, 21), (21, 20)]
            .into_iter()
            .map(|(id, vehicle)| (id, PassengerSeat { vehicle, index: 0 }))
            .collect();
        let sorted = |id| {
            let mut stack = stack_in_graph(&graph, id);
            stack.sort_unstable();
            stack
        };
        for member in [1, 2, 3] {
            assert_eq!(sorted(member), vec![1, 2, 3], "from {member}");
        }
        for member in [10, 11, 12] {
            assert_eq!(
                sorted(member),
                vec![10, 11, 12],
                "a sibling sees its siblings"
            );
        }
        assert_eq!(sorted(99), vec![99], "alone");
        // A cycle ends, and still holds the entity asked about.
        assert!(sorted(20).contains(&20));
    }

    /// The client's exemption, from its riding graph, as the collision query applies it: a
    /// collider on one sibling is not met by the other sibling or the vehicle, and is met by a
    /// stranger.
    #[test]
    fn a_client_exempts_the_stack_its_graph_gives_and_meets_a_stranger() {
        use std::sync::Arc;
        use sunrise_client_render::moving_colliders::{Placed, Prepared, in_level, near};
        use sunrise_core::geometry::{
            Aabb,
            oriented::{ColliderBoxes, Frame, OrientedCollider},
        };
        use sunrise_world::entity_obstruction::EntityIdentity;
        // 10 and 11 ride 12, and 10 carries the collider; 30 is a stranger.
        let graph = [(10, 12), (11, 12)]
            .into_iter()
            .enumerate()
            .map(|(index, (id, vehicle))| (id, PassengerSeat { vehicle, index }))
            .collect();
        let boxes = ColliderBoxes::new(vec![Aabb::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 2.0),
        )]);
        let placed = Prepared::new(vec![Placed {
            id: 10,
            collider: OrientedCollider::new(Frame::translation(Vec3::ZERO), Arc::new(boxes)),
            exempt: stack_in_graph(&graph, 10),
        }]);
        let meets = |id: u32| {
            let mut found = Vec::new();
            in_level(placed.clone(), || {
                near(
                    EntityIdentity(u64::from(id)),
                    Aabb::new(Vec3::new(0.5, 0.5, 0.5), Vec3::new(1.0, 1.5, 1.0)),
                    &mut found,
                );
            });
            !found.is_empty()
        };
        assert!(
            !meets(10) && !meets(11) && !meets(12),
            "the stack is exempt"
        );
        assert!(meets(30), "a stranger meets it");
    }
}
