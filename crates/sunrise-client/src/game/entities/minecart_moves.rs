//! The cart's `Entity.move` — `AbstractMinecart.move`'s own tick move through
//! the move gate (#2414) — in its own module so `minecarts.rs` keeps its size.
use super::Game;
use super::minecarts::{CartKind, Minecart};
use crate::game::collision_source::Mover;
use sunrise_client::{collisions::SceneCollisions, motion::BodyContact};
use sunrise_core::geometry::{Aabb, Vec3};

pub(super) struct CartLanding {
    id: u32,
    kind: CartKind,
    at: Vec3,
    bounds: Aabb,
    distance: f64,
}
impl Game<'_> {
    /// `Entity.move` resets a ray-clipped fall, then `checkFallDamage` uses the
    /// current position, supporting block and the body's last water contact.
    pub(super) fn cart_move_fall(
        &self,
        c: &Minecart,
        from: Vec3,
        moved: Vec3,
    ) -> (f64, Option<CartLanding>) {
        let mut distance = self.vehicle_fall_reset(c.fall_distance, from, moved);
        if moved.y < 0.0 && !c.in_water {
            distance -= f64::from(moved.y as f32);
        }
        if !c.ground.on_ground {
            return (distance, None);
        }
        (
            0.0,
            (distance > 0.0).then(|| CartLanding {
                id: c.id,
                kind: c.kind,
                at: c.position,
                bounds: c.bounds(),
                distance,
            }),
        )
    }

    /// Run `Block.fallOn`, including `MinecartTNT.causeFallDamage`, before the
    /// landing game event. A removed cart cannot resolve block restitution.
    pub(super) fn finish_cart_landing(
        &mut self,
        landing: Option<CartLanding>,
        effect: BodyContact,
        explode: impl FnOnce(&mut Self, f64) -> bool,
    ) -> bool {
        let Some(CartLanding {
            id,
            kind,
            at,
            bounds,
            distance,
        }) = landing
        else {
            return true;
        };
        self.nonliving_fall_on(id, at, effect);
        let removed = if kind == CartKind::Tnt && distance >= 3.0 {
            let speed = distance / 10.0;
            explode(self, speed * speed)
        } else {
            false
        };
        self.post_landing_event(at, bounds, false);
        !removed
    }

    /// Entity.move for a cart: collisions zero the blocked components, the world border's walls
    /// among them (`CollisionGetter.borderCollision`, asked for every entity).
    pub(super) fn move_cart(&mut self, c: &mut Minecart, delta: Vec3) -> bool {
        // The cart's own move is a self move (#2414: vanilla's
        // `AbstractMinecart.move` passes SELF for its own tick); PR 1a
        // records the movement unchanged.
        let mut gate = sunrise_client::move_gate::armed(
            &self.loop_context,
            c.id,
            c.kind.id(),
            sunrise_world::entity_move::MoverType::SelfMovement,
            delta,
        );
        let mut delta = gate.delta();
        // Entity.move: a stuck multiplier scales this move and zeroes the velocity.
        if c.stuck.length_squared() > 1.0e-7 {
            delta = delta.multiply(c.stuck);
            c.stuck = Vec3::ZERO;
            c.velocity = Vec3::ZERO;
        }
        let scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        // A zero move stays zero whatever surrounds it (`Entity.collide`, and the
        // step-up needs a blocked horizontal move): a cart standing on its rails,
        // as most do, gathers no entity boxes.
        let entities = if delta.length_squared() == 0.0 {
            Vec::new()
        } else {
            self.entity_collisions(Mover::Cart(c))
        };
        let mut scene = sunrise_client::collisions::EntityCollisions {
            blocks: scene,
            entities: &entities,
            border: Some((self.border.geometry(), [c.position.x, c.position.z])),
        };
        if let Ok(moved) = sunrise_client::move_gate::entities_resolve(
            &mut gate,
            &mut scene,
            c.bounds(),
            delta,
            0.0,
            c.ground.on_ground,
        ) {
            let from = c.position;
            c.position = Vec3::new(
                c.position.x + moved.x,
                c.position.y + moved.y,
                c.position.z + moved.z,
            );
            let side = game_side!(self);
            let motion = sunrise_client::motion::BodyMotion {
                world: &self.world,
                collision: self.collision,
                facts: self.res.facts(),
                blocks: Some(&self.motion.blocks),
                context: &self.loop_context,
                side: Some(&side),
            };
            if !gate.skipped() {
                motion.record_move(
                    sunrise_client::motion::BodyCollision {
                        footprint: (c.position, c.bounds()),
                        movement: (delta, moved),
                        authoritative: true,
                        context: sunrise_client::motion::BodyMotion::entity_context(
                            c.id, c.position,
                        ),
                        subject: None,
                    },
                    &mut c.ground,
                );
            }
            let effect = motion.legacy_contact(c.position, &c.ground);
            let (fall_distance, landing) = self.cart_move_fall(c, from, moved);
            c.fall_distance = fall_distance;
            if landing.is_some() {
                let id = c.id;
                let join = c.join;
                if !self
                    .finish_cart_landing(landing, effect, |game, power| game.explode_cart(c, power))
                    || !self
                        .carts
                        .iter()
                        .any(|row| row.id == id && row.join == join)
                    || self
                        .persist
                        .removals
                        .contains(&super::commands::Target::Cart(id))
                {
                    return false;
                }
            }
            let entity = (c.id, c.kind.id());
            let response = self.body_motion(&game_side!(self)).restitute(
                entity,
                sunrise_client::motion::BodyMove {
                    wanted: delta,
                    moved,
                    effect,
                },
                sunrise_client::entity_rules::EntityRules::get_gravity(c),
                0.95_f32,
                c.velocity,
                c.fall_distance,
            );
            c.velocity = self.finish_body_restitution(entity, c.position, response);
            self.body_motion(&game_side!(self))
                .apply_block_speed(c.position, &mut c.velocity);
        }
        // OldMinecartBehavior applies block effects here and at tick end.
        // NewMinecartBehavior applies them after recording each track step.
        if !self.minecart_improvements() {
            self.cart_block_effects(c);
        }
        true
    }
}
