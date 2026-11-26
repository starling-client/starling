//! Carrying: a moving collider (an entity's `set_collider`) moves the bodies that stand on it.
//!
//! Once a tick, after a level's entities ticked and so whatever order they ticked in, every
//! body standing on a collider box moves by the displacement its contact point had under that
//! collider's change this tick (translation and rotation), as a collided `Entity.move` with
//! `MoveCause::Carried`: the move a lid's push makes, through the same gate, so it stops at
//! walls, steps, and is seen by an `entity_move` listener. Yaw is never turned.
//!
//! A body's support is the one collider box under it: the highest top, a tie to the lower
//! collider entity id. The supports form a forest, and it is carried from the roots: a collider
//! that stands on another collider is carried first, and the bodies on it use its whole
//! displacement. Then a moving or turning collider shoves nearby bodies in the simultaneous
//! swept path of its boxes. A body that rides something follows its vehicle. The server moves
//! what it simulates, each client its own player.
#[path = "shoving.rs"]
mod shoving;
use super::*;
use sunrise_client_render::moving_colliders::{self, Prepared, Shove};
use sunrise_core::geometry::oriented::OrientedCollider;
use sunrise_world::{
    entity_obstruction::EntityIdentity,
    world_movement::{CollisionCheckRequest, no_world_collision},
};

/// How far under its feet a body may have a box and still stand on it.
const SUPPORT_PROBE: f64 = 0.01;

/// A body standing on a collider.
struct Rider {
    target: MoveWho,
    id: u32,
    /// The collider entity it stands on.
    support: u32,
}

/// What a carry pass works on, kept from tick to tick so a pass allocates nothing once these
/// have grown to the level's size.
#[derive(Default)]
struct Scratch {
    /// The bodies standing on a collider, ordered by the collider.
    riders: Vec<Rider>,
    /// The riders that are colliders' own entities: (entity, the collider it stands on), by entity.
    stacked: Vec<(u32, u32)>,
    /// The colliders with riders, as (depth in the support forest, collider entity).
    order: Vec<(u32, u32)>,
    /// The collider entities of the level, sorted.
    ids: Vec<u32>,
    /// Candidate face heights for clearing a tilted support under an upright body.
    heights: Vec<f64>,
}

thread_local! {
    static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::new(Scratch::default());
}

/// The collider a body with `bounds` and entity `id` stands on: the one box under its feet with
/// the highest top (a tie to the lower collider entity id), found through the level's index.
fn support_of(start: &Prepared, bounds: Aabb, id: u32) -> Option<u32> {
    let mut best: Option<(f64, u32)> = None;
    let probe = Aabb::new(
        bounds
            .min_position()
            .subtract(Vec3::new(0.0, SUPPORT_PROBE, 0.0)),
        bounds.max_position(),
    );
    start.meeting(probe, |placed| {
        if placed.exempt.contains(&id) {
            return;
        }
        let allowed = placed.collider.collide(bounds, Axis::Y, -SUPPORT_PROBE);
        if allowed <= -SUPPORT_PROBE {
            return;
        }
        if placed.collider.intersects(bounds) {
            return;
        }
        let gap = -allowed;
        let better =
            best.is_none_or(|(g, c)| gap < g - 1.0e-9 || (gap <= g + 1.0e-9 && placed.id < c));
        if better {
            best = Some((gap, placed.id));
        }
    });
    best.map(|(_, support)| support)
}

/// A rigid contact point turns with its support, but the body stays upright. When the support
/// tilts, the body's uphill bottom edge can dip below the tilted face even when its step height
/// is 0. Find the smallest upward lift onto a face of `support` that clears the embedded
/// footprint while keeping the body resting on `support`.
fn upright_tilt_clearance(
    support: &OrientedCollider,
    bounds: Aabb,
    rigid: Vec3,
    heights: &mut Vec<f64>,
) -> Vec3 {
    let proposed = bounds.move_by(rigid);
    if !support.intersects(proposed) {
        return rigid;
    }
    let mut max_top = proposed.min(Axis::Y);
    support.overlapping(proposed, |index| {
        let local = support.boxes.boxes()[index as usize];
        max_top = max_top.max(support.frame.world_bounds(local).max(Axis::Y));
        false
    });
    let max_lift = (max_top - proposed.min(Axis::Y)).max(0.0) + COLLISION_EPSILON;
    if max_lift <= COLLISION_EPSILON {
        return rigid;
    }
    heights.clear();
    support.step_heights(proposed, Vec3::ZERO, max_lift, heights);
    heights.sort_unstable_by(f64::total_cmp);
    heights.dedup();
    for &height in heights.iter() {
        let lifted = proposed.move_by(Vec3::new(0.0, height, 0.0));
        if !support.intersects(lifted)
            && support.collide(lifted, Axis::Y, -SUPPORT_PROBE) > -SUPPORT_PROBE
        {
            return rigid.add_vector(Vec3::new(0.0, height, 0.0));
        }
    }
    rigid
}

impl Game<'_> {
    /// Notes, on a client, the collider carrying the player and how old the client's copy of it
    /// is, for the hint sent with the next position report.
    pub(in crate::game) fn observe_carry_age(&mut self, start: &Prepared) {
        let carrier = (!start.is_empty())
            .then(|| {
                let p = self.player.as_ref()?;
                if p.spectator || !p.ground.on_ground || self.riding.is_some() {
                    return None;
                }
                support_of(start, p.bounds(), p.entity_id as u32)
            })
            .flatten()
            .and_then(|id| Some((id, self.collider_now_frame(id)?)));
        if let Some(remote) = self.remote.as_mut() {
            remote.carry_age.observe(carrier);
        }
    }

    /// Carries bodies on colliders that moved this tick, then shoves bodies in the path of a
    /// moving or turning collider. `start` is the colliders as the tick began. A tick where none
    /// moved does nothing.
    pub(in crate::game) fn carry_pass(&mut self, start: &Prepared) {
        // The client handles collider and entity movement packets before its tick
        // (`Minecraft.runTick`). Carry and shove from the last tick's end placement for this
        // level so changes that arrived before the tick (`Rotation`, `Boxes`, `Patch`, a
        // teleport) and changes during it combine into one displacement; the server has no
        // `remote` state and carries from `start` unchanged.
        let last = self
            .remote
            .as_mut()
            .and_then(|r| r.collider_pass_start.take())
            .filter(|(level, _)| *level == self.dim);
        let before = last.as_ref().map_or(start, |(_, placed)| placed);
        if !before.is_empty() && self.any_collider_moved(before) {
            let mut scratch = SCRATCH.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
            let steps = self.push_step_heights();
            scratch.riders.clear();
            self.collider_riders(before, &steps, &mut scratch);
            if !scratch.riders.is_empty() {
                self.carry_in_order(before, &steps, &mut scratch);
                // A passenger that owns a collider moves with its vehicle too.
                self.position_riders();
            }
            self.shove_pass(before, &steps, &scratch.riders);
            SCRATCH.with(|cell| *cell.borrow_mut() = scratch);
        }
        if self
            .remote
            .as_ref()
            .is_some_and(|r| !r.colliders.is_empty())
        {
            let now = self.placed_moving_colliders();
            if let Some(remote) = self.remote.as_mut() {
                remote.collider_pass_start = Some((self.dim, now));
            }
        }
    }

    /// Whether any collider of `start` stands elsewhere now.
    fn any_collider_moved(&self, start: &Prepared) -> bool {
        start.placed().any(|p| {
            self.collider_now_frame(p.id)
                .is_some_and(|now| now != p.collider.frame)
        })
    }

    /// Carries the riders of each collider, a collider after the collider its own entity stands on.
    fn carry_in_order(&mut self, start: &Prepared, steps: &StepHeights, scratch: &mut Scratch) {
        scratch.order.clear();
        let mut at = 0;
        while at < scratch.riders.len() {
            let collider = scratch.riders[at].support;
            // A cycle of colliders on one another stops at the number of colliders there are:
            // they are then carried in the order they were placed.
            let mut depth = 0;
            let mut up = collider;
            while let Ok(i) = scratch.stacked.binary_search_by_key(&up, |&(id, _)| id) {
                up = scratch.stacked[i].1;
                depth += 1;
                if depth > start.len() as u32 || up == collider {
                    break;
                }
            }
            scratch.order.push((depth, collider));
            while at < scratch.riders.len() && scratch.riders[at].support == collider {
                at += 1;
            }
        }
        scratch.order.sort_unstable();
        let mut stand_grid = None;
        let mut wave = 0;
        let mut at = 0;
        while at < scratch.order.len() {
            // The carried moves meet the colliders where they stand now.
            wave = wave.max(scratch.order[at].0);
            let now = self.placed_moving_colliders();
            {
                let _scope =
                    moving_colliders::Scope::enter_level(self.collider_level(), now.clone());
                while at < scratch.order.len() && scratch.order[at].0 <= wave {
                    let collider = scratch.order[at].1;
                    let first = scratch.riders.partition_point(|r| r.support < collider);
                    let count = scratch.riders[first..]
                        .iter()
                        .take_while(|r| r.support == collider)
                        .count();
                    self.carry_riders_of(
                        collider,
                        start,
                        &now,
                        steps,
                        first..first + count,
                        scratch,
                        &mut stand_grid,
                    );
                    at += 1;
                }
            }
            wave += 1;
        }
    }

    /// The bodies standing on a collider of `start`, each with the collider it stands on,
    /// into `scratch`, by collider.
    fn collider_riders(&mut self, start: &Prepared, steps: &StepHeights, scratch: &mut Scratch) {
        let server = self.is_logical_server();
        scratch.riders.clear();
        scratch.stacked.clear();
        scratch.ids.clear();
        scratch.ids.extend(start.placed().map(|p| p.id));
        scratch.ids.sort_unstable();
        for target in self.push_targets() {
            let wanted = if server {
                matches!(target, MoveWho::Entity(_))
            } else {
                matches!(target, MoveWho::Player)
            };
            if !wanted {
                continue;
            }
            let Some(body) = self.push_body(target, None, steps) else {
                continue;
            };
            if !body.on_ground {
                continue;
            }
            let (id, no_physics, rides) = self.carry_facts(target);
            if no_physics || rides {
                continue;
            }
            let best = support_of(start, body.bounds, id);
            if let Some(support) = best {
                scratch.riders.push(Rider {
                    target,
                    id,
                    support,
                });
            }
        }
        scratch.riders.sort_unstable_by_key(|r| r.support);
        for rider in &scratch.riders {
            if scratch.ids.binary_search(&rider.id).is_ok() {
                scratch.stacked.push((rider.id, rider.support));
            }
        }
        scratch.stacked.sort_unstable();
    }

    /// The entity id of a carried body, whether it has no physics, and whether it rides.
    fn carry_facts(&self, target: MoveWho) -> (u32, bool, bool) {
        match target {
            MoveWho::Player => (
                self.player.as_ref().map_or(0, |p| p.entity_id as u32),
                false,
                self.riding.is_some(),
            ),
            MoveWho::Guest(_) => (0, true, false),
            MoveWho::Entity(handle) => {
                let (id, no_physics) = with_entity!(self, handle, entity => {
                    let stored = self.persist.no_physics.get(&entity.id()).copied();
                    (entity.id(), entity.no_physics(stored))
                });
                (id, no_physics, self.push_root(target) != Some(id))
            }
        }
    }

    /// Moves each body standing on `collider` by the displacement of its contact point.
    #[allow(clippy::too_many_arguments)]
    fn carry_riders_of(
        &mut self,
        collider: u32,
        start: &Prepared,
        now: &Prepared,
        steps: &StepHeights,
        riders: std::ops::Range<usize>,
        scratch: &mut Scratch,
        stand_grid: &mut Option<sunrise_client::entities::ObstacleGrid>,
    ) {
        let Some(before) = start.placed().find(|p| p.id == collider) else {
            return;
        };
        let Some(after) = now.placed().find(|p| p.id == collider) else {
            return;
        };
        let turned = !before.collider.frame.same_turn(&after.collider.frame);
        for rider in &scratch.riders[riders] {
            let Some(body) = self.push_body(rider.target, None, steps) else {
                continue;
            };
            let centre = body.bounds.center();
            let contact = Vec3::new(centre.x, body.bounds.min(Axis::Y), centre.z);
            let rigid = after
                .collider
                .frame
                .to_world(before.collider.frame.to_local(contact))
                .subtract(contact);
            let carried = if turned {
                upright_tilt_clearance(&after.collider, body.bounds, rigid, &mut scratch.heights)
            } else {
                rigid
            };
            if carried.length_squared() >= 1.0e-18 {
                let _shove = Shove::enter(rider.id, collider);
                self.move_pushed_body(
                    MoverType::Carried,
                    rider.target,
                    body,
                    carried,
                    false,
                    stand_grid,
                );
            }
            let grounded = self
                .push_body(rider.target, None, steps)
                .is_some_and(|after_body| {
                    self.carried_body_grounded(rider.target, rider.id, after_body)
                });
            match rider.target {
                MoveWho::Player => {
                    if let Some(player) = self.player.as_mut() {
                        player.ground.on_ground = grounded;
                    }
                }
                MoveWho::Entity(handle) => {
                    with_entity_mut!(self, handle, entity => entity.set_piston_on_ground(grounded));
                }
                MoveWho::Guest(conn) => {
                    if let Some(host) = self.lan.as_mut() {
                        host.hub.update_guest(conn, |g| {
                            g.on_ground = grounded;
                            if let Some(m) = g.motion.as_deref_mut() {
                                m.ground.on_ground = grounded;
                            }
                        });
                    }
                }
            }
        }
    }

    /// Collision inspection of the post-carry body and a downward support probe. A body that is
    /// already intersecting a block/entity/collider or has nothing under it is not grounded.
    fn carried_body_grounded(&mut self, target: MoveWho, id: u32, body: Body) -> bool {
        let entities = if target == MoveWho::Player {
            self.entity_collisions(Mover::Player)
        } else {
            self.pushed_entity_colliders(target)
        };
        let mut collisions = EntityCollisions::with_border(
            SceneCollisions {
                world: &self.world,
                binding: self.collision,
                subject: None,
            },
            self.border.geometry(),
            [body.position.x, body.position.z],
        );
        collisions.entities = &entities;
        let mut clear = |bounds| {
            no_world_collision(
                &mut collisions,
                CollisionCheckRequest {
                    entity: EntityIdentity(u64::from(id)),
                    context: CollisionContext::position(body.bounds.min(Axis::Y)),
                    bounds,
                    candidate_budget: 1_000_000,
                },
            )
        };
        clear(body.bounds).is_ok_and(|clear| clear)
            && clear(body.bounds.move_by(Vec3::new(0.0, -SUPPORT_PROBE, 0.0)))
                .is_ok_and(|clear| !clear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::geometry::oriented::{ColliderBoxes, Frame};

    /// A tilting deck clears an upright body's uphill edge regardless of step height (including
    /// step-height-0 bodies such as items).
    #[test]
    fn upright_tilt_clearance_never_embeds_a_zero_step_body() {
        let boxes = Arc::new(ColliderBoxes::new(vec![Aabb::new(
            Vec3::new(-4.0, -1.0, -4.0),
            Vec3::new(4.0, 0.0, 4.0),
        )]));
        let origin = Vec3::new(0.0, 10.0, 0.0);
        let before = Frame::translation(origin);
        let mut heights = Vec::new();
        for (half_w, height) in [(0.125, 0.25), (0.3, 1.8)] {
            for contact in [Vec3::new(2.0, 10.0, 0.0), origin] {
                let body = Aabb::new(
                    contact.add_vector(Vec3::new(-half_w, 0.0, -half_w)),
                    contact.add_vector(Vec3::new(half_w, height, half_w)),
                );
                for axis in [Axis::X, Axis::Z] {
                    for angle in [-0.25_f64, 0.25] {
                        let mut rotation = [0.0, 0.0, 0.0, (angle * 0.5).cos()];
                        rotation[if axis == Axis::X { 0 } else { 2 }] = (angle * 0.5).sin();
                        let support = OrientedCollider::new(
                            Frame::from_quaternion(origin, rotation),
                            boxes.clone(),
                        );
                        let rigid = support
                            .frame
                            .to_world(before.to_local(contact))
                            .subtract(contact);
                        assert!(support.intersects(body.move_by(rigid)));
                        let carried = upright_tilt_clearance(&support, body, rigid, &mut heights);
                        let corrected = body.move_by(carried);
                        assert!(!support.intersects(corrected));
                        assert_eq!(support.collide(corrected, Axis::Y, -SUPPORT_PROBE), 0.0);
                        let expected_extra = half_w * angle.abs().tan();
                        assert!((carried.y - rigid.y - expected_extra).abs() < 1.0e-12);
                    }
                }
            }
        }
    }
}
