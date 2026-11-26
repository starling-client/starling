//! The running state of `ProjectileUtil.getEntityHitResult`.
use super::Game;
use sunrise_core::geometry::{Aabb, Vec3};

#[derive(Clone, Copy, Debug)]
pub(super) struct PickCandidate {
    pub id: u32,
    pub point: [f64; 3],
    pub distance_sq: f64,
    pub inside: bool,
}

/// Ray segment `(from, to)` matching `LocalPlayer.pick`.
pub(super) fn pick_ray_segment(eye: [f64; 3], dir: [f64; 3], reach: f64) -> (Vec3, Vec3) {
    let from = Vec3::new(eye[0], eye[1], eye[2]);
    let to = Vec3::new(
        from.x + dir[0] * reach,
        from.y + dir[1] * reach,
        from.z + dir[2] * reach,
    );
    (from, to)
}

pub(super) struct PickAccumulator {
    pub hovered: Option<PickCandidate>,
    nearest_sq: f64,
    candidates: Option<Vec<PickCandidate>>,
}

impl PickAccumulator {
    pub(super) fn new(max_distance_sq: f64) -> Self {
        Self {
            hovered: None,
            nearest_sq: max_distance_sq,
            candidates: None,
        }
    }

    pub(super) fn consider_candidate(&mut self, game: &Game<'_>, candidate: PickCandidate) -> bool {
        if let Some(candidates) = &mut self.candidates {
            candidates.push(candidate);
            return false;
        }
        self.accept(candidate, game.pick_shares_vehicle(candidate.id), false)
    }

    pub(super) fn consider_box(
        &mut self,
        game: &Game<'_>,
        id: u32,
        from: Vec3,
        to: Vec3,
        bb: Aabb,
    ) -> bool {
        let Some(candidate) = game.test_pick_box(id, from, to, bb) else {
            return false;
        };
        self.consider_candidate(game, candidate)
    }

    /// Reuses the caller's candidate buffer when `CanRiderInteract` is subscribed.
    pub(super) fn recording(max_distance_sq: f64, mut candidates: Vec<PickCandidate>) -> Self {
        candidates.clear();
        Self {
            candidates: Some(candidates),
            ..Self::new(max_distance_sq)
        }
    }

    pub(super) fn dispatch(&mut self, game: &mut Game<'_>) -> Vec<PickCandidate> {
        let candidates = self.candidates.take().unwrap_or_default();
        let camera = game.pick_camera_id();
        for &candidate in &candidates {
            if game.is_pick_camera(candidate.id) {
                continue;
            }
            let same_vehicle = game.pick_shares_vehicle(candidate.id);
            let mut allowed = false;
            if !candidate.inside
                && (candidate.distance_sq < self.nearest_sq || self.nearest_sq == 0.0)
                && same_vehicle
                && let Some(epoch) = game.native_lifecycle.epoch()
                && let Some(wire_id) = game.wire_entity_id(candidate.id)
            {
                let entity = mod_api::EntityRef::new(epoch, wire_id as u32);
                game.ask_can_rider_interact(entity, &mut allowed);
                if game.pick_camera_id() != camera {
                    // Camera changed during hook; refuse the pick rather than recasting it.
                    self.hovered = None;
                    break;
                }
            }
            self.accept(candidate, same_vehicle, allowed);
        }
        candidates
    }

    fn accept(&mut self, candidate: PickCandidate, same_vehicle: bool, allowed: bool) -> bool {
        if candidate.inside {
            if self.nearest_sq < 0.0 {
                return false;
            }
            self.nearest_sq = 0.0;
        } else if candidate.distance_sq < self.nearest_sq || self.nearest_sq == 0.0 {
            if same_vehicle && !allowed {
                if self.nearest_sq != 0.0 {
                    return false;
                }
            } else {
                self.nearest_sq = candidate.distance_sq;
            }
        } else {
            return false;
        }
        self.hovered = Some(candidate);
        true
    }
}

impl Game<'_> {
    /// Vanilla `ProjectileUtil.getEntityHitResult` per-box test (`AABB.contains` and `AABB.clip`).
    pub(super) fn test_pick_box(
        &self,
        id: u32,
        from: Vec3,
        to: Vec3,
        bb: Aabb,
    ) -> Option<PickCandidate> {
        if self.is_pick_camera(id) {
            return None;
        }
        let clip_point = bb.clip(from, to);
        if bb.contains(from) {
            if !self.pickable_from_inside(id) {
                return None;
            }
            let at = clip_point.unwrap_or(from);
            Some(PickCandidate {
                id,
                point: [at.x, at.y, at.z],
                distance_sq: 0.0,
                inside: true,
            })
        } else {
            let at = clip_point?;
            Some(PickCandidate {
                id,
                point: [at.x, at.y, at.z],
                distance_sq: from.distance_squared(at),
                inside: false,
            })
        }
    }

    /// `Entity.canBePickedFromInside`, false only for a sulfur cube holding a body item.
    pub(super) fn pickable_from_inside(&self, id: u32) -> bool {
        !self.mobs.iter().any(|m| {
            m.id == id
                && m.species() == sunrise_client::mobs::Species::SulfurCube
                && sunrise_client::mobs::sulfur_cube::has_body(m)
        })
    }

    pub(super) fn pick_camera_id(&self) -> Option<u32> {
        if let Some(remote) = self.remote.as_ref() {
            if let Some(id) = remote.camera {
                remote.detached_camera.is_none().then_some(id as u32)
            } else {
                Some(remote.entity_id as u32)
            }
        } else {
            self.player
                .as_ref()
                .map(|_| self.own_entity_id() as u32)
                .filter(|&id| id != 0)
        }
    }

    /// True if `id` (or the wire entity behind a synthetic special-target id) is the camera entity.
    pub(super) fn is_pick_camera(&self, id: u32) -> bool {
        let Some(camera) = self.pick_camera_id() else {
            return false;
        };
        id == camera || self.wire_entity_id(id) == Some(camera as i32)
    }

    fn pick_vehicle_step(&self, entity: u32) -> Option<u32> {
        if self.pick_camera_id() == Some(entity) && self.remote_camera_id().is_none() {
            return self.riding.or_else(|| {
                self.remote
                    .as_ref()
                    .and_then(|r| r.passenger_vehicle(entity))
            });
        }
        if let Some(remote) = &self.remote {
            return remote.passenger_vehicle(entity);
        }
        self.vehicle_of_any(entity)
            .or_else(|| {
                self.armor_stands
                    .iter()
                    .find(|a| a.id == entity)
                    .and_then(|a| a.vehicle)
            })
            .or_else(|| {
                self.interactions
                    .get(&(entity as i32))
                    .and_then(|i| i.vehicle)
            })
            .or_else(|| {
                self.text_displays
                    .get(&(entity as i32))
                    .and_then(|d| d.vehicle)
            })
    }

    /// Walks the full vehicle chain (including retained off-level entities) with cycle detection.
    fn pick_vehicle_root(&self, id: u32) -> Option<u32> {
        let start = self.wire_entity_id(id).map_or(id, |wire| wire as u32);
        let mut slow = start;
        let mut fast = start;
        loop {
            let Some(next) = self.pick_vehicle_step(fast) else {
                return Some(fast);
            };
            let Some(next2) = self.pick_vehicle_step(next) else {
                return Some(next);
            };
            fast = next2;
            slow = self.pick_vehicle_step(slow)?;
            if slow == fast {
                return None;
            }
        }
    }

    pub(super) fn pick_shares_vehicle(&self, id: u32) -> bool {
        let camera_root = self
            .pick_camera_id()
            .and_then(|camera| self.pick_vehicle_root(camera))
            .or_else(|| {
                self.controls_camera()
                    .then(|| self.riding.and_then(|v| self.pick_vehicle_root(v)))
                    .flatten()
            });
        camera_root.is_some() && self.pick_vehicle_root(id) == camera_root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ProjectileUtil.getEntityHitResult` with `SulfurCube.canBePickedFromInside`: from inside its
    /// box, a cube is picked without a body item and not with one; from outside, it is picked
    /// either way.
    #[test]
    #[ignore = "daily"]
    fn a_sulfur_cube_holding_a_block_is_not_picked_from_inside() {
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            // A monster can't be summoned on peaceful.
            assert!(game.run_command("difficulty normal"));
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command(
                "summon minecraft:sulfur_cube 4.5 200 4.5 {NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Size:1}"
            ));
            let cube = game
                .mobs
                .iter()
                .find(|m| m.species() == sunrise_client::mobs::Species::SulfurCube)
                .expect("the cube");
            let (id, bounds) = (cube.id, cube.bounds());
            let (min, max) = (bounds.min_position(), bounds.max_position());
            let centre = Vec3::new(
                (min.x + max.x) / 2.0,
                (min.y + max.y) / 2.0,
                (min.z + max.z) / 2.0,
            );
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap());
            let eye = game.aim().unwrap().0[1] - 64.0;
            // Yaw 0 looks along +z.
            (game.yaw, game.pitch) = (0.0, 0.0);
            let eyes_at = |game: &mut Game<'_>, at: Vec3| {
                game.player = Some(
                    PlayerMotion::new(game.motion, Vec3::new(at.x, at.y - eye, at.z)).unwrap(),
                );
            };
            let picked = |game: &Game<'_>| game.mob_hit().map(|(i, _, _)| game.mobs[i].id);
            eyes_at(game, centre);
            assert_eq!(
                picked(game),
                Some(id),
                "an empty cube is picked from inside"
            );
            let cube = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            cube.extra.set_body(Some("minecraft:oak_planks".into()));
            assert_eq!(picked(game), None, "a cube holding a block is not");
            eyes_at(game, Vec3::new(centre.x, centre.y, min.z - 1.0));
            assert_eq!(picked(game), Some(id), "from outside it still is");
        });
    }

    #[test]
    fn same_vehicle_uses_the_running_nearest_not_the_candidates_distance() {
        let mut pick = PickAccumulator::new(25.0);
        assert!(!pick.accept(
            PickCandidate {
                id: 2,
                point: [0.0; 3],
                distance_sq: 1.0,
                inside: false,
            },
            true,
            false
        ));
        assert!(pick.accept(
            PickCandidate {
                id: 1,
                point: [0.0; 3],
                distance_sq: 0.0,
                inside: true,
            },
            true,
            false
        ));
        assert!(pick.accept(
            PickCandidate {
                id: 2,
                point: [0.0; 3],
                distance_sq: 9.0,
                inside: false,
            },
            true,
            false
        ));
        assert_eq!(pick.hovered.unwrap().id, 2);
        assert_eq!(pick.nearest_sq, 0.0);
        assert!(pick.accept(
            PickCandidate {
                id: 3,
                point: [0.0; 3],
                distance_sq: 16.0,
                inside: false,
            },
            false,
            false
        ));
        assert_eq!(pick.nearest_sq, 16.0);
        assert!(!pick.accept(
            PickCandidate {
                id: 4,
                point: [0.0; 3],
                distance_sq: 20.25,
                inside: false,
            },
            false,
            false
        ));
    }
}
