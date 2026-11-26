//! The one stored pick, refreshed before tick inputs and frame extraction.
use super::*;
use crate::game::{Dim, view_vector};
use mod_api::CrosshairTarget;

#[cfg(test)]
#[path = "client_pick/tests.rs"]
mod tests;

#[derive(Default)]
pub(in crate::game) struct ClientPick {
    pub ray: Option<([f64; 3], [f64; 3])>,
    epoch: Option<mod_api::WorldEpoch>,
    dimension: Option<Dim>,
    origin: [f64; 3],
    entity_reach: f64,
    result: Option<CrosshairTarget>,
    pub entity_metadata: (bool, bool),
    pub rider_candidates: Vec<crate::game::pick_accumulator::PickCandidate>,
    pub sections: std::cell::RefCell<crate::game::pick_entity::pick_sections::PickSectionStorage>,
    dispatching: bool,
    /// How many picks have been taken.
    taken: u64,
}

impl Game<'_> {
    /// How many picks (a tick's or a frame's) have been taken: an agent command that turned
    /// waits for the count to move before it reads the target.
    pub(in crate::game) fn client_picks(&self) -> u64 {
        self.client_pick.taken
    }

    pub(in crate::game) fn uses_stored_pick(&self) -> bool {
        self.native_window() && self.client_pick.ray.is_none()
    }

    pub(in crate::game) fn pick_distance(&self, point: [f64; 3]) -> f64 {
        (0..3)
            .map(|i| (point[i] - self.client_pick.origin[i]).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    /// A window always has a decision, including before the first pick (a miss).
    /// None in the outer option means a server ray or the current pick computation.
    pub(in crate::game) fn cached_entity_pick(&self) -> Option<Option<(EntityRef, f64, [f64; 3])>> {
        self.uses_stored_pick()
            .then(|| match self.crosshair_target() {
                Some(CrosshairTarget::Entity { entity, point }) => {
                    Some((entity, self.pick_distance(point), point))
                }
                _ => None,
            })
    }

    /// Consumers read the stored hit as-is until the next pick. In particular,
    /// a block broken earlier in a tick is still that tick's use target.
    pub(in crate::game) fn stored_client_pick(&self) -> Option<CrosshairTarget> {
        if self.client_pick.dispatching {
            return self.client_pick.result;
        }
        (self.client_pick.epoch == self.native_lifecycle.epoch()
            && self.client_pick.dimension == Some(self.dim))
        .then_some(())?;
        self.client_pick.result
    }

    fn pick_target_live(&self, result: CrosshairTarget) -> bool {
        use crate::game::api_values::TryIntoApi;
        match result {
            CrosshairTarget::Block { pos, state, .. } => {
                self.world.block(pos.x, pos.y, pos.z).try_into_api().ok() == Some(state)
            }
            CrosshairTarget::Entity { entity, .. } => {
                self.native_client_entity_target(entity).is_some()
            }
            _ => false,
        }
    }

    /// The sole mod-output geometry boundary. Occlusion is intentionally not reapplied.
    fn accept_client_pick(&self, result: CrosshairTarget) -> bool {
        let (point, reach) = match result {
            CrosshairTarget::Block { hit, .. } => {
                if !matches!(
                    hit.face,
                    [1, 0, 0] | [-1, 0, 0] | [0, 1, 0] | [0, -1, 0] | [0, 0, 1] | [0, 0, -1]
                ) {
                    return false;
                }
                (hit.point, self.block_reach)
            }
            CrosshairTarget::Entity { point, .. } => (point, self.client_pick.entity_reach),
            _ => return false,
        };
        point.iter().all(|v| v.is_finite())
            && self.pick_distance(point) <= reach
            && self.pick_target_live(result)
    }

    pub(in crate::game) fn refresh_client_pick(&mut self, partial_tick: f32) {
        if !self.native_window()
            || self.title.is_some()
            || self.player.is_none()
            || self.native_lifecycle.epoch().is_none()
        {
            return;
        }
        let Some(camera) = self
            .spectated_camera_snapshot()
            .or_else(|| self.player_camera_entity())
        else {
            return;
        };
        let origin = std::array::from_fn(|i| {
            camera.previous[i]
                + f64::from(partial_tick) * (camera.position[i] - camera.previous[i])
                + if i == 1 {
                    f64::from(camera.eye_height)
                } else {
                    0.0
                }
        });
        let direction = view_vector(
            camera.view_y_rot(partial_tick),
            camera.view_x_rot(partial_tick),
        );
        self.client_pick.ray = Some((origin, direction));
        self.client_pick.origin = origin;
        self.client_pick.epoch = self.native_lifecycle.epoch();
        self.client_pick.dimension = Some(self.dim);
        let from = sunrise_core::geometry::Vec3::new(origin[0], origin[1], origin[2]);
        let max_distance = self.block_reach.max(self.entity_reach);
        let block_hit = self.pick_unfiltered();
        // LocalPlayer.pick: on a block hit, blockDistanceSq shortens both the ray and search bound;
        // on a block miss, search uses nominal maxDistance^2 while final comparison uses to.distanceToSqr(from).
        let (entity_max_distance, search_max_distance_sq, block_distance_sq) = match block_hit {
            Some(hit) => {
                let d2 = from.distance_squared(sunrise_core::geometry::Vec3::new(
                    hit.point[0],
                    hit.point[1],
                    hit.point[2],
                ));
                (d2.sqrt(), d2, d2)
            }
            None => {
                let to = sunrise_core::geometry::Vec3::new(
                    from.x + direction[0] * max_distance,
                    from.y + direction[1] * max_distance,
                    from.z + direction[2] * max_distance,
                );
                (
                    max_distance,
                    max_distance * max_distance,
                    to.distance_squared(from),
                )
            }
        };
        let block_candidate = block_hit
            .filter(|hit| {
                from.distance_squared(sunrise_core::geometry::Vec3::new(
                    hit.point[0],
                    hit.point[1],
                    hit.point[2],
                )) < self.block_reach * self.block_reach
            })
            .and_then(|hit| self.crosshair_block_target(hit));
        let camera_id = self.pick_camera_id();
        let attack_result = self
            .active_attack_range()
            .map(|range| self.attack_range_pick_result(&range));
        let mut entity_in_range = None;
        let original = match attack_result {
            Some(crate::game::pick_entity::AttackRangePickResult::Entity(hit)) => self
                .native_lifecycle
                .epoch()
                .map(|epoch| CrosshairTarget::Entity {
                    entity: EntityRef::new(epoch, hit.id),
                    point: hit.point,
                }),
            Some(crate::game::pick_entity::AttackRangePickResult::Block(hit)) => {
                self.crosshair_block_target(hit)
            }
            None | Some(crate::game::pick_entity::AttackRangePickResult::Miss) => {
                let raw_entity_hit =
                    self.pick_entity_candidate_within(entity_max_distance, search_max_distance_sq);
                // Refuse the pick if a rider listener changed camera, epoch, or dimension.
                if camera_id != self.pick_camera_id()
                    || self.client_pick.epoch != self.native_lifecycle.epoch()
                    || self.client_pick.dimension != Some(self.dim)
                {
                    self.client_pick.ray = None;
                    self.publish_client_pick(None);
                    return;
                }
                entity_in_range = raw_entity_hit.filter(|target| match target {
                    CrosshairTarget::Entity { point, .. } => {
                        from.distance_squared(sunrise_core::geometry::Vec3::new(
                            point[0], point[1], point[2],
                        )) < self.entity_reach * self.entity_reach
                    }
                    _ => false,
                });
                match raw_entity_hit {
                    Some(CrosshairTarget::Entity { point, .. })
                        if from.distance_squared(sunrise_core::geometry::Vec3::new(
                            point[0], point[1], point[2],
                        )) < block_distance_sq =>
                    {
                        entity_in_range
                    }
                    _ => block_candidate,
                }
            }
        };
        self.publish_client_pick(original);
        self.client_pick.taken += 1;
        if !mod_api::__private::listened(mod_api::HookId::ClientPick) {
            self.client_pick.ray = None;
            return;
        }
        // Supply block and entity candidates independently only when a ClientPick listener is active.
        let entity_candidate = if matches!(
            attack_result,
            Some(crate::game::pick_entity::AttackRangePickResult::Entity(_))
        ) {
            original
        } else if entity_in_range.is_some() {
            entity_in_range
        } else if block_hit.is_some()
            || matches!(
                attack_result,
                Some(crate::game::pick_entity::AttackRangePickResult::Block(_))
            )
        {
            let independent = self.pick_entity_candidate_within(
                self.entity_reach,
                self.entity_reach * self.entity_reach,
            );
            if camera_id != self.pick_camera_id()
                || self.client_pick.epoch != self.native_lifecycle.epoch()
                || self.client_pick.dimension != Some(self.dim)
            {
                self.client_pick.ray = None;
                self.publish_client_pick(None);
                return;
            }
            independent.filter(|target| match target {
                CrosshairTarget::Entity { point, .. } => {
                    from.distance_squared(sunrise_core::geometry::Vec3::new(
                        point[0], point[1], point[2],
                    )) < self.entity_reach * self.entity_reach
                }
                _ => false,
            })
        } else {
            None
        };
        self.client_pick.ray = None;
        // Capture the same forward-movement envelope as the vanilla item ray,
        // before listeners can change the player or held item.
        self.client_pick.entity_reach = self.client_pick_entity_reach(direction);
        self.storage_scope(|game, _| {
            game.client_pick.dispatching = true;
            let mut result = original;
            game.native_client_pick(mod_api::__private::ClientPickCall {
                partial_tick,
                origin,
                original,
                block_candidate,
                entity_candidate,
                result: &mut result,
            });
            game.client_pick.dispatching = false;
            let valid_context = game.client_pick.epoch == game.native_lifecycle.epoch()
                && game.client_pick.dimension == Some(game.dim);
            let result = result.filter(|&result| {
                valid_context && (Some(result) == original || game.accept_client_pick(result))
            });
            game.publish_client_pick(result);
            ((), true)
        });
    }

    /// Whether Pick Block can name an item for the entity (`Entity.getPickResult`).
    pub(in crate::game) fn has_pick_result(target: Option<crate::game::commands::Target>) -> bool {
        use crate::game::commands::Target;
        !matches!(
            target,
            Some(
                Target::Projectile(_)
                    | Target::DragonPart(_)
                    | Target::Other(_)
                    | Target::General(_)
                    | Target::Tnt(_)
                    | Target::Falling(_)
            )
        )
    }

    fn publish_client_pick(&mut self, result: Option<CrosshairTarget>) {
        self.client_pick.result = result;
        self.client_pick.entity_metadata = match result {
            Some(CrosshairTarget::Entity { entity, .. }) => (
                !self.interactions.contains_key(&(entity.id() as i32))
                    && Self::has_pick_result(self.native_client_entity_target(entity)),
                self.resident_dragon_part(entity.id()).is_none(),
            ),
            _ => (false, false),
        };
    }

    pub(in crate::game) fn ask_can_rider_interact(
        &mut self,
        subject: mod_api::EntityRef,
        allowed: &mut bool,
    ) {
        let mut access = ClientPickEntityAccess {
            game: self,
            subject,
        };
        Self::native_can_rider_interact(mod_api::__private::CanRiderInteractCall {
            subject: mod_api::EntityClientScope::new(&mut access),
            allowed,
        });
    }

    pub(in crate::game) fn native_can_rider_interact(
        call: mod_api::__private::CanRiderInteractCall<'_>,
    ) {
        Self::fire_can_rider_interact(call);
    }

    pub(in crate::game) fn native_client_pick(
        &mut self,
        call: mod_api::__private::ClientPickCall<'_>,
    ) {
        self.fire_client_pick(call);
    }
}

struct ResidentClientPickEntity {
    kind: Option<mod_api::EntityTypeKey>,
    position: [f64; 3],
    rotation: [f32; 2],
    uuid: Option<u128>,
}

impl mod_api::ResidentClientEntity for ResidentClientPickEntity {
    fn kind(&self) -> Option<mod_api::EntityTypeKey> {
        self.kind
    }
    fn position(&self) -> [f64; 3] {
        self.position
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn uuid(&self) -> Option<u128> {
        self.uuid
    }
}

struct ClientPickEntityAccess<'g, 'w> {
    game: &'g mut Game<'w>,
    subject: mod_api::EntityRef,
}

impl mod_api::EntityClientAccess for ClientPickEntityAccess<'_, '_> {
    fn route_kind(&self) -> Option<mod_api::EntityTypeKey> {
        match self.game.native_entity_read_target(self.subject) {
            Some(target) => self.game.native_entity_kind_key(target),
            None => mod_api::__private::resolved_entity_type_key(
                self.game.native_client_tracked(self.subject)?.0,
            ),
        }
    }

    fn with_view(&mut self, call: &mut dyn FnMut(mod_api::EntityClientView<'_>)) {
        let target = self.game.native_entity_read_target(self.subject);
        let (kind, position, rotation) = match target {
            Some(target) => {
                let Some(position) = self.game.target_position(target) else {
                    return;
                };
                (
                    self.game.native_entity_kind_key(target),
                    position,
                    self.game.target_rotation(target),
                )
            }
            None => {
                let Some((name, position)) = self.game.native_client_tracked(self.subject) else {
                    return;
                };
                (
                    mod_api::__private::resolved_entity_type_key(name),
                    position,
                    [0.0, 0.0],
                )
            }
        };
        let uuid = self.game.remote.as_ref().and_then(|r| {
            let id = i32::try_from(self.subject.id()).ok()?;
            self.game
                .known_resident_entity_uuid_at(self.game.native_level_key(), self.subject.id())
                .map(sunrise_client::lan::playerdata::ints_uuid)
                .or_else(|| r.level_player(id).map(|(uuid, _)| uuid))
                .or_else(|| {
                    self.game
                        .persist
                        .uuids
                        .get(&self.subject.id())
                        .copied()
                        .map(sunrise_client::lan::playerdata::ints_uuid)
                })
        });
        let data = ResidentClientPickEntity {
            kind,
            position,
            rotation,
            uuid,
        };
        call(mod_api::EntityClientView::new(self.subject, &data));
    }

    fn host(&mut self) -> &mut dyn mod_api::ClientHost {
        self.game
    }
}
