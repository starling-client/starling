//! Lightning (`ServerLevel.tickThunder`, `LightningBolt`,
//! `LightningBoltRenderer`): strikes on the highest motion-blocking block or
//! a living entity under open sky nearby, skeleton trap horses, fire,
//! `thunderHit` (damage and burning, charged creepers, zombified piglins,
//! witches, mooshroom colours, turtles, burnt items), lightning rods, the sky
//! flash, the sounds and the branching additive bolt.
use super::{
    Game,
    commands::Target,
    entity::{
        Entity as _, EntityHandle, ThunderHit, entity_lists_of, for_each_entity, with_entity,
    },
    minecarts::CartHit,
};
use sunrise_client::advancements::EntityView;
use sunrise_client::mobs::{Damage, DamageKind, Species};
use sunrise_client::random_source;

use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use sunrise_world::level::Ticks;

#[derive(Default)]
struct StrikeState {
    /// Mooshrooms and copper golems change once per bolt.
    converted: Vec<u32>,
    /// The mobs it has struck (`hitEntities`).
    hit: Vec<u32>,
    /// The dropped items in the strike box (`hitEntities` holds every entity).
    hit_items: Vec<u32>,
    /// Items a cushion dropped under it, spared for 20 ticks
    /// (`Cushion.dropItem`: `setInvulnerableTime(20)`).
    spared: Vec<u32>,
}

pub(super) struct Bolt {
    /// Entity-owned registration metadata; content clones are detached.
    pub(super) attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub(in crate::game) join: sunrise_client::join_seq::JoinSeq,
    /// The entity id a server's players know it by (0 until it has one).
    pub id: u32,
    pub at: [f64; 3],
    pub velocity: Vec3,
    pub rotation: [f32; 2],
    pub fall_distance: f64,
    pub fire_ticks: i32,
    pub air: i32,
    pub on_ground: bool,
    pub invulnerable: bool,
    pub invulnerable_time: i32,
    pub portal_cooldown: i32,
    pub custom_name: Option<sunrise_core::nbt::Tag>,
    pub custom_name_visible: bool,
    pub silent: bool,
    pub no_gravity: bool,
    pub glowing: bool,
    pub ticks_frozen: i32,
    pub has_visual_fire: bool,
    /// Vehicle entity id when this bolt rides a serializable entity (`Entity.vehicle`).
    pub mount: Option<u32>,
    pub vehicle_order: i64,
    life: i32,
    flashes: i32,
    seed: i64,
    /// The bolt's own random (`Entity.random`): UUID, then seed, flashes, flash
    /// timing and fire spots draw from it, never the level's.
    random: LegacyRandom,
    visual_only: bool,
    strike: StrikeState,
    /// Blocks it set on fire (`blocksSetOnFire`).
    fires: i32,
    /// The player who channelled it (`LightningBolt.cause`): `Some(None)` for
    /// the host, `Some(Some(conn))` for a guest.
    pub cause: Option<Option<u32>>,
}

sunrise_client::impl_joined!(Bolt);
sunrise_mod_runtime::impl_attachment_lifetime!(Bolt);

impl Bolt {
    /// `LightningBolt.<init>`: `Entity.<init>` draws the UUID first, then `seed` and `flashes`.
    fn new(id: u32, at: [f64; 3], visual_only: bool) -> (Self, [i32; 4]) {
        let mut random = random_source::entity();
        let uuid = sunrise_client::save::insecure_uuid(&mut random);
        let seed = random.next_long();
        let flashes = random.next_int_bound(3).unwrap_or(0) + 1;
        (
            Self {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id,
                at,
                velocity: Vec3::ZERO,
                rotation: [0.0; 2],
                fall_distance: 0.0,
                fire_ticks: 0,
                air: 300,
                on_ground: false,
                invulnerable: false,
                invulnerable_time: 0,
                portal_cooldown: 0,
                custom_name: None,
                custom_name_visible: false,
                silent: false,
                no_gravity: false,
                glowing: false,
                ticks_frozen: 0,
                has_visual_fire: false,
                mount: None,
                vehicle_order: 0,
                life: 2,
                flashes,
                seed,
                random,
                visual_only,
                strike: StrikeState::default(),
                fires: 0,
                cause: None,
            },
            uuid,
        )
    }

    /// `Entity.load` inherited fields (`LightningBolt.readAdditionalSaveData` is empty).
    pub(super) fn apply_nbt(&mut self, tag: &sunrise_core::nbt::Tag) {
        use sunrise_client::{entity_nbt, nbt_path::int_value, save::float};
        if let Some(pos) = entity_nbt::position(tag) {
            self.at = [
                pos.x.clamp(-3.000_051_2e7, 3.000_051_2e7),
                pos.y.clamp(-2.0e7, 2.0e7),
                pos.z.clamp(-3.000_051_2e7, 3.000_051_2e7),
            ];
        }
        self.velocity = entity_nbt::loaded_motion(tag);
        self.rotation = entity_nbt::loaded_rotation(tag);
        self.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
        self.fire_ticks = i32::from(
            tag.get("Fire")
                .and_then(int_value)
                .map_or(0_i16, |v| v as i16),
        );
        self.air = tag.get("Air").and_then(int_value).unwrap_or(300);
        self.on_ground = entity_nbt::flag(tag, "OnGround");
        self.invulnerable = entity_nbt::flag(tag, "Invulnerable");
        self.invulnerable_time = tag
            .get("invulnerable_time")
            .and_then(int_value)
            .unwrap_or(0);
        self.portal_cooldown = tag.get("PortalCooldown").and_then(int_value).unwrap_or(0);
        self.custom_name = tag
            .get("CustomName")
            .and_then(entity_nbt::validated_component);
        self.custom_name_visible = entity_nbt::flag(tag, "CustomNameVisible");
        self.silent = entity_nbt::flag(tag, "Silent");
        self.no_gravity = entity_nbt::flag(tag, "NoGravity");
        self.glowing = entity_nbt::flag(tag, "Glowing");
        self.ticks_frozen = tag.get("TicksFrozen").and_then(int_value).unwrap_or(0);
        self.has_visual_fire = entity_nbt::flag(tag, "HasVisualFire");
    }
}
/// The box `LightningBolt.tick` collects the `LightningStrikeTrigger` bystanders from: 15 blocks
/// around the bolt, 15 down and `6 + 15` up.
fn in_viewer_box(bolt: [f64; 3], p: [f64; 3]) -> bool {
    (p[0] - bolt[0]).abs() <= 15.0
        && (p[2] - bolt[2]).abs() <= 15.0
        && p[1] >= bolt[1] - 15.0
        && p[1] <= bolt[1] + 21.0
}

/// `ServerLevel.tickThunder` -> `Level.getBlockRandomPos(minX, 0, minZ, 15)`:
/// picks the strike column from the level's separate `randValue` LCG.
fn thunder_strike_column(ticks: &mut Ticks, cx: i32, cz: i32) -> (i32, i32) {
    let r = ticks.block_random_pos(cx * 16, 0, cz * 16, 15);
    (r.x, r.z)
}

#[cfg(test)]
impl Bolt {
    /// A spent bolt at `at`, for tests that call `thunder_hit` directly.
    pub(in crate::game) fn struck_at(id: u32, at: [f64; 3]) -> Self {
        Self {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            at,
            velocity: Vec3::ZERO,
            rotation: [0.0; 2],
            fall_distance: 0.0,
            fire_ticks: 0,
            air: 300,
            on_ground: false,
            invulnerable: false,
            invulnerable_time: 0,
            portal_cooldown: 0,
            custom_name: None,
            custom_name_visible: false,
            silent: false,
            no_gravity: false,
            glowing: false,
            ticks_frozen: 0,
            has_visual_fire: false,
            mount: None,
            vehicle_order: 0,
            life: 0,
            flashes: 0,
            seed: 0,
            random: LegacyRandom::new(0),
            visual_only: false,
            strike: StrikeState::default(),
            fires: 0,
            cause: None,
        }
    }
}

impl Game<'_> {
    /// `ServerChunkCache.tickSpawningChunk`: one `tickThunder` roll per
    /// spawning chunk (within 128 blocks of a player) that is in entity
    /// ticking range.
    pub(super) fn tick_thunder(&mut self) {
        // isRaining (rain level > 0.2) and isThundering (> 0.9) both need
        // Level.canHaveWeather: sky light, no ceiling and not the End.
        let (rain, thunder) = self.world.weather.levels(1.0);
        let weather = self.world.shape.sky_light
            && !self.world.shape.has_ceiling
            && self.dim != super::Dim::End;
        if rain <= 0.2 || thunder <= 0.9 || !weather || self.world_debug {
            return;
        }
        let mut scratch = std::mem::take(&mut self.spawn_scratch);
        self.spawning_chunks(&self.level_players(), &mut scratch);
        let levels = self.tickets.levels();
        scratch
            .candidates
            .retain(|&(x, z)| levels.random_ticking(x, z));
        for (cx, cz) in scratch.candidates.drain(..) {
            if self.world.random.next_int_bound(100_000).unwrap_or(1) != 0 {
                continue;
            }
            // getBlockRandomPos(minX, 0, minZ, 15)
            let (x, z) = thunder_strike_column(&mut self.block_ticks, cx, cz);
            let Some(pos) = self.lightning_target(x, z) else {
                continue;
            };
            let p = BlockPos::new(pos[0], pos[1], pos[2]);
            if !self.with_level(|b, level, _| b.is_raining_at(level, p)) {
                continue;
            }
            // Skeleton trap: the local effective difficulty / 100
            // (`getCurrentDifficultyAt(pos)`).
            let difficulty = f64::from(
                self.local_difficulty([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])])
                    .effective,
            );
            let below = &self
                .res
                .facts()
                .get(self.world.block(pos[0], pos[1] - 1, pos[2]))
                .name;
            let trap = self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::SPAWN_MOBS)
                && self.world.random.next_double() < difficulty * 0.01
                && !below.ends_with("lightning_rod");
            if !self.lightning_strike_allowed(None, pos, "weather") {
                continue;
            }
            if trap {
                let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
                // ServerLevel.tickChunk: a trap horse waits for a player.
                if let Some(id) = self.spawn_mob_raw("minecraft:skeleton_horse", at, false)
                    && let Some(h) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    h.extra.trap = true;
                    h.extra.age = 0;
                }
            }
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.5,
            ];
            self.spawn_bolt(at, trap);
        }
        self.spawn_scratch = scratch;
    }
    /// `ServerLevel.findLightningRod`: `PoiManager.findClosest` over the
    /// lightning rod points of interest within 128 blocks of `center` whose
    /// block is the top (`WORLD_SURFACE`) of its column; the strike lands one
    /// above it. Sunrise keeps no point-of-interest index, so this scans the
    /// loaded chunks the way `/locate poi` does.
    fn find_lightning_rod(&self, center: [i32; 3]) -> Option<[i32; 3]> {
        const RANGE: i32 = 128;
        let facts = self.res.facts();
        let mut is_rod: std::collections::HashMap<u32, bool> = std::collections::HashMap::new();
        let mut best: Option<(i64, [i32; 3])> = None;
        let (cx0, cz0) = ((center[0] - RANGE) >> 4, (center[2] - RANGE) >> 4);
        let (cx1, cz1) = ((center[0] + RANGE) >> 4, (center[2] + RANGE) >> 4);
        for cx in cx0..=cx1 {
            for cz in cz0..=cz1 {
                let Some(chunk) = self.world.chunks.get(&(cx, cz)) else {
                    continue;
                };
                for (i, section) in chunk.sections.iter().enumerate() {
                    let mut rod = |state: u32| {
                        *is_rod.entry(state).or_insert_with(|| {
                            facts
                                .get(sunrise_world::registry::BlockStateId(state))
                                .name
                                .ends_with("lightning_rod")
                        })
                    };
                    if !section.blocks.maybe_has(&mut rod) {
                        continue;
                    }
                    let base_y = self.world.bottom_y() + 16 * i as i32;
                    for j in 0..4096 {
                        if !rod(section.get(j)) {
                            continue;
                        }
                        let p = [
                            cx * 16 + (j & 15) as i32,
                            base_y + (j >> 8) as i32,
                            cz * 16 + ((j >> 4) & 15) as i32,
                        ];
                        let d = [p[0] - center[0], p[1] - center[1], p[2] - center[2]]
                            .map(|d| i64::from(d) * i64::from(d))
                            .iter()
                            .sum::<i64>();
                        if d > i64::from(RANGE * RANGE) || best.is_some_and(|(b, _)| d >= b) {
                            continue;
                        }
                        let surface = self.world.world_surface(p[0], p[2], |s| facts.is_air(s));
                        if p[1] == surface - 1 {
                            best = Some((d, p));
                        }
                    }
                }
            }
        }
        best.map(|(_, p)| [p[0], p[1] + 1, p[2]])
    }
    /// `findLightningTargetAround`: the closest lightning rod, else a living
    /// entity under open sky in the column (3 blocks around), else the top of
    /// the column.
    pub(in crate::game) fn lightning_target(&mut self, x: i32, z: i32) -> Option<[i32; 3]> {
        if !self.world.loaded(x, z) {
            return None;
        }
        let top =
            self.with_level(|b, level, _| b.motion_blocking_top(level, BlockPos::new(x, 0, z)));
        if let Some(rod) = self.find_lightning_rod([x, top.y, z]) {
            return Some(rod);
        }
        let sees_sky = |g: &Self, p: [i32; 3]| g.world.light(p[0], p[1], p[2]) >> 4 == 15;
        let mut candidates: Vec<[i32; 3]> = self
            .mobs
            .iter()
            .filter(|m| m.alive())
            .map(|m| {
                [
                    m.position.x.floor() as i32,
                    m.position.y.floor() as i32,
                    m.position.z.floor() as i32,
                ]
            })
            .filter(|p| (p[0] - x).abs() <= 3 && (p[2] - z).abs() <= 3 && p[1] >= top.y - 3)
            .filter(|p| sees_sky(self, *p))
            .collect();
        if let Some(p) = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying())
            .map(|p| {
                [
                    p.position.x.floor() as i32,
                    p.position.y.floor() as i32,
                    p.position.z.floor() as i32,
                ]
            })
            && (p[0] - x).abs() <= 3
            && (p[2] - z).abs() <= 3
            && p[1] >= top.y - 3
            && sees_sky(self, p)
        {
            candidates.push(p);
        }
        if !candidates.is_empty() {
            let i = self
                .world
                .random
                .next_int_bound(candidates.len() as i32)
                .unwrap_or(0) as usize;
            return Some(candidates[i]);
        }
        Some([x, top.y, z])
    }
    pub(super) fn spawn_bolt(&mut self, at: [f64; 3], visual_only: bool) {
        let id = self.take_entity_id();
        let (bolt, uuid) = Bolt::new(id, at, visual_only);
        self.persist.uuids.insert(id, uuid);
        self.lightning_seen[0] += 1;
        self.bolts.push(bolt);
    }
    pub(super) fn lightning_strike_allowed(
        &mut self,
        player: Option<u128>,
        pos: [i32; 3],
        cause: &str,
    ) -> bool {
        let mut cancel = false;
        self.native_lightning_strike(player, pos, cause, &mut cancel);
        !cancel
    }
    /// Load a `minecraft:lightning_bolt` from NBT (`/summon`, `/execute summon`, or a `Passengers` entry).
    pub(in crate::game) fn load_bolt(
        &mut self,
        mut tag: sunrise_core::nbt::Tag,
        mount: Option<u32>,
        from_command: bool,
    ) -> Option<u32> {
        let pos = sunrise_client::entity_nbt::position(&tag).unwrap_or(Vec3::ZERO);
        let at = [
            pos.x.clamp(-3.000_051_2e7, 3.000_051_2e7),
            pos.y.clamp(-2.0e7, 2.0e7),
            pos.z.clamp(-3.000_051_2e7, 3.000_051_2e7),
        ];
        if from_command {
            let block = at.map(|v| v.floor() as i32);
            if !self.lightning_strike_allowed(self.lan_acting_uuid, block, "command") {
                return None;
            }
        }
        Self::normalize_loaded_mob_tags(&mut tag);
        let id = self.take_entity_id();
        let (mut bolt, constructor_uuid) = Bolt::new(id, at, false);
        bolt.apply_nbt(&tag);
        let uuid = sunrise_client::entity_nbt::entity_uuid(&tag).unwrap_or(constructor_uuid);
        if self.is_loaded_uuid_in_level(uuid) {
            return None;
        }
        if let Some(vehicle) = mount {
            bolt.mount = Some(vehicle);
            bolt.vehicle_order = i64::from(id);
            if let Some(seat) = self.vehicle_passenger_pos(vehicle) {
                bolt.at = seat;
            }
        }
        let mut kept = sunrise_core::nbt::Compound::new();
        if let Some(tags @ sunrise_core::nbt::Tag::List(_)) = tag.get("Tags") {
            kept.insert("Tags".into(), tags.clone());
        }
        if let Some(data @ sunrise_core::nbt::Tag::Compound(d)) = tag.get("data")
            && !d.is_empty()
        {
            kept.insert("data".into(), data.clone());
        }
        if let Some(team) = tag.get("Team") {
            kept.insert("Team".into(), team.clone());
        }
        self.remember(id, Some(uuid), sunrise_core::nbt::Tag::Compound(kept));
        self.lightning_seen[0] += 1;
        self.bolts.push(bolt);
        Some(id)
    }
    /// Remove a live bolt and release its UUID and scoreboard membership.
    pub(in crate::game) fn remove_bolt(&mut self, id: u32) {
        let uuid = self.persist.uuid_of(id);
        let before = self.bolts.len();
        self.bolts.retain(|b| b.id != id);
        if self.bolts.len() != before {
            if let Some(uuid) = uuid {
                self.score
                    .board
                    .entity_removed(&sunrise_client::save::uuid_text(uuid));
            }
            self.forget_entity(id);
        }
    }
    /// `Entity.saveWithoutId` for a live bolt (`/data get` and `NbtPredicate`, never chunk saves).
    pub(in crate::game) fn bolt_tag(
        &self,
        b: &Bolt,
        uuid: [i32; 4],
        include_id: bool,
    ) -> sunrise_core::nbt::Tag {
        use sunrise_core::nbt::{Compound, Tag};
        let double = |v: f64| Tag::Double(if v == 0.0 { 0 } else { v.to_bits() });
        let float = |v: f32| Tag::Float(if v == 0.0 { 0 } else { v.to_bits() });
        let pos = if let Some(vehicle) = b.mount
            && let Some(vp) = self.vehicle_position_by_id(vehicle)
        {
            [vp[0], b.at[1], vp[2]]
        } else {
            b.at
        };
        let mut c = Compound::new();
        if include_id {
            c.insert("id".into(), Tag::String("minecraft:lightning_bolt".into()));
        }
        c.insert("Pos".into(), Tag::List(pos.map(double).to_vec()));
        c.insert(
            "Motion".into(),
            Tag::List(
                [b.velocity.x, b.velocity.y, b.velocity.z]
                    .map(double)
                    .to_vec(),
            ),
        );
        c.insert("Rotation".into(), Tag::List(b.rotation.map(float).to_vec()));
        c.insert("fall_distance".into(), double(b.fall_distance));
        c.insert("Fire".into(), Tag::Short(b.fire_ticks as i16));
        c.insert("Air".into(), Tag::Short(b.air as i16));
        c.insert("OnGround".into(), Tag::Byte(i8::from(b.on_ground)));
        c.insert("Invulnerable".into(), Tag::Byte(i8::from(b.invulnerable)));
        c.insert("PortalCooldown".into(), Tag::Int(b.portal_cooldown));
        if b.invulnerable_time > 0 {
            c.insert("invulnerable_time".into(), Tag::Int(b.invulnerable_time));
        }
        c.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
        if let Some(name) = &b.custom_name {
            c.insert("CustomName".into(), name.clone());
        }
        if b.custom_name_visible {
            c.insert("CustomNameVisible".into(), Tag::Byte(1));
        }
        if b.silent {
            c.insert("Silent".into(), Tag::Byte(1));
        }
        if b.no_gravity {
            c.insert("NoGravity".into(), Tag::Byte(1));
        }
        if b.glowing {
            c.insert("Glowing".into(), Tag::Byte(1));
        }
        if b.ticks_frozen > 0 {
            c.insert("TicksFrozen".into(), Tag::Int(b.ticks_frozen));
        }
        if b.has_visual_fire {
            c.insert("HasVisualFire".into(), Tag::Byte(1));
        }
        if let Some(raw) = self.persist.raw.get(&b.id) {
            if let Some(Tag::List(tags)) = raw.get("Tags")
                && !tags.is_empty()
            {
                c.insert("Tags".into(), Tag::List(tags.clone()));
            }
            if let Some(Tag::Compound(data)) = raw.get("data")
                && !data.is_empty()
            {
                c.insert("data".into(), Tag::Compound(data.clone()));
            }
        }
        Tag::Compound(c)
    }
    fn vehicle_position_by_id(&self, vehicle: u32) -> Option<[f64; 3]> {
        if let Some(target) = self.level_entity(vehicle as i32) {
            return self.target_position(target);
        }
        self.player
            .as_ref()
            .filter(|p| p.entity_id as u32 == vehicle)
            .map(|p| [p.position.x, p.position.y, p.position.z])
    }
    fn vehicle_passenger_pos(&self, vehicle: u32) -> Option<[f64; 3]> {
        if let Some(seat) = self.seat(vehicle) {
            let p = seat.passenger_position(0.0);
            return Some([p.x, p.y, p.z]);
        }
        if let Some(mob) = self.mobs.iter().find(|m| m.id == vehicle && !m.removed) {
            let y = mob.position.y + f64::from(mob.height());
            return Some([mob.position.x, y, mob.position.z]);
        }
        if let Some(boat) = self.boats.iter().find(|b| b.id == vehicle) {
            return Some([boat.position.x, boat.position.y + 0.35, boat.position.z]);
        }
        if let Some(cart) = self.carts.iter().find(|c| c.id == vehicle) {
            return Some([cart.position.x, cart.position.y, cart.position.z]);
        }
        self.vehicle_position_by_id(vehicle)
    }
    /// `ClientLevel.addEntity` for a server's lightning bolt: on a client it
    /// only flashes, sounds and draws (`LightningBolt.tick` off the server).
    pub(super) fn spawn_remote_bolt(&mut self, id: u32, at: [f64; 3]) {
        self.bolts.retain(|b| b.id != id);
        let (bolt, _) = Bolt::new(id, at, true);
        self.lightning_seen[0] += 1;
        self.bolts.push(bolt);
    }
    /// `spawnFire`: fire at the strike point and `extra` random neighbours
    /// (from the bolt's own `random`), wherever `BaseFireBlock.getState` fire
    /// can survive, when a player is within the fire-spread radius. Returns how
    /// many blocks it set on fire.
    fn bolt_fire(&mut self, at: [f64; 3], extra: i32, random: &mut LegacyRandom) -> i32 {
        let mut set = 0;
        let base = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        // ServerLevel.canSpreadFireAround at the bolt: no fire at all, and no
        // draws for the extra sources, away from every player (#11376).
        let origin = BlockPos::new(base[0], base[1], base[2]);
        if !self.with_level(|_, level, _| {
            sunrise_world::level::LevelAccess::can_spread_fire_around(level, origin)
        }) {
            return 0;
        }
        let mut spots = vec![base];
        for _ in 0..extra {
            let mut d = || random.next_int_bound(3).unwrap_or(1) - 1;
            spots.push([base[0] + d(), base[1] + d(), base[2] + d()]);
        }
        for p in spots {
            let state = self.world.block(p[0], p[1], p[2]);
            if !self.res.facts().is_air(state) {
                continue;
            }
            // BaseFireBlock.getState and FireBlock/SoulFireBlock.canSurvive.
            let at = BlockPos::new(p[0], p[1], p[2]);
            let fire = self.with_level(|b, level, ticks| {
                let fire = b.fire_state_for(level, ticks, at);
                b.can_survive(level, ticks, at, fire).then_some(fire)
            });
            if let Some(fire) = fire {
                if !self.native_block_ignite_allowed(at, "lightning") {
                    continue;
                }
                let _ = self.level_set(p, fire);
                set += 1;
            }
        }
        set
    }
    /// `LightningBolt.tick` for every bolt.
    pub(super) fn tick_bolts(&mut self) {
        let _save_lease = sunrise_client::join_seq::visibility::PassLease::enter();
        let server = self.remote.is_none();
        let ticking = self.entity_ticking_area();
        let ids: Vec<u32> = self.bolts.iter().map(|b| b.id).collect();
        for id in ids {
            let Some(i) = self.bolts.iter().position(|b| b.id == id) else {
                continue;
            };
            let mut at = self.bolts[i].at;
            if server {
                // ServerLevel.tick: only entities in entity-ticking chunks tick.
                if !ticking(at[0].floor() as i32, at[2].floor() as i32) {
                    continue;
                }
                let mount = self.bolts[i].mount;
                let mount_gone = mount.is_some_and(|vehicle| {
                    self.level_entity(vehicle as i32).is_none()
                        && !self
                            .player
                            .as_ref()
                            .is_some_and(|p| p.entity_id as u32 == vehicle)
                });
                let below_world = at[1] < f64::from(self.world.bottom_y() - 64);
                self.bolts.update(i, |b| {
                    // Entity.rideTick clears deltaMovement before Entity.tick.
                    if b.mount.is_some() {
                        b.velocity = Vec3::ZERO;
                    }
                    // Entity.commonTick: invulnerableTime counts down.
                    if b.invulnerable_time > 0 {
                        b.invulnerable_time -= 1;
                    }
                    // Entity.baseTick: dismount if vehicle left, portal cooldown, Fire countdown.
                    if mount_gone {
                        b.mount = None;
                    }
                    if b.portal_cooldown > 0 {
                        b.portal_cooldown -= 1;
                    }
                    if b.fire_ticks > 0 {
                        b.fire_ticks -= 1;
                    }
                });
                if below_world {
                    self.remove_bolt(id);
                    continue;
                }
            }
            let (life, silent, visual_only, seed) = {
                let b = &self.bolts[i];
                (b.life, b.silent, b.visual_only, b.seed)
            };
            if life == 2 {
                if !silent {
                    // Separate client sound pitch draws from the server bolt's RNG stream.
                    let (thunder_pitch, impact_pitch) = if server {
                        let mut sound_rng = LegacyRandom::new(seed);
                        (
                            0.8 + sound_rng.next_float() * 0.2,
                            0.5 + sound_rng.next_float() * 0.2,
                        )
                    } else {
                        self.bolts.update(i, |b| {
                            (
                                0.8 + b.random.next_float() * 0.2,
                                0.5 + b.random.next_float() * 0.2,
                            )
                        })
                    };
                    self.play_audio(
                        "minecraft:entity.lightning_bolt.thunder",
                        Some(at),
                        10000.0,
                        thunder_pitch,
                    );
                    self.play_audio(
                        "minecraft:entity.lightning_bolt.impact",
                        Some(at),
                        2.0,
                        impact_pitch,
                    );
                }
                if server {
                    // Only Normal and Hard start fires with the strike itself.
                    if !visual_only && self.ui.difficulty >= 2 {
                        let mut rng = self.bolts[i].random.clone();
                        let set = self.bolt_fire(at, 4, &mut rng);
                        if let Some(i) = self.bolts.iter().position(|b| b.id == id) {
                            self.bolts.update(i, |b| {
                                b.random = rng;
                                b.fires += set;
                            });
                        }
                    }
                    // LightningBolt.tick runs powerLightningRod, clearCopperOnLightningStrike,
                    // then gameEvent(LIGHTNING_STRIKE) with the bolt as its source.
                    self.strike_rod(at);
                    self.bolt_clear_copper(at);
                    let ctx = self.entity_event_context(id, None);
                    self.post_game_event("minecraft:lightning_strike", at, ctx);
                }
            }
            let Some(i) = self.bolts.iter().position(|b| b.id == id) else {
                continue;
            };
            enum Step {
                Discard,
                Reflash,
                Active,
                Idle,
            }
            let step = self.bolts.update(i, |b| {
                b.life -= 1;
                if b.life < 0 {
                    if b.flashes == 0 {
                        return Step::Discard;
                    }
                    if b.life < -b.random.next_int_bound(10).unwrap_or(0) {
                        b.flashes -= 1;
                        b.life = 1;
                        b.seed = b.random.next_long();
                        return Step::Reflash;
                    }
                }
                if b.life >= 0 {
                    Step::Active
                } else {
                    Step::Idle
                }
            });
            match step {
                Step::Discard => {
                    if server {
                        let (fires, hit, hit_items) = {
                            let b = &self.bolts[i];
                            (b.fires, b.strike.hit.clone(), b.strike.hit_items.clone())
                        };
                        self.lightning_strike_triggers(at, fires, &hit, &hit_items);
                    }
                    self.remove_bolt(id);
                    continue;
                }
                Step::Reflash => {
                    if server && !visual_only {
                        let mut rng = self.bolts[i].random.clone();
                        let set = self.bolt_fire(at, 0, &mut rng);
                        if let Some(i) = self.bolts.iter().position(|b| b.id == id) {
                            self.bolts.update(i, |b| {
                                b.random = rng;
                                b.fires += set;
                            });
                        }
                    }
                }
                Step::Active | Step::Idle => {}
            }
            if matches!(step, Step::Reflash | Step::Active) {
                if self.flash == 0 {
                    self.lightning_seen[1] += 1;
                }
                self.flash = 2;
                if server && !visual_only {
                    let Some(i) = self.bolts.iter().position(|b| b.id == id) else {
                        continue;
                    };
                    let (mut strike, cause) = self
                        .bolts
                        .update(i, |b| (std::mem::take(&mut b.strike), b.cause));
                    let struck = self.thunder_hit_state(id, at, &mut strike);
                    if let Some(i) = self.bolts.iter().position(|b| b.id == id) {
                        self.bolts.update(i, |b| b.strike = strike);
                    }
                    if let Some(cause) = cause {
                        let who = cause.map_or(Target::Player, Target::Guest);
                        self.for_player(who, |g| g.trigger_channeled_lightning(&struck));
                    }
                }
            }
            // Entity.rideTick: vehicle.positionRider(this).
            if server
                && let Some(i) = self.bolts.iter().position(|b| b.id == id)
                && let Some(vehicle) = self.bolts[i].mount
                && let Some(seat) = self.vehicle_passenger_pos(vehicle)
            {
                at = seat;
                self.bolts.update(i, |b| b.at = at);
            }
        }
    }
    /// `LightningRodBlock.onLightningStrike`: powered for 8 ticks.
    fn strike_rod(&mut self, at: [f64; 3]) {
        let p = [
            at[0].floor() as i32,
            (at[1] - 1.0e-6).floor() as i32,
            at[2].floor() as i32,
        ];
        let state = self.world.block(p[0], p[1], p[2]);
        if !self.res.facts().get(state).name.ends_with("lightning_rod") {
            return;
        }
        let registry = self.pack.assets.registry;
        if let Some(mut s) = registry.state(state).cloned() {
            s.properties.insert("powered".into(), "true".into());
            if let Some(powered) = registry.id(&s) {
                let _ = self.level_set(p, powered);
                self.with_level(|b, level, ticks| {
                    // `level.scheduleTick(pos, this, 8)`: the unpowering. A rod struck
                    // is the same block as before, so its placement does not schedule
                    // it, and without this it stayed powered for good.
                    b.schedule_tick(level, ticks, BlockPos::new(p[0], p[1], p[2]), 8);
                    // `updateNeighbours`: unconditional; the experiments only decide
                    // whether the update carries an orientation.
                    b.lightning_rod_update_neighbors(
                        level,
                        ticks,
                        BlockPos::new(p[0], p[1], p[2]),
                        state,
                    );
                });
            }
            // Level event 3002 with the rod's axis: sparks along it.
            let axis = match s.properties.get("facing").map(String::as_str) {
                Some("east" | "west") => 0,
                Some("north" | "south") => 2,
                _ => 1,
            };
            self.level_event_at(3002, p, axis);
        }
    }
    /// `LightningBolt.clearCopperOnLightningStrike` at the block the bolt struck.
    fn bolt_clear_copper(&mut self, at: [f64; 3]) {
        let p = BlockPos::new(
            at[0].floor() as i32,
            (at[1] - 1.0e-6).floor() as i32,
            at[2].floor() as i32,
        );
        self.with_level(|b, level, ticks| b.lightning_clear_copper(level, ticks, p));
        let _ = self.process_level_events();
    }
    /// `LightningStrikeTrigger` at the bolt's discard: every player within 256
    /// blocks, with the mobs in its 30-block box that it did not hit.
    fn lightning_strike_triggers(
        &mut self,
        at: [f64; 3],
        fires: i32,
        hit: &[u32],
        hit_items: &[u32],
    ) {
        let bolt = EntityView {
            kind: "minecraft:lightning_bolt".into(),
            position: at,
            blocks_set_on_fire: fires,
            ..Default::default()
        };
        let viewers: Vec<EntityView> = self
            .mobs
            .iter()
            .filter(|m| m.alive() && !hit.contains(&m.id))
            .filter(|m| in_viewer_box(at, [m.position.x, m.position.y, m.position.z]))
            .map(|m| self.mob_entity(m))
            .collect();
        // The item entities in the box are viewers too (`getEntities` is every entity).
        let mut viewers = viewers;
        viewers.extend(
            self.dropped
                .iter()
                .filter(|e| !hit_items.contains(&e.id))
                .filter(|e| in_viewer_box(at, [e.position.x, e.position.y, e.position.z]))
                .map(|e| self.item_entity_view(e)),
        );
        // The players are entities in that box too, unless the bolt struck
        // them (`thunderHit` reaches 3 blocks, 9 up: `hitEntities`).
        let struck = |p: [f64; 3]| {
            (p[0] - at[0]).abs() <= 3.0
                && (p[2] - at[2]).abs() <= 3.0
                && p[1] >= at[1] - 3.0
                && p[1] <= at[1] + 9.0
        };
        self.for_each_player(|g| {
            let Some(p) = g.player.as_ref().filter(|p| !p.health.dead_or_dying()) else {
                return;
            };
            let pos = [p.position.x, p.position.y, p.position.z];
            if in_viewer_box(at, pos) && !struck(pos) {
                viewers.push(g.player_entity_for("minecraft:lightning_strike"));
            }
        });
        self.for_each_player(|g| {
            let Some(p) = g.player.as_ref().map(|p| p.position) else {
                return;
            };
            let d2 = (p.x - at[0]).powi(2) + (p.y - at[1]).powi(2) + (p.z - at[2]).powi(2);
            if d2 < 256.0 * 256.0 {
                g.trigger_lightning_strike(&bolt, &viewers);
            }
        });
    }
    /// How many entities of `stage` the strike reaches: one loop per list over
    /// the borrowed slices, where a kind that takes no stage folds away, so a
    /// stage nobody is in is skipped without walking the handles.
    fn thunder_count(&self, strike: Aabb, stage: ThunderHit) -> usize {
        let lists = entity_lists_of!(self);
        let mut n = 0;
        for_each_entity!(lists, e => {
            if e.thunder_hit() == stage && strike.intersects(e.thunder_bounds()) {
                n += 1;
            }
        });
        n
    }
    /// Whether the strike reaches `h` and its kind takes `stage`.
    fn thunder_struck(&self, h: EntityHandle, strike: Aabb, stage: ThunderHit) -> bool {
        with_entity!(self, h, e => e.thunder_hit() == stage && strike.intersects(e.thunder_bounds()))
    }
    /// `thunderHit` for entities whose box meets the strike box, 3 blocks
    /// around and up to 9 above (`getEntities(this, AABB)` tests boxes, not
    /// positions). Returns the mobs in that box before the hit, as
    /// `CHANNELED_LIGHTNING` counts them. Boats, minecarts and experience
    /// orbs are hit too (`VehicleEntity.hurtServer` adds 5 x 10 > 40 and
    /// destroys them; an orb has 5 health).
    #[cfg(test)]
    pub(in crate::game) fn thunder_hit(&mut self, b: &mut Bolt) -> Vec<EntityView> {
        self.thunder_hit_state(b.id, b.at, &mut b.strike)
    }
    fn thunder_hit_state(
        &mut self,
        bolt: u32,
        at: [f64; 3],
        state: &mut StrikeState,
    ) -> Vec<EntityView> {
        let strike = Aabb::new(
            Vec3::new(at[0] - 3.0, at[1] - 3.0, at[2] - 3.0),
            Vec3::new(at[0] + 3.0, at[1] + 9.0, at[2] + 3.0),
        );
        // Whether the box of an entity standing at `p` (half width `w`,
        // height `h`) meets the strike box.
        let meets = move |p: [f64; 3], w: f64, h: f64| {
            strike.intersects(Aabb::new(
                Vec3::new(p[0] - w, p[1], p[2] - w),
                Vec3::new(p[0] + w, p[1] + h, p[2] + w),
            ))
        };
        let damage = Damage {
            kind: DamageKind::Generic,
            attacker: None,
            from: Some(at),
            direct: None,
            attacker_species: None,
        };
        let mut struck = Vec::new();
        let mut powered = Vec::new();
        let peaceful = self.ui.difficulty == 0;
        let mut left = self.thunder_count(strike, ThunderHit::Mob);
        for h in self.entity_handles() {
            if left == 0 {
                break;
            }
            let EntityHandle::Mob(i) = h else { continue };
            if !self.thunder_struck(h, strike, ThunderHit::Mob) {
                continue;
            }
            left -= 1;
            struck.push(self.mob_entity(&self.mobs[i]));
            // LivingConversionEvent.Pre: a canceled conversion leaves the
            // mob to the lightning's fire and damage, as `super.thunderHit`.
            let outcome = match self.mobs[i].species() {
                Species::Pig if !peaceful => Some("minecraft:zombified_piglin"),
                Species::Villager if !peaceful => Some("minecraft:witch"),
                _ => None,
            };
            state.hit.push(self.mobs[i].id);
            if !self.lightning_may_strike(self.mobs[i].id, bolt) {
                continue;
            }
            if let Some(outcome) = outcome {
                let old = self.mobs[i].clone();
                let params = sunrise_client::mobs::conversion::Params {
                    keep_equipment: false,
                    preserve_can_pick_up_loot: old.species() == Species::Pig,
                };
                if self
                    .convert_to(&old, outcome, params, false, |_, _| {})
                    .is_some()
                {
                    continue;
                }
            }
            let m = &mut self.mobs[i];
            let at = [m.position.x, m.position.y, m.position.z];
            match m.species() {
                Species::Mooshroom => {
                    if !state.converted.contains(&m.id) {
                        state.converted.push(m.id);
                        m.extra.variant = 1 - m.extra.variant.min(1);
                        self.play_audio("minecraft:entity.mooshroom.convert", Some(at), 2.0, 1.0);
                    }
                    continue;
                }
                Species::Turtle => {
                    m.damage(
                        crate::game::entity::damage_level_of!(
                            self,
                            &sunrise_client::health::NO_ATTACK,
                            no_push,
                            None
                        ),
                        f32::MAX,
                        damage,
                    );
                    self.finish_mob_damage_at(i);
                    continue;
                }
                Species::Creeper => {
                    m.extra.charged = true;
                    powered.push(m.id);
                }
                // CopperGolem.thunderHit (after Entity.thunderHit's fire and
                // damage): once per bolt, an oxidized golem goes back one step
                // and its weathering timer (or wax, -2) is reset to -1.
                Species::CopperGolem if !state.converted.contains(&m.id) => {
                    state.converted.push(m.id);
                    if m.extra.variant > 0 {
                        m.extra.vector[0] = -1.0;
                        m.extra.variant =
                            sunrise_client::mobs::copper_golem::weather_previous(m.extra.variant);
                        self.persist.force_meta(m.id, 16);
                    }
                }
                _ => {}
            }
            // Entity.thunderHit: burning (8 seconds) and 5 lightning damage.
            if !m.mod_fire_immune {
                m.fire_ticks += 1;
                if m.fire_ticks == 0 {
                    m.ignite_for_ticks(160);
                }
            }

            m.damage(
                crate::game::entity::damage_level_of!(
                    self,
                    &sunrise_client::health::NO_ATTACK,
                    no_push,
                    None
                ),
                5.0,
                damage,
            );
            self.finish_mob_damage_at(i);
        }
        for entity in powered {
            self.native_creeper_power(entity, [at[0], at[1], at[2]]);
        }
        // Cushion.thunderHit: a cushion whose box meets the strike box breaks and drops its item
        // (the bolt is no player, so no creative spare). Last first, since
        // break_cushion removes the entry.
        for i in (0..self.cushions.len()).rev() {
            let (lo, hi) = self.cushions[i].bounds();
            if strike.intersects(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )) && self.lightning_may_strike(self.cushions[i].id, bolt)
            {
                let before = self.dropped.len();
                self.break_cushion(i, false);
                state
                    .spared
                    .extend(self.dropped[before..].iter().map(|e| e.id));
            }
        }
        // Vehicles, then orbs: `VehicleEntity.hurtServer` for 5 damage (x 10
        // over the 40 a vehicle takes) destroys them. Lightning is no fire
        // source, so a TNT cart does not prime. `ExperienceOrb.hurtServer`:
        // 5 damage off 5 health. Last first, since each removes its entry.
        let mut left = self.thunder_count(strike, ThunderHit::Vehicle)
            + self.thunder_count(strike, ThunderHit::Orb);
        for h in self.entity_handles_rev() {
            if left == 0 {
                break;
            }
            if !self.thunder_struck(h, strike, ThunderHit::Vehicle)
                && !self.thunder_struck(h, strike, ThunderHit::Orb)
            {
                continue;
            }
            left -= 1;
            if !self.lightning_may_strike(with_entity!(self, h, e => e.id()), bolt) {
                continue;
            }
            match h {
                EntityHandle::Boat(i) => {
                    let id = self.boats[i].id;
                    let kind = self.boats[i].entity_type();
                    let source = sunrise_client::health::HurtSource {
                        damage_type: "minecraft:lightning_bolt",
                        causing: bolt as i32,
                        direct: bolt as i32,
                        position: Some(at),
                        direct_type: None,
                    };
                    let vanilla = self.saved_refuses_damage(id, "minecraft:lightning_bolt", false);
                    if !self.native_nonliving_invulnerable(id, &kind, source, vanilla) {
                        self.hurt_boat(i, 5.0, false);
                    }
                }
                EntityHandle::Cart(i) => {
                    let (id, kind) = (self.carts[i].id, self.carts[i].kind.id());
                    let source = sunrise_client::health::HurtSource {
                        damage_type: "minecraft:lightning_bolt",
                        causing: bolt as i32,
                        direct: bolt as i32,
                        position: Some(at),
                        direct_type: None,
                    };
                    let vanilla = self.saved_refuses_damage(id, "minecraft:lightning_bolt", false);
                    if !self.native_nonliving_invulnerable(id, kind, source, vanilla) {
                        self.hurt_cart(i, 5.0, false, false, CartHit::default());
                    }
                }
                EntityHandle::Orb(i) => {
                    let id = self.orbs[i].id;
                    let source = sunrise_client::health::HurtSource {
                        damage_type: "minecraft:lightning_bolt",
                        causing: bolt as i32,
                        direct: bolt as i32,
                        position: Some(at),
                        direct_type: None,
                    };
                    let vanilla = self.saved_refuses_damage(id, "minecraft:lightning_bolt", false);
                    if !self.native_nonliving_invulnerable(
                        id,
                        "minecraft:experience_orb",
                        source,
                        vanilla,
                    ) && self.orbs[i].hurt(5.0)
                    {
                        self.orbs.remove(i);
                    }
                }
                _ => {}
            }
        }
        self.native_leaves_flush();
        // Every player in the box, the server's players too (0.6 x 1.8).
        let strike_players = move |g: &mut Self| {
            if let Some(p) = g.player.as_ref().map(|p| p.position)
                && meets([p.x, p.y, p.z], 0.3, 1.8)
                && g.lightning_may_strike(g.own_entity_id() as u32, bolt)
            {
                if let Some(p) = g.player.as_mut()
                    && !p.mod_fire_immune
                {
                    p.fire_ticks += 1;
                    if p.fire_ticks == 0
                        && let Err(error) = p.ignite_for_ticks(160)
                    {
                        g.error = Some(error);
                    }
                }
                g.hurt_player_by("minecraft:lightning_bolt", 5.0);
            }
        };
        strike_players(self);
        self.each_guest_hit(strike_players);
        // Items run Entity.thunderHit, then the shared ItemEntity hurt path.
        let mut left = self.thunder_count(strike, ThunderHit::Item);
        for h in self.entity_handles_rev() {
            if left == 0 {
                break;
            }
            let EntityHandle::Item(i) = h else { continue };
            if !self.thunder_struck(h, strike, ThunderHit::Item) {
                continue;
            }
            left -= 1;
            let e = &self.dropped[i];
            state.hit_items.push(e.id);
            if !self.lightning_may_strike(e.id, bolt) {
                continue;
            }
            let e = &mut self.dropped[i];
            e.fire_ticks += 1;
            if e.fire_ticks == 0 {
                e.fire_ticks = 160;
            }
            // Cushion spare stands in for vanilla's invulnerableTime window.
            if e.age < 20 && state.spared.contains(&e.id) {
                continue;
            }
            let id = e.id;
            self.hurt_item_entity(
                id,
                5.0,
                "minecraft:lightning_bolt",
                Some(Target::Bolt(bolt)),
                None,
                Some(at),
            );
        }
        self.native_leaves_flush();
        struck
    }
    /// `LightningBolt.shouldRenderAtSqrDistance`: a bolt is drawn within 64
    /// blocks times the view scale of the camera, and past that only its sky
    /// flash and thunder remain. Simulation-order render inputs; seeded
    /// geometry lives in the renderer.
    pub(super) fn bolt_instances(
        &self,
        camera: glam::DVec3,
    ) -> Vec<sunrise_client::gfx::lightning::Bolt> {
        let range = 64.0 * self.view_scale();
        self.bolts
            .iter()
            .filter(|b| {
                !self.hides_as_camera(b.id) && {
                    let at = glam::DVec3::new(b.at[0], b.at[1], b.at[2]);
                    at.distance_squared(camera) < range * range
                }
            })
            .map(|b| sunrise_client::gfx::lightning::Bolt {
                at: b.at,
                seed: b.seed,
            })
            .collect()
    }
}

impl Game<'_> {
    /// `EntityStruckByLightningEvent`, before `thunderHit` on entity `entity`
    /// from bolt `bolt`: false when a mod cancelled the hit. The entity still
    /// counts as struck (`hitEntities`).
    fn lightning_may_strike(&mut self, entity: u32, bolt: u32) -> bool {
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::EntityStruckByLightning)
        {
            return true;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return true;
        };
        let mut cancel = false;
        self.native_entity_struck_by_lightning(
            mod_api::EntityRef::new(epoch, entity),
            mod_api::EntityRef::new(epoch, bolt),
            &mut cancel,
        );
        !cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::collect_entities_in;
    use mod_api::{self, ServerHost};
    use std::sync::Arc;
    use sunrise_client::mobs::{Anim, Brain, Mob, MobType, Part, Species};
    use sunrise_core::geometry::Vec3;

    /// `LightningBolt.spawnFire` places nothing, and draws no extra sources,
    /// unless `canSpreadFireAround` passes at the bolt (#11376).
    #[test]
    #[ignore = "daily"]
    fn a_bolt_away_from_players_starts_no_fire() {
        use sunrise_client::gamerules::Value;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let air = game.res.facts().air;
            game.world.set(8, 64, 8, stone).unwrap();
            let bolt = [8.5, 65.0, 8.5];
            // (radius, player's x): fires expected at the strike.
            for (radius, player_x, fires) in [
                (128, 12.5, true),
                (1, 12.5, false),
                (0, 8.5, false),
                (-1, 1000.0, true),
            ] {
                game.world.set(8, 65, 8, air).unwrap();
                game.ui
                    .rules
                    .set("fire_spread_radius_around_player", Value::Int(radius));
                game.player = Some(
                    sunrise_client::motion::PlayerMotion::new(
                        game.motion,
                        Vec3::new(player_x, 65.0, 8.5),
                    )
                    .unwrap(),
                );
                let random = game.world.random.clone();
                let mut bolt_random = LegacyRandom::new(0);
                let set = game.bolt_fire(bolt, 4, &mut bolt_random);
                let burning = game.world.block(8, 65, 8) != air;
                assert_eq!((set > 0, burning), (fires, fires), "radius {radius}");
                if !fires {
                    // Neither the bolt nor level RNG draws when fire is disallowed.
                    assert_eq!(bolt_random.next_long(), LegacyRandom::new(0).next_long());
                    let mut before = random;
                    assert_eq!(game.world.random.next_long(), before.next_long());
                }
            }
        });
    }

    /// `LightningRodBlock.onLightningStrike` updates the neighbours of the block
    /// in front of the rod whether or not redstone experiments are on (#14371):
    /// a wire beside that block is strongly powered by the struck rod.
    #[test]
    #[ignore = "daily"]
    fn a_struck_rod_updates_the_wire_beside_the_block_behind_it() {
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let wire = registry.default_state("minecraft:redstone_wire").unwrap();
        let mut rod = registry
            .state(registry.default_state("minecraft:lightning_rod").unwrap())
            .cloned()
            .unwrap();
        rod.properties.insert("facing".into(), "up".into());
        let rod = registry.id(&rod).unwrap();
        // The rod on stone at (8, 64, 8); wire on stone beside that block.
        game.world.set(8, 64, 8, stone).unwrap();
        game.world.set(9, 63, 8, stone).unwrap();
        game.world.set(9, 64, 8, wire).unwrap();
        game.world.set(8, 65, 8, rod).unwrap();
        assert!(
            !game
                .level
                .enabled_features
                .iter()
                .any(|feature| feature == "minecraft:redstone_experiments")
        );
        game.strike_rod([8.5, 66.0, 8.5]);
        let power = registry
            .state(game.world.block(9, 64, 8))
            .unwrap()
            .properties
            .get("power")
            .cloned();
        assert_eq!(power.as_deref(), Some("15"));
    }

    #[test]
    fn viewer_box_reaches_15_around_and_down_and_21_up() {
        let bolt = [0.0, 64.0, 0.0];
        assert!(in_viewer_box(bolt, [15.0, 49.0, -15.0]));
        assert!(in_viewer_box(bolt, [0.0, 85.0, 0.0]));
        assert!(!in_viewer_box(bolt, [0.0, 85.5, 0.0]));
        assert!(!in_viewer_box(bolt, [0.0, 48.5, 0.0]));
        assert!(!in_viewer_box(bolt, [15.5, 64.0, 0.0]));
    }

    #[test]
    #[ignore = "daily"]
    fn conversion_post_reads_original_and_lightning_outcome() {
        use super::super::native_entity_refs::tests::{fixture_resident_column, with_game};
        use mod_api::{
            EntityJoinLevelEvent, EquipmentSlot, Flow, Hooks, LivingConversionPostEvent,
            LivingConversionPreEvent,
        };
        use mod_platform::__private::install;
        use std::sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };

        #[derive(Debug)]
        struct SeenConversion {
            old: Option<String>,
            outcome: Option<String>,
            weapon: Option<String>,
            mushrooms: usize,
        }
        struct Observer {
            seen: Arc<Mutex<Vec<SeenConversion>>>,
            pre: Arc<AtomicUsize>,
            cancel_cow: Arc<AtomicBool>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn living_conversion_pre(
                &self,
                _: &mut dyn ServerHost,
                _: &mut LivingConversionPreEvent<'_>,
            ) -> Flow {
                self.pre.fetch_add(1, Ordering::SeqCst);
                Flow::Continue
            }
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                if event.kind == "minecraft:cow" && self.cancel_cow.load(Ordering::SeqCst) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn living_conversion_post(
                &self,
                host: &mut dyn ServerHost,
                event: &mut LivingConversionPostEvent<'_>,
            ) {
                let old = host
                    .entity(event.entity)
                    .and_then(|entity| entity.kind(host)?.id(host))
                    .map(|name| name.as_str().to_owned());
                let outcome = host.entity(event.converted);
                let weapon = outcome
                    .and_then(|entity| host.living(entity))
                    .and_then(|living| {
                        living.with_equipment(host, EquipmentSlot::MainHand, |stack| {
                            mod_api::__private::item_key_id(stack.item())
                                .unwrap()
                                .to_string()
                        })
                    });
                let mushrooms = host
                    .level("minecraft:overworld")
                    .and_then(|level| {
                        collect_entities_in(host, level, [-10.0, 60.0, -10.0], [20.0, 70.0, 20.0])
                            .ok()
                    })
                    .unwrap()
                    .into_iter()
                    .filter_map(|entity| host.item_entity(entity))
                    .filter(|item| {
                        item.with_stack(host, |stack| {
                            mod_api::__private::item_key_id(stack.item())
                                .unwrap()
                                .ends_with("mushroom")
                        }) == Some(true)
                    })
                    .count();
                self.seen.lock().unwrap().push(SeenConversion {
                    old,
                    outcome: outcome
                        .and_then(|entity| entity.kind(host)?.id(host))
                        .map(|name| name.as_str().to_owned()),
                    weapon,
                    mushrooms,
                });
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let pre = Arc::new(AtomicUsize::new(0));
        let cancel_cow = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = Arc::from([mod_api::ModEntry::new(
            "lightning-observer",
            Box::new(Observer {
                seen: seen.clone(),
                pre: pre.clone(),
                cancel_cow: cancel_cow.clone(),
            }),
        )]);
        install(mods);
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            fixture_resident_column(&mut game.world, (6, 6));
            let piglin_type = game
                .res
                .mob_types()
                .get("minecraft:piglin")
                .unwrap()
                .clone();
            game.next_entity += 1;
            let mut old = Mob::new(
                game.next_entity,
                piglin_type,
                Vec3::new(100.0, 64.0, 100.0),
                0.0,
            );
            old.removed = true; // The tick loop has already taken this mob out.
            let converted = game
                .convert_to(
                    &old,
                    "minecraft:zombified_piglin",
                    sunrise_client::mobs::conversion::Params::KEEP_ALL,
                    false,
                    |_, _| {},
                )
                .unwrap();
            assert!(game.entity(old.id).is_none());
            assert!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == converted)
                    .unwrap()
                    .has_effect("minecraft:nausea")
            );

            let pig_type = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.next_entity += 1;
            let pig_id = game.next_entity;
            game.mobs.push(Mob::new(
                pig_id,
                pig_type.clone(),
                Vec3::new(0.0, 64.0, 0.0),
                0.0,
            ));
            game.ui.difficulty = 2;
            let mut bolt = Bolt::struck_at(0, [0.0, 64.0, 0.0]);
            game.thunder_hit(&mut bolt);
            assert!(game.entity(pig_id).is_none());
            let mushroom_type = game
                .res
                .mob_types()
                .get("minecraft:mooshroom")
                .unwrap()
                .clone();
            for (x, player) in [(5, true), (10, false)] {
                game.next_entity += 1;
                let id = game.next_entity;
                game.mobs.push(Mob::new(
                    id,
                    mushroom_type.clone(),
                    Vec3::new(f64::from(x) + 0.5, 64.0, 0.5),
                    0.0,
                ));
                if player {
                    game.inventory[game.selected] =
                        Some(sunrise_client::items::ItemStack::new("minecraft:shears", 1));
                    assert!(
                        game.interact_mob(Some(super::super::InteractTarget {
                            id: id as i32,
                            location: [0.0; 3]
                        }))
                        .unwrap()
                    );
                } else {
                    let shears = sunrise_client::items::ItemStack::new("minecraft:shears", 1);
                    assert!(game.shear_entity((x, 64, 0), &shears).unwrap());
                }
            }
            assert_eq!(pre.load(Ordering::SeqCst), 4);
            {
                let seen = seen.lock().unwrap();
                assert_eq!(seen.len(), 4);
                assert_eq!(seen[2].mushrooms, 5);
                assert_eq!(seen[3].mushrooms, 10);
            }
            cancel_cow.store(true, Ordering::SeqCst);
            game.next_entity += 1;
            let old_id = game.next_entity;
            let old = Mob::new(old_id, mushroom_type, Vec3::new(15.0, 64.0, 0.0), 0.0);
            game.mobs.push(old.clone());
            game.next_entity += 1;
            let rider_id = game.next_entity;
            let mut rider = Mob::new(rider_id, pig_type, Vec3::new(15.0, 64.0, 0.0), 0.0);
            rider.extra.mount = Some(old_id);
            game.mobs.push(rider);
            assert!(
                game.convert_to(
                    &old,
                    "minecraft:cow",
                    sunrise_client::mobs::conversion::Params::KEEP_ALL,
                    false,
                    |_, _| {}
                )
                .is_none()
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == rider_id)
                    .unwrap()
                    .extra
                    .mount,
                Some(old_id)
            );
            assert!(!game.mobs.iter().find(|m| m.id == old_id).unwrap().removed);
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 4);
            assert_eq!(seen[0].old.as_deref(), Some("minecraft:piglin"));
            assert_eq!(
                seen[0].outcome.as_deref(),
                Some("minecraft:zombified_piglin")
            );
            assert_eq!(seen[1].old.as_deref(), Some("minecraft:pig"));
            assert_eq!(
                seen[1].outcome.as_deref(),
                Some("minecraft:zombified_piglin")
            );
            assert!(matches!(
                seen[1].weapon.as_deref(),
                Some("minecraft:golden_sword" | "minecraft:golden_spear")
            ));
        });
    }

    /// `ServerLevel.tickThunder` picks its strike column from `Level.randValue`
    /// (`Ticks::block_random_pos`), using bits `2..=5` for X and `10..=13` for Z
    /// rather than drawing from `Level.random`.
    #[test]
    fn thunder_strike_column_uses_block_random_pos_lcg_bits() {
        let mut ticks = Ticks::new(12345);
        let mut expected_lcg = 12345i32;
        for (cx, cz) in [(0, 0), (-3, 5), (12, -8)] {
            expected_lcg = expected_lcg.wrapping_mul(3).wrapping_add(1_013_904_223);
            let r = expected_lcg >> 2;
            let expected = (cx * 16 + (r & 15), cz * 16 + ((r >> 8) & 15));
            assert_eq!(thunder_strike_column(&mut ticks, cx, cz), expected);
        }
    }

    /// `Pig.thunderHit`: striking a pig with lightning converts it to a zombified
    /// piglin while preserving age (#7510).
    #[test]
    fn a_baby_pig_hit_by_lightning_becomes_a_baby_zombified_piglin() {
        let make_pig = |baby: bool| {
            let kind = Arc::new(MobType {
                type_key: Default::default(),
                shadow: None,
                id: "minecraft:pig".into(),
                species: Species::Pig,
                width: 0.9,
                height: 0.9,
                eye_height: 0.85,
                category: "creature".into(),
                max_health: 10.0,
                speed: 0.25,
                flying_speed: 0.02,
                attack_damage: 0.0,
                attack_knockback: 0.0,
                follow_range: 16.0,
                step_height: 0.6,
                armor: 0.0,
                armor_toughness: 0.0,
                knockback_resistance: 0.0,
                jump_strength: 0.42,
                attributes: Default::default(),
                item_stack_limits: Default::default(),
                attribute_bounds: Default::default(),
                gravity: 0.08,
                safe_fall_distance: 3.0,
                fall_damage_multiplier: 1.0,
                water_efficiency: 0.0,
                water_slowdown: 0.8,
                air_speed: 0.02,
                baby_scale: 0.5,
                head_rot_speed: 10.0,
                max_head_x_rot: 40.0,
                max_head_y_rot: 75.0,
                ambient_interval: 80,
                flags: Default::default(),
                model: Arc::new(Part {
                    name: "root".into(),
                    pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    polygons: Vec::new(),
                    children: Vec::new(),
                    layer: None,
                }),
                layers: Default::default(),
                texture: String::new(),
                brain: Brain::Passive,
                anim: Anim::Quadruped,
                sounds: ["", "", ""],
                sound_overrides: Default::default(),
                food: Default::default(),
                tempt: Default::default(),
                passenger_attachments: Vec::new(),
            });
            let mut pig = Mob::new(1, kind, Vec3::ZERO, 45.0);
            if baby {
                pig.extra.age = -24000;
            }
            pig
        };

        let adult_pig = make_pig(false);
        assert!(!adult_pig.is_baby());
        let baby_pig = make_pig(true);
        assert!(baby_pig.is_baby());
        let mut kind = (*adult_pig.kind).clone();
        kind.species = Species::ZombifiedPiglin;
        kind.id = "minecraft:zombified_piglin".into();
        let params = sunrise_client::mobs::conversion::Params {
            keep_equipment: false,
            preserve_can_pick_up_loot: true,
        };
        for (pig, baby) in [(&adult_pig, false), (&baby_pig, true)] {
            let mut zombified = Mob::new(2, Arc::new(kind.clone()), Vec3::ZERO, 0.0);
            let mut pig = pig.clone();
            sunrise_client::mobs::conversion::single(&pig, &mut zombified, params);
            sunrise_client::mobs::conversion::pig_thunder_hit(&mut pig, &mut zombified);
            assert_eq!(zombified.is_baby(), baby);
            assert_eq!(zombified.yaw, 45.0);
            assert!(zombified.persistent);
        }
    }

    /// Spares the entity whose id it holds, and logs each (entity, bolt) asked.
    struct Spare(u32, Arc<std::sync::Mutex<Vec<(u32, u32)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Spare {
        fn entity_struck_by_lightning(
            &self,
            _: &mut dyn ServerHost,
            e: &mut mod_api::EntityStruckByLightningEvent,
        ) -> mod_api::Flow {
            self.1.lock().unwrap().push((e.entity.id(), e.bolt.id()));
            if e.entity.id() == self.0 {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    /// `EntityStruckByLightningEvent`: each struck pig is asked with the bolt;
    /// the spared one stays a pig, the other becomes a zombified piglin, and
    /// both still count as hit.
    #[test]
    #[ignore = "daily"]
    fn a_mod_spares_one_pig_from_the_bolt() {
        let heard = Arc::default();
        let (spared, struck) = (900, 901);
        let mods = vec![mod_api::ModEntry::new(
            "spare",
            Box::new(Spare(spared, Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.ui.difficulty = 2;
                let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                for (id, x) in [(spared, 0.0), (struck, 1.0)] {
                    game.mobs
                        .push(Mob::new(id, pig.clone(), Vec3::new(x, 64.0, 0.0), 0.0));
                }
                let mut bolt = Bolt::struck_at(77, [0.5, 64.0, 0.5]);
                game.thunder_hit(&mut bolt);
                let kind = |game: &Game<'_>, id| {
                    game.mobs
                        .iter()
                        .find(|m| m.id == id && !m.removed)
                        .map(|m| m.species())
                };
                assert_eq!(kind(game, spared), Some(Species::Pig));
                assert_eq!(kind(game, struck), None, "converted");
                let spared_pig = game
                    .mobs
                    .iter()
                    .find(|m| m.id == spared && !m.removed)
                    .unwrap();
                assert!(spared_pig.fire_ticks <= 0, "no fire");
                let mut heard = heard.lock().unwrap().clone();
                heard.sort();
                assert_eq!(heard, [(spared, 77), (struck, 77)]);
                bolt.strike.hit.sort();
                assert_eq!(bolt.strike.hit, [spared, struck]);
            });
        });
    }
}

// A conversion keeps the old mob's state; lives here for the lightning's.
#[cfg(test)]
#[path = "tests/mob_conversion_state_tests.rs"]
mod mob_conversion_state_tests;
