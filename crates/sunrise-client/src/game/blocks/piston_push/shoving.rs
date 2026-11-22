//! The directional push of `PistonMovingBlockEntity.moveCollidedEntities` for mod colliders:
//! the moving boxes meet a body along their sweep, then the deepest advance sets its push.
use super::*;
use sunrise_client_render::moving_colliders::{Placed, Shove};

#[cfg(test)]
#[path = "shoving_tests.rs"]
mod tests;

impl Game<'_> {
    pub(super) fn shove_pass(&mut self, start: &Prepared, steps: &StepHeights, riders: &[Rider]) {
        let end = self.placed_moving_colliders();
        let starts: sunrise_client::fast_hash::FxHashMap<u32, &Placed> =
            start.placed().map(|p| (p.id, p)).collect();
        let mut sweeps: Vec<_> = end
            .placed()
            .filter_map(|after| {
                let before = &starts.get(&after.id)?.collider;
                let delta = after.collider.frame.origin.subtract(before.frame.origin);
                // Turning colliders still carry riders. Their angular sweep is a separate
                // geometry problem; testing just the endpoints would miss a turn's path.
                if delta == Vec3::ZERO || !before.frame.same_turn(&after.collider.frame) {
                    return None;
                }
                let swept = before.world_bounds()?.expand_towards(delta);
                Some((after, before, delta, swept))
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
        for (after, before, delta, swept) in sweeps {
            let source = after.id;
            let first = riders.partition_point(|rider| rider.support < source);
            let supported = &riders[first..];
            let direction = delta.scale(1.0 / delta.length());
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
                let fraction =
                    before.translation_push_fraction(&after.collider, body.bounds, delta);
                if fraction > 0.0 {
                    // The piston's clearance is added once to its largest box advance.
                    let movement = delta
                        .scale(fraction)
                        .add_vector(direction.scale(piston_push::PUSH_OFFSET));
                    {
                        let _scope = moving_colliders::Scope::enter_level(
                            self.collider_level(),
                            current.clone(),
                        );
                        let _shove = Shove::enter(selected.id, source);
                        self.move_pushed_body(
                            MoverType::Carried,
                            None,
                            target,
                            body,
                            movement,
                            false,
                            &mut stand_grid,
                        );
                    }
                    shoved = true;
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
