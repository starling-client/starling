//! `entity_size` (`EntityEvent.Size`): vanilla's `Entity.refreshDimensions` lets a listener
//! change the size it just computed, whenever the pose changes and from the constructor; a
//! player's standing, crouching and swimming candidates are also resolved before its motion, for
//! the pose fit tests.
//! Sunrise computes a mob's size from its state (`Mob::dims`) and keeps a player's in its body
//! (`PlayerMotion::refresh_dimensions`), so the hook is asked here instead: a pass tells it each
//! entity whose pose or vanilla size changed since it was last asked (or that was never asked,
//! the constructor's call), and keeps the answer on the entity (`pose::SizeAnswer`) for as long
//! as that pose and vanilla size hold. The server's mobs are asked before and after their ticks,
//! and its other entities (items, minecarts, boats, projectiles, armor stands, TNT, falling
//! blocks, orbs, rockets, tridents, displays, markers, interactions, end crystals...) at the same
//! passes, standing, each keeping the answer its box takes; a guest's body once per server tick,
//! with its candidates, before its motion; the client's own body for its candidates before its
//! tick and for its current pose after it.
//! A client's copy of any server entity (a mob, a minecart, an armor stand, a display...) asks
//! where a client's `refreshDimensions` runs: from its constructor, before `entity_constructing`;
//! after a synched value that refreshes it (`Entity.onSyncedDataUpdated(DATA_POSE)`, and a
//! kind's own: a baby flag, a cube's size, an armor stand's flags, an interaction's width);
//! and from its tick, where a mob applies a new `minecraft:scale`
//! (`LivingEntity.refreshDirtyAttributes`) and a shulker moves its lid
//! (`Shulker.onPeekAmountChange`). A mob keeps its answer, and other copies keep theirs on
//! their tracking row (`remote::Tracked`); the copy's box follows it.

use super::Game;
use super::entity::{Entity as _, with_entity, with_entity_mut};
use mod_api::{EntityRef, Pose};
use sunrise_client::mobs::Mob;
use sunrise_client::pose::{Dimensions, SizeAnswer};
use sunrise_core::geometry::{Aabb, Vec3};

/// The pose a client sizes its copy of a server mob by: `Entity.getPose`, the synched
/// `DATA_POSE` (a player's as the server sent it, not the one its client tick derives), never a
/// villager's bed position, which moves it without sizing it.
pub(super) fn synced_pose(m: &Mob) -> Pose {
    match &*m.kind.id {
        "minecraft:player" => Pose::from_id(i32::from(m.extra.synced_pose)),
        "minecraft:mannequin" => m.extra.avatar_pose.into(),
        _ => Pose::from_id(i32::from(m.extra.pose)),
    }
}

impl Game<'_> {
    /// Ask `entity_size` for an entity whose vanilla size in `pose` is `vanilla`, unless
    /// `previous` was already asked for both. `None`: nothing to ask, keep `previous`.
    fn ask_entity_size(
        &self,
        id: u32,
        pose: Pose,
        vanilla: [f32; 3],
        previous: Option<SizeAnswer>,
    ) -> Option<SizeAnswer> {
        if previous.is_some_and(|answer| answer.asked(pose, vanilla)) {
            return None;
        }
        let entity = EntityRef::new(self.native_lifecycle.epoch()?, id);
        // `getOldSize`: the size it had (the last answer's, or vanilla's on the first ask).
        let old = previous.map_or(vanilla, |answer| answer.effective());
        let mut size = vanilla;
        self.native_entity_size(entity, pose, old, &mut size);
        // Bound mod answers once, before collision/pathfinding consume them. 32 blocks
        // exceeds even the dragon's natural width (16), while capping a path node's
        // bounding-box scan at 33^3 cells and keeping integer extents representable.
        // Invalid answers retain vanilla size, just like non-finite answers.
        let usable = size
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0 && *v <= 32.0);
        Some(SizeAnswer {
            pose,
            vanilla,
            size: (usable && size != vanilla).then_some(size),
        })
    }

    /// The server's mobs whose pose or size changed since they were last asked.
    pub(super) fn native_mob_sizes(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            let (id, pose, vanilla, previous) = (m.id, m.pose(), m.vanilla_dims(), m.size_answer);
            if let Some(answer) = self.ask_entity_size(id, pose, vanilla, previous)
                && let Some(m) = self.mobs.get_mut(i)
                && m.id == id
            {
                m.size_answer = Some(answer);
            }
        }
    }

    /// The server's entities other than mobs whose vanilla size changed since they were last
    /// asked, or that were never asked (the constructor's call), standing: those of the shared
    /// entity lists that keep an answer (`Entity::vanilla_dimensions`), then displays, markers
    /// and interactions (`OtherEntity::vanilla_dimensions`), then end crystals.
    pub(super) fn native_entity_sizes(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        for handle in self.entity_handles() {
            let asked = with_entity!(self, handle, e => {
                e.vanilla_dimensions()
                    .map(|vanilla| (e.id(), vanilla, e.size_answer()))
            });
            if let Some((id, vanilla, previous)) = asked
                && let Some(answer) = self.ask_entity_size(id, Pose::Standing, vanilla, previous)
            {
                with_entity_mut!(self, handle, e => e.set_size_answer(answer));
            }
        }
        for i in 0..self.others.len() {
            let e = &self.others[i];
            let (id, previous) = (e.id, e.size_answer);
            let Some(vanilla) = e.vanilla_dimensions() else {
                continue;
            };
            if let Some(answer) = self.ask_entity_size(id, Pose::Standing, vanilla, previous) {
                self.others.update(i, |e| e.size_answer = Some(answer));
            }
        }
        for i in 0..self.crystals.len() {
            let c = &self.crystals[i];
            // A crystal no server tracks yet has no entity id to name.
            let (id, previous) = (c.id, c.size_answer);
            if id == 0 {
                continue;
            }
            let vanilla = super::dragon::Crystal::DIMENSIONS;
            if let Some(answer) = self.ask_entity_size(id, Pose::Standing, vanilla, previous) {
                self.crystals.update(i, |c| c.size_answer = Some(answer));
            }
        }
    }

    /// The vanilla size of a client's copy `id` of a non-mob entity of type `kind`
    /// (`Entity.getDimensions(pose)`): its type's, or what its synched values make it.
    fn client_object_dimensions(&self, id: u32, kind: &str) -> Option<[f32; 3]> {
        if let Some(a) = self.armor_stands.iter().find(|a| a.id == id) {
            return Some(a.default_dimensions());
        }
        if let Some(i) = self.interactions.get(&(id as i32)) {
            // Interaction.getDimensions: EntityDimensions.scalable(width, height).
            return Some([i.width, i.height, i.height * 0.85]);
        }
        if let Some(c) = self.potions.clouds.iter().find(|c| c.id == id) {
            // AreaEffectCloud.getDimensions: scalable(radius * 2, 0.5).
            return Some([c.radius * 2.0, 0.5, 0.5 * 0.85]);
        }
        super::entity_type_rows::entity_type_rows(&self.options.resources).dimensions(kind)
    }

    /// What the client copy `id`'s size refresh computes now: its pose and vanilla size, with
    /// the answer it holds. `pose` is a synched pose just received, for a copy that keeps no pose
    /// of its own (anything but a mob).
    fn client_copy_size(
        &mut self,
        id: u32,
        pose: Option<Pose>,
    ) -> Option<(Pose, [f32; 3], Option<SizeAnswer>)> {
        if let Some(i) = self.mob_index.find(&self.mobs, id) {
            let m = &self.mobs[i];
            let pose = synced_pose(m);
            return Some((pose, m.vanilla_dims_in(pose), m.size_answer));
        }
        let remote = self.remote.as_ref()?;
        let previous = remote.entities.get(&(id as i32))?.size_answer;
        let (kind, _) = remote.tracked_entity(id as i32)?;
        let pose = pose
            .or(previous.map(|answer| answer.pose))
            .unwrap_or(Pose::Standing);
        Some((pose, self.client_object_dimensions(id, kind)?, previous))
    }

    fn set_client_copy_size(&mut self, id: u32, answer: SizeAnswer) {
        if let Some(m) = self.mob_index.get_mut(&mut self.mobs, id) {
            m.size_answer = Some(answer);
        } else if let Some(row) = self
            .remote
            .as_mut()
            .and_then(|remote| remote.entities.get_mut(&(id as i32)))
        {
            row.size_answer = Some(answer);
        }
    }

    /// `refreshDimensions` on a client's copy `id` of a server entity: ask `entity_size` when its
    /// pose or vanilla size changed since it was last asked. A mob's client tick calls it, as
    /// `LivingEntity.refreshDirtyAttributes` applies a new `minecraft:scale` there and a shulker
    /// moves its lid there; `pose` as for [`Self::client_copy_size`].
    pub(super) fn native_client_copy_size(&mut self, id: u32, pose: Option<Pose>) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        if let Some((pose, vanilla, previous)) = self.client_copy_size(id, pose)
            && let Some(answer) = self.ask_entity_size(id, pose, vanilla, previous)
        {
            self.set_client_copy_size(id, answer);
        }
    }

    /// `EntityEvent.Size` from the constructor of a client's copy `id` of type `kind`, before
    /// `entity_constructing` and before the copy joins the level: its type's size
    /// (`EntityType.getDimensions`) standing. Keep it with [`Self::native_constructed_size`]
    /// once the copy exists.
    pub(super) fn native_constructing_size(&self, id: u32, kind: &str) -> Option<SizeAnswer> {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return None;
        }
        let vanilla =
            super::entity_type_rows::entity_type_rows(&self.options.resources).dimensions(kind)?;
        self.ask_entity_size(id, Pose::Standing, vanilla, None)
    }

    /// Keep the constructor's answer on the copy it was asked for, now that it exists.
    pub(super) fn native_constructed_size(&mut self, id: u32, answer: Option<SizeAnswer>) {
        if let Some(answer) = answer {
            self.set_client_copy_size(id, answer);
        }
    }

    /// The pose and vanilla size of the client copy `id` before a synched value lands, to tell
    /// after it whether the value refreshed its dimensions; none without a listener.
    pub(super) fn synched_size_key(&mut self, id: u32) -> Option<(Pose, [f32; 3])> {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return None;
        }
        self.client_copy_size(id, None)
            .map(|(pose, vanilla, _)| (pose, vanilla))
    }

    /// After a synched value `(index, value)` landed on the client copy `id`:
    /// `Entity.onSyncedDataUpdated` refreshes the dimensions for `DATA_POSE` (index 6) and for
    /// the values a kind's size follows, so ask when the pose or the vanilla size moved from
    /// `before` ([`Self::synched_size_key`]). A new `minecraft:scale` waits for the copy's tick.
    pub(super) fn native_synched_size(
        &mut self,
        id: u32,
        before: Option<(Pose, [f32; 3])>,
        index: u8,
        value: &sunrise_protocol::entity_data::Value,
    ) {
        let Some(before) = before else {
            return;
        };
        let pose = match (index, value) {
            // `Pose.STREAM_CODEC`: an unknown id is standing, as `from_id` reads it.
            (6, sunrise_protocol::entity_data::Value::Id(pose)) => Some(Pose::from_id(*pose)),
            _ => None,
        };
        if self
            .client_copy_size(id, pose)
            .is_some_and(|(pose, vanilla, _)| (pose, vanilla) != before)
        {
            self.native_client_copy_size(id, pose);
        }
    }

    /// The size a client's copy `id` of a non-mob entity takes from its `entity_size` answer,
    /// while that answer holds for its vanilla size.
    pub(in crate::game) fn client_copy_answer(&self, id: u32) -> Option<[f32; 3]> {
        let remote = self.remote.as_ref()?;
        let answer = remote.entities.get(&(id as i32))?.size_answer?;
        let size = answer.size?;
        let (kind, _) = remote.tracked_entity(id as i32)?;
        (self.client_object_dimensions(id, kind)? == answer.vanilla).then_some(size)
    }

    /// The box of a client's copy `id` of a non-mob entity at `position` (`Entity.getBoundingBox`,
    /// which `refreshDimensions` remakes): `vanilla`, its kind's own box, or the answer's
    /// (`EntityDimensions.makeBoundingBox`) on the same bottom.
    pub(in crate::game) fn client_copy_box(&self, id: u32, position: Vec3, vanilla: Aabb) -> Aabb {
        let Some([width, height, _]) = self.client_copy_answer(id) else {
            return vanilla;
        };
        let half = f64::from(width / 2.0);
        let bottom = vanilla.min_position().y;
        Aabb::new(
            Vec3::new(position.x - half, bottom, position.z - half),
            Vec3::new(
                position.x + half,
                bottom + f64::from(height),
                position.z + half,
            ),
        )
    }

    /// A body's pose or size changed since it was last asked: ask, and size it by the answer.
    fn ask_body_size(
        &self,
        id: u32,
        body: &sunrise_client::motion::PlayerMotion,
    ) -> Option<SizeAnswer> {
        let vanilla = body.pose.dimensions(body.scale).to_array();
        body.pose_fit_sizes
            .iter()
            .flatten()
            .copied()
            .find(|answer| answer.asked(body.pose.into(), vanilla))
            .or_else(|| self.ask_entity_size(id, body.pose.into(), vanilla, body.size_answer))
    }

    /// Snapshot only the three candidate boxes the shared pose fit test asks about.
    fn body_fit_sizes(
        &self,
        id: u32,
        body: &sunrise_client::motion::PlayerMotion,
        previous: Option<SizeAnswer>,
    ) -> [Option<SizeAnswer>; 3] {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return [None; 3];
        }
        let Ok(scale) = body.attributes.computed_value_of("minecraft:scale") else {
            return [None; 3];
        };
        use sunrise_client::pose::Pose as MotionPose;
        [
            MotionPose::Standing,
            MotionPose::Crouching,
            MotionPose::Swimming,
        ]
        .map(|pose| {
            let vanilla = pose.dimensions(scale as f32).to_array();
            self.ask_entity_size(id, pose.into(), vanilla, previous)
                .or_else(|| previous.filter(|answer| answer.asked(pose.into(), vanilla)))
        })
    }

    /// Resolve candidate sizes with the read Host before borrowing the body for motion.
    pub(super) fn native_local_player_fit_sizes(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        let Some(id) = self.remote.as_ref().map(|remote| remote.entity_id as u32) else {
            return;
        };
        let Some(answers) = self
            .player
            .as_ref()
            .map(|body| self.body_fit_sizes(id, body, body.size_answer))
        else {
            return;
        };
        if let Some(body) = self.player.as_mut() {
            body.pose_fit_sizes = answers;
        }
    }

    fn apply_body_size(body: &mut sunrise_client::motion::PlayerMotion, answer: SizeAnswer) {
        body.size_answer = Some(answer);
        body.dimensions = Dimensions::from_array(answer.effective());
    }

    /// The client's own body, after its tick (`LocalPlayer.tick` set its pose).
    pub(super) fn native_local_player_size(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        let Some(id) = self.remote.as_ref().map(|remote| remote.entity_id as u32) else {
            return;
        };
        let Some(answer) = self
            .player
            .as_ref()
            .and_then(|body| self.ask_body_size(id, body))
        else {
            return;
        };
        if let Some(body) = self.player.as_mut() {
            Self::apply_body_size(body, answer);
        }
    }

    /// Each guest's body on the server, once a tick.
    pub(super) fn native_guest_sizes(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::EntitySize) {
            return;
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let asked: Vec<_> = host
            .hub
            .guests
            .as_slice()
            .iter()
            .filter_map(|guest| {
                let body = guest.motion.as_deref()?;
                let answer = self.ask_body_size(guest.entity_id as u32, body);
                Some((
                    guest.conn,
                    answer,
                    self.body_fit_sizes(guest.entity_id as u32, body, answer.or(body.size_answer)),
                ))
            })
            .collect();
        for (conn, answer, fit_sizes) in asked {
            if let Some(body) = self
                .lan
                .as_mut()
                .and_then(|host| host.hub.guests.motion_mut(conn))
            {
                if let Some(answer) = answer {
                    Self::apply_body_size(body, answer);
                }
                body.pose_fit_sizes = fit_sizes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{EntitySizeEvent, Hooks, Host, Pose};
    use std::sync::{Arc, Mutex};

    type Calls = Arc<Mutex<Vec<(Pose, [f32; 3], [f32; 3])>>>;

    struct Grow(Calls);

    #[mod_api::__subscriptions]
    impl Hooks for Grow {
        fn entity_size(&self, _: &dyn Host, event: &mut EntitySizeEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push((event.pose, event.old_size, *event.size));
            event.size[0] *= 2.0;
            event.size[1] *= 2.0;
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_listeners_size_sizes_the_mobs_box_until_vanilla_changes_it() {
        let calls = Calls::default();
        let mods = vec![mod_api::ModEntry::new(
            "grow",
            Box::new(Grow(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let id = game
                    .spawn_mob_spawned(
                        "cow",
                        [1.0, 70.0, 3.0],
                        crate::game::creatures::SpawnSite::Plain,
                    )
                    .unwrap();
                let i = game.mobs.iter().position(|m| m.id == id).unwrap();
                let vanilla = game.mobs[i].vanilla_dims();
                game.native_mob_sizes();
                let grown = [vanilla[0] * 2.0, vanilla[1] * 2.0, vanilla[2]];
                let m = &game.mobs[i];
                assert_eq!((m.width(), m.height()), (grown[0], grown[1]));
                assert_eq!(m.eye_height(), f64::from(vanilla[2]));
                assert!((m.bounds().size().y - f64::from(grown[1])).abs() < 1e-9);
                // Asked once for a pose and vanilla size, from the constructor's.
                game.native_mob_sizes();
                assert_eq!(
                    calls.lock().unwrap().as_slice(),
                    &[(m_pose(game, i), vanilla, vanilla)]
                );
                // Grown down to a baby: asked again, from the size it had.
                sunrise_client::mobs::finalize::set_baby(&mut game.mobs[i], true);
                game.native_mob_sizes();
                let calls = calls.lock().unwrap();
                assert_eq!(calls.len(), 2);
                assert_eq!(calls[1].1, grown);
                assert_eq!(calls[1].2, game.mobs[i].vanilla_dims());
            })
        });
    }

    struct SetSize(Arc<Mutex<[f32; 3]>>);

    #[mod_api::__subscriptions]
    impl Hooks for SetSize {
        fn entity_size(&self, _: &dyn Host, event: &mut EntitySizeEvent<'_>) {
            *event.size = *self.0.lock().unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn invalid_size_answers_leave_the_mobs_default_box() {
        let size = Arc::new(Mutex::new([1.0; 3]));
        let mods = vec![mod_api::ModEntry::new(
            "set_size",
            Box::new(SetSize(size.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let id = game
                    .spawn_mob_spawned(
                        "cow",
                        [1.0, 70.0, 3.0],
                        crate::game::creatures::SpawnSite::Plain,
                    )
                    .unwrap();
                let i = game.mobs.iter().position(|m| m.id == id).unwrap();
                let vanilla = game.mobs[i].vanilla_dims();
                let default_dims = game.mobs[i].dims();
                for invalid in [
                    f32::NAN,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    -1.0,
                    32.001,
                    1e30,
                ] {
                    for axis in 0..3 {
                        let mut answer = vanilla;
                        answer[axis] = invalid;
                        *size.lock().unwrap() = answer;
                        game.mobs[i].size_answer = None;
                        game.native_mob_sizes();
                        let mob = &game.mobs[i];
                        assert_eq!(mob.size_answer.unwrap().effective(), vanilla, "{answer:?}");
                        assert_eq!(mob.dims(), default_dims, "{answer:?}");
                    }
                }
                // The reported crash input must never become a pathfinding extent.
                *size.lock().unwrap() = [1e30; 3];
                game.mobs[i].size_answer = None;
                game.native_mob_sizes();
                assert_eq!(game.mobs[i].dims(), default_dims);
                // Both endpoints are valid: zero-size entities are vanilla too.
                for accepted in [0.0, 32.0] {
                    *size.lock().unwrap() = [accepted; 3];
                    game.mobs[i].size_answer = None;
                    game.native_mob_sizes();
                    assert_eq!(
                        game.mobs[i].dims(),
                        (accepted.into(), accepted.into(), accepted.into())
                    );
                }
                *size.lock().unwrap() = [0.0; 3];
                game.mobs[i].size_answer = None;
                for _ in 0..4 {
                    game.native_mob_sizes();
                    let mob = &game.mobs[i];
                    let path_mob = mob.path_mob();
                    assert_eq!((path_mob.width, path_mob.height), (0.0, 0.0));
                    let world = game.mob_world_base(&[]);
                    assert!(world.no_collision(mob.bounds()));
                    use sunrise_client::mobs::path::{Evaluator, Finder};
                    for evaluator in [Evaluator::Walk, Evaluator::Swim { breach: true }] {
                        let finder = Finder::new(&world, &path_mob, evaluator);
                        let _ = finder.walk_path_type(1, 70, 3);
                        let _ = finder.find(&[[3, 70, 3]], 8.0, 0, 32);
                    }
                }
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn partial_collision_sizes_complete_navigation() {
        // Resource loading and the shared mod fixture lock are not navigation.
        // Start the deadline only once the fixture is ready, while keeping it
        // outside the worker so a saturated collision loop still fails boundedly.
        let (ready, started) = std::sync::mpsc::channel();
        let (done, result) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let size = Arc::new(Mutex::new([0.0; 3]));
            let mods = vec![mod_api::ModEntry::new(
                "set_size",
                Box::new(SetSize(size.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    ready.send(()).unwrap();
                    use sunrise_client::mobs::path::{Evaluator, Finder, PathType};
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                    let registry = &game.pack.assets.registry;
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let fence = registry.default_state("minecraft:oak_fence").unwrap();
                    for x in 0..4 {
                        for z in 0..4 {
                            game.world.set(x, 69, z, stone).unwrap();
                        }
                    }
                    game.world.set(0, 70, 0, fence).unwrap();
                    // At the block corner, even subnormal widths remain represented
                    // in the box's x/z extents; the fence post is not on the route.
                    let id = game
                        .spawn_mob_spawned(
                            "cow",
                            [0.0, 70.0, 0.0],
                            crate::game::creatures::SpawnSite::Plain,
                        )
                        .unwrap();
                    let i = game.mobs.iter().position(|m| m.id == id).unwrap();
                    let vanilla = game.mobs[i].vanilla_dims();
                    for answer in [[0.0; 3], [1e-30; 3], [1e-45; 3], vanilla] {
                        *size.lock().unwrap() = answer;
                        game.mobs[i].size_answer = None;
                        game.native_mob_sizes();
                        let mut mob = game.mobs[i].path_mob();
                        mob.on_ground = true;
                        let world = game.mob_world_base(&[]);
                        let finder = Finder::new(&world, &mob, Evaluator::Walk);
                        assert_eq!(finder.walk_path_type(0, 70, 0), PathType::Fence);
                        let path = finder.find(&[[2, 70, 0]], 8.0, 0, 32);
                        assert!(path.is_some(), "{answer:?}");
                    }
                    let id = game
                        .spawn_mob_spawned(
                            "silverfish",
                            [0.99, 70.0, 0.921591],
                            crate::game::creatures::SpawnSite::Plain,
                        )
                        .unwrap();
                    let i = game.mobs.iter().position(|m| m.id == id).unwrap();
                    game.mobs[i].extra.model_scale = 0.0625;
                    *size.lock().unwrap() = game.mobs[i].vanilla_dims();
                    game.native_mob_sizes();
                    let mut mob = game.mobs[i].path_mob();
                    mob.on_ground = true;
                    assert!((mob.width - 0.025).abs() < 1e-8);
                    assert!((mob.height - 0.01875).abs() < 1e-8);
                    let world = game.mob_world_base(&[]);
                    let finder = Finder::new(&world, &mob, Evaluator::Walk);
                    assert_eq!(finder.walk_path_type(0, 70, 0), PathType::Fence);
                    assert!(finder.find(&[[-1, 70, 0]], 8.0, 0, 32).is_some());
                });
            });
            done.send(()).unwrap();
        });
        started.recv().expect("navigation fixture must initialize");
        result
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("partial-collision navigation must finish within 30 seconds");
        worker.join().unwrap();
    }

    struct ShortCrouch(Arc<std::sync::atomic::AtomicUsize>);

    #[mod_api::__subscriptions]
    impl Hooks for ShortCrouch {
        fn entity_size(&self, _: &dyn Host, event: &mut EntitySizeEvent<'_>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if event.pose == Pose::Crouching {
                event.size[1] = event.size[0] / 0.6 * 0.8;
                event.size[2] = event.size[0];
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn pose_fit_uses_the_resolved_crouch_size_including_cloned_motion() {
        use sunrise_client::{
            motion::{MotionWorld, PlayerMotion},
            pose::Pose as MotionPose,
        };
        use sunrise_core::geometry::Vec3;
        for subscribed in [false, true] {
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mods: mod_api::Mods = if subscribed {
                vec![mod_api::ModEntry::new(
                    "short_crouch",
                    Box::new(ShortCrouch(calls.clone())),
                )]
                .into()
            } else {
                Vec::new().into()
            };
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(0, 65, 0, stone).unwrap();
                    let mut body =
                        PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap();
                    body.shift_key_down = true;
                    body.pose_fit_sizes = game.body_fit_sizes(1, &body, None);
                    assert_eq!(body.pose_fit_sizes.iter().any(Option::is_some), subscribed);
                    assert_eq!(
                        calls.load(std::sync::atomic::Ordering::Relaxed),
                        if subscribed { 3 } else { 0 }
                    );
                    body.size_answer = body.pose_fit_sizes[0];
                    body.pose_fit_sizes = game.body_fit_sizes(1, &body, body.size_answer);
                    assert_eq!(
                        calls.load(std::sync::atomic::Ordering::Relaxed),
                        if subscribed { 5 } else { 0 }
                    );
                    // Motion snapshots own their candidate answers, not a pointer into Game.
                    let mut restored = body.clone();
                    assert_eq!(restored.pose_fit_sizes, body.pose_fit_sizes);
                    body.pose_fit_sizes = [None; 3];
                    let side = game_side!(game);
                    let mut world = MotionWorld {
                        side: &side,
                        entities: &[],
                        player: &mut restored,
                        scene: &mut game.world,
                        assets: &game.pack.assets,
                        collision_binding: game.collision,
                        resources: game.motion,
                        health: &game.health_resources,
                        tag_items: None,
                        loop_context: &game.loop_context,
                        damage_hooks: &game.damage_hooks,
                        block_registries: Some(&game.pack.block_registries),
                    };
                    assert_eq!(
                        world.can_fit_pose(MotionPose::Crouching).unwrap(),
                        subscribed
                    );
                    world.update_local_crouching().unwrap();
                    assert_eq!(world.player.local_crouching, subscribed);
                    world.update_player_pose().unwrap();
                    assert_eq!(
                        world.player.pose,
                        if subscribed {
                            MotionPose::Crouching
                        } else {
                            MotionPose::Swimming
                        }
                    );
                    if subscribed {
                        assert_eq!(world.player.dimensions.height, 0.8);
                        // A scale change after prefill refuses the old candidate answer.
                        let key = world.player.attributes.key("minecraft:scale").unwrap();
                        let handle = world
                            .player
                            .attributes
                            .get_instance(Some(key))
                            .unwrap()
                            .unwrap();
                        world
                            .player
                            .attributes
                            .edit(&handle, |value, observer| {
                                value.set_base_value(0.9, observer)
                            })
                            .unwrap();
                        assert!(!world.can_fit_pose(MotionPose::Crouching).unwrap());
                    }
                })
            });
        }
    }

    fn m_pose(game: &crate::game::Game<'_>, i: usize) -> Pose {
        game.mobs[i].pose()
    }

    /// Each client-copy test boots the normal resource loader, which replaces process-wide
    /// tables (block namespaces, points of interest): run it in a fresh child process.
    fn in_fresh_process(test: &str) -> bool {
        crate::game::native_entity_refs::tests::run_fresh_cases(
            &format!("game::entity_sizes::tests::{test}"),
            "SUNRISE_TEST_CLIENT_COPY_SIZE",
            &[test],
        )
    }

    /// What a client's listeners saw: each `entity_size` call as (entity, pose, old size, new
    /// size), and the order of the size and constructing calls.
    #[derive(Default)]
    struct Seen {
        sizes: Vec<(u32, Pose, [f32; 3], [f32; 3])>,
        order: Vec<String>,
    }

    /// Records every call, and doubles the width and height.
    struct Log(Arc<Mutex<Seen>>);

    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn entity_size(&self, _: &dyn Host, event: &mut EntitySizeEvent<'_>) {
            let mut seen = self.0.lock().unwrap();
            let id = event.entity.id();
            seen.sizes
                .push((id, event.pose, event.old_size, *event.size));
            seen.order.push(format!("size {id}"));
            event.size[0] *= 2.0;
            event.size[1] *= 2.0;
        }
        fn entity_constructing(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::EntityConstructingEvent,
        ) {
            let kind = event.kind.to_string();
            self.0
                .lock()
                .unwrap()
                .order
                .push(format!("constructing {kind}"));
        }
    }

    /// A client logged in to a memory server, receiving that server's packets by their wire
    /// form, with `Log` listening; its chunk at the origin is loaded and empty.
    fn with_loopback_client(test: impl FnOnce(&mut crate::game::Game<'_>, &Mutex<Seen>)) {
        use crate::game::{
            native_client_session::tests::{finish_login, logged_client},
            native_entity_refs::tests::with_client_game,
        };
        let seen = Arc::new(Mutex::new(Seen::default()));
        let mods = vec![mod_api::ModEntry::new("log", Box::new(Log(seen.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let mut server =
                    sunrise_protocol::connection::Connection::local(logged_client(game));
                server
                    .reader
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                finish_login(game, &mut server);
                game.native_lifecycle.begin_open(None);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                test(game, &seen);
            });
        });
    }

    fn deliver(
        game: &mut crate::game::Game<'_>,
        packet: sunrise_protocol::packets::play::Clientbound,
    ) {
        use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
        let body = packet.encode();
        let mut reader = Reader::new(&body);
        let id = reader.varint().unwrap();
        game.handle_packet(Clientbound::decode(id, reader.rest()).unwrap())
            .unwrap();
    }

    fn add_entity(game: &mut crate::game::Game<'_>, id: i32, kind: &str, position: [f64; 3]) {
        use sunrise_protocol::packets::play::{AddEntity, Clientbound};
        let kind = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id(kind)
            .unwrap();
        deliver(
            game,
            Clientbound::AddEntity(AddEntity {
                id,
                uuid: id as u128,
                kind,
                position,
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }),
        );
    }

    fn entity_data(
        game: &mut crate::game::Game<'_>,
        id: i32,
        values: Vec<(u8, i32, sunrise_protocol::entity_data::Value)>,
    ) {
        use sunrise_protocol::packets::play::{Clientbound, SetEntityData};
        deliver(
            game,
            Clientbound::SetEntityData(SetEntityData { id, values }),
        );
    }

    fn scale(game: &mut crate::game::Game<'_>, id: i32, base: f64) {
        use sunrise_protocol::packets::play::{AttributeSnapshot, Clientbound, UpdateAttributes};
        let attribute = game
            .motion
            .attributes
            .id("minecraft:scale")
            .unwrap()
            .registry_id();
        deliver(
            game,
            Clientbound::UpdateAttributes(UpdateAttributes {
                id,
                values: vec![AttributeSnapshot {
                    attribute,
                    base,
                    modifiers: Vec::new(),
                }],
            }),
        );
    }

    /// The listener's answer for `size`: width and height doubled.
    fn doubled([width, height, eye]: [f32; 3]) -> [f32; 3] {
        [width * 2.0, height * 2.0, eye]
    }

    /// NeoForge fires `EntityEvent.Size` from the constructor before `EntityConstructing`: a
    /// client's new copy is sized by its type, standing, before the constructing listeners run.
    #[test]
    #[ignore = "daily"]
    fn a_client_copy_is_sized_before_entity_constructing() {
        if in_fresh_process("a_client_copy_is_sized_before_entity_constructing") {
            return;
        }
        with_loopback_client(|game, seen| {
            add_entity(game, 31, "minecraft:cow", [8.5, 70.0, 8.5]);
            add_entity(game, 32, "minecraft:minecart", [4.5, 70.0, 4.5]);
            let seen = seen.lock().unwrap();
            assert_eq!(
                seen.order,
                [
                    "size 31",
                    "constructing minecraft:cow",
                    "size 32",
                    "constructing minecraft:minecart",
                ]
            );
            // EntityType.COW: 0.9 by 1.4, its eyes at 1.3; MINECART: 0.98 by 0.7, eyes 0.595.
            let (cow, cart) = ([0.9, 1.4, 1.3], [0.98, 0.7, 0.595]);
            assert_eq!(
                seen.sizes,
                [
                    (31, Pose::Standing, cow, cow),
                    (32, Pose::Standing, cart, cart),
                ]
            );
        });
    }

    /// `Entity.onSyncedDataUpdated(DATA_POSE)` → `refreshDimensions` on a client: a copy of a
    /// server villager is sized by its synched pose, whichever of the pose and the bed position
    /// arrives first, asleep and waking, and the same pose sent again asks nothing.
    #[test]
    #[ignore = "daily"]
    fn a_client_copy_is_sized_by_its_synched_pose_not_its_bed() {
        use sunrise_protocol::entity_data::{Value, serializer};
        if in_fresh_process("a_client_copy_is_sized_by_its_synched_pose_not_its_bed") {
            return;
        }
        // EntityType.VILLAGER: 0.6 by 1.95, its eyes at 1.62; LivingEntity.SLEEPING_DIMENSIONS:
        // 0.2 by 0.2, eyes 0.2.
        let (standing, sleeping) = ([0.6, 1.95, 1.62], [0.2_f32; 3]);
        let pose = |id| (6, serializer::POSE, Value::Id(id));
        let bed = |at: Option<[i32; 3]>| (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(at));
        with_loopback_client(|game, seen| {
            for (id, bed_first) in [(66, true), (67, false)] {
                add_entity(game, id, "minecraft:villager", [8.5, 70.0, 8.5]);
                let calls = |seen: &Mutex<Seen>| {
                    let seen = seen.lock().unwrap();
                    let calls = seen.sizes.iter().filter(|call| call.0 == id as u32);
                    calls.copied().collect::<Vec<_>>()
                };
                let mut expected = vec![(id as u32, Pose::Standing, standing, standing)];
                if bed_first {
                    entity_data(game, id, vec![bed(Some([8, 70, 8]))]);
                    // The bed moves it but doesn't size it.
                    assert_eq!(calls(seen), expected, "bed first");
                    entity_data(game, id, vec![pose(2)]);
                } else {
                    entity_data(game, id, vec![pose(2)]);
                    entity_data(game, id, vec![bed(Some([8, 70, 8]))]);
                }
                expected.push((id as u32, Pose::Sleeping, doubled(standing), sleeping));
                assert_eq!(calls(seen), expected, "asleep, bed first {bed_first}");
                // The same pose again: no new size, no new call.
                entity_data(game, id, vec![pose(2)]);
                assert_eq!(calls(seen), expected, "asleep, same pose again");
                let m = game.mobs.iter().find(|m| m.id == id as u32).unwrap();
                assert_eq!((m.width(), m.height()), (0.4, 0.4));
                // Waking: the pose stands it up, before or after the bed is cleared.
                if bed_first {
                    entity_data(game, id, vec![bed(None)]);
                    assert_eq!(calls(seen), expected, "bed cleared first");
                    entity_data(game, id, vec![pose(0)]);
                } else {
                    entity_data(game, id, vec![pose(0)]);
                    entity_data(game, id, vec![bed(None)]);
                }
                expected.push((id as u32, Pose::Standing, doubled(sleeping), standing));
                assert_eq!(calls(seen), expected, "awake, bed first {bed_first}");
            }
        });
    }

    /// `LivingEntity.refreshDirtyAttributes` applies a new `minecraft:scale` at the entity's
    /// tick, so two scale updates before a client tick ask once, with the last scale and the
    /// size the copy had. Its box and F3+B outline then take the answer.
    #[test]
    #[ignore = "daily"]
    fn a_client_copy_takes_a_new_scale_at_its_tick() {
        if in_fresh_process("a_client_copy_takes_a_new_scale_at_its_tick") {
            return;
        }
        with_loopback_client(|game, seen| {
            let at = [8.5, 70.0, 8.5];
            add_entity(game, 31, "minecraft:cow", at);
            let cow = [0.9, 1.4, 1.3];
            scale(game, 31, 1.5);
            scale(game, 31, 2.0);
            assert_eq!(
                seen.lock().unwrap().sizes.len(),
                1,
                "no call before the tick"
            );
            game.tick_remote_entities(true);
            let scaled = [0.9_f32 * 2.0, 1.4_f32 * 2.0, 1.3_f32 * 2.0];
            assert_eq!(
                seen.lock().unwrap().sizes,
                [
                    (31, Pose::Standing, cow, cow),
                    (31, Pose::Standing, doubled(cow), scaled),
                ]
            );
            game.tick_remote_entities(true);
            assert_eq!(seen.lock().unwrap().sizes.len(), 2, "the answer holds");
            let [width, height, _] = doubled(scaled);
            let m = game.mobs.iter().find(|m| m.id == 31).unwrap();
            let p = m.position;
            let half = f64::from(width) / 2.0;
            let min = [p.x - half, p.y, p.z - half];
            let max = [p.x + half, p.y + f64::from(height), p.z + half];
            let b = m.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            assert_eq!(([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]), (min, max));
            assert_eq!(white_outline(game), (min, max));
        });
    }

    /// `Shulker.onPeekAmountChange` → `refreshDimensions` as a floor shulker's lid opens on a
    /// client: each lid move asks again, and the copy's box keeps the answer as the lid moves.
    #[test]
    #[ignore = "daily"]
    fn a_floor_shulkers_client_copy_asks_as_its_lid_moves() {
        use sunrise_protocol::entity_data::{Value, serializer};
        if in_fresh_process("a_floor_shulkers_client_copy_asks_as_its_lid_moves") {
            return;
        }
        with_loopback_client(|game, seen| {
            add_entity(game, 51, "minecraft:shulker", [8.5, 70.0, 8.5]);
            // Shulker.DATA_ATTACH_FACE_ID down, then DATA_PEEK_ID wide open.
            entity_data(
                game,
                51,
                vec![
                    (16, serializer::DIRECTION, Value::Id(0)),
                    (17, serializer::BYTE, Value::Byte(100)),
                ],
            );
            let mut heights = Vec::new();
            for _ in 0..4 {
                let before = seen.lock().unwrap().sizes.len();
                game.tick_remote_entities(true);
                let seen = seen.lock().unwrap();
                assert_eq!(seen.sizes.len(), before + 1, "one call per lid move");
                let (_, pose, _, vanilla) = *seen.sizes.last().unwrap();
                assert_eq!(pose, Pose::Standing);
                let m = game.mobs.iter().find(|m| m.id == 51).unwrap();
                assert_eq!(m.vanilla_dims(), vanilla);
                assert_eq!(m.height(), vanilla[1] * 2.0, "the lid's answer is applied");
                heights.push(vanilla[1]);
            }
            // Shulker.getDefaultDimensions: 1 + the lid's opening, rising.
            assert!(heights.windows(2).all(|h| h[0] < h[1]), "{heights:?}");
        });
    }

    /// A client's copies of entities that aren't mobs ask too: a minecart's box, the
    /// crosshair's pick, its F3+B outline and the box mods read take the answer; an armor stand
    /// asks again when it turns small; a display asks from its constructor.
    #[test]
    #[ignore = "daily"]
    fn client_copies_of_other_kinds_ask_and_take_the_answer() {
        use sunrise_protocol::entity_data::{Value, serializer};
        if in_fresh_process("client_copies_of_other_kinds_ask_and_take_the_answer") {
            return;
        }
        with_loopback_client(|game, seen| {
            let at = [8.5, 70.0, 8.5];
            add_entity(game, 32, "minecraft:minecart", at);
            add_entity(game, 41, "minecraft:armor_stand", [2.5, 70.0, 2.5]);
            add_entity(game, 61, "minecraft:block_display", [4.5, 70.0, 4.5]);
            // EntityType.MINECART: 0.98 by 0.7, eyes 0.595; ARMOR_STAND: 0.5 by 1.975, eyes
            // 1.7775; BLOCK_DISPLAY: none.
            let (cart, stand) = ([0.98, 0.7, 0.595], [0.5, 1.975, 1.7775]);
            // ArmorStand.DATA_CLIENT_FLAGS small: BABY_DIMENSIONS, the type's at half scale
            // with its eyes at 0.9875.
            entity_data(game, 41, vec![(15, serializer::BYTE, Value::Byte(1))]);
            assert_eq!(
                seen.lock().unwrap().sizes,
                [
                    (32, Pose::Standing, cart, cart),
                    (41, Pose::Standing, stand, stand),
                    (61, Pose::Standing, [0.0; 3], [0.0; 3]),
                    (41, Pose::Standing, doubled(stand), [0.25, 0.9875, 0.9875]),
                ]
            );
            let [width, height, _] = doubled(cart);
            let half = f64::from(width / 2.0);
            let min = [at[0] - half, at[1], at[2] - half];
            let max = [at[0] + half, at[1] + f64::from(height), at[2] + half];
            let entity = game.entity(32).unwrap();
            assert_eq!(game.entity_bounding_box(entity), Some([min, max]));
            // Only the cart and the armor stand show an outline: keep the cart's.
            game.armor_stands.clear();
            assert_eq!(white_outline(game), (min, max));
            // Aimed over a vanilla cart (y 70.7) but into the answered box: the crosshair
            // picks the cart.
            game.title = None;
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(5.5, 69.5, 8.5),
                )
                .unwrap(),
            );
            (game.yaw, game.pitch, game.entity_reach) = (-90.0, 0.0, 3.0);
            game.refresh_client_pick(1.0);
            assert!(matches!(
                game.crosshair_target(),
                Some(mod_api::CrosshairTarget::Entity { entity: hit, .. }) if hit == entity
            ));
        });
    }

    /// The F3+B outline's white edges: their lowest and highest corner.
    fn white_outline(game: &mut crate::game::Game<'_>) -> ([f64; 3], [f64; 3]) {
        // F3+B, on entries with no file behind them: nothing is saved.
        game.debug_entries = sunrise_client::debug_entries::DebugEntries::default();
        game.debug_entries.toggle_status("entity_hitboxes", false);
        let gizmos = game.debug_gizmos(1.0, [0.0; 3]);
        let ends: Vec<[f64; 3]> = gizmos
            .lines
            .iter()
            .filter(|line| line.color == sunrise_client::gizmos::WHITE)
            .flat_map(|line| [line.start, line.end])
            .collect();
        assert_eq!(ends.len(), 24, "one white cuboid");
        let extent = |pick: fn(f64, f64) -> f64, from: f64| {
            std::array::from_fn(|axis| ends.iter().map(|end| end[axis]).fold(from, pick))
        };
        (
            extent(f64::min, f64::INFINITY),
            extent(f64::max, f64::NEG_INFINITY),
        )
    }

    /// A server game with `Log` listening (`seen`), or with no listener at all.
    fn with_server(listen: bool, test: impl FnOnce(&mut crate::game::Game<'_>, &Mutex<Seen>)) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let mods: mod_api::Mods = if listen {
            vec![mod_api::ModEntry::new("log", Box::new(Log(seen.clone())))].into()
        } else {
            Vec::new().into()
        };
        mod_platform::testing::with_mods(mods, || with_game(|game| test(game, &seen)));
    }

    /// The box `EntityDimensions.makeBoundingBox` makes for `width` by `height` at `at`.
    fn made_box(at: sunrise_core::geometry::Vec3, [width, height, _]: [f32; 3]) -> [[f64; 3]; 2] {
        let half = f64::from(width / 2.0);
        [
            [at.x - half, at.y, at.z - half],
            [at.x + half, at.y + f64::from(height), at.z + half],
        ]
    }

    fn corners(b: sunrise_core::geometry::Aabb) -> [[f64; 3]; 2] {
        let (lo, hi) = (b.min_position(), b.max_position());
        [[lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]]
    }

    /// The server asks for every family, not only mobs (`EntityEvent.Size` from each
    /// constructor): an item, two minecarts, an armor stand, a snowball and a block display,
    /// each from its type's size, standing; a leash knot, whose box doesn't follow its size,
    /// isn't asked. Their boxes take the answer, and so does what a minecart collides with
    /// (another cart's box, `Mover::Cart`). An armor stand turned small asks again; nothing
    /// asks twice for a size already answered.
    #[test]
    #[ignore = "daily"]
    fn server_entities_ask_and_their_boxes_take_the_answer() {
        use crate::game::collision_source::Mover;
        use crate::game::minecarts::{CartKind, Minecart};
        use sunrise_client::entities::ArmorStand;
        use sunrise_client::projectiles::{Projectile, ProjectileKind};
        use sunrise_core::geometry::Vec3;
        if in_fresh_process("server_entities_ask_and_their_boxes_take_the_answer") {
            return;
        }
        with_server(true, |game, seen| {
            let (cart_at, other_at) = (Vec3::new(8.5, 70.0, 8.5), Vec3::new(10.0, 70.0, 8.5));
            game.carts
                .push(Minecart::new(9001, CartKind::Rideable, cart_at));
            game.carts
                .push(Minecart::new(9002, CartKind::Rideable, other_at));
            game.spawn_item(
                sunrise_client::items::ItemStack::new("minecraft:stone", 1),
                [4.5, 70.0, 4.5],
                [0.0; 3],
                0,
            );
            let item = game.dropped.last().unwrap().id;
            let stand_at = Vec3::new(2.5, 70.0, 6.5);
            game.armor_stands.push(ArmorStand::new(9004, stand_at, 0.0));
            let shot_at = Vec3::new(2.5, 70.0, 2.5);
            let mut shot = Projectile::new(ProjectileKind::Snowball, shot_at, None);
            shot.id = 9003;
            game.creatures.projectiles.push(shot);
            for (id, kind) in [
                (9005, "minecraft:leash_knot"),
                (9006, "minecraft:block_display"),
            ] {
                game.others.push(crate::game::commands::OtherEntity {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    knot_check: None,
                    eye: None,
                    join: Default::default(),
                    id,
                    kind: kind.into(),
                    position: Vec3::new(6.5, 70.0, 2.5),
                    yaw: 0.0,
                    pitch: 0.0,
                    display: None,
                    interaction: None,
                });
            }
            game.native_entity_sizes();
            // EntityType.ITEM: 0.25 by 0.25, eyes 0.2125; MINECART: 0.98 by 0.7, eyes 0.595;
            // ARMOR_STAND: 0.5 by 1.975, eyes 1.7775; SNOWBALL: 0.25 by 0.25, eyes 0.2125.
            let (item_size, cart, stand) = (
                [0.25, 0.25, 0.2125],
                [0.98, 0.7, 0.595],
                [0.5, 1.975, 1.7775],
            );
            let standing = |id, size| (id, Pose::Standing, size, size);
            assert_eq!(
                seen.lock().unwrap().sizes,
                [
                    standing(item, item_size),
                    standing(9001, cart),
                    standing(9002, cart),
                    standing(9004, stand),
                    standing(9003, item_size),
                    // EntityType.BLOCK_DISPLAY: none. The knot (9005) isn't asked.
                    standing(9006, [0.0; 3]),
                ]
            );
            let cart_box = made_box(cart_at, doubled(cart));
            assert_eq!(corners(game.carts[0].bounds()), cart_box);
            let item_at = game.dropped.last().unwrap().position;
            assert_eq!(
                corners(game.dropped.last().unwrap().bounds()),
                made_box(item_at, doubled(item_size))
            );
            assert_eq!(
                corners(game.creatures.projectiles[0].bounds()),
                made_box(shot_at, doubled(item_size))
            );
            assert_eq!(
                corners(game.armor_stands[0].bounds()),
                made_box(stand_at, doubled(stand))
            );
            // The other cart moves against the first one's answered box.
            let other = game.carts[1].clone();
            let solids = game.entity_collisions(Mover::Cart(&other));
            assert!(solids.iter().any(|b| corners(*b) == cart_box), "{solids:?}");
            // The same sizes again: nothing to ask. ArmorStand.setSmall refreshes its size.
            game.native_entity_sizes();
            assert_eq!(seen.lock().unwrap().sizes.len(), 6);
            game.armor_stands.update(0, |a| a.small = true);
            game.native_entity_sizes();
            assert_eq!(
                seen.lock().unwrap().sizes.last(),
                Some(&(9004, Pose::Standing, doubled(stand), [0.25, 0.9875, 0.9875]))
            );
        });
    }

    /// A falling block's landing hurts what its box covers (`FallingBlockEntity.causeFallDamage`
    /// selects by `getBoundingBox`): an anvil whose answer doubled it hits a pig a block away,
    /// which its vanilla 0.98-wide box doesn't reach.
    #[test]
    #[ignore = "daily"]
    fn a_resized_falling_anvil_hits_what_its_answer_covers() {
        use sunrise_core::geometry::Vec3;
        if in_fresh_process("a_resized_falling_anvil_hits_what_its_answer_covers") {
            return;
        }
        with_server(true, |game, _| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
            let at = Vec3::new(8.5, 64.0, 8.5);
            let pig = game
                .spawn_mob_spawned(
                    "pig",
                    [9.5, 64.0, 8.5],
                    crate::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let health = game.mobs.iter().find(|m| m.id == pig).unwrap().health;
            game.falling.push(crate::game::FallingBlock {
                size_answer: None,
                attachment_lifetime: Default::default(),
                rotation: [0.0; 2],
                join: Default::default(),
                id: 9101,
                blocks_building: true,
                state: game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:anvil")
                    .unwrap(),
                start_pos: sunrise_core::position::BlockPos::containing(at),
                position: at,
                previous: at,
                velocity: Vec3::ZERO,
                age: 1,
                no_gravity: false,
                portal: Default::default(),
                drop_item: true,
                fall_distance: 2.0,
                ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
                fire_ticks: 0,
                sync_velocity: false,
                stuck: Vec3::ZERO,
                damage: sunrise_client::falling_block::Damage::anvil(),
                saved: None,
            });
            game.native_entity_sizes();
            let mut anvil = game.falling[0].clone();
            game.falling.remove(0);
            game.falling_block_impact(&mut anvil);
            let hurt = game.mobs.iter().find(|m| m.id == pig).unwrap().health;
            assert!(hurt < health, "{hurt} < {health}");
        });
    }

    /// `AgeableMob.setAge` crossing to grown up refreshes the size (`DATA_BABY_ID` →
    /// `refreshDimensions`): a server calf that grows up in its tick asks once, from the size
    /// it had as a calf.
    #[test]
    #[ignore = "daily"]
    fn a_server_calf_asks_when_it_grows_up() {
        if in_fresh_process("a_server_calf_asks_when_it_grows_up") {
            return;
        }
        with_server(true, |game, seen| {
            crate::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
                game,
                sunrise_core::position::ChunkPos::new(0, 0),
            );
            let id = game
                .spawn_mob_spawned(
                    "cow",
                    [8.5, 70.0, 8.5],
                    crate::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            sunrise_client::mobs::finalize::set_baby(&mut game.mobs[i], true);
            let calf = game.mobs[i].vanilla_dims();
            // One tick from growing up (`AgeableMob.aiStep`: ++age).
            game.mobs[i].extra.age = -1;
            game.native_mob_sizes();
            game.tick_creature_entities().unwrap();
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            assert!(!game.mobs[i].is_baby(), "the calf grew up in its tick");
            // EntityType.COW: 0.9 by 1.4, eyes 1.3.
            let cow = [0.9, 1.4, 1.3];
            let calls: Vec<_> = seen
                .lock()
                .unwrap()
                .sizes
                .iter()
                .filter(|call| call.0 == id)
                .copied()
                .collect();
            assert_eq!(
                calls,
                [
                    (id, Pose::Standing, calf, calf),
                    (id, Pose::Standing, doubled(calf), cow),
                ]
            );
            assert_eq!(game.mobs[i].height(), 1.4_f32 * 2.0);
        });
    }

    /// No listener, no call: the server's entities keep no answer and their vanilla boxes.
    #[test]
    #[ignore = "daily"]
    fn without_a_listener_the_server_asks_nothing() {
        use crate::game::minecarts::{CartKind, Minecart};
        use sunrise_core::geometry::Vec3;
        if in_fresh_process("without_a_listener_the_server_asks_nothing") {
            return;
        }
        with_server(false, |game, _| {
            let at = Vec3::new(8.5, 70.0, 8.5);
            game.carts.push(Minecart::new(9001, CartKind::Rideable, at));
            game.native_mob_sizes();
            game.native_entity_sizes();
            assert_eq!(game.carts[0].size_answer, None);
            assert_eq!(
                corners(game.carts[0].bounds()),
                made_box(at, [0.98, 0.7, 0.0])
            );
        });
    }
}
