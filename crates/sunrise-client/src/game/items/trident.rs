//! Tridents: `TridentItem.releaseUsing` (throw after 10 ticks, or Riptide in
//! water or rain) and `ThrownTrident` (arrow flight with 0.99 water inertia,
//! 8 damage plus Impaling, the reversed bounce off a hit, sticking, Loyalty's
//! return to the owner, pickup) with `ThrownTridentRenderer`, and Channeling's
//! lightning on a hit (`channeling.json`).
use super::level_sounds::LevelSound;
use super::{Game, commands::Target, minecarts::CartHit, native_projectiles::ImpactTransaction};
use mod_api::{SoundSource, TransactionSeal};
use sunrise_client::{
    arrows::{Pickup, back_off, clip_block_hit},
    collisions::SceneCollisions,
    enchant::EffectContext,
    items::ItemStack,
    join_seq::JoinSeq,
    mesher::Vertex,
    mobs::{Damage, DamageKind, Species, Who, anim::Poses},
    projectiles::{Hit, ProjectileWorld},
};
use sunrise_core::{
    geometry::{Aabb, Vec3, traverse_blocks},
    position::BlockPos,
    random::RandomSource,
};

pub(super) use sunrise_gameplay::trident::{HALF, ThrownTrident, breeze_deflect, segment_box};
/// `ThrownTrident.tick` on a server's client: it flies on by itself
/// between the server's updates (`AbstractArrow.tick`: gravity and drag,
/// a block holds it), and Loyalty pulls it back to its owner; only the
/// server decides what it hits.
pub(super) fn remote_tick(
    t: &mut ThrownTrident,
    w: &impl ProjectileWorld,
    owner_eye: Option<Vec3>,
) {
    t.previous = t.position;
    t.shake -= i32::from(t.shake > 0);
    if t.in_ground_time > 4 {
        t.dealt_damage = true;
    }
    if t.loyalty > 0
        && (t.dealt_damage || t.no_physics)
        && let Some(eye) = owner_eye
    {
        t.no_physics = true;
        t.in_ground = false;
        t.loyalty_pull(eye);
    }
    // AbstractArrow.tick releases a changed block only on the server;
    // clients release from the non-SELF move after a piston push.
    if t.in_ground && !t.no_physics {
        t.in_ground_time += 1;
        return;
    }
    t.in_ground_time = 0;
    let m = t.velocity;
    let (x, z) = if t.no_physics {
        (-m.x, -m.z)
    } else {
        (m.x, m.z)
    };
    // Projectile.updateRotation's targets: Mth.atan2 times the folded 180F / (float)PI, the pitch over
    // Vec3.horizontalDistance.
    t.yaw = sunrise_core::geometry::trig::atan2_degrees(x, z);
    t.pitch = sunrise_core::geometry::trig::atan2_degrees(m.y, m.horizontal_distance());
    let to = t.position.add_vector(m);
    if !t.no_physics
        && let Some((at, _, _)) = w.clip(t.position, to)
    {
        // AbstractArrow.onHitBlock's back-off, as the server's tick does it.
        t.position = back_off(at, m);
        t.velocity = Vec3::ZERO;
        t.in_ground = true;
        t.shake = 7;
        return;
    }
    t.position = to;
    // A trident's water inertia (0.99) equals the air drag.
    t.velocity = t.velocity.scale(0.99);
    if !t.no_physics && !t.no_gravity {
        t.velocity.y -= 0.05;
    }
}
/// A player a trident can return to (its owner) or that can pick it up: this
/// game's own, or a server's guest in this level, which is all a server has.
struct Catcher {
    /// The guest's connection; `None` for this game's player.
    guest: Option<u32>,
    /// The guest's entity id (unused for this game's player).
    id: i32,
    feet: Vec3,
    eye: Vec3,
    /// `Player.aiStep` touches things only while alive and not a spectator.
    touches: bool,
    creative: bool,
    uuid: Option<[i32; 4]>,
}
impl Game<'_> {
    /// The trident stack's maximum damage component, else its item's default.
    pub(super) fn trident_max_damage(&self, stack: &ItemStack) -> u32 {
        stack
            .data()
            .and_then(|data| match data.other("minecraft:max_damage") {
                Some(sunrise_core::nbt::Tag::Int(value)) => u32::try_from(*value).ok(),
                _ => None,
            })
            .unwrap_or_else(|| self.items().get(&stack.item).map_or(250, |i| i.max_damage))
    }
    /// `TridentItem.use`: start the throw unless one more use would break it.
    /// Returns false for `InteractionResult.FAIL` (about to break, or Riptide
    /// while dry) so callers neither dip the hand nor consume the click.
    pub(super) fn use_trident(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:trident" {
            return false;
        }
        let max = self.trident_max_damage(stack);
        let riptide = stack.enchantment_level("minecraft:riptide") > 0;
        if stack.damage >= max.saturating_sub(1) || (riptide && !self.in_water_or_rain()) {
            return false;
        }
        self.start_using_item();
        true
    }
    pub(in crate::game) fn in_water_or_rain(&mut self) -> bool {
        let Some(p) = &self.player else { return false };
        let (x, y, z) = (
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        );
        let water = self.res.facts().get(self.world.block(x, y, z)).fluid
            == sunrise_client::blocks::Fluid::Water;
        let (at, top) = (p.position, p.bounds().max_position().y);
        water || self.in_rain(at, top)
    }
    /// The `summon_entity` and `play_sound` effects of `channeling.json`: in a
    /// thunderstorm (`weather_check`) a spot that sees the sky (`can_see_sky`)
    /// gets a lightning bolt and the thunder sound.
    fn channel_lightning(&mut self, at: [f64; 3], guest: Option<u32>) {
        // Level.isThundering: the thunder level, in dimensions with weather.
        if !self.can_have_weather() || self.weather_levels(1.0).1 <= 0.9 {
            return;
        }
        // BlockAndLightGetter.canSeeSky: full sky light.
        let (x, y, z) = (
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        );
        if self.world.light(x, y, z) >> 4 != 15 {
            return;
        }
        if !self.lightning_strike_allowed(self.lan_acting_uuid, [x, y, z], "trident") {
            return;
        }
        self.spawn_bolt(at, false);
        // The summon effect's owner is the thrower: LightningBolt.setCause.
        if let Some(bolt) = self.bolts.last_mut() {
            bolt.cause = Some(guest);
        }
        self.audio
            .play("minecraft:item.trident.thunder", Some(at), 5.0, 1.0);
    }
    /// `minecraft:hit_block` of Channeling (`AbstractArrow.hitBlockEnchantmentEffects`):
    /// the block the trident's path first hits, if it is a lightning rod, with
    /// the hit location clamped into the block (`BlockPos.clampLocationWithin`).
    fn channel_rod_hit(&mut self, from: Vec3, velocity: Vec3, guest: Option<u32>) {
        let to = Vec3::new(
            from.x + velocity.x,
            from.y + velocity.y,
            from.z + velocity.z,
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        // ClipContext.Block.COLLIDER with Fluid.NONE.
        let hit = traverse_blocks(from, to, 1024, |p: BlockPos| {
            let state = self.world.block(p.x, p.y, p.z);
            if self.res.facts().is_air(state) {
                return None;
            }
            let shape = self
                .collision
                .collision_shape(&mut scene, state, [p.x, p.y, p.z])
                .ok()?;
            shape.clip(from, to, p)
        });
        let Some(hit) = hit.ok().flatten() else {
            return;
        };
        let state = self.world.block(hit.block.x, hit.block.y, hit.block.z);
        if !self.res.facts().get(state).name.ends_with("lightning_rod") {
            return;
        }
        let edge = f64::from(1.0e-5f32);
        let clamp =
            |v: f64, origin: i32| v.clamp(f64::from(origin) + edge, f64::from(origin) + 1.0 - edge);
        self.channel_lightning(
            [
                clamp(hit.location.x, hit.block.x),
                clamp(hit.location.y, hit.block.y),
                clamp(hit.location.z, hit.block.z),
            ],
            guest,
        );
    }
    /// `TridentItem.releaseUsing`.
    pub(super) fn release_trident(&mut self, ticks: u32) {
        if ticks < 10 {
            return;
        }
        let Some(stack) = self.held().cloned() else {
            return;
        };
        let Some(eye) = self.eye_position() else {
            return;
        };
        let riptide = stack.enchantment_level("minecraft:riptide");
        // `releaseUsing` checks the wet/passenger conditions, then nextDamageWillBreak again.
        if riptide > 0 && (!self.in_water_or_rain() || self.riding.is_some()) {
            return;
        }
        if stack.damage >= self.trident_max_damage(&stack).saturating_sub(1) {
            return;
        }
        let at = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        // Counted (on the server) before the throw or the spin.
        if self.remote.is_none() {
            self.award_stat("used", "minecraft:trident", 1);
        }
        if riptide > 0 {
            // `hurtWithoutBreaking` and the sound belong to the server; a server's
            // client only launches.
            let server_client = self.remote.is_some();
            if !server_client {
                self.hurt_held_without_breaking(1);
            }
            // trident_spin_attack_strength: 1.5 + 0.75 per level above the first.
            let spin_stack = self.held().cloned();
            let strength = 1.5 + 0.75 * (riptide as f32 - 1.0);
            // TridentItem.releaseUsing: the look in floats from Mth's sine table, scaled to
            // the strength by its float length (Mth.sqrt), then Entity.push.
            use sunrise_core::geometry::trig;
            let (y, x) = (
                f64::from(self.yaw * trig::DEG_TO_RAD),
                f64::from(self.pitch * trig::DEG_TO_RAD),
            );
            let xd = -trig::sin(y) * trig::cos(x);
            let yd = -trig::sin(x);
            let zd = trig::cos(y) * trig::cos(x);
            let scale = strength / f64::from(xd * xd + yd * yd + zd * zd).sqrt() as f32;
            if let Some(p) = self.player.as_mut() {
                p.velocity = Vec3::new(
                    p.velocity.x + f64::from(xd * scale),
                    p.velocity.y + f64::from(yd * scale),
                    p.velocity.z + f64::from(zd * scale),
                );
                // Player.startAutoSpinAttack(20, 8, stack).
                p.auto_spin_attack_ticks = 20;
                p.auto_spin_attack_dmg = 8.0;
                p.auto_spin_attack_item = spin_stack;
            }
            // `player.move(SELF, (0, 1.1999999, 0))` when on the ground: the player's own move
            // (LocalPlayer.move, through the move gate, #2414), which stops at a ceiling and whose
            // end writes the ground state, so the rest of the tick travels in the air.
            if self.player.as_ref().is_some_and(|p| p.ground.on_ground) {
                let _ = self.move_local_player(
                    sunrise_world::entity_move::MoverType::SelfMovement,
                    Vec3::new(0.0, f64::from(1.1999999f32), 0.0),
                    true,
                );
            }
            if !server_client {
                let sound = format!("minecraft:item.trident.riptide_{}", riptide.min(3));
                // level.playSound(null, player, sound, PLAYERS, 1, 1).
                match (self.lan_acting_entity, at) {
                    (Some(player), Some(at)) => {
                        let sound =
                            LevelSound::new(&sound, at, 1.0, 1.0).in_source(SoundSource::Players);
                        self.play_bound_sound(sound, player, None);
                    }
                    _ => self.audio.play(&sound, at, 1.0, 1.0),
                }
                if let (Some(player), Some(name)) =
                    (self.lan_acting_uuid, self.lan_acting_name.clone())
                {
                    self.native_player_riptide(player, &name);
                }
            }
            return;
        }
        // The server throws the trident.
        if self.remote.is_some() {
            return;
        }
        // hurtWithoutBreaking (Unbreaking applies), then the thrown copy.
        self.hurt_held_without_breaking(1);
        let thrown = self.held().cloned().unwrap_or(stack);
        if !self.creative {
            *self.held_mut() = None;
        }
        let start = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let mut projectile = sunrise_client::projectiles::Projectile::new(
            sunrise_client::projectiles::ProjectileKind::Snowball,
            start,
            Some(Who::Player),
        );
        let (v, on_ground) = self.player.as_ref().map_or((Vec3::ZERO, true), |p| {
            (self.known_player_movement(), p.ground.on_ground)
        });
        projectile.shoot_from_rotation(self.pitch, self.yaw, 0.0, 2.5, 1.0, v, on_ground);
        let id = self.take_entity_id();
        if let Some(uuid) = projectile.uuid {
            self.persist.uuids.insert(id, uuid);
        }
        let owner_uuid = self.acting_player_uuid();
        self.tridents.push(ThrownTrident {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            loyalty: thrown.enchantment_level("minecraft:loyalty"),
            stack: thrown,
            position: start,
            previous: start,
            velocity: projectile.velocity,
            needs_sync: false,
            yaw: projectile.yaw,
            pitch: projectile.pitch,
            in_ground: false,
            in_ground_time: 0,
            life: 0,
            shake: 0,
            dealt_damage: false,
            no_physics: false,
            returning: 0,
            pickup: if self.creative {
                Pickup::CreativeOnly
            } else {
                Pickup::Allowed
            },
            left_owner: false,
            has_been_shot: false,
            owner_uuid: Some(owner_uuid),
            no_gravity: false,
            ground: Default::default(),
            fire_ticks: 0,
            fall_distance: 0.0,
            sound_event: sunrise_client::entity_nbt::TRIDENT_HIT_GROUND.to_owned(),
            was_touching_water: false,
            age: 0,
            owner: 0,
            guest: self.lan_acting,
            random: sunrise_client::projectiles::unique_random(start),
            deflected_by: None,
            last_state: None,
        });
        self.player_projectile_shot();
        // level.playSound(null, trident, sound, PLAYERS, 1, 1).
        let throw = LevelSound::new(
            "minecraft:item.trident.throw",
            [start.x, start.y, start.z],
            1.0,
            1.0,
        )
        .in_source(SoundSource::Players);
        self.play_bound_sound(throw, id, None);
    }
    /// `settle_dealt_absorbed` for the player who threw a trident: a LAN guest's own
    /// statistics are only reachable inside `as_guest`, so the host is not credited.
    fn thrower_dealt_absorbed(&mut self, guest: Option<u32>) {
        let Some(conn) = guest.filter(|c| self.lan_acting != Some(*c)) else {
            self.settle_dealt_absorbed(true);
            return;
        };
        let entered = self.as_guest(conn, |g| g.settle_dealt_absorbed(true));
        if entered.is_none() {
            self.settle_dealt_absorbed(false);
        }
    }
    /// The thrower's `PLAYER_HURT_ENTITY` for a trident hit: run as the
    /// guest that threw it, and not at all once they have left.
    fn thrower_hit(
        &mut self,
        guest: Option<u32>,
        victim: &sunrise_client::advancements::EntityView,
        direct: sunrise_client::advancements::EntityView,
        damage: f32,
    ) {
        let Some(conn) = guest.filter(|c| self.lan_acting != Some(*c)) else {
            self.player_hit(victim, "minecraft:trident", Some(direct), damage);
            return;
        };
        self.as_guest(conn, |g| {
            g.player_hit(victim, "minecraft:trident", Some(direct), damage)
        });
    }
    /// The connection of the guest whose entity id is `id`.
    fn trident_guest_conn(&self, id: u32) -> Option<u32> {
        self.lan
            .as_ref()?
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id as u32 == id)
            .map(|g| g.conn)
    }
    /// `ThrownTrident.onHitEntity` on the guest `conn`: the hurt runs as them and
    /// `ServerPlayer.hurtServer` refuses it when the `pvp` game rule or their
    /// teams forbid it (`canHarmPlayer`); the trident bounces off either way.
    fn trident_hits_guest(
        &mut self,
        t: &ThrownTrident,
        conn: u32,
        damage: f32,
        attack: sunrise_client::health::Attack,
    ) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let team_of = |game: &Self, name: &str| game.score.board.players_team(name).cloned();
        let attacker = match t.guest {
            Some(c) => host.hub.guest(c).and_then(|g| team_of(self, &g.name)),
            None => self.player_team().cloned(),
        };
        let target = host.hub.guest(conn).and_then(|g| team_of(self, &g.name));
        let refused = !self.player_may_harm(target.as_ref(), attacker.as_ref());
        let shooter = self.player_attacker(t.guest);
        let from = [t.previous.x, t.previous.y, t.previous.z];
        self.guest_hit_by(conn, |g| {
            g.with_attack(attack, |g| {
                g.combat.refuse_player_hit = refused;
                g.combat.projectile_attacker = shooter;
                g.mob_hits_player(damage, from, "minecraft:trident");
                g.combat.refuse_player_hit = false;
                g.combat.projectile_attacker = None;
            })
        });
    }
    /// The players tridents fly back to or land at: this game's player and the
    /// guests in this level. A server has no player of its own, so its
    /// tridents belong to guests.
    fn trident_catchers(&self) -> Vec<Catcher> {
        let mut out = Vec::new();
        if let Some(eye) = self.eye_position() {
            let feet = self.player.as_ref().map_or(Vec3::ZERO, |p| p.position);
            out.push(Catcher {
                guest: None,
                id: 0,
                feet,
                eye: Vec3::new(eye[0], eye[1], eye[2]),
                touches: true,
                creative: self.creative,
                uuid: self.level.player_uuid,
            });
        }
        if let Some(host) = self.lan.as_ref() {
            for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
                let [x, y, z] = g.position;
                out.push(Catcher {
                    guest: Some(g.conn),
                    id: g.entity_id,
                    feet: Vec3::new(x, y, z),
                    eye: Vec3::new(x, y + g.eye_height(), z),
                    touches: !g.dead && !g.spectator(),
                    creative: g.creative(),
                    uuid: Some(sunrise_client::lan::playerdata::uuid_ints(g.uuid)),
                });
            }
        }
        out
    }
    /// `ThrownTrident.tryPickup` for a trident that has landed or is
    /// returning: the player takes the item (a creative thrower's trident is
    /// only removed) and the rest that does not fit drops at their feet.
    /// False when it could not be given.
    fn trident_taken(&mut self, t: &ThrownTrident, by: &Catcher) -> bool {
        let creative_pickup = t.pickup == Pickup::CreativeOnly && by.creative;
        let returning_to_owner = t.no_physics && t.owner_uuid.is_some() && t.owner_uuid == by.uuid;
        if !creative_pickup {
            if t.pickup != Pickup::Allowed && !returning_to_owner {
                return false;
            }
            let rest = match by.guest {
                None => self.add_stack_to_inventory(t.stack.clone()),
                Some(conn) => {
                    let rest = self.as_guest(conn, |g| g.add_stack_to_inventory(t.stack.clone()));
                    let Some(rest) = rest else {
                        return false;
                    };
                    rest
                }
            };
            if let Some(rest) = rest {
                self.spawn_item(rest, [by.feet.x, by.feet.y, by.feet.z], [0.0; 3], 0);
            }
        }
        // Player.take, for a creative thrower's trident too: the clients near
        // play the pickup sound and fly the trident to its collector.
        if by.guest.is_some()
            && let Some(host) = self.lan.as_ref()
        {
            let at = [t.position.x, t.position.y, t.position.z];
            host.hub.take_item(t.id as i32, by.id, 1, at);
        }
        if by.guest.is_none() {
            let pitch = ((self.world.random.next_float() - self.world.random.next_float()) * 0.7
                + 1.0)
                * 2.0;
            self.audio.play(
                "minecraft:entity.item.pickup",
                Some([by.feet.x, by.feet.y, by.feet.z]),
                0.2,
                pitch,
            );
        }
        true
    }
    /// Apply the environment before impact callbacks can change the world.
    fn trident_block_effects(&mut self, t: &mut ThrownTrident, from: Vec3, to: Vec3) {
        let extinguished = {
            let rain = self.arrow_rain();
            // Block-contact hooks are already delivered by the game's inside pass.
            let (ticks, extinguished) =
                self.arrow_scene(&rain)
                    .apply_block_effects(from, to, t.fire_ticks, |_, _, _| {});
            t.fire_ticks = ticks;
            extinguished
        };
        if extinguished {
            let pitch = 1.6 + (t.random.next_float() - t.random.next_float()) * 0.4;
            self.audio.play(
                "minecraft:entity.generic.extinguish_fire",
                Some([to.x, to.y, to.z]),
                0.7,
                pitch,
            );
        }
    }
    /// Notify at the contact with the live trident in its owning slot. Return the live row
    /// and whether to skip the hit (veto or invalidated contact). Only removal ends the tick.
    fn trident_impact(
        &mut self,
        index: &mut usize,
        t: ThrownTrident,
        hit: Hit,
        through: &mut Option<JoinSeq>,
        transaction: &mut Option<TransactionSeal>,
    ) -> Option<(ThrownTrident, bool)> {
        if !self.native_hears_impacts() {
            return Some((t, false));
        }
        through.get_or_insert_with(JoinSeq::latest);
        let (id, join, guest) = (t.id, t.join, t.guest);
        let motion = |t: &ThrownTrident| {
            (
                t.position,
                t.velocity,
                t.in_ground,
                t.dealt_damage,
                t.no_physics,
                t.owner,
                t.owner_uuid,
                t.guest,
            )
        };
        let before = motion(&t);
        let target = self.native_impact_target(hit);
        self.tridents.put_back(*index, t);
        let mut scope = ImpactTransaction::new(self, true);
        let game = &mut *scope.game;
        let canceled = game.native_trident_impact(id, guest, hit);
        if game.entity(id).is_some()
            && let Some(at) = game.tridents.locate(join)
        {
            let refused = canceled
                || motion(&game.tridents[at]) != before
                || target.is_none()
                || target != game.native_impact_target(hit);
            if let Some(live) = game.tridents.extract(join) {
                *index = at;
                *transaction = scope.seal.take();
                return Some((live, refused));
            }
        }
        scope.close(true);
        if let Some(at) = self.tridents.locate(join) {
            *index = at + 1;
        }
        None
    }

    pub(super) fn tick_tridents(&mut self) {
        if self.tridents.is_empty() {
            return;
        }
        let catchers = self.trident_catchers();
        let ticking = self.entity_ticking_area();
        let mut targets = self.projectile_targets();
        targets.extend(self.lan_projectile_targets());
        let mut i = 0;
        let mut through = None;
        while i < self.tridents.len() {
            let t = &self.tridents[i];
            let target = Target::Trident(t.id);
            if through.is_some_and(|last| t.join > last)
                || self.removal_pending(&target)
                || self.persist.removals.contains(&target)
            {
                i += 1;
                continue;
            }
            let mut transaction = None;
            let at = self.tridents[i].position;
            if !ticking(at.x.floor() as i32, at.z.floor() as i32) {
                self.tridents[i].previous = at;
                i += 1;
                continue;
            }
            let mut t = self.tridents.remove(i);
            t.previous = t.position;
            t.age += 1;
            // AbstractArrow.tick: `if (shakeTime > 0) --shakeTime;`.
            t.shake -= i32::from(t.shake > 0);
            if t.in_ground_time > 4 {
                t.dealt_damage = true;
            }
            // ThrownTrident.tick: Loyalty needs its owner, who is in this level.
            let owner = catchers
                .iter()
                .find(|c| t.owner_uuid.is_some() && c.uuid == t.owner_uuid);
            let owner_eye = owner.map(|c| c.eye);
            let owner_who = owner
                .map(|c| c.guest.map_or(Who::Player, |_| Who::Mob(c.id as u32)))
                .or_else(|| t.owner_uuid.and_then(|uuid| self.who_by_uuid(uuid)));
            if let Some(owner) = owner {
                t.guest = owner.guest;
            }
            // `isAcceptibleReturnOwner` fails for a dead or spectating owner: the
            // trident drops as an item when it can be picked up, then goes.
            if t.loyalty > 0
                && (t.dealt_damage || t.no_physics)
                && owner.is_some_and(|c| !c.touches)
            {
                if t.pickup == Pickup::Allowed {
                    self.spawn_at_location(t.position, f64::from(0.1f32), t.stack.clone());
                }
                continue;
            }
            if t.loyalty > 0
                && (t.dealt_damage || t.no_physics)
                && let Some(owner_eye) = owner_eye
            {
                t.no_physics = true;
                t.in_ground = false;
                let from = t.position;
                t.loyalty_pull(owner_eye);
                // The pull is a position update of its own, before the trident's move.
                let to = t.position;
                self.entity_section_step(t.id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
                if t.returning == 0 {
                    self.audio.play(
                        "minecraft:item.trident.return",
                        Some([t.position.x, t.position.y, t.position.z]),
                        10.0,
                        1.0,
                    );
                }
                t.returning += 1;
            }
            // AbstractArrow clears cached water/rain before its stuck branch.
            let in_rain = {
                let rain = self.arrow_rain();
                self.arrow_scene(&rain).in_rain(t.position)
            };
            if t.was_touching_water || in_rain {
                t.fire_ticks = t.fire_ticks.min(0);
            }
            let stuck_tick = t.in_ground && !t.no_physics;
            // AbstractArrow.tick checks this before moving, and only its
            // flight branch reaches Projectile.tick (a stuck arrow does not).
            if !stuck_tick && !t.left_owner {
                let (_, _, owner_boxes) = self.thrown_ridden(owner_who, &targets);
                let swept = Aabb::new(
                    t.position.add_vector(Vec3::new(-HALF, 0.0, -HALF)),
                    t.position.add_vector(Vec3::new(HALF, 0.5, HALF)),
                )
                .expand_towards(t.velocity)
                .inflate(Vec3::new(1.0, 1.0, 1.0));
                t.left_owner = owner_boxes.iter().all(|bounds| !swept.intersects(*bounds));
            }
            if t.in_ground {
                // AbstractArrow.tick: `if (lastState != blockState && shouldFall())
                // startFalling();` (`blockState` is the block at its position).
                // Its `else tickDespawn()` counts `life` and discards at 1200
                // (`ThrownTrident.tickDespawn`: not for a Loyalty trident that
                // can be picked up).
                let mut falls = false;
                if !t.no_physics {
                    let rain = self.arrow_rain();
                    let scene = self.arrow_scene(&rain);
                    let state = scene.block(BlockPos::containing(t.position));
                    falls = t.last_state != Some(state) && t.should_fall(&scene);
                }
                if falls {
                    t.start_falling();
                } else if t.tick_despawn() {
                    // `discard()`: gone for everything after, so no block
                    // effects (`isAlive`) and no pickup this tick.
                    continue;
                }
                t.in_ground_time += 1;
                if !t.no_physics {
                    let at = t.position;
                    self.trident_block_effects(&mut t, at, at);
                }
            } else if t.no_physics {
                t.turn_towards_movement();
                t.position = Vec3::new(
                    t.position.x + t.velocity.x,
                    t.position.y + t.velocity.y,
                    t.position.z + t.velocity.z,
                );
            } else {
                t.turn_towards_movement();
                // Mobs on the path, then blocks.
                let mut hit_mob = None;
                // ProjectileUtil.computeMargin: the boxes it meets grow from
                // nothing to 0.3 over its first ticks.
                let margin = f64::from(((t.age as i32 - 2) as f32 / 20.0).clamp(0.0, 0.3));
                let grow = Vec3::new(margin, margin, margin);
                if !t.dealt_damage {
                    for m in self.mobs.iter().filter(|m| m.alive()) {
                        if let Some(d) =
                            segment_box(t.position, t.velocity, m.bounds().inflate(grow))
                            && hit_mob.is_none_or(|(_, best)| d < best)
                        {
                            hit_mob = Some((m.id, d));
                        }
                    }
                }
                // The other players of a served world are targets too (their own
                // trident passes its thrower).
                if !t.dealt_damage {
                    for (who, b) in self.lan_projectile_targets() {
                        if let Who::Mob(id) = who
                            && self
                                .trident_guest_conn(id)
                                .is_some_and(|c| Some(c) != t.guest)
                            && let Some(d) = segment_box(t.position, t.velocity, b.inflate(grow))
                            && hit_mob.is_none_or(|(_, best)| d < best)
                        {
                            hit_mob = Some((id, d));
                        }
                    }
                }
                // Boats, minecarts and primed TNT are pickable too (`isPickable`).
                if !t.dealt_damage {
                    for (who, b) in self
                        .vehicle_targets()
                        .into_iter()
                        .chain(self.tnts.iter().map(|t| (Who::Mob(t.id), t.bounds())))
                    {
                        if let Who::Mob(id) = who
                            && let Some(d) = segment_box(t.position, t.velocity, b.inflate(grow))
                            && hit_mob.is_none_or(|(_, best)| d < best)
                        {
                            hit_mob = Some((id, d));
                        }
                    }
                }
                // Preserve the captured contact through ProjectileImpactEvent. Refusing it
                // suppresses damage and deflection; removing the trident ends its tick.
                let refused_mob = if let Some((id, d)) = hit_mob {
                    let from = t.position;
                    let at = from.add_vector(t.velocity.scale(d));
                    t.position = at;
                    self.trident_block_effects(&mut t, from, at);
                    let hit = Hit::Entity {
                        who: Who::Mob(id),
                        at,
                    };
                    let Some((live, refused)) =
                        self.trident_impact(&mut i, t, hit, &mut through, &mut transaction)
                    else {
                        continue;
                    };
                    t = live;
                    refused
                } else {
                    false
                };
                if refused_mob {
                    // No damage, sound or deflection: it flies on from the contact.
                } else if let Some((id, _)) = hit_mob {
                    // `Breeze.deflection`: a trident is not a wind charge, so
                    // `PROJECTILE_DEFLECTION` turns it around. `onHit` does not
                    // run, so the breeze takes no damage and Channeling never fires.
                    let breeze = self
                        .mobs
                        .iter()
                        .find(|m| m.id == id && m.alive() && m.species() == Species::Breeze);
                    if let Some(breeze) = breeze {
                        if t.deflected_by != Some(id) {
                            let sound_at =
                                [breeze.position.x, breeze.position.y, breeze.position.z];
                            self.audio.play(
                                "minecraft:entity.breeze.deflect",
                                Some(sound_at),
                                1.0,
                                1.0,
                            );
                            let jitter = t.random.next_float();
                            let (velocity, yaw) = breeze_deflect(t.velocity, t.yaw, jitter);
                            t.velocity = velocity;
                            t.yaw = yaw;
                            t.deflected_by = Some(id);
                        }
                    } else {
                        // ThrownTrident.getDamage: `EnchantmentHelper.modifyDamage` on 8
                        // (impaling hits `#sensitive_to_impaling`, the aquatic tag).
                        let victim = self
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.kind.id.clone())
                            .or_else(|| {
                                self.trident_guest_conn(id)
                                    .map(|_| "minecraft:player".to_owned())
                            })
                            .or_else(|| {
                                self.boats
                                    .iter()
                                    .find(|b| b.id == id)
                                    .map(|b| b.entity_type())
                            })
                            .or_else(|| {
                                self.carts
                                    .iter()
                                    .find(|c| c.id == id)
                                    .map(|c| c.kind.id().to_owned())
                            });
                        let ctx = EffectContext {
                            this: victim.as_deref(),
                            attacker: Some("minecraft:player"),
                            direct_attacker: Some("minecraft:trident"),
                            is_direct: true,
                            damage_type: Some("minecraft:trident"),
                            items: None,
                            ..Default::default()
                        };
                        let attacker = self
                            .player_attacker(t.guest)
                            .map_or(self.own_entity_id() as u32, |s| s.0);
                        let damage =
                            self.evaluate_weapon_damage(&t.stack, Some((attacker, id)), ctx, 8.0);
                        t.dealt_damage = true;
                        let mut hurt_mob = None;
                        let aim = self.attacker_aim(Some(Who::Player));
                        let mut attack = self.attack_of(attacker as i32);
                        attack.projectile = Some(t.id as i32);
                        attack.projectile_kind = sunrise_client::health::DirectEntity::Arrow;
                        Self::set_projectile_weapon(&mut attack, Some(&t.stack));
                        let trident_source = DamageKind::Named(
                            sunrise_client::mobs::NamedDamage::registered(
                                &self.health_resources.damage,
                                "minecraft:trident",
                            )
                            .expect("registered vanilla trident damage"),
                        );
                        if let Some(i) = self.mob_index.find(&self.mobs, id) {
                            let plan = self.prepare_knockback_credit(
                                id as i32,
                                Some(Who::Player),
                                &attack,
                            );
                            let mut credit = plan.lend(&mut self.last_hurt_by_player);
                            let from = Some([t.position.x, t.position.y, t.position.z]);
                            let join = self.mobs[i].join;
                            let hurt = self.mobs[i].damage_projectile(
                                crate::game::entity::damage_level_of!(
                                    self,
                                    &attack,
                                    &mut credit,
                                    None
                                ),
                                damage,
                                Damage {
                                    kind: trident_source,
                                    attacker: Some(Who::Player),
                                    from,
                                    attacker_species: None,
                                },
                                aim,
                                [t.velocity.x, t.velocity.z],
                            );
                            self.finish_mob_damage_at(i);
                            if hurt {
                                hurt_mob = Some(join);
                            }
                        }
                        self.thrower_dealt_absorbed(t.guest);
                        // ThrownTrident.onHitEntity on a boat or minecart:
                        // VehicleEntity.hurtServer takes the damage; an item
                        // frame's ItemFrame.hurtServer.
                        let creative = t.guest.is_none() && self.creative;
                        let causing = match t.guest {
                            Some(conn) => self.command_guest(conn).map_or(0, |g| g.entity_id),
                            None => self.own_entity_id(),
                        };
                        let source = sunrise_client::health::HurtSource {
                            damage_type: "minecraft:trident",
                            causing,
                            direct: t.id as i32,
                            position: Some([t.previous.x, t.previous.y, t.previous.z]),
                        };
                        let source_creative = match t.guest {
                            Some(conn) => self.command_guest(conn).is_some_and(|g| g.creative()),
                            None => self.creative,
                        };
                        let vanilla =
                            self.saved_refuses_damage(id, "minecraft:trident", source_creative);
                        if let Some(i) = self.boats.iter().position(|b| b.id == id) {
                            let kind = self.boats[i].entity_type();
                            if !crate::game::entity::damage_level_of!(
                                self,
                                self.combat.attack(),
                                no_push,
                                None
                            )
                            .invulnerability_check(
                                sunrise_client::health::Hurting::Entity(id as i32, &kind, source),
                                vanilla,
                            ) {
                                self.hurt_boat(i, damage, creative);
                            }
                        } else if let Some(i) = self.carts.iter().position(|c| c.id == id) {
                            let kind = self.carts[i].kind.id();
                            if !crate::game::entity::damage_level_of!(
                                self,
                                self.combat.attack(),
                                no_push,
                                None
                            )
                            .invulnerability_check(
                                sunrise_client::health::Hurting::Entity(id as i32, kind, source),
                                vanilla,
                            ) {
                                // `damageSources().trident(this, owner)`: the trident is the
                                // direct entity, so no piglin anger.
                                let by = t.guest.map_or(Target::Player, Target::Guest);
                                let hit = CartHit::attributed(Some(by));
                                self.hurt_cart(i, damage, creative, false, hit);
                            }
                        } else {
                            // ItemFrame.hurtServer: the item comes out, or the frame breaks.
                            self.projectile_hurts_frame(id, true, creative);
                        }
                        // ThrownTrident.onHitEntity on another player.
                        if let Some(conn) = self.trident_guest_conn(id) {
                            self.trident_hits_guest(&t, conn, damage, attack);
                        }
                        // `minecraft:post_attack` of Channeling, on the victim.
                        if let Some(join) = hurt_mob
                            && t.stack.enchantment_level("minecraft:channeling") > 0
                            && let Some(m) = self
                                .mobs
                                .iter()
                                .find(|m| m.id == id && m.join == join && !m.removed)
                        {
                            self.channel_lightning(m.position.components(), t.guest);
                        }
                        if let Some(join) = hurt_mob
                            && let Some(m) = self
                                .mobs
                                .iter()
                                .find(|m| m.id == id && m.join == join && !m.removed)
                        {
                            // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                            let victim = self.mob_entity(m);
                            let direct = sunrise_client::advancements::EntityView {
                                kind: "minecraft:trident".into(),
                                position: [t.position.x, t.position.y, t.position.z],
                                ..Default::default()
                            };
                            self.thrower_hit(t.guest, &victim, direct, damage);
                        }
                        // Projectile.onHit: the landing is heard, at the hit.
                        self.projectile_landed_event(
                            t.id,
                            Some(Who::Player),
                            [t.position.x, t.position.y, t.position.z],
                        );
                        self.audio.play(
                            "minecraft:item.trident.hit",
                            Some([t.position.x, t.position.y, t.position.z]),
                            1.0,
                            1.0,
                        );
                        // ProjectileDeflection.REVERSE (half, reversed) scaled by
                        // (0.02, 0.2, 0.02).
                        t.velocity = Vec3::new(
                            t.velocity.x * (-0.02 * 0.5),
                            t.velocity.y * (-0.2 * 0.5),
                            t.velocity.z * (-0.02 * 0.5),
                        );
                    }
                } else {
                    // AbstractArrow.tick: a zero-width ray clip of the movement
                    // (`ClipContext.Block.COLLIDER`, no fluids), then
                    // `AbstractArrow.onHitBlock` on a hit.
                    let from = t.position;
                    let flight = t.velocity;
                    let clip =
                        clip_block_hit(&self.world, self.res.facts(), self.collision, from, flight);
                    // AbstractArrow.stepMoveAndHit: move to the contact and apply the
                    // flight's block effects before ProjectileImpactEvent or onHitBlock.
                    let contact = clip.map_or(from.add_vector(flight), |(at, _, _)| at);
                    t.position = contact;
                    self.trident_block_effects(&mut t, from, contact);
                    // Only onHitBlock backs away from the clipped point.
                    let refused_block = if let Some((at, block, face)) = clip {
                        let hit = Hit::Block {
                            pos: [block.x, block.y, block.z],
                            face,
                            at,
                        };
                        let Some((live, refused)) =
                            self.trident_impact(&mut i, t, hit, &mut through, &mut transaction)
                        else {
                            continue;
                        };
                        t = live;
                        refused
                    } else {
                        false
                    };
                    if !refused_block && let Some((at, block, _)) = clip {
                        let stop = back_off(at, flight);
                        // Projectile.onHit on a block: PROJECTILE_LAND at the
                        // block's centre.
                        self.projectile_landed_event(
                            t.id,
                            Some(Who::Player),
                            [
                                f64::from(block.x) + 0.5,
                                f64::from(block.y) + 0.5,
                                f64::from(block.z) + 0.5,
                            ],
                        );
                        if t.stack.enchantment_level("minecraft:channeling") > 0 {
                            self.channel_rod_hit(from, flight, t.guest);
                        }
                        // 0.05 back along signum(movement), stopped, stuck, shaking.
                        t.position = stop;
                        t.velocity = Vec3::ZERO;
                        t.in_ground = true;
                        t.last_state = Some(self.world.block(block.x, block.y, block.z));
                        t.shake = 7;
                        let sound = t.hit_ground_sound();
                        self.audio
                            .play(&sound, Some([stop.x, stop.y, stop.z]), 1.0, 1.0);
                    }
                }
                if !t.in_ground {
                    // A trident's water inertia (0.99) equals the air drag, a float in
                    // vanilla (`AbstractArrow.tick`); gravity 0.05.
                    let drag = f64::from(0.99f32);
                    let gravity = if t.no_gravity { 0.0 } else { 0.05 };
                    t.velocity = Vec3::new(
                        t.velocity.x * drag,
                        t.velocity.y * drag - gravity,
                        t.velocity.z * drag,
                    );
                }
            }
            if !t.no_physics {
                let half = 0.25;
                let bounds = Aabb::new(
                    t.position.add_vector(Vec3::new(-half, 0.0, -half)),
                    t.position.add_vector(Vec3::new(half, 0.5, half)),
                );
                self.entity_powder_snow_particles(
                    t.previous, t.position, bounds, t.velocity, false,
                );
            }
            // The stuck branch returns before Entity.baseTick. A flight that
            // just hit a block still reaches it, as does a noPhysics return.
            if !stuck_tick {
                t.has_been_shot = true;
                let rain = self.arrow_rain();
                t.was_touching_water = self.arrow_scene(&rain).in_water(t.position);
                // Entity.updateInWaterStateAndDoFluidPushing: resetFallDistance in water.
                if t.was_touching_water {
                    t.fall_distance = 0.0;
                }
                if t.fire_ticks > 0 {
                    t.fire_ticks -= 1;
                }
            }
            // playerTouch: stuck or returning tridents go back to a player.
            // ThrownTrident.playerTouch: only its owner takes it, or anyone
            // once the owner is not in this level (`getOwner() == null`), and
            // not while `AbstractArrow.shakeTime` still runs.
            if (t.in_ground || t.no_physics) && t.shake <= 0 {
                let near = |c: &&Catcher| {
                    c.touches
                        && (owner_who.is_none() || c.uuid == t.owner_uuid)
                        && (t.position.x - c.feet.x).abs() < 1.0
                        && (t.position.z - c.feet.z).abs() < 1.0
                        && t.position.y > c.feet.y - 1.0
                        && t.position.y < c.feet.y + 2.5
                };
                if let Some(by) = catchers.iter().find(near)
                    && self.trident_taken(&t, by)
                {
                    if let Some(seal) = transaction {
                        self.tridents.retire_extracted(t.id, t.join);
                        self.transaction_close(&seal, true);
                    }
                    continue;
                }
            }
            let (id, from, to) = (t.id, t.previous, t.position);
            if transaction.is_some() {
                self.tridents.put_back(i, t);
            } else {
                self.tridents.insert(i, t);
            }
            self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
            if let Some(seal) = transaction {
                self.transaction_close(&seal, true);
            }
            i += 1;
        }
    }
    /// ThrownTridentRenderer.
    pub(super) fn trident_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        run: std::ops::Range<usize>,
    ) {
        // AbstractArrow.shouldRenderAtSqrDistance: ten times the default.
        let range = 10.0
            * super::entity_culling::default_range(
                glam::DVec3::splat(2.0 * HALF),
                self.view_scale(),
            );
        let live = self.tridents[run]
            .iter()
            .filter(|t| {
                // EntityRenderer.shouldRender; ThrownTridentRenderer grows the box by 1.5.
                self.standing_entity_in_view(
                    camera,
                    super::entity_culling::at(t.previous),
                    super::entity_culling::at(t.position),
                    alpha,
                    2.0 * HALF,
                    2.0 * HALF,
                    range,
                    1.5,
                )
            })
            .map(|t| {
                let at = [
                    t.previous.x + (t.position.x - t.previous.x) * alpha,
                    t.previous.y + (t.position.y - t.previous.y) * alpha,
                    t.previous.z + (t.position.z - t.previous.z) * alpha,
                ];
                (t, at)
            });
        self.draw_tridents(camera, live, out);
    }

    pub(super) fn pickup_trident_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        // ItemPickupParticle: a picked-up trident flying to its collector,
        // drawn with the render state extracted at partial tick 1.
        let taken = self.pickups.live.iter().filter_map(|p| match &p.taken {
            super::pickups::Taken::Trident(t) => Some((&**t, super::pickups::position(p, alpha))),
            _ => None,
        });
        self.draw_tridents(camera, taken, out);
    }

    fn draw_tridents<'a>(
        &self,
        camera: glam::DVec3,
        tridents: impl Iterator<Item = (&'a ThrownTrident, [f64; 3])>,
        out: &mut Vec<Vertex>,
    ) {
        let Some(model) = &self.trident_model else {
            return;
        };
        let Some(uv) = self
            .gui
            .atlas
            .sprites
            .get("entity/trident/trident")
            .map(|r| [r.u0, r.v0, r.u1, r.v1])
        else {
            return;
        };
        // The texture's OptiFine emissive overlay, drawn again full bright.
        let glow = self
            .res
            .mob_data()
            .emissive
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .overlay("entity/trident/trident")
            .and_then(|e| self.gui.atlas.sprites.get(e))
            .map(|r| [r.u0, r.v0, r.u1, r.v1]);
        for (t, at) in tridents {
            let p = glam::DVec3::from_array(at) - camera;
            let base = glam::Mat4::from_translation(p.as_vec3())
                * glam::Mat4::from_rotation_y((t.yaw - 90.0).to_radians())
                * glam::Mat4::from_rotation_z((t.pitch + 90.0).to_radians());
            let l = self.world.light(
                t.position.x.floor() as i32,
                t.position.y.floor() as i32,
                t.position.z.floor() as i32,
            );
            if t.fire_ticks > 0 {
                // The flames face the camera, turned round in the front view.
                let fire_yaw =
                    if self.camera_type == sunrise_client::camera::CameraType::ThirdPersonFront {
                        self.yaw + 180.0
                    } else {
                        self.yaw
                    };
                sunrise_client::fire_render::flames(
                    at,
                    0.5,
                    0.5,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    l,
                    out,
                );
            }
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            let poses = Poses::new(model);
            sunrise_client::mobs::render::draw(
                model,
                base,
                &poses,
                uv,
                0xFFFF_FFFF,
                packed,
                0,
                out,
            );
            if let Some(glow) = glow {
                let full = sunrise_client::mobs::render::FULL_BRIGHT;
                sunrise_client::mobs::render::draw(
                    model,
                    base,
                    &poses,
                    glow,
                    0xFFFF_FFFF,
                    full,
                    0,
                    out,
                );
            }
        }
        self.mark_motion(out.len(), 0);
    }
}
#[cfg(test)]
#[path = "trident/impact_tests.rs"]
mod impact_tests;
#[cfg(test)]
mod tests {
    use super::{ThrownTrident, remote_tick};
    use sunrise_client::{mobs::Who, projectiles::ProjectileWorld};
    use sunrise_core::geometry::{Aabb, Vec3};

    struct Air;
    impl ProjectileWorld for Air {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &[]
        }
    }

    #[test]
    fn remote_trident_waits_for_a_non_self_move_to_leave_its_block() {
        let mut trident = ThrownTrident::remote(1, Vec3::new(0.5, 100.0, 0.5), 0.0, 0.0);
        trident.in_ground = true;
        trident.velocity = Vec3::new(1.0, -2.0, 3.0);
        remote_tick(&mut trident, &Air, None);
        assert!(trident.in_ground);
        assert_eq!(trident.velocity, Vec3::new(1.0, -2.0, 3.0));
    }

    /// A stack can reach its final durability point after use began. The release
    /// guard is independent of admission, and short charges still do nothing.
    #[test]
    #[ignore = "daily"]
    fn trident_release_rechecks_stack_durability_after_charge() {
        use crate::game::{Audio, Options, PlayerMotion, Progress, Settings, load_resources};
        use sunrise_core::nbt::Tag;
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join("trident-release"),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = super::Game::new(
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
        game.player =
            Some(PlayerMotion::new(game.motion, super::Vec3::new(4.5, 200.0, 4.5)).unwrap());
        let mut stack = super::ItemStack::new("minecraft:trident", 1);
        stack.damage = 8;
        stack
            .data_mut()
            .set_other("minecraft:max_damage", Some(Tag::Int(10)));
        *game.held_mut() = Some(stack.clone());
        assert!(game.use_trident(&stack));
        assert!(game.using.is_some());
        game.held_mut().as_mut().unwrap().damage = 9;
        game.release_trident(10);
        assert!(game.tridents.is_empty());
        assert_eq!(game.held().unwrap().damage, 9);
        game.using = None;

        stack.damage = 249;
        stack
            .data_mut()
            .set_other("minecraft:max_damage", Some(Tag::Int(500)));
        *game.held_mut() = Some(stack.clone());
        assert!(game.use_trident(&stack));
        assert!(game.using.is_some());
        game.release_trident(9);
        assert!(game.tridents.is_empty());
        assert_eq!(game.held().unwrap().damage, 249);
        game.release_trident(10);
        assert_eq!(game.tridents.len(), 1);
        assert_eq!(game.tridents[0].stack.damage, 250);
        assert!(game.held().is_none());
        assert_eq!(game.tridents[0].pickup, super::Pickup::Allowed);
        assert_eq!(game.tridents[0].owner_uuid, game.level.player_uuid);
        assert!(!game.tridents[0].has_been_shot);
        assert!(!game.tridents[0].left_owner);

        game.creative = true;
        game.using = None;
        let stack = super::ItemStack::new("minecraft:trident", 1);
        *game.held_mut() = Some(stack.clone());
        assert!(game.use_trident(&stack));
        game.release_trident(10);
        assert_eq!(game.tridents[1].pickup, super::Pickup::CreativeOnly);
        assert!(game.held().is_some());
    }

    /// `TridentItem.releaseUsing` launches a Riptide III player along the float look vector
    /// scaled to 3.0 by its float length, added to the velocity it had. The expected components
    /// are computed apart from the code, with every step rounded to f32 as the look is.
    #[test]
    #[ignore = "daily"]
    fn riptide_launch_preserves_float_push() {
        use crate::game::{Audio, Options, PlayerMotion, Progress, Settings, load_resources};
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join("trident-riptide"),
        );
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = super::Game::new(
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
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
        );
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        game.world.set(4, 200, 4, water).unwrap();
        let mut player = PlayerMotion::new(game.motion, super::Vec3::new(4.5, 200.0, 4.5)).unwrap();
        player.velocity = super::Vec3::new(0.1, 0.2, 0.3);
        game.player = Some(player);
        game.yaw = 30.0;
        game.pitch = 20.0;
        let mut stack = super::ItemStack::new("minecraft:trident", 1);
        stack
            .data_mut()
            .enchantments
            .push(("minecraft:riptide".into(), 3));
        *game.held_mut() = Some(stack.clone());
        assert!(game.use_trident(&stack));
        game.release_trident(10);
        let velocity = game.player.as_ref().unwrap().velocity;
        assert_eq!(velocity.x, f64::from_bits(0xbff4_f3bb_2666_6666));
        assert_eq!(velocity.y, f64::from_bits(0xbfea_6d1e_9999_999a));
        assert_eq!(velocity.z, f64::from_bits(0x4005_ee9e_c666_6666));
    }

    /// TridentItem.releaseUsing on the ground: the riptide's `move(SELF, (0, 1.1999999, 0))` is the player's own
    /// move, whose end writes the ground state, so the player is in the air for the rest of the tick (and travels
    /// with air friction) as vanilla's is.
    #[test]
    #[ignore = "daily"]
    fn a_riptide_from_the_ground_leaves_the_player_in_the_air() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            use std::sync::Arc;
            use sunrise_client::{chunk::Chunk, motion::PlayerMotion};
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, water).unwrap();
            let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap();
            player.ground.on_ground = true;
            game.player = Some(player);
            let mut stack = super::ItemStack::new("minecraft:trident", 1);
            stack.enchant("minecraft:riptide", 3);
            *game.held_mut() = Some(stack.clone());
            assert!(game.use_trident(&stack));
            game.release_trident(20);
            let p = game.player.as_ref().unwrap();
            assert!(!p.ground.on_ground);
            assert!(
                (p.position.y - (64.0 + f64::from(1.1999999_f32))).abs() < 1e-9,
                "{}",
                p.position.y
            );
        });
    }
}
