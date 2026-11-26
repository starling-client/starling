//! A server's spectator camera (`ClientPacketListener.handleSetCamera`,
//! `Minecraft.cameraEntity`): `/spectate` or a spectator's click (sent as
//! `MultiPlayerGameMode.spectate`) attaches the view to another entity. The
//! view follows that entity while the player stands still and sends no moves
//! (`LocalPlayer.isControlledCamera`); the server keeps its player on the
//! camera and lets go when the player sneaks.
#[cfg(test)]
#[path = "tests/camera_mount_tests.rs"]
pub(in crate::game) mod camera_mount_tests;
#[cfg(test)]
#[path = "tests/remote_camera_tests.rs"]
pub(in crate::game) mod camera_tests;

use super::{Game, remote::Remote};
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use sunrise_client::motion::PlayerMotion;
use sunrise_client::{
    camera::{AvatarBob, CameraEntity, FovPlayer, HurtBob, LivingCamera, UseItem},
    effects::PlayerEffects,
    entities::{ArmorStand, ItemFrame, Painting},
    mobs::{
        Mob, Species,
        finalize::{self, Attr},
        render, villager,
    },
};
use sunrise_items::{
    equipment::{MAINHAND, OFFHAND},
    items::ItemStack,
};
use sunrise_world::collision_context::EntityContextKind;

/// Hurt and death timers carried by a living camera entity for `GameRenderer.bobHurt`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::game) struct CameraHurt {
    pub hurt_time: i32,
    pub hurt_duration: i32,
    pub hurt_dir: f32,
    pub death_time: Option<i32>,
}

impl CameraHurt {
    pub(in crate::game) fn bob(self, partial: f32) -> HurtBob {
        HurtBob {
            hurt_time: self.hurt_time as f32 - partial,
            hurt_duration: self.hurt_duration,
            hurt_dir: self.hurt_dir,
            death_time: self.death_time.map(|t| t as f32 + partial),
        }
    }
}

/// Fog and sky-blocking effect inputs owned by a living camera entity (`FogRenderer`, `Camera`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::game) struct CameraFogEffects {
    pub blindness: Option<(i32, bool)>,
    pub darkness_blend: Option<(f32, f32)>,
    pub fire_resistance: bool,
    pub night_vision: Option<i32>,
}

impl CameraFogEffects {
    pub(in crate::game) fn from_effects(
        effects: &PlayerEffects,
        darkness_blend: (f32, f32),
    ) -> Self {
        Self {
            blindness: effects
                .get("blindness")
                .map(|effect| (effect.duration, effect.duration == -1)),
            darkness_blend: effects.get("darkness").map(|_| darkness_blend),
            fire_resistance: effects.get("fire_resistance").is_some(),
            night_vision: effects.get("night_vision").map(|effect| effect.duration),
        }
    }
}

/// Snapshot of a detached watched player's HUD state retained after removal or replacement.
#[derive(Clone, Debug)]
pub(in crate::game) struct DetachedPlayerHud {
    pub main_hand_left: bool,
    pub main_hand: Option<ItemStack>,
    pub offhand: Option<ItemStack>,
    pub health: f32,
    pub max_health: f32,
    pub absorption: f32,
    pub armor: i32,
    pub air: i32,
    pub under_water: bool,
    pub frozen: bool,
    pub invulnerable: bool,
    pub regeneration: bool,
    pub poison: bool,
    pub wither: bool,
    pub hunger: bool,
}

/// Single resolved camera entity (`Minecraft.getCameraEntity`), live or detached.
#[derive(Clone, Debug)]
pub(in crate::game) struct ResolvedCameraEntity {
    pub entity_id: u32,
    /// Stored with the subject so a detached camera keeps its original identity.
    pub uuid: Option<u128>,
    pub camera: CameraEntity,
    pub is_local_player: bool,
    pub is_player: bool,
    pub may_build: bool,
    pub is_spectator: bool,
    pub is_sleeping: bool,
    pub blocks_sky: bool,
    pub fog_effects: CameraFogEffects,
    pub hurt: Option<CameraHurt>,
    pub avatar_bob: Option<AvatarBob>,
    pub fov_player: Option<FovPlayer>,
    pub has_player_passenger: bool,
    pub collision_kind: EntityContextKind,
    pub post_effect: Option<&'static str>,
    pub detached_hud: Option<DetachedPlayerHud>,
}

/// The entity `Hud.getCameraPlayer` finds: local player, live remote player, detached remote player, or none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::game) enum CameraPlayer {
    Own,
    Other(usize),
    Detached,
    Absent,
}

/// `Minecraft.setCameraEntity` from a ClientboundSetCameraPacket: the player's
/// own id puts the view back on the player; an id the level does not hold
/// changes nothing (`ClientboundSetCameraPacket.getEntity` is null).
pub(super) fn camera_after_packet(
    current: Option<i32>,
    player_id: i32,
    camera_id: i32,
    in_level: bool,
) -> Option<i32> {
    if camera_id == player_id {
        None
    } else if in_level {
        Some(camera_id)
    } else {
        current
    }
}

/// `LivingEntity.isSleeping` with `getBedOrientation` as its `toYRot`: a villager, player or
/// mannequin lying in a bed, whose facing `sleep_pose` keeps in `ints[1]`.
fn sleeping_view(m: &Mob) -> Option<Option<f32>> {
    if !(m.species() == Species::Villager || render::avatar(m)) || m.extra.ints[1] == 0 {
        return None;
    }
    Some(villager::bed_orientation(m).map(|d| match d {
        mod_api::Direction::South => 0.0,
        mod_api::Direction::West => 90.0,
        mod_api::Direction::North => 180.0,
        _ => 270.0,
    }))
}

/// `Camera.alignWithEntity`'s view of a mob or another player: its
/// interpolated position, `getViewYRot`/`getViewXRot` and eye height, with
/// its scale and camera distance for a detached view. A living entity's
/// `getViewYRot` is its head's yaw (`yHeadRot`), not its body's `yRot`.
pub(super) fn mob_camera_entity(m: &Mob, (scale, camera_distance): (f32, f64)) -> CameraEntity {
    CameraEntity {
        position: [m.position.x, m.position.y, m.position.z],
        previous: [m.previous.x, m.previous.y, m.previous.z],
        x_rot: m.head_pitch,
        y_rot: m.head_yaw,
        x_rot_o: m.previous_head_pitch,
        y_rot_o: m.previous_head_yaw,
        eye_height: m.eye_height() as f32,
        living: Some(LivingCamera {
            scale,
            camera_distance,
            sleeping: sleeping_view(m),
            dead: (m.health <= 0.0).then_some(m.death_time),
        }),
        mount: None,
    }
}

/// `Camera.alignWithEntity`'s view of an armor stand, the usual camera of a
/// server's cutscene or menu scene. `ArmorStand.setYHeadRot` keeps its head
/// on `yRot`, so its view turns with its yaw; its eyes are its dimensions'
/// (`getDimensionsMarker`): none for a marker, `BABY_DIMENSIONS`' 0.9875 for
/// a small stand, the type's 1.7775 otherwise.
#[cfg(test)]
pub(super) fn stand_camera_entity(a: &ArmorStand) -> CameraEntity {
    stand_camera_entity_with(a, (1.0, 4.0))
}

pub(super) fn stand_camera_entity_with(
    a: &ArmorStand,
    (scale, camera_distance): (f32, f64),
) -> CameraEntity {
    let eye_height = if a.marker {
        0.0
    } else {
        a.probe_dimensions().2 as f32 * scale
    };
    CameraEntity {
        position: [a.position.x, a.position.y, a.position.z],
        previous: [a.previous.x, a.previous.y, a.previous.z],
        x_rot: a.pitch,
        y_rot: a.yaw,
        x_rot_o: a.previous_pitch,
        y_rot_o: a.previous_yaw,
        eye_height,
        living: Some(LivingCamera {
            scale,
            camera_distance,
            sleeping: None,
            dead: None,
        }),
        mount: None,
    }
}

/// `Camera.alignWithEntity`'s view of a boat, which is not living (no
/// `getScale`, no camera distance): its interpolated position and `Entity`'s
/// view rotation, with `EntityType.BOAT`'s `eyeHeight(0.5625F)`.
pub(super) fn boat_camera_entity(b: &super::boats::Boat) -> CameraEntity {
    CameraEntity {
        position: [b.position.x, b.position.y, b.position.z],
        previous: [b.previous.x, b.previous.y, b.previous.z],
        x_rot: b.pitch,
        y_rot: b.yaw,
        x_rot_o: b.pitch,
        y_rot_o: b.previous_yaw,
        eye_height: 0.5625_f32,
        living: None,
        mount: None,
    }
}

/// `Camera.alignWithEntity`'s view of an item frame: a hanging entity sits at
/// the centre of its box, the block's centre pulled 0.46875 out of the wall
/// (`HangingEntity.recalculateBoundingBox` of `ItemFrame.createBoundingBox`),
/// does not move, and has `EntityType.ITEM_FRAME`'s zero eye height.
/// `ItemFrame.setDirection` turns it: `get2DDataValue() * 90` in yaw (south 0,
/// west 1, north 2, east 3), or `-90 * getAxisDirection().getStep()` in pitch
/// on the floor and ceiling.
pub(super) fn frame_camera_entity(f: &ItemFrame) -> CameraEntity {
    let step = f.step().map(f64::from);
    let at: [f64; 3] = std::array::from_fn(|i| f64::from(f.pos[i]) + 0.5 - step[i] * 0.46875);
    let (y_rot, x_rot) = match f.facing {
        0 => (0.0, 90.0),
        1 => (0.0, -90.0),
        2 => (180.0, 0.0),
        4 => (90.0, 0.0),
        5 => (270.0, 0.0),
        _ => (0.0, 0.0),
    };
    CameraEntity {
        position: at,
        previous: at,
        x_rot,
        y_rot,
        x_rot_o: x_rot,
        y_rot_o: y_rot,
        eye_height: 0.0,
        living: None,
        mount: None,
    }
}

/// `Camera.alignWithEntity`'s view of a painting: a hanging entity sits at the
/// centre of its box (`HangingEntity.recalculateBoundingBox`), faces
/// `get2DDataValue() * 90`, and keeps `EntityType.PAINTING`'s `0.425F` eyes.
pub(super) fn painting_camera_entity(p: &Painting) -> CameraEntity {
    let (lo, hi) = p.bounds();
    let at = std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5);
    let y_rot = f32::from(p.facing & 3) * 90.0;
    CameraEntity {
        position: at,
        previous: at,
        x_rot: 0.0,
        y_rot,
        x_rot_o: 0.0,
        y_rot_o: y_rot,
        eye_height: 0.425,
        living: None,
        mount: None,
    }
}

/// `Camera.alignWithEntity`'s view of primed TNT: its interpolated position
/// and `Entity`'s view rotation (priming never changes it), with
/// `EntityType.TNT`'s `eyeHeight(0.15F)`.
pub(super) fn tnt_camera_entity(t: &super::tnt::PrimedTnt) -> CameraEntity {
    CameraEntity {
        position: [t.position.x, t.position.y, t.position.z],
        previous: [t.previous.x, t.previous.y, t.previous.z],
        x_rot: t.rotation[1],
        y_rot: t.rotation[0],
        x_rot_o: t.rotation[1],
        y_rot_o: t.rotation[0],
        eye_height: 0.15,
        living: None,
        mount: None,
    }
}

/// `Camera.alignWithEntity`'s view of an `Interaction`: non-living, scalable dimensions.
pub(super) fn interaction_camera_entity(
    i: &super::interactions::RemoteInteraction,
) -> CameraEntity {
    CameraEntity {
        position: [i.position.x, i.position.y, i.position.z],
        previous: [i.previous.x, i.previous.y, i.previous.z],
        x_rot: i.pitch,
        y_rot: i.yaw,
        x_rot_o: i.previous_pitch,
        y_rot_o: i.previous_yaw,
        eye_height: i.eye_height(),
        living: None,
        mount: None,
    }
}

/// Fallback view for any other tracked level entity (`Level.getEntity`).
#[cfg(test)]
fn tracked_camera_entity(t: &super::remote::Tracked) -> CameraEntity {
    CameraEntity {
        position: t.position,
        previous: t.previous,
        x_rot: t.pitch,
        y_rot: t.yaw,
        x_rot_o: t.previous_pitch,
        y_rot_o: t.previous_yaw,
        eye_height: 0.0,
        living: None,
        mount: None,
    }
}

/// Unit-test fixture wrapping entity slices for pure `CameraEntity` view checks.
#[cfg(test)]
pub(super) struct CameraEntities<'a> {
    pub mobs: &'a [Mob],
    pub stands: &'a [ArmorStand],
    pub boats: &'a [super::boats::Boat],
    pub frames: &'a [ItemFrame],
    pub tnts: &'a [super::tnt::PrimedTnt],
    pub displays: &'a indexmap::IndexMap<i32, super::text_displays::RemoteTextDisplay>,
    pub interactions: &'a HashMap<i32, super::interactions::RemoteInteraction>,
    pub tracked: Option<&'a HashMap<i32, super::remote::Tracked>>,
    pub vehicles: &'a HashMap<u32, super::remote_vehicles::PassengerSeat>,
    pub player: Option<(u32, &'a PlayerMotion)>,
    pub living: Box<dyn Fn(&Mob) -> (f32, f64) + 'a>,
    pub stand_living: Box<dyn Fn(&ArmorStand) -> (f32, f64) + 'a>,
}

#[cfg(test)]
impl CameraEntities<'_> {
    pub(super) fn holds(&self, id: u32) -> bool {
        self.mobs.iter().any(|m| m.id == id)
            || self.stands.iter().any(|a| a.id == id)
            || self.boats.iter().any(|b| b.id == id)
            || self.frames.iter().any(|f| f.id == id)
            || self.tnts.iter().any(|t| t.id == id)
            || self.displays.contains_key(&(id as i32))
            || self.interactions.contains_key(&(id as i32))
            || self
                .tracked
                .is_some_and(|tracked| tracked.contains_key(&(id as i32)))
    }

    pub(super) fn view(&self, id: u32) -> Option<CameraEntity> {
        let mut entity = self.entity_view(id)?;
        entity.mount = self.living_vehicle(id);
        Some(entity)
    }

    fn living_vehicle(&self, id: u32) -> Option<(f32, f64)> {
        let vehicle = self.vehicles.get(&id)?.vehicle;
        if let Some(m) = self.mobs.iter().find(|m| m.id == vehicle) {
            return Some((self.living)(m));
        }
        if let Some(a) = self.stands.iter().find(|a| a.id == vehicle) {
            return Some((self.stand_living)(a));
        }
        let (_, p) = self.player.filter(|(player, _)| *player == vehicle)?;
        let attribute =
            |name: &str, default: f64| p.attributes.computed_value_of(name).unwrap_or(default);
        Some((
            attribute("minecraft:scale", 1.0) as f32,
            attribute("minecraft:camera_distance", 4.0),
        ))
    }

    fn entity_view(&self, id: u32) -> Option<CameraEntity> {
        if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            return Some(mob_camera_entity(m, (self.living)(m)));
        }
        if let Some(a) = self.stands.iter().find(|a| a.id == id) {
            return Some(stand_camera_entity_with(a, (self.stand_living)(a)));
        }
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            return Some(boat_camera_entity(b));
        }
        if let Some(f) = self.frames.iter().find(|f| f.id == id) {
            return Some(frame_camera_entity(f));
        }
        if let Some(t) = self.tnts.iter().find(|t| t.id == id) {
            return Some(tnt_camera_entity(t));
        }
        if let Some(d) = self.displays.get(&(id as i32)) {
            return Some(d.camera_entity());
        }
        if let Some(i) = self.interactions.get(&(id as i32)) {
            return Some(interaction_camera_entity(i));
        }
        self.tracked
            .and_then(|tracked| tracked.get(&(id as i32)))
            .map(tracked_camera_entity)
    }
}

/// `GameRenderer.checkEntityPostEffect`: the chain a spectated creeper, spider or
/// enderman puts on the screen, or one a mod registered for its own entity type
/// (`Hooks::register_entity_spectator_shaders`).
pub(super) fn entity_post_effect(kind: &str) -> Option<&'static str> {
    sunrise_client_render::spectator_shaders::chain(kind)
}

impl Game<'_> {
    /// True if this client holds a live entity with `id` in any runtime list or slot.
    fn holds_camera_entity(&self, id: u32) -> bool {
        self.dragon.as_ref().is_some_and(|d| d.id == id)
            || self.mob_index.find_ref(&self.mobs, id).is_some()
            || self.armor_stands.iter().any(|a| a.id == id)
            || self.interactions.contains_key(&(id as i32))
            || self.boats.iter().any(|b| b.id == id)
            || self.carts.iter().any(|c| c.id == id)
            || self.item_frames.iter().any(|f| f.id == id)
            || self.paintings.iter().any(|p| p.id == id)
            || self.tnts.iter().any(|t| t.id == id)
            || self.orbs.iter().any(|o| o.id == id)
            || self.creatures.projectiles.iter().any(|p| p.id == id)
            || self.tridents.iter().any(|t| t.id == id)
            || self.rockets.iter().any(|r| r.id == id)
            || self.dropped.iter().any(|d| d.id == id)
            || self.falling.iter().any(|f| f.id == id)
            || self.crystals.iter().any(|c| c.id == id)
            || self.text_displays.contains_key(&(id as i32))
            || self.hook.as_ref().is_some_and(|h| h.id() == id)
            || self.remote_hooks.iter().any(|h| h.id() == id)
            || self.potions.remote_thrown_pose(id).is_some()
            || self.end.remote_eye_pose(id).is_some()
    }

    /// `ClientPacketListener.handleSetCamera`.
    pub(in crate::game) fn remote_set_camera(&mut self, camera_id: i32) {
        let id = camera_id as u32;
        let in_level = self.remote.as_ref().is_some_and(|r| {
            self.holds_camera_entity(id)
                || r.general.rows.contains_key(&camera_id)
                || r.tracked_entity(camera_id).is_some()
        });
        let player = self
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == camera_id);
        if let Some(r) = self.remote.as_mut() {
            let next = camera_after_packet(r.camera, r.entity_id, camera_id, in_level);
            if in_level || player {
                r.camera = next;
                r.detached_camera = None;
            }
        }
        // Minecraft.setCameraEntity recomputes the spectated entity post effect in either perspective.
        if in_level || player {
            let _ = self.mob_index.find(&self.mobs, id);
            self.check_entity_post_effect(true);
        }
    }

    /// `GameRenderer.clearSpectatedEntityPostEffect`.
    pub(super) fn clear_spectated_entity_post_effect(&mut self) {
        self.spectated_post_effect = None;
        self.spectated_post_effect_active = false;
    }

    /// `GameRenderer.toggleSpectatorPostEffect` (`key.toggleSpectatorShaderEffects` / F4).
    pub(super) fn toggle_spectator_post_effect(&mut self) {
        self.spectated_post_effect_active = !self.spectated_post_effect_active;
    }

    /// `GameRenderer.checkEntityPostEffect` for the camera entity, or for none
    /// (`first_person` false: the third-person view clears it).
    pub(super) fn check_entity_post_effect(&mut self, first_person: bool) {
        self.spectated_post_effect = self
            .camera_subject()
            .filter(|_| first_person)
            .and_then(|c| c.post_effect);
        self.spectated_post_effect_active = self.spectated_post_effect.is_some();
    }

    /// The entity a server attached the view to, when it is not the player (live or detached).
    pub(in crate::game) fn remote_camera_id(&self) -> Option<u32> {
        self.remote.as_ref()?.camera.map(|id| id as u32)
    }

    /// The live entity id currently acting as the remote camera (`None` once removed or replaced).
    pub(in crate::game) fn live_remote_camera_id(&self) -> Option<u32> {
        self.remote
            .as_ref()
            .filter(|r| r.detached_camera.is_none())
            .and_then(|r| r.camera)
            .map(|id| id as u32)
    }

    /// Snapshot of a detached watched player's HUD state, if the detached camera was a player.
    pub(in crate::game) fn detached_camera_hud(&self) -> Option<&DetachedPlayerHud> {
        self.remote
            .as_ref()?
            .detached_camera
            .as_deref()?
            .detached_hud
            .as_ref()
    }

    /// Snapshot and detach the live camera entity before it is removed or replaced in the level.
    pub(in crate::game) fn detach_camera_if_matching(&mut self, matches: impl FnOnce(i32) -> bool) {
        let Some(cam_id) = self
            .remote
            .as_ref()
            .filter(|r| r.detached_camera.is_none())
            .and_then(|r| r.camera)
            .filter(|&id| matches(id))
        else {
            return;
        };
        let Some(mut detached) = self.camera_subject() else {
            return;
        };
        if detached.entity_id != cam_id as u32 {
            return;
        }
        // Entity.setRemoved detaches ride links while keeping old/current pose history intact.
        detached.has_player_passenger = false;
        detached.camera.mount = None;
        if detached.is_player {
            detached.detached_hud = self.snapshot_remote_player_hud(detached.entity_id);
        }
        if let Some(r) = self.remote.as_mut() {
            r.detached_camera = Some(Box::new(detached));
        }
    }

    fn snapshot_remote_player_hud(&self, uid: u32) -> Option<DetachedPlayerHud> {
        let m = self.mob_index.get_ref(&self.mobs, uid)?;
        let mob_id = m.id as i32;
        let max_health = self
            .motion
            .attributes
            .id("minecraft:max_health")
            .and_then(|attr| self.remote_attribute(mob_id, attr))
            .map_or(m.attrs.max_health, |v| v as f32);
        let armor = self
            .motion
            .attributes
            .id("minecraft:armor")
            .and_then(|attr| self.remote_attribute(mob_id, attr))
            .map_or(finalize::value(m, Attr::Armor) as i32, |v| v.floor() as i32);
        let fx = &m.extra.client_effects;
        Some(DetachedPlayerHud {
            main_hand_left: m.extra.main_hand_left,
            main_hand: m.extra.main_hand_stack().cloned(),
            offhand: m.extra.slot_stack(OFFHAND).cloned(),
            health: m.health,
            max_health,
            absorption: m.absorption,
            armor,
            air: m.air,
            under_water: m.fluids.eye_in_water,
            frozen: m.is_fully_frozen(),
            invulnerable: m.invulnerable > 0,
            regeneration: fx.get("regeneration").is_some(),
            poison: fx.get("poison").is_some(),
            wither: fx.get("wither").is_some(),
            hunger: fx.get("hunger").is_some(),
        })
    }

    /// `LocalPlayer.isControlledCamera`: the view is the player's own.
    pub(super) fn controls_camera(&self) -> bool {
        self.remote_camera_id().is_none()
    }
    /// Camera entity's UUID and feet position (`Minecraft.getCameraEntity`), borrowing
    /// any detached snapshot instead of cloning it.
    pub(in crate::game) fn camera_identity_and_position(&self) -> Option<(Option<u128>, [f64; 3])> {
        if let Some(r) = self.remote.as_ref()
            && let Some(id) = r.camera
        {
            if let Some(detached) = r
                .detached_camera
                .as_deref()
                .filter(|d| d.entity_id == id as u32)
            {
                return Some((detached.uuid, detached.camera.position));
            }
            return self
                .live_entity_camera_subject(r, id)
                .map(|c| (c.uuid, c.camera.position));
        }
        let p = self.player.as_ref()?;
        Some((
            self.own_profile_uuid(),
            [p.position.x, p.position.y, p.position.z],
        ))
    }

    /// `Minecraft.getCameraEntity`: local player, live tracked entity, or detached snapshot.
    pub(in crate::game) fn camera_subject(&self) -> Option<ResolvedCameraEntity> {
        if let Some(r) = self.remote.as_ref()
            && let Some(id) = r.camera
        {
            if let Some(detached) = r
                .detached_camera
                .as_deref()
                .filter(|d| d.entity_id == id as u32)
            {
                return Some(detached.clone());
            }
            return self.live_entity_camera_subject(r, id);
        }
        self.local_player_camera_subject()
    }

    /// `camera.entity().isSpectator()`: for lava and powdered-snow fog (`setupFog`).
    #[cfg(test)]
    pub(in crate::game) fn camera_entity_is_spectator(&self) -> bool {
        self.camera_subject()
            .map_or_else(|| self.own_player_is_spectator(), |c| c.is_spectator)
    }

    /// `Camera.getCameraEntityPartialTicks`: `1.0` when frozen unless player or player-ridden.
    pub(in crate::game) fn camera_partial_tick(&self) -> f64 {
        let entity_frozen = !self.clock.runs_normally()
            && self
                .camera_subject()
                .is_some_and(|c| !c.is_player && !c.has_player_passenger);
        self.clock.camera_fraction_for(entity_frozen)
    }

    /// `GameRenderer.shouldRenderBlockOutline`: player camera entity, HUD shown, and build or target gate.
    #[cfg(test)]
    pub(in crate::game) fn should_render_block_outline(&self) -> bool {
        let hit = (!self.entity_targeted()).then(|| self.pick()).flatten();
        self.should_render_block_outline_for_hit(hit.as_ref())
    }

    /// Block hit eligible for outline rendering before mod highlight cancellation.
    pub(in crate::game) fn block_outline_hit(&self) -> Option<super::Hit> {
        let hit = (!self.entity_targeted()).then(|| self.pick()).flatten()?;
        self.should_render_block_outline_for_hit(Some(&hit))
            .then_some(hit)
    }

    /// `GameRenderer.shouldRenderBlockOutline` evaluated against an optional targeted block hit.
    pub(in crate::game) fn should_render_block_outline_for_hit(
        &self,
        hit: Option<&super::Hit>,
    ) -> bool {
        let Some(cam) = self.camera_subject().filter(|c| c.is_player) else {
            return false;
        };
        if !super::hud_render::block_outline_drawn(
            self.ui.hide_gui,
            false,
            true,
            self.shows_panorama(),
        ) {
            return false;
        }
        if cam.may_build {
            return true;
        }
        let Some(hit) = hit else {
            return true;
        };
        if self.spectator() {
            return self.block_interactable_for_spectator(hit);
        }
        let main_hand = if cam.is_local_player {
            self.held()
        } else if let Some(hud) = cam.detached_hud.as_ref() {
            hud.main_hand.as_ref()
        } else {
            self.mob_index
                .find_ref(&self.mobs, cam.entity_id)
                .and_then(|i| {
                    self.mobs[i]
                        .extra
                        .slot_stack(sunrise_items::equipment::MAINHAND)
                })
        };
        main_hand.is_some_and(|stack| {
            self.stack_adventure_predicate(stack, "minecraft:can_break", hit.pos)
                || self.stack_adventure_predicate(stack, "minecraft:can_place_on", hit.pos)
        })
    }

    /// `mainCamera().position()` once aligned, or the camera entity's position before first align.
    /// This is the placement before a mod's `camera_position` overrides, which are render-only:
    /// streaming and the level-entry readiness never follow them.
    pub(in crate::game) fn visual_camera_position(&self) -> Option<[f64; 3]> {
        if self.view.initialized {
            Some(self.view.placed_position)
        } else if self.remote.is_some() {
            self.camera_eye_position()
        } else {
            self.camera_entity_position()
        }
    }

    /// Camera entity feet position (`cameraEntity.position()`).
    pub(in crate::game) fn camera_entity_position(&self) -> Option<[f64; 3]> {
        self.camera_subject().map(|c| c.camera.position)
    }

    /// Camera entity eye position (`cameraEntity.getEyePosition()`).
    pub(in crate::game) fn camera_eye_position(&self) -> Option<[f64; 3]> {
        self.camera_subject().map(|c| {
            let [x, y, z] = c.camera.position;
            [x, y + f64::from(c.camera.eye_height), z]
        })
    }

    /// `AbstractBedBlock.getBedOrientation` as its `toYRot`; `None` for any non-bed block.
    fn bed_orientation_yaw(&self, head: [i32; 3]) -> Option<f32> {
        let state = self.world.block(head[0], head[1], head[2]);
        let entry = self.pack.assets.registry.state(state)?;
        if !entry.name.ends_with("_bed") || !entry.properties.contains_key("part") {
            return None;
        }
        match entry.properties.get("facing").map(String::as_str) {
            Some("south") => Some(0.0),
            Some("west") => Some(90.0),
            Some("north") => Some(180.0),
            Some("east") => Some(270.0),
            _ => None,
        }
    }

    /// `LivingEntity.isSleeping` + `getBedOrientation` for any live mob or remote player.
    fn mob_sleeping_view(&self, m: &Mob) -> Option<Option<f32>> {
        if let Some(head) = m.extra.sleeping_pos {
            return Some(self.bed_orientation_yaw(head));
        }
        sleeping_view(m)
    }

    /// `AbstractClientPlayer.getFieldOfViewModifier` inputs for a spectated remote player.
    fn remote_fov_player(&self, m: &Mob) -> FovPlayer {
        let movement_speed = self
            .motion
            .attributes
            .id("minecraft:movement_speed")
            .and_then(|attr| self.remote_attribute(m.id as i32, attr))
            .unwrap_or(if m.attrs.speed > 0.0 {
                m.attrs.speed
            } else {
                0.1
            });
        let using = (m.extra.using_item > 0).then(|| {
            let slot = if m.extra.using_off_hand {
                OFFHAND
            } else {
                MAINHAND
            };
            let item = match m.extra.slot_stack(slot) {
                Some(s) if &*s.item == "minecraft:bow" => UseItem::Bow,
                Some(s) if self.stack_scopes(s) => UseItem::Spyglass,
                _ => UseItem::Other,
            };
            (item, (m.extra.using_item - 1).max(0))
        });
        FovPlayer {
            flying: false,
            walking_speed: 0.1,
            movement_speed,
            using,
        }
    }

    fn local_player_camera_subject(&self) -> Option<ResolvedCameraEntity> {
        let p = self.player.as_ref()?;
        let scale = p
            .attributes
            .computed_value_of("minecraft:scale")
            .unwrap_or(1.0) as f32;
        let distance = p
            .attributes
            .computed_value_of("minecraft:camera_distance")
            .unwrap_or(4.0);
        let sleeping = self.sleeping.map(|head| self.bed_orientation_yaw(head));
        let mount = self
            .riding
            .and_then(|id| self.mob_index.get_ref(&self.mobs, id))
            .map(|m| self.living_camera_values(m));
        let ride_turn = self
            .riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
            .map_or_else(
                || {
                    self.riding
                        .and_then(|id| self.carts.iter().find(|c| c.id == id))
                        .map_or(0.0, |c| c.passenger_yaw_delta)
                },
                |b| b.delta_rotation,
            );
        let dead = self.death_time();
        let fog_effects =
            CameraFogEffects::from_effects(&self.potions.effects, self.sculk.darkness);
        let camera = CameraEntity {
            position: [p.position.x, p.position.y, p.position.z],
            previous: [
                self.previous_position.x,
                self.previous_position.y,
                self.previous_position.z,
            ],
            x_rot: self.pitch,
            y_rot: self.yaw,
            x_rot_o: self.pitch,
            y_rot_o: self.yaw - ride_turn,
            eye_height: p.dimensions.eye_height,
            living: Some(LivingCamera {
                scale,
                camera_distance: distance,
                sleeping,
                dead,
            }),
            mount,
        };
        Some(ResolvedCameraEntity {
            entity_id: self.own_entity_id() as u32,
            uuid: self.own_profile_uuid(),
            camera,
            is_local_player: true,
            is_player: true,
            may_build: self.may_build(),
            is_spectator: self.own_player_is_spectator(),
            is_sleeping: self.sleeping.is_some(),
            blocks_sky: fog_effects.blindness.is_some() || fog_effects.darkness_blend.is_some(),
            fog_effects,
            hurt: Some(CameraHurt {
                hurt_time: p.health.hurt_animation_time,
                hurt_duration: p.health.hurt.hurt_duration,
                hurt_dir: p.health.hurt_dir,
                death_time: dead,
            }),
            avatar_bob: Some(self.avatar),
            fov_player: self.fov_player(),
            has_player_passenger: false,
            collision_kind: EntityContextKind::Player,
            post_effect: None,
            detached_hud: None,
        })
    }

    /// `Camera.alignWithEntity`: living vehicle's scale and camera distance without allocating closures.
    fn living_vehicle_for(&self, remote: &Remote, id: u32) -> Option<(f32, f64)> {
        let vehicle = remote.passenger_vehicles.get(&id)?.vehicle;
        if let Some(d) = self.dragon.as_ref().filter(|d| d.id == vehicle) {
            return Some(self.remote_entity_living_values(d.id as i32));
        }
        if let Some(m) = self.mob_index.get_ref(&self.mobs, vehicle) {
            return Some(self.living_camera_values(m));
        }
        if let Some(a) = self.armor_stands.iter().find(|a| a.id == vehicle) {
            return Some(self.stand_camera_values(a));
        }
        if vehicle == remote.entity_id as u32
            && let Some(p) = self.player.as_ref()
        {
            let scale = p
                .attributes
                .computed_value_of("minecraft:scale")
                .unwrap_or(1.0) as f32;
            let distance = p
                .attributes
                .computed_value_of("minecraft:camera_distance")
                .unwrap_or(4.0);
            return Some((scale, distance));
        }
        None
    }

    /// `Entity.countPlayerPassengers() > 0` without a fixed depth cutoff.
    pub(in crate::game) fn entity_has_player_passenger(
        &self,
        remote: &Remote,
        target: u32,
    ) -> bool {
        if self.player.is_some() && self.riding == Some(target) {
            return true;
        }
        if remote.passenger_vehicles.is_empty() {
            return false;
        }
        let limit = remote.passenger_vehicles.len() + 1;
        let rides_target = |mut rider: u32| -> bool {
            for _ in 0..limit {
                let next = if rider == remote.entity_id as u32 {
                    self.riding
                        .or_else(|| remote.passenger_vehicles.get(&rider).map(|s| s.vehicle))
                } else {
                    remote.passenger_vehicles.get(&rider).map(|s| s.vehicle)
                };
                match next {
                    Some(v) if v == target => return true,
                    Some(v) => rider = v,
                    None => return false,
                }
            }
            false
        };
        if self.player.is_some()
            && (self.riding.is_some()
                || remote
                    .passenger_vehicles
                    .contains_key(&(remote.entity_id as u32)))
            && rides_target(remote.entity_id as u32)
        {
            return true;
        }
        remote.level_players.keys().any(|&pid| {
            let uid = pid as u32;
            remote.passenger_vehicles.contains_key(&uid) && rides_target(uid)
        })
    }

    /// `TickRateManager.isEntityFrozen` + `ClientLevel.tickPassenger`: ticks while frozen when
    /// this entity is a player, carries a player passenger, or rides inside a player-bearing tree.
    pub(in crate::game) fn entity_ticks_while_frozen(&self, remote: &Remote, id: u32) -> bool {
        if remote.level_players.contains_key(&(id as i32))
            || (self.player.is_some() && id == remote.entity_id as u32)
        {
            return true;
        }
        if self.entity_has_player_passenger(remote, id) {
            return true;
        }
        if remote.passenger_vehicles.is_empty() {
            return false;
        }
        let limit = remote.passenger_vehicles.len() + 1;
        let mut root = id;
        for _ in 0..limit {
            let Some(parent) = remote.passenger_vehicles.get(&root).map(|s| s.vehicle) else {
                break;
            };
            root = parent;
        }
        if root == id {
            return false;
        }
        remote.level_players.contains_key(&(root as i32))
            || (self.player.is_some() && root == remote.entity_id as u32)
            || self.entity_has_player_passenger(remote, root)
    }

    fn tracked_rotation(remote: &Remote, id: i32, fallback: [f32; 2]) -> (f32, f32, f32, f32) {
        if let Some(t) = remote.entities.get(&id) {
            (t.yaw, t.pitch, t.previous_yaw, t.previous_pitch)
        } else {
            (fallback[0], fallback[1], fallback[0], fallback[1])
        }
    }

    /// Resolve a live client entity's pose and camera inputs; never substitutes the detached camera.
    pub(super) fn live_entity_camera_subject(
        &self,
        remote: &Remote,
        id: i32,
    ) -> Option<ResolvedCameraEntity> {
        let uid = id as u32;
        let uuid = remote.level_entity_uuids.get(&id).copied().or_else(|| {
            self.persist
                .uuids
                .get(&uid)
                .copied()
                .map(sunrise_client::lan::playerdata::ints_uuid)
        });
        let mount = self.living_vehicle_for(remote, uid);
        let has_player_passenger = self.entity_has_player_passenger(remote, uid);
        let non_living_subject = |camera, collision_kind, kind: &str| ResolvedCameraEntity {
            entity_id: uid,
            uuid,
            camera,
            is_local_player: false,
            is_player: false,
            may_build: false,
            is_spectator: false,
            is_sleeping: false,
            blocks_sky: false,
            fog_effects: CameraFogEffects::default(),
            hurt: None,
            avatar_bob: None,
            fov_player: None,
            has_player_passenger,
            collision_kind,
            post_effect: entity_post_effect(kind),
            detached_hud: None,
        };
        // The Ender Dragon's live pose and living state live in `self.dragon` (`self.mobs` only holds metadata).
        if let Some(d) = self.dragon.as_ref().filter(|d| d.id == uid) {
            let (scale, camera_distance) = self.remote_entity_living_values(id);
            let dead = (d.health <= 0.0).then_some(d.death_time);
            let fog_effects = self
                .mob_index
                .get_ref(&self.mobs, uid)
                .map(|m| {
                    CameraFogEffects::from_effects(&m.extra.client_effects, m.extra.darkness_blend)
                })
                .unwrap_or_default();
            let camera = CameraEntity {
                position: [d.position.x, d.position.y, d.position.z],
                previous: [d.previous.x, d.previous.y, d.previous.z],
                x_rot: 0.0,
                y_rot: d.yaw,
                x_rot_o: 0.0,
                y_rot_o: d.previous_yaw,
                eye_height: self.type_eye_height("minecraft:ender_dragon") as f32 * scale,
                living: Some(LivingCamera {
                    scale,
                    camera_distance,
                    sleeping: None,
                    dead,
                }),
                mount,
            };
            return Some(ResolvedCameraEntity {
                entity_id: uid,
                uuid,
                camera,
                is_local_player: false,
                is_player: false,
                may_build: false,
                is_spectator: false,
                is_sleeping: false,
                blocks_sky: fog_effects.blindness.is_some() || fog_effects.darkness_blend.is_some(),
                fog_effects,
                hurt: Some(CameraHurt {
                    hurt_time: d.hurt_time,
                    hurt_duration: 10,
                    hurt_dir: 0.0,
                    death_time: dead,
                }),
                avatar_bob: None,
                fov_player: None,
                has_player_passenger,
                collision_kind: EntityContextKind::Living,
                post_effect: entity_post_effect("minecraft:ender_dragon"),
                detached_hud: None,
            });
        }
        if let Some(m) = self.mob_index.get_ref(&self.mobs, uid) {
            let is_player = &*m.kind.id == "minecraft:player";
            let mut camera = mob_camera_entity(m, self.living_camera_values(m));
            if let Some(living) = &mut camera.living {
                living.sleeping = self.mob_sleeping_view(m);
            }
            camera.mount = mount;
            let is_sleeping = camera.living.is_some_and(|l| l.sleeping.is_some());
            let dead = camera.living.and_then(|l| l.dead);
            let fog_effects =
                CameraFogEffects::from_effects(&m.extra.client_effects, m.extra.darkness_blend);
            let is_spectator = is_player
                && remote
                    .remote_player_info_game_mode(id)
                    .map_or(m.extra.spectator, |mode| {
                        mode == sunrise_client::tab_list::SPECTATOR
                    });
            return Some(ResolvedCameraEntity {
                entity_id: uid,
                uuid,
                camera,
                is_local_player: false,
                is_player,
                may_build: is_player,
                is_spectator,
                is_sleeping,
                blocks_sky: fog_effects.blindness.is_some() || fog_effects.darkness_blend.is_some(),
                fog_effects,
                hurt: Some(CameraHurt {
                    hurt_time: m.hurt_time,
                    hurt_duration: 10,
                    hurt_dir: m.extra.hurt_dir,
                    death_time: dead,
                }),
                avatar_bob: is_player.then_some(m.extra.avatar),
                fov_player: is_player.then(|| self.remote_fov_player(m)),
                has_player_passenger,
                collision_kind: if is_player {
                    EntityContextKind::Player
                } else {
                    EntityContextKind::Living
                },
                post_effect: entity_post_effect(&m.kind.id),
                detached_hud: None,
            });
        }
        if let Some(a) = self.armor_stands.iter().find(|a| a.id == uid) {
            let mut camera = stand_camera_entity_with(a, self.stand_camera_values(a));
            camera.mount = mount;
            let fog_effects = CameraFogEffects::from_effects(&a.client_effects, a.darkness_blend);
            return Some(ResolvedCameraEntity {
                entity_id: uid,
                uuid,
                camera,
                is_local_player: false,
                is_player: false,
                may_build: false,
                is_spectator: false,
                is_sleeping: false,
                blocks_sky: fog_effects.blindness.is_some() || fog_effects.darkness_blend.is_some(),
                fog_effects,
                hurt: Some(CameraHurt::default()),
                avatar_bob: None,
                fov_player: None,
                has_player_passenger,
                collision_kind: EntityContextKind::Living,
                post_effect: entity_post_effect("minecraft:armor_stand"),
                detached_hud: None,
            });
        }
        if let Some(i) = self.interactions.get(&id) {
            let mut camera = interaction_camera_entity(i);
            camera.mount = mount;
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                "minecraft:interaction",
            ));
        }
        if let Some(b) = self.boats.iter().find(|b| b.id == uid) {
            let kind = remote
                .entities
                .get(&id)
                .map_or("minecraft:oak_boat", |t| t.kind.as_str());
            let mut camera = boat_camera_entity(b);
            camera.mount = mount;
            return Some(non_living_subject(camera, EntityContextKind::Other, kind));
        }
        if let Some(c) = self.carts.iter().find(|c| c.id == uid) {
            let kind = remote
                .entities
                .get(&id)
                .map_or("minecraft:minecart", |t| t.kind.as_str());
            let camera = CameraEntity {
                position: [c.position.x, c.position.y, c.position.z],
                previous: [c.previous.x, c.previous.y, c.previous.z],
                x_rot: c.pitch,
                y_rot: c.yaw,
                x_rot_o: c.previous_pitch,
                y_rot_o: c.previous_yaw,
                eye_height: self.type_eye_height(kind) as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(camera, EntityContextKind::Other, kind));
        }
        if let Some(f) = self.item_frames.iter().find(|f| f.id == uid) {
            let kind = if f.glow {
                "minecraft:glow_item_frame"
            } else {
                "minecraft:item_frame"
            };
            let mut camera = frame_camera_entity(f);
            camera.mount = mount;
            return Some(non_living_subject(camera, EntityContextKind::Other, kind));
        }
        if let Some(p) = self.paintings.iter().find(|p| p.id == uid) {
            let mut camera = painting_camera_entity(p);
            camera.mount = mount;
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                "minecraft:painting",
            ));
        }
        if let Some(t) = self.tnts.iter().find(|t| t.id == uid) {
            let mut camera = tnt_camera_entity(t);
            camera.mount = mount;
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                "minecraft:tnt",
            ));
        }
        if let Some(o) = self.orbs.iter().find(|o| o.id == uid) {
            let (y_rot, x_rot, y_rot_o, x_rot_o) = Self::tracked_rotation(remote, id, o.rotation);
            let camera = CameraEntity {
                position: [o.position.x, o.position.y, o.position.z],
                previous: [o.previous.x, o.previous.y, o.previous.z],
                x_rot,
                y_rot,
                x_rot_o,
                y_rot_o,
                eye_height: self.type_eye_height("minecraft:experience_orb") as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                "minecraft:experience_orb",
            ));
        }
        if let Some(p) = self.creatures.projectiles.iter().find(|p| p.id == uid) {
            let kind = remote
                .entities
                .get(&id)
                .map_or("minecraft:arrow", |t| t.kind.as_str());
            let camera = CameraEntity {
                position: [p.position.x, p.position.y, p.position.z],
                previous: [p.previous.x, p.previous.y, p.previous.z],
                x_rot: p.pitch,
                y_rot: p.yaw,
                x_rot_o: p.previous_pitch,
                y_rot_o: p.previous_yaw,
                eye_height: self.type_eye_height(kind) as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(camera, EntityContextKind::Other, kind));
        }
        if let Some(t) = self.tridents.iter().find(|t| t.id == uid) {
            let camera = CameraEntity {
                position: [t.position.x, t.position.y, t.position.z],
                previous: [t.previous.x, t.previous.y, t.previous.z],
                x_rot: t.pitch,
                y_rot: t.yaw,
                x_rot_o: t.pitch,
                y_rot_o: t.yaw,
                eye_height: self.type_eye_height("minecraft:trident") as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                "minecraft:trident",
            ));
        }
        if let Some(f) = self.rockets.iter().find(|r| r.id == uid) {
            let (y_rot, x_rot, y_rot_o, x_rot_o) = Self::tracked_rotation(remote, id, f.rotation);
            let camera = CameraEntity {
                position: [f.position.x, f.position.y, f.position.z],
                previous: [f.previous.x, f.previous.y, f.previous.z],
                x_rot,
                y_rot,
                x_rot_o,
                y_rot_o,
                eye_height: self.type_eye_height("minecraft:firework_rocket") as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                "minecraft:firework_rocket",
            ));
        }
        if let Some(d) = self.dropped.iter().find(|d| d.id == uid) {
            let (y_rot, x_rot, y_rot_o, x_rot_o) = Self::tracked_rotation(remote, id, d.rotation);
            let camera = CameraEntity {
                position: [d.position.x, d.position.y, d.position.z],
                previous: [d.previous.x, d.previous.y, d.previous.z],
                x_rot,
                y_rot,
                x_rot_o,
                y_rot_o,
                eye_height: self.type_eye_height("minecraft:item") as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(
                camera,
                EntityContextKind::Item,
                "minecraft:item",
            ));
        }
        if let Some(f) = self.falling.iter().find(|f| f.id == uid) {
            let (y_rot, x_rot, y_rot_o, x_rot_o) = Self::tracked_rotation(remote, id, f.rotation);
            let camera = CameraEntity {
                position: [f.position.x, f.position.y, f.position.z],
                previous: [f.previous.x, f.previous.y, f.previous.z],
                x_rot,
                y_rot,
                x_rot_o,
                y_rot_o,
                eye_height: self.type_eye_height("minecraft:falling_block") as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                "minecraft:falling_block",
            ));
        }
        if let Some((position, previous)) = self.potions.remote_thrown_pose(uid) {
            let kind = remote
                .entities
                .get(&id)
                .map_or("minecraft:splash_potion", |t| t.kind.as_str());
            let (y_rot, x_rot, y_rot_o, x_rot_o) = Self::tracked_rotation(remote, id, [0.0, 0.0]);
            let camera = CameraEntity {
                position,
                previous,
                x_rot,
                y_rot,
                x_rot_o,
                y_rot_o,
                eye_height: self.type_eye_height(kind) as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(camera, EntityContextKind::Other, kind));
        }
        if let Some((position, previous)) = self.end.remote_eye_pose(uid) {
            let (y_rot, x_rot, y_rot_o, x_rot_o) = Self::tracked_rotation(remote, id, [0.0, 0.0]);
            let camera = CameraEntity {
                position,
                previous,
                x_rot,
                y_rot,
                x_rot_o,
                y_rot_o,
                eye_height: self.type_eye_height("minecraft:eye_of_ender") as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                "minecraft:eye_of_ender",
            ));
        }
        if let Some((position, previous)) = self
            .hook
            .as_ref()
            .filter(|h| h.id() == uid)
            .map(|h| {
                (
                    [h.position.x, h.position.y, h.position.z],
                    [h.previous.x, h.previous.y, h.previous.z],
                )
            })
            .or_else(|| {
                self.remote_hooks.iter().find(|h| h.id() == uid).map(|h| {
                    (
                        [h.position.x, h.position.y, h.position.z],
                        [h.previous.x, h.previous.y, h.previous.z],
                    )
                })
            })
        {
            let (y_rot, x_rot, y_rot_o, x_rot_o) = Self::tracked_rotation(remote, id, [0.0, 0.0]);
            let camera = CameraEntity {
                position,
                previous,
                x_rot,
                y_rot,
                x_rot_o,
                y_rot_o,
                eye_height: self.type_eye_height("minecraft:fishing_bobber") as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                "minecraft:fishing_bobber",
            ));
        }
        if let Some(d) = self.text_displays.get(&id) {
            let mut camera = d.camera_entity();
            camera.mount = mount;
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                d.kind(),
            ));
        }
        if let Some(c) = self.crystals.iter().find(|c| c.id == uid) {
            let at = [c.position.x, c.position.y, c.position.z];
            let (y_rot, x_rot, y_rot_o, x_rot_o) = Self::tracked_rotation(remote, id, [0.0, 0.0]);
            let camera = CameraEntity {
                position: at,
                previous: at,
                x_rot,
                y_rot,
                x_rot_o,
                y_rot_o,
                eye_height: self.type_eye_height("minecraft:end_crystal") as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(
                camera,
                EntityContextKind::Other,
                "minecraft:end_crystal",
            ));
        }
        if let Some(row) = remote.general.rows.get(&id) {
            let kind = remote.entities.get(&id).map_or("", |t| t.kind.as_str());
            let camera = CameraEntity {
                position: row.position,
                previous: row.previous,
                x_rot: row.rotation[1],
                y_rot: row.rotation[0],
                x_rot_o: row.previous_rotation[1],
                y_rot_o: row.previous_rotation[0],
                eye_height: self.type_eye_height(kind) as f32,
                living: None,
                mount,
            };
            return Some(non_living_subject(camera, EntityContextKind::Other, kind));
        }
        let tracked = remote.entities.get(&id)?;
        let camera = CameraEntity {
            position: tracked.position,
            previous: tracked.previous,
            x_rot: tracked.pitch,
            y_rot: tracked.yaw,
            x_rot_o: tracked.previous_pitch,
            y_rot_o: tracked.previous_yaw,
            eye_height: self.type_eye_height(&tracked.kind) as f32,
            living: None,
            mount,
        };
        let collision_kind = if tracked.kind == "minecraft:item" {
            EntityContextKind::Item
        } else {
            EntityContextKind::Other
        };
        Some(non_living_subject(camera, collision_kind, &tracked.kind))
    }
    /// `Hud.getCameraPlayer`: whose hotbar, bars and vehicle the HUD shows.
    pub(in crate::game) fn camera_player(&mut self) -> CameraPlayer {
        let Some(id) = self.remote_camera_id() else {
            return CameraPlayer::Own;
        };
        if self
            .remote
            .as_ref()
            .is_some_and(|r| id == r.entity_id as u32)
        {
            return CameraPlayer::Own;
        }
        if let Some(detached) = self
            .remote
            .as_ref()
            .and_then(|r| r.detached_camera.as_deref())
        {
            return if detached.is_player {
                CameraPlayer::Detached
            } else {
                CameraPlayer::Absent
            };
        }
        match self.mob_index.find(&self.mobs, id) {
            Some(i) if &*self.mobs[i].kind.id == "minecraft:player" => CameraPlayer::Other(i),
            _ => CameraPlayer::Absent,
        }
    }

    /// The view of the entity a server attached the camera to, live or detached.
    pub(super) fn spectated_camera_snapshot(&self) -> Option<CameraEntity> {
        self.camera_subject()
            .filter(|c| !c.is_local_player)
            .map(|c| c.camera)
    }

    fn remote_entity_living_values(&self, id: i32) -> (f32, f64) {
        let catalog = &self.motion.attributes;
        let value = |name: &str, default: f64| {
            catalog
                .id(name)
                .and_then(|attribute| self.remote_attribute(id, attribute))
                .unwrap_or(default)
        };
        (
            value("minecraft:scale", 1.0) as f32,
            value("minecraft:camera_distance", 4.0),
        )
    }

    /// Returns synced or type-default scale and camera distance through LivingEntity.sanitizeScale.
    pub(in crate::game) fn living_camera_values(&self, m: &Mob) -> (f32, f64) {
        let catalog = &self.motion.attributes;
        let value = |name: &str, default: f64| {
            catalog
                .id(name)
                .and_then(|attribute| self.remote_attribute(m.id as i32, attribute))
                .or_else(|| m.kind.attributes.get(name).copied())
                .unwrap_or(default)
        };
        let raw_scale = value("minecraft:scale", 1.0) as f32;
        let scale = match m.species() {
            Species::HappyGhast => raw_scale.min(1.0),
            Species::Shulker => raw_scale.min(3.0),
            _ => raw_scale,
        };
        (scale, value("minecraft:camera_distance", 4.0))
    }

    /// The scale and camera distance `Camera.alignWithEntity` reads from an armor stand.
    fn stand_camera_values(&self, a: &ArmorStand) -> (f32, f64) {
        self.remote_entity_living_values(a.id as i32)
    }
    /// The entity under a spectator's crosshair (`Minecraft.hitResult` as an `EntityHitResult`).
    pub(super) fn spectator_click_target(&self) -> Option<i32> {
        self.crosshair_entity_hit().map(|hit| hit.id)
    }

    /// `LevelRenderer`: the camera entity is not drawn from its own eyes, but a sleeping one is.
    pub(super) fn hides_as_camera(&self, id: u32) -> bool {
        self.camera_type.is_first_person()
            && self.live_remote_camera_id() == Some(id)
            && !self
                .mob_index
                .get_ref(&self.mobs, id)
                .is_some_and(|m| self.mob_sleeping_view(m).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::{camera_after_packet, entity_post_effect, stand_camera_entity};
    use sunrise_client::entities::ArmorStand;
    use sunrise_core::geometry::Vec3;

    /// A server's camera on an armor stand looks out of the stand's eyes
    /// (EntityType.ARMOR_STAND `eyeHeight(1.7775F)`, `BABY_DIMENSIONS`
    /// 0.9875F for a small one, nothing for a marker), along its yaw.
    #[test]
    fn an_armor_stand_holds_the_camera_at_its_eyes() {
        let mut stand = ArmorStand::new(7, Vec3::new(10.5, 64.0, -3.5), 90.0);
        stand.yaw = 135.0;
        stand.previous_pitch = -5.0;
        stand.pitch = -20.0;
        let view = stand_camera_entity(&stand);
        assert_eq!(view.position, [10.5, 64.0, -3.5]);
        assert_eq!(view.eye_height, 1.7775f32);
        assert_eq!((view.y_rot_o, view.y_rot), (90.0, 135.0));
        assert_eq!((view.x_rot_o, view.x_rot), (-5.0, -20.0));
        stand.small = true;
        assert_eq!(stand_camera_entity(&stand).eye_height, 0.9875f32);
        stand.marker = true;
        assert_eq!(stand_camera_entity(&stand).eye_height, 0.0);
    }

    /// Only a creeper, spider or enderman has a spectator chain.
    #[test]
    fn spectated_entities_pick_their_chain() {
        assert_eq!(
            entity_post_effect("minecraft:creeper"),
            Some("minecraft:creeper")
        );
        assert_eq!(
            entity_post_effect("minecraft:spider"),
            Some("minecraft:spider")
        );
        assert_eq!(
            entity_post_effect("minecraft:enderman"),
            Some("minecraft:invert")
        );
        assert_eq!(
            entity_post_effect("minecraft:cave_spider"),
            Some("minecraft:spider")
        );
        assert_eq!(entity_post_effect("minecraft:cow"), None);
    }

    /// The camera goes to a known entity, back to the player on its own id,
    /// and stays where it is for an entity the level does not hold.
    #[test]
    fn set_camera_follows_vanilla() {
        assert_eq!(camera_after_packet(None, 5, 9, true), Some(9));
        assert_eq!(camera_after_packet(Some(9), 5, 5, false), None);
        assert_eq!(camera_after_packet(Some(9), 5, 12, false), Some(9));
        assert_eq!(camera_after_packet(None, 5, 12, false), None);
    }
}
