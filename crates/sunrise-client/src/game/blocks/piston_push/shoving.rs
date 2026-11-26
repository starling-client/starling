//! The directional push of `PistonMovingBlockEntity.moveCollidedEntities` for mod colliders:
//! the moving or turning boxes meet a body along their simultaneous sweep, then shove it clear.
use super::*;
use sunrise_client_render::moving_colliders::{Placed, Shove};
use sunrise_core::geometry::oriented::OrientedCollider;

#[cfg(test)]
#[path = "shoving_tests.rs"]
mod tests;

enum SweepKind<'a> {
    Translation {
        before: &'a OrientedCollider,
        delta: Vec3,
    },
    General {
        before: &'a OrientedCollider,
    },
}

impl Game<'_> {
    pub(super) fn shove_pass(&mut self, start: &Prepared, steps: &StepHeights, riders: &[Rider]) {
        let end = self.placed_moving_colliders();
        let starts: sunrise_client::fast_hash::FxHashMap<u32, &Placed> =
            start.placed().map(|p| (p.id, p)).collect();
        let mut sweeps: Vec<_> = end
            .placed()
            .filter_map(|after| {
                let before = &starts.get(&after.id)?.collider;
                if before.frame == after.collider.frame {
                    return None;
                }
                if before.frame.same_turn(&after.collider.frame) {
                    let delta = after.collider.frame.origin.subtract(before.frame.origin);
                    if delta == Vec3::ZERO {
                        return None;
                    }
                    let swept = before.world_bounds()?.expand_towards(delta);
                    Some((after, SweepKind::Translation { before, delta }, swept))
                } else {
                    let bounds = before.boxes.bounds()?;
                    let swept = before.frame.swept_bounds(&after.collider.frame, bounds);
                    Some((after, SweepKind::General { before }, swept))
                }
            })
            .collect();
        if sweeps.is_empty() {
            return;
        }
        sweeps.sort_unstable_by_key(|(after, ..)| after.id);

        let server = self.is_logical_server();
        let targets: Vec<_> = self
            .push_targets()
            .filter(|target| match target {
                MoveWho::Entity(_) => server,
                MoveWho::Player => !server,
                MoveWho::Guest(_) => false,
            })
            .filter_map(|target| self.stable_push_target(target))
            .collect();
        let mut current = end.clone();
        let mut stand_grid = None;
        let mut shoved = false;
        for (after, kind, swept) in sweeps {
            let source = after.id;
            let first = riders.partition_point(|rider| rider.support < source);
            let supported = &riders[first..];
            for &selected in &targets {
                if after.exempt.contains(&selected.id)
                    || supported
                        .iter()
                        .take_while(|r| r.support == source)
                        .any(|r| r.id == selected.id)
                {
                    continue;
                }
                let Some(target) = self.resolve_push_target(selected) else {
                    continue;
                };
                let Some(body) = self.push_body(target, None, steps) else {
                    continue;
                };
                if !swept.intersects(body.bounds) {
                    continue;
                }
                let (_, no_physics, rides) = self.carry_facts(target);
                if no_physics || rides {
                    continue;
                }
                let movement = match kind {
                    SweepKind::Translation { before, delta } => {
                        let fraction =
                            before.translation_push_fraction(&after.collider, body.bounds, delta);
                        if fraction > 0.0 {
                            let direction = delta.scale(1.0 / delta.length());
                            // The piston's clearance is added once to its largest box advance.
                            Some(
                                delta
                                    .scale(fraction)
                                    .add_vector(direction.scale(piston_push::PUSH_OFFSET)),
                            )
                        } else {
                            None
                        }
                    }
                    SweepKind::General { before } => before.sweep_push_displacement(
                        &after.collider,
                        body.bounds,
                        piston_push::PUSH_OFFSET,
                    ),
                };
                if let Some(movement) = movement {
                    {
                        let _scope = moving_colliders::Scope::enter_level(
                            self.collider_level(),
                            current.clone(),
                        );
                        let _shove = Shove::enter(selected.id, source);
                        self.move_pushed_body(
                            MoverType::Carried,
                            target,
                            body,
                            movement,
                            false,
                            &mut stand_grid,
                        );
                    }
                    shoved = true;
                    if let Some(target) = self.resolve_push_target(selected)
                        && !matches!(target, MoveWho::Guest(_))
                    {
                        let moved = match target {
                            MoveWho::Player => self
                                .player
                                .as_ref()
                                .map_or(Vec3::ZERO, |player| player.position),
                            MoveWho::Entity(handle) => {
                                with_entity!(self, handle, entity => entity.position())
                            }
                            MoveWho::Guest(_) => unreachable!(),
                        }
                        .subtract(body.position);
                        if moved != Vec3::ZERO {
                            let (SweepKind::Translation { before, .. }
                            | SweepKind::General { before }) = kind;
                            let rise = after.collider.frame.origin.y - before.frame.origin.y;
                            let normal = movement.normalize();
                            match target {
                                MoveWho::Player => {
                                    if let Some(player) = self.player.as_mut() {
                                        player.velocity = cancel_shove_velocity(
                                            player.velocity,
                                            normal,
                                            movement.y,
                                            rise,
                                        );
                                    }
                                }
                                MoveWho::Entity(handle) => {
                                    with_entity_mut!(self, handle, entity => {
                                        let velocity = cancel_shove_velocity(
                                            *entity.velocity_mut(),
                                            normal,
                                            movement.y,
                                            rise,
                                        );
                                        *entity.velocity_mut() = velocity;
                                    });
                                }
                                MoveWho::Guest(_) => {}
                            }
                        }
                    }
                    if current.placed().any(|p| p.exempt.contains(&selected.id)) {
                        // This body owns a collider, or is the vehicle of a holder. Its
                        // new pose is an obstacle to later shoves, not another sweep.
                        self.position_riders();
                        current = self.placed_moving_colliders();
                    }
                }
            }
        }
        if shoved {
            self.position_riders();
        }
    }
}

/// Removes the part of `velocity` that points into a shove along `normal`. An upward shove
/// raises vertical velocity to the collider's own vertical speed `collider_rise` but never
/// lowers it, so a body already rising faster keeps its speed.
fn cancel_shove_velocity(velocity: Vec3, normal: Vec3, upward: f64, collider_rise: f64) -> Vec3 {
    let inward = velocity.dot(normal);
    if inward >= 0.0 {
        return velocity;
    }
    let mut velocity = velocity.subtract(normal.scale(inward));
    if upward > 0.0 {
        velocity.y = velocity.y.max(collider_rise);
    }
    velocity
}
