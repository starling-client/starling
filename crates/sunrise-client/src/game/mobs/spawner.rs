//! Monster spawners in the game: `SpawnerBlockEntity.serverTick` and
//! `clientTick` for every spawner (`mobs::spawner`), the mobs they add, a
//! spawn egg used on one (`SpawnEggItem.useOn`), their events and the mob
//! spinning inside (`SpawnerRenderer`).
use super::{
    Game,
    creatures::{Finalized, SpawnReason},
    persist::SpawnTree,
};
use std::collections::HashMap;
use sunrise_client::{
    Result,
    containers::BlockEntity,
    mobs::{
        Mob, Who,
        spawner::{SpawnDataExt, Spawner, SpawnerAdded, SpawnerEvent, SpawnerExt, SpawnerSpawn},
        trial_spawner::{TrialSpawnerExt, TrialState},
    },
    particles::{ParticleOptions, ParticleType},
};
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_core::nbt::Tag;
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, LevelRandom, RandomSource},
};

type Pos = (i32, i32, i32);

/// What a spawner tick decided to add, created once the level is free.
pub(super) enum Planned {
    /// `finalizeSpawn` ran on a fresh mob (its provisional id in `Mob::id`);
    /// the spawn data's `equipment`.
    Finalized(Mob, Option<Tag>),
    /// A mob loaded from the spawn data (with its passengers) and its
    /// `equipment`; the provisional id its events carry.
    Loaded(SpawnTree, Option<Tag>, u32),
}

/// What the spawner code keeps between ticks.
#[derive(Default)]
pub(super) struct SpawnerState {
    /// The mob shown in each spawner and the id it was made for
    /// (`getOrCreateDisplayEntity`).
    displays: HashMap<Pos, (String, Mob)>,
}

impl Game<'_> {
    /// `EntityType.byString`: the type is in the registry.
    pub(super) fn entity_type_exists(&self, id: &str) -> bool {
        self.res
            .rest
            .get()
            .is_some_and(|rest| rest.spawner_types.contains_key(id))
    }
    /// Whether `entity_type_exists` has a registry to answer from.
    pub(super) fn entity_types_loaded(&self) -> bool {
        self.res.rest.get().is_some()
    }
    /// The server tick of the spawner at `key`. Its client tick comes after
    /// every block entity's (`client_tick_spawners`).
    ///
    /// The caller only reaches this for a `BlockEntity::Spawner`, and changing
    /// the block removes or replaces that entity, so the block is not read
    /// again here.
    pub(super) fn tick_spawner(&mut self, key: Pos) -> Result<()> {
        let Some((lease, BlockEntity::Spawner(mut spawner))) = self.world.block_entities.take(&key)
        else {
            return Ok(());
        };
        let result = self.server_tick_spawner(key, &mut spawner);
        self.world
            .block_entities
            .put(lease, BlockEntity::Spawner(spawner));
        result
    }
    /// The client ticks of these spawners, then the shown mobs of spawners
    /// that are gone go.
    pub(super) fn client_tick_spawners(&mut self, keys: &[Pos]) {
        for &key in keys {
            self.client_tick_spawner(key);
        }
        self.forget_spawner_displays();
    }
    /// Drops the display mobs of spawners and trial spawners that are gone.
    pub(super) fn forget_spawner_displays(&mut self) {
        let entities = &self.world.block_entities;
        self.spawners.displays.retain(|k, _| {
            matches!(
                entities.get(k),
                Some(BlockEntity::Spawner(_) | BlockEntity::TrialSpawner(_))
            )
        });
    }
    fn server_tick_spawner(&mut self, key: Pos, spawner: &mut Spawner) -> Result<()> {
        self.server_tick_spawner_of(key, spawner, None)
    }
    /// `BaseSpawner.serverTick` at `key`: a spawner block's, or the spawner minecart
    /// `cart` is at (its `broadcastEvent` is an entity event on the cart).
    pub(super) fn server_tick_spawner_of(
        &mut self,
        key: Pos,
        spawner: &mut Spawner,
        cart: Option<u32>,
    ) -> Result<()> {
        // `BaseSpawner.serverTick` does nothing without an alive player in
        // range (`isNearPlayer`) or with the rule off: no level to build for it.
        let at = [key.0, key.1, key.2].map(|c| f64::from(c) + 0.5);
        let range = f64::from(spawner.required_player_range);
        if !self.alive_player_near(at[0], at[1], at[2], range)
            || !self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::SPAWNER_BLOCKS_WORK)
        {
            return Ok(());
        }
        // While its delay counts down, `serverTick` only takes one off it: no
        // level, random or event (a server with hundreds of dungeons near its
        // players built the level for every one of them, every tick).
        if spawner.spawn_delay > 0 {
            spawner.spawn_delay -= 1;
            return Ok(());
        }
        let pos = [key.0, key.1, key.2];
        let mut random = std::mem::replace(
            &mut self.world.random,
            LevelRandom::for_level(LegacyRandom::new(0)),
        );
        let mut events = Vec::new();
        let mut planned: Vec<Planned> = Vec::new();
        // A minecart spawner has no block.
        let source = cart
            .is_none()
            .then(|| mod_api::SpawnSource::Block(mod_api::BlockPos::new(key.0, key.1, key.2)));
        let extras = {
            let mut level = self.natural_level();
            // A minecart spawner has no block to tell the spawn checks of.
            level.spawner = cart.is_none().then_some(mod_api::Spawner::Block {
                pos: mod_api::BlockPos::new(key.0, key.1, key.2),
            });
            let mut spawn = |l: &super::natural_spawning::GameLevel,
                             rng: &mut LevelRandom,
                             r: &SpawnerSpawn| {
                l.game.plan_spawner_mob(l, r, source, &mut planned, rng)
            };
            spawner.server_tick(&level, &mut random, pos, &mut spawn, &mut events);
            (
                std::mem::take(&mut *level.spawned.borrow_mut()),
                std::mem::take(&mut *level.rides.borrow_mut()),
            )
        };
        self.world.random = random;
        let (created, rides) = extras;
        let ids = self.add_planned(planned, created, rides);
        self.apply_spawner_events(key, events, &ids, cart)
    }
    /// Creates the mobs a spawner tick planned; their provisional ids
    /// (counting down from u32::MAX) map to the mobs' ids.
    /// (counting down from `u32::MAX`) map to the mobs' ids.
    pub(super) fn add_planned(
        &mut self,
        planned: Vec<Planned>,
        mut created: Vec<Mob>,
        rides: Vec<(u32, u32)>,
    ) -> HashMap<u32, u32> {
        let _save_lease = sunrise_client::join_seq::visibility::PassLease::enter();
        let mut finalized: Vec<Mob> = Vec::new();
        let mut gears: Vec<Option<Tag>> = Vec::new();
        let mut loads = Vec::new();
        for plan in planned {
            match plan {
                Planned::Finalized(mob, equipment) => {
                    finalized.push(mob);
                    gears.push(equipment);
                }
                Planned::Loaded(tree, equipment, key) => loads.push((tree, equipment, key)),
            }
        }
        // Real ids for finalizeSpawn's provisional mobs, in spawn order, the
        // links that ride by id rewritten after them: the map the spawner's
        // events resolve their provisional ids with.
        let mut ids = self.resolve_spawn_ids([&mut finalized, &mut created], &rides);
        for (mob, equipment) in finalized.into_iter().zip(gears) {
            let id = mob.id;
            let i = self.mobs.len();
            self.mobs.push(mob);
            self.native_mobs_joined(i, false);
            // `nextSpawnData.getEquipment().ifPresent(mob::equip)`.
            if let Some(table) = equipment {
                self.equip_from_table(id, &table);
            }
        }
        for mob in created {
            let i = self.mobs.len();
            self.mobs.push(mob);
            self.native_mobs_joined(i, false);
        }
        for (tree, equipment, key) in loads {
            let id = tree.install(self);
            // `nextSpawnData.getEquipment().ifPresent(mob::equip)`.
            if let Some(table) = equipment {
                self.equip_from_table(id, &table);
            }
            ids.insert(key, id);
        }
        ids
    }
    /// The spawner's `loadEntityRecursive` and `tryAddFreshEntityWithPassengers`:
    /// make the accepted mobs visible to the next attempt before the tick inserts them.
    pub(super) fn plan_spawner_mob(
        &self,
        l: &super::natural_spawning::GameLevel,
        r: &SpawnerSpawn,
        source: Option<mod_api::SpawnSource>,
        planned: &mut Vec<Planned>,
        random: &mut sunrise_core::random::LegacyRandom,
    ) -> Option<SpawnerAdded> {
        let kind = self.res.mob_types().get(r.id)?.clone();
        let [x, y, z] = r.position;
        let at = Vec3::new(x, y, z);
        let key = if r.finalize {
            // The mob a listened position check created, else a new one.
            let mut mob = l
                .take_checked(r.id, r.position, r.yaw)
                .unwrap_or_else(|| Mob::new(l.provisional_id(), kind, at, r.yaw));
            let reason = if r.trial {
                SpawnReason::TrialSpawner
            } else {
                SpawnReason::Spawner
            };
            // TrialSpawner.spawnMob: setPersistenceRequired.
            mob.persistent = r.trial;
            let mut group = None;
            match self.finalize_spawn_event(&mut mob, reason, source, &mut group) {
                (Finalized::Cancelled, _) => return None,
                (outcome, difficulty) => {
                    if outcome == Finalized::Done {
                        let mut level = super::natural_spawning::PassLevel::new(l);
                        let mut cx = sunrise_client::mobs::finalize::Ctx {
                            level: &mut level,
                            random,
                            difficulty,
                            created: Vec::new(),
                            jockey_rider: None,
                        };
                        sunrise_client::mobs::finalize::finalize(&mut cx, &mut mob, reason, group);
                        for m in cx.created {
                            l.record_pending(&m);
                            l.spawned.borrow_mut().push(m);
                        }
                    }
                }
            }
            // Shulker.finalizeSpawn faces it south.
            if mob.species() == sunrise_client::mobs::Species::Shulker {
                mob.yaw = 0.0;
                mob.previous_yaw = 0.0;
                mob.body_yaw = 0.0;
                mob.previous_body_yaw = 0.0;
                mob.head_yaw = 0.0;
                mob.previous_head_yaw = 0.0;
            }
            l.record_pending(&mob);
            let key = mob.id;
            planned.push(Planned::Finalized(mob, r.equipment.cloned()));
            key
        } else {
            let mut tag = r.entity.clone();
            let double = |v: f64| Tag::Double(v.to_bits());
            tag.insert(
                "Pos".into(),
                Tag::List(vec![double(x), double(y), double(z)]),
            );
            tag.insert(
                "Rotation".into(),
                Tag::List(vec![
                    Tag::Float(r.yaw.to_bits()),
                    Tag::Float(0f32.to_bits()),
                ]),
            );
            if r.trial {
                tag.insert("PersistenceRequired".into(), Tag::Byte(1));
            }
            let mut tag = Tag::Compound(tag);
            // EntityType.loadEntityRecursive applies the spawner's placement
            // callback to every passenger as well as the root.
            super::commands::place_summoned(&mut tag, r.position);
            let mut tree = SpawnTree::decode(self, tag)?;
            if !l.admit_pending_tree(&mut tree) {
                return None;
            }
            let key = l.provisional_id();
            planned.push(Planned::Loaded(tree, r.equipment.cloned(), key));
            key
        };
        Some(SpawnerAdded { id: key, mob: true })
    }
    /// What a spawner's tick or a spawn egg did, shown and sent.
    fn apply_spawner_events(
        &mut self,
        key: Pos,
        events: Vec<SpawnerEvent>,
        ids: &HashMap<u32, u32>,
        cart: Option<u32>,
    ) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        for event in events {
            match event {
                // A cart has no block to update: `sendBlockUpdated` finds nothing to send.
                SpawnerEvent::BlockUpdated if cart.is_some() => {}
                SpawnerEvent::BlockUpdated => self.spawner_block_updated(key),
                // `broadcastEvent`: `level.broadcastEntityEvent(cart, id)`.
                SpawnerEvent::BlockEvent(id) if cart.is_some() => {
                    let at = pos.map(|c| f64::from(c) + 0.5);
                    self.lan_entity_event(cart.unwrap_or_default() as i32, id as i8, at);
                }
                SpawnerEvent::BlockEvent(id) => {
                    self.lan_block_event(pos, id, 0);
                    // The client copy's onEventTriggered.
                    if let Some(BlockEntity::Spawner(s)) = self.world.block_entities.get_mut(&key) {
                        s.client_event(id);
                    }
                }
                SpawnerEvent::LevelEvent(id) => self.level_event_at(id, pos, 0),
                SpawnerEvent::EntityPlace(mob, at) => {
                    let who = ids.get(&mob).map(|id| Who::Mob(*id));
                    self.post_block_game_event("minecraft:entity_place", at, who, None);
                }
                SpawnerEvent::SpawnAnim(mob) => {
                    if let Some(id) = ids.get(&mob) {
                        self.mob_poof(*id);
                    }
                }
            }
        }
        Ok(())
    }
    /// `sendBlockUpdated`: the clients load the new update tag (the client
    /// copy's delay follows it, its display mob is made again).
    fn spawner_block_updated(&mut self, key: Pos) {
        self.world
            .path_type_block_updated(sunrise_core::position::BlockPos::new(key.0, key.1, key.2));
        if let Some(BlockEntity::Spawner(s)) = self.world.block_entities.get_mut(&key) {
            s.client_loaded();
        }
        self.spawners.displays.remove(&key);
        if let Some(log) = self.world.block_log.as_mut() {
            log.push(key);
        }
        self.world.modified.insert((key.0 >> 4, key.2 >> 4));
    }
    /// `LivingEntity.makePoofParticles` (entity event 20) for a mob.
    pub(super) fn mob_poof(&mut self, id: u32) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = m.position;
        self.lan_entity_event(id as i32, 20, [at.x, at.y, at.z]);
        self.poof_particles(id);
    }
    /// `Mob.handleEntityEvent` 20 (`spawnAnim` on a client): the poof about
    /// mob `id`.
    pub(super) fn poof_particles(&mut self, id: u32) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let (at, w, h) = (m.position, f64::from(m.width()), f64::from(m.height()));
        let options = ParticleOptions::simple(ParticleType::Poof);
        for _ in 0..20 {
            let r = self.particles.level_random();
            let xa = r.next_gaussian() * 0.02;
            let ya = r.next_gaussian() * 0.02;
            let za = r.next_gaussian() * 0.02;
            let x = at.x + w * (2.0 * r.next_double() - 1.0) - xa * 10.0;
            let y = at.y + h * r.next_double() - ya * 10.0;
            let z = at.z + w * (2.0 * r.next_double() - 1.0) - za * 10.0;
            self.particles.add(&options, [x, y, z], [xa, ya, za]);
        }
    }
    /// `LevelEventHandler` 2004: twenty smoke and flame pairs around the spawner.
    pub(super) fn spawner_burst(&mut self, pos: [i32; 3]) {
        let (smoke, flame) = (
            ParticleOptions::simple(ParticleType::Smoke),
            ParticleOptions::simple(ParticleType::Flame),
        );
        for _ in 0..20 {
            let r = self.particles.level_random();
            let x = f64::from(pos[0]) + 0.5 + (r.next_double() - 0.5) * 2.0;
            let y = f64::from(pos[1]) + 0.5 + (r.next_double() - 0.5) * 2.0;
            let z = f64::from(pos[2]) + 0.5 + (r.next_double() - 0.5) * 2.0;
            self.particles.add(&smoke, [x, y, z], [0.0; 3]);
            self.particles.add(&flame, [x, y, z], [0.0; 3]);
        }
    }
    /// The client copy's tick: particles and the spin while a player is near.
    pub(super) fn client_tick_spawner(&mut self, key: Pos) {
        let near = {
            let Some(BlockEntity::Spawner(s)) = self.world.block_entities.get(&key) else {
                return;
            };
            let range = f64::from(s.required_player_range);
            let center = [
                f64::from(key.0) + 0.5,
                f64::from(key.1) + 0.5,
                f64::from(key.2) + 0.5,
            ];
            self.player.as_ref().is_some_and(|p| {
                let d = (p.position.x - center[0]).powi(2)
                    + (p.position.y - center[1]).powi(2)
                    + (p.position.z - center[2]).powi(2);
                !p.health.dead_or_dying() && !self.spectator() && (range < 0.0 || d < range * range)
            })
        };
        let display = self.update_spawner_display(key);
        let pos = [key.0, key.1, key.2];
        let random = self.particles.level_random();
        let Some(BlockEntity::Spawner(s)) = self.world.block_entities.get_mut(&key) else {
            return;
        };
        if let Some(p) = s.client_tick(near, display, random, pos) {
            self.particles
                .add(&ParticleOptions::simple(ParticleType::Smoke), p, [0.0; 3]);
            self.particles
                .add(&ParticleOptions::simple(ParticleType::Flame), p, [0.0; 3]);
        }
    }
    /// `getOrCreateDisplayEntity`: the mob shown in the spawner, made again
    /// when its type changes. False when there is none.
    pub(super) fn update_spawner_display(&mut self, key: Pos) -> bool {
        let Some(kind) = self.spawner_display_kind(key) else {
            self.spawners.displays.remove(&key);
            return false;
        };
        if self
            .spawners
            .displays
            .get(&key)
            .is_none_or(|(k, _)| *k != kind)
        {
            let Some(t) = self.res.mob_types().get(&kind).cloned() else {
                return false;
            };
            let mob = Mob::new(u32::MAX, t, Vec3::ZERO, 0.0);
            self.spawners.displays.insert(key, (kind, mob));
        }
        true
    }
    /// The type of the mob a spawner shows: its next data's id, when the game
    /// knows the mob.
    /// A trial spawner shows one only in the states with a spinning mob.
    fn spawner_display_kind(&self, key: Pos) -> Option<String> {
        let id = match self.world.block_entities.get(&key)? {
            BlockEntity::Spawner(s) => s.next_data()?.id()?,
            BlockEntity::TrialSpawner(s) => {
                self.trial_spawner_block(key)?.0.spinning_mob_speed()?;
                s.next_spawn_data.as_ref()?.id()?
            }
            _ => return None,
        };
        self.res.mob_types().contains_key(&id).then_some(id)
    }
    /// `SpawnEggItem.useOn` on a spawner: the egg sets its mob, and an op's
    /// egg its effective entity `data` too.
    /// None when the clicked block has no spawner.
    pub(super) fn use_spawn_egg_on_spawner(
        &mut self,
        pos: [i32; 3],
        kind: &str,
        data: Option<&sunrise_core::nbt::Compound>,
    ) -> Option<bool> {
        let key = (pos[0], pos[1], pos[2]);
        let trial = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Spawner(_)) => false,
            Some(BlockEntity::TrialSpawner(_)) => true,
            _ => return None,
        };
        if !self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::SPAWNER_BLOCKS_WORK)
        {
            self.system_message(sunrise_client::text::Text::translate(
                "advMode.notEnabled.spawner",
                &[],
            ));
            return Some(false);
        }
        if trial {
            // TrialSpawnerBlockEntity.setEntityId and setEntityData:
            // overrideEntityToSpawn, which ends in setState(INACTIVE).
            if let Some(BlockEntity::TrialSpawner(t)) = self.world.block_entities.get_mut(&key) {
                match data {
                    Some(data) => t.override_entity_data(kind, data),
                    None => t.override_entity(kind),
                }
            }
            let ominous = self.trial_spawner_block(key).is_some_and(|(_, o)| o);
            let _ = self.set_trial_spawner_state(key, TrialState::Inactive, ominous);
        } else {
            let mut events = Vec::new();
            let mut random = std::mem::replace(
                &mut self.world.random,
                LevelRandom::for_level(LegacyRandom::new(0)),
            );
            if let Some(BlockEntity::Spawner(s)) = self.world.block_entities.get_mut(&key) {
                match data {
                    Some(data) => s.set_entity_data(kind, data, &mut random, &mut events),
                    None => s.set_entity_id(kind, &mut random, &mut events),
                }
            }
            self.world.random = random;
            let _ = self.apply_spawner_events(key, events, &HashMap::new(), None);
        }
        // sendBlockUpdated(3) and the player's block_change game event.
        self.spawner_block_updated(key);
        self.update_spawner_display(key);
        self.post_block_game_event("minecraft:block_change", pos, Some(Who::Player), None);
        Some(true)
    }
    /// `PlayerList.isOp` for the player acting now: a server's guest by their
    /// own level, else the window's player.
    pub(super) fn acting_player_is_op(&self) -> bool {
        self.lan_acting_permission
            .unwrap_or(self.ui.registries.permission)
            > 0
    }
    /// `EntityType.createDefaultStackConfig` on an entity an item made (a
    /// vehicle, a hanging entity, an armor stand): the item's custom name and
    /// custom data (`applyComponentsFromItemStack`:
    /// `Entity.applyImplicitComponent`'s two, saved as `CustomName` and
    /// `data`), then its `minecraft:entity_data` merged over the entity's own
    /// compound and loaded (`updateCustomEntityTag`) when it names this type.
    /// An op-only type (command block and spawner minecarts) takes it from an
    /// op player only, never from a dispenser (`by_player` false).
    pub(super) fn apply_item_stack_config(
        &mut self,
        target: super::commands::Target,
        kind: &str,
        stack: &sunrise_client::items::ItemStack,
        by_player: bool,
    ) {
        let op = by_player && self.acting_player_is_op();
        let configures = stack
            .data()
            .is_some_and(|d| d.custom_name.is_some() || d.other("minecraft:custom_data").is_some())
            || sunrise_client::block_entity_item::spawn_egg_entity_data(stack, kind, op).is_some();
        if !configures {
            return;
        }
        let Some(own) = self.command_target_nbt(target) else {
            return;
        };
        let tag =
            sunrise_client::block_entity_item::stack_configured_entity_by(stack, kind, &own, op);
        // The entity's own compound, its name and data merged in: it loads back.
        let _ = self.apply_entity_nbt(target, tag);
    }
    /// `SpawnerRenderer` and `TrialSpawnerRenderer`: the next mob of every
    /// spawner near the camera, small and turning.
    pub(super) fn spawner_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        // A shader pack's shadow pass draws a block entity only in the sections it draws
        // (`Renderer::shadow_pass_block_entities`, #8464): a spawner's mob out of the view and
        // out of those sections isn't drawn, where it would have gone to the shadow pass alone.
        let shadow_sections = self
            .renderer
            .as_ref()
            .filter(|_| self.pack_ids.is_some() && self.occlusion.shadow_casters.0);
        for (key, (_, mob)) in &self.spawners.displays {
            let at = glam::DVec3::new(f64::from(key.0), f64::from(key.1), f64::from(key.2));
            if !Reach::Default.should_render([key.0, key.1, key.2], camera, 0) {
                continue;
            }
            if let Some(r) = shadow_sections
                && !r.shadow_pass_drew([key.0, key.1, key.2])
                && self.view_frustum.is_some_and(|v| {
                    let o = (at - camera).as_vec3();
                    !v.intersects(o - glam::Vec3::ONE, o + glam::Vec3::splat(2.0))
                })
            {
                continue;
            }
            let (spin, scale) = match self.world.block_entities.get(key) {
                Some(BlockEntity::Spawner(s)) => {
                    s.display_pose(alpha as f32, mob.width(), mob.height())
                }
                Some(BlockEntity::TrialSpawner(s)) => sunrise_client::mobs::spawner::display_pose(
                    s.o_spin,
                    s.spin,
                    alpha as f32,
                    mob.width(),
                    mob.height(),
                ),
                _ => continue,
            };
            let origin = glam::Mat4::from_translation(
                (at - camera).as_vec3() + glam::Vec3::new(0.5, 0.4, 0.5),
            ) * glam::Mat4::from_rotation_y(spin.to_radians())
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.2, 0.0))
                * glam::Mat4::from_rotation_x((-30.0_f32).to_radians())
                * glam::Mat4::from_scale(glam::Vec3::splat(scale));
            let light = self.world.light(key.0, key.1, key.2);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            self.mark_entity(out.len(), &mob.kind.id);
            if mob.species() == sunrise_client::mobs::Species::SulfurCube {
                self.sulfur_cube_body_geometry(
                    mob,
                    alpha as f32,
                    sunrise_client::mobs::render::base_transform_at(mob, alpha as f32, origin),
                    packed,
                    out,
                );
            }
            sunrise_client::mobs::render::render_posed(
                mob,
                alpha as f32,
                origin,
                &lookup,
                packed,
                self,
                out,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BaseSpawner.serverTick` with a player in range: a running delay only
    /// goes down by one a tick, drawing nothing from the level random and
    /// adding no mob. At 0 the spawn runs: with nothing to spawn it calls
    /// `delay`, which draws the next delay in [MinSpawnDelay, MaxSpawnDelay).
    #[test]
    #[ignore = "daily"]
    fn a_running_delay_counts_down_without_drawing() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5))
                    .unwrap(),
            );
            let key = (8, 64, 12);
            let spawner = Spawner {
                spawn_delay: 3,
                ..Spawner::default()
            };
            game.world
                .block_entities
                .insert(key, BlockEntity::Spawner(Box::new(spawner)));
            let delay = |game: &Game<'_>| match game.world.block_entities.get(&key) {
                Some(BlockEntity::Spawner(s)) => s.spawn_delay,
                _ => panic!("the spawner is gone"),
            };
            for expected in [2, 1, 0] {
                let mut before = game.world.random.clone();
                game.tick_spawner(key).unwrap();
                assert_eq!(delay(game), expected);
                assert_eq!(
                    game.world.random.next_long(),
                    before.next_long(),
                    "no draw at delay {expected}"
                );
                assert!(game.mobs.is_empty());
            }
            let mut before = game.world.random.clone();
            game.tick_spawner(key).unwrap();
            assert!((200..800).contains(&delay(game)), "{}", delay(game));
            assert_ne!(game.world.random.next_long(), before.next_long());
        });
    }
}
