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
        let block_hit = self.pick_unfiltered();
        let occluder = block_hit.map(|hit| self.pick_distance(hit.point));
        let block_candidate = block_hit
            .filter(|hit| {
                (0..3)
                    .map(|i| (hit.point[i] - origin[i]).powi(2))
                    .sum::<f64>()
                    <= self.block_reach.powi(2)
            })
            .and_then(|hit| {
                use crate::game::api_values::TryIntoApi;
                Some(CrosshairTarget::Block {
                    pos: mod_api::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                    state: self
                        .world
                        .block(hit.pos[0], hit.pos[1], hit.pos[2])
                        .try_into_api()
                        .ok()?,
                    hit: hit.native(),
                })
            });
        let camera_id = self.pick_camera_id();
        let attack_candidate = self.attack_range_pick_entity();
        let entity_candidate = attack_candidate.or_else(|| self.pick_entity_candidate());
        // A rider listener may change the camera. Neither the entity nor
        // the block traced from the old camera belongs to the new pick.
        if camera_id != self.pick_camera_id()
            || self.client_pick.epoch != self.native_lifecycle.epoch()
            || self.client_pick.dimension != Some(self.dim)
        {
            self.client_pick.ray = None;
            self.publish_client_pick(None);
            return;
        }
        let original = attack_candidate.or_else(|| match entity_candidate {
            Some(CrosshairTarget::Entity { point, .. })
                if occluder.is_none_or(|distance| self.pick_distance(point) < distance) =>
            {
                entity_candidate
            }
            _ => block_candidate,
        });
        self.publish_client_pick(original);
        self.client_pick.taken += 1;
        self.client_pick.ray = None;
        if !mod_api::__private::listened(mod_api::HookId::ClientPick) {
            return;
        }
        // Capture the same forward-movement envelope as the vanilla item ray,
        // before listeners can change the player or held item.
        self.client_pick.entity_reach = self.client_pick_entity_reach(direction);
        let seal = self.transaction_open();
        self.client_pick.dispatching = true;
        let mut result = original;
        self.native_client_pick(mod_api::__private::ClientPickCall {
            partial_tick,
            origin,
            original,
            block_candidate,
            entity_candidate,
            result: &mut result,
        });
        self.client_pick.dispatching = false;
        let valid_context = self.client_pick.epoch == self.native_lifecycle.epoch()
            && self.client_pick.dimension == Some(self.dim);
        let result = result.filter(|&result| {
            valid_context && (Some(result) == original || self.accept_client_pick(result))
        });
        self.publish_client_pick(result);
        self.transaction_close(&seal, true);
    }

    /// Whether Pick Block (middle click) can name an item for the entity: a projectile, a dragon
    /// part, an unmodeled entity or a general row has no `Entity.getPickResult`. This is not
    /// whether the crosshair can target it, which a general type's `pickable` decides.
    pub(in crate::game) fn has_pick_result(target: Option<crate::game::commands::Target>) -> bool {
        use crate::game::commands::Target;
        !matches!(
            target,
            Some(
                Target::Projectile(_)
                    | Target::DragonPart(_)
                    | Target::Other(_)
                    | Target::General(_)
            )
        )
    }

    fn publish_client_pick(&mut self, result: Option<CrosshairTarget>) {
        self.client_pick.result = result;
        self.client_pick.entity_metadata = match result {
            Some(CrosshairTarget::Entity { entity, .. }) => (
                Self::has_pick_result(self.native_client_entity_target(entity)),
                self.level_dragon_part(entity.id()).is_none(),
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
