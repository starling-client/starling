//! The ender dragon fight (`EnderDragonFight`, `EnderDragon`, its phases,
//! `EndCrystal`, `DragonFireball`): the fight state saved in the End's
//! `ender_dragon_fight.dat`, the dragon spawned over the island, the
//! holding pattern over the 24-node path graph, strafing with fireballs,
//! landing on the podium to breathe and charge, healing from the crystals on
//! the spikes, its parts (head and neck take full damage, the rest a quarter
//! plus one), wing knockback, block breaking, the death flight and rays, the
//! exit portal, the egg and the next gateway; and its rendering and boss bar.
#[path = "dragon/death.rs"]
mod death;
#[path = "dragon/residency.rs"]
mod residency;
#[path = "dragon/save_lending.rs"]
mod save_lending;
#[path = "dragon/walls.rs"]
mod walls;
use super::api_values::TryIntoApi;
use super::commands::Target;
use super::{
    Game,
    creatures::SpawnSite,
    potions::{Victim, mob_can_be_affected},
};
use mod_api::{DragonPhase, DragonRespawnStage};
use sunrise_client::{
    health::{DamageContainer, DamageLevel, HurtSource, Hurting, Knock, Reduction},
    hud::HudBatch,
    hud_layout, hud_overlays,
    mesher::Vertex,
    mobs::{Damage, DamageKind, Who},
    particles::{ParticleOptions, ParticleType},
    projectiles::{Projectile, ProjectileKind},
    random_source,
    text::Text,
};
use sunrise_client_core::nbt_value;
use sunrise_client_gameplay::dragon_render::{dissolve_color, dissolve_mask, dragon_rays};
use sunrise_core::nbt::Tag;
use sunrise_core::position::BlockPos;
use sunrise_core::{
    geometry::{Aabb, Vec3, trig},
    jmath,
    random::{LegacyRandom, RandomSource},
};
use sunrise_protocol::entity_data::Value;

/// `EnderDragon.recreateFromPacket`: part `part` (`0..8`) gets wire id `dragon_id + 1 + part`
/// using Java's wrapping 32-bit integer arithmetic.
pub(in crate::game) fn dragon_part_id(dragon_id: u32, part: usize) -> u32 {
    dragon_id.wrapping_add(1).wrapping_add(part as u32)
}

/// Inverse of [`dragon_part_id`]: returns the `0..8` part index when `candidate_id` belongs to `dragon_id`.
pub(in crate::game) fn dragon_part_index(dragon_id: u32, candidate_id: u32) -> Option<usize> {
    let offset = candidate_id.wrapping_sub(dragon_id.wrapping_add(1));
    (offset < 8).then_some(offset as usize)
}

/// `HealOrHarmMobEffect.applyInstantaneousEffect` at an `AreaEffectCloud`'s
/// 0.5 scale: `(int) (scale * (6 << amplifier) + 0.5)`.
fn harm_amount(amplifier: u32) -> f32 {
    (0.5 * f64::from(6u32 << amplifier) + 0.5) as i32 as f32
}

/// Whether a cloud at `at` (its floor) reaches a living entity: its box is
/// `radius * 2` wide and 0.5 tall, and the entity's position must lie within
/// the radius horizontally.
fn cloud_hits(at: [f64; 3], radius: f32, bounds: Aabb, pos: Vec3) -> bool {
    let r = f64::from(radius);
    let area = Aabb::new(
        Vec3::new(at[0] - r, at[1], at[2] - r),
        Vec3::new(at[0] + r, at[1] + 0.5, at[2] + r),
    );
    let (dx, dz) = (pos.x - at[0], pos.z - at[2]);
    bounds.intersects(area) && dx * dx + dz * dz <= f64::from(radius * radius)
}

/// `AreaEffectCloud.WAIT_TIME` default: the cloud is inert for its first 20
/// ticks (no radius growth, no effects).
const BREATH_WAIT: i32 = 20;

/// `AreaEffectCloud.serverTick`: discarded once `tickCount - waitTime >=
/// duration`, so the wait does not count against the duration.
pub(super) fn breath_expired(age: i32, duration: i32) -> bool {
    age - BREATH_WAIT >= duration
}

/// `AreaEffectCloud.serverTick` victims: every 5 ticks after the wait, drop
/// entries whose delay (`reapplicationDelay` 20) has passed, then return the
/// candidates not in the map, recording them until `age + 20`.
fn breath_victims<T: Copy + PartialEq>(
    age: i32,
    victims: &mut Vec<(T, i32)>,
    candidates: impl IntoIterator<Item = T>,
) -> Vec<T> {
    if age < BREATH_WAIT || age % 5 != 0 {
        return Vec::new();
    }
    victims.retain(|&(_, until)| age < until);
    let mut hit = Vec::new();
    for who in candidates {
        if !victims.iter().any(|&(v, _)| v == who) {
            victims.push((who, age + 20));
            hit.push(who);
        }
    }
    hit
}

/// `DragonFireball.onHit`: the cloud moves to the first living entity in the
/// fireball's box inflated by (4, 2, 4) that is within 4 blocks of it.
/// `feet` is the fireball's position (the bottom of its 1x1x1 box).
fn fireball_cloud_origin(feet: [f64; 3], targets: &[(Aabb, Vec3)]) -> [f64; 3] {
    let area = Aabb::new(
        Vec3::new(feet[0] - 4.5, feet[1] - 2.0, feet[2] - 4.5),
        Vec3::new(feet[0] + 4.5, feet[1] + 3.0, feet[2] + 4.5),
    );
    for (bounds, pos) in targets {
        let (dx, dy, dz) = (pos.x - feet[0], pos.y - feet[1], pos.z - feet[2]);
        if bounds.intersects(area) && dx * dx + dy * dy + dz * dz < 16.0 {
            return [pos.x, pos.y, pos.z];
        }
    }
    feet
}

/// Mob ids from here on are dragon parts (`SPECIAL + part`). End crystals use their real entity ids.
pub(super) const SPECIAL: u32 = u32::MAX - 4096;

/// Direct projectile metadata for `hit_special_full` (both shared projectiles and `ThrownTrident`).
pub(super) struct DirectHit<'a> {
    pub id: u32,
    pub position: Vec3,
    pub kind: Option<&'a sunrise_client::projectiles::ProjectileKind>,
    projectile: Option<&'a Projectile>,
    entity_type: Option<&'static str>,
    pub attack: Option<&'a sunrise_client::health::Attack>,
}
impl<'a> DirectHit<'a> {
    pub(super) fn from_projectile(p: &'a Projectile) -> Self {
        Self {
            id: p.id,
            position: p.position,
            kind: Some(&p.kind),
            projectile: Some(p),
            entity_type: None,
            attack: None,
        }
    }
    pub(super) fn trident(
        id: u32,
        position: Vec3,
        attack: Option<&'a sunrise_client::health::Attack>,
    ) -> Self {
        Self {
            id,
            position,
            kind: Some(&ProjectileKind::Trident),
            projectile: None,
            entity_type: Some("minecraft:trident"),
            attack,
        }
    }
    fn resolve_entity_type(&self) -> Option<&'static str> {
        self.entity_type
            .or_else(|| self.projectile.map(Projectile::entity_type))
    }
}

/// `EnderDragon.recreateFromPacket`: signed wrapping `i32` id of part `part` (`i + dragonId + 1`).
pub(in crate::game) fn dragon_part_wire_id(dragon_id: u32, part: usize) -> i32 {
    dragon_part_id(dragon_id, part) as i32
}
const FIGHT_FILE: &str = "data/minecraft/ender_dragon_fight.dat";
pub(super) const MAX_HEALTH: f32 = 200.0;
/// `Mob.getAmbientSoundInterval`.
const AMBIENT_INTERVAL: i32 = 80;

/// A full hit's default knockback (`dealDefaultKnockback`): where it comes from, and the
/// dragon's knockback resistance when it lands, which the hurt alone cannot read.
#[derive(Clone, Copy, Debug)]
struct Push {
    from: Vec3,
    resistance: f64,
}

#[derive(Default)]
pub(super) struct Fight {
    pub(super) loaded: bool,
    needs_scan: bool,
    pub killed: bool,
    pub previously_killed: bool,
    /// `dragonUUID`: the fight's own dragon. Only its death wins the fight
    /// (`setDragonKilled`); a dragon summoned after the fight is won is not it.
    dragon_uuid: Option<[i32; 4]>,
    /// `ticksSinceDragonSeen`: ticks since the fight's dragon last ticked
    /// (`updateDragon`); at 1200 the fight looks for another one.
    dragon_unseen: i32,
    gateways: Vec<i32>,
    exit_portal: Option<[i32; 3]>,
    pub(super) alive_crystals: i32,
    crystal_scan: i32,
    /// `ticksSinceLastPlayerScan`, and whether the last scan found a player
    /// for the fight (`dragonEvent.getPlayers()` is not empty).
    player_scan: i32,
    players: bool,
    /// The spikes' crystals have been placed (worldgen does not place entities here).
    crystals_placed: bool,
    /// The respawn sequence (`DragonRespawnStage`) and its tick count.
    respawn: Option<(Respawn, i32)>,
    /// The four crystals on the exit portal driving the respawn.
    respawn_crystals: Vec<Vec3>,
    /// The player behind the blast under way, whose `explosion` damage
    /// source names them: a crystal's (`EndCrystal.hurtServer`) or a large
    /// fireball's (`Explosion.getIndirectSourceEntity`, its owner).
    blast_by: Option<FightWho>,
    /// The last hit the dragon took (`die(source)`'s source, for the
    /// `player_killed_entity` trigger's `killing_blow`): its damage type and
    /// whether a player was behind it.
    last_blow: Option<(DamageKind, bool)>,
    dirty: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Respawn {
    Start,
    Preparing,
    Pillars,
    Summoning,
}
impl Fight {
    /// `EnderDragonFight.tick`'s ticket condition: the fight's boss bar has a
    /// player (`dragonEvent.getPlayers()` is not empty), which holds the
    /// arena's `TicketType.DRAGON` ticket. The bar is scanned every 20 ticks,
    /// as `updatePlayers` is.
    pub(super) fn arena_watched(&self) -> bool {
        self.players
    }
    /// The fight as mods read it.
    pub(super) fn api(&self) -> mod_api::DragonFightState {
        mod_api::DragonFightState {
            won: self.killed,
            previously_won: self.previously_killed,
            respawn_stage: self.respawn.map(|(stage, _)| stage.api()),
            alive_crystals: self.alive_crystals,
        }
    }
}
impl Respawn {
    fn name(self) -> &'static str {
        match self {
            Respawn::Start => "start",
            Respawn::Preparing => "preparing_to_summon_pillars",
            Respawn::Pillars => "summoning_pillars",
            Respawn::Summoning => "summoning_dragon",
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        [
            Respawn::Start,
            Respawn::Preparing,
            Respawn::Pillars,
            Respawn::Summoning,
        ]
        .into_iter()
        .find(|r| r.name() == name)
    }
    pub(super) fn api(self) -> DragonRespawnStage {
        match self {
            Respawn::Start => DragonRespawnStage::Start,
            Respawn::Preparing => DragonRespawnStage::PreparingToSummonPillars,
            Respawn::Pillars => DragonRespawnStage::SummoningPillars,
            Respawn::Summoning => DragonRespawnStage::SummoningDragon,
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Crystal {
    /// Entity-owned registration metadata; content clones are detached.
    pub(super) attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub join: sunrise_client::join_seq::JoinSeq,
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub position: Vec3,
    /// `Entity.xo`, `yo`, `zo`: where it was when its tick began (`setOldPosAndRot`). It never
    /// moves itself; a client's copy eases to where the server sends it.
    pub previous: Vec3,
    pub rotation: [f32; 2],
    pub fire: i32,
    pub show_bottom: bool,
    /// EndCrystal.time (the bob and spin), starting random.
    pub time: i32,
    pub beam: Option<[i32; 3]>,
    /// `setPermanentlyInvulnerable` (respawned spike crystals).
    pub invulnerable: bool,
    /// `onGround` and `fallDistance`: a crystal never moves itself, so they stay
    /// as loaded until `/tp` grounds it or a cobweb or powder snow resets the
    /// fall (`crystal.rs`).
    pub fall: (bool, f64),
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<sunrise_client::pose::SizeAnswer>,
}
sunrise_client::impl_joined!(Crystal);
sunrise_mod_runtime::impl_attachment_lifetime!(Crystal);
impl Crystal {
    /// `EntityType.END_CRYSTAL.getDimensions()`: width, height and eye height.
    pub(super) const DIMENSIONS: [f32; 3] = [2.0, 2.0, 1.7];
    /// `EntityType.END_CRYSTAL`: 2 x 2.
    pub(super) fn bounds(&self) -> Aabb {
        let p = self.position;
        let vanilla = Aabb::new(
            Vec3::new(p.x - 1.0, p.y, p.z - 1.0),
            Vec3::new(p.x + 1.0, p.y + 2.0, p.z + 1.0),
        );
        sunrise_client::pose::SizeAnswer::sized_box(self.size_answer, Self::DIMENSIONS, p, vanilla)
    }
}
#[derive(Clone, Debug, Default)]
struct Path {
    nodes: Vec<[i32; 3]>,
    next: usize,
}
impl Path {
    fn done(&self) -> bool {
        self.next >= self.nodes.len()
    }
    fn advance(&mut self) {
        self.next += 1;
    }
    fn next_pos(&self) -> [i32; 3] {
        self.nodes[self.next]
    }
}
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Holding {
        path: Option<PathBox>,
        target: Option<Vec3>,
        clockwise: bool,
    },
    Strafe {
        charge: i32,
        path: Option<PathBox>,
        target: Option<Vec3>,
        clockwise: bool,
        /// `attackTarget`: the player it strafes.
        player: Option<FightWho>,
    },
    LandingApproach {
        path: Option<PathBox>,
        target: Option<Vec3>,
    },
    Landing {
        target: Option<Vec3>,
    },
    Takeoff {
        first: bool,
        path: Option<PathBox>,
        target: Option<Vec3>,
    },
    Scanning {
        time: i32,
    },
    Attacking {
        ticks: i32,
    },
    Flaming {
        ticks: i32,
    },
    Charging {
        target: Option<Vec3>,
        since: i32,
    },
    Dying {
        target: Option<Vec3>,
        time: i32,
    },
    Hover {
        target: Option<Vec3>,
    },
}
/// A path compared by identity only (phases compare by kind).
#[derive(Clone, Debug)]
struct PathBox(Path);
impl PartialEq for PathBox {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Phase {
    /// `EnderDragonPhase` ids.
    fn id(&self) -> i32 {
        match self {
            Self::Holding { .. } => 0,
            Self::Strafe { .. } => 1,
            Self::LandingApproach { .. } => 2,
            Self::Landing { .. } => 3,
            Self::Takeoff { .. } => 4,
            Self::Flaming { .. } => 5,
            Self::Scanning { .. } => 6,
            Self::Attacking { .. } => 7,
            Self::Charging { .. } => 8,
            Self::Dying { .. } => 9,
            Self::Hover { .. } => 10,
        }
    }
    fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Strafe {
                charge: 0,
                path: None,
                target: None,
                clockwise: false,
                player: None,
            },
            2 => Self::LandingApproach {
                path: None,
                target: None,
            },
            3 => Self::Landing { target: None },
            4 => Self::Takeoff {
                first: true,
                path: None,
                target: None,
            },
            5 => Self::Flaming { ticks: 0 },
            6 => Self::Scanning { time: 0 },
            7 => Self::Attacking { ticks: 0 },
            8 => Self::Charging {
                target: None,
                since: 0,
            },
            9 => Self::Dying {
                target: None,
                time: 0,
            },
            10 => Self::Hover { target: None },
            _ => Self::Holding {
                path: None,
                target: None,
                clockwise: false,
            },
        }
    }
    fn sitting(&self) -> bool {
        matches!(
            self,
            Self::Scanning { .. }
                | Self::Attacking { .. }
                | Self::Flaming { .. }
                | Self::Hover { .. }
        )
    }
    fn target(&self) -> Option<Vec3> {
        match self {
            Self::Holding { target, .. }
            | Self::Strafe { target, .. }
            | Self::LandingApproach { target, .. }
            | Self::Landing { target }
            | Self::Takeoff { target, .. }
            | Self::Charging { target, .. }
            | Self::Dying { target, .. }
            | Self::Hover { target } => *target,
            _ => None,
        }
    }
    fn fly_speed(&self) -> f32 {
        match self {
            Self::Landing { .. } => 1.5,
            Self::Charging { .. } | Self::Dying { .. } => 3.0,
            // DragonHoverPhase.getFlySpeed.
            Self::Hover { .. } => 1.0,
            _ => 0.6,
        }
    }
}
pub(in crate::game) struct Dragon {
    pub(super) residency: residency::DragonResidency,
    pub join: sunrise_client::join_seq::JoinSeq,
    /// `Entity.id` once tracked to guests (0 before); its parts take the
    /// eight ids after it (`EnderDragon`'s constructor reserves them).
    pub id: u32,
    pub position: Vec3,
    pub previous: Vec3,
    pub(super) velocity: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub previous_yaw: f32,
    /// LivingEntity.yBodyRot, distinct from the flight and part yaw.
    pub body_yaw: f32,
    head_yaw: f32,
    body_rotation: sunrise_client::mobs::control::BodyRotation,
    yaw_accel: f32,
    pub health: f32,
    pub hurt_time: i32,
    /// `LivingEntity.damageCooldownTime` and `lastHurt`: within the window
    /// only damage above the last hit lands, and only the excess.
    invulnerable: i32,
    last_hurt: f32,
    /// Transient `LivingEntity.lastHurtMob`; an owned arrow or wind charge can set it.
    pub(super) last_hurt_mob: Option<(u32, i32)>,
    /// `lastHurtByPlayer` with its 100 tick memory: the player credited with
    /// the kill, and the dragon tick of their last hit.
    hurt_by_player: Option<(FightWho, i32)>,
    pub last_source: Option<(&'static str, Option<i32>, Option<i32>)>,
    pub death_time: i32,
    pub(in crate::game) death_canceled: bool,
    pub flap: f32,
    pub old_flap: f32,
    in_wall: bool,
    phase: Phase,
    /// `Mob.isAggressive` (the synced mob flag), `persistenceRequired` and
    /// the target a mod set (`Mob.target`): kept for native mods, the
    /// dragon's own flight never reads them.
    pub(super) aggressive: bool,
    pub(super) persistent: bool,
    pub(super) target: Option<u32>,
    /// The phase left was `Flaming` (`DragonSittingFlamingPhase.end`) and its
    /// cloud is still to be discarded.
    pub(super) flame_ended: bool,
    flame_count: i32,
    sitting_damage: f32,
    /// `LivingEntity.equipment`: nothing in the fight reads it; it is kept
    /// for mods and saves.
    pub equipment: sunrise_items::equipment::Equipment,
    /// The live attribute map (`AttributeMap`), built when an item with
    /// attribute modifiers is worn or it is read.
    pub live: sunrise_client::living_attributes::LivingAttributes,
    growl: i32,
    /// `Mob.ambientSoundTime`.
    ambient_time: i32,
    pub(super) ticks: i32,
    /// DragonFlightHistory: (y, yRot) samples, newest at `head`.
    history: [(f64, f32); 64],
    head: usize,
    /// head, neck, body, tail1-3, wing1-2.
    pub parts: [residency::DragonPart; 8],
    nodes: Option<[[i32; 3]; 24]>,
    nearest_crystal: Option<usize>,
    /// The player a crystal destroyed while it held its pattern sends it
    /// after (`strafe_after_crystal`), taken on its next tick.
    strafe_request: Option<FightWho>,
    /// `Mob.isNoAi` (`NoAI`): it flaps in place, with no phase or flight.
    pub(super) no_ai: bool,
    pub(super) random: LegacyRandom,
}
/// A player in the fight: the window's own player, or a guest by
/// connection (a LAN or server player).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FightWho {
    Host,
    Guest(u32),
}
/// A player in the End as the dragon sees it.
#[derive(Clone, Copy, Debug)]
pub(super) struct FightPlayer {
    pub(super) who: FightWho,
    pub(super) position: Vec3,
    pub(super) bounds: Aabb,
    pub(super) alive: bool,
    creative: bool,
    spectator: bool,
}
impl FightPlayer {
    /// `Player.canBeSeenAsEnemy`: alive, neither creative nor spectator.
    pub(super) fn enemy(&self) -> bool {
        self.alive && !self.creative && !self.spectator
    }
}
/// `TargetingConditions.forCombat` (visibility aside) in
/// `Level.getNearestPlayer`: the enemy nearest `at` among `players`, within
/// `range` of the dragon at `from` and passing `selector`; none on Peaceful.
fn nearest_enemy(
    players: &[FightPlayer],
    peaceful: bool,
    at: Vec3,
    range: Option<(Vec3, f64)>,
    selector: impl Fn(&FightPlayer) -> bool,
) -> Option<FightPlayer> {
    if peaceful {
        return None;
    }
    players
        .iter()
        .filter(|p| p.enemy() && selector(p))
        .filter(|p| range.is_none_or(|(from, r)| p.position.distance_squared(from) <= r * r))
        .min_by(|a, b| {
            a.position
                .distance_squared(at)
                .total_cmp(&b.position.distance_squared(at))
        })
        .copied()
}
/// Whether `EndCrystal.hurtServer` creates a blast before notifying the fight.
#[derive(Clone, Copy, PartialEq)]
enum CrystalDamage {
    Direct,
    Explosion,
}
/// What `EnderDragon.hurt` made of a hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Hurt {
    /// It returned true; `full` when it was a new hit, not just the excess
    /// over the last one (the hurt sound and animation).
    Landed { full: bool },
    /// It returned false (dying, too little damage, fire on a part).
    Refused,
    /// A perched dragon's `onHurt` took a direct arrow's or wind charge's
    /// damage away (and set it alight for a second).
    Shrugged,
}
/// The dragon's body among its parts (head, neck, body, ...).
const BODY: usize = 2;
struct DragonHitOrigin<'a> {
    direct: Option<&'a ProjectileKind>,
    entity_id: Option<i32>,
    from: Option<Vec3>,
    invulnerability_checked: bool,
    hook_source: Option<HurtSource<'a>>,
    attack: Option<&'a sunrise_client::health::Attack>,
    hooks: DragonHitHooks,
}
struct DragonLethalHit {
    before_health: f32,
    phase_id: i32,
    post: Option<std::sync::Arc<sunrise_client::health::DamagePost>>,
}
enum DragonLethal {
    None,
    Completed,
    Pending(DragonLethalHit),
}
struct DragonHurt {
    hurt: Hurt,
    lethal: DragonLethal,
}
impl DragonHurt {
    const fn plain(hurt: Hurt) -> Self {
        Self {
            hurt,
            lethal: DragonLethal::None,
        }
    }
}
#[derive(Clone, Copy, Default)]
struct DragonHitHooks {
    invulnerability: bool,
    death: bool,
    death_post: bool,
    any: bool,
}
impl DragonHitHooks {
    fn capture() -> Self {
        if mod_api::__private::mods().is_empty() {
            return Self::default();
        }
        let invulnerability =
            mod_api::__private::enabled_listened(mod_api::HookId::EntityInvulnerabilityCheck);
        let death = mod_api::__private::enabled_listened(mod_api::HookId::LivingDeath);
        let death_post = mod_api::__private::enabled_listened(mod_api::HookId::LivingDeathPost);
        let any = invulnerability
            || death
            || death_post
            || [
                mod_api::HookId::LivingIncomingDamage,
                mod_api::HookId::LivingShieldBlock,
                mod_api::HookId::LivingDamagePre,
                mod_api::HookId::LivingDamagePost,
                mod_api::HookId::LivingKnockback,
            ]
            .into_iter()
            .any(mod_api::__private::enabled_listened);
        Self {
            invulnerability,
            death,
            death_post,
            any,
        }
    }
}
/// A direct `AbstractArrow` or `WindCharge`, which a perched dragon shrugs off
/// (`AbstractDragonSittingPhase.onHurt`); a breeze's `BreezeWindCharge` is neither.
fn shrugs(direct: Option<&ProjectileKind>) -> bool {
    matches!(
        direct,
        Some(ProjectileKind::Arrow { .. } | ProjectileKind::Trident | ProjectileKind::WindCharge)
    )
}
/// `Vec3.normalize`: zero under a length of `1.0E-5F`.
fn vanilla_normalize(v: glam::DVec3) -> glam::DVec3 {
    let len = v.length();
    if len < f64::from(1.0e-5_f32) {
        glam::DVec3::ZERO
    } else {
        v / len
    }
}

/// `DragonStrafePlayerPhase`'s firing angle: `Math.acos` of the float dot, one
/// `dmul` by `Mth.RAD_TO_DEG`, narrowed to float, then `+ 0.5F`.
fn strafe_angle(dot: f32) -> f32 {
    (jmath::acos(f64::from(dot)) * f64::from(trig::RAD_TO_DEG)) as f32 + 0.5
}
/// `#minecraft:is_fire`, which the dragon's parts and the crystals refuse
/// (`isInvulnerableToBase`: both types are fire immune).
fn is_fire(kind: DamageKind) -> bool {
    let source = Damage {
        kind,
        attacker: None,
        from: None,
        direct: None,
        attacker_species: None,
    };
    source.is_fire()
}
const PART_SIZES: [(f64, f64); 8] = [
    (1.0, 1.0),
    (3.0, 3.0),
    (5.0, 3.0),
    (2.0, 2.0),
    (2.0, 2.0),
    (2.0, 2.0),
    (4.0, 2.0),
    (4.0, 2.0),
];
const ADJACENCY: [u32; 24] = [
    6146, 8197, 8202, 16404, 32808, 32848, 65696, 131392, 131712, 263424, 526848, 525313, 1581057,
    3166214, 2138120, 6373424, 4358208, 12910976, 9044480, 9706496, 15216640, 13688832, 11763712,
    8257536,
];
/// A* over the dragon's 24 fixed nodes, in Minecraft's open-set order.
fn search_dragon_path(
    nodes: &[[i32; 3]; 24],
    from: usize,
    to: usize,
    min: usize,
    last: Option<[i32; 3]>,
) -> Option<Path> {
    use sunrise_client::mobs::path::heap::{Key, OpenSet};
    let dist = |a: [i32; 3], b: [i32; 3]| {
        (((a[0] - b[0]).pow(2) + (a[1] - b[1]).pow(2) + (a[2] - b[2]).pow(2)) as f32).sqrt()
    };
    let mut g = [f32::MAX; 24];
    let mut came = [usize::MAX; 24];
    let mut closed = [false; 24];
    let mut keys = [Key::default(); 24];
    let mut open = OpenSet::with_capacity(keys.len());
    open.insert(&mut keys, from as u32, dist(nodes[from], nodes[to]));
    g[from] = 0.0;
    let mut closest = from;
    while let Some(cur) = open.pop(&mut keys) {
        let cur = cur as usize;
        if cur == to {
            closest = to;
            break;
        }
        if dist(nodes[cur], nodes[to]) < dist(nodes[closest], nodes[to]) {
            closest = cur;
        }
        closed[cur] = true;
        for i in min..24 {
            if ADJACENCY[cur] & (1 << i) != 0 && !closed[i] {
                let tentative = g[cur] + dist(nodes[cur], nodes[i]);
                if tentative < g[i] {
                    g[i] = tentative;
                    came[i] = cur;
                    let f = tentative + dist(nodes[i], nodes[to]);
                    if keys[i].is_open() {
                        open.change_cost(&mut keys, i as u32, f);
                    } else {
                        open.insert(&mut keys, i as u32, f);
                    }
                }
            }
        }
    }
    if closest == from && from != to {
        return None;
    }
    let mut list = vec![nodes[closest]];
    let mut n = closest;
    while came[n] != usize::MAX {
        n = came[n];
        list.insert(0, nodes[n]);
    }
    if let Some(l) = last {
        list.push(l);
    }
    Some(Path {
        nodes: list,
        next: 0,
    })
}
impl Dragon {
    /// `Entity.teleportSetPosition` with the command's relative motion flags.
    pub(super) fn teleport_motion(&mut self, relative: [bool; 3]) {
        self.velocity = super::commands::teleport_velocity(self.velocity, relative, false);
    }
    /// `EnderDragon`'s constructor: full health, hovering (`EnderDragonPhase.HOVERING`). Only
    /// `EnderDragonFight.createNewDragon` sets the holding pattern; a dragon from `/summon`, a
    /// spawn egg or a save without `DragonPhase` keeps hovering where it was put.
    pub(super) fn new(position: Vec3, yaw: f32, seed: i64) -> Self {
        Self {
            residency: Default::default(),
            join: Default::default(),
            id: 0,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw,
            pitch: 0.0,
            previous_yaw: yaw,
            body_yaw: yaw,
            head_yaw: yaw,
            body_rotation: Default::default(),
            yaw_accel: 0.0,
            health: MAX_HEALTH,
            hurt_time: 0,
            invulnerable: 0,
            last_hurt: 0.0,
            last_hurt_mob: None,
            hurt_by_player: None,
            last_source: None,
            death_time: 0,
            death_canceled: false,
            flap: 0.0,
            old_flap: 0.0,
            in_wall: false,
            phase: Phase::Hover { target: None },
            aggressive: false,
            persistent: false,
            target: None,
            flame_ended: false,
            flame_count: 0,
            sitting_damage: 0.0,
            equipment: Default::default(),
            live: Default::default(),
            growl: 100,
            ambient_time: 0,
            ticks: 0,
            history: [(position.y, yaw); 64],
            head: 0,
            // The parts are entities of their own, at the origin until the first
            // `aiStep` places them (`tickPart`).
            parts: std::array::from_fn(|_| Default::default()),
            nodes: None,
            nearest_crystal: None,
            strafe_request: None,
            no_ai: false,
            random: LegacyRandom::new(seed),
        }
    }
    /// DragonFlightHistory.record.
    fn record(&mut self) {
        self.head = (self.head + 1) & 63;
        self.history[self.head] = (self.position.y, self.yaw);
    }
    /// DragonFlightHistory.get(delay).
    fn sample(&self, delay: usize) -> (f64, f32) {
        self.history[(self.head + 64 - (delay & 63)) & 63]
    }
    /// `DragonFlightHistory.get(delay, partial)`, with
    /// `EnderDragonRenderer.extractRenderState`'s frozen history on death.
    fn sample_lerp(&self, delay: usize, partial: f32) -> (f64, f32) {
        let partial = if self.alive() { partial } else { 0.0 };
        let a = self.sample(delay);
        let b = self.sample(delay + 1);
        let yaw = b.1 + wrap(a.1 - b.1) * partial;
        (b.0 + (a.0 - b.0) * f64::from(partial), yaw)
    }
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }
    /// The entity's own box, not any of the separate `EnderDragonPart` boxes.
    pub(super) fn body_bounds(&self) -> Aabb {
        // EntityTypes.ENDER_DRAGON dimensions: 16 wide, 8 high.
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        )
    }
    /// Where part `i` stands and its box.
    pub(super) fn part_geometry(&self, i: usize) -> (Vec3, Aabb) {
        (self.parts[i].0, self.part_box(i))
    }
    fn part_box(&self, i: usize) -> Aabb {
        let (w, h) = PART_SIZES[i];
        let p = self.parts[i].0;
        Aabb::new(
            Vec3::new(p.x - w / 2.0, p.y, p.z - w / 2.0),
            Vec3::new(p.x + w / 2.0, p.y + h, p.z + w / 2.0),
        )
    }
    /// `EnderDragonPart.hurtServer`: `isInvulnerableToBase` refuses fire
    /// (the dragon's type is fire immune) before `EnderDragon.hurt`.
    /// `direct` is the projectile that hit: a direct `AbstractArrow` or a
    /// `WindCharge` is the arrow a perched dragon shrugs off, a breeze's
    /// `BreezeWindCharge` is neither. Gameplay calls `hurt_part_from`; this
    /// is the no-attacker form the tests use.
    #[cfg(test)]
    fn hurt_part(
        &mut self,
        level: &mut DamageLevel<'_>,
        part: usize,
        damage: f32,
        counts: Option<HurtSource<'_>>,
        kind: DamageKind,
        direct: Option<&ProjectileKind>,
    ) -> Hurt {
        self.hurt_part_from(level, part, damage, counts, kind, direct, None, false)
    }
    /// `hurt_part` with where a full hit's default knockback comes from: the melee attacker's
    /// position (`DamageSource.getSourcePosition`, the direct entity's feet) and the dragon's
    /// knockback resistance at the hit. `None` for a hit that is not a player's melee.
    #[cfg(test)]
    fn hurt_part_from(
        &mut self,
        level: &mut DamageLevel<'_>,
        part: usize,
        damage: f32,
        counts: Option<HurtSource<'_>>,
        kind: DamageKind,
        direct: Option<&ProjectileKind>,
        push: Option<Push>,
        invulnerability_checked: bool,
    ) -> Hurt {
        self.hurt_part_from_outcome(
            level,
            part,
            damage,
            counts,
            kind,
            direct,
            push,
            invulnerability_checked,
            false,
        )
        .hurt
    }
    #[allow(clippy::too_many_arguments)]
    fn hurt_part_from_outcome(
        &mut self,
        level: &mut DamageLevel<'_>,
        part: usize,
        damage: f32,
        counts: Option<HurtSource<'_>>,
        kind: DamageKind,
        direct: Option<&ProjectileKind>,
        push: Option<Push>,
        invulnerability_checked: bool,
        defer_lethal: bool,
    ) -> DragonHurt {
        // The game-level invulnerability hook includes the part's fire immunity in
        // vanilla's answer. Only the standalone test adapter still needs this guard.
        if is_fire(kind) && !invulnerability_checked {
            return DragonHurt::plain(Hurt::Refused);
        }
        self.hurt_from_outcome(
            level,
            part,
            damage,
            counts,
            shrugs(direct),
            push,
            invulnerability_checked,
            defer_lethal,
        )
    }
    /// `EnderDragon.hurt(part, source, damage)`: whether the hit landed
    /// (an arrow is spent, else it turns back). `counts` is the source
    /// `reallyHurt` takes when `source.getEntity() instanceof Player` or its
    /// type always hurts dragons (explosions), and none otherwise; `arrow` a
    /// direct arrow or wind charge, which a perched dragon shrugs off
    /// (`AbstractDragonSittingPhase.onHurt`).
    #[cfg(test)]
    fn hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        part: usize,
        damage: f32,
        counts: Option<HurtSource>,
        arrow: bool,
    ) -> Hurt {
        self.hurt_from(level, part, damage, counts, arrow, None, false)
    }
    /// `EnderDragon.handleKillingBlow`: a flying dragon resets to 1 health and
    /// starts its dying flight; a perched dragon stays at 0 health.
    fn apply_killing_blow(&mut self) {
        if !self.phase.sitting() {
            self.health = 1.0;
            self.set_phase(Phase::Dying {
                target: None,
                time: 0,
            });
        }
    }
    /// `EnderDragon.hurt` after `reallyHurt`: perched damage counts toward takeoff.
    fn finish_sitting_damage(&mut self, before: f32) {
        if self.phase.sitting() {
            self.sitting_damage += before - self.health.max(0.0);
            if self.sitting_damage > 0.25 * MAX_HEALTH {
                self.sitting_damage = 0.0;
                self.set_phase(Phase::Takeoff {
                    first: true,
                    path: None,
                    target: None,
                });
            }
        }
    }
    /// Whether the phase, part scaling and source guards reach LivingEntity.
    fn reaches_base_check(&self, part: usize, damage: f32, counts: bool, arrow: bool) -> bool {
        let damage = if part != 0 && part != 1 {
            damage / 4.0 + damage.min(1.0)
        } else {
            damage
        };
        self.alive()
            && !matches!(self.phase, Phase::Dying { .. })
            && !(arrow && self.phase.sitting())
            && damage >= 0.01
            && counts
    }
    fn hurt_from(
        &mut self,
        level: &mut DamageLevel<'_>,
        part: usize,
        damage: f32,
        counts: Option<HurtSource<'_>>,
        arrow: bool,
        push: Option<Push>,
        invulnerability_checked: bool,
    ) -> Hurt {
        self.hurt_from_outcome(
            level,
            part,
            damage,
            counts,
            arrow,
            push,
            invulnerability_checked,
            false,
        )
        .hurt
    }
    #[allow(clippy::too_many_arguments)]
    fn hurt_from_outcome(
        &mut self,
        level: &mut DamageLevel<'_>,
        part: usize,
        damage: f32,
        counts: Option<HurtSource<'_>>,
        arrow: bool,
        push: Option<Push>,
        invulnerability_checked: bool,
        defer_lethal: bool,
    ) -> DragonHurt {
        if matches!(self.phase, Phase::Dying { .. }) || !self.alive() {
            return DragonHurt::plain(Hurt::Refused);
        }
        if arrow && self.phase.sitting() {
            return DragonHurt::plain(Hurt::Shrugged);
        }
        let mut damage = damage;
        if part != 0 && part != 1 {
            damage = damage / 4.0 + damage.min(1.0);
        }
        if damage < 0.01 {
            return DragonHurt::plain(Hurt::Refused);
        }
        let Some(source) = counts else {
            return DragonHurt::plain(Hurt::Landed { full: false });
        };
        if !invulnerability_checked
            && level.invulnerability_check(
                sunrise_client::health::Hurting::Dragon(self.id as i32, source),
                false,
            )
        {
            return DragonHurt::plain(Hurt::Landed { full: false });
        }
        let before = self.health;
        // reallyHurt: LivingEntity.hurtServer, past its guards: the hit's damage sequence
        // starts, and `LivingIncomingDamageEvent` may change or cancel it. A canceled hit
        // still returns true.
        let mut container = DamageContainer::new(damage);
        let hurting = sunrise_client::health::Hurting::Dragon(self.id as i32, source);
        // The dragon's head turns with its body and never pitches.
        let view = sunrise_world::view_direction::view_vector(0.0, self.yaw);
        let victim = mod_api::entity::health::Victim {
            position: [self.position.x, self.position.y, self.position.z],
            view: [view.x, view.y, view.z],
        };
        let body = super::native_living_body::dragon_body(self);
        if level
            .reborrow()
            .with_victim(body)
            .incoming_canceled(hurting, victim, &mut container)
        {
            return DragonHurt::plain(Hurt::Landed { full: false });
        }
        damage = container.new_damage();
        if damage < 0.0 {
            damage = 0.0;
        }
        // LivingEntity.applyItemBlocking: the dragon raises nothing, but a native
        // mod may block for it.
        if damage > 0.0 {
            let hurting = sunrise_client::health::Hurting::Dragon(self.id as i32, source);
            let block = sunrise_client::health::no_block(damage);
            let blocked = level.shield_block(hurting, damage, block).map_or(0.0, |b| {
                container.record_shield_damage(b.shield_damage.max(0) as f32);
                b.blocked_damage
            });
            damage -= blocked;
            container.record_block(blocked);
        }
        if !damage.is_finite() {
            damage = f32::MAX;
        }
        container.record_new_damage(damage);
        // LivingEntity.hurtServer: within the cooldown only what exceeds the last hit.
        let window = self.invulnerable > 10;
        container.record_should_cause_side_effects(!window);
        let mut dealt = if window {
            if damage <= self.last_hurt {
                return DragonHurt::plain(Hurt::Landed { full: false });
            }
            container.record_post_attack_invulnerability_ticks(self.invulnerable);
            container.reduce(
                Reduction::Invulnerability,
                damage,
                self.last_hurt,
                damage - self.last_hurt,
            )
        } else {
            damage
        };
        // LivingEntity.actuallyHurt: the dragon wears no armor and takes no effects.
        // NeoForge still subtracts the armor-stage answer from the live container amount before Pre.
        let before_armor = container.new_damage();
        dealt = container.reduce(Reduction::Armor, before_armor, before_armor - dealt, dealt);
        let hurting = sunrise_client::health::Hurting::Dragon(self.id as i32, source);
        container.record_new_damage(dealt);
        let body = super::native_living_body::dragon_body(self);
        level
            .reborrow()
            .with_victim(body)
            .damage_pre(hurting, &mut container);
        dealt = container.capture_inflicted();
        // LivingEntity.actuallyHurt: absorption runs after Pre even with 0.0 absorption hearts.
        let (remaining, _, _) = container.absorb(dealt, 0.0);
        if remaining != 0.0 {
            self.health -= remaining;
            container.record_health_damage(remaining);
        }
        self.last_hurt = damage;
        self.invulnerable = container.post_attack_invulnerability_ticks();
        if !window {
            self.hurt_time = 10;
        }
        let full = !window;
        if !defer_lethal {
            let lethal = self.health <= 0.0;
            if lethal {
                if self.phase.sitting() {
                    self.health = 0.0;
                } else {
                    self.apply_killing_blow();
                }
            }
            if full && let Some(Push { from, resistance }) = push {
                self.knockback(
                    level,
                    Knock {
                        strength: f64::from(0.4_f32),
                        x: from.x - self.position.x,
                        z: from.z - self.position.z,
                    },
                    source,
                    resistance,
                );
            }
            let hurting = sunrise_client::health::Hurting::Dragon(self.id as i32, source);
            let body = super::native_living_body::dragon_body(self);
            level
                .reborrow()
                .with_victim(body)
                .damage_post(hurting, &container);
            self.finish_sitting_damage(before);
            return DragonHurt {
                hurt: Hurt::Landed { full },
                lethal: if lethal {
                    DragonLethal::Completed
                } else {
                    DragonLethal::None
                },
            };
        }
        // `LivingEntity.hurtServer` calls `dealDefaultKnockback` (strength
        // `0.4F`) on a full hit, before `die` and before `EnderDragon.hurt` adds
        // sitting damage and maybe takes off. `from` is the attacker's feet for a
        // player's melee hit. Explosions are `#no_knockback`; their push is
        // `explosion_knockback`.
        if full && let Some(Push { from, resistance }) = push {
            self.knockback(
                level,
                Knock {
                    strength: f64::from(0.4_f32),
                    x: from.x - self.position.x,
                    z: from.z - self.position.z,
                },
                source,
                resistance,
            );
        }
        let lethal = self.health <= 0.0;
        let phase_id = self.phase.id();
        if lethal {
            self.health = 0.0;
        }
        // The last of `LivingEntity.hurtServer` (`reallyHurt`'s `super.hurtServer`):
        // `LivingDamageEvent.Post`, health taken or not. A lethal hit hands completion
        // back to `hurt_dragon_part_at` so death hooks run before Post.
        let hurting = sunrise_client::health::Hurting::Dragon(self.id as i32, source);
        let body = super::native_living_body::dragon_body(self);
        let completion = level
            .reborrow()
            .with_victim(body)
            .finish_damage(hurting, container, lethal);
        if lethal {
            return DragonHurt {
                hurt: Hurt::Landed { full },
                lethal: DragonLethal::Pending(DragonLethalHit {
                    before_health: before,
                    phase_id,
                    post: completion.and_then(|c| c.post),
                }),
            };
        }
        self.finish_sitting_damage(before);
        DragonHurt::plain(Hurt::Landed { full })
    }
    /// `EnderDragon.knockback`: `LivingEntity.knockback` only when the phase
    /// is not sitting. `noPhysics` never sets `onGround`, so the vertical component stays.
    pub(super) fn knockback(
        &mut self,
        level: &mut DamageLevel<'_>,
        mut knock: Knock,
        source: HurtSource<'_>,
        resistance: f64,
    ) -> bool {
        !self.phase.sitting()
            && !level.knockback_canceled(Hurting::Dragon(self.id as i32, source), &mut knock)
            && knock.apply(&mut self.velocity, resistance, false, &mut self.random)
    }
    /// `ServerExplosion.hurtEntities` on the dragon: the dragon itself (its
    /// body takes it), then each part, in `Level.getEntities` order, each by
    /// its distance and how much of its box the blast sees (`exposure`).
    fn explosion_hits(
        &self,
        center: [f64; 3],
        radius: f32,
        exposure: impl Fn(Aabb) -> f32,
    ) -> Vec<(usize, f32)> {
        let diameter = f64::from(radius * 2.0);
        let dist = |p: Vec3| {
            ((p.x - center[0]).powi(2) + (p.y - center[1]).powi(2) + (p.z - center[2]).powi(2))
                .sqrt()
                / diameter
        };
        let p = self.position;
        let whole = Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        );
        std::iter::once((2, p, whole))
            .chain((0..8).map(|i| (i, self.parts[i].0, self.part_box(i))))
            .filter_map(|(part, at, b)| {
                let d = dist(at);
                (d <= 1.0).then(|| {
                    let pow = (1.0 - d) * f64::from(exposure(b));
                    (
                        part,
                        ((pow * pow + pow) / 2.0 * 7.0 * diameter + 1.0) as f32,
                    )
                })
            })
            .collect()
    }
    /// `ServerExplosion.hurtEntities` knockback on the dragon itself:
    /// `pushFromExplosion` adds `(1 - dist) * exposure` along the blast to
    /// its eye to the motion (`Entity.push`); the parts are not pushed.
    pub(super) fn whole_box(&self) -> Aabb {
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        )
    }
    pub(super) fn explosion_knockback(
        &self,
        center: [f64; 3],
        radius: f32,
        exposure: f32,
    ) -> Option<[f64; 3]> {
        let diameter = f64::from(radius * 2.0);
        let p = self.position;
        let at = |y: f64| [p.x - center[0], y - center[1], p.z - center[2]];
        let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let dist = len(at(p.y)) / diameter;
        if dist > 1.0 {
            return None;
        }
        // The dragon's eye is 0.85 of its 8 block height, in floats.
        let away = at(p.y + f64::from(8.0f32 * 0.85f32));
        let l = len(away);
        // `Vec3.normalize` gives zero below 1.0E-5F, then `scale(power)`.
        if l < 9.999_999_747_378_752e-6 {
            return Some([0.0; 3]);
        }
        let power = (1.0 - dist) * f64::from(exposure);
        Some(away.map(|d| d / l * power))
    }
    /// `DragonHoldingPatternPhase.onCrystalDestroyed`: strafe `player` if
    /// the dragon can attack them (`EnderDragon.canAttack`, on Peaceful too).
    /// The first such crystal wins: the dragon strafes from then on, and
    /// `onCrystalDestroyed` does nothing in that phase.
    fn strafe_after_crystal(&mut self, player: Option<&FightPlayer>) {
        if let Some(p) = player
            && p.enemy()
            && matches!(self.phase, Phase::Holding { .. })
            && self.strafe_request.is_none()
        {
            self.strafe_request = Some(p.who);
        }
    }
    /// The phase as a native mod sees it.
    pub(super) fn native_phase(&self) -> DragonPhase {
        DragonPhase::from_id(self.phase.id()).expect("the phases are the vanilla ones")
    }
    /// `EnderDragonPhaseManager.setPhase`: nothing when it is the phase it
    /// holds (a strafing dragon too), else the new phase begins fresh.
    pub(super) fn set_native_phase(&mut self, phase: DragonPhase) {
        if phase != self.native_phase() {
            self.set_phase(Phase::from_id(phase.id()));
        }
    }
    fn set_phase(&mut self, phase: Phase) {
        if self.phase.id() == phase.id() && phase.id() != 1 {
            return;
        }
        // `EnderDragonPhaseManager.setPhase`: the outgoing `end()` first.
        if matches!(self.phase, Phase::Flaming { .. }) {
            self.flame_ended = true;
        }
        if let Phase::Flaming { .. } = phase {
            self.flame_count += 1;
        }
        self.phase = phase;
    }
}
/// The source `reallyHurt` takes from a hit `by` the player of that entity
/// id: `DamageSources.explosion(crystal, player)` is `player_explosion` with
/// a player behind it, who is the causing entity, and the direct one of a
/// melee hit. A projectile is the direct entity only when the damage type
/// names one; `magic()` (an ownerless wither skull) does not.
fn hurt_source(kind: DamageKind, by: Option<i32>, direct: Option<i32>) -> HurtSource<'static> {
    let causing = by.unwrap_or(0);
    let damage_type = super::creatures::damage_type(kind, by.is_some());
    let projectile = direct.filter(|_| sunrise_client::health::names_its_projectile(damage_type));
    HurtSource {
        damage_type,
        causing,
        direct: projectile.unwrap_or(if super::creatures::melee(kind) {
            causing
        } else {
            0
        }),
        position: None,
        direct_type: None,
    }
}
/// `Mth.wrapDegrees(float)`: Java's `%` (truncating, as Rust's), then one step of 360.
fn wrap(d: f32) -> f32 {
    let mut r = d % 360.0;
    if r >= 180.0 {
        r -= 360.0;
    }
    if r < -180.0 {
        r += 360.0;
    }
    r
}
/// `Mth.wrapDegrees(double)`.
fn wrap64(d: f64) -> f64 {
    let mut r = d % 360.0;
    if r >= 180.0 {
        r -= 360.0;
    }
    if r < -180.0 {
        r += 360.0;
    }
    r
}
impl Game<'_> {
    fn fight_path(&self) -> std::path::PathBuf {
        sunrise_client::save::dimension_dir(&self.world_root, "minecraft:the_end").join(FIGHT_FILE)
    }
    /// The saved fight, or `createDefault` with the gateways shuffled by the seed.
    pub(super) fn load_fight(&mut self) {
        if self.fight.loaded {
            return;
        }
        let tag = sunrise_client::save::read_dat(&self.fight_path());
        let data = tag.as_ref().and_then(|t| t.get("data"));
        let flag = |k: &str, default: bool| match data.and_then(|d| d.get(k)) {
            Some(Tag::Byte(v)) => *v != 0,
            _ => default,
        };
        let mut fight = Fight {
            loaded: true,
            needs_scan: flag("needs_state_scanning", true),
            killed: flag("dragon_killed", false),
            previously_killed: flag("previously_killed", false),
            crystals_placed: flag("sunrise_crystals_placed", false),
            gateways: match data.and_then(|d| d.get("gateways")) {
                Some(Tag::IntArray(v)) => v.clone(),
                Some(Tag::List(v)) => v
                    .iter()
                    .filter_map(|t| match t {
                        Tag::Int(i) => Some(*i),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
            respawn: match data.and_then(|d| d.get("respawn_stage")) {
                Some(Tag::String(name)) => {
                    Respawn::from_name(&name.to_string().unwrap_or_default()).map(|r| {
                        let time = match data.and_then(|d| d.get("respawn_time")) {
                            Some(Tag::Int(t)) => *t,
                            _ => 0,
                        };
                        (r, time)
                    })
                }
                _ => None,
            },
            respawn_crystals: match data.and_then(|d| d.get("sunrise_respawn_crystals")) {
                Some(Tag::IntArray(v)) => v
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|b| {
                        Vec3::new(
                            f64::from(b[0]) + 0.5,
                            f64::from(b[1]),
                            f64::from(b[2]) + 0.5,
                        )
                    })
                    .collect(),
                _ => Vec::new(),
            },
            exit_portal: match data.and_then(|d| d.get("exit_portal_location")) {
                Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
                _ => None,
            },
            dragon_uuid: match data.and_then(|d| d.get("dragon_uuid")) {
                Some(Tag::IntArray(v)) if v.len() == 4 => Some([v[0], v[1], v[2], v[3]]),
                _ => None,
            },
            // The first tick scans for players.
            player_scan: 21,
            ..Default::default()
        };
        if fight.gateways.is_empty() {
            // Util.shuffle(0..20, RandomSource.create(seed)).
            let mut gateways: Vec<i32> = (0..20).collect();
            let mut random = LegacyRandom::new(self.level.seed);
            for i in (2..=gateways.len()).rev() {
                let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
                gateways.swap(i - 1, j);
            }
            fight.gateways = gateways;
            fight.dirty = true;
        }
        // A world from before this fight existed: one with an active portal
        // already won it.
        if tag.is_none() && self.end_portal_active() {
            fight.killed = true;
            fight.previously_killed = true;
            fight.crystals_placed = true;
            fight.needs_scan = false;
        }
        self.fight = fight;
    }
    fn end_portal_active(&self) -> bool {
        (0..256).any(|y| {
            &*self.res.facts().get(self.world.block(0, y, 0)).name == "minecraft:bedrock"
                && (-3..=3).any(|d| {
                    &*self.res.facts().get(self.world.block(d, y, 0)).name == "minecraft:end_portal"
                })
        })
    }
    pub(super) fn save_fight(&mut self) {
        if !self.fight.loaded || !self.fight.dirty {
            return;
        }

        let f = &self.fight;
        let b = |v: bool| Tag::Byte(i8::from(v));
        let mut data = vec![
            ("needs_state_scanning", b(f.needs_scan)),
            ("dragon_killed", b(f.killed)),
            ("previously_killed", b(f.previously_killed)),
            ("respawn_time", Tag::Int(f.respawn.map_or(0, |r| r.1))),
            ("gateways", Tag::IntArray(f.gateways.clone())),
            ("sunrise_crystals_placed", b(f.crystals_placed)),
        ];
        if let Some(p) = f.exit_portal {
            data.push(("exit_portal_location", Tag::IntArray(p.to_vec())));
        }
        if let Some(u) = f.dragon_uuid {
            data.push(("dragon_uuid", Tag::IntArray(u.to_vec())));
        }
        if let Some((stage, _)) = f.respawn {
            data.push(("respawn_stage", Tag::String(stage.name().into())));
            data.push((
                "sunrise_respawn_crystals",
                Tag::IntArray(
                    f.respawn_crystals
                        .iter()
                        .flat_map(|p| [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32])
                        .collect(),
                ),
            ));
        }
        let tag = Tag::compound([
            ("data", Tag::compound(data)),
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
        ]);
        if sunrise_client::save::write_dat(&self.fight_path(), tag).is_ok() {
            self.fight.dirty = false;
        }
    }
    /// The island's centre chunks are lit (the arena is "loaded").
    fn arena_loaded(&self) -> bool {
        (-2..=1).all(|cz| (-2..=1).all(|cx| self.streaming.is_lit(cx, cz)))
    }
    /// A player's entity id: the acting player's body has it, another
    /// player's comes from the hub (0 when no such player is available).
    fn fight_entity(&self, who: FightWho) -> i32 {
        if who == self.acting_player() {
            return self.player.as_ref().map_or(0, |p| p.entity_id);
        }
        let host = self.lan.as_ref();
        match who {
            FightWho::Host => 0,
            FightWho::Guest(conn) => host
                .and_then(|h| h.hub.guest(conn))
                .map_or(0, |g| g.entity_id),
        }
    }
    /// The player acting now: the guest whose turn it is, else the host.
    pub(super) fn acting_player(&self) -> FightWho {
        self.lan_acting.map_or(FightWho::Host, FightWho::Guest)
    }
    /// The causing player shared by the crystal/deflected-fireball blast.
    pub(super) fn crystal_blast_player(&self) -> Option<super::commands::Target> {
        self.fight.blast_by.map(|who| match who {
            FightWho::Host => super::commands::Target::Player,
            FightWho::Guest(conn) => super::commands::Target::Guest(conn),
        })
    }
    /// Names the acting player behind the blast about to go off (a large
    /// fireball they shot), or no one, for the dragon's kill credit.
    pub(super) fn set_blast_by_player(&mut self, by_player: bool) {
        self.fight.blast_by = by_player.then(|| self.acting_player());
    }
    /// The players in the level the dragon sees (`level.players()`): the
    /// window's own, then every guest in the level.
    pub(super) fn fight_players(&self) -> Vec<FightPlayer> {
        let mut out = Vec::new();
        if let Some(p) = self.player.as_ref() {
            out.push(FightPlayer {
                who: FightWho::Host,
                position: p.position,
                bounds: p.bounds(),
                alive: !p.health.dead_or_dying(),
                creative: self.creative,
                spectator: self.spectator(),
            });
        }
        if let Some(h) = self.lan.as_ref() {
            for g in h.hub.guests.iter().filter(|g| h.hub.here(g) && g.loaded) {
                let [x, y, z] = g.position;
                out.push(FightPlayer {
                    who: FightWho::Guest(g.conn),
                    position: Vec3::new(x, y, z),
                    bounds: super::lan_mobs::guest_bounds(g),
                    alive: !g.dead,
                    creative: g.creative(),
                    spectator: g.spectator(),
                });
            }
        }
        out
    }
    /// Run `f` as the fight's player `who` (a guest's turn, its knockback
    /// sent to its client).
    pub(super) fn as_fight_player(&mut self, who: FightWho, f: impl FnOnce(&mut Self)) {
        match who {
            FightWho::Host => f(self),
            FightWho::Guest(conn) => {
                self.guest_hit_by(conn, f);
            }
        }
    }
    /// `EnderDragonFight.validPlayer`: the players alive within 192 blocks of
    /// (0, 128, 0), the host and the guests in the level alike.
    fn fight_has_players(&self) -> bool {
        let near =
            |p: [f64; 3]| p[0].powi(2) + (p[1] - 128.0).powi(2) + p[2].powi(2) <= 192.0 * 192.0;
        let host = self.player.as_ref().is_some_and(|p| {
            !p.health.dead_or_dying() && near([p.position.x, p.position.y, p.position.z])
        });
        host || self.lan.as_ref().is_some_and(|h| {
            h.hub
                .guests
                .iter()
                .any(|g| h.hub.here(g) && g.loaded && !g.dead && near(g.position))
        })
    }
    /// `EnderDragonFight.tick`, before the level ticks its entities.
    pub(super) fn tick_fight(&mut self) {
        self.load_fight();
        let mut canceled = false;
        if mod_api::__private::listened(mod_api::HookId::DragonFightTick) {
            self.native_dragon_fight_tick(self.fight_level(), &mut canceled);
        }
        // updatePlayers, every 20 ticks.
        if !canceled {
            self.fight.player_scan += 1;
            if self.fight.player_scan >= 20 {
                self.fight.player_scan = 0;
                self.fight.players = self.fight_has_players();
            }
        }
        if !self.arena_loaded() {
            return;
        }
        // World generation's, not the fight's: placed whatever a mod does.
        if !self.fight.crystals_placed {
            self.place_spike_crystals();
            self.fight.crystals_placed = true;
            self.fight.dirty = true;
        }
        if !canceled && self.fight.players {
            if self.fight.needs_scan {
                self.scan_state();
                self.fight.needs_scan = false;
                self.fight.dirty = true;
            }
            if self.fight.respawn.is_some() {
                self.tick_respawn();
            }
            if !self.fight.killed {
                // Without a dragon of its own, or 1200 ticks after it last
                // ticked, the fight takes the End's dragon or makes one
                // (`findOrCreateDragon`).
                let look = match self.fight.dragon_uuid {
                    None => true,
                    Some(_) => {
                        self.fight.dragon_unseen += 1;
                        self.fight.dragon_unseen >= 1200
                    }
                };
                if look {
                    self.find_or_create_dragon();
                    self.fight.dragon_unseen = 0;
                }
                self.fight.crystal_scan += 1;
                if self.fight.crystal_scan >= 100 {
                    self.count_crystals();
                }
            }
        }
        self.assign_dragon_and_crystal_ids();
    }

    /// The End's handle, which the fight's hooks are told.
    fn fight_level(&self) -> mod_api::LevelRef {
        self.level_ref(self.lend_epoch(), self.dim.key())
    }

    pub(super) fn assign_dragon_and_crystal_ids(&mut self) {
        // EnderDragon's constructor takes its id and its parts' from the
        // entity counter (a dragon loaded from the save gets them here too).
        if let Some(d) = self.dragon.as_mut()
            && d.id == 0
        {
            super::entity::assign_stored_id(&mut self.next_entity, &mut d.id, None);
            self.next_entity += 8;
            self.dragon.id_assigned();
        }
        // The spikes' crystals too (EndCrystal's constructor): an id whether
        // or not a client has tracked them yet, to be aimed at by it.
        self.assign_resident_crystal_ids();
    }
    /// EndSpikeFeature's crystals (with fire under them on the bedrock).
    fn place_spike_crystals(&mut self) {
        for spike in sunrise_world::worldgen::end_spike::spikes_for_seed(self.level.seed) {
            let at = Vec3::new(
                f64::from(spike.center_x) + 0.5,
                f64::from(spike.height + 1),
                f64::from(spike.center_z) + 0.5,
            );
            if self
                .crystals
                .iter()
                .any(|c| (c.position.x - at.x).abs() < 1.0 && (c.position.z - at.z).abs() < 1.0)
            {
                continue;
            }
            let (drawn, uuid) = Crystal::own_draws();
            let time = Crystal::start_time(at, drawn, self.clock.still());
            // EndCrystal's constructor takes an id from the entity counter.
            self.next_entity += 1;
            self.persist.uuids.insert(self.next_entity, uuid);
            self.crystals
                .push(Crystal::new(self.next_entity, at, true, time));
            self.world
                .modified
                .insert((at.x.floor() as i32 >> 4, at.z.floor() as i32 >> 4));
        }
    }
    /// `updateCrystalCount`: crystals on the spike tops.
    fn count_crystals(&mut self) {
        self.fight.crystal_scan = 0;
        let spikes = sunrise_world::worldgen::end_spike::spikes_for_seed(self.level.seed);
        self.fight.alive_crystals = self
            .crystals
            .iter()
            .filter(|c| {
                spikes.iter().any(|s| {
                    (c.position.x - f64::from(s.center_x) - 0.5).abs() <= f64::from(s.radius)
                        && (c.position.z - f64::from(s.center_z) - 0.5).abs() <= f64::from(s.radius)
                        && c.position.y >= f64::from(s.height)
                })
            })
            .count() as i32;
    }
    /// `EndCrystal.tick` in every dimension: the time (the spin and bob),
    /// and fire at its block where there is a dragon fight (the End).
    pub(super) fn tick_crystals(&mut self) {
        let fire = (self.dimension_type().has_ender_dragon_fight)
            .then(|| self.pack.assets.registry.default_state("minecraft:fire"))
            .flatten();
        for i in 0..self.crystals.len() {
            // Entity.commonTick's setOldPosAndRot.
            self.crystals[i].previous = self.crystals[i].position;
            self.crystals[i].time += 1;
            self.crystal_block_effects(i);
            let p = self.crystals[i].position;
            let b = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
            if let Some(fire) = fire
                && self.res.facts().is_air(self.world.block(b[0], b[1], b[2]))
                && self.world.loaded(b[0], b[2])
            {
                let _ = self.level_set(b, fire);
            }
        }
    }
    /// `EndCrystal.hurtServer`: remove it, blast only for non-explosion damage,
    /// then notify the dragon fight even when no blast was created.
    fn destroy_crystal(&mut self, index: usize, by: Option<FightWho>, damage: CrystalDamage) {
        if index >= self.crystals.len() || self.crystals[index].invulnerable {
            return;
        }
        let c = self.crystals.remove(index);
        // Read before the blast, which can take other respawn crystals.
        let respawn_crystal = self.fight.respawn.is_some()
            && self
                .fight
                .respawn_crystals
                .iter()
                .any(|p| p.distance_squared(c.position) < 0.01);
        if damage == CrystalDamage::Direct {
            let at = [c.position.x, c.position.y, c.position.z];
            // The blast's damage source carries the player, so the dragon
            // remembers them before the blast can kill it.
            self.fight.blast_by = by;
            // EndCrystal.hurtServer: ExplosionInteraction.BLOCK.
            let interaction =
                self.destroy_type(sunrise_client::gamerules::vanilla::BLOCK_EXPLOSION_DROP_DECAY);
            // EndCrystal is 2 by 2 blocks: its box decides `isInWater`.
            let (x, y, z) = (c.position.x, c.position.y, c.position.z);
            self.combat.explosion_in_water = self.bounds_in_water(Aabb::new(
                Vec3::new(x - 1.0, y, z - 1.0),
                Vec3::new(x + 1.0, y + 2.0, z + 1.0),
            ));
            self.combat.explosion_direct = Some((c.id, "minecraft:end_crystal"));
            let _ = self.server_explode_from(
                at,
                6.0,
                interaction,
                false,
                super::explosion::ExplosionSource::default().with_damage_direct(
                    c.id,
                    Some("minecraft:end_crystal"),
                    sunrise_client::health::DirectEntity::Other,
                ),
            );
            self.combat.explosion_direct = None;
            self.combat.explosion_in_water = false;
            self.fight.blast_by = None;
        }
        self.crystal_destroyed(index, &c, by, respawn_crystal);
    }
    /// `EndCrystal.onDestroyedBy`: also called before removal by EndCrystal.kill.
    fn crystal_destroyed(
        &mut self,
        index: usize,
        c: &Crystal,
        by: Option<FightWho>,
        respawn_crystal: bool,
    ) {
        // `level.getDragonFight()`: only a level with a fight hears of it.
        if !self.dimension_type().has_ender_dragon_fight {
            return;
        }
        if respawn_crystal {
            self.abort_respawn();
        } else {
            // EnderDragonFight.onCrystalDestroyed.
            self.count_crystals();
            // EnderDragon.onCrystalDestroyed: the player behind the damage, else
            // the one nearest the crystal's block (`CRYSTAL_DESTROY_TARGETING`,
            // whose range `TargetingConditions.test` skips without a targeter).
            let player = by.or_else(|| {
                let p = c.position;
                let at = Vec3::new(p.x.floor(), p.y.floor(), p.z.floor());
                let peaceful = self.ui.difficulty == 0;
                nearest_enemy(&self.fight_players(), peaceful, at, None, |_| true).map(|p| p.who)
            });
            if let Some(d) = self.dragon.as_mut() {
                if d.nearest_crystal == Some(index) {
                    d.nearest_crystal = None;
                    let hooks = DragonHitHooks::capture();
                    let hook_source = hooks.any.then(|| HurtSource {
                        damage_type: if player.is_some() {
                            "minecraft:player_explosion"
                        } else {
                            "minecraft:explosion"
                        },
                        causing: player.map_or(0, |who| self.fight_entity(who)),
                        direct: c.id as i32,
                        position: Some([c.position.x, c.position.y, c.position.z]),
                        direct_type: Some("minecraft:end_crystal"),
                    });
                    self.hurt_dragon_part_at(
                        0,
                        10.0,
                        true,
                        player,
                        DamageKind::Explosion,
                        DragonHitOrigin {
                            direct: None,
                            entity_id: None,
                            from: None,
                            invulnerability_checked: false,
                            hook_source,
                            attack: None,
                            hooks,
                        },
                    );
                } else if let Some(n) = d.nearest_crystal.as_mut()
                    && *n > index
                {
                    *n -= 1;
                }
                let target =
                    player.and_then(|who| self.fight_players().into_iter().find(|p| p.who == who));
                if let Some(d) = self.dragon.as_mut() {
                    d.strafe_after_crystal(target.as_ref());
                }
            }
        }
        if mod_api::__private::listened(mod_api::HookId::EndCrystalDestroyed) {
            let p = c.position;
            let pos = BlockPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
            self.native_end_crystal_destroyed(self.fight_level(), pos, respawn_crystal);
        }
    }
    /// The boxes of the living dragon's parts a projectile can hit, in `getSubEntities` order.
    pub(super) fn dragon_part_boxes(&self) -> Option<[Aabb; 8]> {
        self.dragon
            .as_ref()
            .filter(|d| d.alive())
            .map(|d| std::array::from_fn(|i| d.part_box(i)))
    }
    /// Whether `id` is a live End crystal entity id.
    pub(in crate::game) fn is_crystal_id(&self, id: u32) -> bool {
        self.crystals.iter().any(|c| c.id == id)
    }
    /// Whether `id` is a dragon part (`SPECIAL..SPECIAL + 8`) or a live End crystal.
    pub(in crate::game) fn is_special_target(&self, id: u32) -> bool {
        id >= SPECIAL || self.is_crystal_id(id)
    }
    /// Targets for projectiles: crystals (by real entity id) and dragon parts.
    pub(super) fn special_targets(&self) -> Vec<(u32, Aabb)> {
        let mut out = Vec::new();
        for c in self.crystals.iter() {
            out.push((c.id, c.bounds()));
        }
        if let Some(boxes) = self.dragon_part_boxes() {
            for (i, part) in boxes.into_iter().enumerate() {
                out.push((SPECIAL + i as u32, part));
            }
        }
        out
    }
    /// Resolves a crosshair target id (including synthetic dragon-part ids) to its wire entity id.
    pub(in crate::game) fn wire_entity_id(&self, id: u32) -> Option<i32> {
        if (SPECIAL..SPECIAL + 8).contains(&id) {
            let dragon = self
                .dragon
                .as_ref()
                .filter(|d| d.id != 0 || self.remote.is_some())?;
            return Some(dragon_part_wire_id(dragon.id, (id - SPECIAL) as usize));
        }
        Some(id as i32)
    }
    /// `ServerLevel.dragonParts`: where the part with that entity id is
    /// and its box.
    pub(super) fn dragon_part(&self, id: u32) -> Option<(Vec3, Aabb)> {
        let (d, i) = self.resident_dragon_part(id)?;
        d.alive().then(|| d.part_geometry(i))
    }
    /// The dragon the level holds and the index (`getSubEntities` order) of
    /// its part with that entity id, while the dragon is there, dying or
    /// not: a part is an entity for as long as its parent is.
    pub(super) fn resident_dragon_part(&self, id: u32) -> Option<(&Dragon, usize)> {
        self.resident_dragon_part_source(id)
    }
    /// Damage to a special id (from projectiles and melee): what the
    /// dragon part or crystal made of it. `direct` is the projectile that hit.
    pub(super) fn hit_special(
        &mut self,
        id: u32,
        amount: f32,
        by_player: bool,
        kind: DamageKind,
        direct: Option<&Projectile>,
    ) -> Hurt {
        let owner = by_player.then_some(Who::Player);
        let guest = self.lan_acting.filter(|_| by_player);
        let hit = |game: &mut Self| {
            game.hit_special_full(
                id,
                amount,
                owner,
                guest,
                kind,
                None,
                direct.map(DirectHit::from_projectile),
            )
        };
        if id >= SPECIAL
            && mod_api::__private::enabled_listened(mod_api::HookId::LivingDeath)
            && let Some(shot) = direct
            && self.combat.attack().projectile != Some(shot.id as i32)
        {
            let attack = self.projectile_attack(shot);
            self.with_attack(attack, hit)
        } else {
            hit(self)
        }
    }
    /// Full-source damage dispatch for a crystal (real entity id) or dragon part.
    pub(super) fn hit_special_full(
        &mut self,
        id: u32,
        amount: f32,
        owner: Option<Who>,
        guest: Option<u32>,
        kind: DamageKind,
        damage_type_override: Option<&str>,
        direct: Option<DirectHit<'_>>,
    ) -> Hurt {
        if let Some(index) = self.crystals.iter().position(|c| c.id == id) {
            // EndCrystal.hurtServer: false while invulnerable and for fire
            // (its type is fire immune), else it blows up.
            let crystal = &self.crystals[index];
            let crystal_id = crystal.id;
            let by_player = owner == Some(Who::Player);
            let creative = match owner {
                Some(Who::Player) => match guest {
                    Some(conn) if self.lan_acting == Some(conn) => self.creative,
                    Some(conn) => self.command_guest(conn).is_some_and(|g| g.creative()),
                    None => self.creative,
                },
                _ => false,
            };
            let vanilla = is_fire(kind) || (crystal.invulnerable && !(by_player && creative));
            if !mod_api::__private::mods().is_empty()
                && mod_api::__private::enabled_listened(mod_api::HookId::EntityInvulnerabilityCheck)
            {
                let causing = match owner {
                    Some(Who::Player) => match guest {
                        Some(conn) => self.command_guest(conn).map_or(0, |g| g.entity_id),
                        None => self.own_entity_id(),
                    },
                    Some(Who::Mob(oid)) => oid as i32,
                    None => 0,
                };
                let damage_type = damage_type_override
                    .unwrap_or_else(|| super::creatures::damage_type(kind, by_player));
                let source = HurtSource {
                    damage_type,
                    causing,
                    direct: direct.as_ref().map_or(causing, |d| d.id as i32),
                    position: direct
                        .as_ref()
                        .map(|d| [d.position.x, d.position.y, d.position.z]),
                    direct_type: direct.as_ref().and_then(DirectHit::resolve_entity_type),
                };
                if self.native_nonliving_invulnerable(
                    crystal_id,
                    "minecraft:end_crystal",
                    source,
                    vanilla,
                ) {
                    return Hurt::Refused;
                }
            } else if vanilla {
                return Hurt::Refused;
            }
            let Some(index) = self.crystals.iter().position(|c| c.id == id) else {
                return Hurt::Refused;
            };
            let by = by_player.then(|| guest.map_or_else(|| self.acting_player(), FightWho::Guest));
            // A listener may allow a hit on an otherwise invulnerable crystal.
            self.crystals[index].invulnerable = false;
            self.destroy_crystal(index, by, CrystalDamage::Direct);
            Hurt::Landed { full: true }
        } else if id >= SPECIAL {
            let by_player = owner == Some(Who::Player);
            let by = by_player.then(|| guest.map_or_else(|| self.acting_player(), FightWho::Guest));
            let hooks = DragonHitHooks::capture();
            let hook_source = hooks.any.then(|| {
                let causing = match owner {
                    Some(Who::Player) => by.map_or(0, |who| self.fight_entity(who)),
                    Some(Who::Mob(oid)) => oid as i32,
                    None => 0,
                };
                let damage_type = damage_type_override
                    .unwrap_or_else(|| super::creatures::damage_type(kind, by_player));
                let (direct_id, direct_type) = match direct
                    .as_ref()
                    .filter(|_| sunrise_client::health::names_its_projectile(damage_type))
                {
                    Some(d) => (d.id as i32, d.resolve_entity_type()),
                    None if super::creatures::melee(kind) => (causing, None),
                    None => (0, None),
                };
                HurtSource {
                    damage_type,
                    causing,
                    direct: direct_id,
                    position: direct
                        .as_ref()
                        .map(|d| [d.position.x, d.position.y, d.position.z]),
                    direct_type,
                }
            });
            let part = (id - SPECIAL) as usize;
            let from = (matches!(kind, DamageKind::PlayerAttack | DamageKind::Thorns))
                .then(|| self.player.as_ref().map(|p| p.position))
                .flatten();
            self.hurt_dragon_part_at(
                part,
                amount,
                by_player,
                by,
                kind,
                DragonHitOrigin {
                    direct: direct.as_ref().and_then(|d| d.kind),
                    entity_id: direct.as_ref().map(|d| d.id as i32),
                    from,
                    invulnerability_checked: false,
                    hook_source,
                    attack: direct.as_ref().and_then(|d| d.attack),
                    hooks,
                },
            )
        } else {
            Hurt::Refused
        }
    }
    /// The player behind a command's damage source (`source.getEntity() instanceof Player`).
    fn command_player(&self, cause: Option<Target>) -> Option<FightWho> {
        match cause {
            Some(Target::Player) => Some(self.acting_player()),
            Some(Target::Guest(conn)) => Some(FightWho::Guest(conn)),
            _ => None,
        }
    }
    /// `DamageCommand` on the dragon: `EnderDragon.hurtServer` is `hurt(body, ...)`, so a
    /// quarter plus up to 1. `reallyHurt` runs only with a player behind the source or an
    /// `#is_explosion` type, and its `LivingEntity.hurtServer` refuses fire (the dragon is fire
    /// immune); `hurt` succeeds either way unless the dragon is dying or the hit is under 0.01.
    /// `by` is the direct entity, `cause` the causing one (`by` without `from`), and `from`
    /// the source position (`at`, else the direct entity's), which `dealDefaultKnockback`
    /// pushes away from for a type not in `#no_knockback`.
    pub(in crate::game) fn command_damage_dragon(
        &mut self,
        id: u32,
        amount: f32,
        ty: &str,
        kind: DamageKind,
        (by, cause, from): (Option<Target>, Option<Target>, Option<Vec3>),
    ) -> bool {
        let player = self.command_player(cause);
        // A perched dragon shrugs off a direct arrow or wind charge (`onHurt`).
        let direct = match by {
            Some(Target::Projectile(p)) => self
                .creatures
                .projectiles
                .iter()
                .find(|q| q.id == p)
                .map(|q| q.kind.clone()),
            Some(Target::Trident(_)) => Some(ProjectileKind::Trident),
            _ => None,
        };
        // `Entity.isInvulnerableToBase` over the saved flag and window
        // (#916), with the attacker's creative mode as `isCreativePlayer`,
        // before the dragon's own borrow of the game.
        let creative = self.creative_source_of(cause.or(by));
        let saved = self.saved_refuses_damage(id, ty, creative);
        let vanilla = saved || is_fire(kind);
        if self.dragon.as_ref().is_none_or(|d| d.id != id) {
            return false;
        }
        let hooks = DragonHitHooks::capture();
        let (owned_direct_type, hook_source_meta) = if hooks.any {
            let attack = self.combat.attack();
            if let Some(direct_entity) = attack.direct {
                (
                    attack.direct_type.clone(),
                    Some((attack.attacker, direct_entity)),
                )
            } else {
                let causing = cause.and_then(|t| self.network_entity_id(t)).unwrap_or(0);
                let direct_entity = by.and_then(|t| self.network_entity_id(t)).unwrap_or(0);
                (
                    by.map(|t| self.target_type(t)),
                    Some((causing, direct_entity)),
                )
            }
        } else {
            (None, None)
        };
        let hook_source = hook_source_meta.map(|(causing, direct_entity)| HurtSource {
            damage_type: ty,
            causing,
            direct: direct_entity,
            position: from.map(|p| [p.x, p.y, p.z]),
            direct_type: owned_direct_type.as_deref(),
        });
        // The world's tags, so a datapack type is classed as vanilla's are.
        let explosion = self.damage_in_tag(ty, "minecraft:is_explosion");
        let no_knockback = self.damage_in_tag(ty, "minecraft:no_knockback");
        let reaches_base = self.dragon.as_ref().is_some_and(|d| {
            d.reaches_base_check(
                BODY,
                amount,
                player.is_some() || explosion,
                shrugs(direct.as_ref()),
            )
        });
        let invulnerable = if reaches_base {
            if hooks.invulnerability
                && let Some(source) = hook_source
            {
                let level =
                    super::entity::damage_level_of!(self, self.combat.attack(), no_push, None);
                level.invulnerability_check(
                    sunrise_client::health::Hurting::Dragon(id as i32, source),
                    vanilla,
                )
            } else {
                vanilla
            }
        } else {
            false
        };
        if invulnerable {
            // Before the hook, this command treated an otherwise valid fire
            // hit as accepted while the dragon's fire immunity absorbed it.
            if is_fire(kind) && !saved {
                let Some(d) = self.dragon.as_mut().filter(|d| d.id == id) else {
                    return false;
                };
                let level =
                    super::entity::damage_level_of!(self, self.combat.attack(), no_push, None);
                return matches!(
                    d.hurt_from(
                        level,
                        BODY,
                        amount,
                        None,
                        shrugs(direct.as_ref()),
                        None,
                        true
                    ),
                    Hurt::Landed { .. }
                );
            }
            return false;
        }
        let Some(d) = self.dragon.as_mut().filter(|d| d.id == id) else {
            return false;
        };
        let counts = player.is_some() || explosion;
        // With no source position the push has no direction (knockback(0.4, 0, 0)).
        let knock = (!no_knockback).then(|| from.unwrap_or(d.position));
        let entity_id = match by {
            Some(Target::Projectile(id) | Target::Trident(id)) => Some(id as i32),
            _ => None,
        };
        let hurt = self.hurt_dragon_part_at(
            BODY,
            amount,
            counts,
            player,
            kind,
            DragonHitOrigin {
                direct: direct.as_ref(),
                entity_id,
                from: knock,
                invulnerability_checked: true,
                hook_source,
                attack: None,
                hooks,
            },
        );
        // A shrugged-off arrow is `onHurt`'s 0: under 0.01, refused.
        matches!(hurt, Hurt::Landed { .. })
    }
    /// `DamageCommand` on an End crystal: `EndCrystal.hurtServer`. `isInvulnerableToBase`
    /// refuses an `Invulnerable` crystal unless the type bypasses it or a creative player is
    /// behind it, and fire; the dragon's own damage is refused too. Else the crystal is removed,
    /// and blows up unless the damage is an explosion.
    pub(super) fn command_damage_crystal(
        &mut self,
        id: u32,
        ty: &str,
        kind: DamageKind,
        direct: Option<Target>,
        cause: Option<Target>,
        position: Option<[f64; 3]>,
    ) -> bool {
        let Some(index) = self.crystals.iter().position(|c| c.id == id) else {
            return false;
        };
        let creative = cause
            .filter(|t| matches!(t, Target::Player | Target::Guest(_)))
            .and_then(|t| self.for_player(t, |g| g.creative))
            .unwrap_or(false);
        let bypass = self.damage_in_tag(ty, "minecraft:bypasses_invulnerability") || creative;
        let vanilla = (self.crystals[index].invulnerable && !bypass) || is_fire(kind);
        let entity_id = |target| match target {
            Some(Target::Player) => self.own_entity_id(),
            Some(Target::Guest(conn)) => self.command_guest(conn).map_or(0, |g| g.entity_id),
            Some(target) => {
                crate::game::native_entities::target_id(target).map_or(0, |id| id as i32)
            }
            None => 0,
        };
        let source = HurtSource {
            damage_type: ty,
            causing: entity_id(cause),
            direct: entity_id(direct),
            position,
            direct_type: None,
        };
        if self.native_nonliving_invulnerable(id, "minecraft:end_crystal", source, vanilla)
            || matches!(cause, Some(Target::Dragon(_)))
        {
            return false;
        }
        // destroy_crystal keeps a crystal invulnerable to everything else.
        self.crystals[index].invulnerable = false;
        let damage = if self.damage_in_tag(ty, "minecraft:is_explosion") {
            CrystalDamage::Explosion
        } else {
            CrystalDamage::Direct
        };
        let player = self.command_player(cause);
        self.destroy_crystal(index, player, damage);
        true
    }
    /// `hurt_dragon_part` with no projectile behind the hit.
    #[cfg(test)]
    fn hurt_dragon(
        &mut self,
        part: usize,
        damage: f32,
        counts: bool,
        by: Option<FightWho>,
        kind: DamageKind,
    ) -> Hurt {
        self.hurt_dragon_part(part, damage, counts, by, kind, None)
    }
    /// `EnderDragonPart.hurtServer` on the dragon in the level. `by` is the
    /// player behind the hit, remembered for the kill credit.
    #[cfg(test)]
    fn hurt_dragon_part(
        &mut self,
        part: usize,
        damage: f32,
        counts: bool,
        by: Option<FightWho>,
        kind: DamageKind,
        direct: Option<&Projectile>,
    ) -> Hurt {
        // `dealDefaultKnockback` for a player's melee: away from the attacker's
        // feet. The scope names that player to the knockback hook.
        let from = (matches!(kind, DamageKind::PlayerAttack | DamageKind::Thorns))
            .then(|| self.player.as_ref().map(|p| p.position))
            .flatten();
        let hooks = DragonHitHooks::capture();
        let hit = |game: &mut Self| {
            game.hurt_dragon_part_at(
                part,
                damage,
                counts,
                by,
                kind,
                DragonHitOrigin {
                    direct: direct.map(|p| &p.kind),
                    entity_id: direct.map(|p| p.id as i32),
                    from,
                    invulnerability_checked: false,
                    hook_source: None,
                    attack: None,
                    hooks,
                },
            )
        };
        if hooks.death
            && let Some(shot) = direct
            && self.combat.attack().projectile != Some(shot.id as i32)
        {
            let attack = self.projectile_attack(shot);
            self.with_attack(attack, hit)
        } else {
            hit(self)
        }
    }
    /// The dragon's `getAttributeValue(KNOCKBACK_RESISTANCE)` now, which both knockback entries
    /// read: its saved attributes over the type's default. 0 without a dragon.
    pub(super) fn dragon_knockback_resistance(&self) -> f64 {
        self.dragon
            .as_ref()
            .and_then(|d| {
                d.live.read(
                    &self.motion.attributes,
                    "minecraft:ender_dragon",
                    "minecraft:knockback_resistance",
                    sunrise_world::attribute_map::ValueKind::Computed,
                    Vec::new,
                )
            })
            .unwrap_or(0.0)
    }
    /// `hurt_dragon_part` with the position a full hit's knockback pushes away from.
    fn hurt_dragon_part_at(
        &mut self,
        part: usize,
        damage: f32,
        counts: bool,
        by: Option<FightWho>,
        kind: DamageKind,
        origin: DragonHitOrigin<'_>,
    ) -> Hurt {
        let by_id = by.map(|who| self.fight_entity(who));
        let base_source = hurt_source(kind, by_id, origin.entity_id);
        let hook_source = origin.hook_source.unwrap_or(base_source);
        let base_count_source = counts.then_some(base_source);
        let hook_count_source = counts.then_some(hook_source);
        // Part fire immunity is independent of LivingEntity's base check.
        // Commands call the dragon itself and have already checked that base.
        if is_fire(kind) && !origin.invulnerability_checked {
            return Hurt::Refused;
        }
        let death_listened = origin.hooks.death;
        let post_listened = origin.hooks.death_post;
        let defer_lethal = death_listened || post_listened;
        let hit_attack = if death_listened {
            let base_attack = origin.attack.unwrap_or_else(|| self.combat.attack());
            let has_attack = origin.attack.is_some()
                || base_attack.direct.is_some()
                || (hook_source.direct != 0 && base_attack.projectile == Some(hook_source.direct))
                || (hook_source.causing != 0 && base_attack.attacker == hook_source.causing);
            (!has_attack
                && counts
                && hook_source.causing != 0
                && hook_source.direct == hook_source.causing)
                .then(|| self.attack_of(hook_source.causing))
        } else {
            None
        };
        if !origin.invulnerability_checked
            && self
                .dragon
                .as_ref()
                .is_some_and(|d| d.reaches_base_check(part, damage, counts, shrugs(origin.direct)))
        {
            let Some(id) = self.dragon.as_ref().map(|d| d.id) else {
                return Hurt::Refused;
            };
            let attacker = by.map(|who| match who {
                FightWho::Host => Target::Player,
                FightWho::Guest(conn) => Target::Guest(conn),
            });
            let creative = self.creative_source_of(attacker);
            let vanilla =
                self.saved_refuses_damage(id, base_source.damage_type, creative) || is_fire(kind);
            if origin.hooks.invulnerability {
                let attack = hit_attack
                    .as_ref()
                    .or(origin.attack)
                    .unwrap_or_else(|| self.combat.attack());
                let level = super::entity::damage_level_of!(self, attack, no_push, None);
                if level.invulnerability_check(
                    sunrise_client::health::Hurting::Dragon(id as i32, hook_source),
                    vanilla,
                ) {
                    return Hurt::Refused;
                }
            } else if vanilla {
                return Hurt::Refused;
            }
        }
        let weapon = death_listened
            .then(|| {
                hit_attack
                    .as_ref()
                    .or(origin.attack)
                    .unwrap_or_else(|| self.combat.attack())
                    .weapon_of(hook_source.direct)
                    .cloned()
            })
            .flatten();
        let plan = {
            let attack = hit_attack
                .as_ref()
                .or(origin.attack)
                .unwrap_or_else(|| self.combat.attack());
            self.prepare_knockback_credit(
                self.dragon.as_ref().map_or(0, |d| d.id) as i32,
                None,
                attack,
            )
        };
        // `getAttributeValue(KNOCKBACK_RESISTANCE)` as the push reads it, before the hit.
        let push = origin.from.map(|from| Push {
            from,
            resistance: self.dragon_knockback_resistance(),
        });
        let dim = self.dim;
        let (id, join, DragonHurt { hurt, lethal }) = {
            let mut credit = plan.lend(&mut self.last_hurt_by_player);
            let Some(d) = self.dragon.as_mut() else {
                return Hurt::Refused;
            };
            let (id, join) = (d.id, d.join);
            let (cooldown, last_hurt) = (d.invulnerable, d.last_hurt);
            let attack = hit_attack
                .as_ref()
                .or(origin.attack)
                .unwrap_or_else(|| self.combat.attack());
            let level = super::entity::damage_level_of!(self, attack, &mut credit, None);
            let outcome = d.hurt_part_from_outcome(
                level,
                part,
                damage,
                hook_count_source,
                kind,
                origin.direct,
                push,
                true,
                defer_lethal,
            );
            // LivingEntity.hurtServer: damage that is applied sets lastHurtByPlayer
            // (100 ticks); a hit the damage cooldown absorbs returns before that.
            if counts
                && matches!(outcome.hurt, Hurt::Landed { .. })
                && (d.invulnerable != cooldown || d.last_hurt != last_hurt)
                && let Some(who) = by
            {
                d.hurt_by_player = Some((who, d.ticks));
            }
            (id, join, outcome)
        };
        let same_dragon = |game: &Self| {
            game.dim == dim
                && !game.target_removed(&Target::Dragon(id))
                && game
                    .dragon
                    .as_ref()
                    .is_some_and(|d| d.id == id && d.join == join)
        };
        let revalidated = match lethal {
            DragonLethal::None => false,
            DragonLethal::Completed => {
                self.fight.last_blow = Some((kind, by.is_some()));
                self.credit_dragon_kill(false);
                false
            }
            DragonLethal::Pending(lethal) => {
                // LivingEntity.die: NeoForge's LivingDeathEvent first, then handleKillingBlow,
                // kill credit, and living_death_post; LivingDamageEvent.Post finishes after die.
                let apply_death = |game: &mut Self| -> Option<Option<u128>> {
                    if !same_dragon(game)
                        || game.dragon.as_ref().is_none_or(|d| {
                            d.alive()
                                || d.death_canceled
                                || d.phase.id() != lethal.phase_id
                                || matches!(d.phase, Phase::Dying { .. })
                        })
                    {
                        return None;
                    }
                    if let Some(d) = game.dragon.as_mut() {
                        d.apply_killing_blow();
                    }
                    game.fight.last_blow = Some((kind, by.is_some()));
                    Some(game.credit_dragon_kill(post_listened))
                };
                let credited = if death_listened {
                    self.storage_scope(|game, _| {
                        let credited =
                            if game.dragon_death_canceled(id, &hook_source, weapon.as_ref()) {
                                if same_dragon(game)
                                    && let Some(d) = game.dragon.as_mut()
                                {
                                    d.death_canceled = !d.alive();
                                    if !d.death_canceled {
                                        d.death_time = 0;
                                    }
                                }
                                None
                            } else {
                                apply_death(game)
                            };
                        (credited, true)
                    })
                } else {
                    apply_death(self)
                };
                if let Some(credited_player) = credited
                    && post_listened
                {
                    // The attacker is the mob that dealt it, 0 for none or a player.
                    let attacker = if by.is_some() {
                        0
                    } else {
                        u32::try_from(hook_source.causing).unwrap_or(0)
                    };
                    self.native_living_death_post(mod_api::__private::LivingDeathPostCall {
                        entity: id,
                        kind: "minecraft:ender_dragon",
                        damage_type: hook_source.damage_type,
                        attacker,
                        player: None,
                        credited_player,
                        message_key: None,
                    });
                }
                if let Some(post) = lethal.post {
                    self.finish_damage_post(id as i32, "minecraft:ender_dragon", post);
                }
                if same_dragon(self)
                    && let Some(d) = self.dragon.as_mut()
                    && d.phase.id() == lethal.phase_id
                {
                    d.finish_sitting_damage(lethal.before_health);
                }
                true
            }
        };
        if revalidated && !same_dragon(self) {
            return hurt;
        }
        let Some(d) = self.dragon.as_mut() else {
            return hurt;
        };
        if hurt == (Hurt::Landed { full: true }) {
            // Mob.playHurtSound: the ambient sound clock starts again.
            d.ambient_time = -AMBIENT_INTERVAL;
            let at = [d.position.x, d.position.y, d.position.z];
            if let Some(s) = base_count_source {
                d.last_source = Some((
                    s.damage_type,
                    (s.causing != 0).then_some(s.causing),
                    (s.direct != 0).then_some(s.direct),
                ));
            } else {
                d.last_source = Some(("minecraft:generic", None, None));
            }
            let pitch = 0.8 + self.world.random.next_float() * 0.3;
            self.play_audio("minecraft:entity.ender_dragon.hurt", Some(at), 5.0, pitch);
            if kind.is("minecraft:thorns") {
                self.play_audio("minecraft:enchant.thorns.hit", Some(at), 1.0, 1.0);
            }
        }
        hurt
    }
    /// `LivingEntity.die`: `getKillCredit().awardKillScore`, for the player
    /// who hurt the dragon within the last 100 ticks. A guest's kill is
    /// theirs: their statistics, scores and advancements.
    fn credit_dragon_kill(&mut self, resolve_uuid: bool) -> Option<u128> {
        let d = self.dragon.as_ref()?;
        let (who, _) = d.hurt_by_player.filter(|(_, hit)| d.ticks - hit <= 100)?;
        let view = sunrise_client::advancements::EntityView {
            kind: "minecraft:ender_dragon".to_owned(),
            position: [d.position.x, d.position.y, d.position.z],
            ..Default::default()
        };
        let id = d.id;
        let blow = self.fight.last_blow.take();
        let guest = match who {
            // Another guest than the one acting: their state, as for a mob's kill.
            FightWho::Guest(conn) if self.lan_acting != Some(conn) => Some(conn),
            _ => None,
        };
        if let Some(conn) = guest
            && let Some(uuid) = self.as_guest(conn, |g| {
                g.award_dragon_kill(id, &view, blow);
                resolve_uuid.then(|| g.lan_acting_uuid.unwrap_or_else(|| g.own_uuid()))
            })
        {
            return uuid;
        }
        self.award_dragon_kill(id, &view, blow);
        resolve_uuid.then(|| self.lan_acting_uuid.unwrap_or_else(|| self.own_uuid()))
    }
    /// `ServerPlayer.awardKillScore` for the dragon, on the killer's state.
    fn award_dragon_kill(
        &mut self,
        id: u32,
        view: &sunrise_client::advancements::EntityView,
        blow: Option<(DamageKind, bool)>,
    ) {
        self.award_stat("killed", "minecraft:ender_dragon", 1);
        self.award_custom_stat("mob_kills", 1);
        self.score_player_killed(super::commands::Target::Mob(id));
        // KilledTrigger: the damage source `die` was given.
        let blow = blow.map(|(kind, by_player)| {
            let mut view = self.damage_view(Damage {
                kind,
                attacker: by_player.then_some(Who::Player),
                from: None,
                direct: None,
                attacker_species: None,
            });
            // `damageSources().explosion(crystal, player)`: the crystal is the direct entity.
            if kind == DamageKind::Explosion {
                view.direct = Some(sunrise_client::advancements::EntityView {
                    kind: "minecraft:end_crystal".to_owned(),
                    ..Default::default()
                });
                view.is_direct = false;
            }
            view
        });
        self.trigger_killed(view, blow.as_ref());
    }
    /// The dragon part or crystal under the crosshair within reach.
    pub(super) fn dragon_hit(&self) -> Option<(u32, f64)> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, _) = hit?;
            let id = entity.id();
            if let Some(d) = self
                .dragon
                .as_ref()
                .filter(|d| d.id != 0 || self.remote.is_some())
                && let Some(part) = dragon_part_index(d.id, id)
            {
                return Some((SPECIAL + part as u32, distance));
            }
            return self
                .special_targets()
                .into_iter()
                .any(|(special, _)| special == id)
                .then_some((id, distance));
        }
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        let (from, to) = super::pick_accumulator::pick_ray_segment(eye, dir, reach);
        let mut pick = super::pick_accumulator::PickAccumulator::new(reach * reach);
        let mut best: Option<(u32, f64)> = None;
        for (id, b) in self.special_targets() {
            if pick.consider_box(self, id, from, to, b) {
                best = Some((id, pick.hovered.unwrap().distance_sq.sqrt()));
            }
        }
        best
    }

    /// The 24 nodes (`findClosestNode` builds them on first use).
    fn dragon_nodes(&mut self) -> [[i32; 3]; 24] {
        if let Some(n) = self.dragon.as_ref().and_then(|d| d.nodes) {
            return n;
        }
        let mut nodes = [[0; 3]; 24];
        for (i, node) in nodes.iter_mut().enumerate() {
            let (r, step, base, add) = if i < 12 {
                (60.0f32, std::f32::consts::PI / 12.0, i, 5)
            } else if i < 20 {
                (40.0, std::f32::consts::PI / 8.0, i - 12, 15)
            } else {
                (20.0, std::f32::consts::PI / 4.0, i - 20, 5)
            };
            let a = 2.0 * (-std::f32::consts::PI + step * base as f32);
            let (x, z) = ((r * a.cos()).floor() as i32, (r * a.sin()).floor() as i32);
            let y = (self.motion_blocking_no_leaves(x, z) + add).max(73);
            *node = [x, y, z];
        }
        if let Some(d) = self.dragon.as_mut() {
            d.nodes = Some(nodes);
        }
        nodes
    }
    /// `getHeightmapPos(MOTION_BLOCKING_NO_LEAVES)`.
    fn motion_blocking_no_leaves(&self, x: i32, z: i32) -> i32 {
        (0..256)
            .rev()
            .find(|&y| {
                let f = self.res.facts().get(self.world.block(x, y, z));
                (f.full_collision
                    || f.legacy_solid
                    || f.fluid != sunrise_client::blocks::Fluid::None)
                    && !f.name.ends_with("_leaves")
            })
            .map_or(0, |y| y + 1)
    }
    fn podium_top(&self) -> Vec3 {
        Vec3::new(0.5, f64::from(self.motion_blocking_no_leaves(0, 0)), 0.5)
    }
    fn closest_node(&mut self, x: f64, y: f64, z: f64) -> usize {
        let nodes = self.dragon_nodes();
        let start = if self.fight.alive_crystals == 0 {
            12
        } else {
            0
        };
        let p = [x.floor() as i32, y.floor() as i32, z.floor() as i32];
        let mut best = (10_000.0f32, 0);
        for (i, n) in nodes.iter().enumerate().skip(start) {
            let d = ((n[0] - p[0]).pow(2) + (n[1] - p[1]).pow(2) + (n[2] - p[2]).pow(2)) as f32;
            if d < best.0 {
                best = (d, i);
            }
        }
        best.1
    }
    /// `findPath` (A* over the adjacency masks), optionally ending at `last`.
    fn dragon_path(&mut self, from: usize, to: usize, last: Option<[i32; 3]>) -> Option<Path> {
        let nodes = self.dragon_nodes();
        let min = if self.fight.alive_crystals == 0 {
            12
        } else {
            0
        };
        search_dragon_path(&nodes, from, to, min, last)
    }
    fn next_target(&mut self, path: &mut Option<PathBox>) -> Option<Vec3> {
        let p = path.as_mut()?;
        if p.0.done() {
            return None;
        }
        let c = p.0.next_pos();
        p.0.advance();
        let d = self.dragon.as_mut()?;
        let y = loop {
            let y = f64::from(c[1]) + f64::from(d.random.next_float() * 20.0);
            if y >= f64::from(c[1]) {
                break y;
            }
        };
        Some(Vec3::new(f64::from(c[0]), y, f64::from(c[2])))
    }
    /// The holding-pattern style path step: around the outer or inner ring.
    fn ring_path(&mut self, clockwise: &mut bool) -> Option<Path> {
        let d = self.dragon.as_mut()?;
        let flip = d.random.next_int_bound(8).unwrap_or(1) == 0;
        let current = {
            let p = d.position;
            self.closest_node(p.x, p.y, p.z)
        };
        let mut target = current as i32;
        if flip {
            *clockwise = !*clockwise;
            target += 6;
        }
        target += if *clockwise { 1 } else { -1 };
        if self.fight.alive_crystals > 0 {
            target = target.rem_euclid(12);
        } else {
            target = ((target - 12) & 7) + 12;
        }
        let mut path = self.dragon_path(current, target as usize, None)?;
        path.advance();
        Some(path)
    }
    /// `DragonStrafePlayerPhase.setTarget`: a path to the player at `p`.
    fn strafe_begin(&mut self, p: Vec3) {
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let dp = d.position;
        let current = self.closest_node(dp.x, dp.y, dp.z);
        let target = self.closest_node(p.x, p.y, p.z);
        let (fx, fz) = (p.x.floor() as i32, p.z.floor() as i32);
        let sd = sunrise_core::geometry::length(f64::from(fx) - dp.x, f64::from(fz) - dp.z);
        let ho = (f64::from(0.4_f32) + sd / 80.0 - 1.0).min(10.0);
        let last = [fx, (p.y + ho).floor() as i32, fz];
        let mut path = self.dragon_path(current, target, Some(last));
        if let Some(path) = path.as_mut() {
            path.advance();
        }
        let mut boxed = path.map(PathBox);
        let target = self.next_target(&mut boxed);
        if let Some(Dragon {
            phase: Phase::Strafe {
                path, target: t, ..
            },
            ..
        }) = self.dragon.as_mut()
        {
            *path = boxed;
            *t = target;
        }
    }
    /// One phase server tick (`doServerTick`).
    fn tick_phase(&mut self) {
        let players = self.fight_players();
        let peaceful = self.ui.difficulty == 0;
        // A crystal destroyed since: strafe the player it named.
        if let Some(d) = self.dragon.as_mut()
            && let Some(who) = d.strafe_request.take()
            && matches!(d.phase, Phase::Holding { .. })
        {
            let target = players.iter().find(|p| p.who == who).copied();
            if let Some(t) = target {
                d.set_phase(Phase::Strafe {
                    charge: 0,
                    path: None,
                    target: None,
                    clockwise: false,
                    player: Some(t.who),
                });
                self.strafe_begin(t.position);
            }
        }
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let pos = d.position;
        let mut phase = d.phase.clone();
        let collided = false;
        let far = |t: Option<Vec3>| {
            t.map_or(0.0, |t| {
                (t.x - pos.x).powi(2) + (t.y - pos.y).powi(2) + (t.z - pos.z).powi(2)
            })
        };
        let podium = self.podium_top();
        // The egg's place (`getHeightmapPos` at the podium) and its centre.
        let egg = Vec3::new(0.0, podium.y, 0.0);
        let egg_centre = Vec3::new(0.5, podium.y + 0.5, 0.5);
        let mut next: Option<Phase> = None;
        // Where the player a new strafe goes after stands.
        let mut strafe_at: Option<Vec3> = None;
        match &mut phase {
            Phase::Holding {
                path,
                target,
                clockwise,
            } => {
                let dt = far(*target);
                if dt < 100.0 || dt > 22500.0 || collided {
                    let done = path.as_ref().is_some_and(|p| p.0.done());
                    let mut chosen = false;
                    if done {
                        let crystals = self.fight.alive_crystals;
                        let r = &mut self.dragon.as_mut().expect("dragon").random;
                        if r.next_int_bound(crystals + 3).unwrap_or(1) == 0 {
                            next = Some(Phase::LandingApproach {
                                path: None,
                                target: None,
                            });
                            chosen = true;
                        } else if let Some(nearest) =
                            nearest_enemy(&players, peaceful, egg, None, |_| true)
                        {
                            let dist = nearest.position.distance_squared(egg_centre) / 512.0;
                            let r = &mut self.dragon.as_mut().expect("dragon").random;
                            if r.next_int_bound((dist + 2.0) as i32).unwrap_or(1) == 0
                                || r.next_int_bound(crystals + 2).unwrap_or(1) == 0
                            {
                                next = Some(Phase::Strafe {
                                    charge: 0,
                                    path: None,
                                    target: None,
                                    clockwise: false,
                                    player: Some(nearest.who),
                                });
                                strafe_at = Some(nearest.position);
                                chosen = true;
                            }
                        }
                    }
                    if !chosen {
                        if path.as_ref().is_none_or(|p| p.0.done()) {
                            *path = self.ring_path(clockwise).map(PathBox);
                        }
                        if let Some(t) = self.next_target(path) {
                            *target = Some(t);
                        }
                    }
                }
            }
            Phase::Strafe {
                charge,
                path,
                target,
                clockwise,
                player,
            } => {
                // Its target as it is now, whatever became of it; gone from
                // the level, or never found: back to the holding pattern.
                let tracked = player.and_then(|who| players.iter().find(|p| p.who == who));
                let Some(pp) = tracked.map(|p| p.position) else {
                    next = Some(Phase::Holding {
                        path: None,
                        target: None,
                        clockwise: false,
                    });
                    self.dragon.as_mut().expect("dragon").phase = phase;
                    if let Some(n) = next {
                        self.dragon.as_mut().expect("dragon").set_phase(n);
                    }
                    return;
                };
                if path.as_ref().is_some_and(|p| p.0.done()) {
                    let dist = sunrise_core::geometry::length(pp.x - pos.x, pp.z - pos.z);
                    let ho = (f64::from(0.4_f32) + dist / 80.0 - 1.0).min(10.0);
                    *target = Some(Vec3::new(pp.x, pp.y + ho, pp.z));
                }
                let dt = far(*target);
                if dt < 100.0 || dt > 22500.0 {
                    if path.as_ref().is_none_or(|p| p.0.done()) {
                        *path = self.ring_path(clockwise).map(PathBox);
                    }
                    if let Some(t) = self.next_target(path) {
                        *target = Some(t);
                    }
                }
                let d2 = (pp.x - pos.x).powi(2) + (pp.y - pos.y).powi(2) + (pp.z - pos.z).powi(2);
                // Entity.hasLineOfSight: eye to eye (the dragon's eye is 0.85 of
                // its 8 block height), blocks only.
                let sees = d2 < 4096.0 && {
                    let eye = Vec3::new(pos.x, pos.y + 6.8, pos.z);
                    let target_eye =
                        tracked.map_or(pp.y + 1.62, |p| p.bounds.max_position().y - 0.18);
                    self.mob_world_base(&[])
                        .line_of_sight(eye, Vec3::new(pp.x, target_eye, pp.z))
                };
                if sees {
                    *charge += 1;
                    let d = self.dragon.as_ref().expect("dragon");
                    let r = f64::from(d.yaw * trig::DEG_TO_RAD);
                    let dir = vanilla_normalize(glam::DVec3::new(
                        f64::from(trig::sin(r)),
                        0.0,
                        f64::from(-trig::cos(r)),
                    ));
                    let aim = vanilla_normalize(glam::DVec3::new(pp.x - pos.x, 0.0, pp.z - pos.z));
                    let angle = strafe_angle(dir.dot(aim) as f32);
                    if *charge >= 5 && (0.0..10.0).contains(&angle) {
                        // From the head, one block back along the view vector.
                        let head = d.parts[0].0;
                        let view = super::view_vector(d.yaw, 0.0);
                        let start =
                            Vec3::new(head.x - view[0], head.y + 0.5 + 0.5, head.z - view[2]);
                        let to = Vec3::new(pp.x - start.x, pp.y + 0.9 - start.y, pp.z - start.z);
                        self.spawn_dragon_fireball(start, to);
                        *charge = 0;
                        next = Some(Phase::Holding {
                            path: None,
                            target: None,
                            clockwise: false,
                        });
                    }
                } else if *charge > 0 {
                    *charge -= 1;
                }
            }
            Phase::LandingApproach { path, target } => {
                let dt = far(*target);
                if dt < 100.0 || dt > 22500.0 || collided {
                    if path.as_ref().is_none_or(|p| p.0.done()) {
                        let current = self.closest_node(pos.x, pos.y, pos.z);
                        // NEAR_EGG_TARGETING: the enemy nearest the egg.
                        let near_egg = nearest_enemy(&players, peaceful, egg, None, |_| true);
                        let target_node = match near_egg.map(|p| p.position) {
                            Some(pp) => {
                                let aim = glam::DVec2::new(pp.x, pp.z).normalize_or_zero();
                                self.closest_node(-aim.x * 40.0, 105.0, -aim.y * 40.0)
                            }
                            None => self.closest_node(40.0, podium.y, 0.0),
                        };
                        let last = [
                            podium.x.floor() as i32,
                            podium.y as i32,
                            podium.z.floor() as i32,
                        ];
                        let mut p = self.dragon_path(current, target_node, Some(last));
                        if let Some(p) = p.as_mut() {
                            p.advance();
                        }
                        *path = p.map(PathBox);
                    }
                    if let Some(t) = self.next_target(path) {
                        *target = Some(t);
                    }
                    if path.as_ref().is_some_and(|p| p.0.done()) {
                        next = Some(Phase::Landing { target: None });
                    }
                }
            }
            Phase::Landing { target } => {
                let t = *target.get_or_insert(podium);
                if (t.x - pos.x).powi(2) + (t.y - pos.y).powi(2) + (t.z - pos.z).powi(2) < 1.0 {
                    self.dragon.as_mut().expect("dragon").flame_count = 0;
                    next = Some(Phase::Scanning { time: 0 });
                }
            }
            Phase::Takeoff {
                first,
                path,
                target,
            } => {
                if !*first && path.is_some() {
                    if (podium.x - pos.x).powi(2)
                        + (podium.y - pos.y).powi(2)
                        + (podium.z - pos.z).powi(2)
                        >= 100.0
                    {
                        next = Some(Phase::Holding {
                            path: None,
                            target: None,
                            clockwise: false,
                        });
                    }
                } else {
                    *first = false;
                    let current = self.closest_node(pos.x, pos.y, pos.z);
                    let look = self.dragon_look();
                    let mut t = self.closest_node(-look[0] * 40.0, 105.0, -look[2] * 40.0) as i32;
                    if self.fight.alive_crystals > 0 {
                        t = t.rem_euclid(12);
                    } else {
                        t = ((t - 12) & 7) + 12;
                    }
                    let mut p = self.dragon_path(current, t as usize, None).map(PathBox);
                    if let Some(pb) = p.as_mut() {
                        pb.0.advance();
                    }
                    if let Some(t) = self.next_target(&mut p) {
                        *target = Some(t);
                    }
                    *path = p.or(Some(PathBox(Path::default())));
                }
            }
            Phase::Scanning { time } => {
                *time += 1;
                // scanTargeting: within 20 blocks, at most 10 above or below.
                let near = nearest_enemy(&players, peaceful, pos, Some((pos, 20.0)), |p| {
                    (p.position.y - pos.y).abs() <= 10.0
                })
                .map(|p| p.position);
                if let Some(pp) = near {
                    if *time > 25 {
                        next = Some(Phase::Attacking { ticks: 0 });
                    } else {
                        let d = self.dragon.as_mut().expect("dragon");
                        let head = d.parts[0].0;
                        scanning_turn(&mut d.yaw, &mut d.yaw_accel, d.position, head, pp);
                    }
                } else if *time >= 100 {
                    // CHARGE_TARGETING: within 150 blocks.
                    let charge =
                        nearest_enemy(&players, peaceful, pos, Some((pos, 150.0)), |_| true);
                    next = Some(match charge {
                        Some(p) => Phase::Charging {
                            target: Some(p.position),
                            since: 0,
                        },
                        None => Phase::Takeoff {
                            first: true,
                            path: None,
                            target: None,
                        },
                    });
                }
            }
            Phase::Attacking { ticks } => {
                // The growl is `doClientTick`'s (`remote_dragon_phase_tick`).
                *ticks += 1;
                if *ticks > 40 {
                    next = Some(Phase::Flaming { ticks: 0 });
                }
            }
            Phase::Flaming { ticks } => {
                *ticks += 1;
                if *ticks >= 200 {
                    let count = self.dragon.as_ref().map_or(0, |d| d.flame_count);
                    next = Some(if count >= 4 {
                        Phase::Takeoff {
                            first: true,
                            path: None,
                            target: None,
                        }
                    } else {
                        Phase::Scanning { time: 0 }
                    });
                } else if *ticks == 10 {
                    let d = self.dragon.as_ref().expect("dragon");
                    let head = d.parts[0].0;
                    let look = glam::DVec2::new(head.x - pos.x, head.z - pos.z).normalize_or_zero();
                    let (x, z) = (head.x + look.x * 2.5, head.z + look.y * 2.5);
                    let mut y = head.y + 0.5;
                    let (xi, zi) = (x.floor() as i32, z.floor() as i32);
                    while y > 0.0
                        && self
                            .res
                            .facts()
                            .is_air(self.world.block(xi, y.floor() as i32, zi))
                    {
                        y -= 1.0;
                    }
                    let id = self.take_entity_id();
                    self.breath_clouds.push(Breath {
                        attachment_lifetime: Default::default(),
                        join: Default::default(),
                        id,
                        at: [x, y.floor() + 1.0, z],
                        motion: [0.0; 3],
                        fall_distance: 0.0,
                        fire_ticks: 0,
                        first_tick: true,
                        radius: 5.0,
                        age: 0,
                        duration: 200,
                        from_dragon: true,
                        victims: Vec::new(),
                        potion: None,
                    });
                }
            }
            Phase::Charging { target, since } => {
                if target.is_none() {
                    next = Some(Phase::Holding {
                        path: None,
                        target: None,
                        clockwise: false,
                    });
                } else if *since > 0 {
                    *since += 1;
                    if *since >= 10 {
                        next = Some(Phase::Holding {
                            path: None,
                            target: None,
                            clockwise: false,
                        });
                    }
                } else {
                    let dt = far(*target);
                    if dt < 100.0 || dt > 22500.0 {
                        *since += 1;
                    }
                }
            }
            Phase::Dying { target, time } => {
                *time += 1;
                let t = *target.get_or_insert(Vec3::new(
                    0.5,
                    f64::from(self.motion_blocking_no_leaves(0, 0)),
                    0.5,
                ));
                let dt = (t.x - pos.x).powi(2) + (t.y - pos.y).powi(2) + (t.z - pos.z).powi(2);
                let d = self.dragon.as_mut().expect("dragon");
                d.health = if !(100.0..=22500.0).contains(&dt) {
                    0.0
                } else {
                    1.0
                };
            }
            Phase::Hover { target } => {
                target.get_or_insert(pos);
            }
        }
        let d = self.dragon.as_mut().expect("dragon");
        d.phase = phase;
        if let Some(n) = next {
            d.set_phase(n);
            if let Some(at) = strafe_at {
                self.strafe_begin(at);
            }
        }
    }
    /// `getHeadLookVector`.
    fn dragon_look(&self) -> [f64; 3] {
        let Some(d) = self.dragon.as_ref() else {
            return [0.0; 3];
        };
        let pitch = match d.phase {
            Phase::Landing { .. } | Phase::Takeoff { .. } => {
                let p = self.podium_top();
                let dist = ((p.x - d.position.x).powi(2)
                    + (p.y - d.position.y).powi(2)
                    + (p.z - d.position.z).powi(2))
                .sqrt() as f32
                    / 4.0;
                -6.0 / dist.max(1.0) * 1.5 * 5.0
            }
            _ if d.phase.sitting() => -45.0,
            _ => 0.0,
        };
        super::view_vector(d.yaw, pitch).map(|v| v)
    }
    /// `getHeadLookVector(1).normalize()`, which the phases' `doClientTick`s
    /// take (a look the dragon has never turned from stays the zero vector).
    fn dragon_look_normalised(&self) -> [f64; 3] {
        let look = self.dragon_look();
        let norm = (look[0] * look[0] + look[1] * look[1] + look[2] * look[2]).sqrt();
        if norm < 1.0e-5 {
            [0.0; 3]
        } else {
            [look[0] / norm, look[1] / norm, look[2] / norm]
        }
    }
    /// The inherited LivingEntity.tick runs Mob.tickHeadTurn after aiStep.
    pub(super) fn tick_dragon(&mut self) {
        self.dragon_ai_step();
        if let Some(d) = self.dragon.as_mut() {
            let (dx, dz) = (d.position.x - d.previous.x, d.position.z - d.previous.z);
            // EnderDragon inherits Mob.getMaxHeadYRot (75 degrees).
            d.body_rotation.tick(
                dx * dx + dz * dz,
                d.yaw,
                &mut d.body_yaw,
                &mut d.head_yaw,
                75.0,
            );
        }
    }

    /// `EnderDragon.aiStep` (and `tickDeath`) for the dragon's own entity tick.
    fn dragon_ai_step(&mut self) {
        // updateDragon runs when the fight dragon ticks or dies, but not with NoAI.
        let ticks = self.dragon.as_ref().is_some_and(|d| !d.alive() || !d.no_ai);
        if ticks && self.is_fight_dragon() {
            self.fight.dragon_unseen = 0;
        }
        // Entity.baseTick: the saved invulnerability window counts down
        // (#916), before the dragon's own borrow of the game.
        if let Some(dragon) = self.dragon.as_ref() {
            let id = dragon.id;
            if !dragon.equipment.is_empty() {
                self.tick_equipment_enchantments(super::commands::Target::Dragon(id), id);
            }
            self.tick_saved_invulnerable_time(id);
        }
        let target_dead = self
            .dragon
            .as_ref()
            .and_then(|d| d.last_hurt_mob)
            .is_some_and(|(id, _)| !self.last_hurt_target_alive(id));
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        d.previous = d.position;
        d.previous_yaw = d.yaw;
        d.old_flap = d.flap;
        d.ticks = d.ticks.wrapping_add(1);
        if target_dead {
            d.last_hurt_mob = None;
        }
        d.hurt_time = (d.hurt_time - 1).max(0);
        d.invulnerable = (d.invulnerable - 1).max(0);
        for part in &mut d.parts {
            part.1 = part.0;
        }
        if !d.alive() {
            self.tick_dragon_death();
            return;
        }
        // Mob.baseTick: the ambient sound, rarer right after one.
        let roll = d.random.next_int_bound(1000).unwrap_or(999);
        let waited = d.ambient_time;
        d.ambient_time += 1;
        if roll < waited {
            d.ambient_time = -AMBIENT_INTERVAL;
            let at = [d.position.x, d.position.y, d.position.z];
            // LivingEntity.getVoicePitch, from the dragon's own random.
            let r = &mut d.random;
            let pitch = (r.next_float() - r.next_float()) * 0.2 + 1.0;
            self.play_audio(
                "minecraft:entity.ender_dragon.ambient",
                Some(at),
                5.0,
                pitch,
            );
        }
        let d = self.dragon.as_mut().expect("dragon");
        // Flapping, growls and the flap sound.
        // Mth.cos(flapTime * 6.2831855F); flapSpeed's `(float) Math.pow(2.0, movement.y)`.
        let flap_was = trig::cos(f64::from(d.old_flap * std::f32::consts::TAU));
        let v = d.velocity;
        let mut flap_speed = 0.2 / (((v.x * v.x + v.z * v.z).sqrt() as f32) * 10.0 + 1.0);
        flap_speed *= 2f64.powf(v.y) as f32;
        d.flap += if d.phase.sitting() {
            0.1
        } else if d.in_wall {
            flap_speed * 0.5
        } else {
            flap_speed
        };
        let flap_now = trig::cos(f64::from(d.flap * std::f32::consts::TAU));
        let at = [d.position.x, d.position.y, d.position.z];
        if flap_was <= -0.3 && flap_now >= -0.3 {
            let pitch = 0.8 + self.world.random.next_float() * 0.3;
            self.play_audio("minecraft:entity.ender_dragon.flap", Some(at), 5.0, pitch);
        }
        let d = self.dragon.as_mut().expect("dragon");
        if !d.phase.sitting() {
            d.growl -= 1;
            if d.growl < 0 {
                d.growl = 200 + d.random.next_int_bound(200).unwrap_or(0);
                let pitch = 0.8 + self.world.random.next_float() * 0.3;
                self.play_audio("minecraft:entity.ender_dragon.growl", Some(at), 2.5, pitch);
            }
        }
        // checkCrystals: heal 1 every 10 ticks from the nearest crystal, found
        // among those touching the dragon's box grown by 32.
        let d = self.dragon.as_mut().expect("dragon");
        if d.nearest_crystal.is_some() && d.ticks % 10 == 0 && d.health < MAX_HEALTH {
            // LivingEntity.setHealth clamps to the maximum.
            d.health = (d.health + 1.0).min(MAX_HEALTH);
        }
        if d.random.next_int_bound(10).unwrap_or(1) == 0 {
            let p = d.position;
            let nearest = self.nearest_crystal_to(p);
            self.dragon.as_mut().expect("dragon").nearest_crystal = nearest;
        }
        let d = self.dragon.as_mut().expect("dragon");
        d.yaw = wrap(d.yaw);
        // EnderDragon.aiStep with isNoAi: the flap stops half way, and nothing
        // else moves it (no flight history, phase, steering or parts).
        if d.no_ai {
            d.flap = 0.5;
            return;
        }
        d.record();
        self.tick_phase();
        // Steering towards the phase's fly target.
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        if let Some(t) = d.phase.target() {
            let (xd, mut yd, zd) = (t.x - d.position.x, t.y - d.position.y, t.z - d.position.z);
            let dist2 = xd * xd + yd * yd + zd * zd;
            let max = f64::from(d.phase.fly_speed());
            let horizontal = (xd * xd + zd * zd).sqrt();
            if horizontal > 0.0 {
                yd = (yd / horizontal).clamp(-max, max);
            }
            d.velocity.y += yd * 0.01;
            d.yaw = wrap(d.yaw);
            // Mth.sin / Mth.cos (the table) and Mth.atan2, as vanilla computes them: a hovering
            // dragon's target is a hair away, where the exact functions turn it another way.
            let aim = vanilla_normalize(glam::DVec3::new(xd, t.y - d.position.y, zd));
            let r = f64::from(d.yaw * trig::DEG_TO_RAD);
            let dir = vanilla_normalize(glam::DVec3::new(
                f64::from(trig::sin(r)),
                d.velocity.y,
                f64::from(-trig::cos(r)),
            ));
            let dot = ((dir.dot(aim) as f32 + 0.5) / 1.5).max(0.0);
            if xd.abs() > f64::from(1.0e-5_f32) || zd.abs() > f64::from(1.0e-5_f32) {
                let rot =
                    (d.velocity.x * d.velocity.x + d.velocity.z * d.velocity.z).sqrt() as f32 + 1.0;
                let turn = match d.phase {
                    Phase::Landing { .. } => rot.min(40.0) / rot,
                    _ => 0.7 / rot.min(40.0) / rot,
                };
                let delta = wrap(180.0 - trig::atan2(xd, zd) as f32 * trig::RAD_TO_DEG - d.yaw)
                    .clamp(-50.0, 50.0);
                d.yaw_accel *= 0.8;
                d.yaw_accel += delta * turn;
                d.yaw += d.yaw_accel * 0.1;
            }
            let span = (2.0 / (dist2 + 1.0)) as f32;
            let speed = 0.06 * (dot * span + (1.0 - span));
            // moveRelative(speed, (0, 0, -1)): getInputVector in doubles, Mth's sine.
            let r = f64::from(d.yaw * trig::DEG_TO_RAD);
            d.velocity.x += f64::from(speed) * f64::from(trig::sin(r));
            d.velocity.z += -f64::from(speed) * f64::from(trig::cos(r));
            let step = if d.in_wall { 0.800000011920929 } else { 1.0 };
            // The dragon's Entity.move is this physics-less step (#2414): it
            // goes through the move gate like every other move, self cause,
            // before the position is written.
            let gate = sunrise_client::move_gate::armed(
                &self.loop_context,
                d.id,
                "minecraft:ender_dragon",
                sunrise_world::entity_move::MoverType::SelfMovement,
                Vec3::new(
                    d.velocity.x * step,
                    d.velocity.y * step,
                    d.velocity.z * step,
                ),
            );
            if !gate.skipped() {
                let moved = gate.delta();
                d.position = Vec3::new(
                    d.position.x + moved.x,
                    d.position.y + moved.y,
                    d.position.z + moved.z,
                );
            }
            let actual =
                vanilla_normalize(glam::DVec3::new(d.velocity.x, d.velocity.y, d.velocity.z));
            let slide = 0.8 + 0.15 * (actual.dot(dir) + 1.0) / 2.0;
            d.velocity = Vec3::new(
                d.velocity.x * slide,
                d.velocity.y * 0.9100000262260437,
                d.velocity.z * slide,
            );
        }
        // EnderDragon.aiStep assigns the body's angle after flight, before placing parts.
        d.body_yaw = d.yaw;
        self.place_dragon_parts();
        self.dragon_contacts();
        // The chunks the dragon passes through re-save their entity lists.
        if let Some(d) = self.dragon.as_ref() {
            for p in [d.previous, d.position] {
                self.world
                    .modified
                    .insert((p.x.floor() as i32 >> 4, p.z.floor() as i32 >> 4));
            }
        }
    }
    /// The part positions from the yaw and the flight history.
    fn place_dragon_parts(&mut self) {
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        // Mth.sin / Mth.cos on floats, as EnderDragon.aiStep places them.
        let msin = |a: f32| trig::sin(f64::from(a));
        let mcos = |a: f32| trig::cos(f64::from(a));
        let tilt = (d.sample(5).0 - d.sample(10).0) as f32 * 10.0 * trig::DEG_TO_RAD;
        let (ct, st) = (mcos(tilt), msin(tilt));
        let r1 = d.yaw * trig::DEG_TO_RAD;
        let (s1, c1) = (msin(r1), mcos(r1));
        let p = d.position;
        let set = |d: &mut Dragon, i: usize, x: f32, y: f64, z: f32| {
            d.parts[i].0 = Vec3::new(p.x + f64::from(x), p.y + y, p.z + f64::from(z));
        };
        set(d, 2, s1 * 0.5, 0.0, -c1 * 0.5);
        set(d, 6, c1 * 4.5, 2.0, s1 * 4.5);
        set(d, 7, c1 * -4.5, 2.0, s1 * -4.5);
        let r2 = d.yaw * trig::DEG_TO_RAD - d.yaw_accel * 0.01;
        let (s2, c2) = (msin(r2), mcos(r2));
        let y_off = if d.phase.sitting() {
            -1.0
        } else {
            d.sample(5).0 - d.sample(0).0
        } as f32;
        set(
            d,
            0,
            s2 * 6.5 * ct,
            f64::from(y_off + st * 6.5),
            -c2 * 6.5 * ct,
        );
        set(
            d,
            1,
            s2 * 5.5 * ct,
            f64::from(y_off + st * 5.5),
            -c2 * 5.5 * ct,
        );
        let p1 = d.sample(5);
        for i in 0..3 {
            let p0 = d.sample(12 + i * 2);
            let rot = d.yaw * trig::DEG_TO_RAD + wrap(p0.1 - p1.1) * trig::DEG_TO_RAD;
            let (ss, cc) = (msin(rot), mcos(rot));
            let dd = (i as f32 + 1.0) * 2.0;
            let x = -(s1 * 1.5 + ss * dd) * ct;
            let y = p0.0 - p1.0 - f64::from((dd + 1.5) * st) + 1.5;
            let z = (c1 * 1.5 + cc * dd) * ct;
            set(d, 3 + i, x, y, z);
        }
    }
    /// `Level.getEntity`: revenge goals must resolve the dragon too.
    pub(super) fn dragon_target(&self) -> Option<(u32, sunrise_client::mobs::TargetInfo)> {
        let d = self.dragon.as_ref().filter(|d| d.alive())?;
        let p = d.position;
        let bounds = Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        );
        Some((
            d.id,
            sunrise_client::mobs::TargetInfo {
                position: p,
                eye_y: p.y + 6.8,
                bounds,
                hitbox: bounds,
                alive: true,
                can_be_seen_as_enemy: true,
                is_player: false,
                creative: false,
                passenger: false,
            },
        ))
    }
    /// Wing knockback, head and neck damage, and breaking blocks in the way.
    fn dragon_contacts(&mut self) {
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let sitting = d.phase.sitting();
        let recently_hurt = d.hurt_time > 0;
        let body = d.part_box(2);
        let (bc, bz) = (
            (body.min_position().x + body.max_position().x) / 2.0,
            (body.min_position().z + body.max_position().z) / 2.0,
        );
        let wings = [d.part_box(6), d.part_box(7)];
        let heads = [d.part_box(0), d.part_box(1)];
        if !recently_hurt {
            // EntitySelector.NO_CREATIVE_OR_SPECTATOR, every player alike.
            let players: Vec<FightPlayer> = self
                .fight_players()
                .into_iter()
                .filter(|p| p.alive && !p.creative && !p.spectator)
                .collect();
            for p in players {
                // knockBack, once for each wing whose zone holds them.
                for w in wings {
                    let zone = w
                        .inflate(Vec3::new(4.0, 2.0, 4.0))
                        .move_by(Vec3::new(0.0, -2.0, 0.0));
                    if zone.intersects(p.bounds) {
                        let (xd, zd) = (p.position.x - bc, p.position.z - bz);
                        let dd = (xd * xd + zd * zd).max(0.1);
                        let push = Vec3::new(xd / dd * 4.0, f64::from(0.2_f32), zd / dd * 4.0);
                        self.as_fight_player(p.who, |g| g.dragon_wing_knock_back(push, sitting));
                    }
                }
                // hurt: the head and the neck.
                for h in heads {
                    if h.inflate(Vec3::new(1.0, 1.0, 1.0)).intersects(p.bounds) {
                        self.as_fight_player(p.who, |g| {
                            g.dragon_hurts_player("minecraft:mob_attack", 10.0);
                        });
                    }
                }
            }
        }
        // EnderDragon.aiStep queries living entities, not only players.
        if let Some(d) = self.dragon.as_ref() {
            let frame = sunrise_gameplay::dragon_contacts::Frame {
                dragon_id: d.id,
                origin: d.position,
                center: (bc, bz),
                wings: wings.map(|w| {
                    w.inflate(Vec3::new(4.0, 2.0, 4.0))
                        .move_by(Vec3::new(0.0, -2.0, 0.0))
                }),
                heads: heads.map(|h| h.inflate(Vec3::new(1.0, 1.0, 1.0))),
                sitting,
                recently_hurt,
            };
            // Keep team lookup in Game (including tamed pets' owner teams).
            let allowed: Vec<bool> = self
                .mobs
                .iter()
                .map(|m| {
                    sunrise_client::scoreboard::teams_allow_damage(
                        self.mob_team(d.id),
                        self.mob_team_of(m),
                    )
                })
                .collect();
            // knockBack(wing1), knockBack(wing2), hurt(head), hurt(neck): each
            // part's list in turn, so Thorns' level and dragon draws follow it.
            for part in 0..4 {
                for (index, &allowed) in allowed.iter().enumerate() {
                    let id = self.mobs[index].id;
                    let plan = self.prepare_knockback_credit(
                        id as i32,
                        Some(Who::Mob(frame.dragon_id)),
                        self.combat.attack(),
                    );
                    let mut credit = plan.lend(&mut self.last_hurt_by_player);
                    if sunrise_gameplay::dragon_contacts::contact_part(
                        crate::game::entity::damage_level_of!(
                            self,
                            &sunrise_client::health::NO_ATTACK,
                            &mut credit,
                            None
                        ),
                        &mut self.mobs[index],
                        &frame,
                        allowed,
                        part,
                    ) {
                        self.finish_mob_damage_at(index);
                        self.mob_post_attack(
                            id,
                            Some(sunrise_client::mobs::Who::Mob(frame.dragon_id)),
                            None,
                            Some("minecraft:mob_attack"),
                        );
                    }
                }
            }
        }
        self.dragon_check_walls();
    }
    /// `Entity.isSilent` of the dragon: its `Silent`, kept with the fields
    /// Sunrise models nowhere (`remember_tags`: `/summon`, a load).
    fn dragon_silent(&self) -> bool {
        let raw = self
            .dragon
            .as_ref()
            .filter(|d| d.id != 0)
            .and_then(|d| self.persist.raw.get(&d.id));
        saved_silent(raw)
    }

    /// `EnderDragonFight.setDragonKilled`.
    fn dragon_killed(&mut self) {
        let _ = self.activate_exit_portal();
        if let Some(g) = self.fight.gateways.pop() {
            let a = 2.0 * (-std::f64::consts::PI + std::f64::consts::PI / 20.0 * f64::from(g));
            let pos = [
                (96.0 * a.cos()).floor() as i32,
                75,
                (96.0 * a.sin()).floor() as i32,
            ];
            self.level_event_at(3000, pos, 0);
            let _ = self.place_gateway_at(pos);
        }
        // getHeightmapPos(MOTION_BLOCKING, getPodiumLocation(origin)).
        let y = {
            use sunrise_world::{heightmap::HeightmapType, worldgen::terrain::TerrainBlockStates};
            let properties = &self.pack.assets.properties;
            self.world.motion_blocking(0, 0, |s| {
                HeightmapType::MotionBlocking.is_opaque(properties.heightmap_state(s))
            })
        };
        let previously_won = self.fight.previously_killed;
        let mut place_egg = None;
        if mod_api::__private::listened(mod_api::HookId::DragonFightWon) {
            let pos = BlockPos::new(0, y, 0);
            self.native_dragon_fight_won(self.fight_level(), pos, previously_won, &mut place_egg);
        }
        if place_egg.unwrap_or(!previously_won)
            && let Some(egg) = self
                .pack
                .assets
                .registry
                .default_state("minecraft:dragon_egg")
        {
            let _ = self.level_set([0, y, 0], egg);
        }
        self.fight.previously_killed = true;
        self.fight.killed = true;
        self.fight.dirty = true;
        self.save_fight();
    }
    /// `DragonStrafePlayerPhase`'s shot: `new DragonFireball(level, dragon,
    /// direction.normalize())`, whose `AbstractHurtingProjectile` constructor
    /// takes the dragon as its owner and `assignDirectionalMovement`s the
    /// direction normalized again times `accelerationPower` (0.1), then
    /// `snapTo` the start. It joins the level's other projectiles.
    fn spawn_dragon_fireball(&mut self, at: Vec3, dir: Vec3) {
        // Vec3.normalize: a vector shorter than 1e-5 is zero.
        let normalize = |v: Vec3| {
            let l = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
            if l < 1.0e-5 {
                Vec3::new(0.0, 0.0, 0.0)
            } else {
                Vec3::new(v.x / l, v.y / l, v.z / l)
            }
        };
        let n = normalize(normalize(dir));
        self.play_audio(
            "minecraft:entity.ender_dragon.shoot",
            Some([at.x, at.y, at.z]),
            10.0,
            1.0,
        );
        let owner = self.dragon.as_ref().map(|d| Who::Mob(d.id));
        let mut p = sunrise_client::projectiles::Projectile::new(
            sunrise_client::projectiles::ProjectileKind::DragonFireball,
            at,
            owner,
        );
        p.acceleration = 0.1;
        p.velocity = Vec3::new(n.x * 0.1, n.y * 0.1, n.z * 0.1);
        self.add_fresh_projectile(p);
    }
    /// `DragonFireball.onHit` where it burst (`next`): the dragon's breath
    /// cloud and level event 2006, as the fireballs of the dragon's own list
    /// burst before they joined the other projectiles.
    pub(super) fn dragon_fireball_on_hit(&mut self, next: Vec3) {
        let snap_targets: Vec<(Aabb, Vec3)> = self
            .fight_players()
            .iter()
            .filter(|p| p.alive)
            .map(|p| (p.bounds, p.position))
            .chain(
                self.mobs
                    .iter()
                    .filter(|m| m.alive())
                    .map(|m| (m.bounds(), m.position)),
            )
            .collect();
        let id = self.take_entity_id();
        let feet = [next.x, next.y - 0.5, next.z];
        let at = fireball_cloud_origin(feet, &snap_targets);
        self.breath_clouds.push(Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            at,
            motion: [0.0; 3],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 3.0,
            age: 0,
            duration: 600,
            from_dragon: false,
            victims: Vec::new(),
            potion: None,
        });
        self.level_event_at(
            2006,
            [
                next.x.floor() as i32,
                next.y.floor() as i32,
                next.z.floor() as i32,
            ],
            1,
        );
    }
    /// The dragon's breath clouds (`AreaEffectCloud` with instant damage at
    /// a quarter duration scale): shrinking, puffing particles, harming players and mobs.
    /// `DragonSittingFlamingPhase.end`: the flame cloud goes once the
    /// phase is left.
    pub(super) fn end_dragon_flame(&mut self) {
        if self
            .dragon
            .as_mut()
            .is_some_and(|d| std::mem::take(&mut d.flame_ended))
        {
            self.retain_breath_clouds(|c| !c.from_dragon);
        }
    }
    pub(super) fn tick_breath(&mut self) {
        // What instant damage reaches: neither creative nor spectator players.
        let players: Vec<FightPlayer> = self
            .fight_players()
            .into_iter()
            .filter(|p| p.alive && !p.creative && !p.spectator)
            .collect();
        // AreaEffectCloud.serverTick: every living entity, the mobs that
        // instant damage can affect (`canBeAffected`) too; only while a
        // breath cloud is there (lingering potions' clouds tick elsewhere).
        let no_breath = self.breath_clouds.iter().all(|c| c.potion.is_some());
        let mobs: Vec<(u32, Aabb, Vec3)> = if no_breath {
            Vec::new()
        } else {
            let tags = &self.items().entity_type_tags;
            self.mobs
                .iter()
                .filter(|m| {
                    m.alive()
                        && mob_can_be_affected(
                            tags,
                            &m.kind.id,
                            m.kind.flags.entity_tag_slot,
                            "instant_damage",
                        )
                })
                .map(|m| (m.id, m.bounds(), m.position))
                .collect()
        };
        let mut harmed: Vec<(FightWho, f32)> = Vec::new();
        self.end_dragon_flame();
        let mut harmed_mobs: Vec<(u32, i32, [f64; 3], u32)> = Vec::new();
        let ticking = self.entity_ticking_area();
        let through = self.ticked_entities_through;
        let mut clouds = self.breath_clouds.lift();
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
        clouds.retain_mut(|c| {
            // Lingering potions' clouds tick with the potions.
            if c.potion.is_some() {
                return true;
            }
            if c.id > through || !ticking(c.at[0].floor() as i32, c.at[2].floor() as i32) {
                return true;
            }
            self.cloud_fluid_step(c);
            c.age += 1;
            if breath_expired(c.age, c.duration) {
                return self.forget_expired_cloud(c.id);
            }
            if !c.from_dragon && c.age >= BREATH_WAIT {
                // DragonFireball's cloud: radius per tick (7 - 3) / duration.
                c.radius += (7.0 - 3.0) / c.duration as f32;
            }
            let r = c.radius;
            for _ in 0..((std::f32::consts::PI * r * r) as i32).min(40) {
                let a = self.world.random.next_float() * std::f32::consts::TAU;
                let d = self.world.random.next_float().sqrt() * r;
                let at = [
                    c.at[0] + f64::from(a.cos() * d),
                    c.at[1],
                    c.at[2] + f64::from(a.sin() * d),
                ];
                self.particles.add_with(
                    None,
                    &ParticleOptions::simple(ParticleType::DragonBreath),
                    at,
                    [0.0, 0.0, 0.0],
                    false,
                    true,
                );
            }
            // AreaEffectCloud: after its wait, each entity every 20 ticks.
            // Instant damage I from the flame cloud, II from a fireball's,
            // both at the cloud's 0.5 scale.
            let amount = harm_amount(u32::from(!c.from_dragon));
            let inside = players
                .iter()
                .filter(|p| cloud_hits(c.at, r, p.bounds, p.position))
                .map(|p| Victim::Player(p.who))
                .chain(
                    mobs.iter()
                        .filter(|m| cloud_hits(c.at, r, m.1, m.2))
                        .map(|m| Victim::Mob(m.0)),
                );
            for victim in breath_victims(c.age, &mut c.victims, inside) {
                match victim {
                    Victim::Player(who) => harmed.push((who, amount)),
                    Victim::Mob(id) => {
                        harmed_mobs.push((id, i32::from(!c.from_dragon), c.at, c.id))
                    }
                }
            }
            true
        });
        self.breath_clouds
            .restore(clouds, sunrise_client::join_seq::RestoreOrder::SpawnsFirst);
        for (who, amount) in harmed {
            self.as_fight_player(who, |g| {
                g.dragon_hurts_player("minecraft:indirect_magic", amount);
            });
        }
        for (id, amplifier, at, cloud) in harmed_mobs {
            self.breath_affect_mob(id, amplifier, at, Some(cloud));
        }
    }
    /// `EnderDragon.knockBack` on the player in a wing's zone: pushed away
    /// from the body, and hurt for 5 unless the dragon is perched
    /// (`isSitting`) or a mob hurt the player within the last 2 ticks
    /// (`getLastHurtByMobTimestamp() < tickCount - 2`).
    fn dragon_wing_knock_back(&mut self, push: Vec3, sitting: bool) {
        let now = self.world.game_time;
        let hurt_lately = self.player_hurt_by().is_some_and(|(_, at)| at >= now - 2);
        let hurts = !sitting && !hurt_lately && !self.creative;
        if hurts {
            self.note_dragon_attacker("minecraft:mob_attack");
        }
        let before = self.player.as_mut().map(|p| {
            let before = (p.velocity, std::mem::take(&mut p.health.sync_velocity));
            p.velocity = p.velocity.add_vector(push);
            before
        });
        if hurts {
            let origin = self
                .dragon
                .as_ref()
                .map(|d| [d.position.x, d.position.y, d.position.z]);
            let blocking = self.player_blocking(origin, "minecraft:mob_attack", false);
            let attack = self.current_attack();
            let causing = self.dragon.as_ref().map_or(0, |d| d.id as i32);
            if let Some(id) = self.health_resources.damage.id("minecraft:mob_attack")
                && let Ok(outcome) = self
                    .hurt_player_equipment(id, 5.0, blocking, false, causing, false, &attack, None)
            {
                self.combat.forget_noted(outcome.canceled);
                self.after_block(
                    outcome,
                    (self.dragon.as_ref().map(|d| d.id), false),
                    HurtSource {
                        damage_type: "minecraft:mob_attack",
                        causing,
                        direct: causing,
                        position: origin,
                        direct_type: None,
                    },
                    &attack,
                );
            }
        }
        if let Some((velocity, marked)) = before
            && let Some(p) = self.player.as_mut()
        {
            if !p.health.sync_velocity {
                p.velocity = velocity;
            }
            p.health.sync_velocity |= marked;
        }
        self.finish_guest_damage();
        if !sitting
            && !hurt_lately
            && let Some(id) = self.dragon.as_ref().map(|d| d.id)
        {
            self.thorns(id);
        }
    }
    /// (`DamageSources.mobAttack(dragon)`, `indirectMagic(cloud, dragon)`): the
    /// death message names it, or the cloud once the dragon is gone.
    fn dragon_hurts_player(&mut self, damage_type: &str, amount: f32) {
        self.note_dragon_attacker(damage_type);
        self.hurt_player_by(damage_type, amount);
        // EnderDragon.hurt(List) runs effects even when hurtServer refused.
        // The breath cloud's indirect magic never runs them.
        if damage_type == "minecraft:mob_attack"
            && let Some(id) = self.dragon.as_ref().map(|d| d.id)
        {
            self.thorns(id);
        }
    }
    /// The causing entity of the dragon's next `damage_type` damage, for the
    /// death message.
    fn note_dragon_attacker(&mut self, damage_type: &str) {
        if let Some(id) = self.dragon.as_ref().map(|d| d.id) {
            let name = Text::translate("entity.minecraft.ender_dragon", &[]);
            self.note_named_attacker(damage_type, id, name);
        } else if damage_type == "minecraft:indirect_magic" {
            let name = Text::translate("entity.minecraft.area_effect_cloud", &[]);
            self.note_direct_entity(damage_type, name);
        }
    }
    /// `BossHealthOverlay.extractRenderState`: whatever `publish_boss_bars`
    /// (or a server's packets) put on this client's overlay.
    pub(super) fn draw_boss_bars(&self, batch: &mut HudBatch) {
        let draws = self.boss_bar_draws(batch.width as i32, batch.height as i32);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    /// The boss bars' draws on a `w` x `h` GUI.
    pub(super) fn boss_bar_draws(&self, w: i32, h: i32) -> Vec<hud_layout::GuiDraw> {
        let now = sunrise_client::boss_overlay::now_millis();
        let bars = self.boss_overlay.bars(now);
        // NeoForge's `CustomizeGuiOverlayEvent.BossEventProgress`, asked per
        // bar only while a mod listens.
        let customized = mod_api::__private::listened(mod_api::HookId::CustomizeBossEventProgress);
        hud_overlays::boss_overlay(w, h, &bars, &self.gui.atlas.font, |index, at| {
            let Some(bar) = customized
                .then(|| self.boss_overlay.mod_bar(index, now))
                .flatten()
            else {
                return true;
            };
            let Ok(bar) = bar.try_into_api() else {
                return true;
            };
            let mut cancel = false;
            self.native_customize_boss_event_progress(
                &bar,
                &mut at.x,
                &mut at.y,
                &mut at.increment,
                &mut cancel,
            );
            !cancel
        })
    }
    /// The dragon fight's bar for a player at `at`
    /// (`EnderDragonFight.dragonEvent`, visible while the fight is not won
    /// and the player is alive within 192 blocks of (0, 128, 0)).
    pub(super) fn dragon_boss_progress(&self, at: [f64; 3], alive: bool) -> Option<f32> {
        if !self.dimension_type().has_ender_dragon_fight
            || !alive
            || !self.fight.loaded
            || self.fight.killed
        {
            return None;
        }
        let near = at[0].powi(2) + (at[1] - 128.0).powi(2) + at[2].powi(2) <= 192.0 * 192.0;
        if !near {
            return None;
        }
        Some(
            self.dragon
                .as_ref()
                .map_or(1.0, |d| (d.health / MAX_HEALTH).clamp(0.0, 1.0)),
        )
    }
    /// `BossHealthOverlay.shouldPlayMusic`.
    pub(super) fn boss_music(&self) -> bool {
        self.boss_overlay.should_play_music()
    }
}
/// `DragonLandingPhase.doClientTick` and `DragonSittingFlamingPhase.doClientTick`:
/// the dragon-breath particles one tick of `doClientTick` spawns at the head, as
/// (position, velocity) pairs. `look` is `EnderDragon.getHeadLookVector(1)`
/// normalised, `head` the head part's centre (`getY(0.5)`), `motion` the
/// dragon's `getDeltaMovement`, and `random` the stream the jitter comes from.
/// Both phases rotate `look` by a quarter turn and throw the result away, so
/// every puff of a fan leaves along the same line; a flaming dragon walks down
/// that line in six steps.
fn breath_puffs(
    phase: &Phase,
    look: [f64; 3],
    head: [f64; 3],
    motion: [f64; 3],
    random: &mut LegacyRandom,
) -> Vec<([f64; 3], [f64; 3])> {
    // A puff's start: the head's centre jittered by `nextGaussian() / 2`.
    let jitter = |random: &mut LegacyRandom| {
        [
            head[0] + random.next_gaussian() / 2.0,
            head[1] + random.next_gaussian() / 2.0,
            head[2] + random.next_gaussian() / 2.0,
        ]
    };
    match phase {
        // Eight a tick, along `-look * 0.08F` (0.3 for the vertical part) plus
        // the dragon's own motion.
        Phase::Landing { .. } => (0..8)
            .map(|_| {
                (
                    jitter(random),
                    [
                        -look[0] * f64::from(0.08_f32) + motion[0],
                        -look[1] * f64::from(0.3_f32) + motion[1],
                        -look[2] * f64::from(0.08_f32) + motion[2],
                    ],
                )
            })
            .collect(),
        // Every second tick of the first ten: eight starts, each a fan of six
        // puffs down the look line, the vertical part of the speed once.
        Phase::Flaming { ticks } if ticks % 2 == 0 && *ticks < 10 => {
            let mut puffs = Vec::with_capacity(48);
            for _ in 0..8 {
                let at = jitter(random);
                for j in 0..6 {
                    let j = f64::from(j);
                    puffs.push((
                        at,
                        [
                            -look[0] * f64::from(0.08_f32) * j,
                            -look[1] * f64::from(0.6_f32),
                            -look[2] * f64::from(0.08_f32) * j,
                        ],
                    ));
                }
            }
            puffs
        }
        _ => Vec::new(),
    }
}
pub(super) struct Breath {
    /// Entity-owned registration metadata; content clones are detached.
    pub(super) attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub(in crate::game) join: sunrise_client::join_seq::JoinSeq,
    /// Its entity id once a server's players track it (0 until then).
    pub(super) id: u32,
    pub(super) at: [f64; 3],
    pub(super) motion: [f64; 3],
    /// `Entity.fallDistance`: as loaded, until water resets or lava halves it.
    pub(super) fall_distance: f64,
    /// `Entity.remainingFireTicks`: a cloud is fire immune, so its first tick
    /// clears a burning one to 0.
    pub(super) fire_ticks: i32,
    /// `Entity.firstTick`: true until the first base tick finishes, gating `isInLava()`.
    pub(super) first_tick: bool,
    pub(super) radius: f32,
    pub(super) age: i32,
    pub(super) duration: i32,
    pub(super) from_dragon: bool,
    /// Who it hit and until which age (`AreaEffectCloud.victims`).
    pub(super) victims: Vec<(Victim, i32)>,
    /// A lingering potion's cloud (`ThrownLingeringPotion`): what it applies.
    pub(super) potion: Option<super::potions::PotionCloud>,
}

sunrise_client::impl_joined!(Breath, Dragon);
sunrise_mod_runtime::impl_attachment_lifetime!(Breath);

impl Breath {
    /// `AreaEffectCloud.isWaiting`: before its wait time (20 ticks, 10 for a
    /// lingering potion's).
    pub(super) fn waiting(&self) -> bool {
        self.age < self.potion.as_ref().map_or(20, |p| p.settings.wait_time)
    }
}
#[cfg(test)]
impl Dragon {
    pub(super) fn set_render_test_crystal(&mut self, crystal: Option<usize>) {
        self.nearest_crystal = crystal;
    }
}

/// Detached render-only state for GUI dragon previews (`DrawContext::entity`),
/// holding only the fields the renderer reads so by-type draws never allocate
/// `DragonPart`s or entity seeds.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct DragonPreview {
    flap: f32,
    sitting: bool,
    hurt_time: i32,
}

impl DragonPreview {
    pub(in crate::game) fn from_dragon(d: &Dragon) -> Self {
        Self {
            flap: d.flap,
            sitting: d.phase.sitting(),
            hurt_time: d.hurt_time,
        }
    }

    /// Build a detached preview from saved dragon NBT, discarding position and orientation.
    pub(in crate::game) fn from_nbt(data: &sunrise_core::nbt::Compound) -> Self {
        let phase =
            saved_dragon_phase(data.get(&sunrise_core::nbt::JavaString::from("DragonPhase")))
                .unwrap_or(Phase::Hover { target: None });
        Self {
            flap: 0.0,
            sitting: phase.sitting(),
            hurt_time: 0,
        }
    }
}

/// Screen orientation for `gui_dragon_geometry`: body yaw in `EnderDragonRenderer`'s
/// frame, head yaw relative to the body, and head pitch (degrees).
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::game) struct GuiDragonLook {
    pub body_yaw: f32,
    pub head_yaw: f32,
    pub head_pitch: f32,
}

fn saved_dragon_phase(tag: Option<&Tag>) -> Option<Phase> {
    nbt_value::number(tag).map(|p| Phase::from_id(p as i32))
}

impl Game<'_> {
    /// `EnderDragonRenderer.submit` and `EnderDragonModel.setupAnim`, from the real dragon's slot.
    pub(super) fn dragon_geometry(
        &self,
        d: &Dragon,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        additive: &mut Vec<Vertex>,
    ) {
        self.dragon_geometry_as::<false>(
            Some(d),
            DragonPreview::from_dragon(d),
            camera,
            alpha,
            GuiDragonLook::default(),
            out,
            additive,
        );
    }

    /// The dragon as a screen draws it (`DrawContext::entity`): posed still with `look`,
    /// full bright, its body in `out` and additive eyes in `eyes`, and none of the world's
    /// marks, death rays or crystal beam.
    #[inline(always)]
    pub(in crate::game) fn gui_dragon_geometry(
        &self,
        preview: DragonPreview,
        look: GuiDragonLook,
        out: &mut Vec<Vertex>,
        eyes: &mut Vec<Vertex>,
    ) {
        self.dragon_geometry_as::<true>(None, preview, glam::DVec3::ZERO, 1.0, look, out, eyes);
    }

    /// `dragon_geometry` specialised by `GUI` so world draws drop GUI branches and add no slot.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn dragon_geometry_as<const GUI: bool>(
        &self,
        world: Option<&Dragon>,
        preview: DragonPreview,
        camera: glam::DVec3,
        alpha: f64,
        gui: GuiDragonLook,
        out: &mut Vec<Vertex>,
        additive: &mut Vec<Vertex>,
    ) {
        let Some(model) = self.dragon_model.as_ref() else {
            return;
        };
        let a = alpha as f32;
        let sprite = |n: &str| {
            self.gui
                .atlas
                .sprites
                .get(n)
                .map(|r| [r.u0, r.v0, r.u1, r.v1])
        };
        let (Some(skin), Some(eyes)) = (
            sprite("entity/enderdragon/dragon"),
            sprite("entity/enderdragon/dragon_eyes"),
        ) else {
            return;
        };
        let pos = match world {
            Some(d) if !GUI => {
                let p = d.previous.lerp(d.position, alpha);
                glam::DVec3::new(p.x, p.y, p.z)
            }
            _ => glam::DVec3::ZERO,
        };
        let h = |delay: usize| match world {
            Some(d) if !GUI => d.sample_lerp(delay, a),
            _ => (0.0, gui.body_yaw),
        };
        let yr = h(7).1;
        let rot2 = (h(5).0 - h(10).0) as f32;
        let mut base = glam::Mat4::from_translation((pos - camera).as_vec3())
            * glam::Mat4::from_rotation_y((-yr).to_radians())
            * glam::Mat4::from_rotation_x((rot2 * 10.0).to_radians())
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 1.0))
            * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0));
        let rays_base = base * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.0, -2.0));
        // setupAnim.
        let flap = match world {
            Some(d) if !GUI => (d.old_flap + (d.flap - d.old_flap) * a) * std::f32::consts::TAU,
            _ => preview.flap * std::f32::consts::TAU,
        };
        let mut bounce = (flap - 1.0).sin() + 1.0;
        bounce = (bounce * bounce + bounce * 2.0) * 0.05;
        base *=
            glam::Mat4::from_translation(glam::Vec3::new(0.0, (bounce - 2.0) * 16.0, -48.0) / 16.0)
                * glam::Mat4::from_rotation_x((bounce * 2.0).to_radians());
        let mut poses = sunrise_client::mobs::anim::Poses::new(model);
        poses.get("jaw")[3] = (flap.sin() + 1.0) * 0.2;
        let sitting = preview.sitting;
        let landing = match world {
            Some(d) if !GUI => matches!(d.phase, Phase::Landing { .. } | Phase::Takeoff { .. }),
            _ => false,
        };
        let egg_dist = match world {
            Some(d) if !GUI => {
                let p = self.podium_top();
                ((p.x - d.position.x).powi(2)
                    + (p.y - d.position.y).powi(2)
                    + (p.z - d.position.z).powi(2))
                .sqrt()
            }
            _ => 0.0,
        };
        let head_offset = |part: usize, start: (f64, f32), point: (f64, f32)| -> f32 {
            if landing {
                part as f32 / (egg_dist / 4.0).max(1.0) as f32
            } else if sitting {
                part as f32
            } else if part == 6 {
                0.0
            } else {
                (point.0 - start.0) as f32
            }
        };
        let start = h(6);
        let rot2y = wrap(h(5).1 - h(10).1);
        let rot = wrap(h(5).1 + rot2y / 2.0);
        let deg = std::f32::consts::PI / 180.0;
        let (mut xx, mut yy, mut zz) = (0.0f32, 20.0f32, -12.0f32);
        for i in 0..5 {
            let point = h(5 - i);
            let pose = poses.get(&format!("neck{i}"));
            let neck_x = (i as f32 * 0.45 + flap).cos() * 0.15;
            pose[4] = wrap(point.1 - start.1) * deg * 1.5;
            pose[3] = neck_x + head_offset(i, start, point) * deg * 1.5 * 5.0;
            pose[5] = -wrap(point.1 - rot) * deg * 1.5;
            pose[0] = xx;
            pose[1] = yy;
            pose[2] = zz;
            let (xr, yrr) = (pose[3], pose[4]);
            xx -= yrr.sin() * xr.cos() * 10.0;
            yy += xr.sin() * 10.0;
            zz -= yrr.cos() * xr.cos() * 10.0;
        }
        let head = poses.get("head");
        head[0] = xx;
        head[1] = yy;
        head[2] = zz;
        if GUI {
            head[4] = wrap(gui.head_yaw) * deg;
            head[3] = gui.head_pitch * deg;
            head[5] = 0.0;
        } else {
            let current = h(0);
            head[4] = wrap(current.1 - start.1) * deg;
            head[3] = wrap(head_offset(6, start, current)) * deg * 1.5 * 5.0;
            head[5] = -wrap(current.1 - rot) * deg;
        }
        poses.get("body")[5] = -rot2y * 1.5 * deg;
        let lw = [0.125 - flap.cos() * 0.2, -0.25, -(flap.sin() + 0.125) * 0.8];
        let lt = ((flap + 2.0).sin() + 0.5) * 0.75;
        {
            let p = poses.get("left_wing");
            (p[3], p[4], p[5]) = (lw[0], lw[1], lw[2]);
        }
        poses.get("left_wing_tip")[5] = lt;
        {
            let p = poses.get("right_wing");
            (p[3], p[4], p[5]) = (lw[0], -lw[1], -lw[2]);
        }
        poses.get("right_wing_tip")[5] = -lt;
        for side in ["left", "right"] {
            poses.get(&format!("{side}_hind_leg"))[3] = 1.0 + bounce * 0.1;
            poses.get(&format!("{side}_hind_leg_tip"))[3] = 0.5 + bounce * 0.1;
            poses.get(&format!("{side}_hind_foot"))[3] = 0.75 + bounce * 0.1;
            poses.get(&format!("{side}_front_leg"))[3] = 1.3 + bounce * 0.1;
            poses.get(&format!("{side}_front_leg_tip"))[3] = -0.5 - bounce * 0.1;
            poses.get(&format!("{side}_front_foot"))[3] = 0.75 + bounce * 0.1;
        }
        let mut tail_x = 0.0f32;
        let (mut xx, mut yy, mut zz) = (0.0f32, 10.0f32, 60.0f32);
        let start = h(11);
        for i in 0..12 {
            let point = h(12 + i);
            tail_x += (i as f32 * 0.45 + flap).sin() * 0.05;
            let p = poses.get(&format!("tail{i}"));
            p[4] = (wrap(point.1 - start.1) * 1.5 + 180.0) * deg;
            p[3] = tail_x + (point.0 - start.0) as f32 * deg * 1.5 * 5.0;
            p[5] = wrap(point.1 - rot) * deg * 1.5;
            p[0] = xx;
            p[1] = yy;
            p[2] = zz;
            let (xr, yrr) = (p[3], p[4]);
            yy += xr.sin() * 10.0;
            zz -= yrr.cos() * xr.cos() * 10.0;
            xx -= yrr.sin() * xr.cos() * 10.0;
        }
        let light = match world {
            Some(d) if !GUI => {
                let l = self.world.light(
                    d.position.x.floor() as i32,
                    d.position.y.floor() as i32,
                    d.position.z.floor() as i32,
                );
                (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8)
            }
            // `renderEntityInInventory`: LightTexture.FULL_BRIGHT.
            _ => sunrise_client::mobs::render::FULL_BRIGHT,
        };
        let death = match world {
            Some(d) if !GUI && d.death_time > 0 => d.death_time as f32 + a,
            _ => 0.0,
        };
        // Dying: DYING_RENDER_TYPE, `entityCutoutDissolve` of the skin with dragon_exploding as
        // the mask, white at `ARGB.white(1 - deathTime / 200)` and no overlay.
        let mask = sprite("entity/enderdragon/dragon_exploding").filter(|_| death > 0.0);
        let (color, overlay) = if mask.is_some() {
            (dissolve_color(death), sunrise_client::gfx::DISSOLVE_OVERLAY)
        } else {
            (0xFFFF_FFFF, u32::from(preview.hurt_time > 0))
        };
        let first = out.len();
        if !GUI {
            self.mark_entity(first, "minecraft:ender_dragon");
            self.mark_motion(first, 0);
        }
        sunrise_client::mobs::render::draw(model, base, &poses, skin, color, light, overlay, out);
        if let Some(mask) = mask {
            dissolve_mask(&mut out[first..], skin, mask);
        }
        // The skin's OptiFine emissive overlay, full bright.
        let glow = self
            .res
            .mob_data()
            .emissive
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .overlay("entity/enderdragon/dragon")
            .and_then(sprite);
        if let Some(glow) = glow.filter(|_| mask.is_none()) {
            let full = sunrise_client::mobs::render::FULL_BRIGHT;
            sunrise_client::mobs::render::draw(model, base, &poses, glow, color, full, 0, out);
        }
        #[cfg(test)]
        if let Some(d) = world
            && !GUI
        {
            super::native_render_pair_tests::witness_body(d.id, first, out.len());
        }
        // RenderTypes.eyes: the glowing eyes, added on top.
        let first_eye = additive.len();
        sunrise_client::mobs::render::draw(
            model,
            base,
            &poses,
            eyes,
            0xFFFF_FFFF,
            240 | 240 << 8,
            0,
            additive,
        );
        // `RenderTypes.eyes` has an `OitPipelineSet`: improved transparency
        // moves the tagged quads into its stages (the HDR tier is the eyes'
        // shine, `mobs::render::EYES_TIER`).
        if GUI {
            return;
        }
        for v in &mut additive[first_eye..] {
            v.layer = 2
                | sunrise_client::gfx::OIT_ENTITY
                | u32::from(sunrise_client::mesher::BRIGHT) << 16;
        }
        if death > 0.0 {
            let rays = additive.len();
            self.dragon_rays(rays_base, death / 200.0, additive);
            for v in &mut additive[rays..] {
                v.layer |= u32::from(sunrise_client::mesher::VERY_BRIGHT) << 16;
            }
        }
        // The healing beam to the nearest crystal.
        if let Some(d) = world
            && let Some(c) = d.nearest_crystal.and_then(|i| self.crystals.get(i))
        {
            let bob = f64::from(sunrise_client::end_crystal::y(c.time as f32 + a));
            let to = glam::DVec3::new(c.position.x, c.position.y + bob, c.position.z);
            self.crystal_beam(pos, to - pos, self.visual_ticks() as f32 + a, camera, out);
        }
    }
    /// `EnderDragonRenderer.submitRays`: untextured, additive death rays.
    fn dragon_rays(&self, base: glam::Mat4, t: f32, out: &mut Vec<Vertex>) {
        dragon_rays(base, t, self.gui.atlas.font.white(), out);
    }
    /// `submitCrystalBeams`: an eight-sided tube from `from` (+2 up) along
    /// `delta`, the beam texture scrolling along it.
    pub(super) fn crystal_beam(
        &self,
        from: glam::DVec3,
        delta: glam::DVec3,
        time: f32,
        camera: glam::DVec3,
        out: &mut Vec<Vertex>,
    ) {
        let Some(s) = self
            .gui
            .atlas
            .sprites
            .get("entity/end_crystal/end_crystal_beam")
            .copied()
        else {
            return;
        };
        // Like the other layer-6 beams, this is a separate pack material from its body.
        self.mark_stage(out.len(), "NONE");
        let q = |v: f32| (v * 65535.0).round().clamp(0.0, 65535.0) as u32;
        let wrap_light = q(s.v0) | q(s.v1 - s.v0) << 16;
        let (dx, dy, dz) = (delta.x as f32, delta.y as f32, delta.z as f32);
        let horizontal = (dx * dx + dz * dz).sqrt();
        let length = (dx * dx + dy * dy + dz * dz).sqrt();
        let m = glam::Mat4::from_translation(
            (from - camera).as_vec3() + glam::Vec3::new(0.0, 2.0, 0.0),
        ) * glam::Mat4::from_rotation_y(-dz.atan2(dx) - std::f32::consts::FRAC_PI_2)
            * glam::Mat4::from_rotation_x(-horizontal.atan2(dy) - std::f32::consts::FRAC_PI_2);
        let (v0, v1) = (-time * 0.01, length / 32.0 - time * 0.01);
        let (mut last_sin, mut last_cos, mut last_u) = (0.0f32, 0.75f32, 0.0f32);
        let u = |x: f32| s.u0 + (s.u1 - s.u0) * x;
        for i in 1..=8 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let (sin, cos) = (a.sin() * 0.75, a.cos() * 0.75);
            let uu = i as f32 / 8.0;
            for (p, uv, c) in [
                (
                    glam::Vec3::new(last_sin * 0.2, last_cos * 0.2, 0.0),
                    [u(last_u), v0],
                    0xFF00_0000,
                ),
                (
                    glam::Vec3::new(last_sin, last_cos, length),
                    [u(last_u), v1],
                    0xFFFF_FFFF,
                ),
                (glam::Vec3::new(sin, cos, length), [u(uu), v1], 0xFFFF_FFFF),
                (
                    glam::Vec3::new(sin * 0.2, cos * 0.2, 0.0),
                    [u(uu), v0],
                    0xFF00_0000,
                ),
            ] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(p).to_array(),
                    uv,
                    // HDR output: the healing beam shines (entity.wgsl fs_hdr_mark).
                    layer: 6 | u32::from(sunrise_client::mesher::BRIGHT) << 16,
                    color: c,
                    light: wrap_light,

                    glint_uv: [0.0, 0.0],
                });
            }
            (last_sin, last_cos, last_u) = (sin, cos, uu);
        }
    }
    /// Dragon fireballs as the dragon_fireball sprite facing the camera.
    pub(super) fn dragon_fireball_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let Some(s) = self
            .gui
            .atlas
            .sprites
            .get("entity/enderdragon/dragon_fireball")
            .copied()
        else {
            return;
        };
        // DragonFireballRenderer.submit: turned by the camera's orientation.
        let [right, up] = super::billboard_axes(self.camera_orientation());
        for b in self.creatures.projectiles.iter().filter(|p| {
            p.kind == sunrise_client::projectiles::ProjectileKind::DragonFireball
                && !self.hides_as_camera(p.id)
        }) {
            let p = glam::DVec3::new(
                b.previous.x + (b.position.x - b.previous.x) * alpha,
                b.previous.y + (b.position.y - b.previous.y) * alpha,
                b.previous.z + (b.position.z - b.previous.z) * alpha,
            ) - camera;
            // DragonFireballRenderer: scale 2, full bright.
            let h = 1.0;
            for (c, uv) in [
                (p - right * h - up * h, [s.u0, s.v1]),
                (p + right * h - up * h, [s.u1, s.v1]),
                (p + right * h + up * h, [s.u1, s.v0]),
                (p - right * h + up * h, [s.u0, s.v0]),
            ] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: c.as_vec3().to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: 240 | 240 << 8,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }
    pub(super) fn fight_ready(&mut self) -> bool {
        self.load_fight();
        // spawnExitPortal runs from scanState and the dragon's death, both
        // only with players in the fight.
        self.fight.loaded && self.fight.players
    }
    pub(super) fn exit_portal(&self) -> Option<[i32; 3]> {
        self.fight.exit_portal
    }
    pub(super) fn set_exit_portal(&mut self, origin: [i32; 3]) {
        if self.fight.exit_portal.is_none() {
            self.fight.exit_portal = Some(origin);
            self.fight.dirty = true;
        }
    }
}
/// `Entity.isSilent` from an entity's saved compound: `Silent`, written only
/// when set.
fn saved_silent(raw: Option<&Tag>) -> bool {
    matches!(raw.and_then(|t| t.get("Silent")), Some(Tag::Byte(b)) if *b != 0)
}
/// Saving and loading the dragon and the crystals as chunk entities.
impl Dragon {
    /// `Entity.firstTick`: no tick has run yet.
    pub(super) fn first_tick(&self) -> bool {
        self.ticks == 0
    }
    pub(super) fn to_nbt(
        &self,
        uuid: [i32; 4],
        raw: Option<&Tag>,
        items: &sunrise_client::items::Items,
    ) -> Tag {
        let mut tag = sunrise_client::entity_nbt::vehicle_to_nbt(
            "minecraft:ender_dragon",
            self.position,
            self.velocity,
            [self.yaw, self.pitch],
            (false, 0.0),
            uuid,
            raw,
        );
        if let Tag::Compound(map) = &mut tag {
            map.insert("Health".into(), Tag::Float(self.health.to_bits()));
            map.insert("DragonPhase".into(), Tag::Int(self.phase.id()));
            map.insert("DragonDeathTime".into(), Tag::Int(self.death_time));
            if self.persistent {
                map.insert("PersistenceRequired".into(), Tag::Byte(1));
            }
            // Mob.addAdditionalSaveData writes NoAI only when set.
            if self.no_ai {
                map.insert("NoAI".into(), Tag::Byte(1));
            }
            map.insert(
                "sitting_damage_received".into(),
                Tag::Float(self.sitting_damage.to_bits()),
            );
            self.equipment.store(map, items);
            sunrise_client::entity_nbt::put_live_attributes(map, &self.live);
        }
        tag
    }
    pub(super) fn from_nbt(tag: &Tag, seed: i64) -> Option<Self> {
        let (position, velocity, _) = sunrise_client::entity_nbt::boat_from_nbt(tag)?;
        let [yaw, pitch] = sunrise_client::entity_nbt::loaded_rotation(tag);
        let mut d = Self::new(position, yaw, seed);
        d.pitch = pitch;
        d.velocity = velocity;
        d.apply_saved_fields(tag);
        Some(d)
    }
    fn apply_saved_fields(&mut self, tag: &Tag) {
        // Vanilla getFloatOr/getIntOr accept any numeric tag type.
        self.health = nbt_value::float(tag.get("Health")).map_or(self.health, |h| h as f32);
        self.sitting_damage =
            nbt_value::float(tag.get("sitting_damage_received")).unwrap_or(0.0) as f32;
        if let Some(p) = saved_dragon_phase(tag.get("DragonPhase")) {
            self.phase = p;
        }
        if let Some(t) = nbt_value::number(tag.get("DragonDeathTime")) {
            self.death_time = t as i32;
        }
        self.persistent = sunrise_client::entity_nbt::flag(tag, "PersistenceRequired");
        if let Some(equipment) = tag.get("equipment") {
            self.equipment = sunrise_items::equipment::Equipment::read(equipment);
        }
        self.live.load(tag.get("attributes"), |_| true);
        self.no_ai = matches!(tag.get("NoAI"), Some(Tag::Byte(b)) if *b != 0);
    }
}
impl Crystal {
    pub(super) fn to_nbt(&self, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
        let mut tag = sunrise_client::entity_nbt::vehicle_to_nbt(
            "minecraft:end_crystal",
            self.position,
            Vec3::ZERO,
            self.rotation,
            self.fall,
            uuid,
            raw,
        );
        if let Tag::Compound(map) = &mut tag {
            map.insert("Fire".into(), Tag::Short(self.fire as i16));
            map.insert("ShowBottom".into(), Tag::Byte(i8::from(self.show_bottom)));
            if let Some(b) = self.beam {
                map.insert("beam_target".into(), Tag::IntArray(b.to_vec()));
            } else {
                map.remove(&"beam_target".into());
            }
            map.insert(
                "Invulnerable".into(),
                Tag::Byte(i8::from(self.invulnerable)),
            );
        }
        tag
    }
    pub(super) fn from_nbt(tag: &Tag, time: i32) -> Option<Self> {
        let (position, _, _) = sunrise_client::entity_nbt::boat_from_nbt(tag)?;
        // TagValueInput.getShortOr uses NumericTag.shortValue.
        let fire = match tag.get("Fire") {
            Some(Tag::Float(bits)) => sunrise_core::mth::floor_f32(f32::from_bits(*bits)) as i16,
            Some(Tag::Double(bits)) => sunrise_core::mth::floor(f64::from_bits(*bits)) as i16,
            value => nbt_value::number(value).unwrap_or_default() as i16,
        };
        Some(Self {
            size_answer: None,
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 0,
            position,
            previous: position,
            rotation: sunrise_client::entity_nbt::loaded_rotation(tag),
            fire: i32::from(fire),
            show_bottom: sunrise_client::entity_nbt::flag_or(tag, "ShowBottom", true),
            time,
            beam: match tag.get("beam_target") {
                Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
                _ => None,
            },
            invulnerable: sunrise_client::entity_nbt::flag(tag, "Invulnerable"),
            fall: sunrise_client::entity_nbt::ground_from_nbt(tag),
        })
    }
}
impl Game<'_> {
    /// `FireworkRocketEntity.dealExplosionDamage` on the dragon: a living
    /// entity in the rocket's `inflate(5)` box and within 5 blocks, seen at its
    /// feet or middle, takes `EnderDragon.hurtServer` (`hurt(body, ...)`).
    pub(super) fn rocket_blast_dragon(
        &mut self,
        from: Vec3,
        damage: f32,
        reach: Aabb,
        by: Option<FightWho>,
        rocket_id: u32,
        owner: Option<Who>,
    ) {
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let p = d.position;
        let d2 = (p.x - from.x).powi(2) + (p.y - from.y).powi(2) + (p.z - from.z).powi(2);
        if d2 > 25.0 || !d.whole_box().intersects(reach) {
            return;
        }
        let world = self.mob_world_base(&[]);
        let sees = (0..2).any(|step| {
            let to = Vec3::new(p.x, p.y + 8.0 * 0.5 * f64::from(step), p.z);
            world.clear_between(from, to, false)
        });
        if !sees {
            return;
        }
        let amount = super::fireworks::blast_damage(damage, from, p);
        let hooks = DragonHitHooks::capture();
        let hook_source = hooks.any.then(|| {
            let causing = match owner {
                Some(Who::Player) => by.map_or(0, |who| self.fight_entity(who)),
                Some(Who::Mob(id)) => id as i32,
                None => 0,
            };
            HurtSource {
                damage_type: "minecraft:fireworks",
                causing,
                direct: rocket_id as i32,
                position: Some([from.x, from.y, from.z]),
                direct_type: Some("minecraft:firework_rocket"),
            }
        });
        self.hurt_dragon_part_at(
            BODY,
            amount,
            true,
            by,
            DamageKind::Fireworks,
            DragonHitOrigin {
                direct: None,
                entity_id: None,
                from: None,
                invulnerability_checked: false,
                hook_source,
                attack: None,
                hooks,
            },
        );
    }
    /// `ServerExplosion.hurtEntities`: crystals in reach are removed without another
    /// blast; explosions always hurt dragons, except one `explosion_detonate` spared.
    pub(super) fn explosion_hits_special(
        &mut self,
        center: [f64; 3],
        radius: f32,
        blocks: &[[i32; 3]],
        damage_type: &str,
        blast_direct: Option<(u32, Option<&'static str>)>,
    ) {
        let diameter = f64::from(radius * 2.0);
        let in_reach = |p: Vec3| {
            ((p.x - center[0]).powi(2) + (p.y - center[1]).powi(2) + (p.z - center[2]).powi(2))
                .sqrt()
                / diameter
                <= 1.0
        };
        let (direct, direct_type) = match blast_direct {
            Some((id, kind)) => (
                id as i32,
                kind.or_else(|| {
                    self.combat
                        .explosion_direct
                        .filter(|&(d, _)| d == id)
                        .map(|(_, ty)| ty)
                }),
            ),
            None => match self.combat.explosion_direct {
                Some((id, ty)) => (id as i32, Some(ty)),
                None => (self.combat.explosion_source.map_or(0, |id| id as i32), None),
            },
        };
        let crystals: Vec<Vec3> = self
            .crystals
            .iter()
            .filter(|c| in_reach(c.position) && self.combat.visits(c.id))
            .map(|c| c.position)
            .collect();
        // The blast's source carries the player who set it off, if any.
        let by = self.fight.blast_by;
        for at in crystals {
            // Look up the current index after each removal.
            if let Some(i) = self.crystals.iter().position(|c| c.position == at) {
                let id = self.crystals[i].id;
                let causing = self.blast_attacker().map_or(0, |(id, _)| id as i32);
                let vanilla =
                    self.crystals[i].invulnerable && !self.creative_damage_source_id(causing);
                let source = HurtSource {
                    damage_type,
                    causing,
                    direct,
                    position: Some(center),
                    direct_type,
                };
                if self.native_nonliving_invulnerable(id, "minecraft:end_crystal", source, vanilla)
                {
                    continue;
                }
                self.crystals[i].invulnerable = false;
                self.destroy_crystal(i, by, CrystalDamage::Explosion);
            }
        }
        let Some(d) = self.dragon.as_ref().filter(|d| self.combat.visits(d.id)) else {
            return;
        };
        let hits = d.explosion_hits(center, radius, |b| {
            let (lo, hi) = (b.min_position(), b.max_position());
            self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
        });
        let whole = d.whole_box();
        let (lo, hi) = (whole.min_position(), whole.max_position());
        let exposure = self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
        let hooks = DragonHitHooks::capture();
        let blast_source = hooks.any.then(|| HurtSource {
            damage_type,
            causing: self.blast_attacker().map_or(0, |(id, _)| id as i32),
            direct,
            position: Some(center),
            direct_type,
        });
        for (part, amount) in hits {
            self.hurt_dragon_part_at(
                part,
                amount,
                true,
                by,
                DamageKind::Explosion,
                DragonHitOrigin {
                    direct: None,
                    entity_id: None,
                    from: None,
                    invulnerability_checked: false,
                    hook_source: blast_source,
                    attack: None,
                    hooks,
                },
            );
        }
        let Some((id, mut push)) = self
            .dragon
            .as_ref()
            .and_then(|d| Some((d.id, d.explosion_knockback(center, radius, exposure)?)))
        else {
            return;
        };
        self.native_explosion_knockback(self.dim.id(), center, blocks, Some(id), &mut push);
        if let Some(d) = self.dragon.as_mut() {
            d.velocity = Vec3::new(
                d.velocity.x + push[0],
                d.velocity.y + push[1],
                d.velocity.z + push[2],
            );
        }
    }
}
impl Game<'_> {
    /// The dragon as guests track it (`EnderDragon.getAddEntityPacket` and
    /// the synched data that differs from the defaults: its health, defined
    /// as 1, and `DATA_PHASE`, defined as hovering). Vanilla pairs it with no
    /// attribute instances.
    pub(super) fn lan_dragon_state(&self) -> Option<sunrise_client::lan::entity::EntityState> {
        use sunrise_protocol::entity_data::{Value, serializer};
        let d = self.dragon.as_ref().filter(|d| d.id != 0)?;
        let mut meta = Vec::new();
        let health = d.health.max(0.0);
        if health != 1.0 {
            meta.push((9, serializer::FLOAT, Value::Float(health)));
        }
        // Mob.DATA_MOB_FLAGS_ID: aggressive is 4.
        if d.aggressive {
            meta.push((15, serializer::BYTE, Value::Byte(4)));
        }
        let phase = d.phase.id();
        if phase != 10 {
            meta.push((16, serializer::INT, Value::Int(phase)));
        }
        // Mob.DATA_MOB_FLAGS_ID: NoAI is its first bit.
        if d.no_ai {
            meta.push((15, serializer::BYTE, Value::Byte(1)));
        }
        Some(sunrise_client::lan::entity::EntityState {
            id: d.id as i32,
            uuid: super::lan::entity_uuid(d.id),
            kind: "minecraft:ender_dragon".to_owned(),
            position: [d.position.x, d.position.y, d.position.z],
            yaw: d.yaw,
            pitch: 0.0,
            head_yaw: d.yaw,
            on_ground: false,
            velocity: [d.velocity.x, d.velocity.y, d.velocity.z],
            minecart_steps: None,
            data: 0,
            meta,
            display: None,
            forced_meta: [0; 4],
            // Back at their defaults the values leave `meta`, and the clients
            // still hold the old ones.
            defaults: vec![
                (9, serializer::FLOAT, Value::Float(1.0)),
                (15, serializer::BYTE, Value::Byte(0)),
                (16, serializer::INT, Value::Int(10)),
            ],
            equipment: Vec::new(),
            attributes: Vec::new(),
            precise: false,
            vertical_collision: false,
            horizontal_collision: false,
            needs_sync: false,
            sync_position: false,
            sync_velocity: false,
            stepped: true,
            idle: false,
            bare: false,
            leash: None,
            spectator: false,
            watching: false,
            hidden: None,
            general: None,
        })
    }
}
impl Game<'_> {
    /// The dragon and fight for `sunrise-ctl status`.
    pub(super) fn dragon_status(&self) -> serde_json::Value {
        serde_json::json!({
            "dragon": self.dragon.as_ref().map(|d| serde_json::json!({
                "id": d.id,
                "position": [d.position.x, d.position.y, d.position.z],
                "health": d.health,
                "phase": d.phase.id(),
                "death_time": d.death_time,
                // The player it strafes: "host", or the guest's entity id.
                "strafing": match &d.phase {
                    Phase::Strafe { player: Some(FightWho::Host), .. } => serde_json::json!("host"),
                    Phase::Strafe { player: Some(FightWho::Guest(conn)), .. } => serde_json::json!(
                        self.lan.as_ref().and_then(|h| h.hub.guest(*conn)).map(|g| g.entity_id)
                    ),
                    _ => serde_json::Value::Null,
                },
            })),
            "players": self.fight.players,
            "crystals": self.crystals.len(),
            // The tracked id (0 until tracked) and position of each crystal.
            "crystal_list": self.crystals.iter().map(|c| serde_json::json!({
                "id": c.id,
                "position": [c.position.x, c.position.y, c.position.z],
            })).collect::<Vec<_>>(),
            "alive_crystals": self.fight.alive_crystals,
            "killed": self.fight.killed,
            "exit_portal": self.fight.exit_portal,
            "respawn": self.fight.respawn.map(|(s, t)| format!("{} {t}", s.name())),
        })
    }
}
/// Respawning the dragon with end crystals on the exit portal
/// (`EndCrystalItem.useOn`, `EnderDragonFight.tryRespawn`, `DragonRespawnStage`).
impl Game<'_> {
    /// `EndCrystalItem.useOn`'s checks: obsidian or bedrock at `pos`, air
    /// above it and no entity in the two blocks above (else FAIL).
    pub(in crate::game) fn end_crystal_fits(&self, pos: [i32; 3]) -> bool {
        let name = &self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name;
        if !matches!(&**name, "minecraft:obsidian" | "minecraft:bedrock") {
            return false;
        }
        let above = [pos[0], pos[1] + 1, pos[2]];
        if !self
            .res
            .facts()
            .is_air(self.world.block(above[0], above[1], above[2]))
        {
            return false;
        }
        let (x, y, z) = (
            f64::from(above[0]),
            f64::from(above[1]),
            f64::from(above[2]),
        );
        !self.entity_in(Aabb::new(
            Vec3::new(x, y, z),
            Vec3::new(x + 1.0, y + 2.0, z + 1.0),
        ))
    }

    /// `EndCrystalItem.useOn`: on obsidian or bedrock with room above.
    pub(super) fn place_end_crystal(
        &mut self,
        hit: &super::Hit,
        stack: &sunrise_client::items::ItemStack,
    ) -> bool {
        if &*stack.item != "minecraft:end_crystal" || !self.end_crystal_fits(hit.pos) {
            return false;
        }
        let above = [hit.pos[0], hit.pos[1] + 1, hit.pos[2]];
        let (x, y, z) = (
            f64::from(above[0]),
            f64::from(above[1]),
            f64::from(above[2]),
        );
        // EndCrystal's constructor: `this.random.nextInt(100000)`, the crystal's own random.
        let drawn = random_source::entity().next_int_bound(100_000).unwrap_or(0);
        let at = Vec3::new(x + 0.5, y, z + 0.5);
        let time = Crystal::start_time(at, drawn, self.clock.still());
        // EndCrystal's constructor takes an id from the entity counter, so
        // the crystal is an entity a command can name (`@e[type=end_crystal]`).
        self.next_entity += 1;
        self.crystals
            .push(Crystal::new(self.next_entity, at, false, time));
        // EndCrystalItem.useOn: ENTITY_PLACE at the block above, by the player.
        self.post_block_game_event("minecraft:entity_place", above, self.sculk_actor(), None);
        self.world.modified.insert((above[0] >> 4, above[2] >> 4));
        if self.dimension_type().has_ender_dragon_fight {
            self.try_respawn();
        }
        if !self.creative {
            self.consume_held();
        }
        // ItemStack.useOn counts the use.
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        true
    }
    /// `EndCrystal.kill` (`/kill`): the crystal is removed, without a blast
    /// (`Entity.kill` does not hurt it).
    pub(super) fn kill_crystal(&mut self, id: u32) {
        if let Some(i) = self.crystals.iter().position(|c| c.id == id) {
            let c = self.crystals[i].clone();
            let respawn = self.fight.respawn.is_some()
                && self
                    .fight
                    .respawn_crystals
                    .iter()
                    .any(|p| p.distance_squared(c.position) < 0.01);
            self.crystal_destroyed(i, &c, None, respawn);
            self.crystals.retain(|crystal| crystal.id != id);
            self.entity_killed_event([c.position.x, c.position.y, c.position.z], false);
        }
    }
    /// Any entity touching `room` (`Level.getEntities(null, box)`).
    fn entity_in(&self, room: Aabb) -> bool {
        let touches = |lo: Vec3, hi: Vec3| {
            let (a, b) = (room.min_position(), room.max_position());
            lo.x < b.x && hi.x > a.x && lo.y < b.y && hi.y > a.y && lo.z < b.z && hi.z > a.z
        };
        // Level.getEntities: the host's player and every guest in the level.
        let player = self
            .fight_players()
            .iter()
            .any(|p| !p.spectator && touches(p.bounds.min_position(), p.bounds.max_position()));
        player
            || self.crystals.iter().any(|c| {
                touches(
                    Vec3::new(c.position.x - 1.0, c.position.y, c.position.z - 1.0),
                    Vec3::new(c.position.x + 1.0, c.position.y + 2.0, c.position.z + 1.0),
                )
            })
            || self.mobs.iter().any(|m| {
                let w = f64::from(m.width()) / 2.0;
                let h = f64::from(m.height());
                touches(
                    Vec3::new(m.position.x - w, m.position.y, m.position.z - w),
                    Vec3::new(m.position.x + w, m.position.y + h, m.position.z + w),
                )
            })
    }
    /// `EnderDragonFight.tryRespawn`: a crystal on each side of the portal.
    /// True when a respawn started.
    pub(super) fn try_respawn(&mut self) -> bool {
        self.load_fight();
        if !self.fight.killed || self.fight.respawn.is_some() {
            return false;
        }
        let origin = match self.fight.exit_portal {
            Some(o) => o,
            None => {
                let _ = self.activate_exit_portal();
                match self.fight.exit_portal {
                    Some(o) => o,
                    None => return false,
                }
            }
        };
        let mut found = Vec::new();
        for d in [[0, -1], [0, 1], [-1, 0], [1, 0]] {
            // new AABB(center.relative(direction, 3)): the block cube.
            let cell = [origin[0] + d[0] * 3, origin[1] + 1, origin[2] + d[1] * 3];
            let lo = Vec3::new(f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]));
            let hits: Vec<Vec3> = self
                .crystals
                .iter()
                .filter(|c| {
                    let p = c.position;
                    p.x + 1.0 > lo.x
                        && p.x - 1.0 < lo.x + 1.0
                        && p.y + 2.0 > lo.y
                        && p.y < lo.y + 1.0
                        && p.z + 1.0 > lo.z
                        && p.z - 1.0 < lo.z + 1.0
                })
                .map(|c| c.position)
                .collect();
            if hits.is_empty() {
                return false;
            }
            found.extend(hits);
        }
        // respawnDragon: the podium goes back to end stone, then inactive.
        for y in origin[1] - 1..=origin[1] + 3 {
            for z in origin[2] - 3..=origin[2] + 3 {
                for x in origin[0] - 3..=origin[0] + 3 {
                    let name = self.block_name_at([x, y, z]);
                    if matches!(&*name, "minecraft:bedrock" | "minecraft:end_portal") {
                        let _ = self.set_state_named([x, y, z], "minecraft:end_stone", &[]);
                    }
                }
            }
        }
        let _ = self.place_podium(origin, false);
        self.fight.respawn = Some((Respawn::Start, 0));
        self.fight.respawn_crystals = found;
        self.fight.dirty = true;
        self.native_dragon_respawn_stage(self.fight_level(), DragonRespawnStage::Start);
        true
    }
    fn block_name_at(&self, p: [i32; 3]) -> std::sync::Arc<str> {
        self.res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name
            .clone()
    }
    fn respawn_beams(&mut self, target: Option<[i32; 3]>) {
        let list = self.fight.respawn_crystals.clone();
        for c in &mut self.crystals {
            if list.iter().any(|p| p.distance_squared(c.position) < 0.01) {
                c.beam = target;
            }
        }
    }
    /// `LevelEvent 3001`: the dragon's growl over the island.
    fn respawn_growl(&mut self) {
        let pitch = 0.8 + self.world.random.next_float() * 0.3;
        self.play_audio(
            "minecraft:entity.ender_dragon.growl",
            Some([0.5, 128.5, 0.5]),
            64.0,
            pitch,
        );
    }
    /// `setRespawnStage` short of `END`.
    fn set_respawn(&mut self, stage: Respawn) {
        self.fight.respawn = Some((stage, 0));
        self.fight.dirty = true;
        self.native_dragon_respawn_stage(self.fight_level(), stage.api());
    }
    /// `EnderDragonFight.tick` with a respawn in progress.
    fn tick_respawn(&mut self) {
        let Some((stage, time)) = self.fight.respawn else {
            return;
        };
        let alive = self.fight.respawn_crystals.iter().any(|p| {
            self.crystals
                .iter()
                .any(|c| p.distance_squared(c.position) < 0.01)
        });
        if !alive {
            self.abort_respawn();
            return;
        }
        if let Some(r) = self.fight.respawn.as_mut() {
            r.1 += 1;
        }
        const CENTER: [i32; 3] = [0, 128, 0];
        match stage {
            Respawn::Start => {
                self.respawn_beams(Some(CENTER));
                self.set_respawn(Respawn::Preparing);
            }
            Respawn::Preparing => {
                if time < 100 {
                    if matches!(time, 0 | 50..=52) || time >= 95 {
                        self.respawn_growl();
                    }
                } else {
                    self.set_respawn(Respawn::Pillars);
                }
            }
            Respawn::Pillars => {
                let (start, end) = (time % 40 == 0, time % 40 == 39);
                if !(start || end) {
                    return;
                }
                let spikes = sunrise_world::worldgen::end_spike::spikes_for_seed(self.level.seed);
                let index = (time / 40) as usize;
                if let Some(spike) = spikes.get(index) {
                    if start {
                        self.respawn_beams(Some([
                            spike.center_x,
                            spike.height + 1,
                            spike.center_z,
                        ]));
                    } else {
                        self.rebuild_spike(spike);
                    }
                } else if start {
                    self.set_respawn(Respawn::Summoning);
                }
            }
            Respawn::Summoning => {
                if time >= 100 {
                    self.finish_respawn();
                } else if time >= 80 || (1..5).contains(&time) {
                    self.respawn_growl();
                } else if time == 0 {
                    self.respawn_beams(Some(CENTER));
                }
            }
        }
    }
    /// SUMMONING_PILLARS at the end of a beam: the spike is cleared, blown up
    /// and grown again with a guarded crystal.
    fn rebuild_spike(&mut self, spike: &sunrise_world::worldgen::end_spike::EndSpike) {
        let (cx, cz, h) = (spike.center_x, spike.center_z, spike.height);
        let air = self.pack.assets.registry.default_state("minecraft:air");
        let Some(air) = air else {
            return;
        };
        for y in h - 10..=h + 10 {
            for z in cz - 10..=cz + 10 {
                for x in cx - 10..=cx + 10 {
                    if !self.res.facts().is_air(self.world.block(x, y, z)) {
                        let _ = self.level_set([x, y, z], air);
                    }
                }
            }
        }
        // DragonRespawnStage.SUMMONING_PILLARS: ExplosionInteraction.BLOCK.
        let interaction =
            self.destroy_type(sunrise_client::gamerules::vanilla::BLOCK_EXPLOSION_DROP_DECAY);
        let _ = self.server_explode(
            [f64::from(cx) + 0.5, f64::from(h), f64::from(cz) + 0.5],
            5.0,
            interaction,
            false,
        );
        // EndSpikeFeature.placeSpike with crystal_invulnerable and the beam.
        let r = spike.radius;
        for z in cz - r..=cz + r {
            for y in 0..=h + 10 {
                for x in cx - r..=cx + r {
                    let (dx, dz) = (f64::from(x - cx), f64::from(z - cz));
                    if dx * dx + dz * dz <= f64::from(r * r + 1) && y < h {
                        if &*self.block_name_at([x, y, z]) != "minecraft:obsidian" {
                            let _ = self.set_state_named([x, y, z], "minecraft:obsidian", &[]);
                        }
                    } else if y > 65 && !self.res.facts().is_air(self.world.block(x, y, z)) {
                        let _ = self.level_set([x, y, z], air);
                    }
                }
            }
        }
        if spike.guarded {
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    for dy in 0..=3 {
                        let top = dy == 3;
                        if !(dx.abs() == 2 || dz.abs() == 2 || top) {
                            continue;
                        }
                        let x_edge = dx.abs() == 2 || top;
                        let z_edge = dz.abs() == 2 || top;
                        self.set_bars(
                            [cx + dx, h + dy, cz + dz],
                            [
                                x_edge && dz != -2,
                                x_edge && dz != 2,
                                z_edge && dx != -2,
                                z_edge && dx != 2,
                            ],
                        );
                    }
                }
            }
        }
        let at = Vec3::new(f64::from(cx) + 0.5, f64::from(h + 1), f64::from(cz) + 0.5);
        self.crystals
            .retain(|c| c.position.distance_squared(at) > 0.01);
        let (drawn, _) = Crystal::own_draws();
        let time = Crystal::start_time(at, drawn, self.clock.still());
        self.next_entity += 1;
        let crystal_id = self.next_entity;
        self.crystals.push(Crystal {
            beam: Some([0, 128, 0]),
            invulnerable: true,
            ..Crystal::new(crystal_id, at, true, time)
        });
        let _ = self.set_state_named([cx, h, cz], "minecraft:bedrock", &[]);
        let _ = self.set_state_named([cx, h + 1, cz], "minecraft:fire", &[]);
        self.world.modified.insert((cx >> 4, cz >> 4));
    }
    /// Iron bars connected north, south, west and east.
    fn set_bars(&mut self, pos: [i32; 3], sides: [bool; 4]) {
        let registry = self.pack.assets.registry;
        let Some(mut state) = registry
            .default_state("minecraft:iron_bars")
            .and_then(|d| registry.state(d).cloned())
        else {
            return;
        };
        for (name, on) in ["north", "south", "west", "east"].into_iter().zip(sides) {
            state
                .properties
                .insert(name.into(), if on { "true" } else { "false" }.into());
        }
        if let Ok(id) = registry.resolve(&state) {
            let _ = self.level_set(pos, id);
        }
    }
    /// `resetSpikeCrystals`: spike crystals lose their beams and protection.
    pub(super) fn reset_spike_crystals(&mut self) {
        let spikes = sunrise_world::worldgen::end_spike::spikes_for_seed(self.level.seed);
        for c in &mut self.crystals {
            let on_spike = spikes.iter().any(|s| {
                (c.position.x - f64::from(s.center_x) - 0.5).abs() <= f64::from(s.radius)
                    && (c.position.z - f64::from(s.center_z) - 0.5).abs() <= f64::from(s.radius)
                    && c.position.y >= f64::from(s.height)
            });
            if on_spike {
                c.invulnerable = false;
                c.beam = None;
            }
        }
    }
    /// `EnderDragonFight.createNewDragon`: over the podium, holding pattern, a random yaw.
    /// It becomes the fight's dragon (`dragonUUID = dragon.getUUID()`).
    fn create_new_dragon(&mut self) {
        let yaw = self.world.random.next_float() * 360.0;
        let seed = random_source::unique_seed();
        let mut dragon = Dragon::new(Vec3::new(0.0, 128.0, 0.0), yaw, seed);
        // The fight's snapTo sets only the view yaw, not the new living angles.
        dragon.body_yaw = 0.0;
        dragon.head_yaw = 0.0;
        dragon.join = sunrise_client::join_seq::JoinSeq::next();
        dragon.id = self.next_entity + 1;
        dragon.phase = Phase::Holding {
            path: None,
            target: None,
            clockwise: false,
        };
        self.next_entity += 9;
        let id = dragon.id;
        assert!(self.dragon.install(dragon).is_ok());
        self.fight.dragon_uuid = Some(self.uuid_for(id));
        self.fight.dirty = true;
    }
    /// `EnderDragonFight.findOrCreateDragon`: the End's dragon becomes the
    /// fight's, or a new one is made.
    fn find_or_create_dragon(&mut self) {
        match self.dragon.as_ref().map(|d| (d.id, d.alive())) {
            None => self.create_new_dragon(),
            Some((id, true)) => {
                self.fight.dragon_uuid = Some(self.uuid_for(id));
                self.fight.dirty = true;
            }
            // `ServerLevel.getDragons` takes only living dragons, so vanilla
            // makes a new one beside a dying one; with one dragon to a level,
            // the new one comes once the dying one is gone.
            Some((_, false)) => {}
        }
    }
    /// `EnderDragonFight.scanState`, for a world from before the fight was
    /// saved: an active exit portal means the dragon was killed before. A
    /// living dragon there is the fight's, unless there is no active portal:
    /// then it is removed for a new one.
    fn scan_state(&mut self) {
        let active = self.end_portal_active();
        self.fight.previously_killed = active;
        match self.dragon.as_ref().filter(|d| d.alive()).map(|d| d.id) {
            None => self.fight.killed = true,
            Some(id) => {
                self.fight.dragon_uuid = Some(self.uuid_for(id));
                self.fight.killed = false;
                if !active {
                    self.discard_resident_dragon();
                    self.fight.dragon_uuid = None;
                }
            }
        }
        if !self.fight.previously_killed && self.fight.killed {
            self.fight.killed = false;
        }
    }
    /// The dragon is the End fight's own (`EnderDragon.dragonFight`, set
    /// only for the dragon whose UUID is the fight's `dragonUUID`).
    pub(super) fn is_fight_dragon(&mut self) -> bool {
        if !self.dimension_type().has_ender_dragon_fight {
            return false;
        }
        match (self.dragon.as_ref().map(|d| d.id), self.fight.dragon_uuid) {
            (Some(id), Some(uuid)) if id != 0 => self.uuid_for(id) == uuid,
            _ => false,
        }
    }
    /// The END stage: the portal crystals blow up (no block damage) and a
    /// new dragon takes over.
    fn finish_respawn(&mut self) {
        // Portal crystals are not on spikes; clear their revival beams before
        // the spike reset and before they are removed (vanilla resetEndCrystals).
        self.respawn_beams(None);
        self.reset_spike_crystals();
        self.fight.respawn = None;
        let list = std::mem::take(&mut self.fight.respawn_crystals);
        for p in list {
            if let Some(i) = self
                .crystals
                .iter()
                .position(|c| c.position.distance_squared(p) < 0.01)
            {
                let c = self.crystals.remove(i);
                self.world.modified.insert((
                    c.position.x.floor() as i32 >> 4,
                    c.position.z.floor() as i32 >> 4,
                ));
                let _ = self.server_explode_from(
                    [c.position.x, c.position.y, c.position.z],
                    6.0,
                    super::explosion::BlockInteraction::Keep,
                    false,
                    super::explosion::ExplosionSource::default()
                        .with_silence(self.entity_silent(c.id))
                        .with_entity(
                            c.id,
                            "minecraft:end_crystal",
                            sunrise_client::health::DirectEntity::Other,
                        ),
                );
            }
        }
        self.fight.killed = false;
        // setRespawnStage(END): the new dragon is summoned for every player
        // of the fight (the boss bar's players: alive within 192 blocks).
        self.create_new_dragon();
        let view = self
            .dragon
            .as_ref()
            .map(|d| sunrise_client::advancements::EntityView {
                kind: "minecraft:ender_dragon".to_owned(),
                position: [d.position.x, d.position.y, d.position.z],
                ..Default::default()
            });
        if let Some(view) = view {
            self.for_each_player(|g| {
                let near = g.player.as_ref().is_some_and(|p| {
                    !p.health.dead_or_dying()
                        && p.position.x.powi(2)
                            + (p.position.y - 128.0).powi(2)
                            + p.position.z.powi(2)
                            <= 192.0 * 192.0
                });
                if near {
                    g.trigger_entity("minecraft:summoned_entity", "entity", &view);
                }
            });
        }
        self.fight.dirty = true;
        self.count_crystals();
        self.native_dragon_respawn_stage(self.fight_level(), DragonRespawnStage::End);
    }
    /// `abortRespawnSequence`.
    fn abort_respawn(&mut self) {
        self.fight.respawn = None;
        self.respawn_beams(None);
        self.fight.respawn_crystals.clear();
        self.reset_spike_crystals();
        let _ = self.activate_exit_portal();
        self.fight.dirty = true;
    }
    /// `EnderDragon.checkCrystals`' search: the nearest crystal whose box
    /// meets the dragon's box grown by 32.
    fn nearest_crystal_to(&self, p: Vec3) -> Option<usize> {
        // The dragon's 16 by 8 box.
        let reach = Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        )
        .inflate(Vec3::new(32.0, 32.0, 32.0));
        self.crystals
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                // An end crystal's 2 by 2 box.
                reach.intersects(Aabb::new(
                    Vec3::new(c.position.x - 1.0, c.position.y, c.position.z - 1.0),
                    Vec3::new(c.position.x + 1.0, c.position.y + 2.0, c.position.z + 1.0),
                ))
            })
            .min_by(|a, b| {
                let da = (a.1.position.x - p.x).powi(2)
                    + (a.1.position.y - p.y).powi(2)
                    + (a.1.position.z - p.z).powi(2);
                let db = (b.1.position.x - p.x).powi(2)
                    + (b.1.position.y - p.y).powi(2)
                    + (b.1.position.z - p.z).powi(2);
                da.total_cmp(&db)
            })
            .map(|(i, _)| i)
    }
}

/// The dragon on a server's client (`ClientPacketListener.handleAddEntity`
/// and the client side of `EnderDragon.aiStep`): the server flies it and
/// sends its moves, health and phase; the client flaps its wings, keeps
/// its flight history and places its parts, so it can be drawn and hit.
impl Game<'_> {
    /// `Mob.playAmbientSound` when EntityType.spawn places a dragon.
    pub(super) fn dragon_spawn_ambient(&mut self, id: u32) -> bool {
        let Some(d) = self.dragon.as_mut().filter(|d| d.id == id) else {
            return false;
        };
        let at = [d.position.x, d.position.y, d.position.z];
        let pitch = (d.random.next_float() - d.random.next_float()) * 0.2 + 1.0;
        self.play_audio(
            "minecraft:entity.ender_dragon.ambient",
            Some(at),
            5.0,
            pitch,
        );
        true
    }

    /// `EntityType.create` for an egg dragon, or `SummonCommand.createEntity` without NBT.
    /// Unlike the End fight's dragon, this
    /// one is not the fight's (`EnderDragon.aiStep` adopts it only when its UUID is the fight's),
    /// so it can be spawned in any dimension. Sunrise keeps one dragon per level, as `/summon`
    /// and loading do: with one already there, nothing spawns and the egg stays.
    pub(super) fn spawn_dragon_spawned(&mut self, at: [f64; 3], site: SpawnSite) -> Option<u32> {
        self.spawn_dragon_facing(at, site, None)
    }
    /// [`spawn_dragon_spawned`](Self::spawn_dragon_spawned) snapped to `fixed_yaw` when
    /// given (`snapTo(pos, yaw, 0)`), which takes no draw from the level.
    pub(super) fn spawn_dragon_facing(
        &mut self,
        at: [f64; 3],
        site: SpawnSite,
        fixed_yaw: Option<f32>,
    ) -> Option<u32> {
        if self.dragon.is_some() {
            return None;
        }
        let position = Vec3::new(at[0], at[1], at[2]);
        // SummonCommand loads absent Rotation as zero before finalization. Only
        // EntityType.create's egg placement draws a random snapTo yaw.
        // Diverges from upstream: shared mod finalization also admits command dragons.
        let yaw = match (fixed_yaw, site) {
            (Some(yaw), _) => yaw,
            (None, SpawnSite::Finalizing(sunrise_client::mobs::spawn_rules::Reason::Command)) => {
                0.0
            }
            _ => sunrise_client::camera::wrap_degrees(self.world.random.next_float() * 360.0),
        };
        let finalized = match site {
            SpawnSite::Finalizing(reason) => {
                self.finalize_spawn_at("minecraft:ender_dragon", at, reason, None)
                    .0
            }
            SpawnSite::Plain | SpawnSite::Raw => Default::default(),
        };
        if !finalized.skip_initialization {
            // Mob.finalizeSpawn (EnderDragon keeps it): the follow range's
            // triangle and left-handed roll, neither of which changes how the dragon flies.
            let _ = self.world.random.triangle(0.0, 0.114_850_000_000_000_01);
            let _ = self.world.random.next_float();
        }
        if finalized.veto_admission {
            return None;
        }
        // The dragon's own random (`Entity.random`), not the level's.
        let seed = random_source::unique_seed();
        let mut dragon = Dragon::new(position, yaw, seed);
        dragon.join = sunrise_client::join_seq::JoinSeq::next();
        dragon.id = self.next_entity + 1;
        self.next_entity += 9;
        let id = dragon.id;
        // Finalization listeners can install another dragon after the initial
        // refusal check. Keep the constructor's existing replacement order.
        self.replace_resident_dragon(dragon);
        // Entity's constructor assigns the identity before any join callbacks or hits.
        self.uuid_for(id);
        self.place_dragon_parts();
        self.native_joins_flush(false, id).then_some(id)
    }

    /// `handleAddEntity` for `minecraft:ender_dragon`. Its constructor
    /// sets the full health and the hovering phase until the server's
    /// synched data says otherwise.
    pub(super) fn spawn_remote_dragon(&mut self, id: i32, position: [f64; 3], yaw: f32) {
        let at = Vec3::new(position[0], position[1], position[2]);
        let mut d = Dragon::new(at, yaw, i64::from(id));
        d.id = id as u32;
        d.phase = Phase::from_id(10);
        d.join = sunrise_client::join_seq::JoinSeq::next();
        self.replace_resident_dragon(d);
        self.place_dragon_parts();
        if let Some(d) = self.dragon.as_mut() {
            for part in &mut d.parts {
                part.1 = part.0;
            }
        }
    }
    /// The remote dragon, if `id` is it or one of its parts.
    fn remote_dragon_mut(&mut self, id: i32) -> Option<&mut Dragon> {
        let uid = id as u32;
        let remote = self.remote.is_some();
        self.dragon.as_mut().filter(|d| {
            (d.id != 0 || remote) && (d.id == uid || dragon_part_index(d.id, uid).is_some())
        })
    }
    /// `handleSetEntityData`: LivingEntity.DATA_HEALTH_ID and
    /// EnderDragon.DATA_PHASE (`onSyncedDataUpdated` sets the phase).
    pub(super) fn remote_dragon_data(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        let Some(d) = self.remote_dragon_mut(id) else {
            return;
        };
        for (index, _, value) in values {
            match (index, value) {
                (9, Value::Float(health)) => d.health = *health,
                (16, Value::Int(phase)) => d.set_phase(Phase::from_id(*phase)),
                // Mob.DATA_MOB_FLAGS_ID: isNoAi.
                (15, Value::Byte(flags)) => d.no_ai = flags & 1 != 0,
                _ => {}
            }
        }
    }
    /// `handleSetEntityMotion`: the velocity its wings flap by.
    pub(super) fn remote_dragon_motion(&mut self, id: i32, velocity: [f64; 3]) {
        if let Some(d) = self.remote_dragon_mut(id) {
            d.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        }
    }
    /// `handleHurtAnimation` and `handleDamageEvent`: the red flash.
    pub(super) fn remote_dragon_hurt(&mut self, id: i32) {
        if let Some(d) = self.remote_dragon_mut(id) {
            // `LivingEntity.handleDamageEvent`: the red flash. Its sound is the
            // server's (`playHurtSound`); a client plays one only for its own player.
            d.hurt_time = 10;
        }
    }
    /// `handleRemoveEntities`, or a crystal removed: the healing beam
    /// keeps pointing at the same crystal.
    pub(super) fn remove_remote_dragon(&mut self, id: i32) {
        if self.dragon.as_ref().is_some_and(|d| d.id as i32 == id) {
            self.discard_resident_dragon();
            return;
        }
        let Some(index) = self.crystals.iter().position(|c| c.id as i32 == id) else {
            return;
        };
        if let Some(d) = self.dragon.as_mut() {
            match d.nearest_crystal {
                Some(n) if n == index => d.nearest_crystal = None,
                Some(n) if n > index => d.nearest_crystal = Some(n - 1),
                _ => {}
            }
        }
    }
    /// One client tick of the dragon. `lerp`: the step toward its last
    /// sent position and yaw (LivingEntity's interpolation), and the
    /// steps left including this one.
    pub(super) fn tick_remote_dragon(&mut self, lerp: Option<([f64; 3], Option<f32>, u32)>) {
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        d.previous = d.position;
        d.previous_yaw = d.yaw;
        d.old_flap = d.flap;
        d.ticks += 1;
        d.hurt_time = (d.hurt_time - 1).max(0);
        for part in &mut d.parts {
            part.1 = part.0;
        }
        if let Some((target, yaw, steps)) = lerp {
            let n = f64::from(steps.max(1));
            d.position = Vec3::new(
                d.position.x + (target[0] - d.position.x) / n,
                d.position.y + (target[1] - d.position.y) / n,
                d.position.z + (target[2] - d.position.z) / n,
            );
            if let Some(yaw) = yaw {
                d.yaw += wrap(yaw - d.yaw) / steps.max(1) as f32;
            }
        }
        if !d.alive() {
            // EnderDragon.tickDeath on a client: it rises, and bursts
            // from tick 180 to 200; the server removes it.
            d.death_time += 1;
            let (t, p) = (d.death_time, d.position);
            // `EnderDragon.aiStep` still growls when the phase is not sitting.
            let sitting = d.phase.sitting();
            if (180..=200).contains(&t) {
                let r = &mut d.random;
                let o = [
                    f64::from((r.next_float() - 0.5) * 8.0),
                    f64::from((r.next_float() - 0.5) * 4.0),
                    f64::from((r.next_float() - 0.5) * 8.0),
                ];
                self.particles.add(
                    &ParticleOptions::simple(ParticleType::ExplosionEmitter),
                    [p.x + o[0], p.y + 2.0 + o[1], p.z + o[2]],
                    [0.0; 3],
                );
            }
            if let Some(d) = self.dragon.as_mut() {
                // deathMove is new Vec3(0.0, 0.1F, 0.0): the float widened.
                let rise = f64::from(0.1_f32);
                d.position.y += rise;
                for part in &mut d.parts {
                    part.0.y += rise;
                }
            }
            // `EnderDragon.aiStep` after `tickDeath`: the client growl is
            // before the `isDeadOrDying` branch, then a dead dragon adds
            // `ParticleTypes.EXPLOSION` every tick. Both use the risen spot.
            let mut growl = None;
            if let Some(d) = self.dragon.as_mut()
                && !sitting
            {
                d.growl -= 1;
                if d.growl < 0 {
                    // `this.random`: pitch, then `nextInt(200)`, before the puff.
                    let pitch = 0.8 + d.random.next_float() * 0.3;
                    d.growl = 200 + d.random.next_int_bound(200).unwrap_or(0);
                    growl = Some(([d.position.x, d.position.y, d.position.z], pitch));
                }
            }
            if let Some((at, pitch)) = growl {
                self.play_audio("minecraft:entity.ender_dragon.growl", Some(at), 2.5, pitch);
            }
            let Some(d) = self.dragon.as_mut() else {
                return;
            };
            let p = d.position;
            let r = &mut d.random;
            let o = [
                f64::from((r.next_float() - 0.5) * 8.0),
                f64::from((r.next_float() - 0.5) * 4.0),
                f64::from((r.next_float() - 0.5) * 8.0),
            ];
            self.particles.add(
                &ParticleOptions::simple(ParticleType::Explosion),
                [p.x + o[0], p.y + 2.0 + o[1], p.z + o[2]],
                [0.0; 3],
            );
            return;
        }
        // processFlappingMovement and onFlap: its wing beats, and their sound.
        // Mth.cos(flapTime * 6.2831855F); flapSpeed's `(float) Math.pow(2.0, movement.y)`.
        let flap_was = trig::cos(f64::from(d.old_flap * std::f32::consts::TAU));
        let v = d.velocity;
        let mut flap_speed = 0.2 / (((v.x * v.x + v.z * v.z).sqrt() as f32) * 10.0 + 1.0);
        flap_speed *= 2f64.powf(v.y) as f32;
        d.flap += if d.phase.sitting() { 0.1 } else { flap_speed };
        let flap_now = trig::cos(f64::from(d.flap * std::f32::consts::TAU));
        let at = [d.position.x, d.position.y, d.position.z];
        if flap_was <= -0.3 && flap_now >= -0.3 {
            let pitch = 0.8 + self.world.random.next_float() * 0.3;
            self.play_audio("minecraft:entity.ender_dragon.flap", Some(at), 5.0, pitch);
        }
        let d = self.dragon.as_mut().expect("dragon");
        if !d.phase.sitting() {
            d.growl -= 1;
            if d.growl < 0 {
                d.growl = 200 + d.random.next_int_bound(200).unwrap_or(0);
                let pitch = 0.8 + self.world.random.next_float() * 0.3;
                self.play_audio("minecraft:entity.ender_dragon.growl", Some(at), 2.5, pitch);
            }
        }
        // checkCrystals' search runs on a client too (the healing beam).
        let d = self.dragon.as_mut().expect("dragon");
        if d.random.next_int_bound(10).unwrap_or(1) == 0 {
            let p = d.position;
            let nearest = self.nearest_crystal_to(p);
            self.dragon.as_mut().expect("dragon").nearest_crystal = nearest;
        }
        let d = self.dragon.as_mut().expect("dragon");
        d.yaw = wrap(d.yaw);
        // EnderDragon.aiStep on a client with isNoAi, as on the server.
        if d.no_ai {
            d.flap = 0.5;
            return;
        }
        d.record();
        self.remote_dragon_phase_tick();
        self.place_dragon_parts();
    }
    /// `DragonLandingPhase.doClientTick`, `DragonSittingFlamingPhase.doClientTick`,
    /// `DragonDeathPhase.doClientTick` and `DragonSittingAttackingPhase.doClientTick`,
    /// which `EnderDragon.aiStep` runs on a client. The first two breathe
    /// dragon breath from the head (their `look.yRot(...)` results are thrown
    /// away in vanilla, so every puff leaves along the same line), the third
    /// puffs an explosion every 10 ticks and the last growls on every tick.
    fn remote_dragon_phase_tick(&mut self) {
        let look = self.dragon_look_normalised();
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        let head = d.parts[0].0;
        let head_y = head.y + PART_SIZES[0].1 * 0.5;
        let motion = d.velocity;
        let at = [d.position.x, d.position.y, d.position.z];
        // `flameTicks`: the client copy's own counter, advanced before it is read.
        if let Phase::Flaming { ticks } = &mut d.phase {
            *ticks += 1;
        }
        let puffs = breath_puffs(
            &d.phase,
            look,
            [head.x, head_y, head.z],
            [motion.x, motion.y, motion.z],
            &mut d.random,
        );
        let mut emitter = None;
        let mut growl = None;
        match &mut d.phase {
            Phase::Dying { time, .. } => {
                if *time % 10 == 0 {
                    let r = &mut d.random;
                    let o = [
                        f64::from((r.next_float() - 0.5) * 8.0),
                        f64::from((r.next_float() - 0.5) * 4.0),
                        f64::from((r.next_float() - 0.5) * 8.0),
                    ];
                    emitter = Some([at[0] + o[0], at[1] + 2.0 + o[1], at[2] + o[2]]);
                }
                *time += 1;
            }
            Phase::Attacking { .. } => {
                growl = Some(0.8 + d.random.next_float() * 0.3);
            }
            _ => {}
        }
        let breath = ParticleOptions::simple(ParticleType::DragonBreath);
        for (p, v) in puffs {
            self.particles.add(&breath, p, v);
        }
        if let Some(p) = emitter {
            self.particles.add(
                &ParticleOptions::simple(ParticleType::ExplosionEmitter),
                p,
                [0.0; 3],
            );
        }
        if let Some(pitch) = growl {
            self.play_audio("minecraft:entity.ender_dragon.growl", Some(at), 2.5, pitch);
        }
    }
}

#[path = "dragon/crystal.rs"]
mod crystal;

/// `DragonSittingScanningPhase.doServerTick` with a player at `target` and 25 ticks or fewer of
/// scanning: the dragon turns only while that player is more than 10 degrees off its heading
/// (`Math.acos` of the float dot, in degrees, plus 0.5). `yRotA` then eases by 0.8 and gains the
/// delta (`180 - Mth.atan2` in degrees less the yaw, wrapped, held within 100) over the distance
/// from the head plus one capped at 40 and over that distance uncapped, and the yaw takes it.
fn scanning_turn(yaw: &mut f32, yaw_accel: &mut f32, body: Vec3, head: Vec3, target: Vec3) {
    let aim = vanilla_normalize(glam::DVec3::new(target.x - body.x, 0.0, target.z - body.z));
    let r = f64::from(*yaw * trig::DEG_TO_RAD);
    let dir = vanilla_normalize(glam::DVec3::new(
        f64::from(trig::sin(r)),
        0.0,
        f64::from(-trig::cos(r)),
    ));
    let dot = dir.dot(aim) as f32;
    let angle =
        (sunrise_core::jmath::acos(f64::from(dot)) * f64::from(trig::RAD_TO_DEG)) as f32 + 0.5;
    if angle < 0.0 || angle > 10.0 {
        let (xd, zd) = (target.x - head.x, target.z - head.z);
        let delta =
            wrap64(180.0 - trig::atan2(xd, zd) * f64::from(trig::RAD_TO_DEG) - f64::from(*yaw))
                .clamp(-100.0, 100.0);
        *yaw_accel *= 0.8;
        let rot_speed = (xd * xd + zd * zd).sqrt() as f32 + 1.0;
        let dist = if rot_speed > 40.0 { 40.0 } else { rot_speed };
        *yaw_accel += delta as f32 * (0.7 / dist / rot_speed);
        *yaw += *yaw_accel;
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    /// The pinned vanilla 26.3 side is `VanillaTurnOracle`'s `dragon.txt`
    /// (`tools/test_mob_turn_parity.py`, recorded into `.local/witness/26.3/mob_turn/dragon.txt`):
    /// `DragonSittingScanningPhase.doServerTick` with its scan's player at each place, from each
    /// yaw, `yRotA` and head, as the yaw and `yRotA` bits it leaves.
    #[test]
    #[ignore = "run by tools/test_mob_turn_parity.py"]
    fn the_scanning_turn_matches_the_pinned_oracle() {
        run_the_scanning_turn_matches_the_pinned_oracle();
    }

    pub(in crate::game) fn run_the_scanning_turn_matches_the_pinned_oracle() {
        let witness_dir = std::env::var_os("SUNRISE_MOB_TURN_WITNESS_DIR").expect(
            "SUNRISE_MOB_TURN_WITNESS_DIR must be set; run through tools/test_mob_turn_parity.py, which records the witness",
        );
        let witness_path = std::path::Path::new(&witness_dir).join("dragon.txt");
        let witness = std::fs::read_to_string(&witness_path).unwrap_or_else(|err| {
            panic!(
                "failed to read {} ({err}); run through tools/test_mob_turn_parity.py, which records the witness",
                witness_path.display()
            )
        });
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let mut rows = Vec::new();
        let places = [
            [4.0, -3.0],
            [0.0, 30.0],
            [-35.0, 25.0],
            [50.0, 0.0],
            [0.5, -0.5],
            [12.0, 12.0],
            [-1.0, -60.0],
        ];
        for [px, pz] in places {
            for yaw in [45.0f32, 0.0, -170.0, 359.5, 180.0] {
                for accel in [0.0f32, 2.5, -7.25] {
                    for [hx, hz] in [[0.0, 0.0], [6.0, -4.0]] {
                        let (mut y, mut a) = (yaw, accel);
                        super::scanning_turn(
                            &mut y,
                            &mut a,
                            Vec3::new(0.0, 70.0, 0.0),
                            Vec3::new(hx, 72.0, hz),
                            Vec3::new(px, 70.0, pz),
                        );
                        rows.push(format!(
                            "{} {} {:08x} {:08x} {} {} {:08x} {:08x}",
                            bits(px),
                            bits(pz),
                            yaw.to_bits(),
                            accel.to_bits(),
                            bits(hx),
                            bits(hz),
                            y.to_bits(),
                            a.to_bits()
                        ));
                    }
                }
            }
        }
        let actual = rows.join("\n") + "\n";
        if let Some(dir) = std::env::var_os("SUNRISE_TURN_OUTPUT_DIR") {
            std::fs::write(std::path::Path::new(&dir).join("dragon.txt"), &actual).unwrap();
        }
        assert_eq!(rows.len(), 7 * 5 * 3 * 2);
        assert_eq!(actual, witness);
    }

    #[test]
    fn dragon_part_wire_ids_wrap_like_vanilla_i32() {
        let neg_one = (-1i32) as u32;
        for part in 0..8 {
            let wire = dragon_part_wire_id(neg_one, part);
            assert_eq!(wire, part as i32);
            assert_eq!(dragon_part_index(neg_one, wire as u32), Some(part));
        }
        assert_eq!(dragon_part_index(neg_one, (-1i32) as u32), None);
        assert_eq!(dragon_part_index(neg_one, 8), None);

        let max = i32::MAX as u32;
        assert_eq!(dragon_part_wire_id(max, 0), i32::MIN);
        assert_eq!(dragon_part_index(max, i32::MIN as u32), Some(0));
        assert_eq!(dragon_part_wire_id(max, 7), i32::MIN + 7);
        assert_eq!(dragon_part_index(max, (i32::MIN + 7) as u32), Some(7));
    }

    #[test]
    fn dragon_search_preserves_equal_cost_heap_order() {
        let xz = [
            [60, -1],
            [51, 29],
            [29, 51],
            [0, 60],
            [-30, 51],
            [-52, 30],
            [-60, 0],
            [-52, -31],
            [-31, -52],
            [-1, -60],
            [29, -52],
            [51, -30],
            [40, -1],
            [28, 28],
            [0, 40],
            [-29, 28],
            [-40, 0],
            [-29, -29],
            [-1, -40],
            [28, -29],
            [20, -1],
            [0, 20],
            [-20, 0],
            [-1, -20],
        ];
        let heights = [
            83, 88, 75, 82, 86, 82, 87, 88, 83, 75, 78, 74, 80, 77, 79, 73, 74, 82, 74, 73, 85, 75,
            75, 77,
        ];
        let nodes = std::array::from_fn(|i| [xz[i][0], heights[i], xz[i][1]]);
        let path = search_dragon_path(&nodes, 18, 6, 0, None).unwrap();
        // With crystals alive, the ruled array heap visits node 16 before 7 at a tie.
        assert_eq!(path.nodes, [nodes[18], nodes[17], nodes[16], nodes[6]]);
    }

    #[test]
    #[ignore = "daily"]
    fn a_spawn_egg_dragon_joins_once_and_a_canceled_spawn_fails() {
        use mod_api::{EntityJoinLevelEvent, Flow, Hooks, ModEntry, ServerHost};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };

        struct Observer {
            joins: Arc<Mutex<Vec<(u32, bool)>>>,
            cancel: Arc<AtomicBool>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                if event.kind != "minecraft:ender_dragon" {
                    return Flow::Continue;
                }
                self.joins
                    .lock()
                    .unwrap()
                    .push((event.subject.id(), event.loaded_from_disk));
                if self.cancel.load(Ordering::Relaxed) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        let joins = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "dragon-joins",
            Box::new(Observer {
                joins: joins.clone(),
                cancel: cancel.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.dragon.discard();
                game.native_joins_skip();
                let at = [4.5, 200.0, 4.5];
                let id = game
                    .spawn_mob_spawned(
                        "minecraft:ender_dragon",
                        at,
                        super::super::creatures::SpawnSite::Finalizing(
                            sunrise_client::mobs::spawn_rules::Reason::SpawnItemUse,
                        ),
                    )
                    .unwrap();
                assert_eq!(game.dragon.as_ref().unwrap().id, id);
                assert_eq!(*joins.lock().unwrap(), vec![(id, false)]);
                game.native_joins_flush(false, 0);
                assert_eq!(joins.lock().unwrap().len(), 1, "a join is told only once");
                let dispenser =
                    SpawnSite::Finalizing(sunrise_client::mobs::spawn_rules::Reason::Dispenser);
                assert!(game.spawn_dragon_spawned(at, dispenser).is_none());
                assert_eq!(joins.lock().unwrap().len(), 1, "no second dragon joins");

                game.dragon.discard();
                cancel.store(true, Ordering::Relaxed);
                // The dispenser calls the shared constructor directly.
                assert!(game.spawn_dragon_spawned(at, dispenser).is_none());
                assert!(game.dragon.is_none(), "a refused dragon never stays loaded");
                game.native_joins_flush(false, 0);
                assert_eq!(*joins.lock().unwrap(), vec![(id, false), (id + 9, false)]);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn dragon_finalization_preserves_the_site_and_both_cancels() {
        use mod_api::{FinalizeSpawnEvent, Flow, Hooks, Host, ModEntry};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_client::mobs::spawn_rules::Reason;

        #[derive(Clone, Copy, PartialEq)]
        enum Answer {
            Keep,
            Skip,
            Refuse,
        }
        struct Listener {
            answer: Answer,
            reason: Reason,
            calls: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn finalize_spawn(&self, _: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
                assert_eq!(event.kind, "minecraft:ender_dragon");
                assert_eq!(event.reason, self.reason);
                assert!(event.spawner.is_none());
                self.calls.fetch_add(1, Ordering::Relaxed);
                match self.answer {
                    Answer::Keep => Flow::Continue,
                    Answer::Skip => Flow::Cancel,
                    Answer::Refuse => {
                        event.spawn_cancelled = true;
                        Flow::Continue
                    }
                }
            }
        }
        super::super::native_entity_refs::tests::with_game(|game| {
            for reason in [Reason::SpawnItemUse, Reason::Dispenser] {
                for answer in [Answer::Keep, Answer::Skip, Answer::Refuse] {
                    game.dragon.discard();
                    game.world.random.set_seed(42);
                    let calls = Arc::new(AtomicUsize::new(0));
                    let mods: mod_api::Mods = vec![ModEntry::new(
                        "dragon-finalize",
                        Box::new(Listener {
                            answer,
                            reason,
                            calls: calls.clone(),
                        }),
                    )]
                    .into();
                    let result = mod_platform::testing::with_mods(mods, || {
                        game.spawn_dragon_spawned([4.5, 200.0, 4.5], SpawnSite::Finalizing(reason))
                    });
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    assert_eq!(result.is_none(), answer == Answer::Refuse);
                    assert_eq!(game.dragon.is_none(), answer == Answer::Refuse);
                    // snapTo always draws yaw; only completed Mob.finalizeSpawn
                    // draws the follow-range triangle and handedness roll.
                    let mut expected = LegacyRandom::new(42);
                    let yaw = sunrise_client::camera::wrap_degrees(expected.next_float() * 360.0);
                    if let Some(dragon) = game.dragon.as_ref() {
                        assert_eq!(dragon.yaw, yaw);
                        assert_eq!(dragon.previous_yaw, yaw);
                    }
                    if answer == Answer::Keep {
                        expected.triangle(0.0, 0.114_850_000_000_000_01);
                        expected.next_float();
                    }
                    assert_eq!(game.world.random.next_long(), expected.next_long());
                }
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn dragon_summon_command_finalizes_only_without_nbt() {
        use mod_api::{
            EntityJoinLevelEvent, FinalizeSpawnEvent, Flow, Hooks, Host, ModEntry, ServerHost,
        };
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_client::mobs::spawn_rules::Reason;

        #[derive(Clone, Copy, PartialEq)]
        enum Answer {
            Keep,
            Skip,
            Refuse,
            RefuseJoin,
        }
        struct Listener {
            answer: Answer,
            reason: Reason,
            calls: Arc<AtomicUsize>,
            joins: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                assert_eq!(event.kind, "minecraft:ender_dragon");
                assert!(!event.loaded_from_disk);
                self.joins.fetch_add(1, Ordering::Relaxed);
                if self.answer == Answer::RefuseJoin {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn finalize_spawn(&self, _: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
                assert_eq!(event.kind, "minecraft:ender_dragon");
                assert_eq!(event.reason, self.reason);
                assert!(event.spawner.is_none());
                self.calls.fetch_add(1, Ordering::Relaxed);
                match self.answer {
                    Answer::Keep | Answer::RefuseJoin => Flow::Continue,
                    Answer::Skip => Flow::Cancel,
                    Answer::Refuse => {
                        event.spawn_cancelled = true;
                        Flow::Continue
                    }
                }
            }
        }
        super::super::native_entity_refs::tests::with_game(|game| {
            for nbt in [None, Some("{}"), Some("{Silent:1b}")] {
                let reason = Reason::Command;
                for answer in [
                    Answer::Keep,
                    Answer::Skip,
                    Answer::Refuse,
                    Answer::RefuseJoin,
                ] {
                    game.dragon.discard();
                    game.world.random.set_seed(42);
                    let calls = Arc::new(AtomicUsize::new(0));
                    let joins = Arc::new(AtomicUsize::new(0));
                    let mods: mod_api::Mods = vec![ModEntry::new(
                        "dragon-finalize",
                        Box::new(Listener {
                            answer,
                            reason,
                            calls: calls.clone(),
                            joins: joins.clone(),
                        }),
                    )]
                    .into();
                    let result = mod_platform::testing::with_mods(mods, || {
                        game.build_registries();
                        game.set_allow_commands(true);
                        let suffix = nbt.map(|tag| format!(" {tag}")).unwrap_or_default();
                        game.run_command(&format!(
                            "summon minecraft:ender_dragon 4.5 200 4.5{suffix}"
                        ))
                    });
                    assert_eq!(calls.load(Ordering::Relaxed), usize::from(nbt.is_none()));
                    let refused_finalization = nbt.is_none() && answer == Answer::Refuse;
                    assert_eq!(
                        joins.load(Ordering::Relaxed),
                        usize::from(!refused_finalization)
                    );
                    let refused = refused_finalization || answer == Answer::RefuseJoin;
                    assert_eq!(result, !refused);
                    assert_eq!(game.dragon.is_none(), refused);
                    if !refused {
                        assert_eq!(game.dragon_silent(), nbt == Some("{Silent:1b}"));
                        let dragon = game.dragon.as_ref().unwrap();
                        assert_eq!(dragon.position, Vec3::new(4.5, 200.0, 4.5));
                        assert_eq!(dragon.yaw, 0.0);
                        assert_eq!(dragon.previous_yaw, 0.0);
                    }
                    // Command construction consumes no placement yaw. Mob.finalizeSpawn
                    // draws follow range and handedness unless skipped. The NBT
                    // loader seeds the dragon's own random without drawing from the level RNG.
                    let mut expected = LegacyRandom::new(42);
                    if nbt.is_none() && matches!(answer, Answer::Keep | Answer::RefuseJoin) {
                        expected.triangle(0.0, 0.114_850_000_000_000_01);
                        expected.next_float();
                    }
                    assert_eq!(game.world.random.next_long(), expected.next_long());
                }
            }
        });
    }

    #[test]
    fn a_dragon_save_keeps_equipment_only_while_something_is_equipped() {
        use sunrise_client::items::ItemStack;
        use sunrise_items::equipment::{BODY, SADDLE};
        let mut d = Dragon::new(Vec3::ZERO, 0.0, 0);
        // An old save has no `equipment` key: the dragon loads empty and saves none.
        let old = d.to_nbt([0; 4], None, &Default::default());
        assert!(old.get("equipment").is_none());
        assert!(Dragon::from_nbt(&old, 0).unwrap().equipment.is_empty());
        // Body and saddle round-trip with their name and damage.
        let mut body = ItemStack::new("minecraft:diamond_horse_armor", 1);
        body.damage = 6;
        body.data_mut().custom_name = Some(Tag::String("Plate".into()));
        d.equipment[BODY] = Some(body);
        d.equipment[SADDLE] = Some(ItemStack::new("minecraft:saddle", 1));
        let saved = d.to_nbt([0; 4], None, &Default::default());
        assert_eq!(Dragon::from_nbt(&saved, 0).unwrap().equipment, d.equipment);
        // Saving over a raw save that holds a stale key removes it once emptied.
        d.equipment.take_all().for_each(drop);
        assert!(
            d.to_nbt([0; 4], Some(&saved), &Default::default())
                .get("equipment")
                .is_none()
        );
    }

    fn player_box(feet: [f64; 3]) -> (Aabb, Vec3) {
        (
            Aabb::new(
                Vec3::new(feet[0] - 0.3, feet[1], feet[2] - 0.3),
                Vec3::new(feet[0] + 0.3, feet[1] + 1.8, feet[2] + 0.3),
            ),
            Vec3::new(feet[0], feet[1], feet[2]),
        )
    }

    /// `Entity.load` and `EndCrystal.readAdditionalSaveData` read their booleans with
    /// `getBooleanOr`: any number, `ShowBottom` true when absent.
    #[test]
    fn a_crystal_reads_its_flags_from_any_number() {
        let crystal = |fields: Vec<(&str, Tag)>| {
            let pos = [0.5f64, 64.0, 0.5].map(|v| Tag::Double(v.to_bits()));
            let mut all = vec![("Pos", Tag::List(pos.to_vec()))];
            all.extend(fields);
            Crystal::from_nbt(&Tag::compound(all), 0).unwrap()
        };
        let c = crystal(vec![
            ("Invulnerable", Tag::Byte(2)),
            ("ShowBottom", Tag::Int(0)),
        ]);
        assert!(c.invulnerable && !c.show_bottom);
        let c = crystal(Vec::new());
        assert!(!c.invulnerable && c.show_bottom);
    }

    /// Under `SUNRISE_STILL` a crystal's spin and bob start from its position, whichever order
    /// the crystals load in; without it each takes the next draw, as vanilla's do.
    #[test]
    fn a_still_crystal_starts_from_its_position_in_any_load_order() {
        let spikes = [Vec3::new(42.5, 77.0, 0.5), Vec3::new(-33.5, 95.0, 24.5)];
        let starts = |order: [usize; 2], still: bool| {
            let mut random = LegacyRandom::new(5);
            let mut times = [0; 2];
            for i in order {
                let drawn = random.next_int_bound(100_000).unwrap();
                times[i] = Crystal::start_time(spikes[i], drawn, still);
            }
            times
        };
        let still = starts([0, 1], true);
        assert_eq!(still, starts([1, 0], true));
        assert_ne!(still[0], still[1], "each crystal keeps its own phase");
        let first = LegacyRandom::new(5).next_int_bound(100_000).unwrap();
        assert_eq!(starts([0, 1], false)[0], first);
        assert_eq!(
            starts([1, 0], false)[1],
            first,
            "the first loaded takes the first draw"
        );
    }
    /// `Mth.wrapDegrees`: `% 360` then one step of 360, exact for a negative angle
    /// (a remainder made positive and back again loses the float's low bits).
    #[test]
    fn wrap_is_mth_wrap_degrees() {
        for (d, wrapped) in [
            (-10.123456_f32, -10.123456_f32),
            (190.0, -170.0),
            (-190.0, 170.0),
            (540.25, 180.25 - 360.0),
            (-0.0001, -0.0001),
        ] {
            assert_eq!(wrap(d), wrapped, "{d}");
        }
        assert_eq!(wrap64(-10.123456789), -10.123456789);
    }
    /// `EnderDragon.tickDeath` roars (`globalLevelEvent(1028)`) only when
    /// `!isSilent()`; `Silent` is saved only when set, as a byte.
    #[test]
    fn a_dragon_is_silent_only_with_a_set_silent_byte() {
        use sunrise_core::nbt::{Compound, Tag};
        let with = |v: Option<i8>| {
            let mut map = Compound::new();
            if let Some(b) = v {
                map.insert("Silent".into(), Tag::Byte(b));
            }
            Tag::Compound(map)
        };
        assert!(!super::saved_silent(None));
        assert!(!super::saved_silent(Some(&with(None))));
        assert!(!super::saved_silent(Some(&with(Some(0)))));
        assert!(super::saved_silent(Some(&with(Some(1)))));
    }

    #[test]
    fn breath_lifetime_excludes_the_wait() {
        assert!(!breath_expired(219, 200));
        assert!(breath_expired(220, 200));
        assert!(!breath_expired(619, 600));
        assert!(breath_expired(620, 600));
    }

    #[test]
    fn breath_victims_cadence() {
        let (a, b) = (FightWho::Host, FightWho::Guest(1));
        let mut v = Vec::new();
        assert!(breath_victims(15, &mut v, [a]).is_empty());
        assert_eq!(breath_victims(20, &mut v, [a]), vec![a]);
        assert!(breath_victims(25, &mut v, [a]).is_empty());
        // A newcomer is hit at the next 5-tick check, not the next 20.
        assert_eq!(breath_victims(30, &mut v, [a, b]), vec![b]);
        assert!(breath_victims(35, &mut v, [a, b]).is_empty());
        assert_eq!(breath_victims(40, &mut v, [a, b]), vec![a]);
        assert_eq!(breath_victims(50, &mut v, [a, b]), vec![b]);
    }

    #[test]
    fn instant_damage_at_cloud_scale() {
        assert_eq!(harm_amount(0), 3.0);
        assert_eq!(harm_amount(1), 6.0);
    }

    #[test]
    fn cloud_box_is_half_a_block_tall_from_its_floor() {
        let at = [0.0, 64.0, 0.0];
        let hit = |feet: [f64; 3], r: f32| {
            let (b, p) = player_box(feet);
            cloud_hits(at, r, b, p)
        };
        assert!(hit([0.0, 64.0, 0.0], 3.0));
        // Feet above the 0.5 box: the old |dy| < 1 test hit here.
        assert!(!hit([0.0, 64.6, 0.0], 3.0));
        // Feet below, head reaching into the box.
        assert!(hit([0.0, 63.0, 0.0], 3.0));
        // Box overlaps but the centre is outside the radius.
        assert!(!hit([3.2, 64.0, 0.0], 3.0));
        assert!(hit([3.0, 64.0, 0.0], 3.0));
    }

    #[test]
    fn fireball_cloud_snaps_to_a_player_within_four() {
        let (b, p) = player_box([2.0, 70.0, 1.0]);
        assert_eq!(
            fireball_cloud_origin([0.0, 70.0, 0.0], &[(b, p)]),
            [2.0, 70.0, 1.0]
        );
        let (b, p) = player_box([4.5, 70.0, 0.0]);
        assert_eq!(
            fireball_cloud_origin([0.0, 70.0, 0.0], &[(b, p)]),
            [0.0, 70.0, 0.0]
        );
    }

    #[test]
    fn dying_dragons_keep_their_flight_pose_between_frames() {
        let mut dragon = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 432);
        dragon.history.fill((70.0, 170.0));
        dragon.position.y = 90.0;
        dragon.yaw = -170.0;
        dragon.record();
        assert_eq!(dragon.sample_lerp(0, 0.5), (80.0, 180.0));
        dragon.health = 0.0;
        for partial in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(dragon.sample_lerp(0, partial), (70.0, 170.0));
        }
    }

    /// Driven by `tools/test_dragon_render_parity.py`; compares the actual
    /// ray vertices with EnderDragonRenderer's captured VertexConsumer output.
    #[test]
    #[ignore]
    fn dragon_render_parity_fixture() {
        run_dragon_render_parity_fixture();
    }

    pub(in crate::game) fn run_dragon_render_parity_fixture() {
        let path = std::env::var("SUNRISE_DRAGON_RENDER_OUTPUT").expect("harness output path");
        let mut cases = Vec::new();
        for t in [0.0, 0.05, 0.25, 0.5, 0.8, 0.9, 0.95, 1.0] {
            let mut vertices = Vec::new();
            dragon_rays(
                glam::Mat4::IDENTITY,
                t,
                sunrise_client::hud::Rect {
                    u0: 0.0,
                    v0: 0.0,
                    u1: 1.0,
                    v1: 1.0,
                    w: 1,
                    h: 1,
                },
                &mut vertices,
            );
            // The fourth quad vertex only pads a triangle for our index buffer.
            let triangles: Vec<_> = vertices
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|q| &q[..3])
                .map(|v| serde_json::json!([v.pos[0], v.pos[1], v.pos[2], v.color]))
                .collect();
            cases.push(triangles);
        }
        std::fs::write(path, serde_json::to_vec(&cases).unwrap()).unwrap();
    }

    #[test]
    fn the_dying_model_reads_the_mask_where_it_reads_the_skin() {
        // The skin in the atlas's top-left quarter, the mask in its bottom-right one.
        let mut quad = [Vertex {
            pos: [0.0; 3],
            uv: [0.25, 0.125],
            layer: 0,
            color: 0,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        }];
        dissolve_mask(&mut quad, [0.0, 0.0, 0.5, 0.5], [0.5, 0.5, 1.0, 1.0]);
        assert_eq!(quad[0].glint_uv, [0.75, 0.625]);
        // ARGB.white(1 - deathTime / 200): floor(127.5), and past 200 the int shift wraps.
        assert_eq!(dissolve_color(100.0), 0x7FFF_FFFF);
        assert_eq!(dissolve_color(200.5), 0xFFFF_FFFF);
    }

    #[test]
    fn death_rays_sample_inside_the_white_sprite() {
        // A four-texel white sprite surrounded by transparent atlas pixels.
        // A border UV can round outside it during perspective interpolation.
        let size = 1024.0;
        let white = sunrise_client::hud::Rect {
            u0: 20.0 / size,
            v0: 60.0 / size,
            u1: 24.0 / size,
            v1: 64.0 / size,
            w: 4,
            h: 4,
        };
        for t in [0.25, 0.8, 0.95, 1.0] {
            let mut vertices = Vec::new();
            dragon_rays(glam::Mat4::IDENTITY, t, white, &mut vertices);
            assert!(!vertices.is_empty());
            for v in &vertices {
                assert!(
                    v.uv[0] > white.u0 + 0.5 / size && v.uv[0] < white.u1 - 0.5 / size,
                    "death-ray U touches the atlas border: {}",
                    v.uv[0]
                );
                assert!(
                    v.uv[1] > white.v0 + 0.5 / size && v.uv[1] < white.v1 - 0.5 / size,
                    "death-ray V touches the atlas border: {}",
                    v.uv[1]
                );
            }
        }
    }

    /// The damage type of the hits below, which count.
    const HIT: Option<HurtSource> = Some(HurtSource {
        damage_type: "minecraft:player_attack",
        causing: 0,
        direct: 0,
        position: None,
        direct_type: None,
    });
    fn dragon(phase: Phase) -> Dragon {
        let mut d = Dragon::new(Vec3::new(0.0, 70.0, 0.0), 0.0, 1);
        d.phase = phase;
        d
    }

    /// `EnderDragon.readAdditionalSaveData` reads `sitting_damage_received`
    /// (`getFloatOr`, 0 when absent); the writer already saved it. Sunrise
    /// only wrote it, so a perched dragon that took a quarter of its health
    /// but was saved before taking off came back with the damage forgotten.
    #[test]
    fn a_perched_dragons_damage_survives_the_chunk_save() {
        let mut d = dragon(Phase::Flaming { ticks: 0 });
        d.sitting_damage = 12.5;
        let tag = d.to_nbt([1, 2, 3, 4], None, &Default::default());
        let back = Dragon::from_nbt(&tag, 0).unwrap();
        assert_eq!(back.sitting_damage, 12.5);
        // No key at all reads as no damage accumulated.
        let mut bare = tag.clone();
        if let sunrise_core::nbt::Tag::Compound(fields) = &mut bare {
            fields.remove(&"sitting_damage_received".into());
        }
        assert_eq!(Dragon::from_nbt(&bare, 0).unwrap().sitting_damage, 0.0);
    }
    fn flying() -> Phase {
        Phase::Holding {
            path: None,
            target: None,
            clockwise: false,
        }
    }

    /// `DragonLandingPhase.doClientTick`: eight puffs a tick from the head,
    /// along `getHeadLookVector(1)` (a quarter turn of it is thrown away in
    /// vanilla) plus the dragon's own motion.
    #[test]
    fn a_landing_dragon_breathes_eight_puffs_from_its_head() {
        let look = [0.0, -0.6, 0.8];
        let head = [10.0, 70.5, -3.0];
        let motion = [0.25, -0.05, 0.5];
        let mut random = LegacyRandom::new(7);
        let puffs = breath_puffs(
            &Phase::Landing { target: None },
            look,
            head,
            motion,
            &mut random,
        );
        assert_eq!(puffs.len(), 8);
        let speed = [
            -look[0] * f64::from(0.08_f32) + motion[0],
            -look[1] * f64::from(0.3_f32) + motion[1],
            -look[2] * f64::from(0.08_f32) + motion[2],
        ];
        for (at, v) in puffs {
            assert_eq!(v, speed);
            // The head's centre, jittered by `nextGaussian() / 2`.
            for a in 0..3 {
                assert!(
                    (at[a] - head[a]).abs() < 2.0,
                    "puff {} off the head: {at:?}",
                    a
                );
            }
        }
    }

    /// `DragonSittingFlamingPhase.doClientTick`: on the even ticks of the first
    /// ten, eight starts each breathe a fan of six puffs down the look line.
    #[test]
    fn a_flaming_dragon_breathes_a_fan_on_every_second_tick() {
        let look = [0.6, -0.0, 0.8];
        let head = [0.0, 65.5, 4.0];
        for (tick, want) in [(1, 0), (2, 48), (3, 0), (4, 48), (8, 48), (9, 0), (10, 0)] {
            let mut random = LegacyRandom::new(11);
            let puffs = breath_puffs(
                &Phase::Flaming { ticks: tick },
                look,
                head,
                [0.0; 3],
                &mut random,
            );
            assert_eq!(puffs.len(), want, "flame tick {tick}");
        }
        let mut random = LegacyRandom::new(11);
        let puffs = breath_puffs(
            &Phase::Flaming { ticks: 2 },
            look,
            head,
            [0.0; 3],
            &mut random,
        );
        for (i, (at, v)) in puffs.iter().enumerate() {
            let j = (i % 6) as f64;
            // Six puffs leave from each of the eight starts.
            assert_eq!(at, &puffs[i - i % 6].0);
            assert_eq!(
                *v,
                [
                    -look[0] * f64::from(0.08_f32) * j,
                    -look[1] * f64::from(0.6_f32),
                    -look[2] * f64::from(0.08_f32) * j,
                ]
            );
        }
    }

    /// No other phase has a `doClientTick` that breathes.
    #[test]
    fn only_the_landing_and_flaming_phases_breathe() {
        let mut random = LegacyRandom::new(3);
        for phase in [
            flying(),
            Phase::Scanning { time: 5 },
            Phase::Hover { target: None },
            Phase::Takeoff {
                first: false,
                path: None,
                target: None,
            },
            Phase::Attacking { ticks: 3 },
            Phase::Dying {
                target: None,
                time: 0,
            },
        ] {
            assert!(
                breath_puffs(&phase, [0.0, -1.0, 0.0], [0.0; 3], [0.0; 3], &mut random).is_empty(),
                "{phase:?}"
            );
        }
    }

    #[test]
    fn a_perched_dragon_shrugs_arrows_off() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // AbstractDragonSittingPhase.onHurt: an arrow does nothing, melee does.
        let mut d = dragon(Phase::Scanning { time: 0 });
        assert_eq!(d.hurt(level, 0, 9.0, HIT, true), Hurt::Shrugged);
        assert_eq!(d.health, MAX_HEALTH);
        assert_eq!(
            d.hurt(level, 0, 9.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 9.0);
        // In flight an arrow lands.
        let mut d = dragon(flying());
        assert_eq!(
            d.hurt(level, 0, 9.0, HIT, true),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 9.0);
    }

    /// `EnderDragon.knockback` calls `LivingEntity.knockback` only when the
    /// phase is not sitting. `dealDefaultKnockback` uses `0.4F` away from the
    /// attacker's feet. Resistance is 0 and `onGround` stays false
    /// (`noPhysics`), so from velocity `(1, -0.3, 0.5)` and a source 3 by 4
    /// blocks away the push is `(0.5 - 0.6p, -0.3, 0.25 - 0.8p)` with
    /// `p = (double) 0.4F`. A perched hit, including one that makes it take
    /// off, leaves the velocity alone. A hit inside the damage cooldown is
    /// not a full hit, so it does not push either.
    #[test]
    fn a_melee_hit_knocks_a_flying_dragon_and_not_a_perched_one() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let from = Vec3::new(3.0, 70.0, 4.0);
        let velocity = Vec3::new(1.0, -0.3, 0.5);
        let power = f64::from(0.4_f32);
        let pushed = Vec3::new(0.5 - 0.6 * power, -0.3, 0.25 - 0.8 * power);

        let mut perched = dragon(Phase::Scanning { time: 0 });
        perched.velocity = velocity;
        assert_eq!(
            perched.hurt_from(
                level,
                0,
                9.0,
                HIT,
                false,
                Some(Push {
                    from,
                    resistance: 0.0
                }),
                false
            ),
            Hurt::Landed { full: true }
        );
        assert_eq!(perched.velocity, velocity);

        let mut taking_off = dragon(Phase::Flaming { ticks: 0 });
        taking_off.velocity = velocity;
        taking_off.hurt_from(
            level,
            0,
            60.0,
            HIT,
            false,
            Some(Push {
                from,
                resistance: 0.0,
            }),
            false,
        );
        assert!(matches!(taking_off.phase, Phase::Takeoff { .. }));
        assert_eq!(taking_off.velocity, velocity);

        let mut flying = dragon(flying());
        flying.velocity = velocity;
        assert_eq!(
            flying.hurt_from(
                level,
                0,
                9.0,
                HIT,
                false,
                Some(Push {
                    from,
                    resistance: 0.0
                }),
                false
            ),
            Hurt::Landed { full: true }
        );
        assert_eq!(flying.velocity, pushed);
        assert_eq!(
            flying.hurt_from(
                level,
                0,
                20.0,
                HIT,
                false,
                Some(Push {
                    from,
                    resistance: 0.0
                }),
                false
            ),
            Hurt::Landed { full: false }
        );
        assert_eq!(flying.velocity, pushed);
    }

    #[test]
    fn a_part_refuses_fire() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // EnderDragonPart.hurtServer: the dragon is fire immune, so a
        // fireball's hit is refused on the head and on the other parts.
        let large = ProjectileKind::LargeFireball { power: 1 };
        for part in [0, 2] {
            let mut d = dragon(flying());
            let hurt = d.hurt_part(level, part, 6.0, HIT, DamageKind::Fireball, Some(&large));
            assert_eq!(hurt, Hurt::Refused);
            assert_eq!(d.health, MAX_HEALTH);
        }
        let small = ProjectileKind::SmallFireball;
        let mut d = dragon(flying());
        let hurt = d.hurt_part(level, 0, 5.0, HIT, DamageKind::Fireball, Some(&small));
        assert_eq!(hurt, Hurt::Refused);
        assert_eq!(d.health, MAX_HEALTH);
    }

    #[test]
    fn a_perched_dragon_shrugs_a_players_wind_charge_off() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // AbstractDragonSittingPhase.onHurt: a direct WindCharge does
        // nothing, a breeze's BreezeWindCharge is not one.
        let charge = ProjectileKind::WindCharge;
        let mut d = dragon(Phase::Scanning { time: 0 });
        let hurt = d.hurt_part(level, 0, 1.0, HIT, DamageKind::WindCharge, Some(&charge));
        assert_eq!(hurt, Hurt::Shrugged);
        assert_eq!(d.health, MAX_HEALTH);
        let breeze = ProjectileKind::BreezeWindCharge;
        let hurt = d.hurt_part(level, 0, 1.0, HIT, DamageKind::WindCharge, Some(&breeze));
        assert_eq!(hurt, Hurt::Landed { full: true });
        assert_eq!(d.health, MAX_HEALTH - 1.0);
        // In flight the player's charge lands too.
        let mut d = dragon(flying());
        let hurt = d.hurt_part(level, 0, 1.0, HIT, DamageKind::WindCharge, Some(&charge));
        assert_eq!(hurt, Hurt::Landed { full: true });
        assert_eq!(d.health, MAX_HEALTH - 1.0);
    }

    #[test]
    fn only_a_player_owned_wither_skull_takes_the_dragons_health() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // WitherSkull.onHitEntity: EnderDragon.hurt takes the wither's skull
        // (not a player, not an explosion) but takes no health, even perched.
        let skull = ProjectileKind::WitherSkull { dangerous: false };
        let wither = super::super::wither::skull_source(Some(Who::Mob(7)), [0.0; 3]);
        assert_eq!((wither.amount, wither.kind), (8.0, DamageKind::WitherSkull));
        let mut d = dragon(Phase::Scanning { time: 0 });
        let hurt = d.hurt_part(level, 2, wither.amount, None, wither.kind, Some(&skull));
        assert_eq!(hurt, Hurt::Landed { full: false });
        assert_eq!(d.health, MAX_HEALTH);
        // A player's skull: 8 on the head, 8 / 4 + 1 on the body.
        let player = super::super::wither::skull_source(Some(Who::Player), [0.0; 3]);
        let mut d = dragon(Phase::Scanning { time: 0 });
        let hurt = d.hurt_part(level, 0, player.amount, HIT, player.kind, Some(&skull));
        assert_eq!(hurt, Hurt::Landed { full: true });
        assert_eq!(d.health, MAX_HEALTH - 8.0);
        let mut d = dragon(flying());
        d.hurt_part(level, 2, player.amount, HIT, player.kind, Some(&skull));
        assert_eq!(d.health, MAX_HEALTH - 3.0);
    }

    #[test]
    fn the_cooldown_lets_only_the_excess_through() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // LivingEntity.hurtServer: damageCooldownTime > 10 keeps lastHurt.
        let mut d = dragon(flying());
        assert_eq!(
            d.hurt(level, 0, 6.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(
            d.hurt(level, 0, 4.0, HIT, false),
            Hurt::Landed { full: false }
        );
        assert_eq!(d.health, MAX_HEALTH - 6.0);
        assert_eq!(
            d.hurt(level, 0, 10.0, HIT, false),
            Hurt::Landed { full: false }
        );
        assert_eq!(d.health, MAX_HEALTH - 10.0);
        // Ten ticks on, the window is over: a full hit again.
        d.invulnerable = 10;
        assert_eq!(
            d.hurt(level, 0, 3.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 13.0);
    }

    #[test]
    fn parts_and_sources() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        // Neither head nor neck: a quarter plus one.
        let mut d = dragon(flying());
        assert_eq!(
            d.hurt(level, 2, 20.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 6.0);
        // Not a player's and not an explosion: true, but no damage.
        let mut d = dragon(flying());
        assert_eq!(
            d.hurt(level, 0, 20.0, None, false),
            Hurt::Landed { full: false }
        );
        assert_eq!(d.health, MAX_HEALTH);
        // Too little after the part's cut, and nothing while dying.
        assert_eq!(d.hurt(level, 3, 0.001, HIT, false), Hurt::Refused);
        d.phase = Phase::Dying {
            target: None,
            time: 0,
        };
        assert_eq!(d.hurt(level, 0, 20.0, HIT, false), Hurt::Refused);
    }

    /// The dragon hits the hook of these tests saw: entity, damage type,
    /// causing and direct entities, amount.
    type Seen = (String, i32, i32, f32);
    static SEEN: std::sync::Mutex<Vec<(i32, Seen)>> = std::sync::Mutex::new(Vec::new());
    fn seen(entity: i32) -> Vec<Seen> {
        let seen = SEEN.lock().unwrap();
        seen.iter()
            .filter(|s| s.0 == entity)
            .map(|s| s.1.clone())
            .collect()
    }
    /// Sets the native damage hook of these tests: it records the dragon
    /// hits of entities 9001 to 9005, cancels 9001's and doubles 9002's.
    fn hooks() -> sunrise_client::health::DamageHooks {
        use sunrise_client::health::{DamageContainer, DamageHooks, DamageLevel, Hurting, Victim};
        fn hook(
            _: DamageLevel<'_>,
            hurting: Hurting<'_>,
            _: Victim,
            container: &mut DamageContainer,
        ) -> bool {
            let Hurting::Dragon(entity @ 9001..=9005, s) = hurting else {
                return false;
            };
            let amount = container.new_damage();
            let hit = (s.damage_type.to_owned(), s.causing, s.direct, amount);
            SEEN.lock().unwrap().push((entity, hit));
            if entity == 9002 {
                container.record_new_damage(amount * 2.0);
            }
            entity == 9001
        }
        DamageHooks {
            incoming: Some(hook),
            ..DamageHooks::NONE
        }
    }
    #[test]
    fn base_check_only_receives_qualifying_dragon_hits() {
        let mut d = dragon(flying());
        assert!(d.reaches_base_check(0, 1.0, true, true));
        assert!(!d.reaches_base_check(0, 1.0, false, false));
        assert!(!d.reaches_base_check(BODY, 0.001, true, false));
        d.set_phase(Phase::Dying {
            target: None,
            time: 0,
        });
        assert!(!d.reaches_base_check(0, 1.0, true, false));
    }

    #[test]
    #[ignore = "daily"]
    fn carry_dragon_dispatches_only_after_phase_and_source_guards() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Probe(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Probe {
            fn entity_invulnerability_check(
                &self,
                _: mod_api::LevelSide<'_>,
                _: &mut mod_api::EntityInvulnerabilityCheckEvent<'_>,
            ) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let heard = Arc::new(AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "probe",
            Box::new(Probe(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.dragon.replace(dragon(Phase::Scanning { time: 0 }));
                let arrow = ProjectileKind::Trident;
                let hit = |game: &mut Game<'_>, counts, direct| {
                    game.hurt_dragon_part_at(
                        0,
                        2.0,
                        counts,
                        None,
                        DamageKind::Generic,
                        DragonHitOrigin {
                            direct,
                            entity_id: None,
                            from: None,
                            invulnerability_checked: false,
                            hook_source: None,
                            attack: None,
                            hooks: DragonHitHooks::capture(),
                        },
                    )
                };
                assert_eq!(hit(game, true, Some(&arrow)), Hurt::Shrugged);
                assert_eq!(heard.load(Ordering::SeqCst), 0);
                game.dragon.replace(dragon(flying()));
                let id = game.dragon.as_ref().unwrap().id;
                assert!(game.command_damage_dragon(
                    id,
                    2.0,
                    "minecraft:generic",
                    DamageKind::Generic,
                    (None, None, None)
                ));
                assert_eq!(heard.load(Ordering::SeqCst), 0);
                assert!(matches!(hit(game, false, None), Hurt::Landed { .. }));
                assert_eq!(heard.load(Ordering::SeqCst), 0);
                hit(game, true, None);
                assert_eq!(heard.load(Ordering::SeqCst), 1);
            });
        });
    }

    /// `dealDefaultKnockback` reads the dragon's `getAttributeValue(KNOCKBACK_RESISTANCE)` at the
    /// hit: a netherite chestplate's `0.1F` (`ArmorMaterials.NETHERITE`, widened to the
    /// modifier's double) shortens the push to `0.4F * (1 - 0.1F)`.
    #[test]
    #[ignore = "daily"]
    fn a_hit_on_the_dragon_pushes_by_its_live_knockback_resistance() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let mut d = dragon(flying());
            d.velocity = Vec3::new(1.0, -0.3, 0.5);
            d.equipment.set(
                sunrise_items::equipment::CHEST,
                Some(sunrise_client::items::ItemStack::new(
                    "minecraft:netherite_chestplate",
                    1,
                )),
            );
            game.dragon.replace(d);
            game.refresh_stand_and_dragon_equipment_attributes();
            let resistance = f64::from(0.1_f32);
            assert_eq!(game.dragon_knockback_resistance(), resistance);
            let from = Vec3::new(3.0, 70.0, 4.0);
            let hurt = game.hurt_dragon_part_at(
                0,
                9.0,
                true,
                None,
                DamageKind::Generic,
                DragonHitOrigin {
                    direct: None,
                    entity_id: None,
                    from: Some(from),
                    invulnerability_checked: false,
                    hook_source: None,
                    attack: None,
                    hooks: DragonHitHooks::capture(),
                },
            );
            assert_eq!(hurt, Hurt::Landed { full: true });
            let power = f64::from(0.4_f32) * (1.0 - resistance);
            assert_eq!(
                game.dragon.as_ref().unwrap().velocity,
                Vec3::new(0.5 - 0.6 * power, -0.3, 0.25 - 0.8 * power)
            );
        });
    }

    fn dragon_as(entity: u32) -> Dragon {
        let mut d = dragon(flying());
        d.id = entity;
        d
    }

    #[test]
    fn dragon_invulnerability_hook_blocks_the_matching_damage_type() {
        use sunrise_client::health::{DamageHooks, DamageLevel, Hurting};
        fn hook(_: DamageLevel<'_>, hurting: Hurting<'_>, vanilla: bool) -> bool {
            match hurting {
                Hurting::Dragon(9100, source) => source.damage_type == "minecraft:arrow",
                _ => vanilla,
            }
        }
        let hooks = DamageHooks {
            invulnerable: Some(hook),
            ..DamageHooks::NONE
        };
        let mut test_level = sunrise_client::health::TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();
        let mut d = dragon_as(9100);
        let arrow = HurtSource {
            damage_type: "minecraft:arrow",
            ..HIT.unwrap()
        };
        let generic = HurtSource {
            damage_type: "minecraft:generic",
            ..HIT.unwrap()
        };
        assert_eq!(
            d.hurt(level, 0, 4.0, Some(arrow), false),
            Hurt::Landed { full: false }
        );
        assert_eq!(d.health, MAX_HEALTH);
        assert_eq!(
            d.hurt(level, 0, 4.0, Some(generic), false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 4.0);
    }

    #[test]
    fn dragon_part_respects_a_hook_override_of_fire_immunity() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut d = dragon(flying());
        assert_eq!(
            d.hurt_part_from(level, 0, 4.0, HIT, DamageKind::OnFire, None, None, false),
            Hurt::Refused
        );
        assert_eq!(
            d.hurt_part_from(level, 0, 4.0, HIT, DamageKind::OnFire, None, None, true),
            Hurt::Landed { full: true }
        );
        assert_eq!(d.health, MAX_HEALTH - 4.0);
    }

    #[test]
    fn the_native_damage_hook_changes_or_cancels_a_dragons_hit() {
        let mut test_level = sunrise_client::health::TestLevel::with_hooks(0, hooks());
        let level = &mut test_level.level();
        // EnderDragon.hurt returns true whatever reallyHurt did. A canceled
        // hit takes no health, starts no cooldown and isn't full: what
        // hurt_dragon_part reads to credit the player and play the hurt sound.
        let mut d = dragon_as(9001);
        let arrow = HurtSource {
            damage_type: "minecraft:arrow",
            causing: 12,
            direct: 0,
            position: None,
            direct_type: None,
        };
        let hurt = d.hurt(level, 0, 9.0, Some(arrow), false);
        assert_eq!(hurt, Hurt::Landed { full: false });
        assert_eq!(seen(9001), [("minecraft:arrow".to_owned(), 12, 0, 9.0)]);
        assert_eq!(d.health, MAX_HEALTH);
        assert_eq!((d.invulnerable, d.last_hurt, d.hurt_time), (0, 0.0, 0));
        // The hook sees the amount after the part's cut (20 / 4 + 1).
        let mut d = dragon_as(9002);
        assert_eq!(
            d.hurt(level, 2, 20.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(
            seen(9002),
            [("minecraft:player_attack".to_owned(), 0, 0, 6.0)]
        );
        assert_eq!(d.health, MAX_HEALTH - 12.0);
        // Neither a hit that doesn't count nor one cut below 0.01 reaches
        // reallyHurt.
        let mut d = dragon_as(9005);
        assert_eq!(
            d.hurt(level, 0, 9.0, None, false),
            Hurt::Landed { full: false }
        );
        assert_eq!(d.hurt(level, 3, 0.001, HIT, false), Hurt::Refused);
        assert!(seen(9005).is_empty());
    }

    #[test]
    fn dragon_records_shield_damage_and_runs_armor_and_absorption_modifiers() {
        use std::sync::Mutex;
        use sunrise_client::health::{
            DamageContainer, DamageHooks, DamageLevel, Hurting, Reduction, ShieldBlock, Victim,
        };
        static ARMOR_SEEN: Mutex<Vec<f32>> = Mutex::new(Vec::new());
        static ABSORPTION_SEEN: Mutex<Vec<f32>> = Mutex::new(Vec::new());
        static POST_SEEN: Mutex<Vec<(f32, f32, f32, f32, f32, f32)>> = Mutex::new(Vec::new());
        fn on_shield(
            _: DamageLevel<'_>,
            hurting: Hurting<'_>,
            _: f32,
            _: bool,
            block: &mut ShieldBlock,
        ) {
            if matches!(hurting, Hurting::Dragon(9201 | 9202, _)) {
                block.blocked = true;
                block.blocked_damage = 2.0;
                block.shield_damage = 5;
            }
        }
        fn on_incoming(
            _: DamageLevel<'_>,
            hurting: Hurting<'_>,
            _: Victim,
            container: &mut DamageContainer,
        ) -> bool {
            if matches!(hurting, Hurting::Dragon(9201 | 9202, _)) {
                container.add_reduction_modifier(
                    Reduction::Armor,
                    Box::new(|_, taken| {
                        ARMOR_SEEN.lock().unwrap().push(taken);
                        1.0
                    }),
                );
            }
            false
        }
        fn on_pre(_: DamageLevel<'_>, hurting: Hurting<'_>, container: &mut DamageContainer) {
            let absorb_to = match hurting {
                Hurting::Dragon(9201, _) => 3.0,
                Hurting::Dragon(9202, _) => 10.0,
                _ => return,
            };
            container.add_reduction_modifier(
                Reduction::Absorption,
                Box::new(move |_, taken| {
                    ABSORPTION_SEEN.lock().unwrap().push(taken);
                    absorb_to
                }),
            );
        }
        fn on_post(_: DamageLevel<'_>, hurting: Hurting<'_>, container: &DamageContainer) {
            if matches!(hurting, Hurting::Dragon(9201 | 9202, _)) {
                POST_SEEN.lock().unwrap().push((
                    container.blocked(),
                    container.shield_damage(),
                    container.reduction(Reduction::Armor),
                    container.reduction(Reduction::Absorption),
                    container.inflicted(),
                    container.health_damage(),
                ));
            }
        }
        ARMOR_SEEN.lock().unwrap().clear();
        ABSORPTION_SEEN.lock().unwrap().clear();
        POST_SEEN.lock().unwrap().clear();
        let hooks = DamageHooks {
            incoming: Some(on_incoming),
            pre: Some(on_pre),
            post: Some(on_post),
            shield: Some(on_shield),
            ..DamageHooks::NONE
        };
        let mut test_level = sunrise_client::health::TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();

        // Hit 1: 10.0 - 2.0 shield = 8.0, Armor modifier takes 1.0 -> 7.0 inflicted,
        // Pre Absorption modifier receives 0.0 and takes 3.0 -> 4.0 health damage.
        let mut d1 = dragon_as(9201);
        assert_eq!(
            d1.hurt(level, 0, 10.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d1.health, MAX_HEALTH - 4.0);

        // Hit 2: Pre Absorption modifier absorbs all 7.0 -> 0.0 health damage.
        let mut d2 = dragon_as(9202);
        assert_eq!(
            d2.hurt(level, 0, 10.0, HIT, false),
            Hurt::Landed { full: true }
        );
        assert_eq!(d2.health, MAX_HEALTH);

        assert_eq!(*ARMOR_SEEN.lock().unwrap(), [0.0, 0.0]);
        assert_eq!(*ABSORPTION_SEEN.lock().unwrap(), [0.0, 0.0]);
        assert_eq!(
            *POST_SEEN.lock().unwrap(),
            [
                (2.0, 5.0, 1.0, 3.0, 7.0, 4.0),
                (2.0, 5.0, 1.0, 10.0, 7.0, 0.0),
            ]
        );
    }

    #[test]
    fn a_hit_names_the_player_behind_it() {
        let source = |kind, by, direct| {
            let s = super::hurt_source(kind, by, direct);
            (s.damage_type, s.causing, s.direct)
        };
        let melee = source(DamageKind::PlayerAttack, Some(12), None);
        assert_eq!(melee, ("minecraft:player_attack", 12, 12));
        let mut arrow = Projectile::new(ProjectileKind::Trident, Vec3::ZERO, None);
        arrow.id = 37;
        let shot = source(DamageKind::Arrow, Some(12), Some(arrow.id as i32));
        assert_eq!(shot, ("minecraft:arrow", 12, 37));
        let skull = source(DamageKind::WitherSkull, Some(12), Some(arrow.id as i32));
        assert_eq!(skull, ("minecraft:wither_skull", 12, 37));
        // DamageSources.magic(): an ownerless skull names no direct entity.
        let magic = source(DamageKind::Magic, None, Some(arrow.id as i32));
        assert_eq!(magic, ("minecraft:magic", 0, 0));
        let blast = source(DamageKind::Explosion, Some(12), None);
        assert_eq!(blast, ("minecraft:player_explosion", 12, 0));
        let blast = source(DamageKind::Explosion, None, None);
        assert_eq!(blast, ("minecraft:explosion", 0, 0));
    }

    #[test]
    fn a_quarter_of_its_health_while_perched_makes_it_take_off() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut d = dragon(Phase::Flaming { ticks: 0 });
        d.hurt(level, 0, 30.0, HIT, false);
        assert!(d.phase.sitting());
        d.invulnerable = 0;
        d.hurt(level, 0, 21.0, HIT, false);
        assert!(matches!(d.phase, Phase::Takeoff { .. }));
        assert_eq!(d.sitting_damage, 0.0);
    }

    #[test]
    fn an_explosion_pushes_the_dragon_along_the_blast() {
        let mut d = dragon(flying());
        d.position = Vec3::new(0.0, 70.0, 0.0);
        d.velocity = Vec3::ZERO;
        // Power 5 right under the dragon: it is thrown up, away from the blast.
        let push = d.explosion_knockback([0.0, 66.0, 0.0], 5.0, 1.0).unwrap();
        assert!(push[1] > 0.0 && push[0] == 0.0 && push[2] == 0.0);
        // Out of reach or fully covered: no push.
        assert_eq!(d.explosion_knockback([0.0, 40.0, 0.0], 5.0, 1.0), None);
        assert_eq!(
            d.explosion_knockback([0.0, 66.0, 0.0], 5.0, 0.0),
            Some([0.0; 3])
        );
    }

    #[test]
    fn explosions_hit_each_part_by_its_exposure() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut d = dragon(flying());
        for (i, part) in d.parts.iter_mut().enumerate() {
            part.0 = Vec3::new(i as f64 * 4.0, 70.0, 0.0);
        }
        // A power-6 blast at the dragon's feet: diameter 12.
        let center = [0.0, 70.0, 0.0];
        let open = d.explosion_hits(center, 6.0, |_| 1.0);
        // The dragon itself (its body), then the parts within 12 blocks.
        let parts: Vec<usize> = open.iter().map(|h| h.0).collect();
        assert_eq!(parts, vec![2, 0, 1, 2, 3]);
        // At the centre: (1 + 1) / 2 * 7 * 12 + 1.
        assert_eq!(open[0].1, 85.0);
        // Behind cover (no ray reaches the box) only the 1 is left.
        let covered = d.explosion_hits(center, 6.0, |_| 0.0);
        assert!(covered.iter().all(|h| h.1 == 1.0));
        // Through the cooldown the strongest hit after the part's cut wins.
        for (part, amount) in open {
            d.hurt(level, part, amount, HIT, false);
        }
        assert_eq!(d.health, MAX_HEALTH - 85.0);
    }

    fn player(who: FightWho, x: f64, creative: bool, spectator: bool) -> FightPlayer {
        let position = Vec3::new(x, 64.0, 0.0);
        FightPlayer {
            who,
            position,
            bounds: Aabb::new(position, Vec3::new(x + 0.6, 65.8, 0.6)),
            alive: true,
            creative,
            spectator,
        }
    }

    #[test]
    fn the_first_crystal_destroyed_in_a_tick_names_the_strafed_player() {
        // DragonHoldingPatternPhase.onCrystalDestroyed strafes at once, and
        // the strafing phase ignores the next crystal.
        let (host, guest) = (
            player(FightWho::Host, 1.0, false, false),
            player(FightWho::Guest(8), 2.0, false, false),
        );
        let mut d = dragon(flying());
        d.strafe_after_crystal(Some(&host));
        d.strafe_after_crystal(Some(&guest));
        assert_eq!(d.strafe_request, Some(FightWho::Host));
        // Nobody, or a player it can't attack, leaves the next crystal free.
        let creative = player(FightWho::Host, 1.0, true, false);
        let mut d = dragon(flying());
        d.strafe_after_crystal(None);
        d.strafe_after_crystal(Some(&creative));
        d.strafe_after_crystal(Some(&guest));
        assert_eq!(d.strafe_request, Some(FightWho::Guest(8)));
        // Only while it holds its pattern.
        let mut d = dragon(Phase::Scanning { time: 0 });
        d.strafe_after_crystal(Some(&guest));
        assert_eq!(d.strafe_request, None);
    }

    #[test]
    fn the_dragon_targets_any_player_it_can_attack() {
        let at = Vec3::new(0.0, 64.0, 0.0);
        // A spectating host and a creative guest are never targets: the
        // survival guest further away is.
        let players = [
            player(FightWho::Host, 1.0, false, true),
            player(FightWho::Guest(7), 2.0, true, false),
            player(FightWho::Guest(8), 30.0, false, false),
        ];
        let t = nearest_enemy(&players, false, at, None, |_| true);
        assert_eq!(t.map(|p| p.who), Some(FightWho::Guest(8)));
        // Out of a 20-block range, or filtered out, or on Peaceful: none.
        assert!(nearest_enemy(&players, false, at, Some((at, 20.0)), |_| true).is_none());
        assert!(nearest_enemy(&players, false, at, None, |p| p.position.x < 10.0).is_none());
        assert!(nearest_enemy(&players, true, at, None, |_| true).is_none());
        // The nearest of several, measured from where it looks (the egg).
        let players = [
            player(FightWho::Host, 5.0, false, false),
            player(FightWho::Guest(8), -3.0, false, false),
        ];
        let t = nearest_enemy(&players, false, at, None, |_| true);
        assert_eq!(t.map(|p| p.who), Some(FightWho::Guest(8)));
        let t = nearest_enemy(&players, false, Vec3::new(9.0, 64.0, 0.0), None, |_| true);
        assert_eq!(t.map(|p| p.who), Some(FightWho::Host));
    }
    #[test]
    fn a_landed_hurt_sets_the_dragon_hurt_flash_time() {
        let mut test_level = sunrise_client::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut d = dragon_as(42);
        assert_eq!(d.hurt_time, 0);
        let hurt = d.hurt(level, 0, 5.0, HIT, false);
        assert_eq!(hurt, Hurt::Landed { full: true });
        assert_eq!(d.hurt_time, 10);
    }

    #[test]
    fn the_strafe_firing_angle_matches_vanillas_floats() {
        // `(float)(Math.acos(dot) * (180.0F / (float)Math.PI)) + 0.5F` on a JDK: dots at
        // and around cos(10°) cross the [0, 10) gate on single ulps.
        for (dot, angle) in [
            (0x3f80_0000, 0x3f00_0000), // 1.0 -> 0.5
            (0x3f7f_be77, 0x4044_00b1), // 0.999 -> 3.0625422
            (0x3f7c_1c5f, 0x4127_ffca), // over the gate -> 10.4999485
            (0x3f7c_1c5d, 0x4127_fff3), // under it -> 10.499988
            (0x3f7c_1c5c, 0x4128_0007), // -> 10.500007
            (0x3f7c_1bda, 0x4128_0a80), // -> 10.502563
            (0x3f7c_28f6, 0x4126_fb56), // -> 10.436361
            (0x3f7a_e148, 0x413f_a742), // 0.98 -> 11.978334
            (0x3f00_0000, 0x4271_ffff), // 0.5 -> 60.499996
            (0x0000_0000, 0x42b4_ffff), // 0.0 -> 90.49999
            (0xbf80_0000, 0x4334_7fff), // -1.0 -> 180.49998
        ] {
            assert_eq!(
                strafe_angle(f32::from_bits(dot)),
                f32::from_bits(angle),
                "dot {dot:#010x}"
            );
        }
        // Past 1.0 or NaN, `Math.acos` is NaN and the gate refuses, as vanilla's does.
        assert!(strafe_angle(f32::from_bits(0x3f80_0001)).is_nan());
        assert!(strafe_angle(f32::NAN).is_nan());
    }

    #[test]
    #[ignore = "daily"]
    fn a_stationary_no_ai_dragon_runs_its_living_body_turn() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.create_new_dragon();
            let dragon = game.dragon.as_mut().unwrap();
            assert_ne!(dragon.yaw, 0.0);
            assert_eq!((dragon.body_yaw, dragon.head_yaw), (0.0, 0.0));
            dragon.yaw = 35.0;
            dragon.no_ai = true;
            dragon.head_yaw = 90.0;
            let id = dragon.id;
            let entity = game.entity(id).unwrap();
            assert_eq!(entity.set_body_yaw(game, 0.0), Ok(true));
            game.tick_dragon();
            // The changed head is 90 degrees off the raw body. With a 75
            // degree limit the controller turns the body by 15, not the view.
            assert_eq!(entity.body_yaw(game), Some(15.0));
            assert_eq!(game.dragon.as_ref().unwrap().yaw, 35.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_tracked_dragon_lists_its_data_defaults() {
        use sunrise_protocol::entity_data::serializer;
        super::super::native_entity_refs::tests::with_game(|game| {
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 15;
            game.dragon.replace(dragon);
            let state = game.lan_dragon_state().unwrap();
            assert_eq!(
                state.defaults,
                vec![
                    (9, serializer::FLOAT, Value::Float(1.0)),
                    (15, serializer::BYTE, Value::Byte(0)),
                    (16, serializer::INT, Value::Int(10)),
                ]
            );
        });
    }
}

#[cfg(test)]
#[path = "dragon/fight_hooks_tests.rs"]
mod fight_hooks_tests;
#[cfg(test)]
#[path = "dragon/residency_tests.rs"]
mod residency_tests;
#[cfg(test)]
#[path = "dragon/thorns_tests.rs"]
mod thorns_tests;
#[cfg(test)]
#[path = "dragon/walls_tests.rs"]
mod walls_tests;

#[cfg(test)]
mod dragon_fireball_spawn_tests {
    include!("dragon/fireball_spawn_tests.rs");
}
#[cfg(test)]
#[path = "dragon/wing_push_tests.rs"]
mod wing_push_tests;
