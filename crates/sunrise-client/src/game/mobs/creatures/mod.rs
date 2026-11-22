//! Mobs in the game loop: ticking with entity snapshots, natural and
//! chunk-generation spawning (`NaturalSpawner`), mob events (attacks,
//! breeding, conversions, drops), projectiles, player interactions
//! (feeding, shearing, milking, spawn eggs, throwing) and rendering.
#[path = "../../creatures/biome_facts.rs"]
pub(super) mod biome_facts;
#[cfg(test)]
mod borrowed_panic_tests;
mod borrowed_tick;
mod breeding;
mod combat_adaptations;
mod conversion_identity;
mod death_tail;
mod hoglin_target;
mod living_draw;
use living_draw::LivingFrame;
#[path = "../../creatures/mob_conversion.rs"]
mod mob_conversion;
#[cfg(test)]
mod mob_dispatch_panic_tests;
mod player_loot_entity;
mod projectile_targets;
pub(super) use sunrise_gameplay::projectile_world::ProjWorld;
mod pufferfish_player;
#[cfg(test)]
pub(in crate::game) mod render_fixture;
mod residency;
#[cfg(test)]
mod retained_limits_tests;
mod section_steps;
#[cfg(test)]
#[path = "../../creatures/spawn_egg_mob_tests.rs"]
mod spawn_egg_mob_tests;
#[cfg(test)]
#[path = "../../creatures/spectator_view_tests.rs"]
mod spectator_view_tests;
mod world_view;
use super::api_values::{TryIntoApi, TryIntoEngine};
use super::level_sounds::LevelSound;
use super::piston_push::Lid;
use super::{
    Dim, Game, InteractTarget,
    entity_culling::EntityCullDecision,
    native_mob_draw::PlayerRender,
    native_projectiles::{
        Impact, ImpactTransaction, PendingImpact, ProjectileHit, projectile_impact_event,
    },
    view_vector,
};
use mod_api::{EquipmentSlot, LootDrops, SoundSource};
use std::sync::Arc;
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::actions::Action;
use sunrise_client::{
    Result,
    arrows::EntityHit,
    blocks::Fluid,
    crafting::Recipes,
    fast_hash::FxHashMap,
    graphics::EntityCullingMode,
    health::{Attack, HealthEvent, HurtSource, Hurting, Knock},
    items::{ItemStack, Items},
    join_seq::JoinSeq,
    mesher::Vertex,
    mobs::{
        Damage, DamageKind, Mob, MobEvent, MobWorld, PlayerView, Seen, SpawnSetup, Species, Who,
        ai::{eating_sound, eating_sound_is_bound, feed_animal, takes_food},
        entity_sounds::full_id,
        finalize::Group,
        spawning::{self, BiomeFacts},
    },
    particles::{ParticleOptions, ParticleType},
    projectiles::{Hit, Projectile, ProjectileKind},
};
use sunrise_client_render::render_geometry::EntitySpan;
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3, trig::Mth},
    position::BlockPos,
    random::RandomSource,
};

/// `AbstractArrow.onHitEntity` when `hurtOrSimulate` returned false.
/// An enderman that teleported (`projectileReceivesSideEffectsOnHit` is
/// false) leaves the arrow in flight. Every other refusal, including
/// `Shulker.hurtServer` while `isClosed()`, is
/// `deflect(ProjectileDeflection.REVERSE, entity, owner, false, 0.2)`.
fn arrow_miss(dodged: bool) -> EntityHit {
    if dodged {
        EntityHit::Dodged
    } else {
        EntityHit::Refused
    }
}
pub(super) type SpawnReason = sunrise_client::mobs::spawn_rules::Reason;

/// Whether vanilla calls `finalizeSpawn` for a mob a spawn site makes, and with
/// which reason. The reason is vanilla's `EntitySpawnReason` at that site.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SpawnSite {
    /// Vanilla calls `finalizeSpawn` with this reason: `finalize_spawn` is told.
    Finalizing(SpawnReason),
    /// Vanilla creates the mob without `finalizeSpawn` (a breeding, a block's or
    /// an item's mob): the game finalizes it as before, and no mod is told.
    Plain,
    Raw,
}

/// What a spawn site does with a mob after `finalize_spawn` was heard.
#[derive(Debug, PartialEq)]
pub(super) enum Finalized {
    /// Finalized as vanilla does; the site goes on to the class's follow-ups.
    Done,
    /// A mod skipped finalization: the mob spawns uninitialised.
    Skipped,
    /// A mod stopped the spawn: the mob is not added.
    Cancelled,
}

mod death_source;
mod death_weapon;
mod destroy_rules;
pub(in crate::game) use death_source::death_hurt_source;

/// A dying entity whose items drop (`LivingDropsEvent`).
pub(super) struct DyingDrops<'a> {
    pub(super) entity: u32,
    pub(super) kind: &'a str,
    /// The UUID of a dying player.
    pub(super) player: Option<u128>,
    /// The killing blow; none when no mod listens.
    pub(super) source: Option<HurtSource<'a>>,
    /// Whether a player hurt it within 100 ticks.
    pub(super) recently_hit: bool,
}

/// A sound a native mod queued, sent when the host is free.
pub(super) struct NativeSound {
    pub dimension: &'static str,
    pub sound: String,
    pub at: [f64; 3],
    pub source: i32,
    pub volume: f32,
    pub pitch: f32,
    pub seed: i64,
    /// The connection of a player who doesn't hear it, as
    /// `Level.playSound(except, ...)`.
    pub except: Option<u32>,
    /// The entity the sound follows (`ClientboundSoundEntityPacket`), sent
    /// from `at`, where it was when the mod queued the sound.
    pub entity: Option<i32>,
    /// True when queued from inside a `play_level_sound_at_*` listener so the tick flush validates
    /// and delivers the sound without re-dispatching level-sound listeners.
    pub skip_hooks: bool,
}

/// Shared state for the mob runs in one entity pass.
pub(in crate::game) struct MobDrawFrame<'a> {
    living: LivingFrame<'a>,
    precheck: bool,
}

#[derive(Default)]
struct NormalActionFrame {
    events: std::collections::VecDeque<MobEvent>,
    broke: Vec<u32>,
    actor: Option<u32>,
}
#[derive(Default)]
struct NormalMobActions {
    root: NormalActionFrame,
    nested: Vec<NormalActionFrame>,
}
impl NormalMobActions {
    fn new(events: Vec<MobEvent>) -> Self {
        Self {
            root: NormalActionFrame {
                events: events.into(),
                broke: Vec::new(),
                actor: None,
            },
            nested: Vec::new(),
        }
    }
}
struct MobOutputBatch {
    normal: NormalMobActions,
    guests: Vec<(u32, u32, MobEvent)>,
}
#[derive(Default)]
struct PendingMobBatch {
    batches: std::collections::VecDeque<MobOutputBatch>,
    pushes: sunrise_client::mobs::PendingPushes,
}

/// Remove the current action before dispatch; retain only the unentered tail on unwind.
pub(super) struct RemainingActions<'a, T> {
    remaining: std::vec::IntoIter<T>,
    target: &'a mut Vec<T>,
}
impl<'a, T> RemainingActions<'a, T> {
    pub(super) fn new(target: &'a mut Vec<T>) -> Self {
        Self {
            remaining: std::mem::take(target).into_iter(),
            target,
        }
    }
}
impl<T> Iterator for RemainingActions<'_, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.remaining.next()
    }
}
impl<T> Drop for RemainingActions<'_, T> {
    fn drop(&mut self) {
        let remaining = std::mem::replace(&mut self.remaining, Vec::new().into_iter());
        *self.target = remaining.collect();
    }
}

/// Mob-system state owned by the game.
#[derive(Default)]
pub(super) struct Creatures {
    pub projectiles: sunrise_client::join_seq::JoinList<Projectile, false>,
    /// The entities a family tells the mods ticked, kept between ticks
    /// (game/modding/native_ticks.rs).
    pub tick_buffer: Vec<(u32, super::native_ticks::TickKind)>,
    /// What a family's Pre and Post are told from, kept between ticks
    /// (game/modding/native_ticks.rs).
    pub tick_hooks: super::native_ticks::TickBuffers,
    /// The guests (by connection) whose `ServerPlayer.tick` a mod's Pre canceled this tick:
    /// every part of it skips them (game/network/lan_actions/tick.rs).
    pub canceled_players: Vec<u32>,
    /// Wind bursts from a guest's turn, waiting for it to end (game/interface/wind.rs).
    pub wind_bursts: Vec<super::wind::Burst>,
    /// The wind charge whose burst is being handled, by entity id: which
    /// entities have already ticked in vanilla's spawn-ordered tick list
    /// (game/interface/wind.rs).
    pub wind_source: Option<sunrise_client::join_seq::JoinSeq>,
    /// Wind pushes held until the entity's own tick has run, by entity id
    /// (game/interface/wind.rs `apply_late_wind_pushes`).
    pub late_wind_pushes: Vec<(u32, crate::game::Vec3)>,
    /// The players a fishing hook can catch, taken as the hooks tick, and
    /// the pulls on them waiting for the hub (game/items/fishing.rs).
    pub hook_targets: Vec<super::fishing::HookTarget>,
    pub hook_pulls: Vec<(u32, sunrise_core::geometry::Vec3)>,
    /// Mace smash knockback on the other players from a guest's turn, waiting
    /// for it to end (game/items/mace.rs).
    pub smash_pushes: Vec<super::mace::SmashPush>,
    /// Sweep attacks from a guest's turn, waiting for it to end
    /// (game/network/lan_pvp.rs).
    pub sweeps: Vec<super::lan_pvp::Sweep>,
    /// Explosion packets from a guest's turn, waiting for it to end
    /// (game/network/lan_particles.rs).
    pub lan_explosions: Vec<super::lan_particles::LanExplosion>,
    /// The snapshot's buffer between ticks, empty: ~400 bytes a mob, so
    /// thousands of mobs would allocate and free megabytes every tick.
    seen_buf: Vec<Seen>,
    /// Per-tick scratch retained between creature ticks.
    event_buf: Vec<MobEvent>,
    pickable_projectile_targets: Vec<(Who, Aabb)>,
    /// For each arrow an entity hit in this projectile tick, the projectiles
    /// that ticked after it and stayed, in order: a bounced one goes back
    /// before the first still listed (`finish_arrow_hit`).
    bounced_followers: Vec<(u32, Vec<u32>)>,
    pending: std::collections::HashMap<&'static str, PendingMobBatch>,
    /// The mobs whose own tick ran in the last `tick_creature_entities`:
    /// `ServerLevel.tick` ticks only the entities in entity-ticking range, so
    /// what the passes after the loop do for a mob's `aiStep`, goals or brain
    /// (pickups, item searches) is for these alone.
    pub ticked: sunrise_client::fast_hash::FxHashSet<u32>,
    pub deferred_hits: bool,
    /// The wandering trader spawner's clock.
    pub trader: super::wandering_trader::Spawner,
    /// When each player last put a parrot on a shoulder (`timeEntitySatOnShoulder`).
    pub shoulder_time: std::collections::HashMap<[i32; 4], i64>,
    /// The cat spawner (`CatSpawner`): its countdown.
    pub cat: super::cat_spawner::CatSpawner,
    /// Each level's custom spawners, built when its first tick asks (game/mobs/custom_spawners.rs).
    pub custom_spawners: std::collections::HashMap<super::Dim, mod_api::CustomSpawners>,
    /// Native sounds waiting for the server host to be available.
    pub lan_sounds: Vec<NativeSound>,
    /// True while `ask_level_sound` is running `play_level_sound_at_*` listeners so sounds emitted
    /// from inside a listener are not re-dispatched to level-sound hooks.
    pub asking_level_sound: bool,
    /// Particles from a guest's turn, waiting for it to end.
    pub lan_particles: Vec<super::lan_particles::QueuedParticle>,
}
/// `Projectile.isPickable`: a projectile in `#minecraft:redirectable_projectile`
/// (a ghast's fireball and both wind charges) is pickable, so another
/// projectile's `ProjectileUtil.getEntityHitResult` can strike it
/// (`Projectile.onHit` then redirects it). `PICKABLE_PROJECTILE_IDS + j` is
/// `projectiles[j]` during `tick_projectiles`, below the dragon's reserved ids.
const PICKABLE_PROJECTILE_IDS: u32 = super::dragon::SPECIAL - 0x10000;

/// The index of the projectile a `pickable_projectile_hitboxes` id stands for.
pub(super) fn pickable_projectile_hit(id: u32) -> Option<usize> {
    (PICKABLE_PROJECTILE_IDS..super::dragon::SPECIAL)
        .contains(&id)
        .then(|| (id - PICKABLE_PROJECTILE_IDS) as usize)
}

fn pickable_projectile_hitboxes_into(
    projectiles: &[Projectile],
    alive: &[bool],
    skip: usize,
    out: &mut Vec<(Who, Aabb)>,
) {
    out.clear();
    out.extend(
        projectiles
            .iter()
            .enumerate()
            .filter(|(j, q)| *j != skip && alive[*j] && super::deflect::redirectable(q))
            .map(|(j, q)| (Who::Mob(PICKABLE_PROJECTILE_IDS + j as u32), q.aabb())),
    );
}

/// What the server's projectile loop keeps to hand its list back for a projectile told alone,
/// made only in a tick where a mod named a projectile type: each slot's projectile by id, the
/// ids of the list's rows, where those are once a hook changed the list, and the projectile
/// whose own Post is still to tell.
struct ProjectileHandBack {
    slots: Vec<u32>,
    rows: Vec<u32>,
    places: Option<sunrise_client::fast_hash::FxHashMap<u32, usize>>,
    told: Option<(u32, &'static str)>,
}

impl ProjectileHandBack {
    fn new(projectiles: &[Projectile]) -> Box<Self> {
        let slots: Vec<u32> = projectiles.iter().map(|p| p.id).collect();
        Box::new(Self {
            rows: slots.clone(),
            slots,
            places: None,
            told: None,
        })
    }

    /// Where slot `slot`'s projectile is in the list now, if it is still there.
    fn place(&self, slot: usize) -> Option<usize> {
        super::native_ticks::slot_place(&self.places, &self.slots, slot)
    }

    /// After a hook told with the list handed back changed it (it wrote, added or removed a
    /// projectile, or the tick's spawns went first on the hand-back): the loop's state of each
    /// row follows it by id. A new row is kept, and an impact whose projectile a hook took out
    /// is dropped with it.
    fn follow(&mut self, projectiles: &[Projectile], keep: &mut Vec<bool>, impacts: &mut [Impact]) {
        let places = super::native_ticks::places_of(projectiles.iter().map(|p| p.id));
        for impact in impacts {
            impact.index = places.get(&impact.before.id).copied();
        }
        let kept: sunrise_client::fast_hash::FxHashMap<u32, bool> = self
            .rows
            .iter()
            .copied()
            .zip(keep.iter().copied())
            .collect();
        self.rows.clear();
        self.rows.extend(projectiles.iter().map(|p| p.id));
        keep.clear();
        keep.extend(
            self.rows
                .iter()
                .map(|id| kept.get(id).copied().unwrap_or(true)),
        );
        self.places = Some(places);
    }
}

/// Where an arrow an entity turned back goes back among `projectiles`: before
/// the first of `followers` (the projectiles that ticked after it and stayed,
/// in order) still listed, else last. Spawns restored ahead of the survivors
/// and settled impacts can leave the list out of join order, so its place is
/// taken from the tick, not from the join numbers.
fn bounced_place(projectiles: &[Projectile], followers: &[u32]) -> usize {
    followers
        .iter()
        .find_map(|id| projectiles.iter().position(|q| q.id == *id))
        .unwrap_or(projectiles.len())
}

#[cfg(test)]
fn pickable_projectile_hitboxes(
    projectiles: &[Projectile],
    alive: &[bool],
    skip: usize,
) -> Vec<(Who, Aabb)> {
    let mut out = Vec::new();
    pickable_projectile_hitboxes_into(projectiles, alive, skip, &mut out);
    out
}

/// `FlameFeatureRenderer`: fire quads stacked up a burning entity's box,
/// turned to the camera around Y, at full block light. `at` is the entity's
/// feet relative to the camera.
fn flame_geometry(
    atlas: &sunrise_client::hud::HudAtlas,
    at: glam::Vec3,
    width: f32,
    height: f32,
    camera_yaw: f32,
    light: u8,
    out: &mut Vec<Vertex>,
) {
    let (Some(fire0), Some(fire1)) = (
        atlas.sprites.get("block/fire_0").copied(),
        atlas.sprites.get("block/fire_1").copied(),
    ) else {
        return;
    };
    let s = width * 1.4;
    let mut h = height / s;
    // Mth.rotationAroundAxis(Y, camera.orientation): the camera's turn about Y.
    let turn = glam::Mat3::from_rotation_y(std::f32::consts::PI - camera_yaw.to_radians());
    let lift = 0.3 - (h as i32) as f32 * 0.02;
    let packed = (u32::from(light >> 4) * 16) | ((15 * 16) << 8);
    let (mut r, mut yo, mut zo, mut ss) = (0.5f32, 0.0f32, 0.0f32, 0);
    while h > 0.0 {
        let tex = if ss % 2 == 0 { fire0 } else { fire1 };
        let (mut u0, v0, mut u1, v1) = (tex.u0, tex.v0, tex.u1, tex.v1);
        if ss / 2 % 2 == 0 {
            std::mem::swap(&mut u0, &mut u1);
        }
        for (x, y, uv) in [
            (-r, -yo, [u1, v1]),
            (r, -yo, [u0, v1]),
            (r, 1.4 - yo, [u0, v0]),
            (-r, 1.4 - yo, [u1, v0]),
        ] {
            let p = at + turn * glam::Vec3::new(x, y, zo + lift) * s;
            out.push(Vertex {
                normal: [0; 4],
                pos: p.to_array(),
                uv,
                // HDR output: the flames shine like fire (entity.wgsl fs_hdr_mark).
                layer: u32::from(sunrise_client::mesher::INCANDESCENT) << 16,
                color: 0xFFFF_FFFF,
                light: packed,

                glint_uv: [0.0, 0.0],
            });
        }
        h -= 0.45;
        yo -= 0.45;
        r *= 0.9;
        zo -= 0.03;
        ss += 1;
    }
}
pub(super) fn seen(m: &Mob, player: Option<[i32; 4]>, items: &Items) -> Seen {
    m.seen(player, items)
}
/// Whether a mob a packet named may take the click: `mob_hit` skips a mob in
/// its death animation, the one the player rides (it shares the player's root
/// vehicle) and the camera entity the pick starts from, so a packet that names
/// one of those does nothing rather than reaching the feed, shear, leash or
/// name paths the pick path refuses.
pub(super) fn named_mob_pickable(
    alive: bool,
    id: u32,
    riding: Option<u32>,
    camera: Option<u32>,
) -> bool {
    alive && Some(id) != riding && Some(id) != camera
}

/// What an entity drew on the GPU path: the skin frame's mark before its draws, and their
/// camera-relative bounds (`cull_skinned`).
pub(super) type Skinned = Option<((usize, usize, u32), Option<(glam::Vec3, glam::Vec3)>)>;

/// What a block's `onProjectileHit` reads of the projectile that struck it:
/// a thrown projectile ([`ProjectileHitter::of`]) or a fishing bobber
/// (`FishingHook.onHitBlock` calls `Projectile.onHitBlock` too).
#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectileHitter {
    /// `Projectile.getOwner`.
    pub(super) owner: Option<Who>,
    /// On a server, the guest (by connection) who owns it.
    pub(super) guest: Option<u32>,
    /// An `AbstractArrow` (a target block resets after 20 ticks, not 8).
    pub(super) arrow: bool,
    /// Its entity type, for `target_hit`'s projectile predicate.
    pub(super) kind: &'static str,
    pub(super) position: Vec3,
}
impl ProjectileHitter {
    pub(super) fn of(p: &Projectile) -> Self {
        Self {
            owner: p.owner,
            guest: p.guest,
            arrow: p.arrow.is_some(),
            kind: p.entity_type(),
            position: p.position,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OrbOrder {
    BeforeChild,
    AfterChild,
}

impl Game<'_> {
    /// The game's own player's uuid and entity id — the identity behind
    /// [`Self::player_view`], so a tick that lends no guest's view names its
    /// player (#1077). None where the view stands for nobody: no player of
    /// the game's own, a fake one, or a guest's turn acting in its place.
    pub(super) fn player_identity(&self) -> Option<([i32; 4], u32)> {
        if self.lan_acting_fake || self.lan_acting_uuid.is_some() || self.player.is_none() {
            return None;
        }
        let uuid = self.level.player_uuid?;
        let id = u32::try_from(self.player.as_ref()?.entity_id).ok()?;
        Some((uuid, id))
    }
    /// A sulfur cube's attribute map (`LivingEntity.attributes`), from its
    /// type's defaults, before its first tick adopts the body's archetype
    /// modifiers. A map already there (commands, armor) is kept.
    fn init_sulfur_attribute_maps(&mut self) {
        for m in &mut self.mobs {
            if m.species() == sunrise_client::mobs::Species::SulfurCube
                && m.extra.armor_attributes.is_none()
            {
                m.extra.armor_attributes = self.motion.attributes.create(&m.kind.id).map(Box::new);
            }
        }
    }
    pub(super) fn player_view(&self) -> Option<PlayerView> {
        if self.lan_acting_fake {
            return None;
        }
        let p = self.player.as_ref()?;
        let w = f64::from(p.dimensions.width) / 2.0;
        let bounds = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        );
        Some(PlayerView {
            position: p.position,
            bounds,
            eye: [
                p.position.x,
                p.position.y + f64::from(p.dimensions.eye_height),
                p.position.z,
            ],
            alive: !p.health.dead_or_dying(),
            invulnerable: p.invulnerable
                || p.health.permanently_invulnerable
                || p.health.invulnerable_time > 0,
            // EntitySelector.NO_CREATIVE_OR_SPECTATOR.
            creative: self.creative || self.spectator(),
            sneaking: self.held_mapping(Action::Sneak),
            swimming: p.swimming,
            look: view_vector(self.yaw, self.pitch),
            main_hand: self.held().map(|s| s.item.clone()),
            off_hand: self.offhand.as_ref().map(|s| s.item.clone()),
            head: self.armor[0].as_ref().map(|s| s.item.clone()),
            chest: self.armor[1].as_ref().map(|s| s.item.clone()),
            legs: self.armor[2].as_ref().map(|s| s.item.clone()),
            feet: self.armor[3].as_ref().map(|s| s.item.clone()),
            armor_cover: self.armor.iter().filter(|a| a.is_some()).count() as f32 / 4.0,
            invisible: self.potions.effects.get("invisibility").is_some(),
            spectator: self.spectator(),
            gold_armor: self.wearing_gold(),
            hurt_by: self.player_hurt_by(),
            last_hurt: self.combat.last_hurt_mob.map(|(id, at)| (Who::Mob(id), at)),
            velocity: p.velocity,
            yaw: self.yaw,
            sprinting: p.sprinting,
            riding: self.riding.is_some(),
            hero: self.potions.effects.get("hero_of_the_village").is_some(),
            effects: self.potions.effects.list.iter().map(|e| e.id).collect(),
            uuid: self.acting_uuid(),
            allies: std::sync::Arc::new(self.player_allies()),
            remote: false,
            sleeping: self.sleeping.is_some(),
            sleep_timer: self.sleep_timer,
            passenger: self.riding.is_some(),
            riding_vehicle: self.riding,
            vehicle: self.riding.and_then(|id| {
                if let Some(b) = self.boats.iter().find(|b| b.id == id) {
                    // A boat's box: 1.375 wide and 0.5625 high around its position.
                    let w = 0.6875;
                    return Some(sunrise_client::mobs::Vehicle {
                        nautilus: false,
                        bounds: Aabb::new(
                            Vec3::new(b.position.x - w, b.position.y, b.position.z - w),
                            Vec3::new(b.position.x + w, b.position.y + 0.5625, b.position.z + w),
                        ),
                        velocity: b.velocity,
                    });
                }
                let m = self.mobs.iter().find(|m| m.id == id)?;
                matches!(m.species(), Species::Nautilus | Species::ZombieNautilus).then(|| {
                    sunrise_client::mobs::Vehicle {
                        nautilus: true,
                        bounds: m.bounds(),
                        velocity: m.velocity,
                    }
                })
            }),
        })
    }
    /// Biome facts at a position for finalizeSpawn and variant rules.
    pub(super) fn biome_facts(&self, x: i32, y: i32, z: i32) -> BiomeFacts {
        biome_facts::biome_facts_in(
            &self.world,
            world_biomes!(self),
            self.res.mob_data(),
            x,
            y,
            z,
        )
    }
    /// Create a mob of `kind` at a position, finalized like a natural spawn.
    pub(super) fn spawn_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        site: SpawnSite,
    ) -> Option<u32> {
        let mob = self.create_mob_reason(kind, at, persistent, site, None)?;
        self.insert_spawned_mob(mob)
    }
    /// [`spawn_mob`](Self::spawn_mob) for a creation that snaps the mob to a
    /// yaw (`snapTo(pos, yaw, 0)`): the facing takes no draw from the level.
    pub(super) fn spawn_mob_facing(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        site: SpawnSite,
        yaw: f32,
    ) -> Option<u32> {
        let mob = self.create_mob_reason(kind, at, persistent, site, Some(yaw))?;
        self.insert_spawned_mob(mob)
    }
    /// Create a mob of `kind` at a position without `finalizeSpawn`'s rolls,
    /// matching vanilla `EntityType.create(level, reason)`.
    pub(super) fn spawn_mob_raw(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
    ) -> Option<u32> {
        self.spawn_mob(kind, at, persistent, SpawnSite::Raw)
    }
    /// `SummonCommand` and `SpawnEggItem`: vanilla spawns the mob, but it
    /// never calls `Mob.setPersistenceRequired` for it, so the mob is not
    /// persistent: the despawn rule holds it like any other mob, and only
    /// its NBT can make it persistent.
    pub(super) fn spawn_mob_spawned(
        &mut self,
        kind: &str,
        at: [f64; 3],
        site: SpawnSite,
    ) -> Option<u32> {
        // Dragons have a separate entity implementation, but eggs use the same spawn path.
        if matches!(kind, "ender_dragon" | "minecraft:ender_dragon") {
            return self.spawn_dragon_spawned(at, site);
        }
        // The spawn is not a natural one (no jockeys, the patrol leader roll
        // stays), but the mob is not persistent: vanilla's SummonCommand
        // never calls setPersistenceRequired. The flag is false from the
        // start, before the mob joins the level, so a mod's own write in the
        // entity_join hook survives it - clearing it after the join would
        // wipe what the hook set (Mob.setPersistenceRequired from
        // entity_join_level).
        self.spawn_mob(kind, at, false, site)
    }
    /// [`spawn_mob_spawned`](Self::spawn_mob_spawned) for a mob that starts
    /// moving or turned: `velocity` (`Entity.setDeltaMovement`) and, when
    /// given, `[yaw, pitch]` in place of the level random's yaw, set before
    /// it joins. The kind resolves as `spawn_mob_spawned`'s (`cow` is
    /// `minecraft:cow`), and the ender dragon takes its own creation, facing
    /// the given yaw; it has no momentum or pitch of its own here, so a
    /// dragon given a velocity or a pitch is refused.
    pub(in crate::game) fn spawn_mob_spawned_moving(
        &mut self,
        kind: &str,
        at: [f64; 3],
        site: SpawnSite,
        velocity: Vec3,
        rotation: Option<[f32; 2]>,
    ) -> Option<u32> {
        if matches!(kind, "ender_dragon" | "minecraft:ender_dragon") {
            if velocity != Vec3::ZERO || rotation.is_some_and(|[_, pitch]| pitch != 0.0) {
                return None;
            }
            return self.spawn_dragon_facing(at, site, rotation.map(|[yaw, _]| yaw));
        }
        // Not persistent, as `spawn_mob_spawned`'s mob.
        let mut mob = self.create_mob_reason(kind, at, false, site, rotation.map(|r| r[0]))?;
        mob.velocity = velocity;
        if let Some([_, pitch]) = rotation {
            (mob.head_pitch, mob.previous_head_pitch) = (pitch, pitch);
        }
        self.insert_spawned_mob(mob)
    }
    fn create_mob_reason(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        site: SpawnSite,
        fixed_yaw: Option<f32>,
    ) -> Option<Mob> {
        let kind = if kind.contains(':') {
            kind.to_owned()
        } else {
            format!("minecraft:{kind}")
        };
        let t = self.res.mob_types().get(&kind).cloned()?;
        self.next_entity += 1;
        // Yaw belongs to the vanilla creation path, independently of whether
        // the fork's SpawnSite allows finalizeSpawn (and its hook).
        let yaw = match (fixed_yaw, site) {
            (Some(yaw), _) => yaw,
            (None, SpawnSite::Finalizing(SpawnReason::Command)) => 0.0,
            (None, SpawnSite::Finalizing(SpawnReason::SpawnItemUse | SpawnReason::Dispenser)) => {
                sunrise_client::camera::wrap_degrees(self.world.random.next_float() * 360.0)
            }
            _ => self.world.random.next_float() * 360.0,
        };
        // Entity.random first supplies the UUID, then LivingEntity's initial
        // yaw; species constructor draws continue from that same stream.
        let (own, uuid) = sunrise_client::mobs::living_random();
        self.persist.uuids.insert(self.next_entity, uuid);
        let mut mob = Mob::with_random(
            self.next_entity,
            t,
            Vec3::new(at[0], at[1], at[2]),
            yaw,
            own,
        );
        // `EntityType.create`: the mob is constructed before `finalizeSpawn`.
        self.initialize_entity(mod_api::Holder::Entity(mob.id), &kind);
        mob.persistent = persistent;
        match site {
            SpawnSite::Raw => return Some(mob),
            SpawnSite::Finalizing(reason) => match self.finalize_with(&mut mob, reason, None) {
                Finalized::Cancelled => return self.discard_unjoined_mob(mob.id),
                Finalized::Skipped => return Some(mob),
                Finalized::Done => {}
            },
            // A plain spawn finalizes as a summoned one (vanilla's /summon
            // and egg path): its few reason-gated arms behave the same.
            SpawnSite::Plain => {
                let difficulty =
                    self.local_difficulty([mob.position.x, mob.position.y, mob.position.z]);
                self.finalize_plain(
                    &mut mob,
                    &difficulty,
                    sunrise_client::mobs::spawn_rules::Reason::MobSummoned,
                );
            }
        }
        Some(mob)
    }

    fn insert_spawned_mob(&mut self, mob: Mob) -> Option<u32> {
        let id = mob.id;
        self.mobs.push(mob);
        self.native_mobs_joined(self.mobs.len() - 1, false);
        if !self.mobs.iter().any(|m| m.id == id) {
            return None;
        }
        Some(id)
    }
    /// `Mob.burnUndead` for a mob in daylight with an item on its sun
    /// protection slot (the head, or the body for a zombie horse or
    /// nautilus): `hurtAndBreak(random.nextInt(2))` on a damageable item, and
    /// when it breaks the slot empties and `onEquippedItemBroken` broadcasts
    /// the break event.
    fn wear_sun_protection(&mut self, id: u32) {
        use sunrise_core::equipment_slot::EquipmentSlot;
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let slot = m.sun_protection_slot();
        let body = slot == EquipmentSlot::Body;
        let Some(stack) = m.item_by_slot(slot) else {
            return;
        };
        if !sunrise_client::stations::is_damageable(self.res.items(), &stack) {
            return;
        }
        let at = [m.position.x, m.position.y, m.position.z];
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let amount = m.random.next_int_bound(2).unwrap_or(0) as u32;
        if amount == 0 {
            return;
        }
        let (updated, broken) = self.damage_living_stack(&stack, amount, id);
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if m.item_by_slot(slot).as_ref() != Some(&stack) {
            return;
        }
        m.set_equipment_stack(slot as usize, updated);
        if broken.is_some() {
            // LivingEntity.entityEventForEquipmentBreak: head 49, body 65.
            self.lan_entity_event(id as i32, if body { 65 } else { 49 }, at);
        }
    }
    /// `LivingEntity.updateFallFlying`'s wear, `ItemStack.hurtAndBreak(1, this,
    /// slot)` on the glider `MobEvent::GliderWear` picked: Unbreaking (from the
    /// level's random, `Enchantment.modifyItemFilteredCount`) may save it, and
    /// the rest goes on the stack where it is worn. Only a part's `break_now`
    /// breaks one (`canGlideUsing` picks none a point would break): emptied, with its event.
    pub(in crate::game) fn wear_mob_glider(
        &mut self,
        id: u32,
        slot: sunrise_core::equipment_slot::EquipmentSlot,
    ) {
        let Some(stack) = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| sunrise_client::mobs::finalize::equipment_stack(m, slot))
        else {
            return;
        };
        if !sunrise_client::stations::is_damageable(self.res.items(), &stack) {
            return;
        }
        let (updated, broken) = self.damage_living_stack(&stack, 1, id);
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        m.set_equipment_stack(slot as usize, updated);
        if broken.is_some() {
            let event = sunrise_client::mobs::item_break::equipment_break_event(slot);
            self.lan_entity_event(id as i32, event, at);
        }
    }
    /// `LivingEntity.hurtArmor` -> `LivingEntity.doHurtEquipment`, from the hit
    /// `MobEvent::ArmorHurt` reported. Only `Horse` overrides `hurtArmor` for a
    /// mob, and its override asks for the BODY slot alone, so the body piece is
    /// the only one this wears: it loses `max(1, damage / 4)` durability,
    /// Unbreaking applied, and at its maximum the slot empties and the break
    /// event goes out (`LivingEntity.entityEventForEquipmentBreak`: 65 for the
    /// body — 49 to 52 are the four armour slots, which only a player's own
    /// pieces reach). A fire hit leaves a fire-resistant piece alone
    /// (`ItemStack.canBeHurtBy`).
    fn hurt_mob_armor(&mut self, id: u32, damage: f32, source: &HurtSource<'_>) {
        use sunrise_core::equipment_slot::EquipmentSlot;
        if damage <= 0.0 {
            return;
        }
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        let amount = ((damage / 4.0) as u32).max(1);
        let name = source.damage_type;
        // A piece's maximum, or none when it may not lose durability at all.
        // `doHurtEquipment` starts from the item's `equippable` component
        // (`Equippable equippable = item.get(DataComponents.EQUIPPABLE)`), so a
        // damageable item with no such component — a sword put in a horse's body
        // slot by `/item replace` — never wears; `equip_damage_on_hurt` alone
        // cannot say that, since it defaults to true when the component is absent.
        fn wearable(me: &Game<'_>, stack: &ItemStack, name: &str) -> Option<u32> {
            let info = me.res.items().get(&stack.item)?;
            if info.equip_slot.is_none()
                || !sunrise_client::item_sync::damage_on_hurt(stack, info.equip_damage_on_hurt)
                || info.max_damage == 0
                || !sunrise_client::stations::is_damageable(me.res.items(), stack)
                || !me.stack_can_be_hurt_by(stack, name).unwrap_or(true)
            {
                return None;
            }
            Some(info.max_damage)
        }
        let Some(body) = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.item_by_slot(EquipmentSlot::Body))
        else {
            return;
        };
        if wearable(self, &body, name).is_none() {
            return;
        }
        // ArmorHurtEvent for a mob's body armor.
        let mut amount = amount;
        if mod_api::__private::listened(mod_api::HookId::ArmorHurt)
            && let Ok(body_snapshot) = body.try_into_api()
        {
            let kind = self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(super::native_hooks::mob_kind)
                .unwrap_or_default();
            let mut entries = vec![mod_api::ArmorHurt {
                slot: EquipmentSlot::Body,
                stack: body_snapshot,
                damage: super::durability_damage(damage),
                new_damage: super::durability_damage(damage),
            }];
            let mut cancel = false;
            self.native_armor_hurt(None, id, &kind, source, &mut entries, &mut cancel);
            if cancel {
                return;
            }
            amount = super::answered_wear(&entries, EquipmentSlot::Body, amount);
            if amount == 0 {
                return;
            }
            // A mod may have swapped the piece: it is not worn by this hit.
            if self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .and_then(|m| m.item_by_slot(EquipmentSlot::Body))
                != Some(body.clone())
            {
                return;
            }
        }
        let (updated, broken) = self.damage_living_stack(&body, amount, id);
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if m.item_by_slot(EquipmentSlot::Body).as_ref() != Some(&body) {
            return;
        }
        m.set_equipment_stack(EquipmentSlot::Body as usize, updated);
        if broken.is_some() {
            // LivingEntity.onEquippedItemBroken through the level's broadcast.
            let event =
                sunrise_client::mobs::item_break::equipment_break_event(EquipmentSlot::Body);
            self.lan_entity_event(id as i32, event, at);
        }
    }
    /// Finalize a converted frog or witch before it joins the level.
    fn finalize_converted(&mut self, mob: &mut Mob) {
        // A mod that stops a conversion's finalization leaves the mob as converted.
        self.finalize_with(mob, SpawnReason::Conversion, None);
        mob.persistent = true;
    }
    /// `Mob.convertTo(type, ConversionParams.single(...))`: a new mob of
    /// `kind`, made without spawn finalization unless its conversion asks, takes
    /// its place with what `conversion::single` carries over, and the old
    /// mob's riders move onto it. A zombie that comes out of it rolls
    /// `Zombie.handleAttributes(special, CONVERSION)` (the
    /// `postConversionStep` of its `ConversionTracker`). `pre_checked` skips
    /// `LivingConversionEvent.Pre`, which the lightning asks before it picks
    /// its outcome.
    pub(super) fn convert_to(
        &mut self,
        from: &Mob,
        kind: &str,
        params: sunrise_client::mobs::conversion::Params,
        pre_checked: bool,
        after: impl FnOnce(&mut Self, &mut Mob),
    ) -> Option<u32> {
        // Tick conversions can have been removed from the tick list before
        // their queued event is handled. Restore them until Post has returned.
        match self.mobs.iter_mut().find(|m| m.id == from.id) {
            Some(old) => old.removed = false,
            None => {
                let mut old = from.clone();
                old.removed = false;
                self.mobs.push(old);
            }
        }
        let t = self.res.mob_types().get(kind).cloned()?;
        let mut cancel = false;
        let from_kind = (mod_api::__private::listened(mod_api::HookId::LivingConversionPre)
            || mod_api::__private::listened(mod_api::HookId::LivingConversionPost))
        .then(|| super::native_hooks::mob_kind(from));
        if !pre_checked && let Some(from_kind) = &from_kind {
            self.native_living_conversion_pre(from.id, from_kind, kind, &mut cancel);
        }
        if cancel {
            return None;
        }
        let from = self.mobs.iter().find(|m| m.id == from.id)?.clone();
        self.next_entity += 1;
        let mut mob = Mob::new(self.next_entity, t, from.position, from.yaw);
        sunrise_client::mobs::conversion::single(&from, &mut mob, params);
        self.transfer_conversion_identity(from.id, mob.id);
        if matches!(from.species(), Species::Zombie | Species::Husk) {
            let at = [from.position.x, from.position.y, from.position.z];
            let special = self.local_difficulty(at).special();
            sunrise_client::mobs::finalize::zombie_attributes(
                &mut mob,
                special,
                sunrise_client::mobs::spawn_rules::Reason::Conversion,
            );
        }
        match (from.species(), kind) {
            (Species::Tadpole, "minecraft:frog") | (Species::Villager, "minecraft:witch") => {
                self.finalize_converted(&mut mob);
            }
            (Species::Pig, "minecraft:zombified_piglin") => {
                let old = self.mobs.iter_mut().find(|m| m.id == from.id)?;
                sunrise_client::mobs::conversion::pig_thunder_hit(old, &mut mob);
            }
            (Species::Hoglin, "minecraft:zoglin")
            | (Species::Piglin | Species::PiglinBrute, "minecraft:zombified_piglin") => {
                mob.add_effect("minecraft:nausea", 200, 0);
            }
            _ => {}
        }
        after(self, &mut mob);
        let id = mob.id;
        self.mobs.push(mob);
        // ConversionType.SINGLE: `to.startRiding(vehicle, false, false)`.
        self.seat_mob(id);
        self.copy_conversion_attachments(from.id, id, kind);
        self.native_mobs_joined(self.mobs.len() - 1, false);
        let transformed = self.mobs.iter().find(|m| m.id == id)?;
        let transformed_kind = from_kind.as_ref().map(|_| transformed.kind.id.clone());
        // Only move passengers once the new mob's join has succeeded.
        for rider in self.mobs.iter_mut() {
            if rider.extra.mount == Some(from.id) {
                rider.extra.mount = Some(id);
            }
        }
        if let (Some(from_kind), Some(transformed_kind)) = (from_kind, transformed_kind) {
            self.native_living_conversion_post(from.id, &from_kind, id, &transformed_kind);
        }
        if let Some(old) = self.mobs.iter_mut().find(|m| m.id == from.id) {
            old.removed = true;
        }
        Some(id)
    }
    /// Default position for a summon command (3 blocks in front of the player).
    pub(super) fn default_summon_position(&self) -> Result<[f64; 3]> {
        let p = self.player.as_ref().ok_or("no player yet")?;
        let d = view_vector(self.yaw, 0.0);
        Ok([
            p.position.x + d[0] * 3.0,
            p.position.y,
            p.position.z + d[2] * 3.0,
        ])
    }
    /// The `summon` command: spawn a mob in front of the player or at a position.
    pub(super) fn summon_command_mob(
        &mut self,
        kind: &str,
        at: Option<[f64; 3]>,
        baby: bool,
    ) -> Result<u32> {
        let at = match at {
            Some(a) => a,
            None => self.default_summon_position()?,
        };
        let id = self
            .spawn_mob_spawned(kind, at, SpawnSite::Finalizing(SpawnReason::Command))
            .ok_or_else(|| format!("unknown or unsupported mob {kind}"))?;
        if baby && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.extra.age = if m.species() == Species::Sniffer {
                sunrise_client::mobs::sniffer::BABY_START_AGE
            } else {
                -24000
            };
        }
        Ok(id)
    }
    /// The `mobs` command: nearby mobs with position, health and running goals.
    pub(super) fn control_mobs(&self, radius: f64) -> serde_json::Value {
        let center = self.player.as_ref().map_or(Vec3::ZERO, |p| p.position);
        self.control_mobs_at(radius, center)
    }
    pub(super) fn control_mobs_at(&self, radius: f64, center: Vec3) -> serde_json::Value {
        let list: Vec<serde_json::Value> = self
            .mobs
            .iter()
            .filter(|m| m.distance_sqr(center) <= radius * radius)
            .map(|m| {
                let mut state = serde_json::json!({
                    "id": m.id,
                    "kind": m.kind.id,
                    "pos": [m.position.x, m.position.y, m.position.z],
                    "health": m.health,
                    "baby": m.is_baby(),
                    "target": m.ai.target.map(|t| format!("{t:?}")),
                    "goals": m.ai.goals.running_names(),
                    "held": m.extra.main_hand_id(),
                    "mount": m.extra.mount,
                    "rider": m.extra.rider,
                    // A player's input this tick, and whether it steers.
                    "ridden": m.rider.map(|r| r.steer),
                    "controlled": sunrise_client::mobs::equine::controlled(m),
                    "saddled": m.extra.saddled(),
                    "enchants": sunrise_client::mobs::gear::gear(m).map(|g| g.slots.iter().map(|s| s.iter().map(|(e, l)| format!("{e}:{l}")).collect::<Vec<_>>()).collect::<Vec<_>>()),
                    "ints": m.extra.ints,
                    "pois": [m.extra.home, m.extra.bed, m.extra.meeting],
                    "profession": m.extra.profession.index(),
                    "targets": m.ai.targets.running_names(),
                    "path": m.ai.nav.path.as_ref().map(|p| p.nodes.len() - p.next.min(p.nodes.len())),
                    "on_ground": m.ground.on_ground,
                    "variant": m.extra.variant,
                    "color": m.extra.color,
                    "sheared": m.extra.sheared,
                    "burning": m.fire_ticks > 0,
                    "size": m.extra.size,
                    "leash": m.extra.leash.map(|h| format!("{h:?}")),
                    "tame": m.extra.tame,
                    // TamableAnimal.isInSittingPose, what this side draws: a server's client
                    // has only that bit of DATA_FLAGS_ID, never the saved order to sit.
                    "sitting": m.extra.sitting_pose,
                    "temper": if sunrise_client::mobs::equine::is_equine(m.species()) {
                        sunrise_client::mobs::equine::temper(m)
                    } else { i32::from(m.extra.variant3) },
                    "chested": m.extra.chested,
                });
                // Set apart: one json! with every key goes past the default macro recursion
                // limit, and sunrise-client can't raise it (#8169).
                state["head"] = m.extra.head_id().into();
                state["body"] = m.extra.body_id().into();
                state["warden"] = serde_json::json!(sunrise_client::mobs::warden::describe(m));
                state["villager"] = serde_json::json!(sunrise_client::mobs::villager::ext(m).map(|e| serde_json::json!({
                    "food_level": e.food_level,
                    "inventory": e.inventory.iter().map(|s| format!("{}x{}", s.count, s.item)).collect::<Vec<_>>(),
                    "gossips": e.gossips.len(),
                    "last_slept": e.last_slept,
                    "raid": e.raid,
                })));
                state["raider"] = serde_json::json!(sunrise_client::mobs::raider::state(m).map(|s| serde_json::json!({
                    "raid": s.raid_id,
                    "wave": s.wave,
                    "leader": s.patrol_leader,
                    "patrolling": s.patrolling,
                    "patrol_target": s.patrol_target,
                    "vehicle": m.extra.mount,
                })));
                // Drawn seated (Entity.isPassenger).
                state["passenger"] = m.extra.passenger.into();
                state["guardian_beam"] = serde_json::json!(sunrise_client::mobs::guardian::beam(m, 0.0));
                state
            })
            .collect();
        serde_json::json!({ "count": self.mobs.len(), "near": list, "projectiles": self.creatures.projectiles.len() })
    }
    /// Highest motion-blocking (no leaves) block + 1 (Heightmap MOTION_BLOCKING_NO_LEAVES).
    pub(super) fn motion_top(&self, x: i32, z: i32) -> i32 {
        self.motion_top_in(&self.world, self.dim, x, z)
    }
    /// [`Self::motion_top`] in the loaded level `world` of `dim`.
    pub(in crate::game) fn motion_top_in(
        &self,
        world: &sunrise_client::world::World,
        dim: Dim,
        x: i32,
        z: i32,
    ) -> i32 {
        let top = if dim == Dim::Nether { 127 } else { 319 };
        for y in (world.bottom_y()..=top).rev() {
            let f = self.res.facts().get(world.block(x, y, z));
            if (f.full_collision || f.fluid != Fluid::None) && !f.name.ends_with("leaves") {
                return y + 1;
            }
        }
        world.bottom_y()
    }
    /// What the generator finalizes this level's structure mobs with.
    pub(super) fn share_spawn_conditions(&self) {
        if let Some(terrain) = self.end.locator() {
            terrain.set_spawn_conditions(self.difficulty_id() as u8, self.world.day_time);
        }
    }
    /// `Mob.finalizeSpawn` behind `FinalizeSpawnEvent`: the one fire point for
    /// the mobs the game spawns where vanilla calls `finalizeSpawn`. The class's
    /// follow-ups (raider flags and the like) are part of `finalizeSpawn`, so a
    /// site skips them on [`Finalized::Skipped`].
    pub(super) fn finalize_with(
        &mut self,
        mob: &mut Mob,
        reason: SpawnReason,
        spawner: Option<mod_api::SpawnSource>,
    ) -> Finalized {
        let mut group = None;
        let (outcome, difficulty) = self.finalize_spawn_event(mob, reason, spawner, &mut group);
        if outcome == Finalized::Done {
            self.finalize_grouped(mob, &difficulty, reason, group);
        }
        outcome
    }
    /// `Mob.finalizeSpawn`'s event, the half listeners own: they hear the
    /// spawn, cancel it, take it over, set its difficulty, edit the mob
    /// through its lend and change the pack's `group`, which finalization then
    /// reads. The decision and the difficulty the spawn finalizes with come
    /// back together.
    pub(super) fn finalize_spawn_event(
        &self,
        mob: &mut Mob,
        reason: SpawnReason,
        spawner: Option<mod_api::SpawnSource>,
        group: &mut Option<Group>,
    ) -> (Finalized, spawning::LocalDifficulty) {
        let at = [mob.position.x, mob.position.y, mob.position.z];
        let difficulty = self.local_difficulty(at);
        if !mod_api::__private::listened(mod_api::HookId::FinalizeSpawn) {
            return (Finalized::Done, difficulty);
        }
        let kind = mob.kind.id.clone();
        self.finalize_spawn_lent(&kind, at, reason, spawner, difficulty, Some(mob), group)
    }
    /// `Mob.getMaxSpawnClusterSize` as the entity parts of the finalized `mob`'s type answer it
    /// (`EntityPart::spawn_cluster_size`), asked with `mob` lent as it stands before it joins the
    /// level: the first part in dispatch order to answer wins, and none answering leaves vanilla's
    /// `vanilla` to the spawner. A type with no part costs one table lookup. Server-side.
    pub(super) fn spawn_cluster_size(
        &self,
        mob: &mut Mob,
        reason: SpawnReason,
        spawner: Option<mod_api::SpawnSource>,
        vanilla: u32,
    ) -> Option<u32> {
        if !mod_api::__private::has_entity_parts() {
            return None;
        }
        let parts = mod_api::__private::entity_parts();
        let kind = mob.kind.id.clone();
        let list = parts.entity_parts(&kind);
        if list.is_empty() {
            return None;
        }
        let at = [mob.position.x, mob.position.y, mob.position.z];
        self.lend_unspawned(&kind, at, Some(mob), |lent, _| {
            list.iter().find_map(|attached| {
                let _guard =
                    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                        id: attached.mod_id(),
                        hook: "spawn_cluster_size",
                        part: Some(attached.type_name()),
                    });
                attached
                    .part()
                    .spawn_cluster_size(self, lent, reason, spawner, vanilla)
            })
        })
    }
    /// The shared finalization decision for a mob type the game keeps apart
    /// from mobs (the dragon), before the entity joins the level: its lend has
    /// a kind and a position and no mob.
    pub(super) fn finalize_spawn_at(
        &self,
        kind: &str,
        at: [f64; 3],
        reason: SpawnReason,
        spawner: Option<mod_api::SpawnSource>,
    ) -> (Finalized, spawning::LocalDifficulty) {
        let difficulty = self.local_difficulty(at);
        if !mod_api::__private::listened(mod_api::HookId::FinalizeSpawn) {
            return (Finalized::Done, difficulty);
        }
        self.finalize_spawn_lent(kind, at, reason, spawner, difficulty, None, &mut None)
    }
    /// Fires `finalize_spawn` with `mob` lent to it and the group data as the
    /// API's; a group the listener changed comes back as the game's.
    #[allow(clippy::too_many_arguments)]
    fn finalize_spawn_lent(
        &self,
        kind: &str,
        at: [f64; 3],
        reason: SpawnReason,
        spawner: Option<mod_api::SpawnSource>,
        difficulty: spawning::LocalDifficulty,
        mob: Option<&mut Mob>,
        group: &mut Option<Group>,
    ) -> (Finalized, spawning::LocalDifficulty) {
        let mut heard = mod_api::Difficulty::from(difficulty);
        let (mut spawn_cancelled, mut skip) = (false, false);
        let given = group
            .as_ref()
            .and_then(|g| super::native_unspawned_mob::native_spawn_group(g, self.lend_epoch()));
        let mut native_group = given.clone();
        self.lend_unspawned(kind, at, mob, |mob, edits| {
            self.native_finalize_spawn(mod_api::__private::FinalizeSpawnCall {
                dimension: self.dim.id(),
                kind,
                reason,
                pos: at,
                spawner,
                difficulty: &mut heard,
                spawn_cancelled: &mut spawn_cancelled,
                mob,
                edits,
                group: &mut native_group,
                cancel: &mut skip,
            });
        });
        if native_group != given {
            *group = native_group
                .as_ref()
                .and_then(super::native_unspawned_mob::spawn_group);
        }
        if spawn_cancelled {
            return (Finalized::Cancelled, difficulty);
        }
        if skip {
            return (Finalized::Skipped, difficulty);
        }
        (Finalized::Done, heard.into())
    }
    /// `ServerLevel.getCurrentDifficultyAt` in the level in place.
    pub(super) fn local_difficulty(&self, at: [f64; 3]) -> spawning::LocalDifficulty {
        local_difficulty_in(&self.world, at)
    }
    /// Generation's animals for the chunks generated since, once lit: chunk
    /// generation places them whether the level ticks or not.
    pub(super) fn spawn_fresh_chunks(&mut self) {
        if !self.world.chunks.has_fresh() {
            return;
        }
        let fresh: Vec<(i32, i32)> = self
            .world
            .chunks
            .fresh_keys_ordered()
            .into_iter()
            .filter(|k| self.world.chunks.get(k).is_some_and(|c| c.fresh && c.lit))
            .collect();
        for key in fresh {
            if let Some(c) = self.world.chunks.get_mut(&key) {
                Arc::make_mut(c).fresh = false;
            }
            self.world.chunks.remove_fresh(&key);
            self.spawn_generation_creatures(key.0, key.1);
        }
    }
    /// Advance every mob one tick, then apply their events.
    pub(super) fn tick_creatures(&mut self) -> Result<()> {
        // Hurts dealt since the last creature tick (explosions, commands, the
        // systems after it) teleport an enderman before it ticks, not in its AI step.
        self.settle_hurt_teleports()?;
        self.attack_ticks = self.attack_ticks.saturating_add(1);
        self.refresh_gear_knockback_resistance();
        self.tick_leads();
        // Vanilla's spawning and raids run before ChunkMap.tick.
        self.tick_spawners();
        let ticked =
            sunrise_client::spans::time("tick_creature_entities", || self.tick_creature_entities());
        self.dismount_underwater_mob_riders();
        ticked
    }
    /// `DistanceManager.inEntityTickingRange` in an entity-ticking chunk
    /// (block coordinates), from every player's tickets (`update_chunk_tickets`).
    pub(super) fn entity_ticking_area(&self) -> impl Fn(i32, i32) -> bool + use<> {
        let levels = self.tickets.levels();
        move |x, z| levels.entity_ticking_at(x, z)
    }
    /// Whether the chunk's own status is entity-ticking (block coordinates), whatever the
    /// simulation distance: the chunks whose entities are on `ServerLevel`'s entity tick list
    /// (`Visibility.fromFullChunkStatus`), so they check despawning.
    pub(super) fn entity_tick_listed_area(&self) -> impl Fn(i32, i32) -> bool + use<> {
        let levels = self.tickets.levels();
        move |x, z| levels.loaded_entity_ticking(x >> 4, z >> 4)
    }
    /// The entity phase of `tick_creatures`: every mob's tick, deaths, and the
    /// projectiles and effects that follow from them.
    pub(super) fn tick_creature_entities(&mut self) -> Result<()> {
        // Its steps as spans for `sunrise-report` (nothing is read while spans are off).
        let mut laps = sunrise_client::spans::laps();
        let blocked = self.mobs_below_off_level();
        let level_id = self.dim.id();
        // `EntityEvent.Size` for the sizes that changed outside the mob ticks (spawns,
        // commands, a player's interaction), before the ticks move by them.
        self.native_mob_sizes();
        // Hit-driven deaths already ran inside the hurt. Handle deaths from other
        // state changes before the mob ticks.
        let killed: Vec<Mob> = self
            .mobs
            .iter()
            .filter(|m| !m.alive() && !m.removed && !self.died.contains(&m.id))
            .cloned()
            .collect();
        for m in killed {
            self.died.insert(m.id);
            self.mob_die(&m);
        }
        // Their victims have been looted and mourned: no charged creeper is
        // waiting any more.
        // Resolve saved attacker UUIDs before the mobs run baseTick.
        self.resolve_last_hurt_by_refs();
        self.combat.charged_exploders.clear();
        self.combat.exploded.clear();
        self.combat.exploded_kinds.clear();
        // EntityTickEvent.Pre for the mobs about to tick, before the loop holds them: once for
        // the family, then each mob it let through checks its sleep. A mob of a type a mod
        // named hears its Pre alone, inside the loop, right before its own tick
        // (borrowed_tick.rs).
        let mut round = self.native_tick_round();
        let sleeping = mod_api::__private::listened(mod_api::HookId::CanContinueSleeping);
        if round.listening() || sleeping {
            let ticking = self.entity_ticking_area();
            let mut i = 0;
            while let Some(m) = self.mobs.get(i) {
                let (x, z) = (m.position.x.floor() as i32, m.position.z.floor() as i32);
                if !blocked.contains(&m.id) && self.world.loaded(x, z) && ticking(x, z) {
                    // A mob told alone is told, and checks its sleep, at its own turn.
                    if !round.alone_mob(&m.kind) {
                        if round.listening() {
                            let kind = super::native_ticks::TickKind::Mob(m.kind.clone());
                            round.add(m.id, kind);
                        } else {
                            self.sync_sleeping_mob(i);
                        }
                    }
                }
                i += 1;
            }
            self.native_tick_pre_batch(&mut round);
            let (mut n, mut from) = (0, 0);
            while sleeping && let Some(id) = round.let_through(n) {
                if let Some(i) = super::native_ticks::place(&self.mobs, from, |m| m.id == id) {
                    self.sync_sleeping_mob(i);
                    from = i + 1;
                }
                n += 1;
            }
        }
        self.init_sulfur_attribute_maps();
        let uuid = self.level.player_uuid;
        // LivingEntity.isPushable reads onClimbable live; a mob's cached one
        // is only refreshed by its own push step, so one that has not ticked
        // yet (the first tick after a load, a fresh spawn) or has moved since
        // would be pushed out of a ladder or scaffolding. Refreshed here, from
        // where each mob stands now, before the snapshot others push it by.
        let climbing: Vec<bool> = {
            let w = self.mob_world_base(&[]);
            self.mobs.iter().map(|m| m.on_climbable(&w)).collect()
        };
        for (m, climbing) in self.mobs.iter_mut().zip(climbing) {
            m.on_climbable = climbing;
        }
        let mut snapshot = std::mem::take(&mut self.creatures.seen_buf);
        snapshot.extend(self.mobs.iter().map(|m| seen(m, uuid, self.items())));
        let mut events = std::mem::take(&mut self.creatures.event_buf);
        events.clear();
        let passenger_objects = self.retained_passenger_snapshot(uuid);
        let mut pending = self.creatures.pending.remove(level_id).unwrap_or_default();
        let primable = self.sulfur_cubes_adopted_explosive();
        let mut mobs = self.mobs.lift();
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
        let mut dead = Vec::new();
        let mut corpses = Vec::new();
        let mut died = std::mem::take(&mut self.died);
        let mut ticked = std::mem::take(&mut self.creatures.ticked);
        ticked.clear();
        // With guests on, each mob sees its nearest player (game/network/lan_mobs.rs).
        let mut guests = self.lan_guest_views();
        // What stands in a guest's slot while a mob holds that guest's view for
        // its tick (see `lent` below).
        let mut spare = guests.first().map(|g| g.1.clone());
        let mut bindings = match self.lan.as_mut() {
            Some(host) if !guests.is_empty() => std::mem::take(&mut host.mob_players),
            _ => Default::default(),
        };
        // The guests' entity ids by connection, for the shulker bullets fired
        // at the guest a mob sees.
        let mut guest_ids: Vec<(u32, u32)> = match self.lan.as_ref() {
            Some(host) if !guests.is_empty() => host
                .hub
                .guests
                .iter()
                .map(|g| (g.conn, g.entity_id as u32))
                .collect(),
            _ => Vec::new(),
        };
        let mut guest_events = Vec::new();
        let mut player_credit = std::mem::take(&mut self.last_hurt_by_player);
        // The block writes since the last tick: mobs re-path around them.
        let mut path_writes = std::mem::take(self.world.path_writes.get_or_insert_with(Vec::new));
        // The mobs whose bodies go this tick: (id, feet, width, height, split).
        // 🤖 Agent: the last field is the grown slime, magma cube or sulfur
        // cube the body leaves behind (`AbstractCubeMob.remove`), captured as
        // the body goes because it has left the list by the time it splits.
        let mut gone = Vec::new();
        // The level's random and attachments, lent to the mobs (brain behaviours draw from
        // the random, and a hit's damage hooks get both) and taken back after their ticks.
        let lent = sunrise_client::health::LentLevel::take(&mut self.world);
        let (lent, dispatch_panic) = self.tick_borrowed_creatures(borrowed_tick::BorrowedMobTick {
            mobs: &mut mobs,
            snapshot: &mut snapshot,
            passenger_objects: &passenger_objects,
            blocked: &blocked,
            events: &mut events,
            pushes: &mut pending.pushes,
            dead: &mut dead,
            corpses: &mut corpses,
            died: &mut died,
            ticked: &mut ticked,
            guests: &mut guests,
            spare: &mut spare,
            bindings: &mut bindings,
            guest_ids: &mut guest_ids,
            guest_events: &mut guest_events,
            player_credit: &mut player_credit,
            path_writes: &mut path_writes,
            gone: &mut gone,
            lent,
            laps: &mut laps,
            uuid,
            level_id,
            round: &mut round,
        });
        if dispatch_panic.is_some() {
            // The aborted pass never applies retention, including previously
            // finished despawn answers. Do not carry those into the retry.
            for m in &mut mobs {
                m.despawn = mod_api::Despawn::default();
            }
        }
        lent.give_back(&mut self.world);
        self.last_hurt_by_player = player_credit;
        if let Some(host) = self.lan.as_mut()
            && !guests.is_empty()
        {
            // A set: a scan of every mob for each binding is quadratic, and a
            // server's loaded mobs run into the thousands.
            let live: sunrise_client::fast_hash::FxHashSet<u32> =
                mobs.iter().map(|m| m.id).collect();
            bindings.retain(|id, _| live.contains(id));
            host.mob_players = bindings;
        }
        // The mobs added during the tick (into the emptied `self.mobs`) come
        // first, as `append` put them; with none, the ticked list goes back
        // as it is instead of being copied into a new one, every mob a tick.
        self.mobs
            .restore(mobs, sunrise_client::join_seq::RestoreOrder::SpawnsFirst);
        if dispatch_panic.is_some() {
            // These death callbacks have not run, and the borrowed block writes
            // must still invalidate paths on the next attempted pass.
            for mob in &dead {
                died.remove(&mob.id);
            }
            self.world
                .path_writes
                .get_or_insert_with(Vec::new)
                .extend(path_writes);
        }
        self.died = died;
        snapshot.clear();
        self.creatures.seen_buf = snapshot;
        self.creatures.ticked = ticked;
        // Completed brain actions are never fed back into the producer pass.
        if !events.is_empty() || !guest_events.is_empty() {
            pending.batches.push_back(MobOutputBatch {
                normal: NormalMobActions::new(std::mem::take(&mut events)),
                guests: std::mem::take(&mut guest_events),
            });
        }
        if dispatch_panic.is_none() {
            // The successful final sweep consumed all pushes for surviving mobs.
            pending.pushes = Default::default();
        }
        self.creatures.pending.insert(level_id, pending);
        self.creatures.event_buf = events;
        if let Some(payload) = dispatch_panic {
            // Restoration may itself panic and replace this payload; a second
            // destructor panic may abort. panic=abort cannot recover at all.
            std::panic::resume_unwind(payload);
        }
        self.native_tick_post_batch(round);
        // ... and entity event 60 goes out as it is: the poof.
        // 🤖 Agent: the split follows the poof, as `LivingEntity.tickDeath`
        // broadcasts event 60 before it calls `remove`.
        for (id, at, width, height, split) in gone {
            self.finish_mob_death_animation(id, at, width, height, split);
        }
        laps.lap("creature_retain");
        self.sync_riders();
        self.tick_skeleton_traps();
        self.guardian_bubbles();
        self.villager_reputation_tick();
        self.villager_item_sense();
        self.villager_pickups();
        laps.lap("creature_villagers");
        for m in dead {
            self.mob_die(&m);
        }
        for mut m in corpses {
            let names: Vec<&'static str> = m
                .extra
                .effects
                .iter()
                .map(|e| e.id.trim_start_matches("minecraft:"))
                .collect();
            let bounds = m.bounds();
            self.effects_on_death(&names, bounds, m.silent, Some(m.id), Some(&mut m.random));
        }
        self.deliver_pending_mob_actions(level_id)?;
        self.finish_creature_entities(&mut laps, &primable)
    }

    fn deliver_pending_mob_actions(&mut self, level_id: &'static str) -> Result<()> {
        let mut pending = self.creatures.pending.remove(level_id).unwrap_or_default();
        let delivery = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            while let Some(batch) = pending.batches.front_mut() {
                self.creatures.deferred_hits = true;
                let result = self.handle_mob_events_pending(&mut batch.normal);
                self.creatures.deferred_hits = false;
                result?;
                self.lan_guest_mob_events(&mut batch.guests);
                self.creatures.event_buf = pending
                    .batches
                    .pop_front()
                    .unwrap()
                    .normal
                    .root
                    .events
                    .into();
            }
            if !pending.pushes.is_empty() {
                for mob in &mut self.mobs {
                    sunrise_client::mobs::apply_mob_pushes(mob, &mut pending.pushes);
                }
                // As in the producer's successful final sweep, removed targets
                // have no remaining owner to receive a push.
                pending.pushes = Default::default();
            }
            Ok(())
        }));
        self.creatures.pending.insert(level_id, pending);
        match delivery {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn finish_creature_entities(
        &mut self,
        laps: &mut sunrise_client::spans::Laps,
        primable: &[(u32, i32)],
    ) -> Result<()> {
        self.drain_entity_inside();
        laps.lap("creature_events");
        self.ordered_before(super::ordered_tick::Batch::Projectiles)?;
        self.tick_projectiles()?;
        self.drain_entity_inside();
        self.tick_fangs();
        laps.lap("creature_projectiles");
        self.tick_allays();
        self.tick_pandas();
        self.tick_foxes();
        self.tick_dolphins();
        self.tick_mob_looting();
        self.tick_breeze_effects();
        self.tick_sniffer_effects();
        self.piglin_pickups();
        self.sulfur_cube_pickups();
        self.sulfur_cube_power_priming(primable);
        laps.lap("creature_systems");
        // Hurt and death sounds of damage dealt outside the mob ticks.
        let mut queued = Vec::new();
        for m in &mut self.mobs {
            queued.append(&mut m.outbox);
        }
        self.handle_mob_events(queued)?;
        // The enderman teleports those hurts asked for (`Enderman.hurtServer`).
        self.settle_hurt_teleports()?;
        laps.lap("creature_outbox");
        // And for the sizes the ticks changed (a pose, a baby grown, a scale).
        self.native_mob_sizes();
        Ok(())
    }

    /// `LivingEntity.tickDeath` broadcasts the poof before `remove`, which
    /// also splits a large cube before it reaches `Entity.setRemoved`.
    fn finish_mob_death_animation(
        &mut self,
        id: u32,
        at: Vec3,
        width: f64,
        height: f64,
        split: Option<Mob>,
    ) {
        let at = at.components();
        self.lan_entity_event(id as i32, 60, at);
        self.death_poof(at, width, height);
        if let Some(mob) = split {
            self.split_dead_cube(mob);
        }
    }

    /// `LivingEntity.makePoofParticles` (entity event 60) about a mob's box.
    pub(super) fn death_poof(&mut self, at: [f64; 3], width: f64, height: f64) {
        let options = sunrise_client::particles::ParticleOptions::simple(
            sunrise_client::particles::ParticleType::Poof,
        );
        for _ in 0..20 {
            let r = self.particles.level_random();
            let xa = r.next_gaussian() * 0.02;
            let ya = r.next_gaussian() * 0.02;
            let za = r.next_gaussian() * 0.02;
            let x = at[0] + width * (2.0 * r.next_double() - 1.0) - xa * 10.0;
            let y = at[1] + height * r.next_double() - ya * 10.0;
            let z = at[2] + width * (2.0 * r.next_double() - 1.0) - za * 10.0;
            self.particles.add(&options, [x, y, z], [xa, ya, za]);
        }
    }
    /// Settle a hit while its damage owner still has the level in place.
    pub(in crate::game) fn finish_mob_damage_at(&mut self, index: usize) {
        let Some(mob) = self.mobs.get_mut(index) else {
            return;
        };
        let Some(hit) = mob.damage_completion.take() else {
            return;
        };
        let id = mob.id;
        // Keep the hit apart from the body: the death hooks may remove or revive it.
        let kind = hit.post.as_ref().map(|_| mob.kind.clone());
        if self.died.insert(id) {
            let death = mob.clone();
            self.mob_die(&death);
        }
        if let Some((post, kind)) = hit.post.zip(kind) {
            self.finish_damage_post(id as i32, &kind.id, post);
        }
    }

    /// `LivingEntity.die` / `dropAllDeathLoot`. Does not finish a hurt.
    pub(super) fn mob_die(&mut self, m: &Mob) {
        // Death reads the remembered player without consuming it. A body
        // still in the level can answer getKillCredit after this call.
        let source = (!mod_api::__private::mods().is_empty()).then(|| self.mob_damage_source(m));
        let killer = self
            .last_hurt_by_player
            .get(&m.id)
            .copied()
            .filter(|_| m.ai.last_hurt_by_player > 0)
            .and_then(|credit| self.resolve_player_credit(credit));
        if self.mob_death_canceled(m) {
            return;
        }
        // LivingEntity.die: a sleeper gets up, and its bed is free again.
        if let Some(bed) = sunrise_client::mobs::villager::sleeping_pos(m) {
            self.wake_villager(m.id);
            // The body may be gone already.
            self.set_bed_occupied(bed, false);
        }
        // TamableAnimal.die, before the rest of LivingEntity.die.
        self.tell_owner_pet_died(m, killer);
        self.love_cause.remove(&m.id);
        let by_player = m.ai.last_hurt_by_player > 0;
        let mut dying = m.clone();
        // LivingEntity.die: the death is heard, and a sculk catalyst nearby
        // takes the experience (before the loot is dropped). It takes what
        // would drop, whoever killed the mob. Experience the catalyst takes is
        // never dropped, so living_experience_drop does not hear of it.
        self.sculk_mob_died(m);
        let mut catalyst_xp = 0;
        let consumed =
            self.catalyst_takes_experience([m.position.x, m.position.y, m.position.z], |_| {
                catalyst_xp = if sunrise_client::mobs::drops::drops_experience(&dying) {
                    sunrise_client::mobs::drops::base_experience(&mut dying).max(0) as u32
                } else {
                    0
                };
                catalyst_xp
            });
        // CatalystListener: with experience to take, the player who last hurt
        // the mob gets KILL_MOB_NEAR_SCULK_CATALYST.
        if consumed && catalyst_xp > 0 {
            let who = match killer {
                Some(Some(conn)) if self.lan_acting == Some(conn) => {
                    Some(super::commands::Target::Player)
                }
                Some(Some(conn)) => Some(super::commands::Target::Guest(conn)),
                Some(None) if self.lan_acting.is_none() => Some(super::commands::Target::Player),
                _ => None,
            };
            if let Some(who) = who {
                self.for_player(who, |g| {
                    // CatalystListener: trigger(player, killedEntity, damageSource).
                    let view = g.mob_entity(m);
                    let blow = g.last_blow(m);
                    g.trigger_killed_as(
                        "minecraft:kill_mob_near_sculk_catalyst",
                        &view,
                        blow.as_ref(),
                    );
                });
            }
        }
        if let Some(killer) = killer {
            // Only the resolved player receives statistics and advancements.
            match killer {
                Some(conn) if self.lan_acting != Some(conn) => {
                    self.as_guest(conn, |g| g.credit_kill(m));
                }
                _ => self.credit_kill(m),
            }
        }
        // The body keeps its scores until it is removed (`ServerLevel.
        // EntityCallbacks.onDestroyed` → `Scoreboard.entityRemoved`, run by
        // `score_destroyed_entities`): LivingEntity.die does not reset them.
        // Raider.die's hero and Villager.tellWitnessesThatIWasMurdered go to
        // the causing entity, not the credited player: a tame wolf's kill is
        // neither its owner's heroism nor a murder by them.
        let slain_by_player = by_player && !killed_by_mob(m);
        self.raider_died(m, slain_by_player, killer.flatten());
        self.villager_murdered(m, slain_by_player, killer.flatten());
        let pos = [m.position.x, m.position.y + 0.5, m.position.z];
        let killer_entity = self.killer_loot_entity(killer.flatten());
        // LAST_DAMAGE_PLAYER only while the player is still in the world and
        // the hurt memory lasts (LivingEntity.dropFromLootTable). Experience
        // and equipment use the timer alone (dropExperience / playerKilled):
        // a guest who has left still counts.
        let player = killer
            .filter(|_| m.ai.last_hurt_by_player > 0)
            .and_then(|conn| self.killer_uuid(conn));
        if let Some(player) = player.filter(|_| m.extra.leash.is_some()) {
            self.lead_killers.insert(m.id, (player, self.dim));
        }
        self.drop_death_loot(
            &mut dying,
            player,
            m.ai.last_hurt_by_player > 0,
            killer_entity,
            consumed,
            pos,
            source,
        );
        self.finish_mob_death(m, &mut dying, source);
    }
    /// `AbstractCubeMob.remove`: split only as the dead body is removed.
    fn split_dead_cube(&mut self, mut dying: Mob) {
        if matches!(
            dying.species(),
            Species::Slime | Species::MagmaCube | Species::SulfurCube
        ) && dying.extra.size > 1
        {
            self.split_cube_children(&mut dying);
        }
    }
    /// The killer's side of a mob's death: `ServerPlayer.awardKillScore`'s
    /// statistics and scores, and the `player_killed_entity` trigger.
    fn credit_kill(&mut self, m: &Mob) {
        // Player.killedEntity: the killed statistic goes to the causing
        // entity, so a kill by a mob (a tame wolf credited to its owner)
        // leaves it out; ServerPlayer.awardKillScore counts the rest.
        if !killed_by_mob(m) {
            self.award_stat("killed", &m.kind.id, 1);
        }
        self.award_custom_stat("mob_kills", 1);
        self.score_player_killed(super::commands::Target::Mob(m.id));
        let view = self.mob_entity(m);
        let blow = self.last_blow(m);
        self.trigger_killed(&view, blow.as_ref());
    }
    /// The damage source a mob died to (`die`'s), its direct entity included when the mob
    /// recorded one ([`Mob::last_direct`]).
    fn last_blow(&self, m: &Mob) -> Option<sunrise_client::advancements::DamageView> {
        let (d, _) = m.last_damage?;
        let mut view = self.damage_view(d);
        if let Some(kind) = m.last_direct {
            view.direct = Some(sunrise_client::advancements::EntityView {
                kind: kind.into(),
                position: d.from.unwrap_or_default(),
                ..Default::default()
            });
            view.is_direct = false;
        }
        Some(view)
    }
    /// A mob's damage source as advancement predicates see it: its type, the
    /// causing entity, and the projectile when one dealt it.
    pub(in crate::game) fn damage_view(
        &self,
        d: Damage,
    ) -> sunrise_client::advancements::DamageView {
        let source = match d.attacker {
            // Its callers are KilledTrigger's: the source as their killing_blow sees it.
            Some(Who::Player) => Some(self.player_entity_for_any(&[
                "minecraft:player_killed_entity",
                "minecraft:kill_mob_near_sculk_catalyst",
            ])),
            Some(Who::Mob(id)) => self
                .mobs
                .iter()
                .find(|o| o.id == id)
                .map(|o| self.mob_entity(o)),
            None => None,
        };
        let projectile = match d.kind {
            DamageKind::Arrow => Some("minecraft:arrow"),
            DamageKind::Fireball => Some("minecraft:fireball"),
            DamageKind::WitherSkull => Some("minecraft:wither_skull"),
            DamageKind::Fireworks => Some("minecraft:firework_rocket"),
            _ => None,
        };
        let direct = match projectile {
            Some(kind) => Some(sunrise_client::advancements::EntityView {
                kind: kind.into(),
                position: d.from.unwrap_or_default(),
                ..Default::default()
            }),
            None => source.clone(),
        };
        sunrise_client::advancements::DamageView {
            kind: source_damage_type(d).to_owned(),
            // DamageSource.isDirect: the causing entity is the direct one.
            is_direct: projectile.is_none(),
            direct,
            source,
            ..Default::default()
        }
    }
    /// `dropAllDeathLoot` through `mobs::drops`: the entity loot table from the
    /// world's random sequence, equipment by drop chance, species extras and
    /// the experience orbs, spawned in order.
    /// The UUID of the player `last_hurt_by_player` keeps: the guest on
    /// connection `conn` (None once they left), else the player in place
    /// (None on a server, which has none of its own).
    /// The entity id of the player `killer_uuid` names for `conn`.
    pub(super) fn player_entity_id_by_connection(&self, conn: Option<u32>) -> Option<i32> {
        match conn {
            Some(conn) if self.lan_acting != Some(conn) => {
                Some(self.lan.as_ref()?.hub.guest(conn)?.entity_id)
            }
            Some(_) => Some(self.own_entity_id()),
            None if self.lan_acting.is_none() && self.player.is_some() => {
                Some(self.own_entity_id())
            }
            None => None,
        }
    }
    fn killer_uuid(&mut self, conn: Option<u32>) -> Option<u128> {
        match conn {
            // During that guest's own turn, the player in place has their UUID.
            Some(conn) if self.lan_acting != Some(conn) => {
                Some(self.lan.as_ref()?.hub.guest(conn)?.uuid)
            }
            Some(_) => Some(self.native_acting_id()),
            None if self.lan_acting.is_none() && self.player.is_some() => {
                Some(self.native_acting_id())
            }
            None => None,
        }
    }
    /// `LivingEntity.dropExperience`: the experience a dying mob or player is worth once
    /// the mods have had their say; 0 when they cancel it.
    pub(super) fn living_experience_drop(
        &mut self,
        entity: u32,
        kind: &str,
        attacker: Option<u128>,
        original: i32,
    ) -> i32 {
        let (mut amount, mut cancel) = (original, false);
        self.native_living_experience_drop(
            entity,
            kind,
            attacker,
            original,
            &mut amount,
            &mut cancel,
        );
        if cancel { 0 } else { amount.max(0) }
    }

    /// Adds the items a dying entity dropped to the level, each at its own
    /// position. With mods, they first hear the list (`native_living_drops`):
    /// edited or canceled.
    pub(super) fn drop_death_items(
        &mut self,
        dying: &DyingDrops<'_>,
        stacks: Vec<(ItemStack, [f64; 3], i32)>,
        pickup_delay: u32,
        mut velocity: impl FnMut(&mut Self) -> Option<[f64; 3]>,
    ) {
        let Some(source) = &dying.source else {
            for (stack, position, age) in stacks {
                let mut random = super::spill::prepare_item_random(None);
                random.state.age = age;
                let motion = velocity(self).unwrap_or(random.velocity);
                self.add_item_entity_prepared(
                    stack,
                    position,
                    Some(motion),
                    pickup_delay,
                    Some(random),
                );
            }
            return;
        };
        // Vanilla builds each item entity (and draws its motion) when it
        // collects the drop, before the event sees the list.
        let mut drops: Vec<super::spill::BlockItemDrop> = stacks
            .into_iter()
            .map(|(stack, position, age)| {
                let random = super::spill::prepare_item_random(None);
                super::spill::BlockItemDrop {
                    random: Some(random),
                    stack,
                    position,
                    velocity: velocity(self).unwrap_or(random.velocity),
                    pickup_delay,
                    state: super::spill::DropState {
                        age,
                        ..Default::default()
                    },
                }
            })
            .filter(|drop| drop.stack.count > 0 && &*drop.stack.item != "minecraft:air")
            .collect();
        let mut cancel = false;
        if mod_api::__private::listened(mod_api::HookId::LivingDrops)
            && let Ok(mut snapshots) = drops.try_into_api()
        {
            self.native_living_drops(mod_api::__private::LivingDropsCall {
                entity: dying.entity,
                kind: dying.kind,
                player: dying.player,
                source,
                recently_hit: dying.recently_hit,
                drops: &mut snapshots,
                cancel: &mut cancel,
            });
            if let Ok(mut next) = snapshots.try_into_engine(self.items()) {
                for (drop, original) in next.iter_mut().zip(&drops) {
                    drop.random = original.random;
                }
                drops = next;
            } else {
                cancel = false;
            }
        }
        if cancel {
            return;
        }
        for drop in drops {
            if drop.stack.count > 0 && &*drop.stack.item != "minecraft:air" {
                self.add_item_drop(drop);
            }
        }
    }
    /// Melee or ranged damage to another mob; whether the hit landed.
    pub(in crate::game) fn mob_hits_mob(
        &mut self,
        target: u32,
        amount: f32,
        mut source: Damage,
    ) -> bool {
        if source.attacker_species.is_none()
            && let Some(Who::Mob(id)) = source.attacker
        {
            source.attacker_species = self.mobs.iter().find(|m| m.id == id).map(|m| m.species());
        }
        let mut killed = false;
        let mut settle = false;
        let mut hurt = false;
        let mut infested = Vec::new();
        let catch_up = self.catch_up_countdowns(source.attacker, target);
        let aim = self.attacker_aim(source.attacker);
        // So `living_knockback` sees who hit and what they hold.
        let attack = self.attack_of_who(source.attacker);
        let plan = self.prepare_knockback_credit(target as i32, source.attacker, &attack);
        let mut credit = plan.lend(&mut self.last_hurt_by_player);
        let target_index = self.mobs.iter().position(|m| m.id == target);
        let attacker_index = match source.attacker {
            Some(Who::Mob(id)) if sunrise_client::health::hit_host_listened() => {
                self.mobs.iter().position(|m| m.id == id)
            }
            _ => None,
        };
        if let Some(ti) = target_index {
            // The attacker's own random is lent through a live borrow beside the victim's: a
            // mob attacker from the same list, the attacking player from its own field.
            let (t, lend) = match attacker_index.filter(|&ai| ai != ti) {
                Some(ai) => {
                    let [t, a] = self
                        .mobs
                        .get_disjoint_mut([ti, ai])
                        .expect("two different mobs");
                    (t, sunrise_client::health::HitLend::mob(a))
                }
                None => (
                    &mut self.mobs[ti],
                    sunrise_client::health::HitLend::attacking_player(
                        self.player.as_mut(),
                        attack.attacker,
                    ),
                ),
            };
            let previous_invulnerable = t.invulnerable;
            let hit = t.damage_aimed(
                crate::game::entity::damage_level_of!(self, &attack, &mut credit, lend),
                amount,
                source,
                aim,
            );
            hurt = hit;
            // Squid.hurtServer can report false after its superclass accepted a lethal hit.
            settle = t.damage_completion.is_some();
            if hit && catch_up && t.invulnerable > previous_invulnerable {
                t.hurt_time = (t.hurt_time - 1).max(0);
                t.invulnerable = (t.invulnerable - 1).max(0);
            }
            if hit {
                killed = !t.alive();
                // The hurt or death sound the mob raised.
                let center = t.center();
                for event in std::mem::take(&mut t.outbox) {
                    match event {
                        MobEvent::Sound(s, _, volume, pitch) => {
                            self.audio.play(&full_id(s), Some(center), volume, pitch);
                        }
                        MobEvent::Infested { at, velocities } => infested.push((at, velocities)),
                        other => {
                            // Left for the end of the tick.
                            t.outbox.push(other);
                        }
                    }
                }
            }
        }
        if settle && let Some(index) = target_index {
            self.finish_mob_damage_at(index);
            killed &= self
                .mobs
                .get(index)
                .is_some_and(|m| m.id == target && !m.alive());
        }
        for (at, velocities) in infested {
            self.spawn_infested(at, velocities);
        }
        if hurt
            && let Some(Who::Mob(attacker)) = source.attacker
            && let Some(m) = self
                .mobs
                .iter()
                .find(|m| m.id == attacker && m.species() == Species::Axolotl)
        {
            self.audio.play(
                "minecraft:entity.axolotl.attack",
                Some([m.position.x, m.position.y, m.position.z]),
                1.0,
                1.0,
            );
        }
        if killed && let Some(Who::Mob(killer)) = source.attacker {
            self.zombie_killed(killer, target);
        }
        self.settle_mob_hurt(target);
        hurt
    }
    /// Apply reactions that need the game in `hurtServer`, without waiting for
    /// a mob tick: a Creaking's heart response or a Warden's anger and targeting.
    /// Wardens can react even when the base hurt call returned false.
    pub(in crate::game) fn settle_mob_hurt(&mut self, id: u32) {
        use sunrise_client::mobs::{creaking, warden};

        let Some(index) = self.mobs.iter().position(|m| {
            m.id == id
                && (creaking::has_pending_hurt(m)
                    || m.extra
                        .warden
                        .as_ref()
                        .is_some_and(|st| st.has_pending_hurt()))
        }) else {
            return;
        };
        if self.mobs[index].species() == Species::Creaking {
            self.finish_creaking_hurt(index);
            return;
        }
        let mut events = Vec::new();
        let uuid = self.level.player_uuid;
        let others: Vec<_> = self
            .mobs
            .iter()
            .map(|m| seen(m, uuid, self.items()))
            .collect();
        let mut mob = self.mobs.swap_remove(index);
        {
            let world = self.mob_world_base(&others);
            warden::handle_hurt(&mut mob, &world, &mut events);
        }
        self.mobs.push(mob);
        let last = self.mobs.len() - 1;
        if index < last {
            self.mobs.swap(index, last);
        }
        if !events.is_empty() {
            let _ = self.handle_mob_events(events);
        }
    }

    pub(in crate::game) fn settle_creaking_hurt(&mut self, id: u32) {
        if let Some(index) = self.mob_index.find(&self.mobs, id)
            && sunrise_client::mobs::creaking::has_pending_hurt(&self.mobs[index])
        {
            self.finish_creaking_hurt(index);
        }
    }

    fn finish_creaking_hurt(&mut self, index: usize) {
        let mob = &mut self.mobs[index];
        let (id, join) = (mob.id, mob.join);
        let mut events = Vec::new();
        let reaction = sunrise_client::mobs::creaking::take_hurt(mob, &mut events);
        let _ = self.handle_mob_events(events);
        if let Some((home, by_player)) = reaction
            && self
                .mob_object(id)
                .is_some_and(|mob| mob.join == join && !mob.removed)
        {
            self.creaking_hurt(home, id, by_player);
        }
    }
    /// `LivingEntity.resolvePlayerResponsibleForDamage` after `source` hurt a mob, through the
    /// hit's credit loan (a tame wolf's hit is its owner's; an ownerless tame wolf clears it):
    /// what an accepted hit's site commits, for tests that set up the credit without a hit.
    #[cfg(test)]
    pub(super) fn resolve_player_responsible(&mut self, victim: u32, source: Damage) {
        if !matches!(source.attacker, Some(Who::Mob(_))) {
            return;
        }
        let plan =
            self.prepare_knockback_credit(victim as i32, source.attacker, self.combat.attack());
        let mut credit = plan.lend(&mut self.last_hurt_by_player);
        if let Some(mob) = self.mobs.iter_mut().find(|mob| mob.id == victim) {
            sunrise_client::health::KnockbackCredit::accepted_mob_hit(&mut credit, mob);
        }
    }
    /// Remember the acting player for a mob's `lastHurtByPlayer`.
    pub(super) fn remember_mob_player_credit(&mut self, victim: u32) {
        if let Some(uuid) = self.player_uuid_of(self.lan_acting) {
            self.last_hurt_by_player.insert(
                victim,
                super::combat::PlayerCredit(sunrise_client::lan::playerdata::ints_uuid(uuid)),
            );
        }
    }

    /// Resolve `EntityReference.getEntity` across levels. None inside means
    /// the player in place; Some is a guest connection.
    pub(super) fn resolve_player_credit(
        &self,
        credit: super::combat::PlayerCredit,
    ) -> Option<Option<u32>> {
        crate::game::entity::player_resolver!(self)
            .resolve(credit.0)
            .map(|guest| guest.or(self.lan_acting))
    }
    /// `LivingEntity.dropFromGiftLootTable`: the gift table from its random
    /// sequence with the mob as `this` (a chicken's egg depends on its
    /// variant); the loot data of the game's resources decides, and without it
    /// the items' table engine (no entity) rolls it. Whether anything dropped.
    fn drop_gift_loot(&mut self, mob: u32, table: &'static str, at: [f64; 3]) -> bool {
        use sunrise_client::mobs::drops;
        use sunrise_world::loot::EntityLootParams;
        let loot = self.server_loot.get();
        let this = self
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .map(|m| drops::loot_entity(m, None));
        let stacks = match (loot, this) {
            (Some(loot), Some(this)) if loot.has_table(table) => {
                let params = EntityLootParams {
                    this,
                    ..EntityLootParams::default()
                };
                let seed = self.level.seed;
                match loot.random_sequence(table) {
                    Some(id) => {
                        let random = self.rand_sequences.get(id, seed);
                        loot.roll_entity(table, random, &params)
                            .iter()
                            .filter(|s| !s.is_empty())
                            .map(from_loot)
                            .collect()
                    }
                    None => Vec::new(),
                }
            }
            _ => self
                .res
                .items()
                .loot_table(table, false, None, &mut self.world.random),
        };
        let dropped = !stacks.is_empty();
        for d in stacks {
            // The ItemEntity constructor's motion, from the entity's own
            // random (not the level's).
            self.spawn_item_own(d, at, 10);
        }
        dropped
    }
    /// Adds a projectile, giving it an id from the entity counter
    /// (`Entity.id`, the same counter as the rest) when it has none. Its id.
    pub(in crate::game) fn add_projectile(
        &mut self,
        mut p: sunrise_client::projectiles::Projectile,
    ) -> u32 {
        if p.id == 0 {
            self.next_entity += 1;
            p.id = self.next_entity;
        }
        // The UUID its own random made, unless it already has one (a client
        // copy's from the spawn packet).
        if let Some(uuid) = p.uuid.take() {
            self.persist.uuids.get_or_insert_with(p.id, || uuid);
        }
        // Projectile.setOwner: the acting player's or the mob's UUID, which
        // its save keeps even once the owner is gone.
        if p.owner_uuid.is_none() {
            p.owner_uuid = match p.owner {
                Some(Who::Player) => Some(self.acting_player_uuid()),
                Some(Who::Mob(id)) => Some(self.uuid_for(id)),
                None => None,
            };
        }
        let player = (p.owner == Some(Who::Player))
            .then(|| sunrise_client::lan::playerdata::ints_uuid(self.acting_player_uuid()));
        let name = if player.is_some() {
            self.acting_player_name().to_owned()
        } else {
            String::new()
        };
        let (id, kind) = (p.id, p.entity_type());
        if let Some(a) = p.arrow.as_mut()
            && !p.stuck
        {
            a.spawn_pos = Some(p.position);
            a.spawn_velocity = Some(p.velocity);
            a.spawn_rot = Some((p.pitch, p.yaw));
        }
        self.creatures.projectiles.push(p);
        self.native_projectile_launch(player, &name, id, kind);
        id
    }
    /// `MushroomCow.shear`'s `sendParticles(EXPLOSION, x, getY(0.5), z, 1, 0, 0, 0, 0)`:
    /// one puff over a sheared mooshroom, half its `height` up, shown here and to
    /// the players within range.
    pub(super) fn mooshroom_shear_puff(&mut self, at: [f64; 3], height: f64) {
        let at = [at[0], at[1] + 0.5 * height, at[2]];
        let options = ParticleOptions::simple(ParticleType::Explosion);
        self.particles
            .send_particles(&options, at, [0.0; 3], 0.0, 1);
        self.lan_level_particles(&options, at, [0.0; 3], 0.0, 1, None);
    }
    /// `Entity.sendBubbleColumnParticles` at the top of the column block `pos`: two
    /// splashes and two bubbles over it, which the level random places, shown here
    /// and to the players within range.
    pub(super) fn bubble_column_particles(&mut self, pos: BlockPos) {
        for _ in 0..2 {
            let r = &mut self.world.random;
            let splash = [
                f64::from(pos.x) + r.next_double(),
                f64::from(pos.y + 1),
                f64::from(pos.z) + r.next_double(),
            ];
            let bubble = [
                f64::from(pos.x) + r.next_double(),
                f64::from(pos.y + 1),
                f64::from(pos.z) + r.next_double(),
            ];
            let splash_options = ParticleOptions::simple(ParticleType::Splash);
            self.particles
                .send_particles(&splash_options, splash, [0.0; 3], 1.0, 1);
            self.lan_level_particles(&splash_options, splash, [0.0; 3], 1.0, 1, None);
            let bubble_options = ParticleOptions::simple(ParticleType::Bubble);
            self.particles
                .send_particles(&bubble_options, bubble, [0.0, 0.01, 0.0], 0.2, 1);
            self.lan_level_particles(&bubble_options, bubble, [0.0, 0.01, 0.0], 0.2, 1, None);
        }
    }
    /// A mob's projectile enters the level.
    fn launch_mob_projectile(&mut self, mut p: Projectile) {
        self.next_entity += 1;
        p.id = self.next_entity;
        let at = [p.position.x, p.position.y, p.position.z];
        // LlamaSpit.recreateFromPacket: the spit's burst on the client.
        if p.kind == ProjectileKind::LlamaSpit {
            let v = p.velocity;
            self.particles.llama_spit_burst(at, [v.x, v.y, v.z]);
        }
        self.spawn_mob_projectile(p);
    }
    /// `Entity.igniteForSeconds`: `seconds * 20` ticks of fire, scaled by the
    /// target's burning-time attribute.
    fn ignite_entity(&mut self, target: Who, seconds: i32) -> Result<()> {
        match target {
            Who::Player => {
                if !self.creative
                    && let Some(p) = self.player.as_mut()
                {
                    p.ignite_for_ticks(seconds * 20)?;
                }
            }
            Who::Mob(id) => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    m.ignite_for_ticks(seconds * 20);
                }
            }
        }
        Ok(())
    }
    pub(super) fn handle_mob_events(&mut self, mut events: Vec<MobEvent>) -> Result<()> {
        self.handle_mob_events_reusable(&mut events)
    }

    pub(in crate::game) fn handle_mob_events_reusable(
        &mut self,
        events: &mut Vec<MobEvent>,
    ) -> Result<()> {
        let mut actions = NormalMobActions::new(std::mem::take(events));
        let result = self.handle_mob_events_pending(&mut actions);
        *events = actions.root.events.into();
        result
    }

    fn handle_mob_events_pending(&mut self, actions: &mut NormalMobActions) -> Result<()> {
        loop {
            let frame = actions.nested.last_mut().unwrap_or(&mut actions.root);
            let Some(e) = frame.events.pop_front() else {
                if actions.nested.pop().is_some() {
                    continue;
                }
                return Ok(());
            };
            let actor = frame.actor;
            let broke = &mut frame.broke;
            match e {
                MobEvent::ForPlayer { conn, event } => {
                    let mut event = *event;
                    // Projectiles are spawned in the world rather than run
                    // as a guest, but must retain the selected guest target.
                    if let Some(conn) = conn
                        && let MobEvent::Projectile(p) = &mut event
                        && let ProjectileKind::ShulkerBullet { target, .. } = &mut p.kind
                        && *target == Some(Who::Player)
                    {
                        *target = self.guest_identity(conn).map(|(_, id)| Who::Mob(id));
                        self.handle_mob_events(vec![event])?;
                    } else if let Some(conn) = conn {
                        if let Some(mob) = actor {
                            self.lan_guest_mob_events(&mut vec![(conn, mob, event)]);
                        } else {
                            // No nested block-break actor is in scope. A
                            // missing guest must not redirect to another player.
                            if let Some(result) =
                                self.as_guest(conn, |game| game.handle_mob_events(vec![event]))
                            {
                                result?;
                            }
                        }
                    } else {
                        self.handle_mob_events(vec![event])?;
                    }
                }
                MobEvent::BubbleColumnParticles { pos } => {
                    self.bubble_column_particles(BlockPos::new(pos[0], pos[1], pos[2]))
                }
                MobEvent::LandingDust {
                    state,
                    at,
                    count,
                    speed,
                } => self.landing_dust(state, Vec3::new(at[0], at[1], at[2]), count, speed),
                MobEvent::DrinkFinished { mob, stack } => self.finish_mob_drink(mob, &stack),
                MobEvent::SquidCycle { mob, at } => self.lan_entity_event(mob as i32, 19, at),
                MobEvent::RabbitHop { mob, at } => self.lan_entity_event(mob as i32, 1, at),
                MobEvent::Drown { mob, at, bubbles } => self.mob_drown(mob, at, bubbles),
                MobEvent::PowderSnowParticle { at, velocity } => {
                    self.snowflake_particle(at, velocity);
                }
                MobEvent::SunProtectionWear { mob } => self.wear_sun_protection(mob),
                MobEvent::ArmorHurt {
                    mob,
                    amount,
                    source,
                } => self.hurt_mob_armor(mob, amount, &source),
                MobEvent::GliderWear { mob, slot } => self.wear_mob_glider(mob, slot),
                MobEvent::EntityEvent { mob, event, at } => {
                    // Ravager.roar (event 69 ends it): with the raw mob_griefing rule (not
                    // canEntityGrief), ROAR_TARGET_WITH_GRIEFING also takes the armor stands.
                    if event == 69
                        && self
                            .ui
                            .rules
                            .bool_at(sunrise_client::gamerules::vanilla::MOB_GRIEFING)
                        && let Some(area) = self
                            .mobs
                            .iter()
                            .find(|m| m.id == mob && m.species() == Species::Ravager)
                            .map(|m| m.bounds().inflate(Vec3::new(4.0, 4.0, 4.0)))
                    {
                        self.roar_pushes_armor_stands(area, at);
                    }
                    self.lan_entity_event(mob as i32, event, at)
                }
                MobEvent::LevelParticles {
                    kind,
                    at,
                    spread,
                    speed,
                    count,
                } => {
                    if let Some(ty) = sunrise_client::particles::ParticleType::from_name(kind) {
                        let options = sunrise_client::particles::ParticleOptions::simple(ty);
                        self.particles
                            .send_particles(&options, at, spread, speed, count);
                        self.lan_level_particles(&options, at, spread, speed, count, None);
                    }
                }
                // The LAN packet has no item for a particle yet: the host's window only.
                MobEvent::ItemParticles {
                    item,
                    at,
                    spread,
                    speed,
                    count,
                } => {
                    let options =
                        self.item_particle(&sunrise_client::items::ItemStack::new(item, 1));
                    self.particles
                        .send_particles(&options, at, spread, speed, count);
                }
                MobEvent::Sound(name, at, volume, pitch) => {
                    self.audio.play(&full_id(name), Some(at), volume, pitch)
                }
                MobEvent::ProfessionWorkSound(name, at, volume, pitch) => {
                    self.audio.play(&name, Some(at), volume, pitch)
                }
                MobEvent::LocalSound(name, at, volume, pitch) => {
                    self.audio
                        .play_unshared(&full_id(name), Some(at), volume, pitch)
                }
                MobEvent::BoundSound {
                    mob,
                    sound,
                    source,
                    at,
                    volume,
                    pitch,
                } => {
                    let name = full_id(sound);
                    let sound = LevelSound::new(&name, at, volume, pitch).in_source(source);
                    self.play_bound_sound(sound, mob, None);
                }
                MobEvent::LevelEventSound {
                    sound,
                    at,
                    volume,
                    spread,
                } => {
                    // WitherBoss.customServerAiStep: the spawn roar is global
                    // level event 1023 at the block position.
                    if sound == "entity.wither.spawn" {
                        let pos = at.map(|v| v.floor() as i32);
                        self.global_level_event(1023, pos, 0);
                    } else {
                        // The client's own level random: nothing drawn from the server's.
                        let mut r = sunrise_core::random::LegacyRandom::new(
                            self.world.game_time.wrapping_mul(341_873_128_712),
                        );
                        let pitch = (r.next_float() - r.next_float()) * spread + 1.0;
                        self.audio.play(&full_id(sound), Some(at), volume, pitch)
                    }
                }
                MobEvent::SoundId(name, at, volume, pitch) => {
                    self.audio.play(&name, Some(at), volume, pitch)
                }
                MobEvent::GameEvent {
                    event, at, source, ..
                } if event == "minecraft:flap"
                    && self.mobs.iter().any(|m| {
                        m.id == source && matches!(m.species(), Species::Chicken | Species::Parrot)
                    }) =>
                {
                    let ctx = self.who_context(Some(Who::Mob(source)), None);
                    self.post_game_event(event, at, ctx);
                }
                // A mob's own `gameEvent(..)` calls: Armadillo.rollUp/rollOut's ENTITY_ACTION,
                // PRIME_FUSE, EAT, a fox's DRINK, the shulker's lid, an evoker's vexes
                // (ENTITY_PLACE), TELEPORT, EQUIP/UNEQUIP, updateFallFlying's ELYTRA_GLIDE.
                MobEvent::GameEvent {
                    event, at, source, ..
                } if matches!(
                    event,
                    "minecraft:entity_action"
                        | "minecraft:prime_fuse"
                        | "minecraft:eat"
                        | "minecraft:drink"
                        | "minecraft:container_open"
                        | "minecraft:container_close"
                        | "minecraft:entity_place"
                        | "minecraft:teleport"
                        | "minecraft:equip"
                        | "minecraft:unequip"
                        | "minecraft:elytra_glide"
                ) =>
                {
                    let ctx = self.who_context(Some(Who::Mob(source)), None);
                    self.post_game_event(event, at, ctx);
                }
                MobEvent::GameEvent {
                    event: event @ ("minecraft:block_place" | "minecraft:block_destroy"),
                    at,
                    source,
                    state,
                } => {
                    let ctx = self.who_context(Some(Who::Mob(source)), Some(state));
                    self.post_game_event(event, at, ctx);
                }
                // `FarmlandBlock.turnToBaseBlock` and `TurtleEggBlock.decreaseEggs`:
                // the landing's own block change posts its game event, from the
                // mob that landed (`Context.of(entity, state)`) unless the block
                // destroyed itself under it (`Context.of(state)`).
                MobEvent::LandingGameEvent {
                    event,
                    at,
                    source,
                    state,
                } => {
                    let who = source.map(Who::Mob);
                    self.post_block_game_event(event, at, who, Some(state));
                }
                // The sculk layer tracks steps and bee/bat flaps itself
                // (`entity_motion_events`, `tick_mob_sculk_events`). Birds
                // above use their movement's flyDist/nextFlap threshold.
                MobEvent::GameEvent { .. } => {}
                MobEvent::AttackPlayer(amount, from) => {
                    self.combat.hit_landed =
                        self.mob_hits_player(amount, from, "minecraft:mob_attack");
                }
                // Bee.doHurtTarget: a sting that hurt leaves the stinger in the
                // body, then the poison; the bee's own stinger, calm-down and
                // sound follow only then too.
                MobEvent::Sting {
                    target,
                    amount,
                    poison_ticks,
                    source,
                } => self.sting(target, amount, poison_ticks, source),
                MobEvent::AttackPlayerWithEffect {
                    amount,
                    from,
                    effect,
                    ticks,
                    amplifier,
                } => {
                    self.combat.hit_landed =
                        self.mob_hits_player(amount, from, "minecraft:mob_attack");
                    if self.combat.hit_landed {
                        self.give_player_effect(effect, ticks, amplifier);
                    }
                }
                MobEvent::DamagePlayer { amount, from, kind } => {
                    self.mob_hits_player(amount, from, kind);
                }
                MobEvent::ElderCurse { at, silent } => self.elder_curse(at, silent),
                MobEvent::SetVelocity { target, velocity } => {
                    let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
                    match target {
                        Who::Player => {
                            if let Some(p) = self.player.as_mut() {
                                p.velocity = v;
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                m.velocity = v;
                            }
                        }
                    }
                }
                MobEvent::RemoveKilled { target } => {
                    // The bite that killed it ran `LivingEntity.die` (loot, such
                    // as a magma cube's froglight, and experience) before
                    // `eatEntity` removes it: a removed mob never reaches the
                    // next tick's deaths, so it dies here first.
                    if let Some(m) = self
                        .mobs
                        .iter()
                        .find(|m| m.id == target && !m.alive() && !m.removed)
                        .cloned()
                    {
                        if self.died.insert(target) {
                            self.mob_die(&m);
                        }
                        self.remove_mob_now(target, false);
                    }
                }
                MobEvent::Push { target, velocity } => {
                    let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
                    match target {
                        Who::Player => {
                            if let Some(p) = self.player.as_mut() {
                                p.velocity = p.velocity.add_vector(v);
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                m.velocity = m.velocity.add_vector(v);
                            }
                        }
                    }
                }
                MobEvent::PushJoined { id, join, velocity } => {
                    if let Some(mob) = self.mob_object_mut(id).filter(|mob| mob.join == join) {
                        mob.velocity = mob.velocity.add_vector(Vec3::new(
                            velocity[0],
                            velocity[1],
                            velocity[2],
                        ));
                        mob.needs_sync = true;
                    }
                }
                MobEvent::Knockback {
                    target,
                    power,
                    x,
                    z,
                    source,
                    apply_blocking,
                } => match target {
                    // Ram knockback checks blocking independently of the hurt call.
                    Who::Player => self.mob_pushes_player(power, x, z, source, apply_blocking),
                    Who::Mob(id) => {
                        let attack = self.attack_of_who(source.attacker);
                        let plan =
                            self.prepare_knockback_credit(id as i32, None, self.combat.attack());
                        let mut credit = plan.lend(&mut self.last_hurt_by_player);
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            let level = crate::game::entity::damage_level_of!(
                                self,
                                &attack,
                                &mut credit,
                                None
                            );
                            m.knockback(level, power, x, z, &source, 0.0);
                        }
                    }
                },
                MobEvent::LidPush {
                    shulker,
                    area,
                    delta,
                } => {
                    let (lid, delta) = (
                        Lid::Shulker(shulker),
                        Vec3::new(delta[0], delta[1], delta[2]),
                    );
                    // Use the same client push as a shulker box's lid.
                    if self.remote.is_some() {
                        self.lid_push_local(lid, area, delta);
                    } else {
                        self.lid_push(lid, area, delta);
                    }
                }
                MobEvent::Wololo { mob } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        // DyeColor.RED.
                        m.extra.color = 14;
                    }
                }
                MobEvent::Fangs {
                    at,
                    yaw,
                    delay,
                    owner,
                } => self.add_fangs(at, yaw, delay, owner),
                MobEvent::Shoot(..) => {}
                MobEvent::ExplosionSource {
                    mob,
                    charged,
                    name,
                    in_water,
                    kind,
                } => {
                    self.combat.explosion_source = Some(mob);
                    self.combat.explosion_in_water = in_water;
                    // The creeper causes the blast's damage, but it is out of
                    // the mob list already: its name comes with the event.
                    self.combat.exploded.push((mob, name));
                    self.combat.exploded_kinds.push((mob, kind));
                    if charged {
                        self.combat.charged_exploders.push(mob);
                    }
                }
                MobEvent::LingeringCloud {
                    at,
                    effects,
                    hidden,
                    ambient,
                    color,
                } => {
                    self.spawn_creeper_cloud(at, effects, hidden, ambient, color);
                }
                MobEvent::Explode(at, radius, species, silent) => {
                    // The exploding mob causes the damage (Creeper.explodeCreeper).
                    let noted = self.combat.explosion_source.is_some();
                    let source_mob = self.combat.explosion_source.or_else(|| {
                        self.mobs
                            .iter()
                            .filter(|m| {
                                (m.position.x - at[0]).powi(2) + (m.position.z - at[2]).powi(2)
                                    < 1.0
                            })
                            .map(|m| m.id)
                            .next()
                    });
                    self.combat.explosion_source = source_mob;
                    self.combat.explosion_source_named = noted;
                    let portal_proof = species == Species::SulfurCube
                        && source_mob.is_some_and(|id| {
                            self.portals
                                .entities
                                .get(&id)
                                .is_some_and(|p| p.cooldown > 0)
                        });
                    self.combat.explosion_cause = match species {
                        Species::Creeper => Some("minecraft:creeper"),
                        Species::Wither => Some("minecraft:wither"),
                        Species::SulfurCube => Some("minecraft:sulfur_cube"),
                        _ => None,
                    }
                    .map(|kind| sunrise_client::advancements::EntityView {
                        kind: kind.into(),
                        position: at,
                        ..Default::default()
                    });
                    // Only a creeper or a wither empties the hives it blows up.
                    // The creeper is already gone from the list, so the event
                    // carries its kind. A sulfur cube's blast is TNT's.
                    let mut source =
                        super::explosion::ExplosionSource::default().with_silence(silent);
                    if let Some(id) = source_mob {
                        source = source
                            .with_damage_direct(id, sunrise_client::health::DirectEntity::Living);
                    }
                    if let Some(id) = source_mob
                        && matches!(species, Species::Creeper | Species::Wither)
                    {
                        let kind = match species {
                            Species::Creeper => "minecraft:creeper",
                            _ => "minecraft:wither",
                        };
                        source = source.with_entity(
                            id,
                            kind,
                            sunrise_client::health::DirectEntity::Living,
                        );
                    }
                    let result = match species {
                        Species::Creeper | Species::Wither => self.with_hive_emptying_blast(|g| {
                            g.explode(source_mob, at, radius, source)
                        }),
                        Species::SulfurCube => self.explode_sulfur_cube(
                            source_mob,
                            at,
                            radius,
                            source.with_portal_proof(portal_proof),
                        ),
                        _ => self.explode(source_mob, at, radius, source),
                    };
                    self.combat.explosion_source = None;
                    self.combat.explosion_source_named = false;
                    self.combat.explosion_in_water = false;
                    self.combat.explosion_cause = None;
                    result?
                }
                MobEvent::AttackMob {
                    target,
                    amount,
                    source,
                } => {
                    self.combat.hit_landed = self.mob_attacks_mob(target, amount, source);
                }
                MobEvent::DamageMob {
                    target,
                    amount,
                    source,
                } => {
                    self.mob_hits_mob(target, amount, source);
                }
                MobEvent::AttackMobWithEffect {
                    target,
                    amount,
                    source,
                    effect,
                    ticks,
                    amplifier,
                } => {
                    // CaveSpider, Husk and WitherSkeleton.doHurtTarget: the
                    // effect sits inside the branch where the hit landed, and
                    // `LivingEntity.addEffect` takes a mob as well as a player.
                    self.combat.hit_landed = self.mob_attacks_mob(target, amount, source);
                    if self.combat.hit_landed
                        && let Some(t) = self.mobs.iter_mut().find(|m| m.id == target)
                    {
                        t.add_effect(effect, ticks, amplifier);
                    }
                }
                MobEvent::CubeHurtPlayer { cube, amount, from } => {
                    // AbstractCubeMob.dealDamage: the attack sound (and its
                    // two pitch draws, made even for a silent cube, whose
                    // Entity.playSound is then skipped) only follows a hit
                    // that landed.
                    if self.hurt_player_from(
                        amount,
                        Some(from),
                        "minecraft:mob_attack",
                        Some(Some(cube)),
                        false,
                    ) && let Some(m) = self.mobs.iter_mut().find(|m| m.id == cube)
                    {
                        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                        let at = m.center();
                        if !m.silent {
                            self.handle_mob_events(vec![MobEvent::Sound(
                                "entity.slime.attack",
                                at,
                                1.0,
                                pitch,
                            )])?;
                        }
                    }
                }
                MobEvent::CubeHurtMob {
                    cube,
                    target,
                    amount,
                    source,
                } => {
                    // AbstractCubeMob.dealDamage: the attack sound (and its
                    // two pitch draws, made even for a silent cube, whose
                    // Entity.playSound is then skipped) only follows a hit
                    // that landed.
                    if self.mob_hits_mob(target, amount, source)
                        && let Some(m) = self.mobs.iter_mut().find(|m| m.id == cube)
                    {
                        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                        let at = m.center();
                        if !m.silent {
                            self.handle_mob_events(vec![MobEvent::Sound(
                                "entity.slime.attack",
                                at,
                                1.0,
                                pitch,
                            )])?;
                        }
                        self.mob_post_attack(
                            target,
                            source.attacker,
                            None,
                            Some(super::creatures::damage_type(source.kind, false)),
                        );
                    }
                }
                MobEvent::SonicBoom {
                    target,
                    attacker,
                    from,
                    direction,
                } => {
                    // SonicBoom: the push only follows a hit that landed, scaled
                    // by the target's KNOCKBACK_RESISTANCE.
                    let (hurt, resistance) = match target {
                        Who::Player => {
                            let hurt = self.mob_hits_player(10.0, from, "minecraft:sonic_boom");
                            let resistance = self.player.as_mut().map_or(0.0, |p| {
                                p.attribute("minecraft:knockback_resistance").unwrap_or(0.0)
                            });
                            (hurt, resistance)
                        }
                        Who::Mob(id) => {
                            let source = Damage {
                                kind: DamageKind::SonicBoom,
                                attacker: Some(Who::Mob(attacker)),
                                from: Some(from),
                                attacker_species: Some(Species::Warden),
                            };
                            let hurt = self.mob_hits_mob(id, 10.0, source);
                            let resistance = self
                                .mobs
                                .iter()
                                .find(|m| m.id == id)
                                .map_or(0.0, super::lan_attributes::mob_knockback_resistance);
                            (hurt, resistance)
                        }
                    };
                    if hurt {
                        let horizontal = 2.5 * (1.0 - resistance);
                        let vertical = 0.5 * (1.0 - resistance);
                        let velocity = [
                            direction[0] * horizontal,
                            direction[1] * vertical,
                            direction[2] * horizontal,
                        ];
                        self.handle_mob_events(vec![MobEvent::Push { target, velocity }])?;
                    }
                }
                MobEvent::FinishAdmiring { mob } => self.finish_admiring(mob),
                MobEvent::Hiss { mob } => {
                    if let Some(m) = self.mobs.iter().find(|m| m.id == mob) {
                        let at = m.center();
                        let sound = sunrise_client::mobs::finalize::variant_sound(m, "hiss")
                            .unwrap_or("entity.cat.hiss");
                        self.audio.play(&full_id(sound), Some(at), 1.0, 1.0);
                    }
                }
                MobEvent::Tamed { .. }
                | MobEvent::Buck { .. }
                | MobEvent::StabDismount { .. }
                | MobEvent::EjectPassengers { .. } => self.mount_event(&e),
                MobEvent::IronGolemHit {
                    target,
                    amount,
                    source,
                } => {
                    // IronGolem.doHurtTarget: the toss only follows a hit that
                    // landed, scaled by the target's KNOCKBACK_RESISTANCE.
                    let hurt = match target {
                        Who::Player => {
                            let from = source.from.unwrap_or_default();
                            self.mob_hits_player(amount, from, "minecraft:mob_attack")
                        }
                        Who::Mob(id) => self.mob_hits_mob(id, amount, source),
                    };
                    if hurt {
                        let resistance = match target {
                            Who::Player => self.player.as_mut().map_or(0.0, |p| {
                                p.attribute("minecraft:knockback_resistance").unwrap_or(0.0)
                            }),
                            Who::Mob(id) => {
                                self.mobs.iter().find(|m| m.id == id).map_or(0.0, |m| {
                                    super::lan_attributes::mob_knockback_resistance(m)
                                })
                            }
                        };
                        let up = f64::from(0.4f32) * (1.0 - resistance).max(0.0);
                        match target {
                            Who::Player => {
                                if let Some(p) = self.player.as_mut() {
                                    p.velocity.y += up;
                                }
                            }
                            Who::Mob(id) => {
                                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                    m.velocity.y += up;
                                }
                            }
                        }
                        if let Who::Mob(id) = target {
                            self.mob_post_attack(
                                id,
                                source.attacker,
                                None,
                                Some("minecraft:mob_attack"),
                            );
                        }
                    }
                }
                MobEvent::Bonemeal { mob, pos } => {
                    self.villager_bonemeal(mob, BlockPos::new(pos[0], pos[1], pos[2]));
                }
                MobEvent::RingBell { bell } => {
                    let pos = BlockPos::new(bell[0], bell[1], bell[2]);
                    let rang = self
                        .with_level(|b, level, ticks| b.bell_attempt_to_ring(level, ticks, pos));
                    if rang {
                        let _ = self.process_level_events();
                    }
                }
                MobEvent::ChestOpen { chest, open, mob } => self.golem_chest_open(chest, open, mob),
                MobEvent::ChestSlot { chest, slot, stack } => {
                    self.golem_chest_slot(chest, slot, stack)
                }
                MobEvent::BecomeStatue { mob } => self.golem_become_statue(mob),
                MobEvent::EnterHive { mob, hive } => self.bee_enter_hive(mob, hive),
                MobEvent::ParrotToShoulder { mob } => self.parrot_to_shoulder(mob),
                MobEvent::Infested { at, velocities } => self.spawn_infested(at, velocities),
                MobEvent::Gift { target, item } => self.receive_gift(target, item),
                MobEvent::HurtSelf { mob, kind, amount } => {
                    let plan =
                        self.prepare_knockback_credit(mob as i32, None, self.combat.attack());
                    let mut credit = plan.lend(&mut self.last_hurt_by_player);
                    if let Some(index) = self.mob_index.find(&self.mobs, mob) {
                        self.mobs[index].damage(
                            crate::game::entity::damage_level_of!(
                                self,
                                self.combat.attack(),
                                &mut credit,
                                None
                            ),
                            amount,
                            sunrise_client::mobs::Damage::of(kind),
                        );
                        self.finish_mob_damage_at(index);
                    }
                }
                MobEvent::SetTarget { mob, target } => {
                    let player = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.mob_players.get(&mob).copied())
                        .or(self.lan_acting);
                    self.set_event_ai_target(mob, Some(target), player);
                }
                MobEvent::PiglinAlertTarget(alert) => self.piglin_alert_target(alert),
                MobEvent::HoglinAttackTarget {
                    mob,
                    target,
                    player,
                } => self.hoglin_attack_target(mob, target, player),
                MobEvent::HuntHoglin { mob, target, ticks } => {
                    let mut acquire = false;
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.ints[0] = ticks;
                        acquire = m.ai.target.is_none();
                    }
                    if acquire {
                        let player = self
                            .lan
                            .as_ref()
                            .and_then(|host| host.mob_players.get(&mob).copied())
                            .or(self.lan_acting);
                        self.set_event_ai_target(mob, Some(Who::Mob(target)), player);
                    }
                }
                MobEvent::Projectile(p) => self.launch_mob_projectile(p),
                MobEvent::BowShot {
                    mob,
                    weapon,
                    mut arrow,
                } => {
                    self.mob_bow_ammo(mob, &weapon, &mut arrow);
                    if let Some(state) = arrow.arrow.as_mut() {
                        state.fired_from_weapon = Some(weapon);
                        arrow.bonus.damage = 0.0;
                    }
                    self.launch_mob_projectile(arrow);
                }
                MobEvent::CrossbowShot { ammo, mut arrow } => {
                    if &*ammo.item == "minecraft:firework_rocket" {
                        // `CrossbowItem.createProjectile`: a rocket, from 0.15 under the eyes.
                        let mut at = arrow.position;
                        at.y -= f64::from(0.15f32) - 0.1;
                        self.launch_angled_rocket(at, &ammo, arrow.owner, |_| arrow.velocity);
                    } else {
                        Self::arrow_from_ammo(&ammo, &mut arrow);
                        self.launch_mob_projectile(arrow);
                    }
                }
                MobEvent::CrossbowLoad {
                    mob,
                    weapon,
                    at,
                    pitch,
                } => {
                    if self.mob_crossbow_ammo(mob, &weapon) {
                        self.audio.play(
                            "minecraft:item.crossbow.loading_end",
                            Some(at),
                            1.0,
                            pitch,
                        );
                    }
                }
                MobEvent::Spawn(mut m) => {
                    self.next_entity += 1;
                    m.id = self.next_entity;
                    self.mobs.push(*m);
                    self.native_mobs_joined(self.mobs.len() - 1, false);
                }
                MobEvent::SpawnKind { kind, at, setup } => {
                    if let Some(id) = self.spawn_mob(
                        &kind,
                        at,
                        setup.persistent,
                        if setup.skip_finalize {
                            SpawnSite::Raw
                        } else {
                            SpawnSite::Finalizing(
                                sunrise_client::mobs::spawn_rules::Reason::MobSummoned,
                            )
                        },
                    ) {
                        self.apply_setup(id, &setup);
                    }
                }
                MobEvent::Convert {
                    from,
                    kind,
                    params,
                    after,
                } => self.convert_event(&from, kind, params, after),
                MobEvent::ThrowItem {
                    mob,
                    stack,
                    at,
                    motion,
                    pickup_delay,
                } => {
                    // `thrownItem.setThrower(the mob)`.
                    self.spawn_item(stack, at, motion, pickup_delay);
                    let thrower = self.mob_uuid(mob);
                    if let Some(thrown) = self.just_spawned_item() {
                        thrown.thrower = Some(thrower);
                    }
                }
                MobEvent::DropStack { mob, stack, at } => {
                    self.native_entity_drop_item(mob, &stack.item, stack.count as i32, at);
                    self.spawn_item_own(stack, at, 10);
                }
                MobEvent::DropItem {
                    mob,
                    item,
                    count,
                    at,
                } => {
                    self.native_entity_drop_item(mob, &item, count as i32, at);
                    // The ItemEntity constructor's motion, from the entity's
                    // own random (not the level's), so spawnAtLocation drops
                    // (a fox's berries, a panda's held food, a goat's horn)
                    // land apart.
                    self.spawn_item_own(
                        ItemStack {
                            item,
                            count,
                            damage: 0,
                            extra: None,
                        },
                        at,
                        10,
                    );
                }
                MobEvent::DropGoatHorn {
                    mob,
                    screaming,
                    at,
                    motion,
                } => {
                    let instrument =
                        sunrise_client::mobs::goat::create_horn(self.mob_uuid(mob), screaming);
                    let mut horn = ItemStack::new("minecraft:goat_horn", 1);
                    horn.data_mut().set_other(
                        "minecraft:instrument",
                        Some(sunrise_core::nbt::Tag::String(instrument.into())),
                    );
                    // Goat.dropHorn's ItemEntity constructor leaves pickupDelay at
                    // 0 (no setDefaultPickUpDelay).
                    self.spawn_item(horn, at, motion, 0);
                }
                MobEvent::DropGift { table, at } => {
                    let stacks =
                        self.res
                            .items()
                            .loot_table(table, false, None, &mut self.world.random);
                    for stack in stacks {
                        // The ItemEntity constructor's motion, from the
                        // entity's own random (not the level's).
                        self.spawn_item_own(stack, at, 10);
                    }
                }
                // `Level.setBlockAndUpdate`: neighbour and shape updates, so a crop
                // above trampled farmland breaks and drops.
                MobEvent::SetBlock { pos, state } => self.level_set(pos, state)?,
                MobEvent::EndermanPlaceBlock {
                    mob,
                    pos,
                    carried,
                    occupants,
                } => {
                    self.enderman_place_block(mob, pos, carried, &occupants)?;
                }
                MobEvent::PlaceBlock { mob, pos, state } => {
                    self.mob_place_block(mob, pos, state)?
                }
                MobEvent::CropRaided {
                    pos,
                    state,
                    old,
                    mob,
                } => {
                    self.level_set_with_flags(
                        pos,
                        state,
                        sunrise_world::level::flags::UPDATE_CLIENTS,
                    )?;
                    if let Some(old) = old {
                        self.post_block_game_event(
                            "minecraft:block_change",
                            pos,
                            Some(Who::Mob(mob)),
                            None,
                        );
                        self.level_event_at(2001, pos, old.0 as i32);
                    }
                }
                MobEvent::CrackTurtleEgg { pos } => self.crack_turtle_egg(pos)?,
                MobEvent::BedOccupied { pos, occupied } => self.set_bed_occupied(pos, occupied),
                MobEvent::SetDoorOpen { pos, open } => {
                    let pos = BlockPos::new(pos[0], pos[1], pos[2]);
                    self.with_level(|b, level, ticks| b.set_door_open(level, ticks, pos, open));
                    self.process_level_events()?;
                }
                MobEvent::TakeBlock { mob, pos, state } => {
                    // The block must still be the one the enderman picked.
                    if self.world.block(pos[0], pos[1], pos[2]) == state
                        && self.mob_may_destroy(mob, pos)
                    {
                        let air = self.world.air;
                        self.level_set(pos, air)?;
                    } else if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.carried = None;
                    }
                }
                MobEvent::IfBroke {
                    mob,
                    effects,
                    otherwise,
                } => {
                    let follow = if broke.contains(&mob) {
                        effects
                    } else {
                        otherwise
                    };
                    actions.nested.push(NormalActionFrame {
                        events: follow.into(),
                        broke: Vec::new(),
                        actor: Some(mob),
                    });
                }
                MobEvent::Jump { mob } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.ai.jumping = true;
                    }
                }
                MobEvent::BreakDoor { mob, pos } => {
                    if !self.mob_may_destroy(mob, pos) {
                        continue;
                    }
                    // BreakDoorGoal.tick: removeBlock, then level events 1021
                    // and 2001 with the state that is there now.
                    let kind = self
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .map(super::native_hooks::mob_kind)
                        .unwrap_or_default();
                    let air = self.world.air;
                    let at = BlockPos::new(pos[0], pos[1], pos[2]);
                    self.with_level(|b, level, ticks| b.set_block(level, ticks, at, air, 3));
                    self.process_level_events()?;
                    self.level_event_at(1021, pos, 0);
                    let left = self.world.block(pos[0], pos[1], pos[2]);
                    self.level_event_at(2001, pos, left.0 as i32);
                    self.native_entity_break_door(mob, &kind, pos);
                }
                MobEvent::DestroyProgress { mob, pos, progress } => {
                    self.server_destroy_block_progress(mob as i32, pos, progress);
                }
                MobEvent::LevelEvent { id, pos, data } => self.level_event_at(id, pos, data),
                MobEvent::SetBlockName {
                    mob,
                    pos,
                    name,
                    effect,
                    flags,
                } => {
                    if effect {
                        // EatBlockGoal passes the block's default state constant
                        // (Blocks.GRASS_BLOCK.defaultBlockState()), not the live state
                        // (which could be snowy).
                        let old = self.world.block(pos[0], pos[1], pos[2]);
                        let state = self
                            .pack
                            .assets
                            .registry
                            .default_state(&self.res.facts().get(old).name)
                            .unwrap_or(old);
                        self.level_event_at(2001, pos, state.0 as i32);
                    }
                    if let Some(state) = self.pack.assets.registry.default_state(name) {
                        let old_name = self
                            .res
                            .facts()
                            .get(self.world.block(pos[0], pos[1], pos[2]))
                            .name
                            .clone();
                        let mut cancel = false;
                        if let Some(mob) = mob
                            && let Some(kind) = self
                                .mobs
                                .iter()
                                .find(|m| m.id == mob)
                                .map(|m| m.kind.id.clone())
                        {
                            let from = self.world.block(pos[0], pos[1], pos[2]);
                            self.native_entity_change_block_pre(
                                crate::game::native_hooks::EntityChangeBlockPreCall {
                                    player: None,
                                    entity: Some(mob),
                                    kind: &kind,
                                    pos,
                                    from,
                                    to: state,
                                    cancel: &mut cancel,
                                },
                            );
                        }
                        if !cancel {
                            self.level_set_with_flags(pos, state, flags)?;
                            if let Some(mob) = mob
                                && let Some(m) = self.mobs.iter().find(|m| m.id == mob)
                            {
                                let kind = m.kind.id.clone();
                                self.native_entity_change_block(mob, &kind, pos, &old_name, name);
                            }
                        }
                    }
                }
                MobEvent::Infest {
                    pos,
                    infested,
                    poof,
                } => {
                    let registry = &self.pack.assets.registry;
                    let old = self.world.block(pos[0], pos[1], pos[2]);
                    if let Some(mut state) = registry.state(old).cloned() {
                        let (from, to) = if infested {
                            ("minecraft:", "minecraft:infested_")
                        } else {
                            ("minecraft:infested_", "minecraft:")
                        };
                        state.name = state.name.replacen(from, to, 1);
                        if let Ok(id) = registry.resolve(&state) {
                            self.level_set(pos, id)?;
                        }
                    }
                    if let Some(mob) = poof {
                        self.mob_poof(mob);
                    }
                }
                MobEvent::BreakBlock {
                    pos,
                    drop,
                    by,
                    breaker,
                } => {
                    if let Some(mob) = by {
                        if !self.mob_may_destroy(mob, pos) {
                            continue;
                        }
                        broke.push(mob);
                    }
                    let old = self.world.block(pos[0], pos[1], pos[2]);
                    // Block.spawnDestroyParticles: fire breaks leave nothing.
                    if !self.res.facts().get(old).has_class("BaseFireBlock") {
                        self.level_event_at(2001, pos, old.0 as i32);
                    }
                    if drop {
                        let entity = by.or(breaker);
                        let key = (pos[0], pos[1], pos[2]);
                        // Level.destroyBlock binds its original subject before the roll.
                        // A writable loot listener can replace it before BlockDrops runs.
                        let data = if self.native_authoritative()
                            && mod_api::__private::listened(mod_api::HookId::BlockDrops)
                        {
                            match self.world.block_entities.get(&key) {
                                Some(row)
                                    if mod_api::__private::listened(mod_api::HookId::LootDrops) =>
                                {
                                    self.saved_block_entity_data(
                                        key,
                                        row,
                                        &self.res.facts().get(old).name,
                                    )
                                }
                                Some(_) => None,
                                None => super::spill::saved_entity(&self.world, pos)
                                    .and_then(super::spill::packed_block_entity_payload),
                            }
                        } else {
                            None
                        };
                        let block_entity =
                            super::spill::BlockDropEntity::at(&self.world, key, data.as_ref());
                        let mut drops = self.roll_block_loot(pos, old, entity, Default::default());
                        self.native_block_loot(pos, old, None, None, None, &mut drops);
                        let dropped =
                            self.entity_block_dropped(pos, old, entity, block_entity, drops);
                        let canceled = dropped.canceled;
                        self.spawn_block_drops(pos, dropped);
                        // InfestedBlock.spawnAfterBreak runs after the drops,
                        // unless BlockDropsEvent canceled it.
                        if !canceled
                            && self
                                .res
                                .facts()
                                .get(old)
                                .name
                                .starts_with("minecraft:infested_")
                            && self
                                .ui
                                .rules
                                .bool_at(sunrise_client::gamerules::vanilla::BLOCK_DROPS)
                        {
                            let at = [
                                f64::from(pos[0]) + 0.5,
                                f64::from(pos[1]),
                                f64::from(pos[2]) + 0.5,
                            ];
                            if let Some(id) =
                                self.spawn_mob("minecraft:silverfish", at, false, SpawnSite::Plain)
                            {
                                self.mob_poof(id);
                            }
                        }
                    }
                    // Level.destroyBlock: setBlock(fluidState.createLegacyBlock(), 3),
                    // so water stays water and a waterlogged block leaves its water.
                    let b = self.res.behaviours();
                    let left = b.legacy_block(b.fluid(old));
                    if left != old {
                        self.level_set(pos, left)?;
                    }
                }
                MobEvent::Particles { kind, at, count } => {
                    // Mob particle bursts (hearts, smoke, splashes) around a point.
                    if let Some(ty) = sunrise_client::particles::ParticleType::from_name(kind) {
                        if kind.starts_with("item_") {
                            self.ensure_item_particle_sprites();
                        }
                        let options = sunrise_client::particles::ParticleOptions::simple(ty);
                        if ty == sunrise_client::particles::ParticleType::SonicBoom {
                            self.lan_sonic_boom(&options, at);
                        }
                        // Count 0: one particle exactly there, at rest.
                        if count == 0 {
                            self.particles.add(&options, at, [0.0; 3]);
                        }
                        for _ in 0..count {
                            let r = &mut self.world.random;
                            let p = [
                                at[0] + (f64::from(r.next_float()) - 0.5),
                                at[1] + (f64::from(r.next_float()) - 0.5) * 0.5,
                                at[2] + (f64::from(r.next_float()) - 0.5),
                            ];
                            let v = [
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                            ];
                            self.particles.add(&options, p, v);
                        }
                    }
                }
                MobEvent::AroundSelf {
                    mob,
                    event,
                    kind,
                    feet,
                    width,
                    height,
                    burst,
                } => {
                    self.lan_entity_event(mob as i32, event, feet);
                    self.particles_around(kind, feet, [width, height], burst)
                }
                MobEvent::ParticleAt { kind, at, velocity } => {
                    if let Some(ty) = sunrise_client::particles::ParticleType::from_name(kind) {
                        let options = sunrise_client::particles::ParticleOptions::simple(ty);
                        self.particles.add(&options, at, velocity);
                    }
                }
                MobEvent::ParticleColored {
                    at,
                    color: [r, g, b],
                } => {
                    let options = sunrise_client::particles::ParticleOptions::colored(
                        sunrise_client::particles::ParticleType::EntityEffect,
                        [r, g, b, 1.0],
                    );
                    self.particles.add(&options, at, [0.0; 3]);
                }
                MobEvent::Experience { at, amount } => self.add_orb(at, amount),
                MobEvent::AxolotlSupportPlayer { mob } => {
                    let previous = self
                        .potions
                        .effects
                        .get("regeneration")
                        .map_or(0, |e| e.duration);
                    if (0..=2399).contains(&previous) {
                        self.potions
                            .effects
                            .add(sunrise_client::effects::EffectInstance::new(
                                "regeneration",
                                (previous + 100).min(2400),
                                0,
                            ));
                    }
                    self.potions
                        .effects
                        .list
                        .retain(|e| e.id != "mining_fatigue");
                    let source = self
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .map(|m| self.mob_entity(m));
                    self.trigger_effects_changed_by(source);
                }
                MobEvent::DolphinTreasure { mob } => {
                    // ServerLevel.findNearestMapStructure(#dolphin_located, 50).
                    let Some(at) = self.mobs.iter().find(|m| m.id == mob).map(|m| {
                        [m.position.x, m.position.y, m.position.z].map(|v| v.floor() as i32)
                    }) else {
                        continue;
                    };
                    let names = self.structure_tag("minecraft:dolphin_located");
                    let found = self
                        .end
                        .locator()
                        .filter(|_| !names.is_empty())
                        .and_then(|t| t.locate(&names, at, 50));
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.home = found.as_ref().map(|(_, p)| *p);
                    }
                    // DolphinSwimToTreasureGoal.start: entity event 38 once it has a treasure.
                    if found.is_some() {
                        self.mob_event_particles(mob, 38);
                    }
                }
                MobEvent::EffectPlayer {
                    effect,
                    ticks,
                    amplifier,
                    ambient,
                } => {
                    self.give_mob_effect(effect, ticks, amplifier, ambient);
                }
                MobEvent::EffectMob {
                    target,
                    effect,
                    ticks,
                    amplifier,
                } => {
                    if let Some(t) = self.mobs.iter_mut().find(|m| m.id == target) {
                        t.add_effect(effect, ticks, amplifier);
                    }
                }
                MobEvent::Ignite(target, seconds) => self.ignite_entity(target, seconds)?,
                MobEvent::ZombieIgnite(target, seconds) if self.combat.hit_landed => {
                    self.ignite_entity(target, seconds)?
                }
                MobEvent::ZombieIgnite(..) => {}
                MobEvent::Breed {
                    parent,
                    partner,
                    equine,
                    parent_variant,
                    wolf_voice,
                    experience,
                } => {
                    let child = self.breed(
                        parent,
                        partner,
                        equine,
                        parent_variant,
                        wolf_voice,
                        experience,
                    );
                    self.native_entity_breed(parent, partner, child);
                }
                MobEvent::LockGaze { mob, partner } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        sunrise_client::mobs::animal_brain::lock_gaze(m, partner);
                    }
                }
                MobEvent::BreedTarget { mob, partner } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        sunrise_client::mobs::animal_brain::set_breed_partner(m, partner);
                    }
                }
                MobEvent::Retreat { mob, from, at } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        sunrise_client::mobs::piglin::receive_retreat(m, from, at);
                    }
                }
                MobEvent::StartRiding { rider, vehicle } => {
                    if sunrise_client::mobs::riders::start_riding(&mut self.mobs, rider, vehicle) {
                        self.seat_mob(rider);
                        self.break_leash(rider, true, false);
                        // sendEventAndTriggers: the server players already on it.
                        self.note_started_riding(vehicle);
                    }
                }
                // The borrowed tick has already applied these fish events,
                // including settling each sting before the next event.
                MobEvent::StartFollowing { .. }
                | MobEvent::SchoolSize { .. }
                | MobEvent::PufferfishSting { .. } => {}
                MobEvent::LeashBreak { mob, drop, sound } => {
                    self.tick_break_leash(mob, drop, sound)
                }
                MobEvent::NotifyLeashHolder { holder } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == holder) {
                        sunrise_client::mobs::happy_ghast::notify_leash_holder(m);
                    }
                }
                MobEvent::Vibration {
                    from,
                    to,
                    ticks,
                    source,
                } => self.mob_vibration_particle(source, from, to, ticks),
                MobEvent::DigParticles { at } => self.warden_dig_particles(at),
                MobEvent::BlockBreakEffect { pos, block } => {
                    // Turtle.aiStep broadcasts LevelEvent 2001, not the
                    // particles themselves: the event carries the particles and
                    // the sand's break sound, and reaches the clients the same
                    // way a broken block's does.
                    let state = self.world.block(block[0], block[1], block[2]);
                    self.level_event_at(2001, pos, state.0 as i32);
                }
                MobEvent::Darkness { at, radius } => self.apply_darkness(at, radius),
                MobEvent::CreakingTearDown { center, spread } => {
                    self.creaking_tear_down(center, spread)
                }
                MobEvent::CureZombieVillager { mob } => self.cure_zombie_villager(mob),
                MobEvent::Reinforcements { mob, target } => self.call_reinforcements(mob, target),
                MobEvent::VillagerMeet { mob, other } => self.villager_meet(mob, other),
                MobEvent::VillagerTrade { mob, player } => self.villager_traded_by(mob, player),
                MobEvent::VillagerOffers { mob } => {
                    if let Some(i) = self.mobs.iter().position(|m| m.id == mob) {
                        self.ensure_offers(i);
                    }
                }
                MobEvent::VillagerBirth {
                    parent,
                    partner,
                    child,
                } => self.villager_birth(parent, partner, child),
                MobEvent::WorkAtComposter { mob, pos } => self.villager_work_at_composter(mob, pos),
                MobEvent::GolemCheck { mob, needed } => self.villager_golem_check(mob, needed),
                MobEvent::Firework { at, color, flight } => {
                    self.villager_firework(at, color, flight)
                }
                MobEvent::ThrowGift { table, from, to } => self.throw_gift(&table, from, to),
                MobEvent::PatrolTarget { mob, target } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        sunrise_client::mobs::raider::set_patrol_target(m, target);
                    }
                }
                MobEvent::RaiderTarget {
                    mob,
                    target,
                    aggressive,
                } => {
                    let player = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.mob_players.get(&mob).copied())
                        .or(self.lan_acting);
                    self.set_event_ai_target(mob, Some(target), player);
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.ai.aggressive |= aggressive;
                    }
                }
                MobEvent::PandaSneezed { mob } => self.panda_sneezed(mob),
                MobEvent::FallToPassengers {
                    mob,
                    distance,
                    modifier,
                } => self.fall_to_passengers(mob, distance, modifier),
                MobEvent::ShedScute { mob, at, pitch } => {
                    // The mob already checked `shouldDropLoot` (a baby, `mob_drops`).
                    let table = "minecraft:gameplay/armadillo_shed";
                    let drops = self.sequence_loot(table, table, None);
                    for d in drops {
                        self.spawn_at_location(Vec3::new(at[0], at[1], at[2]), 0.0, d);
                    }
                    self.audio.play(
                        "minecraft:entity.armadillo.scute_drop",
                        Some(at),
                        1.0,
                        pitch,
                    );
                    let ctx = self.who_context(Some(Who::Mob(mob)), None);
                    self.post_game_event("minecraft:entity_place", at, ctx);
                }
                MobEvent::LayEgg { mob, at, pitch } => {
                    // Chicken.aiStep: `dropFromGiftLootTable` says whether
                    // anything dropped; only then the sound and the game event.
                    if self.drop_gift_loot(mob, "minecraft:gameplay/chicken_lay", at) {
                        self.audio
                            .play("minecraft:entity.chicken.egg", Some(at), 1.0, pitch);
                        let ctx = self.who_context(Some(Who::Mob(mob)), None);
                        self.post_game_event("minecraft:entity_place", at, ctx);
                    }
                }
                MobEvent::GiftLoot { mob, table, at } => {
                    // Turtle.ageBoundaryReached and Panda.afterSneeze: only
                    // with mob loot on.
                    if matches!(
                        table,
                        "minecraft:gameplay/turtle_grow" | "minecraft:gameplay/panda_sneeze"
                    ) && !self
                        .ui
                        .rules
                        .bool_at(sunrise_client::gamerules::vanilla::MOB_DROPS)
                    {
                        continue;
                    }
                    self.drop_gift_loot(mob, table, at);
                }
            }
        }
    }
    /// `TurtleEggBlock.decreaseEggs` after an entity's step or landing won its
    /// roll (`destroyEgg`): the break sound and the remaining eggs (a stack of
    /// one is destroyed after it).
    pub(super) fn crack_turtle_egg(&mut self, pos: [i32; 3]) -> Result<()> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let eggs = self
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.name == "minecraft:turtle_egg")
            .and_then(|s| s.properties.get("eggs"))
            .and_then(|e| e.parse::<i32>().ok());
        if let Some(eggs) = eggs {
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| {
                b.turtle_egg_destroyed_by_player(level, ticks, at, state);
                if eggs <= 1 {
                    b.destroy_block(level, ticks, at, false);
                }
            });
            self.process_level_events()?;
        }
        Ok(())
    }
    /// `TurtleEggBlock.randomTick` at the last hatch stage: one baby turtle
    /// per egg, a day from grown, with the egg as its home (the sound, the
    /// crumbs and the block's removal came from the level).
    pub(super) fn hatch_turtles(
        &mut self,
        pos: [i32; 3],
        state: sunrise_world::registry::BlockStateId,
    ) {
        let eggs = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("eggs"))
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(1);
        for i in 0..eggs {
            let at = [
                f64::from(pos[0]) + 0.3 + f64::from(i) * 0.2,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.3,
            ];
            if let Some(id) = self.spawn_mob_raw("minecraft:turtle", at, true)
                && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
            {
                m.extra.age = -24000;
                m.extra.home = Some(pos);
                m.yaw = 0.0;
                m.body_yaw = 0.0;
                m.head_yaw = 0.0;
            }
        }
    }
    fn apply_setup(&mut self, id: u32, setup: &SpawnSetup) {
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if setup.baby {
            m.extra.age = -24000;
        }
        if setup.nausea {
            m.add_effect("minecraft:nausea", 200, 0);
        }
        if setup.size > 0 {
            m.extra.size = setup.size;
        }
        m.yaw = setup.yaw;
        m.body_yaw = setup.yaw;
        m.head_yaw = setup.yaw;
        m.velocity = Vec3::new(setup.velocity[0], setup.velocity[1], setup.velocity[2]);
        if let Some(h) = setup.health {
            m.health = h;
        }
        if let Some(v) = setup.variant {
            m.extra.variant = v;
        }
        if let Some(owner) = setup.owner_mob {
            m.extra.counter2 = owner as i32;
        }
        if let Some(home) = setup.home {
            m.extra.home = Some(home);
        }
        if let Some(life) = setup.life {
            m.extra.egg_time = life;
        }
    }
    /// `Animal.finalizeSpawnChildFromBreeding`'s single experience orb, drawn
    /// from the breeding parent after the rule check.
    fn award_breeding_experience(&mut self, parent: u32, at: [f64; 3]) {
        let mob_drops = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::MOB_DROPS);
        if let Some(amount) = self
            .mobs
            .iter_mut()
            .find(|m| m.id == parent)
            .and_then(|m| sunrise_client::mobs::breeding_experience(m, mob_drops))
        {
            self.add_orb(at, amount);
        }
    }
    fn breeding_orb(
        &mut self,
        parent: u32,
        at: [f64; 3],
        experience: sunrise_client::mobs::BreedExperience,
    ) {
        match experience {
            sunrise_client::mobs::BreedExperience::Deferred => {
                self.award_breeding_experience(parent, at)
            }
            sunrise_client::mobs::BreedExperience::Drawn(Some(amount)) => self.add_orb(at, amount),
            sunrise_client::mobs::BreedExperience::Drawn(None) => {}
        }
    }
    /// `ServerPlayer.tellNeutralMobsThatIDied`: the mobs within 32 blocks
    /// across and 10 up and down get `NeutralMob.playerDied`, when the
    /// `forgive_dead_players` rule is on. A mob's `Who::Player` is the guest
    /// it is bound to in `bindings` (game/network/lan_mobs.rs), else the host's
    /// player, so only the mobs whose binding is `conn` (`None` for the
    /// host's player) hear it.
    pub(in crate::game) fn tell_neutral_mobs_player_died(
        &mut self,
        feet: [f64; 3],
        conn: Option<u32>,
    ) {
        // A client of someone else's world has the server's mobs only to show.
        if self.remote.is_some()
            || !self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::FORGIVE_DEAD_PLAYERS)
        {
            return;
        }
        let block = feet.map(f64::floor);
        let area = Aabb::new(
            Vec3::new(block[0], block[1], block[2]),
            Vec3::new(block[0] + 1.0, block[1] + 1.0, block[2] + 1.0),
        )
        .inflate(Vec3::new(32.0, 10.0, 32.0));
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let ids: Vec<u32> = self
            .mobs
            .iter()
            .filter(|m| {
                host.mob_players.get(&m.id).copied() == conn
                    && m.bounds().intersects(area)
                    && sunrise_client::mobs::ai::forgives_dead_player(m)
            })
            .map(|m| m.id)
            .collect();
        for id in ids {
            let Some(epoch) = self.native_lifecycle.epoch() else {
                if let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) {
                    sunrise_client::mobs::ai::forgive_dead_player(mob);
                }
                continue;
            };
            let mob_ref = mod_api::MobRef::new(mod_api::EntityRef::new(epoch, id));
            let change = self
                .native_mob_target_event(mob_ref, None, false)
                .and_then(|target| self.native_mob_bind_target(id, target))
                .map_or(
                    sunrise_client::mobs::TargetChange::Keep,
                    sunrise_client::mobs::TargetChange::Set,
                );
            if let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) {
                sunrise_client::mobs::ai::forgive_dead_player_with_change(mob, change);
            }
        }
    }
    /// Animal.spawnChildFromBreeding
    fn breed(
        &mut self,
        parent: u32,
        partner: u32,
        equine: Option<Box<sunrise_client::mobs::equine::Offspring>>,
        parent_variant: Option<bool>,
        wolf_voice: Option<u8>,
        experience: sunrise_client::mobs::BreedExperience,
    ) -> Option<u32> {
        let p = self.mobs.iter().find(|m| m.id == parent).cloned()?;
        let q = self.mobs.iter().find(|m| m.id == partner).cloned();
        // Sniffer.spawnChildFromBreeding: an egg instead of a baby.
        if p.species() == Species::Sniffer {
            self.finalize_breeding(&p, partner, None, OrbOrder::BeforeChild);
            self.lay_sniffer_egg(&p);
            return None;
        }
        // TurtleBreedGoal.breed: the goal gave the mother her egg; only the
        // experience is left, and only with mob loot on.
        if matches!(p.species(), Species::Turtle | Species::Frog) {
            self.finalize_breeding(&p, partner, None, OrbOrder::BeforeChild);
            // One `new ExperienceOrb`, not split as `ExperienceOrb.award` does.
            self.award_breeding_experience(parent, [p.position.x, p.position.y, p.position.z]);
            return None;
        }
        let coin = if sunrise_client::mobs::draws_parent_variant(p.species()) {
            match parent_variant {
                Some(choice) => choice,
                None => {
                    let parent = self.mobs.iter_mut().find(|m| m.id == parent)?;
                    parent.random.next_boolean()
                }
            }
        } else {
            // Other species either draw their own variants or do not need a coin.
            false
        };
        // The baby's variant; a rabbit draws its own (Rabbit.getBreedOffspring).
        let rabbit_variant = self.bred_rabbit_variant(parent, &p, q.as_ref());
        // Axolotl.getBreedOffspring: useRareVariant (one in 1200) makes it blue,
        // else either parent's variant. The draws come from the mother's random.
        let axolotl = if p.species() == Species::Axolotl {
            let other = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            self.mobs.iter_mut().find(|m| m.id == parent).map(|m| {
                if m.random.next_int_bound(1200).unwrap_or(1) == 0 {
                    // Axolotl.Variant.getRareSpawnVariant: the one blue, still a draw.
                    let _ = m.random.next_int_bound(1);
                    4
                } else if m.random.next_boolean() {
                    p.extra.variant
                } else {
                    other
                }
            })
        } else {
            None
        };
        // MushroomCow.getOffspringVariant, from the mother's random: the other
        // colour 1 time in 1024 when both parents share one, otherwise either
        // parent's colour by a coin flip.
        let mooshroom = if p.species() == Species::Mooshroom {
            let mate = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            let own = p.extra.variant;
            self.mobs.iter_mut().find(|m| m.id == parent).map(|m| {
                if own == mate && m.random.next_int_bound(1024).unwrap_or(1) == 0 {
                    if own == 1 { 0 } else { 1 }
                } else if m.random.next_boolean() {
                    own
                } else {
                    mate
                }
            })
        } else {
            None
        };
        // A horse and a donkey have a mule.
        let kind = match (p.species(), q.as_ref().map(|q| q.species())) {
            (Species::Horse, Some(Species::Donkey)) | (Species::Donkey, Some(Species::Horse)) => {
                "minecraft:mule".to_owned()
            }
            _ => p.kind.id.clone(),
        };
        // Animal.spawnChildFromBreeding places the child at yaw zero. Its constructor still
        // spends LivingEntity's yaw draw on the child's stream, not the level's.
        let mut child = self.create_mob_reason(
            &kind,
            [p.position.x, p.position.y, p.position.z],
            true,
            SpawnSite::Raw,
            Some(0.0),
        )?;
        'offspring: {
            let child = &mut child;
            child.extra.age = -24000;
            // No Mob.finalizeSpawn either: no follow range bonus, right-handed.
            spawning::bred_offspring(child);
            // getBreedOffspring creates the baby without finalizeSpawn: it keeps
            // the default (`classic`) sound variant.
            child.extra.sound_variant = None;
            // Fox.getBreedOffspring runs no finalizeSpawn: nothing in the
            // baby fox's mouth.
            if p.species() == Species::Fox {
                child.extra.set_main_hand(None);
            }
            // Fox.FoxBreedGoal.breed: the baby trusts the players who fed its
            // parents into love.
            if p.species() == Species::Fox {
                let parent_cause = self
                    .love_cause
                    .get(&parent)
                    .copied()
                    .and_then(|conn| self.killer_uuid(conn));
                let partner_cause = self
                    .love_cause
                    .get(&partner)
                    .copied()
                    .and_then(|conn| self.killer_uuid(conn));
                sunrise_client::mobs::fox::trust_breeders(
                    child,
                    parent_cause.map(sunrise_client::lan::playerdata::uuid_ints),
                    partner_cause.map(sunrise_client::lan::playerdata::uuid_ints),
                );
            }
            // Goat.getBreedOffspring: GoatAi.initMemories, then the baby
            // screams if the parent a coin toss picks does, or on a 2% roll.
            if p.species() == Species::Goat {
                child.extra.lost_horns = [false; 2];
                sunrise_client::mobs::finalize::goat_init_memories(child, &mut self.world.random);
                let pick_parent = self.world.random.next_boolean();
                let parent_screams = if pick_parent {
                    p.extra.screaming
                } else {
                    q.as_ref().unwrap_or(&p).extra.screaming
                };
                child.extra.screaming = parent_screams || self.world.random.next_double() < 0.02;
            }
            // Inheritance was drawn synchronously in the breeding goal,
            // before the parent's later same-tick aiStep rolls.
            if let Some(o) = equine {
                sunrise_client::mobs::equine::apply_offspring(child, &o);
                break 'offspring;
            }
            // Camel.getBreedOffspring creates a camel with default attributes.
            if sunrise_client::mobs::camel::is_camel(p.species()) {
                break 'offspring;
            }
            if p.species() == Species::Cat {
                // Cat.getBreedOffspring.
                let other = q.as_ref().unwrap_or(&p);
                child.extra.variant = if coin {
                    p.extra.variant
                } else {
                    other.extra.variant
                };
                if p.extra.tame {
                    child.extra.tame = true;
                    child.extra.owner = p.extra.owner;
                    let mixed = dye_mix(
                        &self.loop_context,
                        world_recipes!(self),
                        &mut self.world.random,
                        p.extra.color,
                        other.extra.color,
                    );
                    child.extra.color = mixed;
                }
                break 'offspring;
            }
            if p.species() == Species::Wolf {
                // Wolf.getBreedOffspring: a parent's variant; tame parents
                // give a tame baby with their collar colours mixed.
                let other = q.as_ref().unwrap_or(&p);
                child.extra.variant = if coin {
                    p.extra.variant
                } else {
                    other.extra.variant
                };
                if p.extra.tame {
                    sunrise_client::mobs::tame::set_tame(child, true);
                    child.extra.owner = p.extra.owner;
                    let mixed = dye_mix(
                        &self.loop_context,
                        world_recipes!(self),
                        &mut self.world.random,
                        p.extra.color,
                        other.extra.color,
                    );
                    child.extra.color = mixed;
                }
                child.extra.sound_variant = wolf_voice.or_else(|| self.bred_wolf_voice(parent));
                break 'offspring;
            }
            // Panda.getBreedOffspring: genes from both parents.
            if p.species() == Species::Panda {
                sunrise_client::mobs::panda::inherit(child, &p, q.as_ref().unwrap_or(&p));
                break 'offspring;
            }
            // Axolotl.getBreedOffspring: the drawn variant, and the baby is persistent.
            if let Some(variant) = axolotl {
                child.extra.variant = variant;
                child.persistent = true;
                break 'offspring;
            }
            // Nautilus.getBreedOffspring: a tame parent's baby is tame too.
            if p.species() == Species::Nautilus && p.extra.tame {
                child.extra.tame = true;
                child.extra.owner = p.extra.owner;
                child.persistent = true;
            }
            // Offspring inherit a parent's variant (pigs/cows/chickens) or mixed wool.
            child.extra.variant = p.extra.variant;
            // Fox, Pig, Cow and Chicken.getBreedOffspring: either parent's variant.
            if sunrise_client::mobs::draws_parent_variant(p.species())
                && p.species() != Species::Cat
                && !coin
            {
                child.extra.variant = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            }
            // The rabbit's own draw above.
            if let Some(variant) = rabbit_variant {
                child.extra.variant = variant;
            }
            if let Some(variant) = mooshroom {
                child.extra.variant = variant;
            }
            // Sheep.getBreedOffspring: the lamb's wool mixes both parents' dyes.
            if p.species() == Species::Sheep {
                let other = q.as_ref().unwrap_or(&p);
                let mixed = dye_mix(
                    &self.loop_context,
                    world_recipes!(self),
                    &mut self.world.random,
                    p.extra.color,
                    other.extra.color,
                );
                child.extra.color = mixed;
            }
        }
        // BabyEntitySpawnEvent: the baby is constructed and has inherited (its draws are
        // spent); breeding is finalized and the baby inserted after the listeners.
        if mod_api::__private::listened(mod_api::HookId::BabyEntitySpawn) {
            let conn = self
                .love_cause
                .get(&parent)
                .or(self.love_cause.get(&partner))
                .copied();
            let player = (
                conn.and_then(|c| self.killer_uuid(c)),
                conn.and_then(|c| self.player_entity_id_by_connection(c)),
            );
            match self.breed_with_listeners(parent, partner, player, child, |game, baby| {
                game.finish_live_breeding(&p, partner, baby, experience)
            }) {
                // The levels' normal join hooks have now run if this was the root commit.
                Some(joined) => {
                    return joined.filter(|&id| {
                        self.resident_target_dimension(super::commands::Target::Mob(id))
                            == Some(self.dim)
                    });
                }
                None => {
                    breeding_credit(&mut self.love_cause, parent, partner);
                    if let Some(q) = self.mobs.iter_mut().find(|m| m.id == partner) {
                        q.extra.age = 6000;
                        q.extra.in_love = 0;
                    }
                    return None;
                }
            }
        }
        self.finish_live_breeding(&p, partner, child, experience)
    }

    fn finish_live_breeding(
        &mut self,
        p: &Mob,
        partner: u32,
        child: Mob,
        experience: sunrise_client::mobs::BreedExperience,
    ) -> Option<u32> {
        let parent = p.id;
        let order = if sunrise_client::mobs::orb_after_child(p.species()) {
            OrbOrder::AfterChild
        } else {
            OrbOrder::BeforeChild
        };
        self.finalize_breeding(p, partner, Some(&child), order);
        let at = [p.position.x, p.position.y, p.position.z];
        match order {
            OrbOrder::AfterChild => {
                let joined = self.insert_spawned_mob(child);
                self.mob_event_particles(parent, 18);
                self.breeding_orb(parent, at, experience);
                joined
            }
            OrbOrder::BeforeChild => {
                self.breeding_orb(parent, at, experience);
                self.insert_spawned_mob(child)
            }
        }
    }
    /// `MobEffectUtil.addEffectToPlayersAround` for an elder guardian:
    /// mining fatigue III for five minutes unless a stronger or longer one
    /// is on, with the curse sound and the elder's apparition.
    fn elder_curse(&mut self, at: [f64; 3], silent: bool) {
        let mut cursed = Vec::new();
        self.for_each_player(|g| {
            if g.elder_curse_player(at, silent) {
                cursed.push(g.lan_acting);
            }
        });
        // A guest's client shows the apparition and plays the curse itself
        // (ClientboundGameEventPacket.GUARDIAN_ELDER_EFFECT), the sound only
        // for 1: `isSilent() ? 0.0F : 1.0F`.
        if let Some(host) = self.lan.as_ref() {
            for conn in cursed.into_iter().flatten() {
                host.hub.send_to(
                    conn,
                    &sunrise_protocol::packets::play::Clientbound::GameEvent(
                        sunrise_protocol::packets::play::GameEvent {
                            event: sunrise_client::lan::hub::game_event::GUARDIAN_ELDER_EFFECT,
                            param: crate::mobs::guardian::elder_curse_param(silent),
                        },
                    ),
                );
            }
        }
    }
    /// The curse on the player in place; whether it took.
    fn elder_curse_player(&mut self, at: [f64; 3], silent: bool) -> bool {
        if self.creative || self.spectator() {
            return false;
        }
        let Some(p) = self.player.as_ref() else {
            return false;
        };
        let d = p.position.subtract(Vec3::new(at[0], at[1], at[2]));
        if d.x * d.x + d.y * d.y + d.z * d.z >= 2500.0 || p.health.dead_or_dying() {
            return false;
        }
        // The player's effects are kept by their bare name. A curse as strong
        // that won't end within the next minute stays (endsWithin(1199)).
        let active = self.potions.effects.get("mining_fatigue");
        if active.is_some_and(|e| e.amplifier >= 2 && !e.ends_within(1199)) {
            return false;
        }
        let at = [p.position.x, p.position.y, p.position.z];
        self.give_player_effect("minecraft:mining_fatigue", 6000, 2);
        if self.lan_acting.is_none() {
            // ClientPacketListener.handleGameEvent: the apparition always, the
            // curse sound unless the elder is silent.
            if crate::mobs::guardian::elder_curse_param(silent) == 1.0 {
                self.audio
                    .play("minecraft:entity.elder_guardian.curse", Some(at), 1.0, 1.0);
            }
            self.particles.elder_guardian();
        }
        true
    }
    /// `hurtServer` on the player. `from` is the source's position (none: no
    /// direction), `known`: the causing mob when it is given (`/damage`),
    /// else the melee attacker is the mob nearest to `from`.
    pub(super) fn hurt_player_from(
        &mut self,
        amount: f32,
        from: Option<[f64; 3]>,
        damage_type: &str,
        known: Option<Option<u32>>,
        by_creative_player: bool,
    ) -> bool {
        let post_attack_if_alive = std::mem::take(&mut self.combat.post_attack_if_alive);
        let rocket = std::mem::take(&mut self.combat.rocket_hit);
        let pierce = std::mem::take(&mut self.combat.piercing_hit);
        let melee = self.combat.melee_mob.take();
        // A projectile entity dealt it: `#is_projectile`, or a firework
        // rocket (its `fireworks` type is not in the tag).
        let projectile = rocket
            || world_stations!(self)
                .enchantments
                .damage_tags
                .get("minecraft:is_projectile")
                .is_some_and(|t| t.contains(damage_type));
        let blocking = self.player_blocking(from, damage_type, pierce);
        let raised = blocking.is_some();
        // Melee: the attacker stands at `from` (another player's hit has no mob behind it).
        let attacker = match known {
            Some(known) => known,
            None if melee.is_some() => melee,
            None => from
                .filter(|_| !projectile && damage_type != "minecraft:player_attack")
                .and_then(|from| {
                    self.mobs
                        .iter()
                        .filter(|m| m.alive())
                        .min_by(|a, b| {
                            let d = |m: &sunrise_client::mobs::Mob| {
                                (m.position.x - from[0]).powi(2) + (m.position.z - from[2]).powi(2)
                            };
                            d(a).total_cmp(&d(b))
                        })
                        .map(|m| m.id)
                }),
        };
        // The causing entity for the death message.
        let owner = self.combat.projectile_owner.take();
        // The mob that caused the hit, else the attack the caller landed it in (a projectile's
        // owner, another player's blow): `combat.attack`, read through `current_attack`.
        let mut attack = match attacker.or(owner) {
            Some(id) => self.attack_of(id as i32),
            None => self.current_attack(),
        };
        attack.projectile = self.combat.attack().projectile;
        attack.projectile_kind = self.combat.attack().projectile_kind;
        attack.direct = self.combat.attack().direct;
        attack.direct_kind = self.combat.attack().direct_kind;
        attack.position = from;
        attack.projectile_weapon = self.combat.attack().projectile_weapon.clone();
        if let Some(a) = attacker.or(owner) {
            self.note_attacker(damage_type, a);
        }
        if let Some((id, name)) = self.combat.projectile_attacker.take() {
            self.note_named_attacker(damage_type, id, name);
        }
        // The projectile is the direct entity: named when it has no owner.
        if let Some(name) = self.combat.projectile_direct.take() {
            self.note_direct_entity(damage_type, name);
        }
        // DamageSource.scalesWithDifficulty: a mob (or its projectile's
        // owner) caused the hit, not a player.
        let living_cause = attacker.or(owner).is_some();
        // The native damage hook's causing entity: the one given, or the
        // owner of the projectile that hit (`projectile_owner` outlives a
        // projectile's other hits), not the guessed melee attacker.
        let owner = owner.filter(|_| projectile);
        let causing = known.map_or_else(
            || attacker.or(owner).map_or(attack.attacker, |id| id as i32),
            |known| known.map_or(0, |id| id as i32),
        );
        let mut outcome = sunrise_client::health::BlockOutcome::default();
        // The hit's own push, held back until after the shield's (`hurtServer`
        // runs `applyItemBlocking` before `dealDefaultKnockback`).
        let mut default_push: Option<([f64; 2], HurtSource<'_>)> = None;
        let events_before = self.player.as_ref().map_or(0, |p| p.events.len());
        let damage_outcome = if let Some(id) = self.health_resources.damage.id(damage_type) {
            let weapon = self.player_armor_weapon(attacker, projectile);
            self.hurt_player_equipment(
                id,
                amount,
                blocking,
                living_cause,
                causing,
                by_creative_player,
                &attack,
                weapon,
            )
        } else {
            Ok(Default::default())
        };
        if let Ok(o) = damage_outcome
            && let Some(player) = self.player.as_mut()
            && {
                outcome = o;
                o.hurt
            }
        {
            // LivingEntity.hurtServer's dealDefaultKnockback: not inside the
            // hurt cooldown (a stronger hit there only adds its difference).
            if o.knockback {
                // No source position: a random direction.
                let (xd, zd) = from.map_or((0.0, 0.0), |from| {
                    (from[0] - player.position.x, from[2] - player.position.z)
                });
                // dealDefaultKnockback's `indicateDamage`, unless blocked.
                if o.blocked <= 0.0 {
                    player.health.indicate_damage(xd, zd, player.yaw);
                }
                let source = HurtSource {
                    damage_type,
                    causing,
                    direct: attack.projectile_direct(damage_type),
                    position: None,
                };
                default_push = Some(([xd, zd], source));
            }
        }
        // LivingEntity.playSecondaryHurtSound: a thorns hit that landed whole
        // sounds where the player stands, for everyone around.
        if let Some(p) = self.player.as_ref()
            && p.events.get(events_before..).is_some_and(|new| {
                new.iter().any(|e| {
                    matches!(
                        e,
                        sunrise_client::motion::MotionEvent::Health(HealthEvent::Phase(
                            "thorns-sound"
                        ))
                    )
                })
            })
        {
            let at = [p.position.x, p.position.y, p.position.z];
            self.audio
                .play("minecraft:enchant.thorns.hit", Some(at), 1.0, 1.0);
        }
        // LivingEntity.hurtServer: ENTITY_HURT_PLAYER, unless it returned
        // early (invulnerable, or inside the hurt cooldown).
        // A hit a native mod canceled records no damage (hurtServer never ran).
        self.combat.forget_noted(outcome.canceled);
        // Player motion queues accepted-hit memory events; the normal drain runs after
        // this function returns. A knockback listener must see those memories already
        // committed, including absorbed/blocked hits that still reach Criterion.
        if mod_api::__private::listened(mod_api::HookId::LivingKnockback)
            && self.player.as_ref().is_some_and(|player| {
                player.events[events_before..].iter().any(|event| {
                    matches!(
                        event,
                        sunrise_client::motion::MotionEvent::Health(HealthEvent::Criterion { .. })
                    )
                })
            })
        {
            self.commit_pending_hurt_memories(damage_type, Some(&attack));
        }
        if outcome.hurt || outcome.sound {
            let source = attacker
                .or(owner)
                .and_then(|id| self.mob_index.get(&self.mobs, id))
                .map(|m| self.mob_entity(m));
            self.trigger_entity_hurt_player(&sunrise_client::advancements::DamageView {
                kind: damage_type.to_owned(),
                // Arrows and the like name no direct entity here; a
                // rocket's burst names the rocket.
                direct: if rocket {
                    Some(sunrise_client::advancements::EntityView {
                        kind: "minecraft:firework_rocket".to_owned(),
                        position: from.unwrap_or_default(),
                        ..Default::default()
                    })
                } else {
                    source.clone().filter(|_| !projectile)
                },
                is_direct: !projectile,
                source,
                original: amount,
                actual: outcome.dealt,
                blocked: outcome.blocked > 0.0,
            });
        }
        if raised {
            let source = HurtSource {
                damage_type,
                causing,
                direct: attack.projectile_direct(damage_type),
                position: None,
            };
            self.after_block(outcome, (attacker, projectile), source, &attack);
        }
        // `LivingEntity.dealDefaultKnockback`: after the item's block push, so
        // a half-blocked hit pushes the defender back the way vanilla does.
        if let Some(([xd, zd], source)) = default_push {
            self.knock_back_player(f64::from(0.4_f32), xd, zd, source, &attack, amount);
            if let Some(p) = self.player.as_mut()
                && let Some(r) = p.hurt_log.last_mut()
            {
                r.knockback = Some([p.velocity.x, p.velocity.y, p.velocity.z]);
            }
        }
        self.finish_guest_damage();
        // The attack runs `EnchantmentHelper.doPostAttackEffects` after `hurtServer` returns.
        let killed = self.player.as_ref().is_some_and(|p| {
            super::wither::killed(true, p.health.dead_or_dying(), p.health.totem_used)
        });
        if outcome.hurt
            && let Some(a) = attacker.or(owner)
            && !(post_attack_if_alive && killed)
            && damage_type != "minecraft:thorns"
        {
            self.thorns(a);
        }
        outcome.hurt
    }
    /// A status effect from a mob, arrow or splash potion (`addEffect`).
    pub(in crate::game) fn give_player_effect(
        &mut self,
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
    ) {
        self.give_mob_effect(effect, ticks, amplifier, false);
    }
    /// A mob's effect on the player. Creative and spectator players shrug off
    /// the damaging effects here, but the nautilus's ambient Breath is plain
    /// `addEffect` and lands in every game mode.
    fn give_mob_effect(&mut self, effect: &'static str, ticks: i32, amplifier: i32, ambient: bool) {
        let immune = self.creative || self.spectator();
        if let Some(instance) = mob_effect_instance(effect, ticks, amplifier, ambient, immune) {
            self.apply_player_effect(instance);
        }
    }
    /// `WitherRoseBlock.entityInside` for the player: on a server level that
    /// isn't peaceful, `getBeeInteractionEffect` (Wither, 40 ticks) unless the
    /// player is invulnerable to wither damage (creative, spectator). A
    /// server's player gets it from the server.
    pub(super) fn wither_rose_contact(&mut self) {
        if self.remote.is_some() || self.ui.difficulty == 0 {
            return;
        }
        self.give_player_effect("minecraft:wither", 40, 0);
    }
    pub(in crate::game) fn hurt_player_by(&mut self, damage_type: &str, amount: f32) {
        let mut canceled = false;
        let attack = self.current_attack();
        if self.player.is_some()
            && let Some(id) = self.health_resources.damage.id(damage_type)
            && let Ok(o) =
                self.hurt_player_equipment(id, amount, None, false, 0, false, &attack, None)
        {
            canceled = o.canceled;
        }
        self.combat.forget_noted(canceled);
        self.finish_guest_damage();
    }
    /// `Player.aiStep` touching arrows in its box grown by (1, 0.5, 1):
    /// `AbstractArrow.playerTouch`, then `Player.take`'s pickup sound at the
    /// arrow (`ClientPacketListener.handleTakeItemEntity`).
    pub(super) fn touch_arrows(&mut self) {
        use sunrise_client::arrows::Touch;
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if p.health.dead_or_dying() || self.spectator() {
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let reach = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        )
        .inflate(Vec3::new(1.0, 0.5, 1.0));
        // Player.hasInfiniteMaterials.
        let creative = self.creative;
        let mut projectiles = self.creatures.projectiles.lift();
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
        let mut picked = Vec::new();
        projectiles.retain(|shot| {
            let Some(a) = shot.arrow.as_ref().filter(|a| !a.removed) else {
                return true;
            };
            let at = shot.position;
            let bounds = Aabb::new(
                Vec3::new(at.x - 0.25, at.y, at.z - 0.25),
                Vec3::new(at.x + 0.25, at.y + 0.5, at.z + 0.25),
            );
            if !bounds.intersects(reach) {
                return true;
            }
            let taken = match a.player_touch(creative) {
                Touch::Ignore => false,
                Touch::Take => true,
                // Inventory.add: a creative player takes what does not fit.
                Touch::Give(stack) => self.add_stack_to_inventory(stack).is_none() || creative,
            };
            if taken {
                let r = &mut self.world.random;
                let pitch = (r.next_float() - r.next_float()) * 1.4 + 2.0;
                self.audio.play(
                    "minecraft:entity.item.pickup",
                    Some([at.x, at.y, at.z]),
                    0.2,
                    pitch,
                );
                picked.push(shot.clone());
            }
            !taken
        });
        self.creatures.projectiles.restore(
            projectiles,
            sunrise_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
        for shot in picked {
            self.add_pickup(
                super::pickups::Taken::Arrow(Box::new(shot)),
                super::pickups::Collector::Player,
            );
        }
    }

    /// Hands the tick's lifted projectiles back for a hook told about one of them, so its handles
    /// read the live rows, and takes them again. Whether they came back as they went: the same
    /// rows in the same order, unwritten (the list's write stamp), with no spawn of the tick put
    /// before them.
    fn projectiles_told(
        &mut self,
        projectiles: sunrise_client::join_seq::JoinList<Projectile, false>,
        tell: impl FnOnce(&mut Self),
    ) -> (sunrise_client::join_seq::JoinList<Projectile, false>, bool) {
        let no_spawns = self.creatures.projectiles.is_empty();
        self.creatures.projectiles.restore(
            projectiles,
            sunrise_client::join_seq::RestoreOrder::SpawnsFirst,
        );
        let restored = self.creatures.projectiles.stamp();
        tell(self);
        let same = no_spawns && self.creatures.projectiles.stamp() == restored;
        (self.creatures.projectiles.lift(), same)
    }

    pub(super) fn tick_projectiles_now(&mut self) -> Result<()> {
        if self.creatures.projectiles.is_empty() {
            self.flush_lan_explosions();
            return self.flush_wind_bursts();
        }
        let ticking = self.entity_ticking_area();
        // `EntityTickList.forEach`: a projectile that joined during this
        // tick's entity pass (a mob's shot) first ticks on the next one; a
        // player's or a dispenser's, which join before the pass, tick now.
        let through = self.ticked_entities_through;
        // EntityTickEvent.Pre for the projectiles about to tick, once for the family before the
        // loop holds them, and the Post once the hits of their ticks have landed. A projectile
        // of a type a mod named is told alone, right before and right after its own tick.
        let mut round = self.native_tick_round();
        let mut alone_any = false;
        if round.listening() {
            for p in self.creatures.projectiles.iter() {
                let at = p.position;
                if p.id <= through && ticking(at.x.floor() as i32, at.z.floor() as i32) {
                    if round.alone_kind(p.entity_type()) {
                        alone_any = true;
                    } else {
                        round.add(p.id, super::native_ticks::TickKind::Fixed(p.entity_type()));
                    }
                }
            }
            self.native_tick_pre_batch(&mut round);
        }
        let mut projectiles = self.creatures.projectiles.lift();
        // The tick's own projectiles: what joins during it (a hook's spawn) ticks from the next.
        let last_ticked_join = projectiles.iter().map(|p| p.join).max();
        let mut hits = Vec::new();
        // The breezes that deflected a projectile, and where.
        let mut deflections: Vec<(u32, [f64; 3])> = Vec::new();
        let mut arrow_events = Vec::new();
        let mut shoot_events = Vec::new();
        let mut lava_hurt_bullets = Vec::new();
        let mut trails = Vec::new();
        let mut bubble_tops = Vec::new();
        let mut snow_contacts = Vec::new();
        let mut section_steps = section_steps::begin(self);
        let mut keep_flags = vec![true; projectiles.len()];
        let mut winds = std::mem::take(&mut self.creatures.pickable_projectile_targets);
        // The impacts the mods may cancel (`native_projectile_impact`), on a server.
        let hear = self.native_hears_impacts();
        let mut impacts: Vec<Impact> = Vec::new();
        let mut vehicle_winds = self.vehicle_winds();
        // What the ticks read of the other entities (the mobs, the guests, the breezes), built
        // again after a hook told in the loop changed it (`projectile_targets_read`).
        let mut seen = self.projectile_tick_targets();
        let border = Some(self.border.bounds());
        // ThrownEnderpearl.tick: a dead player's pearl is discarded in
        // place of its tick while ender_pearls_vanish_on_death is on.
        let vanish = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::ENDER_PEARLS_VANISH_ON_DEATH);
        // A projectile told alone hands the list back for its hooks, so their handles read the
        // live rows. What that needs is made only when a mod named a projectile type this tick
        // (`ProjectileHandBack`): with none, the loop is the family's own.
        let mut hand_back = alone_any.then(|| ProjectileHandBack::new(&projectiles));
        for slot in 0..projectiles.len() {
            let i = match hand_back.as_deref() {
                None => slot,
                Some(back) => match back.place(slot) {
                    Some(i) => i,
                    None => continue,
                },
            };
            if projectiles[i].id > through
                || hear && self.projectile_removal_pending(projectiles[i].id)
            {
                continue;
            }
            // ServerLevel.tick: losing an entity-ticking ticket must not
            // advance projectile flags or motion while its chunk unloads.
            let at = projectiles[i].position;
            if !ticking(at.x.floor() as i32, at.z.floor() as i32) {
                projectiles.update(i, |p| p.previous = at);
                continue;
            }
            // Of a type a mod named: its own Pre, with the list handed back.
            let i = match hand_back.as_deref_mut() {
                None => i,
                Some(back) => {
                    let (id, kind) = (projectiles[i].id, projectiles[i].entity_type());
                    if !round.alone_kind(kind) {
                        i
                    } else {
                        let mut ticks = true;
                        let same;
                        let read = self.projectile_targets_read();
                        (projectiles, same) = self.projectiles_told(projectiles, |game| {
                            ticks = game.native_tick_pre_one(&mut round, id, kind);
                        });
                        // A hook that wrote what the hit tests read: the next projectile sees
                        // its writes.
                        if self.projectile_targets_changed(&read, &seen) {
                            seen = self.projectile_tick_targets();
                        }
                        if !same {
                            back.follow(&projectiles, &mut keep_flags, &mut impacts);
                        }
                        match back.place(slot) {
                            Some(at) if ticks => {
                                back.told = Some((id, kind));
                                at
                            }
                            _ => continue,
                        }
                    }
                }
            };
            // A mod canceled its tick: it stays as it is.
            if round.canceled(projectiles[i].id) {
                continue;
            }
            let section_from = projectiles[i].position;
            // A ghast's fireball or a wind charge is pickable, so a pearl, an
            // arrow or a trident can strike it (`Projectile.isPickable`,
            // `Projectile.onHit`).
            winds.clear();
            if matches!(projectiles[i].kind, ProjectileKind::EnderPearl)
                || projectiles[i].arrow.is_some()
            {
                pickable_projectile_hitboxes_into(&projectiles, &keep_flags, i, &mut winds);
            }
            let mut redirect = None;
            {
                let w = self.mob_world_base(&[]);
                let rain = self.arrow_rain();
                let scene = self.arrow_scene(&rain);
                let pw = ProjWorld {
                    w: &w,
                    targets: &seen.targets,
                    border,
                };
                let pw_guests = ProjWorld {
                    w: &w,
                    targets: &seen.targets_guests,
                    border,
                };
                projectiles.update(i, |p| {
                    if vanish
                        && matches!(p.kind, ProjectileKind::EnderPearl)
                        && p.owner == Some(Who::Player)
                        && p.guest
                            .map_or(seen.host_dead, |c| seen.dead_guests.contains(&c))
                    {
                        keep_flags[i] = false;
                        return;
                    }
                    // The projectile as it was, to fly on if a mod cancels its impact.
                    let before = (hear && !p.stuck).then(|| p.clone());
                    let wind_before = vehicle_winds.before(p);
                    let mob_shot = !matches!(p.owner, Some(Who::Player));
                    // A player's arrows, snowballs and eggs meet every player; a
                    // guest's flies from that guest (Projectile.getOwner), not
                    // from the host, until it has left them.
                    let pvp = !mob_shot && super::lan_pvp::meets_players(&p.kind);
                    let shooter = p
                        .guest
                        .filter(|_| pvp)
                        .and_then(|c| seen.guest_entities.iter().find(|(g, _)| *g == c))
                        .map(|&(_, id)| Who::Mob(id));
                    if shooter.is_some() {
                        p.owner = shooter;
                    }
                    let use_guests = (mob_shot || pvp) && seen.has_guests;
                    let (keep, hit) = match (p.arrow.is_some(), use_guests, winds.is_empty()) {
                        (true, false, true) => sunrise_client::arrows::tick_to_block_contact(
                            p, &scene, pw.targets, hear,
                        ),
                        (true, true, true) => sunrise_client::arrows::tick_to_block_contact(
                            p,
                            &scene,
                            pw_guests.targets,
                            hear,
                        ),
                        (true, with_guests, false) => {
                            let mut targets = if with_guests {
                                pw_guests.targets.to_vec()
                            } else {
                                pw.targets.to_vec()
                            };
                            targets.extend_from_slice(&winds);
                            sunrise_client::arrows::tick_to_block_contact(p, &scene, &targets, hear)
                        }
                        (false, false, true) => p.tick(&pw),
                        (false, true, true) => p.tick(&pw_guests),
                        (false, pvp_shot, false) => {
                            let mut targets = if pvp_shot {
                                pw_guests.targets.to_vec()
                            } else {
                                pw.targets.to_vec()
                            };
                            targets.extend_from_slice(&winds);
                            p.tick(&ProjWorld {
                                w: if pvp_shot { pw_guests.w } else { pw.w },
                                targets: &targets,
                                border,
                            })
                        }
                    };
                    if (p.is_thrown() && !matches!(p.kind, ProjectileKind::LlamaSpit))
                        || p.is_arrow() && p.arrow.as_ref().is_none_or(|a| !a.no_physics)
                    {
                        for block in sunrise_client::powder_snow::contacts(
                            p.previous,
                            p.position,
                            p.bounds(),
                            false,
                            |at| w.block_name(at.x, at.y, at.z) == "minecraft:powder_snow",
                        ) {
                            snow_contacts.push((p.id, block, p.position, p.velocity));
                        }
                    }
                    if shooter.is_some() {
                        p.owner = Some(Who::Player);
                    }
                    if let Some(at) = p.take_shoot_event() {
                        shoot_events.push((p.owner, p.guest, [at.x, at.y, at.z]));
                    }
                    if p.take_lava_hurt() {
                        lava_hurt_bullets.push(p.position.components());
                    }
                    // AbstractHurtingProjectile.tick ends with createParticleTrail.
                    if super::remote_projectiles::has_smoke_trail(&p.kind) {
                        trails.push(p.position);
                    }
                    // ThrownEnderpearl.onAboveBubbleColumn / onInsideBubbleColumn.
                    if keep && hit == Hit::None && matches!(p.kind, ProjectileKind::EnderPearl) {
                        let (at, h) = (p.position, 0.125);
                        sunrise_client::motion::bubble_column_push_over(
                            w.world,
                            w.registry,
                            w.facts,
                            w.collision,
                            Aabb::new(
                                Vec3::new(at.x - h, at.y, at.z - h),
                                Vec3::new(at.x + h, at.y + 2.0 * h, at.z + h),
                            ),
                            &mut p.velocity,
                            |pos| bubble_tops.push(pos),
                        );
                    }
                    let events_from = arrow_events.len();
                    let mut shot_after_hit = false;
                    if let Some(a) = p.arrow.as_mut() {
                        for event in a.events.drain(..) {
                            if event == sunrise_client::arrows::ArrowEvent::ShootEvent {
                                if matches!(hit, Hit::Block { .. }) {
                                    shot_after_hit = true;
                                } else {
                                    shoot_events.push((p.owner, p.guest, p.position.components()));
                                }
                            } else {
                                arrow_events.push((p.id, p.position, event));
                            }
                        }
                    }
                    let wind = matches!(
                        p.kind,
                        ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                    );
                    // Entity.deflection: a breeze turns every projectile but wind charges away.
                    if let Hit::Entity {
                        who: Who::Mob(id),
                        at,
                    } = hit
                        && !wind
                        && seen.breezes.contains(&id)
                    {
                        super::wind::reverse(p);
                        if p.arrow.is_some() {
                            sunrise_client::arrows::finish_entity_hit(
                                p,
                                &scene,
                                EntityHit::Deflected,
                            );
                            let a = p.arrow.as_mut().expect("an arrow");
                            arrow_events.extend(a.events.drain(..).map(|e| {
                                if e == sunrise_client::arrows::ArrowEvent::ShootEvent {
                                    shoot_events.push((p.owner, p.guest, p.position.components()));
                                }
                                (p.id, p.position, e)
                            }));
                        }
                        deflections.push((id, [at.x, at.y, at.z]));
                        return;
                    }
                    // AbstractWindCharge.tick: out of the world's top it bursts.
                    if wind
                        && keep
                        && let Some(hit) = self.wind_ceiling_contact(p)
                    {
                        hits.push(ProjectileHit {
                            projectile: p.clone(),
                            hit,
                            shot_after_hit: false,
                        });
                        keep_flags[i] = false;
                        return;
                    }
                    let wind_vehicle = vehicle_winds.met(wind_before, hit);
                    if hit != Hit::None {
                        if let Some(before) = before
                            && match hit {
                                Hit::Block { .. } => true,
                                Hit::Entity {
                                    who: Who::Mob(id), ..
                                } => pickable_projectile_hit(id).is_none(),
                                // A server's players are hit as entities, not Who::Player.
                                _ => false,
                            }
                        {
                            impacts.push(Impact {
                                index: Some(i),
                                keep,
                                before,
                                hit,
                                target: self.native_impact_target(hit),
                                wind_vehicle,
                                hit_slot: hits.len(),
                                events: events_from..arrow_events.len(),
                            });
                        }
                        hits.push(ProjectileHit {
                            projectile: p.clone(),
                            hit,
                            shot_after_hit,
                        });
                    }
                    keep_flags[i] = keep;
                    // Projectile.onHit: the redirectable projectile it hit (a ghast's
                    // fireball or a wind charge) turns before its own tick.
                    if let Hit::Entity {
                        who: Who::Mob(id), ..
                    } = hit
                        && let Some(j) = pickable_projectile_hit(id)
                    {
                        redirect = Some((j, (p.owner, p.owner_uuid, p.guest)));
                    }
                });
            }
            if let Some((j, (owner, owner_uuid, guest))) = redirect {
                let look = self.owner_look(owner, guest);
                projectiles.update(j, |wind| {
                    super::deflect::aim_deflect(wind, look, owner, owner_uuid, guest);
                });
            }
            keep_flags[i] = keep_flags[i]
                && section_steps::record(
                    self,
                    &mut section_steps,
                    projectiles.get(i),
                    section_from,
                    keep_flags[i],
                );
            // Its own Post, right after its tick, also when it ended in it. Its block effects and
            // hits land after the family's ticks, as they do for every projectile.
            if let Some(back) = hand_back.as_deref_mut()
                && let Some((id, kind)) = back.told.take()
            {
                let same;
                let read = self.projectile_targets_read();
                (projectiles, same) = self.projectiles_told(projectiles, |game| {
                    game.native_tick_post_one(&round, id, kind);
                });
                if self.projectile_targets_changed(&read, &seen) {
                    seen = self.projectile_tick_targets();
                }
                if !same {
                    back.follow(&projectiles, &mut keep_flags, &mut impacts);
                }
            }
        }
        self.creatures.pickable_projectile_targets = winds;
        // An arrow an entity hit leaves the batch for its hit's handling;
        // the level never takes it out (AbstractArrow.onHitEntity), so one
        // that flies on goes back where this tick had it.
        self.creatures.bounced_followers.clear();
        for ProjectileHit {
            projectile: p, hit, ..
        } in &hits
        {
            if p.arrow.is_some()
                && matches!(hit, Hit::Entity { .. })
                && let Some(i) = projectiles.iter().position(|q| q.id == p.id)
            {
                let followers = (i + 1..projectiles.len())
                    .filter(|&j| keep_flags[j])
                    .map(|j| projectiles[j].id)
                    .collect();
                self.creatures.bounced_followers.push((p.id, followers));
            }
        }
        // Do not retire an impact owner just because vanilla would consume it. Its live
        // row is what the event resolves and what transaction commit will remove.
        for impact in &mut impacts {
            if let Some(i) = impact.index {
                impact.keep = keep_flags[i];
                keep_flags[i] = true;
            }
        }
        let mut flags = keep_flags.into_iter();
        projectiles.retain(|_| flags.next().unwrap());
        let hears_impact = !impacts.is_empty();
        let scope = ImpactTransaction::new(self, hears_impact);
        let game = &mut *scope.game;
        let pending = if hears_impact {
            game.native_settle_impacts(
                &mut projectiles,
                impacts,
                &mut hits,
                &mut arrow_events,
                &mut shoot_events,
            )
        } else {
            FxHashMap::default()
        };
        for (id, block, position, movement) in snow_contacts {
            if !hears_impact || !game.projectile_removal_pending(id) {
                game.powder_snow_particle(block, position, movement);
            }
        }
        for (owner, guest, at) in shoot_events {
            game.with_projectile_dispatch(&mut projectiles, |game| {
                game.projectile_shoot_event(owner, guest, at);
            });
        }
        // ShulkerBullet.hurtServer's sound and crits, then Entity.lavaHurt's burn.
        for at in lava_hurt_bullets {
            game.shulker_bullet_hurt(at);
            game.lava_hurt_sound(at);
        }
        game.smoke_trails(trails);
        game.dragon_breath_trails();
        for pos in bubble_tops {
            game.bubble_column_particles(pos);
        }
        // Breeze.onProjectileDeflect: level.playSound(null, breeze,
        // BREEZE_DEFLECT, HOSTILE, 1, 1).
        for (breeze, at) in deflections {
            let sound = LevelSound::new("minecraft:entity.breeze.deflect", at, 1.0, 1.0)
                .in_source(SoundSource::Hostile);
            game.play_bound_sound(sound, breeze, None);
        }
        for (id, at, event) in arrow_events {
            if !hears_impact
                || !projectile_impact_event(&event)
                || !game.projectile_removal_pending(id)
            {
                game.arrow_event(at, event);
            }
        }
        // `Entity.applyEffectsFromBlocks` sweeps the original cohort by join identity:
        // settlement can remove rows or append hook spawns. Contacts need the lent
        // level here, after the read-only borrow, each still in this batch's tick.
        for i in 0..projectiles.len() {
            if last_ticked_join.is_none_or(|last| projectiles[i].join > last)
                || hears_impact && game.projectile_removal_pending(projectiles[i].id)
            {
                continue;
            }
            projectiles.update(i, |p| game.sweep_projectile_inside(p));
            projectiles.update(i, |p| game.lava_mark_hurt(p));
        }
        game.creatures.projectiles.restore(
            projectiles,
            sunrise_client::join_seq::RestoreOrder::SpawnsFirst,
        );
        section_steps::finish(game, section_steps);
        let result = game.apply_projectile_hits(hits, &mut vehicle_winds, pending, hears_impact);
        scope.close(result.is_ok());
        self.creatures.bounced_followers.clear();
        result?;
        self.flush_lan_explosions();
        let flushed = self.flush_wind_bursts();
        self.native_tick_post_batch(round);
        flushed
    }
    fn apply_projectile_hits(
        &mut self,
        hits: Vec<ProjectileHit>,
        vehicle_winds: &mut super::wind::VehicleWinds,
        mut pending: FxHashMap<JoinSeq, PendingImpact>,
        hears_impact: bool,
    ) -> Result<()> {
        for ProjectileHit {
            projectile: p,
            hit,
            shot_after_hit,
        } in hits
        {
            let (id, join) = (p.id, p.join);
            if hears_impact && self.projectile_removal_pending(id) {
                continue;
            }
            let contact = pending.remove(&join);
            let deferred_arrow =
                contact.is_some() && p.arrow.is_some() && matches!(hit, Hit::Block { .. });
            let wind_flight = vehicle_winds.take(id, hit);
            let landed = match (contact, wind_flight) {
                (Some(contact), Some(before)) => {
                    self.apply_vehicle_wind_impact(p, hit, contact, before)
                }
                (Some(contact), None) => self
                    .apply_captured_impact(p, hit, contact)
                    .map(|p| (p, hit)),
                (None, Some(before)) if self.wind_vehicle_broken(hit) => {
                    let (p, keep, hit) = self.broken_vehicle_retick(before);
                    if keep {
                        if let Some(ceiling) = self.wind_ceiling_contact(&p) {
                            Some((p, ceiling))
                        } else {
                            self.add_projectile(p);
                            None
                        }
                    } else {
                        (hit != Hit::None).then_some((p, hit))
                    }
                }
                (None, _) => Some((p, hit)),
            };
            if let Some((p, hit)) = landed {
                self.apply_projectile_hit(&p, hit)?;
                if shot_after_hit {
                    self.projectile_shoot_event(p.owner, p.guest, p.position.components());
                }
            }
            if deferred_arrow {
                self.finish_deferred_arrow_tick(id, join);
            }
        }
        Ok(())
    }

    fn apply_projectile_hit(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        // A player's arrow on another player lands as the victim's hurt.
        if let Some(victim) = self.projectile_pvp_victim(p, hit) {
            return self.projectile_hits_player(p, hit, victim);
        }
        // A guest's projectile lands as theirs: a pearl takes them, what it
        // hurts answers to them. One hitting the host stays the host's hit.
        match p.guest {
            Some(conn)
                if !matches!(
                    hit,
                    Hit::Entity {
                        who: Who::Player,
                        ..
                    }
                ) =>
            {
                self.guest_projectile_hit(conn, p, hit)
            }
            _ => self.projectile_hit(p, hit),
        }
    }
    /// `LivingEntity.hurtServer` for a snowball, egg or pearl (`kind`) that hurt the mob `id`,
    /// 0 damage included: the accepted hit already credited its owner; for a player,
    /// also fire PLAYER_HURT_ENTITY.
    fn thrown_hurt_mob(&mut self, p: &Projectile, id: u32, kind: &str, amount: f32) {
        if p.owner == Some(Who::Player)
            && let Some(m) = self.mobs.iter().find(|m| m.id == id)
        {
            let victim = self.mob_entity(m);
            let direct = sunrise_client::advancements::EntityView {
                kind: kind.into(),
                position: [p.position.x, p.position.y, p.position.z],
                ..Default::default()
            };
            self.player_hit(&victim, "minecraft:thrown", Some(direct), amount);
        }
    }
    /// `projectile_hit` for a projectile the guest `conn` shot, run as them;
    /// as the host's when they have left.
    fn guest_projectile_hit(&mut self, conn: u32, p: &Projectile, hit: Hit) -> Result<()> {
        let result = self.as_guest(conn, |g| g.projectile_hit(p, hit));
        // The mob hit now answers to the guest (LivingEntity.lastHurtByPlayer).
        if result.is_some()
            && self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .is_some_and(|guest| guest.participates_online())
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && self.mobs.iter().any(|m| m.id == id)
            && let Some(host) = self.lan.as_mut()
        {
            host.mob_players.insert(id, conn);
        }
        match result {
            Some(r) => r,
            None => self.projectile_hit(p, hit),
        }
    }
    /// `EnchantmentHelper.doPostAttackEffects` after the player's shot `p`
    /// (its `direct` entity, a hit of `damage_type`) hurt the mob `who`: the
    /// victim's own post-attack enchantments answer the player (Thorns).
    pub(super) fn shot_post_attack(
        &mut self,
        p: &Projectile,
        who: Who,
        direct: &str,
        damage_type: &str,
    ) {
        if let Who::Mob(id) = who {
            self.mob_post_attack(id, p.owner, Some(direct), Some(damage_type));
        }
    }
    /// A projectile's hurt of `amount` on the boat or minecart `id`
    /// (`VehicleEntity.hurtServer`): even 0 shakes it (`Snowball`, `ThrownEgg`,
    /// `ThrownEnderpearl` `.onHitEntity`), 5 from a small fireball destroys it.
    pub(super) fn hurt_vehicle(&mut self, p: &Projectile, id: u32, amount: f32) {
        let creative = p.owner == Some(Who::Player) && self.creative;
        let damage_kind = self.projectile_damage_kind(p);
        let damage_type = source_damage_type(Damage {
            kind: damage_kind,
            attacker: p.owner,
            from: None,
            attacker_species: None,
        });
        if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            let kind = self.boats[i].entity_type();
            if !self.vehicle_refuses_projectile(p, id, &kind, damage_type) {
                self.hurt_boat(i, amount, creative);
            }
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            let kind = self.carts[i].kind.id();
            if !self.vehicle_refuses_projectile(p, id, kind, damage_type) {
                self.hurt_cart(
                    i,
                    amount,
                    creative,
                    false,
                    super::minecarts::CartHit::projectile(p.owner, p.guest),
                );
            }
        }
    }
    /// Whether the per-source invulnerability hook rejects a projectile hit on a vehicle.
    fn vehicle_refuses_projectile(
        &mut self,
        p: &Projectile,
        id: u32,
        kind: &str,
        damage_type: &str,
    ) -> bool {
        let causing = match p.owner {
            Some(Who::Player) => match p.guest {
                Some(conn) => self.command_guest(conn).map_or(0, |g| g.entity_id),
                None => self.own_entity_id(),
            },
            Some(Who::Mob(id)) => id as i32,
            None => 0,
        };
        let source = sunrise_client::health::HurtSource {
            damage_type,
            causing,
            direct: p.id as i32,
            position: Some([p.position.x, p.position.y, p.position.z]),
        };
        let creative = match p.owner {
            Some(Who::Player) => match p.guest {
                Some(conn) => self.command_guest(conn).is_some_and(|g| g.creative()),
                None => self.creative,
            },
            _ => false,
        };
        let vanilla = self.saved_refuses_damage(id, damage_type, creative);
        crate::game::entity::damage_level_of!(self, self.combat.attack(), no_push, None)
            .invulnerability_check(
                sunrise_client::health::Hurting::Entity(id as i32, kind, source),
                vanilla,
            )
    }
    /// `BellBlock.onProjectileHit`: a projectile that strikes the bell proper
    /// rings it, and a player's shot counts as the player's ring.
    pub(super) fn bell_projectile_hit(
        &mut self,
        p: &ProjectileHitter,
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    ) {
        let Some(face) = Direction::ALL.into_iter().find(|d| d.normal() == face) else {
            return;
        };
        let block = BlockPos::new(pos[0], pos[1], pos[2]);
        let rang = self
            .with_level(|b, level, ticks| b.bell_projectile_hit(level, ticks, block, face, at.y));
        if rang {
            let _ = self.process_level_events();
            if p.owner == Some(Who::Player) {
                self.award_custom_stat("bell_ring", 1);
            }
        }
    }
    /// `AmethystBlock.onProjectileHit`: a projectile that strikes an amethyst
    /// block rings `block.amethyst_block.chime` there, pitched by the level random.
    pub(super) fn amethyst_projectile_hit(&mut self, pos: [i32; 3]) {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if &*self.res.facts().get(state).name != "minecraft:amethyst_block" {
            return;
        }
        let pitch = 0.5 + self.world.random.next_float() * 1.2;
        self.level_events
            .push(sunrise_world::level::LevelEvent::Sound {
                pos: BlockPos::new(pos[0], pos[1], pos[2]),
                sound: "minecraft:block.amethyst_block.chime".into(),
                source: None,
                volume: 1.0,
                pitch,
            });
        let _ = self.process_level_events();
    }
    /// `onProjectileHit` of chorus flowers, decorated pots and speleothems
    /// (pointed dripstone, sulfur spikes): an impact projectile
    /// (`#impact_projectiles`) breaks the block when `Projectile.mayInteract`
    /// and `Projectile.mayBreak` allow it, which takes the
    /// `projectiles_can_break_blocks` rule and a shooter that may break blocks
    /// (`canBreakBlockInAdventureMode`: a resolved mob's arrow needs its
    /// actual projectile predicate and mob_griefing). Other owner/kind paths
    /// retain their existing policy. A pot is cracked first (flags 260).
    fn projectile_breaks_block(&mut self, p: &Projectile, pos: [i32; 3]) {
        use sunrise_client::command::args::GameMode;
        let impact = matches!(
            p.kind,
            ProjectileKind::Arrow { .. }
                | ProjectileKind::Trident
                | ProjectileKind::Snowball
                | ProjectileKind::Thrown(_)
                | ProjectileKind::Egg
                | ProjectileKind::SmallFireball
                | ProjectileKind::LargeFireball { .. }
                | ProjectileKind::WitherSkull { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        );
        use sunrise_client::arrows::OwnerLookup;
        let shooter_may_break = match p.owner {
            Some(Who::Player) => self.mode != GameMode::Adventure,
            Some(Who::Mob(_)) => {
                matches!(p.kind, ProjectileKind::Arrow { .. })
                    && self
                        .ui
                        .rules
                        .bool_at(sunrise_client::gamerules::vanilla::MOB_GRIEFING)
                    && p.arrow
                        .as_ref()
                        .and_then(|a| a.can_break.as_ref())
                        .is_some_and(|predicate| {
                            super::game_mode::projectile_predicate_supported(predicate)
                                && self.adventure_predicate_at(predicate, pos)
                        })
            }
            None => !p.owner_uuid.is_some_and(|u| self.non_player_entity(u)),
        };
        if !impact
            || !shooter_may_break
            || !self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::PROJECTILES_CAN_BREAK_BLOCKS)
        {
            return;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        // `destroyBlock(pos, true, projectile)`; a speleothem's names no entity.
        let mut entity = Some(p.id);
        match &*self.res.facts().get(state).name {
            "minecraft:chorus_flower"
                if matches!(
                    p.kind,
                    ProjectileKind::Arrow { .. } | ProjectileKind::Snowball
                ) =>
            {
                // `ChorusFlowerBlock.onProjectileHit` supplies the projectile,
                // even when it has no owner, as the direct block's loot entity.
                let _ = self.destroy_block_by(pos, Some(p.id));
                return;
            }
            "minecraft:chorus_flower" => {}
            // SpeleothemBlock.onProjectileHit (pointed dripstone and sulfur spikes): a
            // thrown trident faster than 0.6 breaks it.
            "minecraft:pointed_dripstone" | "minecraft:sulfur_spike"
                if matches!(p.kind, ProjectileKind::Trident)
                    && p.arrow.as_ref().is_some_and(|a| a.hit_speed > 0.6) =>
            {
                entity = None;
            }
            "minecraft:decorated_pot" => {
                let registry = &self.pack.assets.registry;
                let Some(mut pot) = registry.state(state).cloned() else {
                    return;
                };
                pot.properties.insert("cracked".into(), "true".into());
                let Ok(cracked) = registry.resolve(&pot) else {
                    return;
                };
                self.with_level(|b, level, ticks| b.set_block(level, ticks, at, cracked, 260));
            }
            _ => return,
        }
        let _ = self.destroy_block_by(pos, entity);
    }
    /// What an arrow's tick asked for: its particles and sounds.
    pub(in crate::game) fn arrow_event(
        &mut self,
        at: Vec3,
        event: sunrise_client::arrows::ArrowEvent,
    ) {
        use sunrise_client::arrows::{ArrowEvent, ArrowParticle, ArrowSound, BlockReaction};
        use sunrise_client::particles::{ParticleOptions, ParticleType};
        let kind = |k| match k {
            ArrowParticle::Bubble => ParticleType::Bubble,
            ArrowParticle::Crit => ParticleType::Crit,
            ArrowParticle::Splash => ParticleType::Splash,
        };
        match event {
            ArrowEvent::Particle {
                kind: k,
                position,
                velocity,
            } => self.particles.add(
                &ParticleOptions::simple(kind(k)),
                [position.x, position.y, position.z],
                [velocity.x, velocity.y, velocity.z],
            ),
            // Arrow.tick: `broadcastEntityEvent(this, 0)`. The window and the
            // guests burst the particles themselves (`remote_arrow_puff`).
            ArrowEvent::PotionPuff { id, .. } => {
                self.lan_entity_event(id as i32, 0, [at.x, at.y, at.z]);
            }
            ArrowEvent::Sound {
                sound,
                volume,
                pitch,
            } => {
                let name = match sound {
                    ArrowSound::Splash => "minecraft:entity.generic.splash",
                    ArrowSound::HitGround => "minecraft:entity.arrow.hit",
                    ArrowSound::CrossbowHit => "minecraft:item.crossbow.hit",
                    ArrowSound::Extinguish => "minecraft:entity.generic.extinguish_fire",
                    ArrowSound::TridentHitGround => "minecraft:item.trident.hit_ground",
                    ArrowSound::TridentHit => "minecraft:item.trident.hit",
                };
                // Each one is the projectile's own `Entity.playSound`, on `getSoundSource()`:
                // NEUTRAL, which neither AbstractArrow nor ThrownTrident overrides. The
                // name-based guess would put `entity.arrow.*` and `item.*` on PLAYERS.
                self.audio
                    .play_on(6, name, Some([at.x, at.y, at.z]), volume, pitch);
            }
            ArrowEvent::BubbleColumnParticles { pos } => self.bubble_column_particles(pos),
            ArrowEvent::Block {
                pos,
                reaction,
                owner,
            } => match reaction {
                BlockReaction::PrimeTnt => {
                    // `TntBlock.onProjectileHit`: the arrow's owner lights it.
                    if self.tnt_prime_allowed(pos) {
                        self.spawn_primed_tnt(pos, false, owner, None);
                        self.with_level(|b, level, ticks| b.remove_block(level, ticks, pos, false));
                        let _ = self.process_level_events();
                    }
                }
                BlockReaction::Light(lit) => {
                    let _ = self.level_set([pos.x, pos.y, pos.z], lit);
                }
            },
            ArrowEvent::SplashEvent => {
                self.projectile_event("minecraft:splash", None, [at.x, at.y, at.z]);
            }
            // finish_arrow_hit drops the pickup, which it holds.
            ArrowEvent::ShootEvent | ArrowEvent::Drop => {}
        }
    }
    /// `Level.isRainingAt` for arrows.
    /// What a projectile can hit: the mobs, the player, the dragon's parts and
    /// the End crystals (under reserved ids, game/mobs/dragon.rs).
    /// The boats and minecarts a projectile can strike.
    pub(super) fn vehicle_targets(&self) -> Vec<(Who, Aabb)> {
        let mut targets: Vec<(Who, Aabb)> = self
            .boats
            .iter()
            .map(|b| (Who::Mob(b.id), b.bounds()))
            .collect();
        targets.extend(self.carts.iter().map(|c| (Who::Mob(c.id), c.bounds())));
        targets
    }

    pub(super) fn arrow_rain(&self) -> impl Fn(BlockPos) -> bool + '_ {
        move |p: BlockPos| self.raining_at(p)
    }
    /// The level as arrows see it.
    pub(super) fn arrow_scene<'s>(
        &'s self,
        rain: &'s dyn Fn(BlockPos) -> bool,
    ) -> sunrise_client::arrows::ArrowScene<'s, 's> {
        sunrise_client::arrows::ArrowScene {
            world: &self.world,
            registry: self.pack.assets.registry,
            facts: self.res.facts(),
            collision: self.collision,
            fluids: &self.motion.fluids,
            loop_context: &self.loop_context,
            raining: rain,
            mob_griefing: self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::MOB_GRIEFING),
            tnt_explodes: self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::TNT_EXPLODES),
            mob_hooks: self
                .grief_hooked()
                .then_some(self as &dyn sunrise_client::mobs::MobHooks),
            owners: self,
        }
    }
    /// Dispatch with the projectile's actual owner context, including a guest shooter.
    pub(super) fn projectile_shoot_event(
        &mut self,
        owner: Option<Who>,
        guest: Option<u32>,
        at: [f64; 3],
    ) {
        if let Some(conn) = guest.filter(|conn| self.lan_acting != Some(*conn)) {
            let dispatched = self
                .as_guest(conn, |g| {
                    g.projectile_event("minecraft:projectile_shoot", owner, at);
                })
                .is_some();
            // An owner who left cannot resolve to the host. The event
            // still occurs, with no resolved owner (Projectile.getOwner).
            if !dispatched {
                self.projectile_event("minecraft:projectile_shoot", None, at);
            }
        } else {
            self.projectile_event("minecraft:projectile_shoot", owner, at);
        }
    }
    /// The rest of the tick of an arrow that hit an entity
    /// (`arrows::finish_entity_hit`): its sounds and drop, and back among the
    /// projectiles if it flies on.
    fn finish_arrow_hit(&mut self, mut arrow: Projectile, outcome: EntityHit) {
        let keep = {
            let rain = self.arrow_rain();
            sunrise_client::arrows::finish_entity_hit(&mut arrow, &self.arrow_scene(&rain), outcome)
        };
        let Some(a) = arrow.arrow.as_mut() else {
            return;
        };
        let (events, item) = (std::mem::take(&mut a.events), a.pickup_item.clone());
        for event in events {
            if event == sunrise_client::arrows::ArrowEvent::Drop {
                self.spawn_at_location(arrow.position, f64::from(0.1f32), item.clone());
            } else if event == sunrise_client::arrows::ArrowEvent::ShootEvent {
                self.projectile_shoot_event(arrow.owner, arrow.guest, arrow.position.components());
            } else {
                self.arrow_event(arrow.position, event);
            }
        }
        if keep {
            // The arrow never left the level (AbstractArrow.onHitEntity's
            // deflect(REVERSE), or a dodge it flies on past): it goes back
            // where it ticked, keeping its place in EntityTickList's order,
            // and nothing hears it launch again.
            let followers = self
                .creatures
                .bounced_followers
                .iter()
                .position(|(id, _)| *id == arrow.id)
                .map(|k| self.creatures.bounced_followers.swap_remove(k).1)
                .unwrap_or_default();
            let projectiles = &mut self.creatures.projectiles;
            let index = bounced_place(projectiles, &followers);
            projectiles.insert(index, arrow);
        }
    }
    /// `ThrownEnderpearl.onHit` for a living non-player owner: the same hook,
    /// then the normal entity teleport, without a player's endermite roll or damage.
    fn mob_pearl_teleport(&mut self, pearl: &Projectile, id: u32, hit: Hit) {
        let Some(owner) = self.entity(id) else { return };
        let Some(from) = self
            .mobs
            .iter()
            .find(|m| {
                m.id == id
                    && m.alive()
                    && !m.removed
                    && !sunrise_client::mobs::villager::sleeping(m)
            })
            .map(|m| m.position)
        else {
            return;
        };
        let dimension = self.dim;
        let failures = mod_api::runtime::isolated_failures();
        let _: std::result::Result<(), ()> = mod_api::run_transaction(self, |game, _| {
            let (mut target, _, cancel) =
                game.native_pearl_teleport(owner, pearl.id, from, pearl.previous, 0.0, hit);
            if mod_api::runtime::isolated_failures() != failures {
                return Err(());
            }
            // Listener writes stay in the transaction; never overwrite a changed,
            // dead, sleeping or removed owner using the old decision.
            if cancel
                || game.dim != dimension
                || game.entity(id) != Some(owner)
                || !game.mobs.iter().any(|m| {
                    m.id == id
                        && m.alive()
                        && !m.removed
                        && m.position == from
                        && !sunrise_client::mobs::villager::sleeping(m)
                })
            {
                return Ok(());
            }
            if !target.iter().all(|v| v.is_finite()) {
                target = [pearl.previous.x, pearl.previous.y, pearl.previous.z];
            }
            if !super::native_entity_moves::in_bounds(target) {
                return Ok(());
            }
            let _ = game.place_teleport_filed(
                super::commands::Target::Mob(id),
                target,
                None,
                [false; 3],
                true,
            );
            if let Some(m) = game.mobs.iter_mut().find(|m| m.id == id) {
                m.fall_distance = 0.0;
                // Entity.teleport's setOldRot: keep the owner's angles without
                // interpolating from the pre-teleport rotation on the client.
                m.previous_yaw = m.yaw;
                m.previous_head_pitch = m.head_pitch;
            }
            game.audio
                .play("minecraft:entity.player.teleport", Some(target), 1.0, 1.0);
            Ok(())
        });
    }

    /// A projectile's hit, landed in the attack of its owner.
    pub(super) fn projectile_hit(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        let attack = self.projectile_attack(p);
        let result = self.with_attack(attack, |g| g.projectile_hit_landed(p, hit));
        self.settle_dealt_absorbed(p.owner == Some(Who::Player));
        result?;
        if let Hit::Entity {
            who: Who::Mob(id), ..
        } = hit
        {
            self.settle_mob_hurt(id);
        }
        // The hit's hurt teleports an enderman or a shulker now (`hurtServer`).
        self.settle_hurt_teleports()
    }
    fn projectile_hit_landed(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        // Projectile.onHitBlock: BlockState.onProjectileHit, for every
        // projectile (a wind charge out of the world's top has no face).
        if let Hit::Block { pos, at, face } = hit
            && face != [0; 3]
        {
            self.projectile_block_impact(&ProjectileHitter::of(p), pos, at, face)?;
            self.projectile_breaks_block(p, pos);
        }
        self.combat.projectile_owner = match p.owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        self.combat.projectile_direct = match hit {
            Hit::Entity {
                who: Who::Player, ..
            } => Some(self.target_name(super::commands::Target::Projectile(p.id))),
            _ => None,
        };
        let at = match hit {
            Hit::Entity { at, .. } | Hit::Block { at, .. } => at,
            Hit::None => return Ok(()),
        };
        // Projectile.onHitBlock: BlockState.onProjectileHit, for every
        // projectile (a wind charge out of the world's top has no face).
        if let Hit::Block { pos, face, .. } = hit
            && face != [0; 3]
        {
            self.target_block_hit(&ProjectileHitter::of(p), pos, at, face);
        }
        if let Hit::Entity {
            who: Who::Mob(id),
            at,
        } = hit
            && self.lan_projectile_hit(p, id, at)
        {
            return Ok(());
        }
        if matches!(
            p.kind,
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
        ) {
            return self.wind_charge_hit(p, hit);
        }
        // A wither skull's hit on a part or crystal goes through
        // `wither_skull_hit` below, as on any entity, and then explodes.
        if let Hit::Entity {
            who: Who::Mob(id), ..
        } = hit
            && id >= super::dragon::SPECIAL
            && !matches!(
                p.kind,
                ProjectileKind::WitherSkull { .. } | ProjectileKind::DragonFireball
            )
        {
            let by_player = p.owner == Some(Who::Player);
            if p.arrow.is_some() {
                // AbstractArrow.onHitEntity: spent if the part or crystal took
                // it, else turned back (a perched dragon sets it alight first).
                let mut arrow = p.clone();
                let amount = self.shot_damage(
                    &mut arrow,
                    Some(if id >= super::dragon::CRYSTAL_IDS {
                        "minecraft:end_crystal"
                    } else {
                        "minecraft:ender_dragon"
                    }),
                    Who::Mob(id),
                );
                let outcome = match self.hit_special(
                    id,
                    amount,
                    by_player,
                    self.projectile_damage_kind(p),
                    Some(p),
                ) {
                    super::dragon::Hurt::Landed { .. } => EntityHit::Hurt,
                    super::dragon::Hurt::Refused => EntityHit::Refused,
                    super::dragon::Hurt::Shrugged => {
                        if let Some(a) = arrow.arrow.as_mut() {
                            a.ignite_for_seconds(1.0);
                        }
                        EntityHit::Refused
                    }
                };
                self.finish_arrow_hit(arrow, outcome);
                return Ok(());
            }
            let amount = match p.kind {
                ProjectileKind::SmallFireball => 5.0,
                ProjectileKind::LargeFireball { .. } => 6.0,
                ProjectileKind::Thrown(t) => t.damage,
                _ => 0.0,
            };
            let kind = match p.kind {
                ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                    sunrise_client::mobs::DamageKind::Fireball
                }
                _ => sunrise_client::mobs::DamageKind::Thrown,
            };
            self.hit_special(id, amount, by_player, kind, Some(p));
            // ThrowableProjectile.onHit goes on after onHitEntity: a pearl
            // still takes its owner to where it broke (into the crystal's
            // blast), an egg still hatches, a snowball still breaks. A large
            // fireball still explodes (LargeFireball.onHit), whatever the
            // part or crystal made of its hit (both refuse fire), and the
            // blast takes the crystal.
            if !matches!(
                p.kind,
                ProjectileKind::EnderPearl
                    | ProjectileKind::Egg
                    | ProjectileKind::Snowball
                    | ProjectileKind::Thrown(_)
                    | ProjectileKind::LargeFireball { .. }
            ) {
                return Ok(());
            }
        }
        if p.arrow.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && self.item_frames.iter().any(|f| f.id == id)
        {
            // AbstractArrow.onHitEntity on an item frame: spent when the
            // frame took the hit, turned back when a fixed one refused it.
            let by_player = p.owner == Some(Who::Player);
            let hurt = self.projectile_hurts_frame(id, by_player, by_player && self.creative);
            let outcome = if hurt {
                EntityHit::Hurt
            } else {
                EntityHit::Refused
            };
            self.finish_arrow_hit(p.clone(), outcome);
            return Ok(());
        }
        if p.arrow.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && let Some(i) = self.carts.iter().position(|c| c.id == id)
        {
            // AbstractArrow.onHitEntity on a minecart.
            let mut arrow = p.clone();
            let victim_type = self.carts[i].kind.id().to_owned();
            let amount = self.shot_damage(&mut arrow, Some(&victim_type), Who::Mob(id));
            let burning = p.arrow.as_ref().is_some_and(|a| a.on_fire());
            let creative = p.owner == Some(Who::Player) && self.creative;
            let kind = self.carts[i].kind.id();
            let damage_type = sunrise_client::health::damage_type(
                self.projectile_damage_kind(p),
                p.owner.is_some(),
            );
            let refused = self.vehicle_refuses_projectile(p, id, kind, damage_type);
            if !refused {
                self.arrow_hits_cart(i, amount, p.velocity, burning, creative, p.owner, p.guest);
            }
            self.finish_arrow_hit(
                arrow,
                if refused {
                    EntityHit::Refused
                } else {
                    EntityHit::Hurt
                },
            );
            return Ok(());
        }
        if p.arrow.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && let Some(i) = self.boats.iter().position(|b| b.id == id)
        {
            // AbstractArrow.onHitEntity on a boat: VehicleEntity.hurtServer
            // takes the arrow's damage, and the arrow is spent.
            let mut arrow = p.clone();
            let victim_type = self.boats[i].entity_type();
            let amount = self.shot_damage(&mut arrow, Some(&victim_type), Who::Mob(id));
            let creative = p.owner == Some(Who::Player) && self.creative;
            let kind = self.boats[i].entity_type();
            let damage_type = sunrise_client::health::damage_type(
                self.projectile_damage_kind(p),
                p.owner.is_some(),
            );
            let refused = self.vehicle_refuses_projectile(p, id, &kind, damage_type);
            if !refused {
                self.hurt_boat(i, amount, creative);
            }
            self.finish_arrow_hit(
                arrow,
                if refused {
                    EntityHit::Refused
                } else {
                    EntityHit::Hurt
                },
            );
            return Ok(());
        }
        let atp = [at.x, at.y, at.z];
        // Projectile.onHit: the landing is heard.
        self.projectile_landed_event(p.id, p.owner, atp);
        let source_kind = self.projectile_damage_kind(p);
        let attacker_species = match p.owner {
            Some(Who::Mob(id)) => self.mobs.iter().find(|m| m.id == id).map(|m| m.species()),
            _ => None,
        };
        let source = Damage {
            kind: source_kind,
            attacker: p.owner,
            from: Some([p.previous.x, p.previous.y, p.previous.z]),
            attacker_species,
        };
        let aim = self.attacker_aim(source.attacker);
        match (&p.kind, hit) {
            // ThrownTrident.onHitEntity: 8 damage (Impaling on aquatic mobs),
            // dealt whether or not the hurt lands, then it bounces off.
            (ProjectileKind::Trident, Hit::Entity { who, .. }) => {
                let mut trident = p.clone();
                let victim = match who {
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| m.kind.id.clone()),
                    Who::Player => Some("minecraft:player".to_owned()),
                };
                let amount = self.shot_damage(&mut trident, victim.as_deref(), who);
                let hurt = match who {
                    Who::Player => self.mob_hits_player(
                        amount,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:trident",
                    ),
                    Who::Mob(id) => self.damage_projectile_with_weapon(p, id, amount, source, aim),
                };
                let outcome = if hurt {
                    self.shot_post_attack(p, who, "minecraft:trident", "minecraft:trident");
                    EntityHit::Hurt
                } else {
                    EntityHit::Refused
                };
                self.burning_arrow_primes(p, who);
                self.finish_arrow_hit(trident, outcome);
            }
            // AbstractArrow.onHitBlock plays its sound itself (ArrowEvent).
            (ProjectileKind::Trident, Hit::Block { .. }) => {}
            // AbstractArrow.onHitEntity on a ghast's fireball or a wind charge:
            // `hurtOrSimulate` fails (`AbstractHurtingProjectile.hurtServer` is
            // false; the projectile turned in `Projectile.onHit`, which already
            // ran), so the arrow takes `deflect(REVERSE, 0.2)` instead of vanishing.
            (
                ProjectileKind::Arrow { .. },
                Hit::Entity {
                    who: Who::Mob(id), ..
                },
            ) if pickable_projectile_hit(id).is_some() => {
                let mut arrow = p.clone();
                arrow.arrow_damage();
                self.finish_arrow_hit(arrow, EntityHit::Refused);
            }
            (ProjectileKind::Arrow { .. }, Hit::Entity { who, .. }) => {
                let mut arrow = p.clone();
                let victim = match who {
                    Who::Player => Some("minecraft:player".to_owned()),
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| m.kind.id.clone()),
                };
                let amount = self.shot_damage(&mut arrow, victim.as_deref(), who);
                let player_fire = self.player.as_ref().map(|pl| pl.fire_ticks);
                // AbstractArrow.onHitEntity saves the fire, ignites, then puts
                // it back when hurtOrSimulate fails and the target still takes
                // a projectile's side effects.
                let mob_fire = match who {
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| (m.join, m.fire_ticks)),
                    Who::Player => None,
                };
                let same_target = |mob: &Mob, id| {
                    mob.id == id
                        && !mob.removed
                        && mob_fire.is_some_and(|(join, _)| mob.join == join)
                };
                let mut dodged = false;
                // AbstractArrow.onHitEntity: a burning arrow sets its target alight.
                if p.arrow.as_ref().is_some_and(|a| a.on_fire()) {
                    self.handle_mob_events(vec![MobEvent::Ignite(who, 5)])?;
                }
                let hurt = match who {
                    Who::Player => {
                        let hurt = self.arrow_hits_player(amount, p);
                        // AbstractArrow.onHitEntity: a hit that hurt leaves a
                        // non-piercing arrow stuck in the body.
                        if hurt && p.arrow.as_ref().is_some_and(|a| (a.pierce as i8) <= 0) {
                            self.combat.stuck.add_arrow();
                        }
                        hurt
                    }
                    Who::Mob(id) => {
                        let hurt = self.damage_projectile_with_weapon(p, id, amount, source, aim);
                        dodged = !hurt
                            && self
                                .mobs
                                .iter()
                                .find(|m| same_target(m, id))
                                .is_some_and(|m| {
                                    m.species() == Species::Enderman
                                        && sunrise_client::mobs::enderman::dodges(m, &source)
                                });
                        if p.owner == Some(Who::Player) {
                            if hurt && let Some(m) = self.mobs.iter().find(|m| same_target(m, id)) {
                                // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                                let victim = self.mob_entity(m);
                                let spectral = p.arrow.as_ref().is_some_and(|a| {
                                    &*a.pickup_item.item == "minecraft:spectral_arrow"
                                });
                                let direct = sunrise_client::advancements::EntityView {
                                    kind: if spectral {
                                        "minecraft:spectral_arrow"
                                    } else {
                                        "minecraft:arrow"
                                    }
                                    .into(),
                                    position: [p.position.x, p.position.y, p.position.z],
                                    ..Default::default()
                                };
                                self.player_hit(&victim, "minecraft:arrow", Some(direct), amount);
                            }
                            // AbstractArrow.onHitEntity: KILLED_BY_ARROW for a kill
                            // (piercing arrows are not modelled).
                            if hurt
                                && let Some(m) = self.mobs.iter().find(|m| same_target(m, id))
                                && !m.alive()
                            {
                                let victim = self.mob_entity(m);
                                let weapon =
                                    p.arrow.as_ref().and_then(|a| a.fired_from_weapon.as_ref());
                                self.trigger_killed_by_arrow(&[victim], weapon);
                            }
                        }
                        hurt
                    }
                };
                let target_present = match who {
                    Who::Player => self.player.is_some(),
                    Who::Mob(id) => self.mobs.iter().any(|m| same_target(m, id)),
                };
                if target_present {
                    self.burning_arrow_primes(p, who);
                }
                // doKnockback: Punch pushes along the flight, less the target's
                // knockback resistance.
                let flight = (p.velocity.x * p.velocity.x + p.velocity.z * p.velocity.z).sqrt();
                let resistance = match who {
                    Who::Player => self
                        .player
                        .as_mut()
                        .and_then(|pl| pl.attribute("minecraft:knockback_resistance").ok())
                        .unwrap_or(0.0),
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map_or(0.0, |m| m.knockback_resistance()),
                };
                let knockback = if !hurt || !target_present {
                    0.0
                } else if let Some(weapon) =
                    p.arrow.as_ref().and_then(|a| a.fired_from_weapon.as_ref())
                {
                    let victim = match who {
                        Who::Player => Some("minecraft:player"),
                        Who::Mob(id) => self
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.kind.id.as_str()),
                    };
                    let owner = match p.owner {
                        Some(Who::Player) => Some("minecraft:player"),
                        Some(Who::Mob(id)) => self
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.kind.id.as_str()),
                        None => None,
                    };
                    let arrow_type = if p
                        .arrow
                        .as_ref()
                        .is_some_and(|a| &*a.pickup_item.item == "minecraft:spectral_arrow")
                    {
                        "minecraft:spectral_arrow"
                    } else {
                        "minecraft:arrow"
                    };
                    let ctx = sunrise_client::enchant::EffectContext {
                        this: victim,
                        attacker: owner,
                        direct_attacker: Some(arrow_type),
                        damage_type: Some("minecraft:arrow"),
                        items: Some(self.res.items()),
                        ..Default::default()
                    };
                    self.res.stations().enchantments.modify_value(
                        Some(weapon),
                        "minecraft:knockback",
                        &ctx,
                        &mut self.world.random,
                        0.0,
                    )
                } else {
                    p.bonus.punch as f32
                };
                let push = f64::from(knockback) * 0.6 * (1.0 - resistance).max(0.0);
                if hurt && push > 0.0 && flight > 1e-6 {
                    let k = push / flight;
                    self.handle_mob_events(vec![MobEvent::Push {
                        target: who,
                        velocity: [p.velocity.x * k, 0.1, p.velocity.z * k],
                    }])?;
                }
                if hurt && target_present {
                    let spectral = p
                        .arrow
                        .as_ref()
                        .is_some_and(|a| &*a.pickup_item.item == "minecraft:spectral_arrow");
                    let direct = if spectral {
                        "minecraft:spectral_arrow"
                    } else {
                        "minecraft:arrow"
                    };
                    self.shot_post_attack(p, who, direct, "minecraft:arrow");
                    self.arrow_post_hurt_effects(p, who);
                }
                match (hurt, who) {
                    (true, _) => self.finish_arrow_hit(arrow, EntityHit::Hurt),
                    // What the player took no damage from (a raised shield)
                    // keeps its fire as it was and turns the arrow back.
                    (false, Who::Player) => {
                        if let (Some(pl), Some(ticks)) = (self.player.as_mut(), player_fire) {
                            pl.set_remaining_fire_ticks(ticks);
                        }
                        self.finish_arrow_hit(arrow, EntityHit::Refused);
                    }
                    // EnderMan.hurtServer teleported away: the arrow flies on.
                    (false, Who::Mob(_)) if dodged => {
                        self.finish_arrow_hit(arrow, arrow_miss(true));
                    }
                    // hurtOrSimulate returned false and
                    // projectileReceivesSideEffectsOnHit(false) is the default
                    // (Shulker.hurtServer on a closed shell, invulnerability).
                    // The fire the arrow just set goes back, then
                    // deflect(REVERSE, entity, owner, false, 0.2). The hit
                    // sound plays only when the hurt lands.
                    (false, Who::Mob(id)) => {
                        if let Some((join, ticks)) = mob_fire
                            && let Some(m) = self
                                .mobs
                                .iter_mut()
                                .find(|m| m.id == id && m.join == join && !m.removed)
                        {
                            m.fire_ticks = ticks;
                        }
                        self.finish_arrow_hit(arrow, arrow_miss(false));
                    }
                }
            }
            // AbstractArrow.onHitBlock plays its sound itself (ArrowEvent).
            (ProjectileKind::Arrow { .. }, Hit::Block { .. }) => {}
            (ProjectileKind::Snowball, hit) => {
                if let Hit::Entity {
                    who: Who::Player, ..
                } = hit
                {
                    self.mob_hits_player(
                        0.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:thrown",
                    );
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(i) = self.mob_index.find(&self.mobs, id)
                {
                    // Snowball.onHitEntity: 3 damage to blazes, 0 otherwise.
                    let amount = if self.mobs[i].species() == Species::Blaze {
                        3.0
                    } else {
                        0.0
                    };
                    if self.damage_projectile_with_weapon(p, id, amount, source, aim) {
                        self.thrown_hurt_mob(p, id, "minecraft:snowball", amount);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    // Snowball.onHitEntity: 0 damage, which still shakes a
                    // boat or minecart (VehicleEntity.hurtServer).
                    self.hurt_vehicle(p, id, 0.0);
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, "minecraft:snowball");
            }
            // a mod's copy of Snowball: its damage to anything it hits.
            (ProjectileKind::Thrown(t), hit) => {
                match hit {
                    Hit::Entity {
                        who: Who::Mob(id), ..
                    } => {
                        // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                        if self.damage_projectile_with_weapon(p, id, t.damage, source, aim)
                            && p.owner == Some(Who::Player)
                            && let Some(m) = self.mobs.iter().find(|m| m.id == id)
                        {
                            let victim = self.mob_entity(m);
                            let direct = sunrise_client::advancements::EntityView {
                                kind: t.id.into(),
                                position: [p.position.x, p.position.y, p.position.z],
                                ..Default::default()
                            };
                            self.player_hit(&victim, "minecraft:thrown", Some(direct), t.damage);
                        }
                    }
                    // Player.hurtServer observes invulnerability before refusing 0 damage.
                    Hit::Entity {
                        who: Who::Player, ..
                    } => {
                        let from = [p.previous.x, p.previous.y, p.previous.z];
                        self.mob_hits_player(t.damage, from, "minecraft:thrown");
                    }
                    _ => {}
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, t.item);
            }
            (ProjectileKind::Egg, hit) => {
                if let Hit::Entity {
                    who: Who::Player, ..
                } = hit
                {
                    self.mob_hits_player(
                        0.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:thrown",
                    );
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && self.mob_index.find(&self.mobs, id).is_some()
                {
                    if self.damage_projectile_with_weapon(p, id, 0.0, source, aim) {
                        self.thrown_hurt_mob(p, id, "minecraft:egg", 0.0);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    // ThrownEgg.onHitEntity: the same 0 damage.
                    self.hurt_vehicle(p, id, 0.0);
                }
                // ThrownEgg.onHit: 1/8 chance of a chick, 1/32 of four.
                if self.world.random.next_int_bound(8).unwrap_or(1) == 0 {
                    let n = if self.world.random.next_int_bound(32).unwrap_or(1) == 0 {
                        4
                    } else {
                        1
                    };
                    for _ in 0..n {
                        if let Some(id) = self.spawn_mob_raw("minecraft:chicken", atp, true)
                            && let Some(c) = self.mobs.iter_mut().find(|m| m.id == id)
                        {
                            (c.extra.age, c.extra.variant) = (-24000, p.egg_variant.unwrap_or(0));
                        }
                    }
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, "minecraft:egg");
            }
            (ProjectileKind::EnderPearl, hit) => {
                if let Hit::Entity {
                    who: Who::Player, ..
                } = hit
                {
                    self.mob_hits_player(
                        0.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:thrown",
                    );
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && self.mob_index.find(&self.mobs, id).is_some()
                {
                    if self.damage_projectile_with_weapon(p, id, 0.0, source, aim) {
                        self.thrown_hurt_mob(p, id, "minecraft:ender_pearl", 0.0);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    // ThrownEnderpearl.onHitEntity: the same 0 damage.
                    self.hurt_vehicle(p, id, 0.0);
                }
                // To where the pearl started its last tick (oldPosition).
                let mut to = p.previous;
                let mut damage = 5.0;
                let mut cancel = false;
                // EntityTeleportEvent.EnderPearl: a server player's pearl.
                if p.owner == Some(Who::Player)
                    && let Some(from) = self.player.as_ref().map(|pl| pl.position)
                    && let Some(owner) = self.native_acting_player().map(|player| player.entity())
                {
                    let dimension = self.dim;
                    let target;
                    (target, damage, cancel) =
                        self.native_pearl_teleport(owner, p.id, from, to, damage, hit);
                    cancel |= self.dim != dimension
                        || self.player.as_ref().is_none_or(|pl| pl.position != from)
                        || self.native_acting_player().map(|player| player.entity()) != Some(owner);
                    // A target that is not a number keeps the pearl's.
                    if target.iter().all(|v| v.is_finite()) {
                        to = Vec3::new(target[0], target[1], target[2]);
                    }
                }
                // ThrownEnderpearl.isAllowedToTeleportOwner: an owner who is
                // dead or asleep cannot take the pearl, so it is discarded
                // where it lands (no teleport and no 5 damage).
                let taken = self.player.as_ref().is_some_and(|pl| {
                    sunrise_client::projectiles::owner_can_take_pearl(
                        !pl.health.dead_or_dying(),
                        self.sleeping.is_some(),
                    )
                });
                if p.owner == Some(Who::Player) && taken && !cancel {
                    // ThrownEnderpearl: 5% endermite where the player was
                    // (when monsters spawn), teleport, 5 damage.
                    if self.world.random.next_float() < 0.05
                        && self
                            .ui
                            .rules
                            .bool_at(sunrise_client::gamerules::vanilla::SPAWN_MONSTERS)
                        && self.ui.difficulty != 0
                        && let Some(pl) = self.player.as_ref()
                    {
                        let from = pl.position;
                        self.spawn_mob_raw("minecraft:endermite", [from.x, from.y, from.z], false);
                    }
                    // ServerPlayer.teleport: removeVehicle before the placement.
                    section_steps::teleport_player(self, to);
                    self.hurt_player_by("minecraft:ender_pearl", damage);
                    self.audio.play(
                        "minecraft:entity.player.teleport",
                        Some([to.x, to.y, to.z]),
                        1.0,
                        1.0,
                    );
                }
                if let Some(Who::Mob(id)) = p.owner {
                    self.mob_pearl_teleport(p, id, hit);
                }
            }
            (ProjectileKind::SmallFireball, hit) => match hit {
                Hit::Entity { who, .. } => match who {
                    Who::Player => {
                        // SmallFireball.onHitEntity: remember the fire, ignite,
                        // and restore it when the hurt is refused (a raised shield).
                        let remaining = self.player.as_ref().map(|p| p.fire_ticks);
                        if let Some(p) = self.player.as_mut() {
                            p.ignite_for_ticks(100)?;
                        }
                        let hurt = self.mob_hits_player(
                            5.0,
                            [p.previous.x, p.previous.y, p.previous.z],
                            "minecraft:fireball",
                        );
                        if !hurt
                            && let Some(remaining) = remaining
                            && let Some(pl) = self.player.as_mut()
                        {
                            pl.set_remaining_fire_ticks(remaining);
                        }
                    }
                    Who::Mob(id) => {
                        if let Some(i) = self.mob_index.find(&self.mobs, id) {
                            let m = &mut self.mobs[i];
                            let (join, remaining) = (m.join, m.fire_ticks);
                            if !m.fire_immune() {
                                m.ignite_for_ticks(100);
                            }
                            if self.damage_projectile_with_weapon(p, id, 5.0, source, aim) {
                                self.shot_post_attack(
                                    p,
                                    who,
                                    "minecraft:small_fireball",
                                    "minecraft:fireball",
                                );
                            } else if let Some(i) = self.mob_index.find(&self.mobs, id)
                                && self.mobs[i].join == join
                                && !self.mobs[i].removed
                            {
                                self.mobs[i].fire_ticks = remaining;
                            }
                        } else {
                            // SmallFireball.onHitEntity: 5 damage to a boat or minecart.
                            self.hurt_vehicle(p, id, 5.0);
                        }
                    }
                },
                Hit::Block { pos, face, .. } => {
                    // SmallFireball.onHitBlock: a mob's only with mobGriefing
                    // (NeoForge: `canEntityGrief` for the owner);
                    // BaseFireBlock.getState placed with updates (the fire
                    // ticks, ages and goes out where it can't stay).
                    let fire = [pos[0] + face[0], pos[1] + face[1], pos[2] + face[2]];
                    let may = match p.owner {
                        Some(Who::Mob(owner)) => self.can_entity_grief(Some(owner)),
                        _ => true,
                    };
                    if may
                        && self
                            .res
                            .facts()
                            .is_air(self.world.block(fire[0], fire[1], fire[2]))
                    {
                        let at = BlockPos::new(fire[0], fire[1], fire[2]);
                        let state =
                            self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, at));
                        self.level_set(fire, state)?;
                    }
                }
                Hit::None => {}
            },
            (ProjectileKind::LargeFireball { power }, hit) => {
                // Fireball explosions are caused by the shooter. A direct hit
                // on the player takes `projectile_owner`, so read the owner
                // from the fireball.
                let owner = match p.owner {
                    Some(Who::Mob(id)) => Some(id),
                    _ => None,
                };
                if let Hit::Entity { who, .. } = hit {
                    match who {
                        Who::Player => {
                            self.mob_hits_player(
                                6.0,
                                [p.previous.x, p.previous.y, p.previous.z],
                                "minecraft:fireball",
                            );
                        }
                        Who::Mob(id) => {
                            self.damage_projectile_with_weapon(p, id, 6.0, source, aim);
                            self.shot_post_attack(
                                p,
                                who,
                                "minecraft:fireball",
                                "minecraft:fireball",
                            );
                        }
                    }
                }
                self.combat.explosion_owner = owner;
                self.combat.explosion_in_water = p.touching_water;
                // The blast's source names the owner, so a dragon it kills
                // remembers the player who shot it.
                self.set_blast_by_player(p.owner == Some(Who::Player));
                // LargeFireball.onHit: the fire follows
                // `EventHooks.canEntityGrief(level, getOwner())`, so a mod's
                // answer for whoever owns the fireball decides: the ghast that
                // shot it, or the player who sent it back.
                let fire = self.large_fireball_fire(p.owner, p.guest);
                self.combat.explosion_direct = Some(p.id);
                let result = self.explode_mob(
                    atp,
                    *power as f32,
                    fire,
                    super::explosion::ExplosionSource::default()
                        .with_silence(self.entity_silent(p.id))
                        .with_entity(
                            p.id,
                            "minecraft:fireball",
                            sunrise_client::health::DirectEntity::LargeFireball,
                        ),
                );
                self.combat.explosion_direct = None;
                self.set_blast_by_player(false);
                self.combat.explosion_owner = None;
                self.combat.explosion_in_water = false;
                result?;
            }
            // AbstractThrownPotion.onHit, then onHitAsPotion from the stack it
            // flies as: the splash happens where the hit was, a lingering
            // potion's cloud sits at the mob it hit (ThrownLingeringPotion),
            // and the potion that lands names the hurt it deals.
            (kind @ (ProjectileKind::Potion(..) | ProjectileKind::ExperienceBottle), hit) => {
                // A bottle flies as the bottle item (`getDefaultItem`).
                let bottle;
                let stack = match kind {
                    ProjectileKind::Potion(stack, _) => stack,
                    _ => {
                        bottle = Box::new(ItemStack::new("minecraft:experience_bottle", 1));
                        &bottle
                    }
                };
                let block = match hit {
                    Hit::Block { pos, face, .. } => Some((
                        pos,
                        Direction::approximate_nearest(Vec3::new(
                            f64::from(face[0]),
                            f64::from(face[1]),
                            f64::from(face[2]),
                        )),
                    )),
                    _ => None,
                };
                let mob = match hit {
                    Hit::Entity {
                        who: Who::Player, ..
                    } => self.player.as_ref().map(|pv| pv.position),
                    Hit::Entity {
                        who: Who::Mob(id), ..
                    } => self.mobs.iter().find(|m| m.id == id).map(|m| m.position),
                    _ => None,
                };
                let owner = self.potion_owner(p.owner);
                let item = stack.item.to_string();
                self.splash(
                    stack,
                    at,
                    mob,
                    block,
                    p.age,
                    &owner,
                    self.entity_silent(p.id),
                    p.velocity,
                );
                if stack.item.as_ref() != "minecraft:experience_bottle" {
                    self.native_potion_splash(&item, [at.x, at.y, at.z]);
                }
            }
            // LlamaSpit.onHitEntity hurts only when the owner is a LivingEntity.
            (ProjectileKind::LlamaSpit, Hit::Entity { .. }) if p.owner.is_none() => {}
            (ProjectileKind::LlamaSpit, Hit::Entity { who, .. }) => match who {
                Who::Player => {
                    self.mob_hits_player(
                        1.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:spit",
                    );
                }
                Who::Mob(id) => {
                    if self.damage_projectile_with_weapon(p, id, 1.0, source, aim) {
                        self.shot_post_attack(p, who, "minecraft:llama_spit", "minecraft:spit");
                    }
                }
            },
            // ShulkerBullet.onHitEntity: 4 damage, then levitation for 10 s.
            (ProjectileKind::ShulkerBullet { .. }, Hit::Entity { who, .. }) => match who {
                Who::Player => {
                    // Only a hit that hurt (`wasHurt`) brings the levitation.
                    if self.mob_hits_player(
                        4.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:mob_projectile",
                    ) {
                        self.give_player_effect("minecraft:levitation", 200, 0);
                    }
                }
                Who::Mob(id) => {
                    let Some(index) = self.mob_index.find(&self.mobs, id) else {
                        return Ok(());
                    };
                    let join = self.mobs[index].join;
                    let same_target = |m: &Mob| m.id == id && m.join == join && !m.removed;
                    if self.damage_projectile_with_weapon(p, id, 4.0, source, aim) {
                        if let Some(m) = self.mobs.iter_mut().find(|m| same_target(m)) {
                            // Shulker.hurtServer: a shulker hit by a bullet.
                            sunrise_client::mobs::shulker::hit_by_bullet(m);
                        }
                        // The hurt's teleport (`teleportSomewhere`,
                        // `hitByShulkerBullet`) comes before the levitation.
                        self.settle_hurt_teleports()?;
                        self.shot_post_attack(
                            p,
                            who,
                            "minecraft:shulker_bullet",
                            "minecraft:mob_projectile",
                        );
                        if let Some(m) = self.mobs.iter_mut().find(|m| same_target(m)) {
                            m.add_effect("minecraft:levitation", 200, 0);
                        }
                    }
                }
            },
            // DragonFireball.onHit (its own owner's hit never gets here): the
            // dragon's breath where it burst.
            (ProjectileKind::DragonFireball, _) => self.dragon_fireball_on_hit(at),
            (ProjectileKind::WitherSkull { .. }, hit) => {
                let who = match hit {
                    Hit::Entity { who, .. } => Some(who),
                    _ => None,
                };
                self.wither_skull_hit(p, who, atp)?;
            }
            (ProjectileKind::ShulkerBullet { .. }, Hit::Block { .. }) => {
                self.audio
                    .play("minecraft:entity.shulker_bullet.hit", Some(atp), 1.0, 1.0);
                if let Some(ty) = sunrise_client::particles::ParticleType::from_name("explosion") {
                    let options = sunrise_client::particles::ParticleOptions::simple(ty);
                    for _ in 0..2 {
                        let r = &mut self.world.random;
                        let at = [
                            atp[0] + r.next_gaussian() * 0.2,
                            atp[1] + r.next_gaussian() * 0.2,
                            atp[2] + r.next_gaussian() * 0.2,
                        ];
                        self.particles.add(&options, at, [0.0; 3]);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// `Snowball`/`ThrownEgg.onHit`: entity event 3 for the guests, before the
    /// projectile is discarded (the clients lose it when the tracker next runs).
    fn thrown_break_event(&mut self, id: u32, at: [f64; 3]) {
        if id != 0 {
            self.lan_entity_event(id as i32, 3, at);
        }
    }
    /// `Snowball`/`ThrownEgg.handleEntityEvent(3)`: eight crumbs of the
    /// thrown item where it broke, at rest for a snowball, scattered by the
    /// client entity's random (the particle engine's here) for an egg.
    pub(super) fn item_burst(&mut self, at: [f64; 3], item: &str) {
        let options = self.item_particle(&sunrise_client::items::ItemStack::new(item, 1));
        let egg = item == "minecraft:egg";
        for _ in 0..8 {
            let v = if egg {
                let r = self.particles.random();
                [
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                ]
            } else {
                [0.0; 3]
            };
            self.particles.add(&options, at, v);
        }
    }
    /// `AgeableMob.mobInteract`: whether a click with `item` on `m` gets to
    /// the golden dandelion check (`canUseGoldenDandelion`, which a tadpole
    /// calls with `isBaby` true). A panda that is scared or on its back and an
    /// armadillo in its shell answer in their own `mobInteract` first.
    pub(super) fn age_lock_takes(&self, m: &Mob, item: &str) -> bool {
        let species = m.species();
        let baby = if species == Species::Tadpole {
            true
        } else if sunrise_client::mobs::finalize::ageable(species) {
            m.is_baby()
                && !(species == Species::Panda
                    && (sunrise_client::mobs::panda::scared(m)
                        || sunrise_client::mobs::panda::is_on_back(m)))
                && !(species == Species::Armadillo && sunrise_client::mobs::armadillo::scared(m))
        } else {
            false
        };
        let cannot = self
            .items()
            .entity_type_tags
            .contains("minecraft:cannot_be_age_locked", &m.kind.id);
        sunrise_client::mobs::ai::can_use_golden_dandelion(
            item,
            baby,
            m.extra.age_lock_particle_timer,
            cannot,
        )
    }
    /// `AgeableMob.setAgeLocked`: a golden dandelion on mob `index` flips its
    /// age lock (a tadpole's or an ageable mob's own `setAgeLockedData`) and is
    /// used up; a locked mob stays (`setPersistenceRequired`), and the lock's
    /// use or unuse sound plays at its block for everyone near.
    pub(super) fn set_age_locked(&mut self, index: usize) {
        let m = &mut self.mobs[index];
        if m.species() == Species::Tadpole {
            sunrise_client::mobs::aquatic::set_age_locked_data(m);
        } else {
            sunrise_client::mobs::ai::set_age_locked_data(m);
        }
        self.consume_held();
        let m = &mut self.mobs[index];
        let locked = m.extra.age_locked;
        if locked {
            m.persistent = true;
        }
        // Level.playSound(null, blockPosition, ..): the middle of the block.
        let at = [
            m.position.x.floor() + 0.5,
            m.position.y.floor() + 0.5,
            m.position.z.floor() + 0.5,
        ];
        let chunk = (
            m.position.x.floor() as i32 >> 4,
            m.position.z.floor() as i32 >> 4,
        );
        self.world.modified.insert(chunk);
        let sound = if locked {
            "minecraft:item.golden_dandelion.use"
        } else {
            "minecraft:item.golden_dandelion.unuse"
        };
        self.audio.play(sound, Some(at), 1.0, 1.0);
    }
    /// The mob a normal interaction can reach: a packet's exact validated ID,
    /// or the crosshair's mob before its blocking shape.
    pub(in crate::game) fn mob_interaction_target(
        &self,
        target: Option<InteractTarget>,
    ) -> Option<usize> {
        let (index, distance, _) = (match target {
            // The id the packet named, under the filters the crosshair pick
            // applies (`mob_hit`): a mob in its death animation is not clicked
            // by either path, so naming one in a packet must not reach the
            // feed, shear, leash or name paths either.
            Some(t) => Self::named_entity(&self.mobs, t.id, |m| m.id)
                .filter(|&i| {
                    let m = &self.mobs[i];
                    named_mob_pickable(m.alive(), m.id, self.riding, self.remote_camera_id())
                })
                .map(|i| (i, 0.0, [0.0; 3])),
            None => self.mob_hit(),
        })?;
        // Only a client's own pick is stopped by a block: the range check passed.
        if target.is_none()
            && !self.uses_stored_pick()
            && let Some(hit) = self.pick_unfiltered()
            && let Some(eye) = self.eye_position()
        {
            let d = ((hit.point[0] - eye[0]).powi(2)
                + (hit.point[1] - eye[1]).powi(2)
                + (hit.point[2] - eye[2]).powi(2))
            .sqrt();
            if d < distance {
                return None;
            }
        }
        Some(index)
    }
    /// Right click on a mob (Mob.interact / mobInteract), on the mob under the
    /// crosshair or on the one a guest's interact packet named: returns true
    /// when consumed.
    pub(super) fn interact_mob(&mut self, target: Option<InteractTarget>) -> Result<bool> {
        let Some(mut index) = self.mob_interaction_target(target) else {
            return Ok(false);
        };
        // CamelHusk.interact: any click on it keeps it (`setPersistenceRequired`).
        sunrise_client::mobs::camel::keep_on_interact(&mut self.mobs[index]);
        // PlayerInteractTrigger: the name tag as it was before it named the mob.
        let used = self.held().cloned();
        if self.name_mob(index) {
            if let Some(stack) = &used {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            return Ok(true);
        }
        // Shears ask the mob's entity parts first, then fire `player_shear_entity`.
        if self.held_shears() {
            let id = self.mobs[index].id;
            if let Some(consumed) = self.player_shear(index) {
                return Ok(consumed);
            }
            // The shear hook may have moved the list: the mob by its id.
            match self.mobs.iter().position(|m| m.id == id) {
                Some(now) => index = now,
                None => return Ok(true),
            }
        }
        if self.spawn_egg_offspring(index) {
            self.swing_arm();
            return Ok(true);
        }
        {
            let item = self.held().map(|h| h.item.to_string()).unwrap_or_default();
            let used = self.held().cloned();
            if let Some(context) = self.interact_leash_with_item_context(index, &item) {
                let used = match context {
                    super::leads::LeashInteraction::WithItem => used,
                    super::leads::LeashInteraction::WithoutItem => {
                        Some(ItemStack::new("minecraft:air", 0))
                    }
                };
                if let Some(stack) = &used {
                    let target = self.mob_entity(&self.mobs[index]);
                    self.trigger_item_entity(
                        "minecraft:player_interacted_with_entity",
                        stack,
                        &target,
                    );
                }
                self.swing_arm();
                return Ok(true);
            }
        }
        let held = self.held().cloned();
        let item = held
            .as_ref()
            .map(|h| h.item.to_string())
            .unwrap_or_default();
        let shears = self.held_shears();
        // Bucketable.bucketMobPickup: a water bucket scoops up a fish, a
        // tadpole or an axolotl, with the data its bucket carries.
        if item == "minecraft:water_bucket"
            && self.mobs[index].alive()
            && let Some((bucket, sound)) = bucketable(self.mobs[index].species())
        {
            let mut filled = ItemStack::new(bucket, 1);
            filled.extra = Some(Box::new(sunrise_client::entity_nbt::bucket_components(
                &self.mobs[index],
            )));
            self.audio
                .play(sound, Some(self.mobs[index].center()), 1.0, 1.0);
            self.trigger_item("minecraft:filled_bucket", &filled);
            // PlayerInteractTrigger: the water bucket as it was, and the mob picked up.
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            let species = bucket.trim_end_matches("_bucket").to_owned();
            let id = self.mobs[index].id as i32;
            // Leashable.dropLeash, then discard.
            self.break_leash(id as u32, true, false);
            self.remove_mob_now(id as u32, false);
            if self.creative {
                if let Some(rest) = self.add_stack_to_inventory(filled) {
                    self.spawn_item(rest, self.eye_position().unwrap_or([0.0; 3]), [0.0; 3], 40);
                }
            } else {
                *self.held_mut() = Some(filled);
            }
            if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
            {
                self.native_player_bucket_entity(player, &name, id, &species);
            }
            self.swing_arm();
            return Ok(true);
        }
        // AgeableMob.mobInteract: a golden dandelion on a baby freezes its
        // growth, or lets it grow again.
        if self.mobs[index].alive() && self.age_lock_takes(&self.mobs[index], &item) {
            let entity_id = self.mobs[index].id;
            self.set_age_locked(index);
            if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
            {
                self.native_player_toggle_entity_age_lock(
                    player,
                    &name,
                    entity_id,
                    self.mobs[index].extra.age_locked,
                );
            }
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        let creative = self.creative;
        // The hand's slot (Player.interactOn passes the hand).
        let selected = self.held_slot();
        // Panda.mobInteract: bamboo, or a click on a panda lying on its back.
        if self.mobs[index].species() == Species::Panda && self.mobs[index].alive() {
            let taken = self.interact_panda(index, &item);
            if taken {
                self.swing_arm();
            }
            return Ok(taken);
        }
        if self.interact_zombie_villager(index, &item) {
            return Ok(true);
        }
        if self.interact_trader(index, &item)? {
            self.swing_arm();
            return Ok(true);
        }
        if !self.mobs[index].kind.food.contains(&item)
            && self.interact_sheep_dye(index, self.hand, false)
        {
            // PlayerInteractTrigger: the dye as it was before it coloured the sheep.
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_allay(index, held.as_ref()) {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_sulfur_cube(index, &item) {
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_armadillo(index, &item) {
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_copper_golem(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_shear_mob(index, shears) {
            self.swing_arm();
            return Ok(true);
        }
        if let Some(sheared) = self.shear_saddle(index, &item) {
            if sheared {
                self.swing_arm();
            }
            return Ok(sheared);
        }
        // AbstractHorse.mobInteract: feed, equip or ride.
        if self.interact_mount(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        // PiglinAi.mobInteract: hand over barter currency to admire.
        if self.mobs[index].alive() && held.as_ref().is_some_and(|h| self.give_piglin(index, h)) {
            self.swing_arm();
            if !creative {
                self.consume_player_slot(selected, 1);
            }
            return Ok(true);
        }
        // Creeper.mobInteract: any item in #creeper_igniters, damaged when it
        // is damageable and used up otherwise.
        let creeper_igniter = self
            .res
            .items()
            .item_tags
            .get("minecraft:creeper_igniters")
            .is_some_and(|t| t.contains(&*item));
        let fish = self
            .res
            .items()
            .item_tags
            .get("minecraft:fishes")
            .is_some_and(|t| t.contains(&*item));
        let damageable = self
            .res
            .items()
            .get(&item)
            .is_some_and(|i| i.max_damage > 0);
        // Wolf.mobInteract: the owner's #wolf_collar_dyes item only ever dyes the
        // collar; one of the colour the collar already has does nothing.
        let wolf_collar_dye = self
            .res
            .items()
            .item_tags
            .get("minecraft:wolf_collar_dyes")
            .is_some_and(|t| t.contains(&*item));
        let parrot_poisonous = self
            .res
            .items()
            .item_tags
            .get("minecraft:parrot_poisonous_food")
            .is_some_and(|t| t.contains(&*item));
        let player_at = self.eye_position();
        let player_feet = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        let acting = self.acting_player_uuid();
        let (acting_id, acting_entity_id) = (self.native_acting_id(), self.own_entity_id());
        let sneaking = self.held_mapping(Action::Sneak)
            || self.keys.raw(&winit::keyboard::KeyCode::ShiftRight);
        let held_damage = held.as_ref().map_or(0, |h| h.damage as i32);
        let held_extra = held.as_ref().and_then(|h| h.extra.clone());
        let feeder = sunrise_client::health::Attack {
            player_uuid: self.player_uuid_of(self.lan_acting),
            ..Default::default()
        };
        let credit_plan =
            self.prepare_knockback_credit(self.mobs[index].id as i32, Some(Who::Player), &feeder);
        let m = &mut self.mobs[index];
        if !m.alive() {
            return Ok(false);
        }
        let species = m.species();
        let center = m.center();
        // Wolf armor the owner takes off with shears or leaves a dropped stack of.
        let mut armor_drop: Option<ItemStack> = None;
        let mut equip_event: Option<bool> = None;
        let mut consume = false;
        let mut give: Option<&str> = None;
        let mut sound: Option<(&str, f32, f32)> = None;
        // Sounds played as vanilla plays them: (event, volume, pitch) at the feet.
        let mut eat: Option<(&'static str, f32, f32)> = None;
        let feet = [m.position.x, m.position.y, m.position.z];
        let mut handled = true;
        let mut ignited_creeper = false;
        let mut clear_wolf_target = false;
        let mut drops: Vec<(String, u32)> = Vec::new();
        // The shearing table rolled, for the mods.
        let mut drops_table = String::new();
        let mob = (m.id, super::native_hooks::mob_kind(m));
        // `Sheep.shear` adds a spread on top of the ItemEntity's own motion.
        let mut wool = false;
        let is_food = m.kind.food.contains(&item);
        let baby_turtle = species == Species::Turtle && m.is_baby();
        // MushroomCow.getEffectsFromItemStack: the item's block supplies them.
        let stew_effects = (species == Species::Mooshroom && m.extra.variant == 1 && !m.is_baby())
            .then(|| world_items!(self).suspicious_stew_effects(&item))
            .flatten();
        let mut give_stew: Option<Vec<(Arc<str>, i32)>> = None;
        // `Some(true)` for the four spell particles, `Some(false)` for the two
        // smoke puffs of a cow that already holds effects.
        let mut stew_particles: Option<bool> = None;
        // `Ocelot.mobInteract`: whether the fish won its trust.
        let mut ocelot_trust: Option<bool> = None;
        match species {
            Species::Cow | Species::Mooshroom if item == "minecraft:bucket" && !m.is_baby() => {
                give = Some("minecraft:milk_bucket");
                consume = true;
                sound = Some(("entity.cow.milk", 1.0, 1.0));
            }
            // Goat.mobInteract: a bucket on an adult gives milk.
            Species::Goat if item == "minecraft:bucket" && !m.is_baby() => {
                give = Some("minecraft:milk_bucket");
                consume = true;
                let milk = if m.extra.screaming {
                    "entity.goat.screaming.milk"
                } else {
                    "entity.goat.milk"
                };
                sound = Some((milk, 1.0, 1.0));
            }
            // MushroomCow.mobInteract: a bowl gives back what a flower left as
            // a suspicious stew, or plain stew when it holds no effects.
            Species::Mooshroom if item == "minecraft:bowl" && !m.is_baby() => {
                consume = true;
                match m.extra.stew_effects.take() {
                    Some(effects) => {
                        give = Some("minecraft:suspicious_stew");
                        give_stew = Some(effects);
                        // `playSound(.., 1.0F, 1.0F)` at the feet.
                        eat = Some(("entity.mooshroom.suspicious_milk", 1.0, 1.0));
                    }
                    None => {
                        give = Some("minecraft:mushroom_stew");
                        sound = Some(("entity.mooshroom.milk", 1.0, 1.0));
                    }
                }
            }
            Species::Mooshroom if shears && !m.is_baby() => {
                let player = Some(self.native_acting_id());
                self.shear_mooshroom(index, player, held.as_ref().expect("held shears"));
                let ctx = self.who_context(Some(sunrise_client::mobs::Who::Player), None);
                self.post_game_event("minecraft:shear", feet, ctx);
                self.damage_held_tool();
            }
            // MushroomCow.mobInteract: a brown adult takes the block's effects
            // unless it holds some already, when it only puffs smoke.
            Species::Mooshroom if let Some(effects) = stew_effects => {
                if m.extra.stew_effects.is_some() {
                    stew_particles = Some(false);
                } else {
                    m.extra.stew_effects = Some(effects.to_vec());
                    consume = true;
                    eat = Some(("entity.mooshroom.eat", 2.0, 1.0));
                    stew_particles = Some(true);
                }
            }
            Species::Sheep if shears && !m.extra.sheared && !m.is_baby() => {
                self.audio.play_bound(
                    "minecraft:entity.sheep.shear",
                    7,
                    1.0,
                    1.0,
                    sunrise_client::sound_engine::SoundEntity::new(m.id, feet, m.silent),
                    None,
                );
                m.extra.sheared = true;
                let color = sunrise_client::mobs::render::DYE_NAMES[m.extra.color as usize & 15];
                drops_table = format!("minecraft:shearing/sheep/{color}");
                let stacks = world_items!(self).loot_table(
                    &drops_table,
                    false,
                    None,
                    &mut self.world.random,
                );
                drops.extend(stacks.into_iter().map(|s| (s.item.to_string(), s.count)));
                wool = true;
                let ctx = self.who_context(Some(sunrise_client::mobs::Who::Player), None);
                self.post_game_event("minecraft:shear", feet, ctx);
                self.damage_held_tool();
            }
            // Entity.interact: shears take a tamed wolf's armor off for its owner.
            Species::Wolf
                if item == "minecraft:shears"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.body_id().is_some()
                    && !sneaking =>
            {
                armor_drop = m.equipment_stacks()[6].clone();
                m.extra.set_body_stack(None);
                eat = Some(("item.armor.unequip_wolf", 1.0, 1.0));
                equip_event = Some(false);
                self.damage_held_tool();
            }
            // Wolf.mobInteract: heal, dye the collar or sit/stand when tame;
            // a bone tames a wild one a third of the time.
            Species::Wolf if m.extra.tame && is_food && m.health < m.attrs.max_health => {
                let heal = self
                    .res
                    .items()
                    .get(&item)
                    .and_then(|i| i.food.as_ref())
                    .map_or(2.0, |f| f.0 as f32 * 2.0);
                // TamableAnimal.feed(.., 2.0F, 2.0F): heals twice the nutrition,
                // then playEatingSound, which wolves leave empty.
                m.heal(&self.loop_context, heal);
                consume = true;
            }
            // Wolf.mobInteract: the owner puts wolf armor on a tamed adult.
            Species::Wolf
                if item == "minecraft:wolf_armor"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.body_id().is_none()
                    && !m.is_baby() =>
            {
                let mut body = ItemStack::new(item.as_str(), 1);
                body.damage = held_damage.max(0) as u32;
                body.extra = held_extra;
                m.extra.set_body_stack(Some(body));
                consume = true;
                eat = Some(("item.armor.equip_wolf", 1.0, 1.0));
                equip_event = Some(true);
            }
            // Wolf.mobInteract: an armadillo scute repairs a sitting wolf's
            // armor by an eighth of its durability.
            Species::Wolf
                if item == "minecraft:armadillo_scute"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.sitting
                    && m.extra.body_id().is_some()
                    && m.extra.body_damage() > 0 =>
            {
                m.extra.set_body_damage(
                    m.extra.body_damage()
                        - (sunrise_client::mobs::WOLF_ARMOR_DURABILITY as f32 * 0.125) as i32,
                );
                consume = true;
                eat = Some(("item.wolf_armor.repair", 1.0, 1.0));
            }
            Species::Wolf
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && item.ends_with("_dye")
                    && sunrise_client::mobs::render::DYE_NAMES
                        .iter()
                        .position(|d| {
                            item.trim_start_matches("minecraft:")
                                .trim_end_matches("_dye")
                                == *d
                        })
                        .is_some_and(|c| c as u8 != m.extra.color) =>
            {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = sunrise_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                {
                    m.extra.color = c as u8;
                }
                consume = true;
            }
            // Wolf.mobInteract: Animal.mobInteract first breeds an adult or
            // grows a baby with the food; only when it did not does the owner
            // make the pet sit or stand.
            Species::Wolf
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && !wolf_collar_dye
                    && !(is_food && takes_food(m)) =>
            {
                m.extra.sitting = !m.extra.sitting;
                m.ai.jumping = false;
                m.nav_stop();
                clear_wolf_target = true;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            Species::Wolf
                if !m.extra.tame
                    && item == "minecraft:bone"
                    && !sunrise_client::mobs::tame::angry(m) =>
            {
                consume = true;
                let rolled = m.random.next_int_bound(3).unwrap_or(1) == 0;
                let tamed =
                    rolled && !self.animal_tame_cancelled(index, acting_id, acting_entity_id);
                let m = &mut self.mobs[index];
                if tamed {
                    sunrise_client::mobs::tame::set_tame(m, true);
                    m.extra.owner = Some(acting);
                    m.persistent = true;
                    m.nav_stop();
                    clear_wolf_target = true;
                    m.extra.sitting = true;
                }
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                    self.pet_tamed(index);
                }
                let m = &self.mobs[index];
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // Cat.mobInteract: dye, heal or sit when tame; fish may tame a wild one.
            Species::Cat
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && item.ends_with("_dye")
                    && sunrise_client::mobs::render::DYE_NAMES
                        .iter()
                        .position(|d| {
                            item.trim_start_matches("minecraft:")
                                .trim_end_matches("_dye")
                                == *d
                        })
                        .is_some_and(|c| c as u8 != m.extra.color) =>
            {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = sunrise_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                {
                    m.extra.color = c as u8;
                }
                m.persistent = true;
                consume = true;
            }
            Species::Cat
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && is_food
                    && m.health < m.attrs.max_health =>
            {
                let heal = self
                    .res
                    .items()
                    .get(&item)
                    .and_then(|i| i.food.as_ref())
                    .map_or(1.0, |f| f.0 as f32);
                // TamableAnimal.feed.
                m.heal(&self.loop_context, heal);
                consume = true;
                eat = eating_sound(m, &mut self.world.random);
            }
            // Cat.mobInteract: Animal.mobInteract first breeds an adult or
            // grows a baby with the food; only when it did not does the owner
            // make the pet sit or stand.
            Species::Cat
                if m.extra.tame && m.extra.owner == Some(acting) && !(is_food && takes_food(m)) =>
            {
                m.extra.sitting = !m.extra.sitting;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // Parrot.mobInteract: the owner makes a tame parrot on the ground
            // sit or stand (poisonous food is handled apart).
            Species::Parrot
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.ground.on_ground
                    && !parrot_poisonous =>
            {
                m.extra.sitting = !m.extra.sitting;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            Species::Cat if !m.extra.tame && is_food => {
                consume = true;
                m.persistent = true;
                let rolled = m.random.next_int_bound(3).unwrap_or(1) == 0;
                let tamed =
                    rolled && !self.animal_tame_cancelled(index, acting_id, acting_entity_id);
                let m = &mut self.mobs[index];
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = Some(acting);
                    m.extra.sitting = true;
                }
                eat = eating_sound(m, &mut self.world.random);
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                    self.pet_tamed(index);
                }
                let m = &self.mobs[index];
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // IronGolem.mobInteract: an iron ingot repairs 25 health.
            Species::IronGolem
                if item == "minecraft:iron_ingot" && m.health < m.attrs.max_health =>
            {
                m.heal(&self.loop_context, 25.0);
                consume = true;
                eat = Some(("entity.iron_golem.repair", 1.0, m.golem_repair_pitch()));
            }
            Species::Creeper if creeper_igniter => {
                m.extra.flag = true;
                ignited_creeper = true;
                // FIRECHARGE_USE for a fire charge, else FLINTANDSTEEL_USE,
                // at the creeper's feet with a pitch from its own random.
                let event = if item == "minecraft:fire_charge" {
                    "item.firecharge.use"
                } else {
                    "item.flintandsteel.use"
                };
                eat = Some((event, 1.0, m.random.next_float() * 0.4 + 0.8));
                if damageable {
                    self.damage_held_tool();
                } else {
                    consume = true;
                }
            }
            // Parrot.mobInteract: #parrot_poisonous_food (a cookie) poisons the
            // parrot for 45 s and then hurts it with a player attack of
            // Float.MAX_VALUE, which an invulnerable one shrugs off.
            Species::Parrot
                if self
                    .res
                    .items()
                    .item_tags
                    .get("minecraft:parrot_poisonous_food")
                    .is_some_and(|t| t.contains(&*item)) =>
            {
                consume = true;
                m.add_effect("minecraft:poison", 900, 0);
                let id = m.id;
                let hurt = m.damage(
                    crate::game::entity::damage_level_of!(
                        self,
                        &sunrise_client::health::NO_ATTACK,
                        &mut credit_plan.lend(&mut self.last_hurt_by_player),
                        None
                    ),
                    f32::MAX,
                    sunrise_client::mobs::Damage::player(player_at.unwrap_or(center)),
                );
                // The feeder gets the kill, as for a normal player attack.
                if hurt {
                    self.remember_mob_player_credit(id);
                }
                self.finish_mob_damage_at(index);
            }
            // Parrot.mobInteract: parrot food may tame a wild one (one time in
            // ten); a parrot never takes it as Animal food (isFood is false).
            Species::Parrot if !m.extra.tame && is_food => {
                consume = true;
                if !m.silent {
                    let pitch = 1.0 + (m.random.next_float() - m.random.next_float()) * 0.2;
                    eat = Some(("entity.parrot.eat", 1.0, pitch));
                }
                let rolled = m.random.next_int_bound(10).unwrap_or(1) == 0;
                let tamed =
                    rolled && !self.animal_tame_cancelled(index, acting_id, acting_entity_id);
                let m = &mut self.mobs[index];
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = Some(acting);
                }
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                    self.pet_tamed(index);
                }
            }
            // Dolphin.mobInteract: a fish makes a baby grow by a tenth and an
            // adult remember it (`gotFish`), to swim to a treasure.
            Species::Dolphin if fish => {
                eat = Some(("entity.dolphin.eat", 1.0, 1.0));
                if sunrise_client::mobs::ai::can_age_up(m) {
                    let secs = ((-m.extra.age / 20) as f32 * 0.1) as i32;
                    sunrise_client::mobs::ai::age_up(m, secs, true);
                } else {
                    m.extra.flag = true;
                }
                consume = true;
            }
            // Tadpole.feed: frog food ages it by a tenth of the time left to
            // grow into a frog (it grows on its next tick once past 24000).
            Species::Tadpole if is_food && !m.extra.age_locked => {
                consume = true;
                let left = 24_000 - m.extra.age;
                m.extra.age += ((left / 20) as f32 * 0.1) as i32 * 20;
                // One happy villager particle at rest about the body.
                self.particles_around_mob(
                    self.mobs[index].id,
                    "happy_villager",
                    sunrise_client::particles::Burst::HORSE_EATING,
                );
            }
            // Bee.mobInteract: a wither rose or an open eyeblossom is eaten for
            // its `getBeeInteractionEffect` instead of breeding the bee.
            Species::Bee
                if is_food
                    && matches!(
                        &*item,
                        "minecraft:wither_rose" | "minecraft:open_eyeblossom"
                    ) =>
            {
                if &*item == "minecraft:wither_rose" {
                    m.add_effect("minecraft:wither", 40, 0);
                } else {
                    m.add_effect("minecraft:poison", 25, 0);
                }
                consume = true;
            }
            // Ocelot.mobInteract: raw fish from a hand within 3 blocks, while it comes for the
            // food (`temptGoal.isRunning`), is used up; one time in three it trusts the player
            // (entity event 41, hearts), otherwise smoke (event 40). Anything else is
            // `Animal.mobInteract`.
            Species::Ocelot
                if is_food
                    && !m.extra.trusting
                    && m.ai.goals.is_running("OcelotTemptGoal")
                    && player_feet.is_some_and(|p| {
                        let d = [
                            p[0] - m.position.x,
                            p[1] - m.position.y,
                            p[2] - m.position.z,
                        ];
                        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 9.0
                    }) =>
            {
                consume = true;
                let trusts = m.random.next_int_bound(3).unwrap_or(1) == 0;
                if trusts {
                    m.extra.trusting = true;
                }
                ocelot_trust = Some(trusts);
            }
            // HappyGhast.canFallInLove: never, so only a ghastling takes food.
            _ if is_food
                && m.kind.flags.animal
                && species != Species::Parrot
                && (species != Species::HappyGhast || m.is_baby()) =>
            {
                // Animal.mobInteract: breed adults, speed up babies, then
                // playEatingSound (silent unless the species overrides it).
                match feed_animal(m, &mut self.world.random, Some(acting)) {
                    Some(sound) => {
                        consume = true;
                        eat = sound;
                        // Axolotl.usePlayerItem: a tropical fish bucket is
                        // emptied into a water bucket in its place.
                        if species == Species::Axolotl && &*item == "minecraft:tropical_fish_bucket"
                        {
                            give = Some("minecraft:water_bucket");
                        }
                        // Animal.setInLove: entity event 18 (a baby that
                        // only grew is not in love).
                        if self.mobs[index].extra.in_love > 0 {
                            let id = self.mobs[index].id;
                            self.love_cause.insert(id, self.lan_acting);
                            let kind = self.mobs[index].kind.id.clone();
                            self.native_entity_enter_love_mode(id, &kind, self.lan_acting_uuid);
                            self.mob_event_particles(id, 18);
                        }
                    }
                    // Animal.mobInteract still consumes the client click when
                    // an adult cannot breed, without swinging or taking food.
                    None => return Ok(true),
                }
            }
            _ => handled = false,
        }
        if !handled {
            return Ok(false);
        }
        if clear_wolf_target {
            self.set_event_ai_target(mob.0, None, self.lan_acting);
        }
        if ignited_creeper {
            self.native_creeper_ignite(
                self.mobs[index].id,
                sunrise_client::lan::playerdata::ints_uuid(acting),
                &item,
            );
        }
        if let Some(trusts) = ocelot_trust {
            // `spawnTrustingParticles` on the clients: entity event 41 (hearts) or 40 (smoke).
            self.mob_event_particles(self.mobs[index].id, if trusts { 41 } else { 40 });
        }
        if let Some(fresh) = stew_particles {
            // `SpellParticleOption.create(EFFECT, -1, 1)` is the plain white
            // effect particle; the smoke is the already-fed answer.
            let ty = if fresh {
                ParticleType::Effect
            } else {
                ParticleType::Smoke
            };
            let options = ParticleOptions::simple(ty);
            let at = [feet[0], feet[1] + 0.5, feet[2]];
            let (spread, count) = ([0.5, 0.0, 0.5], if fresh { 4 } else { 2 });
            // shortcut: scalar speed models DEFAULT randomization; 26.3 sends
            // ALTERNATIVE_WITH_SPEED with a per-axis maximum speed.
            self.particles
                .send_particles(&options, at, spread, 0.2, count);
            self.lan_send_particles(super::lan_particles::QueuedParticle {
                dimension: self.dim.id(),
                options,
                state: None,
                at,
                spread,
                speed: [0.2; 3],
                count: count as i32,
                randomization: 0,
            });
        }
        // PlayerInteractTrigger: the item used on the mob, once the interaction took.
        if let Some(stack) = &held {
            let target = self.mob_entity(&self.mobs[index]);
            self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
        }
        // Turtle.ageBoundaryReached: food that finishes a baby's growth.
        if baby_turtle && !self.mobs[index].is_baby() {
            self.handle_mob_events(vec![MobEvent::GiftLoot {
                mob: self.mobs[index].id,
                table: "minecraft:gameplay/turtle_grow",
                at: feet,
            }])?;
        }
        self.swing_arm();
        match sound {
            // Sheep.shear: level.playSound(null, this, SHEEP_SHEAR, PLAYERS, 1, 1).
            // DyeItem.interactLivingEntity: level.playSound(player, sheep,
            // DYE_USE, PLAYERS, 1, 1), whose player is left out because their
            // client played it; Sunrise's own client plays it from the packet,
            // so every player gets it.
            Some((s @ ("entity.sheep.shear" | "item.dye.use"), volume, pitch)) => {
                let name = full_id(s);
                let id = self.mobs[index].id;
                let sound =
                    LevelSound::new(&name, feet, volume, pitch).in_source(SoundSource::Players);
                self.play_bound_sound(sound, id, None);
            }
            Some((s, volume, pitch)) => {
                self.audio.play(&full_id(s), Some(feet), volume, pitch);
            }
            None => {}
        }
        if let Some((s, volume, pitch)) = eat {
            let name = full_id(s);
            if eating_sound_is_bound(species) {
                let id = self.mobs[index].id;
                let sound =
                    LevelSound::new(&name, feet, volume, pitch).in_source(SoundSource::Neutral);
                self.play_bound_sound(sound, id, None);
            } else {
                self.audio.play(&name, Some(feet), volume, pitch);
            }
        }
        if consume && !creative {
            self.consume_player_slot(selected, 1);
        }
        if let Some(g) = give {
            let mut stack = ItemStack {
                item: g.into(),
                count: 1,
                damage: 0,
                extra: None,
            };
            if let Some(effects) = give_stew {
                stack.data_mut().stew_effects = effects;
            }
            if self.stack_in(selected).is_none() {
                *self.stack_in_mut(selected) = Some(stack);
            } else if let Some(rest) = self.add_stack_to_inventory(stack) {
                let at = self.eye_position().unwrap_or(center);
                self.spawn_item(rest, at, [0.0, 0.0, 0.0], 40);
            }
        }
        if let Some(stack) = armor_drop {
            // Mob.shearItem: PLAYER_SHEARED_EQUIPMENT with the armor and the wolf.
            let target = self.mob_entity(&self.mobs[index]);
            self.trigger_item_entity("minecraft:player_sheared_equipment", &stack, &target);
            self.spawn_item(stack, [center[0], center[1] + 0.5, center[2]], [0.0; 3], 10);
        }
        if let Some(equip) = equip_event {
            let id = self.mobs[index].id;
            let pos = self.mobs[index].position;
            self.mob_equip_event(id, [pos.x, pos.y, pos.z], equip);
        }
        let mut drops: Vec<ItemStack> = drops
            .into_iter()
            .map(|(item, count)| ItemStack::new(&item, count))
            .collect();
        if !drops.is_empty() {
            let player = Some(self.native_acting_id());
            self.native_use_drops(
                &drops_table,
                feet,
                None,
                Some((mob.0, &mob.1)),
                player,
                held.as_ref(),
                &mut drops,
            );
        }
        for stack in drops {
            for _ in 0..stack.count {
                // `spawnAtLocation(stack, 1.0)`: an ItemEntity a block above
                // the feet that starts with the constructor's own motion, and
                // `setDefaultPickUpDelay`. Sheep.shear adds its spread on top;
                // MushroomCow.shear builds the entity by hand instead, so its
                // mushrooms keep the constructor's motion and no pickup delay.
                let mut spread = [0.0; 3];
                if wool {
                    spread[0] = (self.world.random.next_float() - self.world.random.next_float())
                        as f64
                        * 0.1;
                    spread[1] = self.world.random.next_float() as f64 * 0.05;
                    spread[2] = (self.world.random.next_float() - self.world.random.next_float())
                        as f64
                        * 0.1;
                }
                self.spawn_item_own(
                    ItemStack {
                        count: 1,
                        ..stack.clone()
                    },
                    [feet[0], feet[1] + 1.0, feet[2]],
                    if wool { 10 } else { 0 },
                );
                if let Some(e) = self.just_spawned_item() {
                    e.velocity = e
                        .velocity
                        .add_vector(Vec3::new(spread[0], spread[1], spread[2]));
                }
            }
        }
        Ok(true)
    }
    /// `SnowGolem.mobInteract` and `Bogged.mobInteract` with shears: the
    /// pumpkin or the mushrooms come off (the shearing loot table), the shear
    /// game event is posted and the tool takes one damage.
    fn interact_shear_mob(&mut self, index: usize, shears: bool) -> bool {
        let m = &self.mobs[index];
        if !shears || !m.alive() || m.extra.sheared {
            return false;
        }
        let (sound, table, height) = match m.species() {
            Species::SnowGolem => (
                "minecraft:entity.snow_golem.shear",
                "minecraft:shearing/snow_golem",
                m.eye_height(),
            ),
            Species::Bogged => (
                "minecraft:entity.bogged.shear",
                "minecraft:shearing/bogged",
                f64::from(m.height()),
            ),
            _ => return false,
        };
        let at = m.position;
        let mob = (m.id, super::native_hooks::mob_kind(m));
        let tool = self.held().cloned();
        // shear: level.playSound(null, this, sound, PLAYERS, 1, 1).
        let shear =
            LevelSound::new(sound, [at.x, at.y, at.z], 1.0, 1.0).in_source(SoundSource::Players);
        self.play_bound_sound(shear, mob.0, None);
        self.mobs[index].extra.sheared = true;
        let mut stacks = self.sequence_loot(table, table, tool.as_ref());
        let origin = [at.x, at.y, at.z];
        let player = Some(self.native_acting_id());
        self.native_use_drops(
            table,
            origin,
            None,
            Some((mob.0, &mob.1)),
            player,
            tool.as_ref(),
            &mut stacks,
        );
        for stack in stacks {
            self.spawn_at_location(at, height, stack);
        }
        let ctx = self.who_context(Some(sunrise_client::mobs::Who::Player), None);
        self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
        self.damage_held_tool();
        true
    }
    /// `Armadillo.mobInteract`: a brush brushes a scute off an adult
    /// (`brushOffScute`) and loses 16 durability.
    fn interact_armadillo(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if item != "minecraft:brush"
            || m.species() != Species::Armadillo
            || !m.alive()
            || m.is_baby()
        {
            return false;
        }
        let (id, at) = (m.id, m.position);
        let feet = [at.x, at.y, at.z];
        let tool = self.held().cloned();
        let table = "minecraft:brush/armadillo";
        let mut stacks = self.sequence_loot(table, table, tool.as_ref());
        let kind = super::native_hooks::mob_kind(&self.mobs[index]);
        let player = Some(self.native_acting_id());
        self.native_use_drops(
            table,
            feet,
            None,
            Some((id, &kind)),
            player,
            tool.as_ref(),
            &mut stacks,
        );
        for d in stacks {
            self.spawn_at_location(at, 0.0, d);
        }
        self.audio
            .play("minecraft:entity.armadillo.brush", Some(feet), 1.0, 1.0);
        let ctx = self.who_context(Some(Who::Mob(id)), None);
        self.post_game_event("minecraft:entity_interact", feet, ctx);
        // ItemStack.hurtAndBreak(16).
        if !self.creative
            && let Some(stack) = tool
        {
            let max = self
                .res
                .items()
                .get(&stack.item)
                .map_or(0, |i| i.max_damage);
            if max > 0 {
                self.damage_held(16);
            }
        }
        true
    }
    /// `ItemStack.hurtAndBreak(1, ...)` on the held tool: Unbreaking, break sound and stat.
    pub(super) fn damage_held_tool(&mut self) {
        self.damage_held(1);
    }
    /// Throwables and spawn eggs (SnowballItem/EggItem/EnderpearlItem/SpawnEggItem.use).
    pub(super) fn throw_item(&mut self, stack: &ItemStack) -> Result<bool> {
        let item = &*stack.item;
        let kind = match item {
            "minecraft:snowball" => ProjectileKind::Snowball,
            "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" => ProjectileKind::Egg,
            "minecraft:ender_pearl" => ProjectileKind::EnderPearl,
            "minecraft:wind_charge" => {
                let thrown = self.throw_wind_charge(stack)?;
                if thrown {
                    self.apply_use_cooldown(stack);
                }
                return Ok(thrown);
            }
            _ if item.ends_with("_spawn_egg") => return self.use_spawn_egg(stack),
            _ => match sunrise_client::projectiles::thrown_by_item(item) {
                Some(t) => ProjectileKind::Thrown(t),
                None => return Ok(false),
            },
        };
        let Some(eye) = self.eye_position() else {
            return Ok(false);
        };
        let Some(p) = self.player.as_ref() else {
            return Ok(false);
        };
        // Projectile.shootFromRotation adds getKnownMovement: what the thrower last moved.
        let on_ground = p.ground.on_ground;
        let velocity = self.known_player_movement();
        let start = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let mut proj = Projectile::new(kind.clone(), start, Some(Who::Player));
        proj.set_egg_item(stack);
        proj.shoot_from_rotation(self.pitch, self.yaw, 0.0, 1.5, 1.0, velocity, on_ground);
        self.spawn_projectile(proj, stack);
        // SnowballItem.use / EggItem.use / EnderpearlItem.use: awardStat(ITEM_USED).
        self.award_stat("used", item, 1);
        // ItemStack.use's applyAfterUseComponentSideEffects: the pearl's
        // second is the `use_cooldown` component, as for any thrown item.
        self.apply_use_cooldown(stack);
        let sound = match kind {
            ProjectileKind::Snowball | ProjectileKind::Thrown(_) => {
                "minecraft:entity.snowball.throw"
            }
            ProjectileKind::Egg => "minecraft:entity.egg.throw",
            _ => "minecraft:entity.ender_pearl.throw",
        };
        let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
        self.audio.play(sound, Some(eye), 0.5, pitch);
        self.swing_arm();
        self.consume_held();
        if kind == ProjectileKind::Egg
            && let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
        {
            self.native_player_egg_throw(player, &name);
        }
        Ok(true)
    }
    /// `Mob.checkAndHandleImportantInteractions` with a spawn egg of mob
    /// `index`'s own kind (`SpawnEggItem.spawnOffspringFromSpawnEgg`): a baby
    /// at the mob's feet, taking the parent's variant as `getBreedOffspring`
    /// does, and one egg used. False (PASS) when the mob has no baby form.
    fn spawn_egg_offspring(&mut self, index: usize) -> bool {
        let Some(stack) = self.held().cloned() else {
            return false;
        };
        let parent = &self.mobs[index];
        if !parent.alive()
            || !stack.item.ends_with("_spawn_egg")
            || sunrise_client::block_entity_item::item_entity_data(&stack, self.res.items())
                .is_none_or(|data| data.kind != parent.kind.id)
            || self.ui.difficulty == 0 && !parent.kind.allowed_in_peaceful()
            || !sunrise_client::mobs::species::can_be_a_baby(parent.species())
            // WanderingTrader.getBreedOffspring returns null: no offspring, the
            // click carries on to trading.
            || parent.species() == Species::WanderingTrader
        {
            return false;
        }
        let (kind, at) = (
            parent.kind.id.clone(),
            [parent.position.x, parent.position.y, parent.position.z],
        );
        let (species, variant, color) =
            (parent.species(), parent.extra.variant, parent.extra.color);
        // `MushroomCow.getBreedOffspring(level, this)`: the egg's mate is the
        // parent itself, so the colour always rolls `nextInt(1024) == 0` to
        // flip, and every other roll spends the coin flip that can only pick
        // that same colour again. Both come from the parent's own random.
        let variant = if species == Species::Mooshroom {
            let random = &mut self.mobs[index].random;
            if random.next_int_bound(1024).unwrap_or(1) == 0 {
                if variant == 1 { 0 } else { 1 }
            } else {
                let _ = random.next_boolean();
                variant
            }
        } else {
            variant
        };
        let target = self.mob_entity(&self.mobs[index]);
        // SpawnEggItem.spawnOffspringFromSpawnEgg: `type.create(level, SPAWN_ITEM_USE)`
        // creates the baby without finalizeSpawn, and `snapTo(pos, 0, 0)` faces
        // it south without a level-random draw.
        let Some(child) = self.create_mob_reason(&kind, at, false, SpawnSite::Raw, Some(0.0))
        else {
            return false;
        };
        let Some(id) = self.insert_spawned_mob(child) else {
            return false;
        };
        if let Some(child) = self.mobs.iter_mut().find(|m| m.id == id) {
            sunrise_client::mobs::finalize::spawn_egg_offspring(child, species, variant, color);
        }
        // `Mob.checkAndHandleImportantInteractions` then calls
        // `onOffspringSpawnedFromEgg`: `Fox`'s makes the baby trust the player
        // who used the egg (a fox egg used on a block trusts nobody).
        if species == Species::Fox {
            let spawner = self.acting_player_uuid();
            if let Some(child) = self.mobs.iter_mut().find(|m| m.id == id) {
                sunrise_client::mobs::fox::add_trusted_entity(child, spawner);
            }
        }
        self.trigger_item_entity("minecraft:player_interacted_with_entity", &stack, &target);
        self.consume_held();
        // `Mob.interact`: the egg's `SUCCESS_SERVER` consumes the click, so the
        // interaction posts `ENTITY_INTERACT` with the player, at the mob.
        let ctx = self.who_context(Some(Who::Player), None);
        self.post_game_event("minecraft:entity_interact", at, ctx);
        true
    }
    /// `SpawnEggItem.use`: the SOURCE_ONLY ray must hit a liquid block.
    pub(super) fn spawn_egg_fluid_hit(&self) -> Option<super::Hit> {
        let hit = self.bucket_hit()?;
        matches!(
            &*self.block_name(hit.pos),
            "minecraft:water" | "minecraft:lava"
        )
        .then_some(hit)
    }
    /// Shared peaceful-mode eligibility for spawn eggs.
    pub(super) fn spawn_egg_allowed(&self, kind: &str) -> bool {
        self.ui.difficulty != 0
            || self
                .res
                .mob_types()
                .get(kind)
                .is_none_or(|t| t.allowed_in_peaceful())
    }
    /// `SpawnEggItem.useOn` for a block, or `use` for a fluid source.
    fn use_spawn_egg(&mut self, stack: &ItemStack) -> Result<bool> {
        let fluid = matches!(self.guest_use, Some(super::GuestUse::Item(_)));
        let Some(hit) = (if fluid {
            self.spawn_egg_fluid_hit()
        } else {
            self.pick()
        }) else {
            return Ok(false);
        };
        // ItemStack.useOn in adventure mode: an egg used on a block without a
        // matching `can_place_on` passes to `use`, whose fluid ray finds none.
        if !fluid && !self.may_use_on(hit.pos) {
            return Ok(false);
        }
        let items = self.res.shared_items();
        let Some(data) = sunrise_client::block_entity_item::item_entity_data(stack, &items)
            .filter(|data| self.spawn_egg_allowed(&data.kind))
        else {
            self.use_failed = true;
            return Ok(true);
        };
        if fluid
            && !self.may_build()
            && !self.can_place_on_in_adventure([
                hit.pos[0] - hit.face[0],
                hit.pos[1] - hit.face[1],
                hit.pos[2] - hit.face[2],
            ])
        {
            self.use_failed = true;
            return Ok(true);
        }
        // SpawnEggItem.useOn on a spawner sets its mob (and an op's egg data).
        let spawner_data = self.acting_player_is_op().then_some(data.compound);
        if let Some(done) = self.use_spawn_egg_on_spawner(hit.pos, &data.kind, spawner_data) {
            if done {
                self.swing_arm();
                self.award_stat("used", &stack.item, 1);
                self.consume_held();
            }
            return Ok(true);
        }
        // useOn: a clicked block without a collision shape (vines, grass)
        // takes the mob itself, any other the one on the clicked face.
        let p = if fluid || self.collision_empty(hit.pos) {
            hit.pos
        } else {
            [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ]
        };
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        ];
        let Some(id) = self.spawn_mob_spawned(
            &data.kind,
            at,
            SpawnSite::Finalizing(SpawnReason::SpawnItemUse),
        ) else {
            return Ok(false);
        };
        // SpawnEggItem.spawnMob: `EntityType.create` finalizes the mob, then
        // `PostSpawnProcessor` puts the egg's components on it.
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            sunrise_client::entity_nbt::apply_item_components(m, stack);
        }
        self.apply_spawn_egg_entity_data(id, &data);
        self.post_block_game_event("minecraft:entity_place", p, Some(Who::Player), None);
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        self.consume_held();
        Ok(true)
    }
    /// `EntityType.updateCustomEntityTag`: the egg's `minecraft:entity_data` on
    /// the mob it spawned, once the guards let it through, as
    /// `TypedEntityData.loadInto`: the mob's own compound, the data merged over
    /// it, loaded back into the same mob (`/data merge entity`'s loader).
    pub(super) fn apply_spawn_egg_entity_data(
        &mut self,
        id: u32,
        data: &sunrise_client::block_entity_item::ItemEntityData<'_>,
    ) {
        use sunrise_core::nbt::Tag;
        // A type-only component has no custom fields to apply.
        if data.compound.len() == 1 || !self.acting_player_is_op() && data.op_only() {
            return;
        }
        let Some(i) = self
            .mobs
            .iter()
            .position(|m| m.id == id && m.kind.id == data.kind)
        else {
            return;
        };
        if let Tag::Compound(mut tag) = self.mob_tree_nbt(i) {
            sunrise_client::data_cmd::merge_compound(&mut tag, data.compound);
            // Nothing to report: the mob's own compound always loads back.
            let _ = self.apply_entity_nbt(super::commands::Target::Mob(id), Tag::Compound(tag));
        }
    }
    /// A beam target's centre (half its height), interpolated.
    fn beam_target(&self, who: Option<Who>, alpha: f64) -> Option<glam::DVec3> {
        match who? {
            Who::Player => {
                let p = self.player.as_ref()?;
                let (a, b) = (self.previous_position, p.position);
                Some(glam::DVec3::new(
                    a.x + (b.x - a.x) * alpha,
                    a.y + (b.y - a.y) * alpha + f64::from(p.dimensions.height) * 0.5,
                    a.z + (b.z - a.z) * alpha,
                ))
            }
            Who::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id)?;
                Some(glam::DVec3::new(
                    m.previous.x + (m.position.x - m.previous.x) * alpha,
                    m.previous.y
                        + (m.position.y - m.previous.y) * alpha
                        + f64::from(m.height()) * 0.5,
                    m.previous.z + (m.position.z - m.previous.z) * alpha,
                ))
            }
        }
    }
    /// `Guardian.aiStep` on a client: two bubbles a tick trailing a guardian
    /// that swims along, and bubbles up the beam to its target. The server
    /// never draws them, so the mob's own random stays untouched.
    pub(super) fn guardian_bubbles(&mut self) {
        use sunrise_client::mobs::guardian;
        use sunrise_client::particles::{ParticleOptions, ParticleType};
        let mut swimming = Vec::new();
        let mut beams = Vec::new();
        for m in &self.mobs {
            if !m.alive() || !matches!(m.species(), Species::Guardian | Species::ElderGuardian) {
                continue;
            }
            let at = [m.position.x, m.position.y, m.position.z];
            if m.extra.flag2 && m.fluids.in_water {
                let (width, height, _) = m.dims();
                swimming.push((at, width, height, m.view_vector()));
            }
            if let Some((_, scale)) = guardian::beam(m, 0.0)
                && let Some(t) = self.beam_target(m.ai.target, 1.0)
            {
                beams.push(([at[0], m.eye_y(), at[2]], [t.x, t.y, t.z], f64::from(scale)));
            }
        }
        let bubble = ParticleOptions::simple(ParticleType::Bubble);
        for (at, width, height, view) in swimming {
            for _ in 0..2 {
                let (x, y, z) = {
                    let r = self.particles.random();
                    (r.next_double(), r.next_double(), r.next_double())
                };
                let p = [
                    at[0] + width * (2.0 * x - 1.0) * 0.5 - view[0] * 1.5,
                    at[1] + height * y - view[1] * 1.5,
                    at[2] + width * (2.0 * z - 1.0) * 0.5 - view[2] * 1.5,
                ];
                self.particles.add(&bubble, p, [0.0; 3]);
            }
        }
        for (eye, target, scale) in beams {
            let d = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if length <= 0.0 {
                continue;
            }
            let mut dist = self.particles.random().next_double();
            while dist < length {
                let r = self.particles.random();
                dist += 1.8 - scale + r.next_double() * (1.7 - scale);
                let f = dist / length;
                let p = [eye[0] + d[0] * f, eye[1] + d[1] * f, eye[2] + d[2] * f];
                self.particles.add(&bubble, p, [0.0; 3]);
            }
        }
    }
    /// Mob models and projectiles for the entity pass.
    /// The renderer's models for the GPU path (`mobs::skin`) this frame, if it
    /// poses entities: a new frame of draws, the store restarted when the
    /// renderer's no longer matches. Once per frame, before any entity draws.
    pub(super) fn begin_skinning(&self) {
        let renderer = self.renderer.as_ref().filter(|r| r.skins_entities());
        match renderer.and_then(|r| r.skinned_models()) {
            Some(stored) => {
                let mut models = self.skin_models.borrow_mut();
                // Reuse helps CPU-bound crowds, but regressed TAA frame tails in
                // windowed measurements. Keep the original arithmetic with TAA on.
                models.set_rotation_cache_enabled(!self.settings.sunrise.taa);
                models.begin(&mut self.skin_frame.borrow_mut(), stored);
                models.tagged = renderer.is_some_and(|r| r.skins_tagged());
            }
            None => self.skin_frame.borrow_mut().clear_draws(),
        }
    }

    /// Whether this frame's models can be posed on the GPU (`begin_skinning`).
    pub(super) fn skinning(&self) -> bool {
        self.renderer
            .as_ref()
            .is_some_and(|r| r.skins_entities() && r.skinned_models().is_some())
    }

    /// Culls one entity's vertices from `entity_start`, counting what it drew
    /// on the GPU path by the corners of `skinned`'s bounds; a culled entity's
    /// posed draws since `skinned`'s mark are taken back.
    pub(super) fn cull_skinned(
        &self,
        out: &mut Vec<Vertex>,
        entity_start: usize,
        skinned: Skinned,
    ) {
        let corners = skinned.and_then(|(_, bounds)| bounds);
        if out.len() == entity_start {
            if let Some((lo, hi)) = corners
                && self.cull_entity_camera_bounds(lo, hi)
                && let Some((mark, _)) = skinned
            {
                if self.occlusion.shadow_casters.0 {
                    let mut frame = self.skin_frame.borrow_mut();
                    let end = frame.vertices as usize;
                    sunrise_client::gfx::pack_draw::EntitySpan::hide(
                        &mut frame.spans,
                        mark.2 as usize,
                        end,
                    );
                } else {
                    self.skin_frame.borrow_mut().truncate(mark);
                }
            }
            return;
        }
        if let Some((lo, hi)) = corners {
            out.extend([lo, hi].map(|pos| Vertex {
                pos: pos.to_array(),
                ..bytemuck::Zeroable::zeroed()
            }));
        }
        let len = out.len();
        let hidden = self.cull_entity_vertices(out, entity_start);
        if out.len() < len {
            if let Some((mark, _)) = skinned {
                self.skin_frame.borrow_mut().truncate(mark);
            }
        } else if corners.is_some() {
            if let Some((mark, _)) = skinned {
                // Previous GPU-only entities can leave zero-length CPU spans at
                // this same offset. Use this query's result, not their metadata.
                if hidden {
                    let mut skin = self.skin_frame.borrow_mut();
                    let end = skin.vertices as usize;
                    EntitySpan::hide(&mut skin.spans, mark.2 as usize, end);
                }
            }
            out.truncate(len - 2);
            // The temporary bounds are not real geometry. Keep the culling end marker
            // at the true CPU end before the next entity appends a span there.
            for span in self.entity_spans.borrow_mut().iter_mut().rev() {
                if span.start as usize <= out.len() {
                    break;
                }
                span.start = out.len() as u32;
            }
        }
    }

    pub(in crate::game) fn begin_mob_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
    ) -> MobDrawFrame<'_> {
        // The coarse mob bounds pay off only in dense async scenes; otherwise,
        // keep the existing exact mesh-culling path.
        let precheck = self.settings.sunrise.entity_culling == EntityCullingMode::Asynchronous
            && self
                .occlusion
                .async_culler
                .as_ref()
                .is_some_and(|culler| culler.camera_is_stationary())
            && self.settings.sunrise.entity_culling_precheck != 0
            && self.mobs.len() >= self.settings.sunrise.entity_culling_precheck as usize
            && !self.occlusion.shadow_casters.0
            && mod_api::__private::mods().is_empty();
        self.rendered_mobs.set(0);
        // What custom entity model animations read of the frame.
        sunrise_client::mobs::cem_anim::set_frame(sunrise_client::mobs::cem_anim::Frame {
            time: self.world.game_time as f64 + alpha,
            day_time: self.world.day_time as f64 + alpha,
            dimension: match self.dim {
                Dim::Overworld | Dim::Extra(_) => 0.0,
                Dim::Nether => -1.0,
                Dim::End => 1.0,
            },
            player: self.player.as_ref().map_or(camera.to_array(), |p| {
                [p.position.x, p.position.y, p.position.z]
            }),
            player_rot: [self.pitch, self.yaw],
            paused: self.pause,
            first_person: self.camera_type.is_first_person(),
            third_person_reversed: self.camera_type
                == sunrise_client::camera::CameraType::ThirdPersonFront,
            ..Default::default()
        });
        MobDrawFrame {
            precheck,
            living: LivingFrame {
                camera,
                alpha,
                scale: self.view_scale(),
                viewer: self.player_team(),
                skinning: self.skinning(),
                listened: self.mob_render_listened(),
            },
        }
    }

    pub(in crate::game) fn mob_run_geometry(
        &self,
        frame: &MobDrawFrame<'_>,
        range: std::ops::Range<usize>,
        out: &mut Vec<Vertex>,
    ) {
        if frame.precheck {
            self.mob_run::<true>(&frame.living, range, out);
        } else {
            self.mob_run::<false>(&frame.living, range, out);
        }
    }

    fn mob_run<const PRECHECK: bool>(
        &self,
        frame: &LivingFrame<'_>,
        range: std::ops::Range<usize>,
        out: &mut Vec<Vertex>,
    ) {
        for mob in &self.mobs[range] {
            self.living_geometry::<PRECHECK>(mob, frame, out);
        }
    }

    #[cfg(test)]
    pub(in crate::game) fn mob_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let frame = self.begin_mob_geometry(camera, alpha);
        self.mob_run_geometry(&frame, 0..self.mobs.len(), out);
        self.mark_motion(out.len(), 0);
        self.other_creature_geometry(camera, alpha, out);
    }

    pub(in crate::game) fn other_creature_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        self.elder_apparition_geometry(alpha, out);
        self.spawner_geometry(camera, alpha, out);
        let live = self.creatures.projectiles.iter().map(|a| {
            let at = [
                a.previous.x + (a.position.x - a.previous.x) * alpha,
                a.previous.y + (a.position.y - a.previous.y) * alpha,
                a.previous.z + (a.position.z - a.previous.z) * alpha,
            ];
            (a, at)
        });
        self.projectile_geometry(camera, alpha, live, out);
    }

    pub(super) fn pickup_arrow_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        // ItemPickupParticle: a picked-up arrow flying to its collector.
        let taken = self.pickups.live.iter().filter_map(|p| match &p.taken {
            super::pickups::Taken::Arrow(a) => Some((&**a, super::pickups::position(p, alpha))),
            _ => None,
        });
        self.projectile_geometry(camera, alpha, taken, out);
    }

    fn projectile_geometry<'a>(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        projectiles: impl Iterator<Item = (&'a Projectile, [f64; 3])>,
        out: &mut Vec<Vertex>,
    ) {
        let atlas = &self.gui.atlas;
        let (right, up) = thrown_item_axes(self.camera_type, self.yaw, self.pitch);
        // A glowing projectile's outline: what it drew up to the next one.
        let mut glowing = None;
        for (a, at) in projectiles {
            if let Some((start, rgb)) = glowing.take() {
                self.mark_outline(&out[start..], rgb);
            }
            self.mark_entity(out.len(), a.entity_type());
            if a.id != 0
                && let Some(rgb) = self.thing_outline(a.id, a.kind.id())
            {
                glowing = Some((out.len(), rgb));
            }
            let p = glam::DVec3::from_array(at) - camera;
            let c = p.as_vec3();
            // EntityRenderer.getPackedLightCoords: the block its light probe
            // (the eye) is in, at the position this frame draws rather than
            // this tick's, and not the one under its feet.
            let light = self.world.light(
                at[0].floor() as i32,
                a.light_probe_y(alpha).floor() as i32,
                at[2].floor() as i32,
            );
            let mut packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            // EntityRenderer.getBlockLightLevel: a shulker bullet and anything
            // on fire are drawn at 15 block light (the fireball renderers'
            // own override); the sky light stays the level's.
            if let Some(block) = a.block_light_override() {
                packed = (packed & 0xFF) | (u32::from(block) * 16) << 8;
            }
            if a.is_arrow() {
                let Some(model) = self.arrow_model.as_ref() else {
                    continue;
                };
                // SpectralArrowRenderer, and TippableArrowRenderer's `getColor() > 0`.
                let texture = if a.is_spectral_arrow() {
                    "entity/projectiles/arrow_spectral"
                } else if a.arrow.as_ref().is_some_and(|s| s.effect_color > 0) {
                    "entity/projectiles/arrow_tipped"
                } else {
                    "entity/projectiles/arrow"
                };
                let Some(r) = atlas.sprites.get(texture).copied() else {
                    continue;
                };
                // ArrowRenderer: `Mth.lerp(partialTick, state.yRotO, state.yRot)`
                // and the same for the pitch, so a turning arrow is drawn
                // between the two ticks it turned over.
                let alpha32 = alpha as f32;
                let yaw = a.previous_yaw + alpha32 * (a.yaw - a.previous_yaw);
                let pitch = a.previous_pitch + alpha32 * (a.pitch - a.previous_pitch);
                let m = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_rotation_y((yaw - 90.0).to_radians())
                    * glam::Mat4::from_rotation_z(pitch.to_radians());
                // ArrowRenderer: `state.shake = shakeTime - partialTicks`; ArrowModel.setupAnim
                // rolls the root while the shake timer runs.
                let shake = a
                    .arrow
                    .as_ref()
                    .map_or(0.0, |s| s.shake as f32 - alpha as f32);
                let roll = if shake > 0.0 {
                    -(shake * 3.0).msin() * shake * (std::f32::consts::PI / 180.0)
                } else {
                    0.0
                };
                let pose_for = |part: &sunrise_client::mobs::Part| {
                    let mut pose = part.pose;
                    if std::ptr::eq(part, model) {
                        pose[5] += roll;
                    }
                    pose
                };
                sunrise_client::mobs::render_model(
                    model,
                    m,
                    &pose_for,
                    [r.u0, r.v0, r.u1, r.v1],
                    packed,
                    0,
                    out,
                );
                if a.arrow.as_ref().is_some_and(|s| s.on_fire()) {
                    flame_geometry(atlas, c, 0.5, 0.5, self.yaw, light, out);
                }
                continue;
            }
            // ThrownTridentRenderer.
            if matches!(a.kind, ProjectileKind::Trident) {
                let Some(model) = self.trident_model.as_ref() else {
                    continue;
                };
                let Some(r) = atlas.sprites.get("entity/trident/trident").copied() else {
                    continue;
                };
                let m = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_rotation_y((a.yaw - 90.0).to_radians())
                    * glam::Mat4::from_rotation_z((a.pitch + 90.0).to_radians());
                let poses = sunrise_client::mobs::anim::Poses::new(model);
                sunrise_client::mobs::render::draw(
                    model,
                    m,
                    &poses,
                    [r.u0, r.v0, r.u1, r.v1],
                    0xFFFF_FFFF,
                    packed,
                    0,
                    out,
                );
                continue;
            }
            // WindChargeRenderer.
            if matches!(
                a.kind,
                ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
            ) {
                self.wind_charge_geometry(a, c, packed, alpha as f32, out);
                continue;
            }
            // ShulkerBulletRenderer: the tumbling bullet model, then a larger
            // translucent copy of it (colour 0x26FFFFFF); always 15 block light.
            if matches!(a.kind, ProjectileKind::ShulkerBullet { .. }) {
                let model = &self.entity_models.shulker_bullet;
                if let (Some(model), Some(r)) =
                    (model, atlas.sprites.get("entity/shulker/spark").copied())
                {
                    use sunrise_core::geometry::trig;
                    let tc = a.age as f32 + alpha as f32;
                    let mut poses = sunrise_client::mobs::anim::Poses::new(model);
                    poses.yrot("main", a.yaw.to_radians());
                    poses.xrot("main", a.pitch.to_radians());
                    let light = (packed & 0xFF) | ((15 * 16) << 8);
                    let base = glam::Mat4::from_translation(c)
                        * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.15, 0.0))
                        * glam::Mat4::from_rotation_y(
                            (trig::sin(f64::from(tc * 0.1)) * 180.0).to_radians(),
                        )
                        * glam::Mat4::from_rotation_x(
                            (trig::cos(f64::from(tc * 0.1)) * 180.0).to_radians(),
                        )
                        * glam::Mat4::from_rotation_z(
                            (trig::sin(f64::from(tc * 0.15)) * 360.0).to_radians(),
                        )
                        * glam::Mat4::from_scale(glam::Vec3::new(-0.5, -0.5, 0.5));
                    let uv = [r.u0, r.v0, r.u1, r.v1];
                    sunrise_client::mobs::render::draw(
                        model,
                        base,
                        &poses,
                        uv,
                        0xFFFF_FFFF,
                        light,
                        0,
                        out,
                    );
                    // The translucent shell blends over the opaque core
                    // (`RenderTypes.entityTranslucent`).
                    let shell = out.len();
                    sunrise_client::mobs::render::draw(
                        model,
                        base * glam::Mat4::from_scale(glam::Vec3::splat(1.5)),
                        &poses,
                        uv,
                        0x26FF_FFFF,
                        light,
                        0,
                        out,
                    );
                    sunrise_client::gfx::mark_oit(&mut out[shell..]);
                }
                continue;
            }
            // WitherSkullRenderer: the skull model facing its flight.
            if let Some(dangerous) = super::wither::skull_kind(a) {
                let model = &self.entity_models.wither_skull;
                let tex = if dangerous {
                    "entity/wither/wither_invulnerable"
                } else {
                    "entity/wither/wither"
                };
                if let (Some(model), Some(r)) = (model, atlas.sprites.get(tex).copied()) {
                    let mut poses = sunrise_client::mobs::anim::Poses::new(model);
                    poses.yrot("head", a.yaw.to_radians());
                    poses.xrot("head", a.pitch.to_radians());
                    let base = glam::Mat4::from_translation(c)
                        * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
                    sunrise_client::mobs::render::draw(
                        model,
                        base,
                        &poses,
                        [r.u0, r.v0, r.u1, r.v1],
                        0xFFFF_FFFF,
                        packed,
                        0,
                        out,
                    );
                }
                continue;
            }
            if self.thrown_item_geometry(a, alpha, at, c, packed, out) {
                continue;
            }
            // `Entity.displayFireAnimation`: a thrown item that flew through a
            // fire block burns (`ThrowableProjectile.tick`'s
            // `applyEffectsFromBlocks`), and ThrownItemRenderer draws the flame
            // over its box as any other entity renderer does.
            if a.on_fire() {
                let size = a.kind.size() as f32;
                flame_geometry(atlas, c, size, size, self.yaw, light, out);
            }
            // `ThrownItemRenderer` for a fireball: the item `Fireball.getItem`
            // returns (a fire charge until `setItem` gave it another), its model
            // at the renderer's scale for the kind. `packed` is already 15, the
            // renderer's full bright.
            if let Some(stack) = a.item.as_ref() {
                let owner = super::item_draw::owner_at(at, 0.0, a.kind.id());
                let state = self.resolve_item(stack, DisplayContext::Ground, Some(&owner), 0);
                let pose = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_quat(self.camera_orientation())
                    * glam::Mat4::from_scale(glam::Vec3::splat(a.sprite_scale()));
                self.submit_item_world(&state, pose, packed, out);
                continue;
            }
            let Some(name) = a.item_sprite() else {
                continue;
            };
            let Some(r) = atlas.sprites.get(name).copied() else {
                continue;
            };
            let lift = is_item_billboard(&a.kind);
            let quad = thrown_item_quad(c, right, up, a.sprite_scale(), lift);
            for (v, uv) in quad
                .iter()
                .zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]])
            {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: v.to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: packed,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
        if let Some((start, rgb)) = glowing {
            self.mark_outline(&out[start..], rgb);
        }
    }
}
/// `ThrownItemRenderer.submit` turns the item by `CameraRenderState.orientation`
/// (`Camera.setRotation`): the billboard's right and up axes are the camera's.
/// They stay continuous at pitch ±90, and in third-person front view they are
/// the turned-round camera's, not the player's.
fn thrown_item_axes(
    camera_type: sunrise_client::camera::CameraType,
    yaw: f32,
    pitch: f32,
) -> (glam::Vec3, glam::Vec3) {
    let orientation = super::camera_rotation(camera_type, yaw, pitch);
    (orientation * glam::Vec3::X, orientation * glam::Vec3::Y)
}
/// The four corners of the item billboard, in the order its UVs are laid out.
/// `ThrownItemRenderer` only scales and billboards it; the lift is
/// `item/generated`'s ground display transform, `translation [0, 2, 0]` of 16
/// units = 0.125 blocks, which is `0.125 * scale` == `0.5 * s`.
fn thrown_item_quad(
    at: glam::Vec3,
    right: glam::Vec3,
    up: glam::Vec3,
    scale: f32,
    lift: bool,
) -> [glam::Vec3; 4] {
    let s = 0.25 * scale;
    let rise = if lift { 0.5 * s } else { 0.0 };
    let centre = at + up * rise;
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .map(|k| centre + right * k.0 * s + up * k.1 * s)
}
/// Whether vanilla draws the kind as `ThrownItemRenderer`'s item billboard, the
/// only ones that take that lift. Llama spit shares this quad but
/// `LlamaSpitRenderer` is a model, with its own offset, and a fireball's item
/// is drawn as the item model that carries the lift itself.
fn is_item_billboard(kind: &ProjectileKind) -> bool {
    matches!(
        kind,
        ProjectileKind::Snowball | ProjectileKind::Egg | ProjectileKind::EnderPearl
    )
}
/// `Bucketable.getBucketItemStack` and `getPickupSound` of a mob that a water
/// bucket picks up.
fn bucketable(species: Species) -> Option<(&'static str, &'static str)> {
    let fish = "minecraft:item.bucket.fill_fish";
    Some(match species {
        Species::Cod => ("minecraft:cod_bucket", fish),
        Species::Salmon => ("minecraft:salmon_bucket", fish),
        Species::Pufferfish => ("minecraft:pufferfish_bucket", fish),
        Species::TropicalFish => ("minecraft:tropical_fish_bucket", fish),
        Species::Tadpole => (
            "minecraft:tadpole_bucket",
            "minecraft:item.bucket.fill_tadpole",
        ),
        Species::Axolotl => (
            "minecraft:axolotl_bucket",
            "minecraft:item.bucket.fill_axolotl",
        ),
        _ => return None,
    })
}
pub(super) use sunrise_client::health::{damage_type, melee, source_damage_type};
/// A live item as a loot stack: ordinary enchantments and the complete saved
/// component patch, including removals. The TOOL getter resolves its prototype.
pub(in crate::game) fn to_loot(stack: &ItemStack) -> sunrise_world::loot::LootStack {
    let components = match sunrise_client::item_data::component_snapshot(stack) {
        Some(sunrise_core::nbt::Tag::Compound(patch)) => patch
            .into_iter()
            .map(|(id, value)| {
                let id = id.to_string_lossy();
                if let Some(base) = id.strip_prefix('!') {
                    (base.to_owned(), None)
                } else {
                    (id, Some(value))
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    sunrise_world::loot::LootStack {
        item: stack.item.to_string(),
        count: stack.count as i32,
        damage: stack.damage as i32,
        enchantments: stack
            .data()
            .map(|data| data.enchantments.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|(e, l)| (e.to_string(), *l as i32))
            .collect(),
        potion: None,
        map: false,
        exploration: None,
        ominous_amplifier: None,
        stew_effects: Vec::new(),
        names: Vec::new(),
        components,
    }
}
/// A generated loot stack as a Sunrise item stack.
pub(super) fn from_loot(stack: &sunrise_world::loot::LootStack) -> ItemStack {
    let mut item = ItemStack {
        item: Arc::from(stack.item.as_str()),
        count: stack.count.max(1) as u32,
        damage: stack.damage.max(0) as u32,
        extra: None,
    };
    for (id, level) in &stack.enchantments {
        item.enchant(id, (*level).max(0) as u32);
    }
    if let Some(potion) = &stack.potion {
        item = item.with_potion(potion);
    }
    if let Some(amplifier) = stack.ominous_amplifier.filter(|a| *a > 0) {
        item.data_mut().ominous_amplifier = Some(amplifier as u8);
    }
    for (id, text) in &stack.names {
        item.data_mut().set_loot_name(id, text);
    }
    for (id, value) in &stack.components {
        item.data_mut().set_loot_component(id, value);
    }
    item
}
/// The instance a mob effect gives the player; `None` for a creative or
/// spectator player (`immune`) unless it is ambient (the nautilus's Breath).
fn mob_effect_instance(
    effect: &'static str,
    ticks: i32,
    amplifier: i32,
    ambient: bool,
    immune: bool,
) -> Option<sunrise_client::effects::EffectInstance> {
    if immune && !ambient {
        return None;
    }
    let id = effect.strip_prefix("minecraft:").unwrap_or(effect);
    let mut instance = sunrise_client::effects::EffectInstance::new(id, ticks, amplifier);
    instance.ambient = ambient;
    Some(instance)
}

include!("mob_query_hooks.rs");

impl Game<'_> {
    /// `EventHooks.canEntityGrief(level, entity)`: the `mob_griefing` rule,
    /// which a mod may change for `entity` (`EntityMobGriefingEvent`). No
    /// entity: the rule alone, as NeoForge.
    pub(super) fn can_entity_grief(&self, entity: Option<u32>) -> bool {
        let rule = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::MOB_GRIEFING);
        entity.map_or(rule, |entity| self.entity_grief(entity, rule))
    }
    /// Whether a large fireball's blast lights fire (`LargeFireball.onHit`):
    /// [`Self::can_entity_grief`] for its owner, as `getOwner()` gives it. That
    /// is the mob that shot it, or the player who owns it (the guest on `guest`,
    /// else the player in place). An owner that is gone, or none, reads the rule.
    pub(in crate::game) fn large_fireball_fire(
        &self,
        owner: Option<Who>,
        guest: Option<u32>,
    ) -> bool {
        self.can_entity_grief(match owner {
            Some(Who::Mob(id)) => Some(id),
            Some(Who::Player) => self
                .player_entity_id_by_connection(guest)
                .map(|id| id as u32),
            None => None,
        })
    }
    /// Whether a mod may answer `can_entity_grief` differently from the rule:
    /// the sites that skip their whole work while griefing is off skip it only
    /// when no mod listens.
    pub(super) fn grief_hooked(&self) -> bool {
        self.native_authoritative()
            && mod_api::__private::listened(mod_api::HookId::EntityMobGriefing)
    }
    /// [`Self::can_entity_grief`] for `entity`, with the rule already read.
    pub(super) fn entity_grief(&self, entity: u32, rule: bool) -> bool {
        if !self.grief_hooked() {
            return rule;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return rule;
        };
        let mut can_grief = rule;
        self.native_entity_mob_griefing(mod_api::EntityRef::new(epoch, entity), &mut can_grief);
        can_grief
    }
}

#[cfg(test)]
#[path = "mob_griefing_tests.rs"]
mod mob_griefing_tests;

#[cfg(test)]
mod control_mobs_sitting_tests;

#[cfg(test)]
mod living_visibility_tests;

#[cfg(test)]
mod enchanted_entity_loot_tests;

#[cfg(test)]
mod enderman_anger_tests;

#[cfg(test)]
mod local_sound_event_tests;

#[cfg(test)]
mod general_creature_tests;

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_hand_egg_checks_and_spawns_the_type_named_by_its_component() {
    use crate::game::{commands::peaceful_tests, execution_role::ExecutionRole};
    use sunrise_client::{
        chunk::block_entity::BlockEntity,
        mobs::spawner::{SpawnDataExt, SpawnerExt},
    };
    use sunrise_core::nbt::Tag;
    peaceful_tests::with_types(ExecutionRole::IntegratedServer, |game| {
        game.set_mode_id(0);
        game.forced_hit = Some(crate::game::Hit {
            pos: [4, 64, 4],
            face: [0, 1, 0],
            point: [4.5, 65.0, 4.5],
            inside: false,
        });
        let egg = peaceful_tests::trader_egg();
        game.inventory[game.selected] = Some(egg.clone());
        assert!(game.use_spawn_egg(&egg).unwrap());
        assert!(game.use_failed);
        assert!(game.mobs.is_empty());
        assert_eq!(game.held().unwrap().count, 1);

        game.use_failed = false;
        game.set_difficulty_forced(2);
        assert!(game.use_spawn_egg(&egg).unwrap());
        assert!(!game.use_failed);
        assert!(game.held().is_none());
        assert_eq!(game.mobs.len(), 1);
        assert_eq!(game.mobs[0].kind.id, peaceful_tests::TRADER);

        let mut egg = egg;
        egg.data_mut().set_other(
            "minecraft:entity_data",
            Some(Tag::compound([
                ("id", Tag::String(peaceful_tests::TRADER.into())),
                ("NoAI", Tag::Byte(1)),
            ])),
        );
        let key = (4, 64, 4);
        for op in [false, true] {
            game.lan_acting_permission = Some(if op { 2 } else { 0 });
            game.world
                .block_entities
                .insert(key, BlockEntity::Spawner(Box::default()));
            game.inventory[game.selected] = Some(egg.clone());
            assert!(game.use_spawn_egg(&egg).unwrap());
            assert!(game.held().is_none());
            let BlockEntity::Spawner(spawner) = &game.world.block_entities[&key] else {
                unreachable!()
            };
            let data = spawner.next_data().unwrap();
            assert_eq!(data.id().as_deref(), Some(peaceful_tests::TRADER));
            assert_eq!(data.entity.get(&"NoAI".into()), op.then_some(&Tag::Byte(1)));
        }
    });
}

/// Whether the killing blow's causing entity (`DamageSource.getEntity`) was a
/// mob, such as a tame wolf whose owner gets the kill credit.
fn killed_by_mob(m: &Mob) -> bool {
    m.last_damage
        .is_some_and(|(d, _)| matches!(d.attacker, Some(Who::Mob(_))))
}

/// `Animal.finalizeSpawnChildFromBreeding`'s player: the one who fed the
/// parent, or else the one who fed the partner (`getLoveCause`). `Some(None)`
/// is the host, `Some(Some(conn))` a guest. Both animals are spent.
/// `DyeColor.getMixedColor`: the colour of the crafting recipe that mixes
/// the two dyes, else one of the two picked at random.
fn dye_mix(
    context: &sunrise_world::native_load_context::LoopContext,
    recipes: &Recipes,
    random: &mut impl RandomSource,
    a: u8,
    b: u8,
) -> u8 {
    if let Some(mixed) = dye_color_mix_in_recipes(context, recipes, a, b) {
        return mixed;
    }
    if random.next_boolean() { a } else { b }
}

/// `DyeColor.findColorMixInRecipes`: the dye colour a two-dye crafting recipe
/// makes, when its result is a dye. The two dyes are the grid's first row
/// (`CraftingInput.of(2, 1, ...)`) and the recipe manager answers for it; the
/// result's `_dye` item names the colour.
fn dye_color_mix_in_recipes(
    context: &sunrise_world::native_load_context::LoopContext,
    recipes: &Recipes,
    a: u8,
    b: u8,
) -> Option<u8> {
    let dye = |c: u8| {
        let name = sunrise_client::mobs::render::DYE_NAMES[usize::from(c) & 15];
        Some(ItemStack::new(&format!("minecraft:{name}_dye"), 1))
    };
    let grid = [dye(a), dye(b), None, None];
    let (_, result) = recipes.find(context, &grid, 2)?;
    let name = result
        .item
        .strip_prefix("minecraft:")?
        .strip_suffix("_dye")?;
    let index = sunrise_client::mobs::render::DYE_NAMES
        .iter()
        .position(|n| *n == name)?;
    u8::try_from(index).ok()
}

fn breeding_credit(
    love_cause: &mut std::collections::HashMap<u32, Option<u32>>,
    parent: u32,
    partner: u32,
) -> Option<Option<u32>> {
    let from_parent = love_cause.remove(&parent);
    let from_partner = love_cause.remove(&partner);
    from_parent.or(from_partner)
}

#[cfg(test)]
mod mob_subscription_tests;

#[cfg(test)]
mod shulker_arrow_tests;

#[cfg(test)]
mod breeding_tests;

#[cfg(test)]
mod projectile_batch_tests;

#[cfg(test)]
mod breeze_random_tests;

#[cfg(test)]
mod damage_type_tests;

#[cfg(test)]
mod pearl_tests;

#[cfg(test)]
mod knockback_tests;

#[cfg(test)]
mod dye_mix_tests;

impl sunrise_client::arrows::OwnerLookup for Game<'_> {
    fn non_player_entity(&self, uuid: [i32; 4]) -> bool {
        let found = self.target_by_uuid(sunrise_client::lan::playerdata::ints_uuid(uuid));
        !matches!(
            found,
            None | Some(super::commands::Target::Player | super::commands::Target::Guest(_))
        )
    }
}

#[cfg(test)]
mod conversion_subscriber_tests;

#[cfg(test)]
mod living_drops_tests;

#[cfg(test)]
mod armor_destroy_tests;

#[cfg(test)]
mod conversion_tag_tests;

#[cfg(test)]
mod pickable_projectile_tests;

#[cfg(test)]
mod mob_pearl_hook_tests;

#[cfg(test)]
mod breeding_orb_tests;

#[cfg(test)]
mod enchanted_block_loot_breaker_tests;

#[cfg(test)]
#[path = "rabbit_colour_tests.rs"]
mod rabbit_colour_tests;

#[cfg(test)]
mod chorus_projectile_drop_tests;

#[cfg(test)]
mod loaded_arrow_shot_tests;

#[cfg(test)]
mod loaded_mob_arrow_permission_tests;

#[cfg(test)]
mod h169_glide_travel_tests;

/// `ServerLevel.getCurrentDifficultyAt` in `world`: the difficulty, the level's clock, and
/// the inhabited time and moon of the chunk at `at` (both 0 when it isn't loaded).
pub(in crate::game) fn local_difficulty_in(
    world: &sunrise_client::world::World,
    at: [f64; 3],
) -> spawning::LocalDifficulty {
    use sunrise_world::food::Difficulty;
    let base = match world.health_rules.difficulty {
        Difficulty::Peaceful => 0,
        Difficulty::Easy => 1,
        Difficulty::Normal => 2,
        Difficulty::Hard => 3,
    };
    let key = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
    let (inhabited, moon) = if world.chunks.contains_key(&key) {
        let phase = world.day_time.div_euclid(24000).rem_euclid(8) as usize;
        (
            world.inhabited.get(&key).copied().unwrap_or(0),
            spawning::MOON_BRIGHTNESS[phase],
        )
    } else {
        (0, 0.0)
    };
    spawning::LocalDifficulty::new(base, world.day_time, inhabited, moon)
}
#[cfg(test)]
#[path = "../../entities/tests/leash_without_item_criterion_tests.rs"]
mod leash_without_item_criterion_tests;
#[cfg(test)]
mod pending_birth_matrix_tests;
#[cfg(test)]
mod pending_birth_tests;

#[cfg(test)]
#[path = "../../entities/tests/leash_without_item_postbaseline_controls.rs"]
mod leash_without_item_postbaseline_controls;

#[cfg(test)]
#[path = "../tests/revenge_home_tests.rs"]
mod revenge_home_tests;

mod knockback;

#[cfg(test)]
mod death_loot_context_tests;

#[cfg(test)]
mod breeding_heart_tests;

#[cfg(test)]
#[path = "frame_projectile_tests.rs"]
mod frame_projectile_tests;
