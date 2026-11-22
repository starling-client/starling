//! An entity's motion and shape for native server mods, through an
//! `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387):
//! velocity, ground, box, eyes and view. They read the entity a handle names
//! as `native_entity_refs.rs` finds it, and return `None` (or `false`) for a
//! stale handle. On a client (API @API:client_entity_refs@) they read the
//! entities the server shows the window, and nothing moves one.
use super::entity::{Entity as _, EntityHandle, with_entity, with_entity_mut};
use super::{Game, commands::Target};
use mod_api::{ClipBlock, ClipFluid, ClipHit, EntityRef, LevelWriteError};
use sunrise_client::{collisions::SceneCollisions, motion::player_context};
use sunrise_core::geometry::{Aabb, Axis, Vec3};
use sunrise_core::position::BlockPos;

/// `Entity.calculateViewVector` for a look turned `pitch_offset` degrees down: the unit
/// vector along `yaw` and `pitch + pitch_offset`.
fn turned_view_vector(yaw: f32, pitch: f32, pitch_offset: f32) -> [f64; 3] {
    let v = sunrise_world::view_direction::view_vector(pitch + pitch_offset, yaw);
    [v.x, v.y, v.z]
}

/// `Entity.getOnPos()`: the supporting block when there is one, else the block `1.0E-5F`
/// under the feet at `position`.
pub(super) fn on_block_position(support: Option<BlockPos>, position: Vec3) -> [i32; 3] {
    match support {
        Some(p) => [p.x, p.y, p.z],
        None => [
            position.x.floor() as i32,
            (position.y - f64::from(1.0e-5_f32)).floor() as i32,
            position.z.floor() as i32,
        ],
    }
}

/// The end of a ray `distance` blocks from `from` along the unit vector `look`
/// (`from.add(look.scale(distance))`).
fn ray_end(from: [f64; 3], look: [f64; 3], distance: f64) -> [f64; 3] {
    std::array::from_fn(|i| from[i] + look[i] * distance)
}

fn corners(b: Aabb) -> [[f64; 3]; 2] {
    let axes = [Axis::X, Axis::Y, Axis::Z];
    [axes.map(|a| b.min(a)), axes.map(|a| b.max(a))]
}

impl Game<'_> {
    /// The simulated entity (one of the shared entity walk's lists) with `id`.
    pub(super) fn native_walk_entity(&self, id: u32) -> Option<EntityHandle> {
        self.entity_handles()
            .find(|h| with_entity!(self, *h, e => e.id() == id))
    }
    /// This window's own player, when a client's target names it.
    fn native_client_player(&self, t: Target) -> Option<&sunrise_client::motion::PlayerMotion> {
        self.remote.as_ref()?;
        (t == Target::Player).then_some(())?;
        self.player.as_ref()
    }
    /// The server's copy of the player a target names.
    pub(super) fn native_target_guest(
        &self,
        t: Target,
    ) -> Option<&sunrise_client::lan::guest::Guest> {
        match t {
            Target::Player => self.command_guest(self.lan_acting?),
            Target::Guest(conn) => self.command_guest(conn),
            _ => None,
        }
    }
    /// The entity's velocity in blocks a tick (`Entity.getDeltaMovement`). A
    /// player's is the move its client last reported (`getKnownMovement`); an
    /// entity Sunrise doesn't simulate (a display, a painting) has none, so
    /// zero. On a client, another entity's is the motion the server last
    /// sent it (`Entity.lerpMotion`). API 379.
    pub(super) fn entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let t = self.native_entity_read_target(entity)?;
        if !self.is_logical_server() {
            // The client's copy: its own player's motion, and the motion the
            // server sent for an entity it tracks.
            let v = match t {
                Target::Player => self.player.as_ref()?.velocity,
                _ => match self.native_walk_entity(entity.id()) {
                    Some(h) => with_entity!(self, h, e => e.velocity()),
                    None => Vec3::ZERO,
                },
            };
            return Some([v.x, v.y, v.z]);
        }
        if let Some(g) = self.native_target_guest(t) {
            return Some(std::array::from_fn(|i| {
                g.position[i] - g.previous_position[i]
            }));
        }
        let v = match self.native_walk_entity(entity.id()) {
            Some(h) => with_entity!(self, h, e => e.velocity()),
            None => Vec3::ZERO,
        };
        Some([v.x, v.y, v.z])
    }
    /// `Entity.getKnownSpeed`, in blocks a tick: a player's reported move; a
    /// vehicle with a live controlling player, that player's; else what the
    /// entity moved in its last tick (`computeSpeed`: position minus the
    /// previous tick's). Every list the walk keeps has a previous position.
    /// A frame, painting, crystal, spawner, cloud, fangs or marker never
    /// moves, so zero; the dragon uses its own previous position.
    pub(super) fn native_entity_known_speed(&self, entity: EntityRef) -> Option<Vec3> {
        let t = self.native_entity_target(entity)?;
        if let Some(g) = self.native_target_guest(t) {
            let [x, y, z] = g.known_movement;
            return Some(Vec3::new(x, y, z));
        }
        if matches!(t, Target::Player) {
            return Some(self.known_player_movement());
        }
        if let Some(rider) = self.native_controlling_player(t, entity) {
            return self.native_entity_known_speed(rider);
        }
        if let Target::Dragon(_) = t {
            let d = self.dragon.as_ref()?;
            return Some(d.position.subtract(d.previous));
        }
        Some(match self.native_walk_entity(entity.id()) {
            Some(h) => with_entity!(self, h, e => e.position().subtract(e.previous_position())),
            None => Vec3::ZERO,
        })
    }
    /// Whether `Entity.getControllingPassenger` can be a player: a boat's
    /// first passenger, or a mount's rider as `equine::controlled` decides
    /// (saddled; a pig or strider's rider holding the stick; a harnessed
    /// happy ghast), the rule the mount code steers by.
    fn native_steered(&self, t: Target) -> bool {
        match t {
            Target::Boat(_) => true,
            Target::Mob(id) => self
                .mobs
                .iter()
                .any(|m| m.id == id && sunrise_client::mobs::equine::controlled(m)),
            _ => false,
        }
    }
    /// `Entity.getControllingPassenger` when it is a player.
    fn native_controlling_player(&self, t: Target, entity: EntityRef) -> Option<EntityRef> {
        let steered = self.native_steered(t);
        let first = *self.entity_passengers(entity).ok_or(()).ok()?.first()?;
        let player = matches!(
            self.native_entity_target(first)?,
            Target::Player | Target::Guest(_)
        );
        (steered && player).then_some(first)
    }
    /// Sets the entity's velocity (`Entity.setDeltaMovement`); clients see it
    /// as they see the entity move. False for a player (its client moves
    /// it), an entity Sunrise doesn't simulate, a value that isn't finite,
    /// or on a client. API 379.
    pub(super) fn entity_set_velocity(&mut self, entity: EntityRef, velocity: [f64; 3]) -> bool {
        if self.remote.is_some()
            || !velocity.iter().all(|v| v.is_finite())
            || self.native_entity_target(entity).is_none()
        {
            return false;
        }
        let Some(h) = self.native_walk_entity(entity.id()) else {
            return false;
        };
        let [x, y, z] = velocity;
        with_entity_mut!(self, h, e => *e.velocity_mut() = Vec3::new(x, y, z));
        if let EntityHandle::Mob(i) = h {
            self.mobs[i].needs_sync = true;
        }
        true
    }
    /// `Entity.resetFallDistance`: how far the entity has fallen is forgotten, so its next
    /// landing hurts it as a step does (a fan's or a launcher's push, a mod's soft block).
    /// Players too, the acting one and guests. False for a stale handle or an entity not in
    /// the level in place; true for a kind that keeps no fall distance, which has none to forget.
    pub(super) fn entity_reset_fall_distance(&mut self, entity: EntityRef) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        if matches!(target, Target::Player | Target::Guest(_)) {
            let motion = match target {
                Target::Player => self.player.as_mut(),
                Target::Guest(conn) => self
                    .lan
                    .as_mut()
                    .and_then(|host| host.hub.guests.motion_mut(conn)),
                _ => None,
            };
            let Some(motion) = motion else {
                return false;
            };
            motion.fall_distance = 0.0;
            return true;
        }
        let Some(h) = self.native_walk_entity(entity.id()) else {
            return false;
        };
        with_entity_mut!(self, h, e => e.reset_fall_distance());
        true
    }
    /// Whether the entity stands on a block (`Entity.onGround`). API 379.
    pub(super) fn entity_on_ground(&self, entity: EntityRef) -> Option<bool> {
        let t = self.native_entity_read_target(entity)?;
        if t == Target::Player
            && let Some(p) = self.player.as_ref()
        {
            return Some(p.ground.on_ground);
        }
        if let Some(g) = self.native_target_guest(t) {
            return Some(g.on_ground);
        }
        if let Target::Hook(id) = t
            && let Some((hook, _)) = self.fishing_hook_by_id(id)
        {
            return Some(hook.ground.on_ground);
        }
        Some(
            self.native_walk_entity(entity.id())
                .is_some_and(|h| with_entity!(self, h, e => e.on_ground())),
        )
    }
    /// `Entity.setOnGround` on its live server body, with no movement or fall processing.
    pub(super) fn entity_set_on_ground(
        &mut self,
        entity: EntityRef,
        on_ground: bool,
    ) -> Result<bool, LevelWriteError> {
        if !self.is_logical_server() {
            return Err(LevelWriteError::Unavailable);
        }
        let Some(target) = self.native_entity_target(entity) else {
            return Ok(false);
        };
        match target {
            Target::Player | Target::Guest(_) => {
                self.set_server_player_ground(target, entity.id(), on_ground)
            }
            Target::Hook(id) => self.set_fishing_hook_ground(id, on_ground),
            _ => {
                let Some(handle) = self.native_walk_entity(entity.id()) else {
                    return Ok(false);
                };
                with_entity_mut!(self, handle, e => {
                    e.update_retained_ground(&self.world, self.collision, &world_items!(self).entity_type_tags, on_ground, None)
                        .map_err(|_| LevelWriteError::Unavailable)
                })
            }
        }
    }

    /// The acting player's record and motion must agree when the guest turn ends. A parked
    /// player's position and equipment belong to its hub row, not to the last simulated motion.
    fn set_server_player_ground(
        &mut self,
        target: Target,
        entity_id: u32,
        on_ground: bool,
    ) -> Result<bool, LevelWriteError> {
        let Some(body) = self.native_player_body(target) else {
            return Ok(false);
        };
        let (world, position, bounds, descending, held, armor, conn) = match target {
            Target::Player => (
                &self.world,
                body.position,
                body.bounds(),
                // PlayerInput updates the guest row before the sneak callback and the body tick.
                self.lan_acting
                    .and_then(|conn| self.command_guest(conn))
                    .map_or(body.shift_key_down, |guest| guest.sneaking),
                self.inventory.get(self.selected).and_then(Option::as_ref),
                &self.armor,
                self.lan_acting,
            ),
            Target::Guest(conn) => {
                let Some(guest) = self.command_guest(conn) else {
                    return Ok(false);
                };
                let world = if guest.dimension == self.dim.id() {
                    &self.world
                } else {
                    self.levels
                        .parked_world_of(&guest.dimension)
                        .ok_or(LevelWriteError::Unavailable)?
                };
                let [x, y, z] = guest.position;
                (
                    world,
                    Vec3::new(x, y, z),
                    guest.bounds(),
                    guest.sneaking,
                    guest.inventory.get(guest.selected).and_then(Option::as_ref),
                    &guest.armor,
                    Some(conn),
                )
            }
            _ => return Ok(false),
        };
        let mut subject = body.powder_snow_subject();
        subject.leather_boots = super::worn_boots::leather_boots(armor);
        let mut scene = SceneCollisions {
            world,
            binding: self.collision,
            subject: Some(subject),
        };
        let mut ground = body.ground;
        ground.on_ground = on_ground;
        let scan = scene.update_ground_support(
            &mut ground,
            position,
            bounds,
            None,
            player_context(
                entity_id,
                position.y,
                descending,
                held.map(|s| s.item.as_ref()),
            ),
        );
        let Some(body) = self.native_player_body_mut(target) else {
            return Ok(false);
        };
        body.ground = ground;
        if let Some((conn, host)) = conn.zip(self.lan.as_mut()) {
            host.hub
                .update_guest(conn, |guest| guest.on_ground = on_ground);
        }
        scan.map(|()| true)
            .map_err(|_| LevelWriteError::Unavailable)
    }

    /// The entity's box (`Entity.getBoundingBox`): its lowest and highest
    /// corner. Displays and markers have zero-sized physical boxes; interactions use
    /// their saved dimensions. End crystals have their 2 x 2 box. None for the dragon; a dragon part's is its own. API 379.
    pub(super) fn entity_bounding_box(&self, entity: EntityRef) -> Option<[[f64; 3]; 2]> {
        if let Some(row) = self.native_client_general(entity) {
            return Some(corners(row.bounds()));
        }
        let t = self.native_entity_read_target(entity)?;
        if let Some(p) = self.native_client_player(t) {
            return Some(corners(p.bounds()));
        }
        if let Some(g) = self.native_target_guest(t) {
            return Some(corners(g.bounds()));
        }
        if let Some(h) = self.native_walk_entity(entity.id()) {
            return Some(corners(with_entity!(self, h, e => e.bounds())));
        }
        let (lo, hi) = match t {
            Target::Other(id) => {
                return self
                    .others
                    .iter()
                    .find(|e| e.id == id)?
                    .display_marker_or_interaction_bounds()
                    .map(corners);
            }
            Target::DragonPart(id) => {
                let (dragon, part) = self.resident_dragon_part(id)?;
                return Some(corners(dragon.part_geometry(part).1));
            }
            Target::Crystal(id) => {
                return Some(corners(self.crystals.iter().find(|c| c.id == id)?.bounds()));
            }
            Target::Cushion(id) => self.cushions.iter().find(|c| c.id == id)?.bounds(),
            Target::Frame(id) => self.item_frames.iter().find(|f| f.id == id)?.bounds(),
            Target::Painting(id) => self.paintings.iter().find(|p| p.id == id)?.bounds(),
            _ => return None,
        };
        // A position that came off the wire can be NaN or infinite: no box.
        if lo.iter().chain(&hi).any(|v| !v.is_finite()) {
            return None;
        }
        Some([lo, hi])
    }
    /// The height of the entity's eyes above its feet (`Entity.getEyeHeight`).
    /// API 379.
    pub(super) fn entity_eye_height(&self, entity: EntityRef) -> Option<f64> {
        Some(self.target_eye_height(self.native_entity_read_target(entity)?))
    }
    /// Where the entity's eyes are (`Entity.getEyePosition`). API
    /// 379.
    pub(super) fn entity_eye_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let [x, y, z] = self.entity_position(entity)?;
        Some([x, y + self.entity_eye_height(entity)?, z])
    }
    /// The unit vector the entity looks along (`Entity.getViewVector`, from
    /// its pitch and yaw). API 379.
    pub(super) fn entity_view_vector(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let [yaw, pitch] = self.entity_rotation(entity)?;
        Some(turned_view_vector(yaw, pitch, 0.0))
    }
    /// The unit vector that points up out of the entity's head
    /// (`Entity.getUpVector`): its view vector turned a quarter of a turn
    /// toward the zenith.
    pub(super) fn entity_up_vector(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let [yaw, pitch] = self.entity_rotation(entity)?;
        Some(turned_view_vector(yaw, pitch, -90.0))
    }
    /// `Entity.mainSupportingBlockPos`, retained by bodies with a shared ground state.
    /// Families that have not moved onto that state still use their existing support read.
    fn native_supporting_block(&self, entity: EntityRef, t: Target) -> Option<BlockPos> {
        match t {
            Target::Player => return self.player.as_ref()?.ground.supporting_block,
            Target::Guest(_) => {
                return self
                    .native_target_guest(t)?
                    .motion
                    .as_ref()?
                    .ground
                    .supporting_block;
            }
            Target::Hook(id) => return self.fishing_hook_by_id(id)?.0.ground.supporting_block,
            _ => {}
        }
        let handle = self.native_walk_entity(entity.id())?;
        if let Some(ground) = with_entity!(self, handle, e => e.ground_state()) {
            return ground.supporting_block;
        }
        if !self.entity_on_ground(entity)? {
            return None;
        }
        let (position, bounds) = with_entity!(self, handle, e => (e.position(), e.bounds()));
        self.supporting_block(position, bounds)
    }
    /// The block the entity stands on (`Entity.getOnPos`). API 379.
    pub(super) fn entity_on_block(&self, entity: EntityRef) -> Option<[i32; 3]> {
        let t = self.native_entity_target(entity)?;
        let [x, y, z] = self.entity_position(entity)?;
        Some(on_block_position(
            self.native_supporting_block(entity, t),
            Vec3::new(x, y, z),
        ))
    }
    /// What the entity looks at within `distance` blocks of its eyes: `Entity.pick`'s block
    /// ray along the view vector, outlines and no fluids.
    pub(super) fn entity_pick(&self, entity: EntityRef, distance: f64) -> Option<ClipHit> {
        if !(distance.is_finite() && distance >= 0.0) {
            return None;
        }
        let from = self.entity_eye_position(entity)?;
        let to = ray_end(from, self.entity_view_vector(entity)?, distance);
        self.level_clip(
            self.entity_level(entity)?,
            from,
            to,
            ClipBlock::Outline,
            ClipFluid::None,
        )
    }
    /// The square of the distance from the entity's feet to `pos`
    /// (`Entity.distanceToSqr`). API 379.
    pub(super) fn entity_distance_to_sqr(&self, entity: EntityRef, pos: [f64; 3]) -> Option<f64> {
        let p = self.entity_position(entity)?;
        let [dx, dy, dz] = std::array::from_fn(|i| p[i] - pos[i]);
        Some(dx * dx + dy * dy + dz * dz)
    }
}

#[cfg(test)]
mod tests {
    use crate::game::entity::Entity as _;
    use mod_api::EntityRef;
    use sunrise_core::geometry::{Aabb, Vec3};

    fn assert_near(got: [f64; 3], want: [f64; 3]) {
        for i in 0..3 {
            assert!((got[i] - want[i]).abs() < 1.0e-4, "{got:?} is not {want:?}");
        }
    }

    /// Unit vectors worked out by hand from `Entity.calculateViewVector`
    /// (`x = -sin(yaw) cos(pitch)`, `y = -sin(pitch)`, `z = cos(yaw) cos(pitch)`),
    /// the look turned 90 degrees up for the up vector.
    #[test]
    fn the_up_vector_tilts_with_the_look() {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        // Level and facing south: straight up.
        assert_near(super::turned_view_vector(0.0, 0.0, -90.0), [0.0, 1.0, 0.0]);
        // Looking down 45 degrees, the top of the head leans forward (south).
        assert_near(
            super::turned_view_vector(0.0, 45.0, -90.0),
            [0.0, half, half],
        );
        // The same look facing west (yaw 90): forward is -x.
        assert_near(
            super::turned_view_vector(90.0, 45.0, -90.0),
            [-half, half, 0.0],
        );
        // Looking straight up, the head's top points back (north when facing south).
        assert_near(
            super::turned_view_vector(0.0, -90.0, -90.0),
            [0.0, 0.0, -1.0],
        );
        // No turn is the view vector itself: south, and down 30 degrees.
        assert_near(super::turned_view_vector(0.0, 0.0, 0.0), [0.0, 0.0, 1.0]);
        assert_near(
            super::turned_view_vector(0.0, 30.0, 0.0),
            [0.0, -0.5, 3.0_f64.sqrt() / 2.0],
        );
    }

    #[test]
    fn on_block_is_the_support_else_the_cell_just_under_the_feet() {
        use sunrise_core::position::BlockPos;
        let feet = Vec3::new(0.9, 65.0, -0.2);
        // A support wins even where it is not under the feet's own column.
        assert_eq!(
            super::on_block_position(Some(BlockPos::new(1, 64, -1)), feet),
            [1, 64, -1]
        );
        // None: the cell under the feet; a hair under, so the block of a flush stand.
        assert_eq!(super::on_block_position(None, feet), [0, 64, -1]);
        assert_eq!(
            super::on_block_position(None, Vec3::new(0.5, 65.5, 0.5)),
            [0, 65, 0]
        );
    }

    #[test]
    fn a_pick_ray_ends_a_distance_along_the_look() {
        assert_eq!(
            super::ray_end([1.0, 2.0, 3.0], [0.0, -1.0, 0.0], 4.5),
            [1.0, -2.5, 3.0]
        );
        assert_eq!(
            super::ray_end([1.0, 2.0, 3.0], [0.6, 0.0, 0.8], 0.0),
            [1.0, 2.0, 3.0]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn on_block_picks_the_support_nearest_the_entity_and_pick_clips_blocks() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::BlockFace;
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, mobs::Mob};
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            // A zombie 0.6 wide straddling x = 1: its box is over blocks 0 and 1.
            game.mobs
                .push(Mob::new(40, kind, Vec3::new(0.9, 65.0, 0.5), 0.0));
            let entity = game.entity(40).unwrap();
            // Only block 1 is under the box: it supports, not the block of the feet's own column.
            game.world.set(1, 64, 0, stone).unwrap();
            let _ = game.mobs[0].update_retained_ground(
                &game.world,
                game.collision,
                &world_items!(game).entity_type_tags,
                true,
                None,
            );
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));
            // Adding a nearer block doesn't change the support until the next check.
            game.world.set(0, 64, 0, stone).unwrap();
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));
            let _ = game.mobs[0].update_retained_ground(
                &game.world,
                game.collision,
                &world_items!(game).entity_type_tags,
                true,
                None,
            );
            assert_eq!(game.entity_on_block(entity), Some([0, 64, 0]));
            game.mobs[0].position = Vec3::new(1.1, 65.0, 0.5);
            let _ = game.mobs[0].update_retained_ground(
                &game.world,
                game.collision,
                &world_items!(game).entity_type_tags,
                true,
                None,
            );
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));
            // Off the ground there is no support: the cell under the feet.
            game.mobs[0].position = Vec3::new(0.9, 65.0, 0.5);
            let _ = game.mobs[0].update_retained_ground(
                &game.world,
                game.collision,
                &world_items!(game).entity_type_tags,
                false,
                None,
            );
            assert_eq!(game.entity_on_block(entity), Some([0, 64, 0]));
            let air = game
                .pack
                .assets
                .registry
                .default_state("minecraft:air")
                .unwrap();
            game.world.set(0, 64, 0, air).unwrap();
            let _ = game.mobs[0].update_retained_ground(
                &game.world,
                game.collision,
                &world_items!(game).entity_type_tags,
                true,
                None,
            );
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));

            // Facing south (yaw 0) level, the eyes see a block two cells ahead.
            game.mobs[0].position = Vec3::new(0.5, 65.0, 0.5);
            let eyes = game.entity_eye_position(entity).unwrap();
            let wall = [0, eyes[1].floor() as i32, 2];
            game.world.set(wall[0], wall[1], wall[2], stone).unwrap();
            let hit = game.entity_pick(entity, 3.0).unwrap();
            assert!(!hit.miss);
            assert_eq!(hit.block, wall);
            assert_eq!(hit.face, BlockFace::North);
            assert_near(hit.location, [0.5, eyes[1], 2.0]);
            // Too short a ray misses, as does one over an empty column.
            assert!(game.entity_pick(entity, 1.0).unwrap().miss);
            game.mobs[0].position = Vec3::new(5.5, 65.0, 0.5);
            assert!(game.entity_pick(entity, 3.0).unwrap().miss);
            // Looking straight down from the stand hits the top of its support.
            game.mobs[0].position = Vec3::new(1.5, 65.0, 0.5);
            game.mobs[0].head_pitch = 90.0;
            let down = game.entity_pick(entity, 3.0).unwrap();
            assert_eq!((down.block, down.face), ([1, 64, 0], BlockFace::Up));
            assert_near(down.location, [1.5, 65.0, 0.5]);
            // Looking down 45 degrees tilts the up vector forward.
            game.mobs[0].head_pitch = 45.0;
            assert_near(
                game.entity_up_vector(entity).unwrap(),
                [
                    0.0,
                    std::f64::consts::FRAC_1_SQRT_2,
                    std::f64::consts::FRAC_1_SQRT_2,
                ],
            );
            // A ray without a length, or with none, or from a stale handle: nothing.
            assert!(game.entity_pick(entity, -1.0).is_none());
            assert!(game.entity_pick(entity, f64::NAN).is_none());
            let stale = EntityRef::new(mod_api::WorldEpoch::new(entity.epoch().get() + 1), 40);
            assert!(game.entity_pick(stale, 3.0).is_none());
            assert!(game.entity_on_block(stale).is_none());
            assert!(game.entity_up_vector(stale).is_none());
        });
    }

    #[test]
    fn corners_are_lowest_then_highest() {
        let b = Aabb::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(super::corners(b), [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
    }

    #[test]
    #[ignore = "daily"]
    fn set_on_ground_refreshes_the_live_footprint_without_moving_the_body() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        use std::sync::Arc;
        use sunrise_client::{
            chunk::Chunk,
            entities::{ArmorStand, ItemEntity},
            items::ItemStack,
            projectiles::{Projectile, ProjectileKind},
        };
        use sunrise_core::position::BlockPos;

        if run_fresh_cases(
            "game::native_entity_motion::tests::set_on_ground_refreshes_the_live_footprint_without_moving_the_body",
            "SUNRISE_GROUND_SETTER_CASE",
            &["support"],
        ) {
            return;
        }
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let snow = registry.default_state("minecraft:powder_snow").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            let supported = [1, 64, 0];
            let under_feet = [0, 64, 0];
            game.world.set(1, 64, 0, snow).unwrap();
            // The feet are over air. Only the edge of this stand is over the snow.
            let position = Vec3::new(0.9, 65.0, 0.5);
            let velocity = Vec3::new(0.25, -0.125, 0.5);
            let mut stand = ArmorStand::new(40, position, 25.0);
            stand.velocity = velocity;
            stand.enchantment_fall_distance = 1.5;
            stand.ground.horizontal_collision = true;
            game.armor_stands.push(stand);
            let entity = game.entity(40).unwrap();
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_ground(game), Some(true));
            assert_eq!(entity.on_block(game), Some(under_feet));
            assert_eq!(entity.set_on_ground(game, false), Ok(true));
            game.armor_stands[0].equipment[2] = Some(ItemStack::new("minecraft:leather_boots", 1));
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_block(game), Some(supported));
            game.world.set(1, 64, 0, air).unwrap();
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_block(game), Some(supported));
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_block(game), Some(under_feet));
            assert_eq!(entity.on_ground(game), Some(true));
            game.world.set(1, 64, 0, snow).unwrap();
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_block(game), Some(supported));
            assert_eq!(entity.set_on_ground(game, false), Ok(true));
            assert_eq!(entity.on_block(game), Some(under_feet));
            let stand = &game.armor_stands[0];
            assert_eq!(stand.ground.supporting_block, None::<BlockPos>);
            assert!(!stand.ground.on_ground_no_blocks);
            assert!(stand.ground.horizontal_collision);
            assert_eq!(stand.enchantment_fall_distance, 1.5);
            assert_eq!(
                (stand.position, stand.velocity, stand.yaw),
                (position, velocity, 25.0)
            );
            let stale = EntityRef::new(mod_api::WorldEpoch::new(entity.epoch().get() + 1), 40);
            assert_eq!(stale.set_on_ground(game, true), Ok(false));
            assert_eq!(entity.on_ground(game), Some(false));

            // A scan with invalid bounds fails, but vanilla sets the flag before the query.
            game.armor_stands[0].position.y = f64::INFINITY;
            assert_eq!(
                entity.set_on_ground(game, true),
                Err(mod_api::LevelWriteError::Unavailable)
            );
            assert_eq!(entity.on_ground(game), Some(true));

            // A fall over 2.5 blocks selects PowderSnowBlock's 0.9f-high shape.
            // Only the edge of either body overlaps the snow.
            let feet = Vec3::new(0.95, 64.0 + f64::from(0.9_f32), 0.5);
            let mut shot = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                feet,
                None,
            );
            shot.id = 41;
            shot.arrow.as_mut().unwrap().fall_distance = 3.0;
            game.creatures.projectiles.push(shot);
            let projectile = game.entity(41).unwrap();
            assert_eq!(projectile.on_ground(game), Some(false));
            assert_eq!(projectile.set_on_ground(game, true), Ok(true));
            assert_eq!(projectile.on_ground(game), Some(true));
            assert_eq!(projectile.on_block(game), Some(supported));
            assert_eq!(projectile.set_on_ground(game, false), Ok(true));
            assert_eq!(projectile.on_ground(game), Some(false));
            assert_eq!(projectile.on_block(game), Some(under_feet));

            let mut drop = ItemEntity::new(
                42,
                ItemStack::new("minecraft:stone", 1),
                feet,
                Vec3::ZERO,
                0.0,
            );
            drop.fall_distance = 3.0;
            game.dropped.push(drop);
            let item = game.entity(42).unwrap();
            assert_eq!(item.set_on_ground(game, true), Ok(true));
            assert_eq!(item.on_block(game), Some(supported));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn set_on_ground_follows_the_server_players_parked_and_acting_bodies() {
        use super::super::{
            Dim, native_entity_refs::tests::run_fresh_cases, native_guest_turn_tests::with_guests,
        };
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;

        if run_fresh_cases(
            "game::native_entity_motion::tests::set_on_ground_follows_the_server_players_parked_and_acting_bodies",
            "SUNRISE_GROUND_SETTER_PLAYER_CASE",
            &["players"],
        ) {
            return;
        }
        with_guests(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let scaffolding = registry.default_state("minecraft:scaffolding").unwrap();
            let entity = game.entity(12).unwrap();
            assert_eq!(entity.set_on_ground(game, true), Ok(false), "no body yet");
            game.as_guest(12, |_| {}).unwrap();
            // The hub has the latest position; the parked motion still sits elsewhere.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.position = [0.9, 65.0, 0.5])
                .unwrap();
            game.world.set(1, 64, 0, stone).unwrap();
            game.enter_level(Dim::Nether).unwrap();
            // This level has no support at the guest's coordinates. The parked
            // Overworld under their footprint does.
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_ground(game), Some(true));
            assert_eq!(entity.on_block(game), Some([1, 64, 0]));
            game.enter_level(Dim::Overworld).unwrap();
            let hook = game
                .as_guest(12, |game| {
                    assert_eq!(entity.on_ground(game), Some(true));
                    assert_eq!(entity.on_block(game), Some([1, 64, 0]));
                    assert_eq!(entity.set_on_ground(game, false), Ok(true));
                    assert_eq!(entity.on_ground(game), Some(false));
                    // The sneak callback runs after assigning the hub flag, before the body tick.
                    game.world.set(1, 64, 0, scaffolding).unwrap();
                    assert!(!game.player.as_ref().unwrap().shift_key_down);
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(12, |guest| guest.sneaking = true)
                        .unwrap();
                    assert_eq!(entity.set_on_ground(game, true), Ok(true));
                    assert_eq!(entity.on_block(game), Some([0, 64, 0]));
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(12, |guest| guest.sneaking = false)
                        .unwrap();
                    assert_eq!(entity.set_on_ground(game, true), Ok(true));
                    assert_eq!(entity.on_block(game), Some([1, 64, 0]));
                    assert_eq!(entity.set_on_ground(game, false), Ok(true));
                    game.world.set(1, 64, 0, stone).unwrap();

                    // The caster's live hook moves between the acting slot and the hub.
                    game.install_fishing_hook(super::super::fishing::Hook::remote(
                        900,
                        Vec3::new(0.95, 65.0, 0.5),
                        [101, 102, 103, 104],
                    ));
                    let hook = game.entity(900).unwrap();
                    assert_eq!(hook.set_on_ground(game, true), Ok(true));
                    assert_eq!(hook.on_block(game), Some([1, 64, 0]));
                    hook
                })
                .unwrap();
            assert_eq!(hook.on_ground(game), Some(true));
            assert_eq!(hook.on_block(game), Some([1, 64, 0]));
            assert_eq!(hook.set_on_ground(game, false), Ok(true));
            assert_eq!(hook.on_block(game), Some([0, 64, 0]));
            assert_eq!(entity.on_ground(game), Some(false));
            assert_eq!(entity.on_block(game), Some([0, 64, 0]));
            let guest = game.command_guest(12).unwrap();
            assert!(!guest.on_ground);
            assert!(!guest.motion.as_ref().unwrap().ground.on_ground);
            assert_eq!(guest.motion.as_ref().unwrap().ground.supporting_block, None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn reset_fall_distance_forgets_a_fall_for_mobs_carts_and_players() {
        use super::super::minecarts::{CartKind, Minecart};
        use sunrise_client::mobs::Mob;
        fn fake_fall<'a>(game: &'a mut super::super::Game<'_>) -> &'a mut f64 {
            let host = game.lan.as_mut().unwrap();
            let conn = host.hub.guests.iter().find(|g| g.uuid == 2).unwrap().conn;
            &mut host
                .hub
                .guests
                .motion_mut(conn)
                .expect("a fake has a motion")
                .fall_distance
        }
        super::super::native_entity_refs::tests::with_game(|game| {
            super::super::bossbars::audience_tests::add_real_and_fake(game);
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut zombie = Mob::new(40, kind, Vec3::ZERO, 0.0);
            zombie.fall_distance = 12.0;
            game.mobs.push(zombie);
            let mut cart = Minecart::new(41, CartKind::Rideable, Vec3::ZERO);
            cart.fall_distance = 7.0;
            game.carts.push(cart);
            let (zombie, cart) = (game.entity(40).unwrap(), game.entity(41).unwrap());
            assert!(game.entity_reset_fall_distance(zombie));
            assert!(game.entity_reset_fall_distance(cart));
            assert_eq!(game.mobs[0].fall_distance, 0.0);
            assert_eq!(game.carts[0].fall_distance, 0.0);
            // A player: the fake guest's own motion.
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 2, "fake").unwrap().entity();
            *fake_fall(game) = 9.0;
            assert!(game.entity_reset_fall_distance(fake));
            assert_eq!(*fake_fall(game), 0.0);
            // An entity that is gone: nothing.
            game.mobs.clear();
            assert!(!game.entity_reset_fall_distance(zombie));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn only_a_mount_its_rider_controls_takes_the_riders_speed() {
        use super::super::commands::Target;
        use sunrise_client::mobs::{Mob, equine::Rider};
        super::super::native_entity_refs::tests::with_game(|game| {
            for (id, name) in [(30, "minecraft:horse"), (31, "minecraft:pig")] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                let mut m = Mob::new(id, kind, Vec3::ZERO, 0.0);
                m.rider = Some(Rider::default());
                game.mobs.push(m);
            }
            // A horse being broken in, unsaddled: its own speed (AbstractHorse.java:962).
            assert!(!game.native_steered(Target::Mob(30)));
            game.mobs[0].extra.set_saddled(true);
            assert!(game.native_steered(Target::Mob(30)));
            // A saddled pig whose rider holds no stick (Pig.java:97-100).
            game.mobs[1].extra.set_saddled(true);
            assert!(!game.native_steered(Target::Mob(31)));
            game.mobs[1].rider.as_mut().unwrap().steer = true;
            assert!(game.native_steered(Target::Mob(31)));
        });
    }
}
