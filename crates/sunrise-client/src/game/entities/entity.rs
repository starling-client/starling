//! `Entity`: what every kind in the game's entity lists answers to, so a system
//! can treat them alike and only write down where vanilla's kinds differ.
//!
//! Default methods follow vanilla's `Entity`; a kind overrides one only where
//! its vanilla class does. [`EntityHandle`] names one entity by list and index,
//! so a system can walk handles and reach each entity after other borrows of
//! `Game` end. The walk allocates nothing and dispatch is static: hot paths pay
//! no vtable. A new kind is one line in `entity_lists!` and one `impl Entity`.

use super::commands::Target;
use super::persist::ChunkKey;
use super::{FallingBlock, Game, general::GeneralRow};
use super::{
    boats::Boat, fireworks::Rocket, minecarts::Minecart, tnt::PrimedTnt, trident::ThrownTrident,
};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use sunrise_client::collisions::{PowderSnowSubject, SceneCollisions};
use sunrise_client::entities::{
    ArmorStand, Cushion, ExperienceOrb, ItemEntity, ItemFrame, Painting,
};
use sunrise_client::entity_rules::EntityRules;
use sunrise_client::join_seq::JoinSeq;
use sunrise_client::join_seq::Joined as _;
use sunrise_client::mobs::{Mob, Species};
use sunrise_client::projectiles::{Projectile, ProjectileKind};
use sunrise_core::geometry::{Aabb, Direction, Vec3};
use sunrise_core::nbt::Tag;
use sunrise_core::position::BlockPos;
use sunrise_core::random::RandomSource as _;
use sunrise_world::{
    block_collision_scan::CollisionScanError, collision_context::CollisionContext,
    ground_support::GroundState, level::data::PushReaction, move_gate::MoveGate, piston_push,
};

/// Assign a stored owner's delayed id at its existing allocation point.
/// Kinds without a lifetime slot pass None; a bound owner must keep its id.
pub(super) fn assign_stored_id(
    next: &mut u32,
    id: &mut u32,
    lifetime: Option<&sunrise_mod_runtime::attachments::AttachmentLifetime>,
) -> u32 {
    debug_assert!(*id != 0 || lifetime.is_none_or(|slot| !slot.is_bound()));
    super::lan_entities::assign_entity_id(next, id)
}

/// What `Entity.thunderHit` does to a kind: it sets the entity on fire and
/// calls `hurtServer(lightning_bolt, 5)`, so a kind whose `hurtServer` does
/// nothing (projectiles, rockets, TNT, falling blocks, armor stands here) stays
/// untouched. A strike runs the stages in this order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ThunderHit {
    Untouched,
    /// Burns, takes 5 damage, and the kinds with their own `thunderHit` change.
    Mob,
    /// `VehicleEntity.hurtServer`: 5 x 10 is over the 40 that destroys it.
    Vehicle,
    /// `ExperienceOrb.hurtServer`: 5 damage off 5 health.
    Orb,
    /// `ItemEntity.hurtServer`: burns up unless fire resistant.
    Item,
}

/// What a blast knows about itself, for the entities in its reach to answer.
#[derive(Clone, Copy)]
pub(super) struct Blast {
    /// `VehicleEntity.ignoreExplosion`: a mob's blast, with mob griefing off.
    pub(super) vehicles_ignore: bool,
    /// `ItemEntity.ignoreExplosion` (`shouldAffectBlocklikeEntities`).
    pub(super) items_ignore: bool,
    /// The exploding mob, which `getEntities(source, box)` leaves out.
    pub(super) spared: Option<u32>,
}

pub(super) trait Entity: EntityRules {
    /// The kind's serializer overlays its object-owned attachments itself (for
    /// recursive object trees); the common save walk must not encode them twice.
    const SERIALIZES_ATTACHMENTS: bool = false;

    /// `Entity.getId`.
    fn id(&self) -> u32;
    fn position(&self) -> Vec3;
    fn position_mut(&mut self) -> &mut Vec3;
    /// Where it stood when the last tick began (`Entity.lastKnownPosition`'s
    /// reading): `position - previous_position` is `computeSpeed`.
    fn previous_position(&self) -> Vec3;
    /// `Entity.getDeltaMovement`.
    fn velocity(&self) -> Vec3;
    fn velocity_mut(&mut self) -> &mut Vec3;
    /// `Entity.resetFallDistance`: how far it has fallen is forgotten, so its next landing
    /// hurts it as a step does. Required, so a new kind states whether it keeps a fall distance.
    fn reset_fall_distance(&mut self);
    /// `Entity.getYRot` and `getXRot`: yaw, pitch.
    fn rotation(&self) -> [f32; 2];
    /// The rotation part of `Entity.teleportSetPosition`: `setYRot`, `setXRot`
    /// (clamped to -90..90), then `setOldRot`.
    fn set_rotation(&mut self, rotation: [f32; 2]);
    fn bounds(&self) -> Aabb;
    /// What a selector finds this entity as (`@e`), or `None` for one it cannot
    /// name yet. Required, so a new kind states which.
    fn selector_target(&self) -> Option<Target>;
    /// The entity type id a native mod's join hook names (`minecraft:oak_boat`).
    fn native_type(&self) -> Cow<'_, str>;
    /// Saves entity `index` of this kind into its chunk's entity list: the
    /// chunk and the NBT when it lies in `keys` and is saved with its chunk.
    /// Required, so a new kind states how it saves.
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)>;
    /// Loads a saved entity whose type id is `kind` as this kind and joins it to
    /// the world, or hands `tag` back when `kind` is not one this kind loads.
    /// Required, so a new kind states how it loads: it cannot be saved and then
    /// dropped silently on the way back in.
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag>;

    fn on_ground(&self) -> bool {
        self.ground_state().is_some_and(|g| g.on_ground)
    }
    fn body_fall_distance(&self) -> f64 {
        0.0
    }
    fn ground_state(&self) -> Option<&GroundState> {
        None
    }
    fn ground_state_mut(&mut self) -> Option<&mut GroundState> {
        None
    }
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        (
            sunrise_client::motion::BodyMotion::entity_context(self.id(), self.position()),
            Some(PowderSnowSubject {
                fall_distance: self.body_fall_distance(),
                ..Default::default()
            }),
        )
    }
    /// Update the live owner's retained support. None is `Entity.setOnGround`'s null movement.
    fn update_retained_ground(
        &mut self,
        world: &sunrise_client::world::World,
        binding: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
        tags: &sunrise_items::items::EntityTypeTags,
        on_ground: bool,
        movement: Option<Vec3>,
    ) -> Result<bool, CollisionScanError<std::io::Error>> {
        let Some(mut ground) = self.ground_state().copied() else {
            return Ok(false);
        };
        ground.on_ground = on_ground;
        let (context, subject) = self.support_collision(tags);
        let mut scene = SceneCollisions {
            world,
            binding,
            subject,
        };
        let scan = scene.update_ground_support(
            &mut ground,
            self.position(),
            self.bounds(),
            movement,
            context,
        );
        if let Some(owner) = self.ground_state_mut() {
            // Entity.setOnGround writes the flag before checking support.
            *owner = ground;
            return scan.map(|()| true);
        }
        Ok(false)
    }
    /// `Entity.maxUpStep`.
    fn get_step_height(&self) -> f32 {
        0.0
    }
    fn is_alive(&self) -> bool {
        true
    }
    /// `Entity.isRemoved`.
    fn is_removed(&self) -> bool {
        false
    }
    /// `instanceof LivingEntity`.
    fn is_living(&self) -> bool {
        false
    }
    /// `Entity.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        false
    }
    /// Whether it presses plates and tripwires: `getEntitiesOfClass` skips
    /// removed entities, and the block's predicate skips the ones above.
    fn triggers_blocks(&self) -> bool {
        !self.is_removed() && !self.is_ignoring_block_triggers()
    }
    /// Whether `tick_blocks` sends the blocks in its box to `entityInside`
    /// (`applyEffectsFromBlocks`). A kind that function sweeps another way, or
    /// whose tick skips that step, says no.
    fn sweeps_blocks_inside(&self) -> bool {
        true
    }
    /// `Entity.thunderHit` for this kind.
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Untouched
    }
    /// The box a lightning strike tests against.
    fn thunder_bounds(&self) -> Aabb {
        self.bounds()
    }
    /// `Entity.ignoreExplosion`: the blast neither hurts nor pushes it. Also
    /// answers for what `getEntities(source, box)` never lists: the exploding
    /// mob, and the dead.
    fn ignores_explosion(&self, _blast: &Blast) -> bool {
        false
    }
    /// Whether a burst of wind moves it: what `Entity.push` moves, unless the
    /// kind says otherwise.
    fn pushed_by_wind(&self) -> bool {
        self.pushed_by_explosion()
    }
    /// Whether `Entity.push` moves it. An arrow in the ground, a wind charge and
    /// an armour stand (`explosion_breaks_decorations` takes those) stay put.
    fn pushed_by_explosion(&self) -> bool {
        true
    }
    /// The box the blast casts its rays at: `getSeenPercent`.
    fn explosion_box(&self) -> Aabb {
        self.bounds()
    }
    /// How far above its feet `pushFromExplosion` aims: `getEyeHeight`, which
    /// defaults to 0.85 of the box.
    fn explosion_eye_height(&self) -> f64 {
        let b = self.explosion_box();
        f64::from((b.max_position().y - b.min_position().y) as f32 * 0.85f32)
    }
    /// Whether this kind can still be waiting for its tick when a projectile
    /// ticks, though it joined before it (orbs, falling blocks, primed TNT: they
    /// tick between the batches, `game/world/ordered_tick.rs`).
    fn ticks_after_projectiles(&self) -> bool {
        false
    }
    /// Whether a new member of this kind waits for its spawn packet before its first tick.
    fn waits_for_spawn_packet(&self) -> bool {
        false
    }
    /// `Entity.push` with the explosion's knockback.
    fn push_from_explosion(&mut self, push: Vec3) {
        let v = self.velocity_mut();
        *v = Vec3::new(v.x + push.x, v.y + push.y, v.z + push.z);
    }
    /// `Entity.isSupportedBy`.
    fn is_supported_by(&self, pos: BlockPos) -> bool {
        self.ground_state()
            .is_some_and(|g| g.supporting_block == Some(pos))
    }
    /// `Entity.getEyeY`, where `updateFluidHeightAndDoFluidPushing` reads the
    /// eye's fluid; the top of the box unless a kind sets it.
    fn fluid_eye_y(&self) -> f64 {
        self.bounds().max_position().y
    }
    /// `Entity.updateFluidHeightAndDoFluidPushing`: its water and lava heights
    /// (`(0, 0)` for a kind that is not pushed by fluid), and the current of
    /// each fluid added to its velocity. One body for every kind.
    fn push_by_fluid(
        &mut self,
        world: &sunrise_client::world::World,
        fluids: &sunrise_client::motion::FluidResources,
        context: &sunrise_world::native_load_context::LoopContext,
    ) -> (f64, f64) {
        if !self.is_pushed_by_fluid() {
            return (0.0, 0.0);
        }
        let (bounds, at, eye) = (self.bounds(), self.position(), self.fluid_eye_y());
        let (water, lava, _) = sunrise_client::entities::update_entity_fluids(
            bounds,
            at,
            eye,
            self.velocity_mut(),
            world,
            fluids,
            context,
        );
        (water, lava)
    }
    /// `BubbleColumnBlock.entityInside` through `applyEffectsFromBlocks`: a
    /// column lifts or drags every kind alike.
    fn push_by_bubble_column(
        &mut self,
        world: &sunrise_client::world::World,
        registry: &sunrise_world::registry::BlockRegistry,
        facts: &sunrise_client::blocks::BlockFacts,
        collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
    ) -> bool {
        self.push_by_bubble_column_over(world, registry, facts, collision, |_| {})
    }
    /// [`Entity::push_by_bubble_column`], telling `surface` the position of
    /// each column block it lies in (for the bubbles at the top).
    fn push_by_bubble_column_over(
        &mut self,
        world: &sunrise_client::world::World,
        registry: &sunrise_world::registry::BlockRegistry,
        facts: &sunrise_client::blocks::BlockFacts,
        collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
        surface: impl FnMut(BlockPos),
    ) -> bool {
        let bounds = self.bounds();
        sunrise_client::motion::bubble_column_push_over(
            world,
            registry,
            facts,
            collision,
            bounds,
            self.velocity_mut(),
            surface,
        )
    }
    /// `Entity.getPistonPushReaction`.
    fn get_piston_push_reaction(&self) -> PushReaction {
        PushReaction::PushPull
    }
    /// Width and height of the box a portal sweep follows, or None for a kind
    /// the game does not carry through portals (so far falling blocks; fishing
    /// hooks are off the walk, and `FishingHook.canUsePortal` is false in
    /// vanilla).
    fn portal_size(&self) -> Option<(f32, f32)> {
        None
    }
    /// Whether [`Self::portal_size`] is Some, without working the size out (a
    /// mob's depends on its age, scale and pose).
    fn sweeps_portals(&self) -> bool {
        self.portal_size().is_some()
    }
    /// `Entity.canUsePortal(false)` for a kind in `portal_size`. A kind that
    /// cannot stays tracked, so its cooldown and process run out.
    fn can_use_portal(&self, _game: &Game<'_>) -> bool {
        true
    }
    /// A boat or minecart: the kinds whose portal pass opens the tick, before
    /// they move (`AbstractMinecart.tick` handles its portal first).
    fn is_vehicle(&self) -> bool {
        false
    }
    /// Whether movement applies portal block effects before Entity's base tick.
    fn portal_after_move(&self) -> bool {
        false
    }
    /// Where it stood at the start of its tick, for the kinds that store it
    /// (None: `portals.before` has it).
    fn portal_from(&self) -> Option<Vec3> {
        None
    }
    /// Whether this tick's movement can have entered a portal block: a stuck
    /// arrow or a trident in the ground that did not move enters none.
    fn portal_sweeps(&self) -> bool {
        true
    }
    /// `Entity.noPhysics` between ticks: its moves pass through the blocks,
    /// and a shulker's lid leaves it be. `stored` is the value a mod wrote for
    /// a kind that keeps no field of its own (`EntityMemory.no_physics`); None
    /// when none was written, so the kind answers its own default.
    fn no_physics(&self, stored: Option<bool>) -> bool {
        stored.unwrap_or(false)
    }
    /// Apply this kind's resolved movement after the invocation's gate is armed.
    /// Every entity kind states how it moves; family ticks resolve through
    /// the same gate in their own movement path.
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3);
    /// Bounds for `AbstractArrow.shouldFall` after a non-SELF piston move.
    fn piston_release_bounds(&self) -> Option<Aabb> {
        None
    }
    fn release_after_piston(&mut self, _no_block_collision: bool) {}
    /// Set the ground flag for a push, or for the carry pass's collider contact.
    /// Terrain support still comes from `Entity.checkSupportingBlock`; a collider
    /// is not `mainSupportingBlockPos`.
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        if let Some(ground) = self.ground_state_mut() {
            ground.on_ground = on_ground;
        }
    }
    /// `Entity.setOnGround` from outside the entity's own move (`/tp`'s
    /// `TeleportCommand.performTeleport`); kinds that keep no ground flag ignore it.
    fn set_on_ground(&mut self, _on_ground: bool) {}
    /// `PistonMovingBlockEntity.causeBounce`: a slime block sets the velocity
    /// along the movement axis.
    fn piston_slime_bounce(&mut self, movement: Direction) {
        let velocity = self.velocity_mut();
        *velocity = piston_push::bounce_velocity(movement, *velocity);
    }
}

/// Bounds and identity needed by the render occlusion culler. Moving renderers
/// use interpolated entity bounds; attached entities use their own world bounds.
pub(super) trait RenderEntityBounds {
    fn render_id(&self) -> u32;
    fn render_bounds(&self, alpha: f64) -> Aabb;
    fn render_glowing(&self, game: &Game<'_>) -> bool {
        game.tracked_entity_glowing(self.render_id())
    }
}

impl RenderEntityBounds for Mob {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, alpha: f64) -> Aabb {
        // Keep models with large independent appendages on the old envelope:
        // their tentacles and wings can extend several blocks past the entity
        // dimensions. Equipment and custom rendering bypass this pre-check at
        // the call site.
        let (width, height, _) = self.awake_dims();
        let at = self.previous.lerp(self.position, f64::from(alpha as f32));
        if matches!(
            self.species(),
            Species::Guardian | Species::ElderGuardian | Species::Phantom
        ) {
            // The model's root scales its appendages too (the elder guardian
            // uses 2.35), so the margin must grow along with the model.
            let root_scale = self.kind.model.pose[6..]
                .iter()
                .copied()
                .map(f32::abs)
                .fold(1.0, f32::max);
            let radius = width.max(height) * 2.5 + 3.0 * f64::from(root_scale * self.model_scale());
            return Aabb::of_size(at, Vec3::new(radius * 2.0, radius * 2.0, radius * 2.0));
        }

        // Vanilla's ordinary models can rotate their head, limbs and death
        // pose beyond the collision box. This envelope covers those poses while
        // remaining close to the rendered mesh for the common upright mobs.
        // A few quadruped death and riding poses reach several blocks past
        // their standing width. Keep a bounded fallback radius for ordinary
        // models while still shrinking the common pig/zombie envelope below
        // the old five-block radius.
        let horizontal = (width * 2.0).max(height * 1.5).max(3.5);
        let vertical = height * 3.0 + 0.5;
        Aabb::new(
            at.subtract(Vec3::new(horizontal, vertical, horizontal)),
            at.add_vector(Vec3::new(horizontal, vertical, horizontal)),
        )
    }

    fn render_glowing(&self, _game: &Game<'_>) -> bool {
        self.is_currently_glowing()
    }
}

impl RenderEntityBounds for ArmorStand {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        // ArmorStandRenderer poses the limbs, then attaches hand items, head
        // items, armour and folded elytra. Those can extend well beyond the
        // 0.5 x 1.975 collision box (especially raised arms and head banners).
        // This axis-aligned envelope contains every vanilla pose while staying
        // independent of the stand's yaw and partial-tick rotation.
        let scale = if self.small { 0.5 } else { 1.0 };
        let equipped = self.equipment.iter().any(Option::is_some);
        let horizontal = if equipped { 2.0 } else { 1.5 * scale };
        let lower = if equipped { -1.0 } else { -0.5 * scale };
        let upper = 3.5 * scale
            + if self.equipment[5].is_some() {
                1.5
            } else {
                0.0
            };
        Aabb::new(
            at.subtract(Vec3::new(horizontal, -lower, horizontal)),
            at.add_vector(Vec3::new(horizontal, upper, horizontal)),
        )
    }
}

impl RenderEntityBounds for ItemEntity {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        // DroppedItemRenderer bobs, spins, and fans out up to five item copies.
        // The draw path uses the resolved model's exact posed extents; this
        // fallback envelope also covers the largest vanilla ground item.
        Aabb::new(
            at.subtract(Vec3::new(0.75, 0.1, 0.75)),
            at.add_vector(Vec3::new(0.75, 1.0, 0.75)),
        )
    }
}

impl RenderEntityBounds for Boat {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        // The boat model turns with yaw and its animated paddles sweep outside
        // the sized collision box. Hurt and bubble-column rotations are covered
        // by this orientation-independent model envelope.
        Aabb::new(
            at.subtract(Vec3::new(1.75, 0.75, 1.75)),
            at.add_vector(Vec3::new(1.75, 1.5, 1.75)),
        )
    }
}

impl RenderEntityBounds for Minecart {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, alpha: f64) -> Aabb {
        let at = self.previous.lerp(self.position, alpha);
        let offset = self.custom_offset.map_or_else(
            || {
                if self.kind == super::minecarts::CartKind::Hopper {
                    1.0
                } else {
                    6.0
                }
            },
            f64::from,
        );
        let display_shift = 0.75 * (offset - 8.0) / 16.0;
        let display_low = at.y + 0.375 - 0.4 + display_shift;
        let display_high = at.y + 0.375 + 0.4 + display_shift + 0.9;
        Aabb::new(
            Vec3::new(at.x - 1.25, (at.y - 0.5).min(display_low), at.z - 1.25),
            Vec3::new(at.x + 1.25, (at.y + 1.0).max(display_high), at.z + 1.25),
        )
    }
}

fn aabb_from_extents(bounds: ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(bounds.0[0], bounds.0[1], bounds.0[2]),
        Vec3::new(bounds.1[0], bounds.1[1], bounds.1[2]),
    )
}

impl RenderEntityBounds for Painting {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, _alpha: f64) -> Aabb {
        aabb_from_extents(self.bounds())
    }
}

impl RenderEntityBounds for ItemFrame {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, _alpha: f64) -> Aabb {
        let (lo, hi) = self.bounds();
        let step = self.step().map(f64::from);
        let center: [f64; 3] = std::array::from_fn(|axis| (lo[axis] + hi[axis]) * 0.5);
        // Maps fill a full block, while the held item projects toward the
        // camera from the frame plane. Include both in the occlusion envelope.
        let half: [f64; 3] = std::array::from_fn(|axis| if step[axis] != 0.0 { 0.75 } else { 0.5 });
        aabb_from_extents((
            std::array::from_fn(|axis| center[axis] - half[axis]),
            std::array::from_fn(|axis| center[axis] + half[axis]),
        ))
    }
}

impl RenderEntityBounds for Cushion {
    fn render_id(&self) -> u32 {
        self.id
    }

    fn render_bounds(&self, _alpha: f64) -> Aabb {
        aabb_from_extents(self.bounds())
    }
}

/// Whether this projectile still lives its list's tick: an arrow's state was
/// not removed. A thrown projectile leaves its list through the batch's own
/// keep flags, not a field of its own.
fn shot_alive(shot: &Projectile) -> bool {
    shot.arrow.as_ref().is_none_or(|a| !a.removed)
}

/// The item's view of the shared inside walk: its box, its life, the batch's
/// blocks, and the part's contact, fired mid-walk over a lent level. An item
/// has no per-block vanilla arms of its own in the walk: its fire, lava and
/// cactus contacts are its own tick's (`ItemEntity::environment_tick`), kept
/// where they were.
pub(in crate::game) struct ItemInside<'a, 'g> {
    pub(in crate::game) game: &'a mut Game<'g>,
    pub(in crate::game) item: &'a mut ItemEntity,
}

impl sunrise_client::block_contact::InsideWalk for ItemInside<'_, '_> {
    type Effects = mod_api::InsideEffects;
    type Error = ();
    fn bounds_at(&self, at: Vec3) -> Aabb {
        let b = self.item.bounds();
        let d = at.subtract(self.item.position);
        Aabb::new(
            b.min_position().add_vector(d),
            b.max_position().add_vector(d),
        )
    }
    fn is_alive(&self) -> bool {
        self.item.health > 0
    }
    fn block(&self, at: sunrise_core::position::BlockPos) -> sunrise_world::registry::BlockStateId {
        self.game.world.block(at.x, at.y, at.z)
    }
    fn is_air(&self, state: sunrise_world::registry::BlockStateId) -> bool {
        self.game.pack.assets.registry.state(state).is_none_or(|d| {
            matches!(
                d.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    }
    fn inside_shape(
        &mut self,
        _state: sunrise_world::registry::BlockStateId,
        _at: sunrise_core::position::BlockPos,
    ) -> sunrise_client::block_contact::InsideShape {
        // `getEntityInsideCollisionShape` is the whole block for every block
        // an item meets, as the item's own sweep always treated it.
        sunrise_client::block_contact::InsideShape::Full
    }
    fn entity_inside(
        &mut self,
        at: sunrise_core::position::BlockPos,
        state: sunrise_world::registry::BlockStateId,
        _step: i32,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) -> std::result::Result<(), ()> {
        let id = self.item.id;
        self.game
            .fire_batch_inside(id as i32, "minecraft:item", state, at, precise, effects);
        Ok(())
    }
}

/// The projectile's view of the shared inside walk, the item's shape of it.
pub(in crate::game) struct ProjectileInside<'a, 'g> {
    pub(in crate::game) game: &'a mut Game<'g>,
    pub(in crate::game) shot: &'a mut Projectile,
}

impl sunrise_client::block_contact::InsideWalk for ProjectileInside<'_, '_> {
    type Effects = mod_api::InsideEffects;
    type Error = ();
    fn bounds_at(&self, at: Vec3) -> Aabb {
        let b = self.shot.bounds();
        let d = at.subtract(self.shot.position);
        Aabb::new(
            b.min_position().add_vector(d),
            b.max_position().add_vector(d),
        )
    }
    fn is_alive(&self) -> bool {
        shot_alive(self.shot)
    }
    fn block(&self, at: sunrise_core::position::BlockPos) -> sunrise_world::registry::BlockStateId {
        self.game.world.block(at.x, at.y, at.z)
    }
    fn is_air(&self, state: sunrise_world::registry::BlockStateId) -> bool {
        self.game.pack.assets.registry.state(state).is_none_or(|d| {
            matches!(
                d.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        })
    }
    fn inside_shape(
        &mut self,
        _state: sunrise_world::registry::BlockStateId,
        _at: sunrise_core::position::BlockPos,
    ) -> sunrise_client::block_contact::InsideShape {
        sunrise_client::block_contact::InsideShape::Full
    }
    fn entity_inside(
        &mut self,
        at: sunrise_core::position::BlockPos,
        state: sunrise_world::registry::BlockStateId,
        _step: i32,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) -> std::result::Result<(), ()> {
        let (id, kind) = (self.shot.id, self.shot.entity_type());
        self.game
            .fire_batch_inside(id as i32, kind, state, at, precise, effects);
        Ok(())
    }
}
#[cfg(test)]
mod render_bounds_tests {
    use super::*;

    #[test]
    fn armor_stand_bounds_include_raised_equipment_space() {
        let mut stand = ArmorStand::new(1, Vec3::new(4.0, 64.0, -2.0), 90.0);
        stand.show_arms = true;
        stand.pose[2] = [180.0, 0.0, 45.0];
        stand.equipment[0] = Some(sunrise_client::items::ItemStack {
            item: "minecraft:iron_sword".into(),
            count: 1,
            damage: 0,
            extra: None,
        });
        stand.equipment[5] = Some(sunrise_client::items::ItemStack {
            item: "minecraft:banner".into(),
            count: 1,
            damage: 0,
            extra: None,
        });
        let bounds = stand.render_bounds(0.5);
        assert!(bounds.min_position().x <= 2.0);
        assert!(bounds.max_position().x >= 6.0);
        assert!(bounds.min_position().y <= 63.0);
        assert!(bounds.max_position().y >= 69.0);
        assert!(bounds.min_position().z <= -4.0);
        assert!(bounds.max_position().z >= 0.0);
    }

    #[test]
    fn boat_bounds_cover_the_rotated_paddle_sweep() {
        let boat = Boat::new(2, "oak".into(), Vec3::new(0.0, 10.0, 0.0), 90.0);
        let bounds = boat.render_bounds(0.5);
        assert!(bounds.min_position().x <= -1.75);
        assert!(bounds.max_position().x >= 1.75);
        assert!(bounds.min_position().y <= 9.25);
        assert!(bounds.max_position().y >= 11.5);
        assert!(bounds.min_position().z <= -1.75);
        assert!(bounds.max_position().z >= 1.75);
    }

    #[test]
    fn item_frame_bounds_include_the_full_map_and_held_item_depth() {
        let frame = ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 3,
            pos: [0, 0, 0],
            facing: 5,
            glow: false,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            ticks: 0,
            custom_name: None,
            random: sunrise_core::random::LegacyRandom::new(0),
        };
        let bounds = frame.render_bounds(1.0);
        assert!(bounds.min_position().x <= -0.71875);
        assert!(bounds.max_position().x >= 0.78125);
        assert!(bounds.min_position().y <= 0.0);
        assert!(bounds.max_position().y >= 1.0);
        assert!(bounds.min_position().z <= 0.0);
        assert!(bounds.max_position().z >= 1.0);
    }

    #[test]
    fn minecart_bounds_follow_custom_display_offset() {
        let mut cart = Minecart::new(
            4,
            super::super::minecarts::CartKind::Chest,
            Vec3::new(0.0, 10.0, 0.0),
        );
        cart.custom_offset = Some(32);
        let bounds = cart.render_bounds(1.0);
        assert!(bounds.min_position().y <= 9.5);
        assert!(bounds.max_position().y >= 12.8);
    }
}

/// The common retained support owned by moving bodies.
macro_rules! stored_ground {
    () => {
        stored_ground!(fall_distance);
    };
    ($fall_distance:ident) => {
        fn body_fall_distance(&self) -> f64 {
            self.$fall_distance
        }
        fn ground_state(&self) -> Option<&sunrise_world::ground_support::GroundState> {
            Some(&self.ground)
        }
        fn ground_state_mut(&mut self) -> Option<&mut sunrise_world::ground_support::GroundState> {
            Some(&mut self.ground)
        }
    };
}

/// The fields every kind stores under the same names.
macro_rules! stored_motion {
    () => {
        fn id(&self) -> u32 {
            self.id
        }
        fn position(&self) -> Vec3 {
            self.position
        }
        fn position_mut(&mut self) -> &mut Vec3 {
            &mut self.position
        }
        fn previous_position(&self) -> Vec3 {
            self.previous
        }
        fn velocity(&self) -> Vec3 {
            self.velocity
        }
        fn velocity_mut(&mut self) -> &mut Vec3 {
            &mut self.velocity
        }
    };
}

/// Apply the common non-damage `Entity` writes to the state the mover actually owns.
/// An absent field means this mover does not track that effect.
pub(super) fn apply_contact_writes(
    writes: Vec<mod_api::EntityWrite>,
    velocity: &mut Vec3,
    mut fall_distance: Option<&mut f64>,
    mut stuck: Option<&mut Vec3>,
    mut fire_ticks: Option<&mut i32>,
) {
    use mod_api::EntityWrite;
    for write in writes {
        match write {
            EntityWrite::Push([x, y, z]) => *velocity = velocity.add_vector(Vec3::new(x, y, z)),
            EntityWrite::SetMovement([x, y, z]) => *velocity = Vec3::new(x, y, z),
            EntityWrite::StuckIn([x, y, z]) => {
                if let Some(stuck) = stuck.as_deref_mut() {
                    *stuck = Vec3::new(x, y, z);
                }
                if let Some(fall) = fall_distance.as_deref_mut() {
                    *fall = 0.0;
                }
            }
            EntityWrite::ResetFallDistance => {
                if let Some(fall) = fall_distance.as_deref_mut() {
                    *fall = 0.0;
                }
            }
            EntityWrite::SetFireTicks(ticks) => {
                if let Some(fire) = fire_ticks.as_deref_mut() {
                    *fire = ticks;
                }
            }
            EntityWrite::AddEffect { .. } | EntityWrite::Hurt { .. } => {}
        }
    }
}

impl Game<'_> {
    /// `Block.fallOn` effects for bodies whose `causeFallDamage` does nothing.
    pub(super) fn nonliving_fall_on(
        &mut self,
        id: u32,
        feet: Vec3,
        contact: sunrise_client::motion::BodyContact,
    ) {
        // FarmlandBlock draws the level's float before checking LivingEntity.
        match &*self.res.facts().get(contact.state).name {
            "minecraft:farmland" => {
                self.world.random.next_float();
            }
            "minecraft:honey_block" => {
                let at = [feet.x, feet.y, feet.z];
                if !self.entity_silent(id) {
                    self.audio
                        .play("minecraft:block.honey_block.slide", Some(at), 1.0, 1.0);
                }
                self.lan_entity_event(id as i32, 54, at);
            }
            _ => {}
        }
    }

    /// Finish a body bounce before the caller changes the live entity's delta movement.
    pub(super) fn finish_body_restitution(
        &mut self,
        (id, kind): (u32, &str),
        feet: Vec3,
        response: sunrise_client::motion::BodyRestitution,
    ) -> Vec3 {
        let Some(bounce) = response.bounce else {
            return response.velocity;
        };
        // ShelfMushroomBlock calls ClientLevel.playSound with a null player, and
        // ClientLevel.gameEvent does nothing. The authoritative level sends both.
        if self.options.execution_role.is_client() || self.remote.is_some() {
            return response.velocity;
        }
        if !matches!(kind, "minecraft:item" | "minecraft:tnt")
            && self.motion.landing.has_shelf_bounce(bounce.state)
        {
            let pos = bounce.position;
            self.audio.play(
                "minecraft:block.shelf_mushroom.bounce",
                Some([
                    f64::from(pos.x) + 0.5,
                    f64::from(pos.y) + 0.5,
                    f64::from(pos.z) + 0.5,
                ]),
                1.0,
                1.0,
            );
        }
        let movement = sunrise_client::block_contact::bounce(
            &mut sunrise_client::health::DamageLevel::of(
                &mut self.world,
                self.pack.assets.registry,
                Some(world_items!(self)),
                Some(&self.pack.block_registries),
                &self.damage_hooks,
            )
            .no_push(),
            sunrise_client::block_contact::Contact {
                entity: id as i32,
                kind,
                state: bounce.state,
                position: bounce.position,
            },
            bounce.fall_distance,
            response.velocity,
        );
        let context = self.entity_event_context(id, None);
        self.post_game_event("minecraft:bounce", [feet.x, feet.y, feet.z], context);
        self.persist.sync_body_position.insert(id);
        movement
    }
}

/// One `ItemEntity.tick`, with the fluid push and the bubble column decided by
/// the `Entity` defaults every kind shares: fluids, move, column, settle,
/// fluids again. Returns false when the item should be removed. A server's tick
/// runs the halves itself, with the landing's `Block.fallOn` between them.
impl Game<'_> {
    /// Tick an item owned by the caller. The two results are ordinary survival
    /// and explicit consumption by a block-contact handler. `escape_speed`:
    /// the item is inside a block and moves without physics this tick, pushed
    /// out at this speed.
    /// The item's `Entity.checkInsideBlocks`, run at its batch with the part's
    /// contact fired mid-walk: the shared sweep over the item's move, the staged
    /// fire over a lent level (as `item_fall_on` lends one), and the flush of
    /// the queued writes into this item, its own tick. Nothing runs without a
    /// listening part: one `any_inside` test is the whole cost.
    pub(in crate::game) fn sweep_item_inside(&mut self, item: &mut ItemEntity) {
        if !sunrise_client::block_contact::HOOKS
            .get()
            .is_some_and(|hooks| (hooks.any_inside)())
        {
            return;
        }
        let mut effects = mod_api::InsideEffects::default();
        let mut inside = ItemInside { game: self, item };
        let movement = sunrise_world::entity_move::MovementRecord {
            from: inside.item.previous,
            to: inside.item.position,
            original: None,
        };
        let mut visited = crate::fast_hash::FxHashSet::default();
        let _ = sunrise_client::block_contact::sweep_movements(
            &mut inside,
            &[movement],
            &mut visited,
            &mut effects,
        );
        let ItemInside { game, item } = inside;
        // `applyAndClear`: the writes apply in the collector's order, in this
        // item's tick, until it is destroyed.
        for planned in effects.take_planned() {
            if item.health <= 0 {
                break;
            }
            if let mod_api::PlannedInside::Write(write) = planned {
                game.apply_item_write(item, write);
            }
        }
    }

    /// Apply a contact Part's write to the moving item. Items hold a multiplier
    /// for the next move (`makeStuckInBlock`), but keep no active effects.
    fn apply_item_write(&mut self, item: &mut ItemEntity, write: mod_api::EntityWrite) {
        let triple = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        match write {
            mod_api::EntityWrite::Push(v) => {
                item.velocity = item.velocity.add_vector(triple(v));
            }
            mod_api::EntityWrite::SetMovement(v) => item.velocity = triple(v),
            mod_api::EntityWrite::StuckIn(v) => {
                item.stuck = triple(v);
                item.fall_distance = 0.0;
            }
            mod_api::EntityWrite::Hurt {
                damage_type,
                amount,
            } => {
                if let Some(name) =
                    mod_api::__private::parts_installation::damage_type_name(damage_type)
                {
                    self.hurt_moving_item(item, name, amount);
                }
            }
            mod_api::EntityWrite::ResetFallDistance => item.fall_distance = 0.0,
            mod_api::EntityWrite::SetFireTicks(ticks) => item.fire_ticks = ticks,
            mod_api::EntityWrite::AddEffect { .. } => {}
        }
    }

    /// The projectile's `Entity.checkInsideBlocks`, the same batch sweep the
    /// item's is: the shared walk over its move, the staged fire over a lent
    /// level, and the flush into this projectile's own tick.
    pub(in crate::game) fn sweep_projectile_inside(&mut self, shot: &mut Projectile) {
        if !sunrise_client::block_contact::HOOKS
            .get()
            .is_some_and(|hooks| (hooks.any_inside)())
        {
            return;
        }
        let mut effects = mod_api::InsideEffects::default();
        let mut inside = ProjectileInside { game: self, shot };
        let movement = sunrise_world::entity_move::MovementRecord {
            from: inside.shot.previous,
            to: inside.shot.position,
            original: None,
        };
        let mut visited = crate::fast_hash::FxHashSet::default();
        let _ = sunrise_client::block_contact::sweep_movements(
            &mut inside,
            &[movement],
            &mut visited,
            &mut effects,
        );
        let ProjectileInside { game, shot } = inside;
        for planned in effects.take_planned() {
            if !shot_alive(shot) {
                break;
            }
            if let mod_api::PlannedInside::Write(write) = planned {
                game.apply_projectile_write(shot, write);
            }
        }
    }

    /// `Projectile.hurtServer` from lava's `lavaHurt` this tick: `markHurt`,
    /// unless the projectile is invulnerable to lava (`isInvulnerableToBase`).
    pub(in crate::game) fn lava_mark_hurt(&mut self, shot: &mut Projectile) {
        if !shot.take_lava_marked() {
            return;
        }
        let vanilla = self.saved_refuses_damage(shot.id, "minecraft:lava", false);
        if !self.refuses_lava_hurt(shot.id, shot.entity_type(), vanilla) {
            shot.sync_velocity = true;
        }
    }

    /// A typed entity write for a projectile, applied in its own tick. A
    /// projectile takes no effects, no damage table and keeps no stuck
    /// multiplier: `Hurt` and `AddEffect` are skipped, `StuckIn` scales the
    /// movement once, as vanilla's projectiles keep none of the three.
    fn apply_projectile_write(&mut self, shot: &mut Projectile, write: mod_api::EntityWrite) {
        let triple = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        match write {
            mod_api::EntityWrite::Push(v) => {
                shot.velocity = shot.velocity.add_vector(triple(v));
            }
            mod_api::EntityWrite::SetMovement(v) => shot.velocity = triple(v),
            mod_api::EntityWrite::StuckIn(v) => {
                shot.velocity = Vec3::new(
                    shot.velocity.x * v[0],
                    shot.velocity.y * v[1],
                    shot.velocity.z * v[2],
                );
            }
            mod_api::EntityWrite::ResetFallDistance => shot.fall_distance = 0.0,
            mod_api::EntityWrite::SetFireTicks(ticks) => shot.fire_ticks = ticks,
            mod_api::EntityWrite::Hurt { .. } | mod_api::EntityWrite::AddEffect { .. } => {}
        }
    }

    /// An `entity_inside` part's contact for a kind the batch ticks, fired
    /// mid-walk over a lent level, the whole callback staged: only an answering
    /// callback's writes join the walk, and a panic there is contained, leaving
    /// the batch's rows and the level as they were. The level is lent and given
    /// back per contact, as `item_fall_on` lends its own (#2194's carrier
    /// discipline).
    pub(in crate::game) fn fire_batch_inside(
        &mut self,
        entity: i32,
        kind: &'static str,
        state: sunrise_world::registry::BlockStateId,
        at: sunrise_core::position::BlockPos,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) {
        let Some(dimension) = sunrise_client::block_contact::current_level() else {
            return;
        };
        if !sunrise_client::block_contact::HOOKS
            .get()
            .is_some_and(|hooks| (hooks.inside_listens)(state))
        {
            return;
        }
        let mut staged = mod_api::InsideEffects::default();
        let mut writes = Vec::new();
        let mut lent = sunrise_client::health::LentLevel::take(&mut self.world);
        let mut level = sunrise_client::health::DamageLevel::lent(
            &self.world,
            self.pack.assets.registry,
            &mut lent,
            Some(world_items!(self)),
            Some(&self.pack.block_registries),
            &self.damage_hooks,
        )
        .with_effects(crate::game::entity::hit_effects_of!(self, None));
        if let Some(hooks) = sunrise_client::block_contact::HOOKS.get() {
            (hooks.inside_fire)(
                &mut level,
                dimension,
                entity,
                kind,
                state,
                at,
                precise,
                &mut staged,
                &mut writes,
            );
        }
        lent.give_back(&mut self.world);
        effects.absorb_step(staged);
        sunrise_client::block_contact::inside_written(state, at, writes);
    }

    pub(super) fn body_motion<'a>(
        &'a self,
        side: &'a dyn sunrise_client::motion::SideLevel,
    ) -> sunrise_client::motion::BodyMotion<'a> {
        sunrise_client::motion::BodyMotion {
            world: &self.world,
            collision: self.collision,
            facts: self.res.facts(),
            blocks: Some(&self.res.motion().blocks),
            context: &self.loop_context,
            side: Some(side),
        }
    }

    /// Step on the retained block before the simulating body's inside scan.
    /// Client copies edit motion only; the server hands the writes to the body owner.
    pub(super) fn body_step_on(
        &mut self,
        entity: (u32, &str),
        footprint: (Vec3, Aabb),
        ground: &sunrise_world::ground_support::GroundState,
        velocity: &mut Vec3,
    ) -> Vec<mod_api::EntityWrite> {
        use sunrise_client::block_contact::{StepLevel, step_listens, step_on};
        let side = game_side!(self);
        let motion = self.body_motion(&side);
        if self.options.execution_role.is_client() || self.remote.is_some() {
            motion.step_on(entity, footprint, ground, velocity);
            return Vec::new();
        }
        let Some(step) = motion.step_contact(entity, footprint, ground, velocity) else {
            return Vec::new();
        };
        if !step_listens(step.contact.state) {
            return Vec::new();
        }
        let mut level = sunrise_client::health::DamageLevel::of(
            &mut self.world,
            self.pack.assets.registry,
            Some(world_items!(self)),
            Some(&self.pack.block_registries),
            &self.damage_hooks,
        );
        step_on(StepLevel::Server(&mut level), step, velocity)
    }

    pub(super) fn tick_item_with_block_contacts(
        &mut self,
        item: &mut ItemEntity,
        escape_speed: Option<f32>,
        entity_boxes: &[Aabb],
        client: bool,
        splash: &mut impl FnMut(sunrise_client::entities::ItemSplash),
    ) -> (bool, bool) {
        let no_physics = escape_speed.is_some();
        // Entity.baseTick splashes before moving, except on the first tick, at
        // the motion before the currents push it.
        let before = item.velocity;
        let heights =
            item.push_by_fluid(&self.world, &self.res.motion().fluids, &self.loop_context);
        item.water_entry(heights.0 > 0.0, item.ticks == 0, before, splash);
        item.lava_fall(heights.1 > 0.0);
        let mut step = item.tick_move_before_block_effects(
            &self.world,
            self.res.facts(),
            self.collision,
            no_physics,
            escape_speed,
            heights,
            entity_boxes,
            Some(self.border.geometry()),
            Some(&self.res.motion().blocks),
            &self.loop_context,
        );
        item.check_fall(&mut step, |fall, from, moved| {
            self.vehicle_fall_reset(fall, from, moved)
        });
        if !client && let Some((on, distance)) = step.landed() {
            self.item_fall_on(item.id, on, distance);
        }
        if let Some(response) = item.move_restitution(self.body_motion(&game_side!(self)), step) {
            item.velocity =
                self.finish_body_restitution((item.id, "minecraft:item"), item.position, response);
        }
        if let Some(pos) = step.vertical_contact {
            let state = self.world.block(pos.x, pos.y, pos.z);
            let position = item.position;
            let mut discarded = false;
            if crate::game::api_values::block_state::block_listens(
                mod_api::HookId::ItemAfterVerticalCollision,
                state,
            ) {
                let mut stack = Some(item.stack.clone());
                self.native_item_after_vertical_collision(
                    crate::game::native_hooks::ItemAfterVerticalCollisionCall {
                        dimension: self.dim.id(),
                        client,
                        entity: item.id,
                        state,
                        block_pos: [pos.x, pos.y, pos.z],
                        item_pos: [position.x, position.y, position.z],
                        stack: &mut stack,
                        discard: &mut discarded,
                    },
                );
                match stack {
                    Some(stack) => item.stack = stack,
                    None => discarded = true,
                }
            }
            if discarded || item.stack.count == 0 {
                return (false, true);
            }
        }
        if !no_physics {
            item.apply_move_speed(self.body_motion(&game_side!(self)), step);
            for write in self.body_step_on(
                (item.id, "minecraft:item"),
                (item.position, item.bounds()),
                &item.ground,
                &mut item.velocity,
            ) {
                self.apply_item_write(item, write);
                if item.health <= 0 {
                    return (false, true);
                }
            }
            if !client {
                self.sweep_item_inside(item);
                if item.health <= 0 {
                    return (false, true);
                }
            }
        }
        item.finish_move_block_effects(&self.world, self.res.facts(), step);
        item.push_by_bubble_column(
            &self.world,
            self.pack.assets.registry,
            self.res.facts(),
            self.collision,
        );
        let alive = item.tick_settle(
            &self.world,
            self.pack.assets.registry,
            self.res.facts(),
            step,
            &self.loop_context,
            self,
        );
        // ItemEntity.tick refreshes immersion and splashes on water entry.
        let before = item.velocity;
        let (water, _) =
            item.push_by_fluid(&self.world, &self.res.motion().fluids, &self.loop_context);
        item.water_entry(water > 0.0, false, before, splash);
        (alive, false)
    }
}

/// The game's lists of entities, in the order systems walk them. Hands them to
/// `$then`, so one list drives the handles and every typed dispatch.
macro_rules! entity_lists {
    ($then:ident, $($args:tt)*) => {
        $crate::game::entity::$then! {
            ($($args)*)
            Mob(Mob) mobs: mobs,
            Item(ItemEntity) dropped: dropped,
            Boat(Boat) boats: boats,
            Cart(Minecart) carts: carts,
            ArmorStand(ArmorStand) armor_stands: armor_stands,
            Tnt(PrimedTnt) tnts: tnts,
            Falling(FallingBlock) falling: falling,
            Orb(ExperienceOrb) orbs: orbs,
            Projectile(Projectile) projectiles: creatures.projectiles,
            Rocket(Rocket) rockets: rockets,
            Trident(ThrownTrident) tridents: tridents,
            General(GeneralRow) general: resident_entities.general.rows,
        }
    };
}
pub(crate) use entity_lists;

macro_rules! direct_entity_edit {
    (Projectile) => {
        false
    };
    (Cart) => {
        false
    };
    (Falling) => {
        false
    };
    (Boat) => {
        false
    };
    ($kind:ident) => {
        true
    };
}
pub(crate) use direct_entity_edit;

macro_rules! define_entity_handles {
    (() $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        /// Every list, borrowed apart from the rest of `Game`, for
        /// [`for_each_entity`] while the world is borrowed mutably.
        #[derive(Clone, Copy)]
        pub(super) struct EntityLists<'w> {
            $(pub(super) $field: &'w [$ty],)*
        }

        /// One entity, by list and index.
        #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
        pub(super) enum EntityHandle {
            $($kind(usize),)*
        }

        /// Allocation-free storage-order walk with stable IDs and verified index hints.
        /// The next ID is captured before the caller can remove the current entity.
        pub(super) struct EntityIdCursor {
            before: JoinSeq,
            $($field: (Option<(u32, JoinSeq)>, usize, usize),)*
        }

        /// Every list moved out of `Game`, for [`for_each_entity_mut`]: the
        /// world stays readable while the entities change.
        pub(super) struct TakenEntities {
            $(pub(super) $field: sunrise_client::join_seq::JoinList<$ty, { $crate::game::entity::direct_entity_edit!($kind) }>,)*
        }


        impl<'w> EntityLists<'w> {
            /// Merge lists already ordered by join number, taking the lowest head at each
            /// step. The cursors live on the stack. The draw pass reads only its own lists.
            fn merge_in_join_order<const DRAWN_ONLY: bool>(self)
                -> impl Iterator<Item = (EntityHandle, JoinSeq)> + use<'w, DRAWN_ONLY>
            {
                let mut cursors = [$(self.$field.len()),*].map(|_| 0usize);
                std::iter::from_fn(move || {
                    let mut best: Option<(usize, EntityHandle, JoinSeq)> = None;
                    let mut list = 0;
                    $(
                        let index = cursors[list];
                        let handle = EntityHandle::$kind(index);
                        if (!DRAWN_ONLY || DrawFamily::of(handle).is_some())
                            && let Some(row) = self.$field.get(index)
                        {
                            let seq = row.join_seq();
                            if best.is_none_or(|(_, _, lowest)| seq < lowest) {
                                best = Some((list, handle, seq));
                            }
                        }
                        list += 1;
                    )*
                    let _ = list;
                    let (list, handle, seq) = best?;
                    cursors[list] += 1;
                    Some((handle, seq))
                })
            }

            /// Visits every entity once in join order. On authoritative lists, tick passes
            /// can reorder storage; report any such list rather than taking it as join order.
            pub(super) fn walk_in_join_order(
                self,
                mut visit: impl FnMut(EntityHandle, JoinSeq),
            ) -> Result<(), JoinOrderError> {
                let mut last = JoinSeq::UNJOINED;
                for (handle, seq) in self.merge_in_join_order::<false>() {
                    if !seq.is_joined() {
                        return Err(JoinOrderError::Unjoined(handle));
                    }
                    if seq <= last {
                        return Err(JoinOrderError::OutOfOrder(handle));
                    }
                    last = seq;
                    visit(handle, seq);
                }
                Ok(())
            }
        }

        impl Game<'_> {
            /// Moves every list out; give it back with `restore_entities`.
            pub(super) fn take_entities(&mut self) -> TakenEntities {
                TakenEntities {
                    $($field: self$(.$list)+.lift(),)*
                }
            }
            pub(super) fn restore_entities(&mut self, taken: TakenEntities) {
                $({
                    let expected = self$(.$list)+.len() + taken.$field.len();
                    self$(.$list)+.restore(taken.$field, sunrise_client::join_seq::RestoreOrder::SurvivorsFirst);
                    debug_assert_eq!(self$(.$list)+.len(), expected);
                })*
            }
            /// Every entity's handle: lengths read now, nothing allocated, and
            /// no borrow of `Game` held, so the caller can change it mid-walk.
            pub(super) fn entity_handles(&self) -> impl Iterator<Item = EntityHandle> + use<> {
                std::iter::empty()
                    $(.chain((0..self$(.$list)+.len()).map(EntityHandle::$kind)))*
            }

            pub(super) fn entity_id_cursor(&self) -> EntityIdCursor {
                EntityIdCursor {
                    before: JoinSeq::upcoming(),
                    $($field: (self$(.$list)+.first().map(|e| (e.id(), e.join_seq())), 0, self$(.$list)+.len()),)*
                }
            }

            /// Verify a stable identity before returning any index-bearing handle.
            /// Removed next identities resume at the next surviving original join.
            /// The captured join ceiling excludes entities spawned by callbacks.
            pub(super) fn next_entity_id(&self, cursor: &mut EntityIdCursor) -> Option<(u32, EntityHandle)> {
                $({
                    let (next, hint, remaining) = &mut cursor.$field;
                    if *remaining > 0 {
                        if let Some((id, join)) = *next {
                            let index = if self$(.$list)+.get(*hint).is_some_and(|e| e.id() == id && e.join_seq() == join) {
                                Some(*hint)
                            } else {
                                self$(.$list)+.iter().position(|e| e.id() == id && e.join_seq() == join).or_else(|| {
                                    self$(.$list)+.iter().position(|e| e.join_seq() > join && e.join_seq() < cursor.before)
                                })
                            };
                            if let Some(i) = index {
                                *remaining -= 1;
                                *hint = i + 1;
                                *next = self$(.$list)+.get(i + 1).filter(|e| e.join_seq() < cursor.before).map(|e| (e.id(), e.join_seq()));
                                return Some((self$(.$list)+[i].id(), EntityHandle::$kind(i)));
                            }
                        }
                        *remaining = 0;
                    }
                })*
                None
            }

            /// Resolve one stable ID, checking its cheap index hint before searching.
            pub(super) fn refresh_entity_handle(&self, id: u32, hint: EntityHandle) -> Option<EntityHandle> {
                match hint {
                    $(EntityHandle::$kind(i) => {
                        if self$(.$list)+.get(i).is_some_and(|e| e.id() == id) { Some(hint) }
                        else { self$(.$list)+.iter().position(|e| e.id() == id).map(EntityHandle::$kind) }
                    })*
                }
            }

            /// Takes the entity out of its list (`Entity.discard` before it ever ticked).
            pub(super) fn discard_entity(&mut self, handle: EntityHandle) {
                match handle {
                    $(EntityHandle::$kind(i) => {
                        self$(.$list)+.remove(i);
                    })*
                }
            }

            /// [`Self::entity_handles`], last of each list first, so the
            /// caller can remove the entity it is on.
            pub(super) fn entity_handles_rev(&self) -> impl Iterator<Item = EntityHandle> + use<> {
                std::iter::empty()
                    $(.chain((0..self$(.$list)+.len()).rev().map(EntityHandle::$kind)))*
            }
        }
    };
}
pub(crate) use define_entity_handles;

/// Why [`EntityLists::walk_in_join_order`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "the tick order work that reads this has not landed yet"
)]
pub(super) enum JoinOrderError {
    /// Never put in its list through the list's own `push`, so it has no
    /// join number.
    Unjoined(EntityHandle),
    /// Joined before an entity the walk had already visited: its list is out
    /// of join order.
    OutOfOrder(EntityHandle),
}

/// The families `entity_geometry` draws in one ordered pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DrawFamily {
    Mob,
    LocalPlayer,
    RemoteGeneral,
    Dragon,
    Crystal,
    Item,
    Orb,
    Falling,
    Tnt,
    ArmorStand,
    Boat,
    Cart,
    Trident,
    Rocket,
}

/// Entities of one family, by list index, that joined back to back as far as the ordered
/// pass is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DrawRun {
    pub(super) family: DrawFamily,
    pub(super) range: std::ops::Range<usize>,
}

impl DrawFamily {
    /// The drawn family of an entity and its index in that family's list; `None` for the
    /// entities the pass does not draw.
    fn of(handle: EntityHandle) -> Option<(Self, usize)> {
        use EntityHandle as H;
        Some(match handle {
            H::Mob(i) => (Self::Mob, i),
            H::Item(i) => (Self::Item, i),
            H::Orb(i) => (Self::Orb, i),
            H::Falling(i) => (Self::Falling, i),
            H::Tnt(i) => (Self::Tnt, i),
            H::ArmorStand(i) => (Self::ArmorStand, i),
            H::Boat(i) => (Self::Boat, i),
            H::Cart(i) => (Self::Cart, i),
            H::Trident(i) => (Self::Trident, i),
            H::Rocket(i) => (Self::Rocket, i),
            _ => return None,
        })
    }
}

/// Draw sources that the shared `EntityLists` walk does not own.
#[derive(Default)]
struct DrawExtras<'a> {
    local_player: Option<JoinSeq>,
    dragon: Option<JoinSeq>,
    crystals: &'a [super::dragon::Crystal],
    general: Option<&'a super::remote::general::RemoteGenerals>,
}

/// The drawn entities as runs in the order they joined the level. Keep the next draw-only
/// entity cached, so the existing body walk only compares its join with that one head.
fn draw_runs(lists: EntityLists<'_>, mut extra: DrawExtras<'_>, runs: &mut Vec<DrawRun>) {
    use DrawFamily as F;
    enum Head {
        LocalPlayer,
        Dragon,
        Crystal(usize),
        General(usize),
    }
    let mut crystals = extra.crystals.iter().enumerate().peekable();
    let mut general = extra
        .general
        .into_iter()
        .flat_map(|g| g.rows.values())
        .enumerate()
        .peekable();
    let mut next_extra = || {
        let (join, head) = [
            extra.local_player.map(|seq| (seq, Head::LocalPlayer)),
            extra.dragon.map(|seq| (seq, Head::Dragon)),
            crystals.peek().map(|&(i, c)| (c.join, Head::Crystal(i))),
            general.peek().map(|&(i, row)| (row.join, Head::General(i))),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|&(seq, _)| seq)?;
        let (family, index) = match head {
            Head::LocalPlayer => {
                extra.local_player = None;
                (F::LocalPlayer, 0)
            }
            Head::Dragon => {
                extra.dragon = None;
                (F::Dragon, 0)
            }
            Head::Crystal(index) => {
                crystals.next();
                (F::Crystal, index)
            }
            Head::General(index) => {
                general.next();
                (F::RemoteGeneral, index)
            }
        };
        Some((join, family, index))
    };
    runs.clear();
    let mut push = |family, index| match runs.last_mut() {
        Some(run) if run.family == family && run.range.end == index => run.range.end += 1,
        _ => runs.push(DrawRun {
            family,
            range: index..index + 1,
        }),
    };
    let mut pending = next_extra();
    for (handle, seq) in lists.merge_in_join_order::<true>() {
        while let Some((join, family, index)) = pending
            && join < seq
        {
            push(family, index);
            pending = next_extra();
        }
        if let Some((family, index)) = DrawFamily::of(handle) {
            push(family, index);
        }
    }
    while let Some((_, family, index)) = pending {
        push(family, index);
        pending = next_extra();
    }
}

impl Game<'_> {
    /// The entities of the families `entity_geometry` draws in one ordered pass, as runs in the
    /// order they joined the level (`EntityLookup.byId`, which
    /// `LevelExtractor.extractVisibleEntities` follows), whatever their family.
    ///
    /// The window sees a client level, including when the server is integrated. Its packet
    /// handlers append freshly joined bodies without reordering survivors. A dropped item
    /// reserves its sequence at AddEntity and takes that slot when its stack arrives. Client
    /// ticks restore lifted rows before nested spawns and put a lent mount back at its old
    /// index. Respawn either keeps the lists or clears them for a new level. The server's
    /// save loads, level swaps, and tick-specific list reorders never feed this merge; they
    /// publish to the window through packets.
    pub(super) fn ordered_draw_runs(
        &self,
        draw_general: bool,
        draw_crystals: bool,
        runs: &mut Vec<DrawRun>,
    ) {
        // A first-person body can draw into the shadow pass only.
        let local_player = self
            .player_model_shown()
            .then(|| self.resident_entities.local_player.join());
        draw_runs(
            entity_lists_of!(self),
            DrawExtras {
                local_player,
                dragon: self.dragon.as_ref().map(|dragon| dragon.join),
                crystals: if draw_crystals { &self.crystals } else { &[] },
                // Pending snapshots do not change the IndexMap's order or the spawn stamp.
                general: self
                    .remote
                    .as_ref()
                    .filter(|_| draw_general)
                    .map(|r| &r.general),
            },
            runs,
        );
    }
}

entity_lists!(define_entity_handles,);
sunrise_client::impl_joined!(FallingBlock);

macro_rules! entity_access {
    (shared, $list:expr, $index:expr, $entity:ident, $body:expr) => {{
        let $entity = &$list[$index];
        $body
    }};
    (exclusive, $list:expr, $index:expr, $entity:ident, $body:expr) => {
        $list.update($index, |$entity| $body)
    };
}
pub(crate) use entity_access;

macro_rules! match_entity {
    (($borrow:ident, $entity:ident, $game:expr, $handle:expr, $body:expr)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        match $handle {
            $($crate::game::entity::EntityHandle::$kind(i) => {
                $crate::game::entity::entity_access!($borrow, $game$(.$list)+, i, $entity, $body)
            })*
        }
    };
}
pub(crate) use match_entity;

/// Runs `$body` with `$entity` bound to the handle's entity, as its own type:
/// one match, each arm compiled for its kind, so nothing goes through a vtable.
macro_rules! with_entity {
    ($game:expr, $handle:expr, $entity:ident => $body:expr) => {
        $crate::game::entity::entity_lists!(match_entity, shared, $entity, $game, $handle, $body)
    };
}
pub(super) use with_entity;

#[path = "entity/list_borrows.rs"]
mod list_borrows;
pub(super) use list_borrows::entity_lists_of;
pub(crate) use list_borrows::{borrow_level_list, borrow_lists};
/// What a hit lends its listeners besides the level ([`sunrise_client::health::HitEffects`]):
/// the level sound queue and sound-seed stream, and the entity the site lends (`$lend`: an
/// `Option<`[`HitLend`](sunrise_client::health::HitLend)`>`, `None` where nothing is lent). Only field borrows of
/// `$game`, so it composes with the borrows of [`damage_level_of`].
macro_rules! hit_effects_of {
    // A site whose `$game` is borrowed whole queues into its own `$sounds`, which it appends to
    // `creatures.lan_sounds` once the level is dropped (the seeds are still drawn at the call).
    ($game:ident, $lend:expr, in $sounds:expr) => {
        sunrise_client::health::HitEffects::new(
            &mut $sounds,
            $game.lan.as_ref().map(|lan| &lan.hub),
            $game.dim.id(),
            $game.native_lifecycle.epoch(),
            $lend,
        )
    };
    ($game:ident, $lend:expr) => {
        sunrise_client::health::HitEffects::new(
            &mut $game.creatures.lan_sounds,
            $game.lan.as_ref().map(|lan| &lan.hub),
            $game.dim.id(),
            $game.native_lifecycle.epoch(),
            $lend,
        )
    };
}
/// The level `$game` ticks, lent to a hit's damage hooks with its block registry and merged
/// tag tables, so a mob or the player beside them can take the hit. `$attack` is the
/// [`Attack`](sunrise_client::health::Attack) the site took first. Every site names what it
/// lends, in one place: its credit (a per-victim plan's loan, `prepare_knockback_credit`, or
/// `no_push` for a site that never makes a living push: a damage type tagged `no_knockback`, or
/// a check that pushes nothing), and the entity whose own random its listeners may draw
/// (`$lend`: the attacker the site holds a live borrow of, built with `HitLend`, or `None` where
/// it cannot lend one).
macro_rules! damage_level_of {
    ($game:ident, $attack:expr, no_push, $lend:expr $(, $equipment:expr)?) => {
        &mut sunrise_client::health::DamageLevel::of(
            &mut $game.world,
            $game.pack.assets.registry,
            Some(world_items!($game)),
            Some(&$game.pack.block_registries),
            &$game.damage_hooks,
        )
        .attacked_by($attack)
        .no_push()
        .with_effects(crate::game::entity::hit_effects_of!($game, $lend))
        $(.with_player_equipment($equipment))?
    };
    ($game:ident, $attack:expr, $credit:expr, $lend:expr) => {
        &mut sunrise_client::health::DamageLevel::of(
            &mut $game.world,
            $game.pack.assets.registry,
            Some(world_items!($game)),
            Some(&$game.pack.block_registries),
            &$game.damage_hooks,
        )
        .attacked_by($attack)
        .with_knockback_credit($credit)
        .with_effects(crate::game::entity::hit_effects_of!($game, $lend))
    };
}
pub(super) use damage_level_of;
/// `$game`'s [`PlayerResolver`](sunrise_client::health::PlayerResolver),
/// borrowing only the fields it reads (`Game::acting_uuid`'s).
macro_rules! player_resolver {
    ($game:ident) => {
        sunrise_client::health::PlayerResolver {
            in_place: $game.player.is_some().then(|| {
                $game
                    .lan_acting_uuid
                    .map(sunrise_client::lan::playerdata::uuid_ints)
                    .or($game.level.player_uuid)
                    .unwrap_or_default()
            }),
            hub: $game.lan.as_ref().map(|lan| &lan.hub),
        }
    };
}
pub(super) use hit_effects_of;
pub(super) use player_resolver;

macro_rules! walk_lists {
    (($lists:expr, $entity:ident, $body:block)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $(for $entity in $lists.$field $body)*
    };
}
pub(crate) use walk_lists;

/// Runs `$body` for every entity, with `$entity` as a reference to its own
/// type: one loop per list, each compiled for its kind, so nothing allocates
/// or goes through a vtable and a check a kind answers with a constant folds
/// away. A kind added to `entity_lists!` joins every walk.
macro_rules! for_each_entity {
    ($lists:expr, $entity:ident => $body:block) => {
        $crate::game::entity::entity_lists!(walk_lists, $lists, $entity, $body)
    };
}
pub(super) use for_each_entity;

macro_rules! walk_lists_mut {
    (($lists:expr, $entity:ident, $body:block)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $($lists.$field.for_each_mut(|$entity| $body);)*
    };
}
pub(crate) use walk_lists_mut;

/// [`for_each_entity`] over [`TakenEntities`], with the entity mutable.
macro_rules! for_each_entity_mut {
    ($lists:expr, $entity:ident => $body:block) => {
        $crate::game::entity::entity_lists!(walk_lists_mut, $lists, $entity, $body)
    };
}
pub(super) use for_each_entity_mut;

/// [`with_entity`], with the entity mutable.
macro_rules! with_entity_mut {
    ($game:expr, $handle:expr, $entity:ident => $body:expr) => {
        $crate::game::entity::entity_lists!(match_entity, exclusive, $entity, $game, $handle, $body)
    };
}
pub(super) use with_entity_mut;

macro_rules! save_lists {
    (($game:expr, $keys:expr, $out:expr, $include_transient:expr)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $(for i in 0..$game$(.$list)+.len() {
            let id = $game$(.$list)+[i].id();
            if ($include_transient || !$game.persist.transient.contains(&id))
                && let Some((key, mut tag)) = <$ty as $crate::game::entity::Entity>::save_nbt($game, i, $keys)
            {
                if !<$ty as $crate::game::entity::Entity>::SERIALIZES_ATTACHMENTS {
                    $crate::game::persist::write_attachments(&mut tag, id, &$game.entity_attachments);
                }
                $out.entry(key).or_default().push((id, $game$(.$list)+[i].join_seq(), tag));
            }
        })*
    };
}
pub(crate) use save_lists;

/// Saves every entity of every list in `entity_lists!` into `$out` by chunk,
/// each with its id and join number (`$out` maps a chunk to `(id, JoinSeq, Tag)`),
/// each through its own `Entity::save_nbt`: one loop per list, static dispatch,
/// nothing allocated beyond the tags. A kind added to the lists cannot be left
/// out, since `save_nbt` has no default.
macro_rules! save_entities_of_every_kind {
    ($game:expr, $keys:expr, $out:expr, $include_transient:expr) => {
        $crate::game::entity::entity_lists!(save_lists, $game, $keys, $out, $include_transient)
    };
}

/// A matched kind consumes only its own explicit token; failed kind probes never do.
macro_rules! load_with_transfer {
    ($game:expr, $state:expr, $ordinary:ident, $transfer:ident, $($arg:expr),+ $(,)?) => {{
        if let Some(state) = $state.take() {
            $game.$transfer($($arg),+, Some(state));
        } else {
            $game.$ordinary($($arg),+);
        }
    }};
}

macro_rules! load_lists {
    (($game:expr, $kind:expr, $uuid:expr, $player:expr, $tag:expr, $transferred:expr)
     $($kind_name:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {{
        let mut tag = $tag;
        $(match <$ty as $crate::game::entity::Entity>::load_nbt($game, $kind, $uuid, $player, tag, $transferred) {
            Ok(()) => return Ok(()),
            Err(back) => tag = back,
        })*
        Err(tag)
    }};
}
pub(crate) use load_lists;

/// Loads a saved entity as the first kind in `entity_lists!` that claims its
/// type id, through that kind's own `Entity::load_nbt`: one test per kind,
/// static dispatch, nothing allocated (the compound moves through). Evaluates
/// to `Err(tag)` when no kind claims it, and `return`s `Ok` from the caller
/// otherwise. A kind added to the lists cannot be left out, since `load_nbt`
/// has no default.
macro_rules! load_entity_of_any_kind {
    ($game:expr, $kind:expr, $uuid:expr, $player:expr, $tag:expr, $transferred:expr) => {
        $crate::game::entity::entity_lists!(
            load_lists,
            $game,
            $kind,
            $uuid,
            $player,
            $tag,
            $transferred
        )
    };
}

/// The type ids the minecart kind loads.
fn loads_as_cart(kind: &str) -> bool {
    super::minecarts::CartKind::from_id(kind).is_some()
}
/// The type id the primed TNT kind loads.
fn loads_as_tnt(kind: &str) -> bool {
    kind == "minecraft:tnt"
}

impl Game<'_> {
    /// The type ids the mob kind loads.
    fn loads_as_mob(&self, kind: &str) -> bool {
        self.res.mob_types().contains_key(kind)
    }
    /// The type ids the boat kind loads.
    fn loads_as_boat(&self, kind: &str) -> bool {
        super::boats::any_boat_wood(kind)
            .is_some_and(|(wood, chest)| self.boat_models.has(wood, chest))
    }
    /// Whether `load_listed` loads `kind` as a vehicle that takes its mob
    /// riders (`take_mob_passengers`): a mob, boat, minecart or primed TNT.
    pub(super) fn listed_kind_takes_riders(&self, kind: &str) -> bool {
        self.loads_as_mob(kind)
            || self.loads_as_boat(kind)
            || loads_as_cart(kind)
            || loads_as_tnt(kind)
    }
    /// Loads a saved entity as the first kind in `entity_lists!` that claims
    /// its type id: `Err` hands the compound back when none does.
    pub(super) fn load_listed(
        &mut self,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
    ) -> Result<(), Tag> {
        load_entity_of_any_kind!(self, kind, uuid, player_uuid, tag, &mut None)
    }
    /// Restore a live portal object with its explicitly captured runtime ownership.
    /// This dispatch covers only reconstructible portal kinds; ordinary disk loads
    /// continue through the canonical kind walk without transfer state.
    pub(super) fn load_listed_transferred(
        &mut self,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        state: super::entity_initialization::TransferredEntityState,
    ) -> Result<(), Tag> {
        let mut state = Some(state);
        load_entity_of_any_kind!(self, kind, uuid, player_uuid, tag, &mut state)
            .or_else(|tag| {
                self.load_unlisted_transferred(
                    Self::chunk_of(sunrise_client::entity_nbt::position(&tag).unwrap_or_default()),
                    kind,
                    uuid,
                    tag,
                    &mut state,
                )
            })
            .or_else(|tag| {
                let saved = tag.clone();
                self.load_other_transferred(tag, kind, state.take())
                    .map(|_| ())
                    .ok_or(saved)
            })
    }

    /// The ids of the entities that are not on the walk of `entity_lists!`.
    pub(super) fn entity_ids_off_walk(&self) -> impl Iterator<Item = u32> + '_ {
        self.cushions
            .iter()
            .map(|e| e.id)
            .chain(self.item_frames.iter().map(|e| e.id))
            .chain(self.paintings.iter().map(|e| e.id))
            .chain(self.others.iter().map(|e| e.id))
            .chain(self.breath_clouds.iter().map(|e| e.id))
            .chain(self.fangs.list.iter().map(|e| e.id))
            .chain(self.crystals.iter().map(|e| e.id))
            .chain(self.dragon.iter().map(|e| e.id))
            .chain(
                self.dragon
                    .iter()
                    .filter(|dragon| dragon.id != 0)
                    .flat_map(|dragon| {
                        (0..dragon.parts.len())
                            .filter_map(move |offset| dragon.id.checked_add(offset as u32 + 1))
                    }),
            )
            .chain(self.ominous_spawners.iter().map(|e| e.id))
            .chain(self.resident_entities.knots.iter().map(|e| e.id))
            // Cast hooks live with their players, not in the saveable lists.
            // Keep both their identities and attachment holders until they leave.
            .chain(self.fishing_hooks_here().map(|(hook, _)| hook.id()))
            .chain(self.natural_knot_id_iter())
    }
    /// The ids of every entity in the level that is not removed: one walk.
    pub(super) fn live_entity_ids(&self) -> HashSet<u32> {
        let mut alive: HashSet<u32> = self.entity_ids_off_walk().collect();
        for_each_entity!(entity_lists_of!(self), entity => {
            if !entity.is_removed() {
                alive.insert(entity.id());
            }
        });
        alive
    }
    /// Saves every entity of every kind that lies in `keys` into `out`, by chunk.
    pub(super) fn save_every_entity(
        &mut self,
        keys: &HashSet<ChunkKey>,
        out: &mut HashMap<ChunkKey, Vec<(u32, JoinSeq, Tag)>>,
        include_transient: bool,
    ) {
        save_entities_of_every_kind!(self, keys, out, include_transient);
    }
}

impl Entity for Mob {
    const SERIALIZES_ATTACHMENTS: bool = true;
    stored_ground!();

    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn support_collision(
        &self,
        tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        let (context, subject) = Mob::support_collision(self, Some(tags));
        (context, Some(subject))
    }
    /// `Entity.noPhysics`: written through the mod API; no mob tick rewrites it
    /// (a vex sets it only for the length of its own, custom travel). Its own
    /// field is the write's store, so `stored` is None for a mob.
    fn no_physics(&self, _stored: Option<bool>) -> bool {
        self.no_physics
    }
    /// `Entity.move(MoverType.PISTON, ...)` ends in `setPos`, which `Shulker`
    /// overrides to snap to the block centre.
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let moved = self.position.add_vector(movement);
        if self.species() == Species::Shulker {
            crate::mobs::shulker::client_set_pos(self, moved.x, moved.y, moved.z);
        } else {
            self.position = moved;
        }
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.head_pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.body_yaw = yaw;
        self.head_yaw = yaw;
        self.head_pitch = pitch.clamp(-90.0, 90.0);
        self.previous_yaw = self.yaw;
        self.previous_head_pitch = self.head_pitch;
    }
    fn native_type(&self) -> Cow<'_, str> {
        super::native_hooks::mob_kind(self).into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_mob(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        _uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !game.loads_as_mob(kind) {
            return Err(tag);
        }
        load_with_transfer!(game, transferred, load_mob, load_mob_transferred, tag, None);
        Ok(())
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((self.width(), self.height()))
    }
    fn sweeps_portals(&self) -> bool {
        true
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.mob_portal_eligible(self)
    }
    /// `PersistentEntitySectionManager.Callback.onRemove`: discard stops
    /// section enumeration immediately, before the entity list is swept.
    fn selector_target(&self) -> Option<Target> {
        (!self.removed && self.death_time == 0).then_some(Target::Mob(self.id))
    }
    fn bounds(&self) -> Aabb {
        Mob::bounds(self)
    }
    fn get_step_height(&self) -> f32 {
        self.kind.step_height as f32
    }
    fn is_alive(&self) -> bool {
        !self.removed && self.alive()
    }
    fn is_removed(&self) -> bool {
        self.removed
    }
    fn is_living(&self) -> bool {
        true
    }
    /// `Bat.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        self.species() == Species::Bat
    }
    /// `Warden.ignoreExplosion`: while it digs or emerges.
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        !self.alive()
            || blast.spared == Some(self.id)
            || (self.species() == Species::Warden
                && sunrise_client::mobs::warden::digging_or_emerging(self))
    }
    /// `getEyeHeight` of the mob as it stands: a baby's, a slime's or a
    /// phantom's own scaled eye height, not the adult kind's. The blast's box
    /// is its real `bounds`, so it needs no override.
    fn explosion_eye_height(&self) -> f64 {
        self.eye_height()
    }
    /// `Entity.push` sets `needsSync`.
    fn push_from_explosion(&mut self, push: Vec3) {
        self.velocity = Vec3::new(
            self.velocity.x + push.x,
            self.velocity.y + push.y,
            self.velocity.z + push.z,
        );
        self.needs_sync = true;
    }
    fn thunder_hit(&self) -> ThunderHit {
        if self.alive() {
            ThunderHit::Mob
        } else {
            ThunderHit::Untouched
        }
    }
    /// The kind's own size, as the strike has always read it.
    fn thunder_bounds(&self) -> Aabb {
        let (p, w) = (self.position, self.kind.width / 2.0);
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + self.kind.height, p.z + w),
        )
    }
}

impl Entity for ItemEntity {
    stored_ground!();
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        (
            self.collision_context(),
            Some(PowderSnowSubject {
                fall_distance: self.fall_distance,
                ..Default::default()
            }),
        )
    }
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn fluid_eye_y(&self) -> f64 {
        ItemEntity::fluid_eye_y(self)
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:item".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_item(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:item") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_item,
            load_item_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.25, 0.25))
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Item(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Item
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.items_ignore
    }
    fn explosion_eye_height(&self) -> f64 {
        Self::EYE_HEIGHT
    }
    fn bounds(&self) -> Aabb {
        ItemEntity::bounds(self)
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        ItemEntity::piston_slime_bounce(self, movement);
    }
}

impl Entity for ExperienceOrb {
    stored_ground!();
    fn reset_fall_distance(&mut self) {
        // An orb keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn ticks_after_projectiles(&self) -> bool {
        true
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:experience_orb".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_orb(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:experience_orb") {
            return Err(tag);
        }
        load_with_transfer!(game, transferred, load_orb, load_orb_transferred, tag, uuid);
        Ok(())
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.5, 0.5))
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Orb(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Orb
    }
    fn explosion_eye_height(&self) -> f64 {
        f64::from(0.5f32 * 0.85f32)
    }
    fn bounds(&self) -> Aabb {
        ExperienceOrb::bounds(self)
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = piston_push::bounce_velocity(movement, self.velocity);
        self.ground.on_ground = false;
        self.ground.clear_support();
    }
}

impl Entity for Boat {
    stored_ground!();
    fn reset_fall_distance(&mut self) {
        // A boat keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
    }
    fn native_type(&self) -> Cow<'_, str> {
        self.entity_type_name().into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_boat(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !game.loads_as_boat(kind) {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_boat,
            load_boat_transferred,
            tag,
            uuid,
            kind
        );
        Ok(())
    }
    stored_motion!();
    fn is_vehicle(&self) -> bool {
        true
    }
    /// `Boat.getEyeY`: the top of the 0.5625 box.
    fn fluid_eye_y(&self) -> f64 {
        self.position.y + super::boats::HEIGHT
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((super::boats::WIDTH as f32, super::boats::HEIGHT as f32))
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.vehicle_can_use_portal(self.id)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Boat(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Vehicle
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.vehicles_ignore
    }
    /// `EntityType.OAK_BOAT & co`: the full 0.5625.
    fn explosion_eye_height(&self) -> f64 {
        0.5625
    }
    fn bounds(&self) -> Aabb {
        Boat::bounds(self)
    }
}

impl Entity for Minecart {
    stored_ground!();
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
    }
    fn native_type(&self) -> Cow<'_, str> {
        self.kind.id().into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_cart(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !loads_as_cart(kind) {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_cart,
            load_cart_transferred,
            tag,
            uuid,
            kind
        );
        Ok(())
    }
    stored_motion!();
    fn is_vehicle(&self) -> bool {
        true
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((
            super::minecarts::WIDTH as f32,
            super::minecarts::HEIGHT as f32,
        ))
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.vehicle_can_use_portal(self.id)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Cart(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Vehicle
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.vehicles_ignore
    }
    fn bounds(&self) -> Aabb {
        Minecart::bounds(self)
    }
    /// `tick_blocks` sweeps minecarts along with the players.
    fn sweeps_blocks_inside(&self) -> bool {
        false
    }
}

/// Width and height `EntityType.ARMOR_STAND` gives a stand.
fn armor_stand_size(stand: &ArmorStand) -> (f64, f64) {
    if stand.marker {
        (0.0, 0.0)
    } else if stand.small {
        (0.25, f64::from(0.9875f32))
    } else {
        (0.5, f64::from(1.975f32))
    }
}

impl Entity for ArmorStand {
    stored_ground!(enchantment_fall_distance);
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        let (context, subject) = ArmorStand::support_collision(self);
        (context, Some(subject))
    }
    fn reset_fall_distance(&mut self) {
        self.enchantment_fall_distance = 0.0;
    }
    /// `LivingEntity.isAlive`: health above 0 (removal is the level's).
    fn is_alive(&self) -> bool {
        self.health > 0.0
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.previous_yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
    }
    fn fluid_eye_y(&self) -> f64 {
        ArmorStand::fluid_eye_y(self)
    }
    fn native_type(&self) -> Cow<'_, str> {
        super::armor_stands::TYPE_ID.into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_armor_stand(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:armor_stand") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_armor_stand,
            load_armor_stand_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Stand(self.id))
    }
    /// `ArmorStand.ignoreExplosion`: a visible stand feels only a burst that
    /// affects block-like entities (`items_ignore` is that flag, negated).
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        self.invisible || blast.items_ignore
    }
    /// `EntityType.ARMOR_STAND`'s size, halved when small; a marker has none.
    fn explosion_box(&self) -> Aabb {
        let (w, h) = armor_stand_size(self);
        let half = w / 2.0;
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - half, p.y, p.z - half),
            Vec3::new(p.x + half, p.y + h, p.z + half),
        )
    }
    fn explosion_eye_height(&self) -> f64 {
        if self.marker {
            0.0
        } else if self.small {
            f64::from(0.9875f32)
        } else {
            f64::from(1.7775f32)
        }
    }
    /// A burst of wind pushes a stand that a blast only breaks.
    fn pushed_by_wind(&self) -> bool {
        true
    }
    fn bounds(&self) -> Aabb {
        ArmorStand::bounds(self)
    }
    fn is_living(&self) -> bool {
        true
    }
    /// `ArmorStand.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        self.marker
    }
    /// A NoGravity stand is not swept for the blocks it stands in (plates,
    /// fire): measured beside the real server, it never presses a plate nor
    /// catches fire, and does again once `NoGravity` is cleared.
    fn sweeps_blocks_inside(&self) -> bool {
        !self.no_gravity
    }
    fn pushed_by_explosion(&self) -> bool {
        false
    }
    /// `Entity.noPhysics`: `Marker` or `NoGravity` unless a mod wrote it,
    /// recomputed from them on load because `readAdditionalSaveData` does.
    /// Its own field is the write's store, so `stored` is None for a stand.
    fn no_physics(&self, _stored: Option<bool>) -> bool {
        self.no_physics.unwrap_or(self.marker || self.no_gravity)
    }
    /// `ArmorStand.getPistonPushReaction`: a marker is ignored.
    fn get_piston_push_reaction(&self) -> PushReaction {
        if self.marker {
            PushReaction::IgnoreEntity
        } else {
            PushReaction::PushPull
        }
    }
    /// `ArmorStand.getDefaultDimensions`: the box a portal sweep follows, and
    /// the size its exit is placed by. `ArmorStand.readAdditionalSaveData`
    /// makes a stand without `hasPhysics` (a marker or a `NoGravity` one)
    /// `noPhysics`, so `Entity.isAffectedByBlocks` never runs its sweep and it
    /// enters no portal; a marker's box is `MARKER_DIMENSIONS`, 0 by 0, too.
    fn portal_size(&self) -> Option<(f32, f32)> {
        if self.marker || self.no_gravity {
            return None;
        }
        let (width, height) = armor_stand_size(self);
        Some((width as f32, height as f32))
    }
}

impl Entity for PrimedTnt {
    stored_ground!();
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        (self.collision_context(), Some(self.powder_snow_subject()))
    }
    fn reset_fall_distance(&mut self) {
        // Primed TNT keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn ticks_after_projectiles(&self) -> bool {
        true
    }
    fn waits_for_spawn_packet(&self) -> bool {
        true
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:tnt".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_tnt(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !loads_as_tnt(kind) {
            return Err(tag);
        }
        load_with_transfer!(game, transferred, load_tnt, load_tnt_transferred, tag, uuid);
        Ok(())
    }
    stored_motion!();
    /// `EntityTypes.TNT`'s explicit `eyeHeight(0.15F)`. Nothing vanilla reads
    /// for TNT depends on it (only `isEyeInFluid` does), but it is the type's.
    fn fluid_eye_y(&self) -> f64 {
        self.position.y + f64::from(0.15_f32)
    }
    /// `PrimedTnt` is 0.98 across.
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.98, 0.98))
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.tnt_can_use_portal(self.id)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Tnt(self.id))
    }
    /// Pushed from its position, not its eyes.
    fn explosion_eye_height(&self) -> f64 {
        0.0
    }
    fn bounds(&self) -> Aabb {
        PrimedTnt::bounds(self)
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = piston_push::bounce_velocity(movement, self.velocity);
        // Leave the ground so the next tick does not apply the on-ground
        // Y * -0.5 friction that cancels the launch.
        self.ground.on_ground = false;
        self.ground.clear_support();
    }
}

impl Entity for FallingBlock {
    stored_ground!();
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        (
            sunrise_client::motion::BodyMotion::entity_context(self.id, self.position),
            Some(PowderSnowSubject {
                fall_distance: self.fall_distance,
                falling_block: true,
                ..Default::default()
            }),
        )
    }
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn ticks_after_projectiles(&self) -> bool {
        true
    }
    fn waits_for_spawn_packet(&self) -> bool {
        true
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:falling_block".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_falling(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:falling_block") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_falling,
            load_falling_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Falling(self.id))
    }
    fn bounds(&self) -> Aabb {
        sunrise_client::falling_block::bounds(self.position)
    }
}

impl Entity for Projectile {
    stored_ground!();
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        (
            sunrise_client::motion::BodyMotion::entity_context(self.id, self.position),
            Some(PowderSnowSubject {
                fall_distance: self
                    .arrow
                    .as_ref()
                    .map_or(self.fall_distance, |arrow| arrow.fall_distance),
                ..Default::default()
            }),
        )
    }
    fn reset_fall_distance(&mut self) {
        if let Some(arrow) = self.arrow.as_deref_mut() {
            arrow.fall_distance = 0.0;
        }
    }
    fn portal_after_move(&self) -> bool {
        !matches!(
            self.kind,
            sunrise_client::projectiles::ProjectileKind::LlamaSpit
                | sunrise_client::projectiles::ProjectileKind::ShulkerBullet { .. }
        )
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
        self.previous_yaw = self.yaw;
        self.previous_pitch = self.pitch;
    }
    /// `Entity.move(MoverType.PISTON)` ends in `setOnGroundWithMovement`.
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.ground.on_ground = on_ground;
    }
    fn set_on_ground(&mut self, on_ground: bool) {
        self.ground.on_ground = on_ground;
    }
    fn native_type(&self) -> Cow<'_, str> {
        self.entity_type().into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_projectile(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        match kind {
            "minecraft:arrow" | "minecraft:spectral_arrow" => {
                load_with_transfer!(
                    game,
                    transferred,
                    load_arrow,
                    load_arrow_transferred,
                    tag,
                    uuid,
                    player_uuid
                )
            }
            "minecraft:llama_spit" | "minecraft:shulker_bullet" => {
                load_with_transfer!(
                    game,
                    transferred,
                    load_spit_or_bullet,
                    load_spit_or_bullet_transferred,
                    tag,
                    uuid,
                    player_uuid
                )
            }
            "minecraft:fireball"
            | "minecraft:small_fireball"
            | "minecraft:dragon_fireball"
            | "minecraft:wind_charge"
            | "minecraft:breeze_wind_charge"
            | "minecraft:wither_skull" => load_with_transfer!(
                game,
                transferred,
                load_fireball,
                load_fireball_transferred,
                tag,
                uuid,
                player_uuid
            ),
            id if sunrise_client::entity_nbt::throwable_kind(id).is_some() => {
                load_with_transfer!(
                    game,
                    transferred,
                    load_throwable,
                    load_throwable_transferred,
                    tag,
                    uuid,
                    player_uuid
                )
            }
            _ => return Err(tag),
        }
        Ok(())
    }
    stored_motion!();
    /// Arrows and the thrown items and fireballs a chunk saves.
    fn portal_size(&self) -> Option<(f32, f32)> {
        (super::nether_portal::is_arrow(self) || super::nether_portal::is_portal_projectile(self))
            .then(|| super::nether_portal::projectile_box(self))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn portal_sweeps(&self) -> bool {
        !super::nether_portal::is_arrow(self) || !self.stuck || self.previous != self.position
    }
    fn selector_target(&self) -> Option<Target> {
        use sunrise_client::projectiles::ProjectileKind as K;
        // The kinds /data can write (entity_nbt).
        let selectable = self.is_arrow()
            || sunrise_client::entity_nbt::saved_with_chunk(self)
            || matches!(
                self.kind,
                K::LargeFireball { .. }
                    | K::SmallFireball
                    | K::DragonFireball
                    | K::WindCharge
                    | K::BreezeWindCharge
            );
        selectable.then_some(Target::Projectile(self.id))
    }
    /// `AbstractArrow.push` does nothing in the ground; a wind charge's motion
    /// stays 0.
    fn pushed_by_explosion(&self) -> bool {
        !self.stuck && !self.kind.ignores_blast()
    }
    fn explosion_eye_height(&self) -> f64 {
        self.kind.eye_height()
    }
    fn push_from_explosion(&mut self, push: Vec3) {
        self.velocity = Vec3::new(
            self.velocity.x + push.x,
            self.velocity.y + push.y,
            self.velocity.z + push.z,
        );
        self.needs_sync = true;
    }
    fn bounds(&self) -> Aabb {
        Projectile::bounds(self)
    }
    /// A shulker bullet's, set when it is made, unless a mod wrote one: the
    /// write has no field on this struct and lands in `stored`, so a cleared
    /// flag is what the movement code reads; an arrow's `AbstractArrow.noPhysics`.
    fn no_physics(&self, stored: Option<bool>) -> bool {
        stored.unwrap_or_else(|| {
            matches!(self.kind, ProjectileKind::ShulkerBullet { .. })
                || self.arrow.as_ref().is_some_and(|a| a.no_physics)
        })
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        Projectile::move_by_piston(self, movement);
    }
    fn piston_release_bounds(&self) -> Option<Aabb> {
        Projectile::piston_release_bounds(self)
    }
    fn release_after_piston(&mut self, no_block_collision: bool) {
        Projectile::release_after_piston(self, no_block_collision);
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        Projectile::piston_slime_bounce(self, movement);
    }
}

impl Entity for Rocket {
    stored_ground!();
    fn reset_fall_distance(&mut self) {
        // A firework rocket keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:firework_rocket".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_rocket(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:firework_rocket") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_rocket,
            load_rocket_transferred,
            tag,
            uuid,
            player_uuid
        );
        Ok(())
    }
    stored_motion!();
    /// A rocket that carries its owner stays.
    fn portal_size(&self) -> Option<(f32, f32)> {
        (self.attached_to.is_none() && !self.attached).then_some((0.25, 0.25))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Rocket(self.id))
    }
    /// 0.25 wide, so 0.2125 to the eyes.
    fn explosion_eye_height(&self) -> f64 {
        0.2125
    }
    fn bounds(&self) -> Aabb {
        Rocket::bounds(self)
    }
    /// `FireworkRocketEntity.tick`: boosting a player skips the step.
    fn sweeps_blocks_inside(&self) -> bool {
        !self.attached
    }
}

impl Entity for ThrownTrident {
    stored_ground!();
    fn reset_fall_distance(&mut self) {
        // A thrown trident keeps no fall distance here.
    }
    fn portal_after_move(&self) -> bool {
        true
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
    }
    fn set_on_ground(&mut self, on_ground: bool) {
        self.ground.on_ground = on_ground;
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:trident".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_trident(index, keys)
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> Result<(), Tag> {
        if !(kind == "minecraft:trident") {
            return Err(tag);
        }
        load_with_transfer!(
            game,
            transferred,
            load_trident,
            load_trident_transferred,
            tag,
            uuid
        );
        Ok(())
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.5, 0.5))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn portal_sweeps(&self) -> bool {
        !self.in_ground || self.previous != self.position
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Trident(self.id))
    }
    /// Like any arrow (`AbstractArrow.push`) not pushed while in the ground.
    fn pushed_by_explosion(&self) -> bool {
        !self.in_ground
    }
    /// `EntityType.TRIDENT`: eyes at 0.13.
    fn explosion_eye_height(&self) -> f64 {
        f64::from(0.13_f32)
    }
    fn bounds(&self) -> Aabb {
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - 0.25, p.y, p.z - 0.25),
            Vec3::new(p.x + 0.25, p.y + 0.5, p.z + 0.25),
        )
    }
    /// `AbstractArrow.noPhysics`, set once a loyal trident turns back. Its own
    /// field is the write's store, so `stored` is None for a trident.
    fn no_physics(&self, _stored: Option<bool>) -> bool {
        self.no_physics
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        ThrownTrident::move_by_piston(self, movement);
    }
    fn piston_release_bounds(&self) -> Option<Aabb> {
        ThrownTrident::piston_release_bounds(self)
    }
    fn release_after_piston(&mut self, no_block_collision: bool) {
        ThrownTrident::release_after_piston(self, no_block_collision);
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        ThrownTrident::piston_slime_bounce(self, movement);
    }
    /// `GameLevel::tick_blocks` sweeps tridents itself.
    fn sweeps_blocks_inside(&self) -> bool {
        false
    }
}

impl EntityRules for Rocket {}
impl EntityRules for Boat {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `AbstractBoat.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}
impl EntityRules for Minecart {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `AbstractMinecart.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        if self.in_water { 0.005 } else { 0.04 }
    }
}
impl EntityRules for PrimedTnt {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `PrimedTnt.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}
impl EntityRules for FallingBlock {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `FallingBlockEntity.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}

#[cfg(test)]
mod no_gravity_stand_tests {
    use super::Entity as _;
    use sunrise_client::entities::ArmorStand;
    use sunrise_core::geometry::Vec3;

    #[test]
    fn a_no_gravity_stand_is_not_swept_for_the_blocks_it_is_in() {
        let mut stand = ArmorStand::new(1, Vec3::new(0.5, 100.0, 0.5), 0.0);
        // A resting stand presses a plate (measured beside the real server)...
        assert!(stand.sweeps_blocks_inside() && stand.triggers_blocks());
        // ...a NoGravity one does not, nor does a marker.
        stand.no_gravity = true;
        assert!(!stand.sweeps_blocks_inside());
        stand.no_gravity = false;
        stand.marker = true;
        assert!(!stand.triggers_blocks());
    }
}

#[cfg(test)]
mod load_claim_tests {
    /// `Mob` is first in `entity_lists!` and claims every id in `mob_types`;
    /// the old load tried every other kind first. Both agree only while no
    /// mob species shares an id with another kind, which this checks.
    #[test]
    fn no_mob_species_is_an_id_another_kind_loads() {
        let others = [
            "minecraft:item",
            "minecraft:experience_orb",
            "minecraft:arrow",
            "minecraft:spectral_arrow",
            "minecraft:llama_spit",
            "minecraft:shulker_bullet",
            "minecraft:fireball",
            "minecraft:small_fireball",
            "minecraft:wind_charge",
            "minecraft:breeze_wind_charge",
            "minecraft:falling_block",
            "minecraft:trident",
            "minecraft:firework_rocket",
            "minecraft:tnt",
            "minecraft:armor_stand",
            "minecraft:ominous_item_spawner",
            "minecraft:ender_dragon",
            "minecraft:end_crystal",
            "minecraft:item_frame",
            "minecraft:glow_item_frame",
            "minecraft:cushion",
            "minecraft:painting",
        ];
        for def in sunrise_client::mobs::species::ALL {
            let id = format!("minecraft:{}", def.name);
            assert!(!others.contains(&id.as_str()), "{id}");
            assert!(
                sunrise_client::entity_nbt::throwable_kind(&id).is_none(),
                "{id}"
            );
            assert!(super::super::boats::any_boat_wood(&id).is_none(), "{id}");
            assert!(
                super::super::minecarts::CartKind::from_id(&id).is_none(),
                "{id}"
            );
        }
    }
}

#[cfg(test)]
mod trident_selector_tests {
    use super::Entity as _;
    use crate::game::{commands::Target, trident::ThrownTrident};
    use sunrise_core::geometry::Vec3;

    /// `@e`, `/kill` and `/data` reach a thrown trident like any other entity
    /// (`EntityTypeTest.forClass(Entity.class)`: every entity in the level).
    #[test]
    fn a_thrown_trident_is_a_selector_target() {
        let t = ThrownTrident::remote(9, Vec3::new(0.5, 100.0, 0.5), 0.0, 0.0);
        assert_eq!(t.selector_target(), Some(Target::Trident(9)));
    }
}

#[cfg(test)]
#[path = "entity/restore_tests.rs"]
mod restore_tests;

#[cfg(test)]
mod join_order_tests {
    use super::{DrawFamily, EntityHandle, EntityLists, JoinOrderError};
    use crate::game::{
        boats::Boat, creatures::render_fixture::mob, fireworks::Rocket, trident::ThrownTrident,
    };
    use sunrise_client::entities::ArmorStand;
    use sunrise_client::join_seq::{JoinList, JoinSeq};
    use sunrise_client::mobs::{Mob, Species};
    use sunrise_core::geometry::Vec3;

    fn at() -> Vec3 {
        Vec3::new(0.5, 100.0, 0.5)
    }

    #[derive(Default)]
    struct Lists {
        mobs: JoinList<Mob>,
        stands: JoinList<ArmorStand>,
        tridents: JoinList<ThrownTrident>,
        rockets: JoinList<Rocket>,
        boats: JoinList<Boat, false>,
    }

    impl Lists {
        fn view(&self) -> EntityLists<'_> {
            EntityLists {
                mobs: &self.mobs,
                dropped: &[],
                boats: &self.boats,
                carts: &[],
                armor_stands: &self.stands,
                tnts: &[],
                falling: &[],
                orbs: &[],
                projectiles: &[],
                rockets: &self.rockets,
                tridents: &self.tridents,
                general: &[],
            }
        }
        fn walk(&self) -> Result<Vec<EntityHandle>, JoinOrderError> {
            let mut seen = Vec::new();
            self.view().walk_in_join_order(|h, _| seen.push(h))?;
            Ok(seen)
        }
    }

    /// Joins different kinds of entities interleaved, the way a level gets them.
    fn mixed() -> (Lists, Vec<EntityHandle>) {
        let mut l = Lists::default();
        l.trident(1);
        l.rocket(2);
        l.stand(3);
        l.boat(4);
        l.trident(5);
        l.stand(6);
        l.rocket(7);
        let joined = vec![
            EntityHandle::Trident(0),
            EntityHandle::Rocket(0),
            EntityHandle::ArmorStand(0),
            EntityHandle::Boat(0),
            EntityHandle::Trident(1),
            EntityHandle::ArmorStand(1),
            EntityHandle::Rocket(1),
        ];
        (l, joined)
    }

    impl Lists {
        fn stand(&mut self, id: u32) {
            self.stands.push(ArmorStand::new(id, at(), 0.0));
        }
        fn trident(&mut self, id: u32) {
            self.tridents
                .push(ThrownTrident::remote(id, at(), 0.0, 0.0));
        }
        fn rocket(&mut self, id: u32) {
            self.rockets.push(Rocket::remote(id, at(), Vec3::ZERO));
        }
        fn boat(&mut self, id: u32) {
            self.boats.push(Boat::new(id, "oak".into(), at(), 0.0));
        }
    }

    /// The walk visits entities in the order they joined, not list by list.
    #[test]
    fn mixed_kinds_are_visited_in_join_order() {
        let (lists, joined) = mixed();
        assert_eq!(lists.walk().unwrap(), joined);
        // Walked list by list they would come in another order.
        assert_ne!(joined[0], EntityHandle::ArmorStand(0));
    }

    fn runs(
        lists: &Lists,
        local_player: Option<JoinSeq>,
    ) -> Vec<(DrawFamily, std::ops::Range<usize>)> {
        let mut runs = Vec::new();
        super::draw_runs(
            lists.view(),
            super::DrawExtras {
                local_player,
                ..Default::default()
            },
            &mut runs,
        );
        runs.into_iter().map(|r| (r.family, r.range)).collect()
    }

    /// The ordered draw takes the families' entities in join order, not family by family, and
    /// each run names the list range its family draws.
    #[test]
    fn draw_runs_interleave_families_in_join_order() {
        let mut lists = Lists::default();
        let cow = |id| mob(id, "minecraft:cow", Species::Cow, 0.9, 1.4);
        lists.mobs.push(cow(4));
        lists.mobs.push(cow(3));
        let local_player = JoinSeq::next();
        lists.mobs.push(cow(2));
        lists.trident(6);
        lists.rocket(8);
        lists.stand(5);
        lists.mobs.push(cow(1));
        lists.boat(7);
        lists.trident(9);
        lists.stand(11);
        lists.rocket(10);
        assert_eq!(
            runs(&lists, Some(local_player)),
            [
                (DrawFamily::Mob, 0..2),
                (DrawFamily::LocalPlayer, 0..1),
                (DrawFamily::Mob, 2..3),
                (DrawFamily::Trident, 0..1),
                (DrawFamily::Rocket, 0..1),
                (DrawFamily::ArmorStand, 0..1),
                (DrawFamily::Mob, 3..4),
                (DrawFamily::Boat, 0..1),
                (DrawFamily::Trident, 1..2),
                (DrawFamily::ArmorStand, 1..2),
                (DrawFamily::Rocket, 1..2),
            ]
        );
    }

    /// Entities of one family that joined back to back are one run.
    #[test]
    fn draw_runs_join_a_family_that_joined_back_to_back() {
        let mut lists = Lists::default();
        lists.stand(1);
        lists.stand(2);
        lists.boat(3);
        lists.stand(4);
        assert_eq!(
            runs(&lists, None),
            [
                (DrawFamily::ArmorStand, 0..2),
                (DrawFamily::Boat, 0..1),
                (DrawFamily::ArmorStand, 2..3),
            ]
        );
    }

    /// Entities of an undrawn family do not split a run.
    #[test]
    fn an_undrawn_family_splits_no_run() {
        use sunrise_client::projectiles::{Projectile, ProjectileKind};
        let mut lists = Lists::default();
        lists.stand(1);
        let mut projectiles = JoinList::<Projectile, false>::default();
        projectiles.push(Projectile::new(ProjectileKind::DragonFireball, at(), None));
        lists.stand(2);
        let mut draw = Vec::new();
        super::draw_runs(
            EntityLists {
                projectiles: &projectiles,
                ..lists.view()
            },
            super::DrawExtras::default(),
            &mut draw,
        );
        assert_eq!(
            draw,
            [super::DrawRun {
                family: DrawFamily::ArmorStand,
                range: 0..2
            }]
        );
    }

    /// An entity taken out and put back (a tick takes its list) keeps its place.
    #[test]
    fn an_entity_put_back_keeps_its_join_number() {
        let (mut lists, joined) = mixed();
        let taken = lists.tridents.lift();
        let before: Vec<_> = taken
            .iter()
            .map(sunrise_client::join_seq::Joined::join_seq)
            .collect();
        let mut back = JoinList::default();
        for t in taken {
            back.push(t);
        }
        lists.tridents = back;
        let after: Vec<_> = lists
            .tridents
            .iter()
            .map(sunrise_client::join_seq::Joined::join_seq)
            .collect();
        assert_eq!(before, after);
        assert_eq!(lists.walk().unwrap(), joined);
    }

    /// The walk is checked: a list out of join order, or an entity that never
    /// went through the list's `push`, is reported and not walked past.
    #[test]
    fn the_walk_reports_a_list_out_of_order_and_an_entity_that_never_joined() {
        let (mut lists, _) = mixed();
        lists.tridents.swap(0, 1);
        assert!(matches!(lists.walk(), Err(JoinOrderError::OutOfOrder(_))));
        let (mut lists, _) = mixed();
        lists.stands.push(ArmorStand::new(9, at(), 0.0));
        lists.stands[2].join = sunrise_client::join_seq::JoinSeq::UNJOINED;
        assert_eq!(
            lists.walk(),
            Err(JoinOrderError::Unjoined(EntityHandle::ArmorStand(2)))
        );
    }
}

#[cfg(test)]
mod set_rotation_tests {
    use super::Entity as _;
    use sunrise_client::entities::ExperienceOrb;

    /// `Entity.teleportSetPosition`: yaw as given, pitch clamped to -90..90, on
    /// every kind (the ones with no field of their own used to drop it).
    #[test]
    fn set_rotation_keeps_yaw_and_clamps_pitch_on_kinds_that_had_none() {
        let mut orb = ExperienceOrb {
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            join: Default::default(),
            no_gravity: false,
            id: 0,
            position: sunrise_core::geometry::Vec3::ZERO,
            previous: sunrise_core::geometry::Vec3::ZERO,
            velocity: sunrise_core::geometry::Vec3::ZERO,
            value: 1,
            age: 0,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            rotation: [0.0, 0.0],
            following: None,
            fire_ticks: 0,
        };
        orb.set_rotation([90.0, 10.0]);
        assert_eq!(orb.rotation(), [90.0, 10.0]);
        orb.set_rotation([0.0, 200.0]);
        assert_eq!(orb.rotation(), [0.0, 90.0]);
    }
}

#[cfg(test)]
mod scoped_edit_tests {
    use super::*;

    #[test]
    fn scoped_walk_skips_only_one_entity_and_keeps_later_kinds() {
        let mut lists = TakenEntities {
            mobs: Default::default(),
            dropped: Default::default(),
            boats: Default::default(),
            carts: Default::default(),
            armor_stands: Default::default(),
            tnts: Default::default(),
            falling: Default::default(),
            orbs: Default::default(),
            projectiles: Default::default(),
            rockets: Default::default(),
            tridents: Default::default(),
            general: Default::default(),
        };
        lists
            .boats
            .push(Boat::new(1, "oak".into(), Vec3::ZERO, 0.0));
        lists
            .boats
            .push(Boat::new(2, "oak".into(), Vec3::ZERO, 0.0));
        lists.carts.push(Minecart::new(
            3,
            super::super::minecarts::CartKind::Rideable,
            Vec3::ZERO,
        ));
        let mut visited = Vec::new();
        for_each_entity_mut!(lists, entity => {
            if entity.id() == 1 {
                return;
            }
            visited.push(entity.id());
            entity.velocity_mut().x = 4.0;
        });
        assert_eq!(visited, [2, 3]);
        assert_eq!(lists.boats[0].velocity.x, 0.0);
        assert_eq!(lists.boats[1].velocity.x, 4.0);
        assert_eq!(lists.carts[0].velocity.x, 4.0);
        let previous = entity_access!(exclusive, lists.boats, 1, boat, {
            let previous = boat.velocity.x;
            boat.velocity.x = 7.0;
            previous
        });
        assert_eq!(previous, 4.0);
        assert_eq!(lists.boats[1].velocity.x, 7.0);
    }
}

#[cfg(test)]
#[path = "entity/taken_pass_tests.rs"]
pub(in crate::game) mod taken_pass_tests;
