//! The running state of `ProjectileUtil.getEntityHitResult`.
use super::Game;

#[derive(Clone, Copy, Debug)]
pub(super) struct PickCandidate {
    pub id: u32,
    pub distance: f64,
}

pub(super) struct PickAccumulator {
    pub hovered: Option<PickCandidate>,
    nearest: f64,
    candidates: Option<Vec<PickCandidate>>,
}

impl PickAccumulator {
    pub(super) fn new(reach: f64) -> Self {
        Self {
            hovered: None,
            nearest: reach,
            candidates: None,
        }
    }

    pub(super) fn consider(&mut self, game: &Game<'_>, id: u32, distance: f64) -> bool {
        if game.pick_camera_id() == Some(id) {
            return false;
        }
        // A ray that starts inside the box picks it only when `canBePickedFromInside`.
        if distance == 0.0 && !game.pickable_from_inside(id) {
            return false;
        }
        if let Some(candidates) = &mut self.candidates {
            candidates.push(PickCandidate { id, distance });
            return false;
        }
        self.accept(
            PickCandidate { id, distance },
            game.pick_shares_vehicle(id),
            false,
        )
    }

    /// Candidate geometry is copied while the family lists are borrowed; only a
    /// subscribed hook needs this reusable buffer so its Host can borrow Game.
    pub(super) fn recording(reach: f64, mut candidates: Vec<PickCandidate>) -> Self {
        candidates.clear();
        Self {
            candidates: Some(candidates),
            ..Self::new(reach)
        }
    }

    pub(super) fn dispatch(&mut self, game: &mut Game<'_>) -> Vec<PickCandidate> {
        let candidates = self.candidates.take().unwrap_or_default();
        let camera = game.pick_camera_id();
        for &candidate in &candidates {
            if game.pick_camera_id() == Some(candidate.id) {
                continue;
            }
            let same_vehicle = game.pick_shares_vehicle(candidate.id);
            let mut allowed = false;
            if candidate.distance != 0.0
                && (candidate.distance < self.nearest || self.nearest == 0.0)
                && same_vehicle
                && let Some(epoch) = game.native_lifecycle.epoch()
            {
                let entity = mod_api::EntityRef::new(epoch, candidate.id);
                game.ask_can_rider_interact(entity, &mut allowed);
                if game.pick_camera_id() != camera {
                    // The captured ray and vehicle decision no longer belong
                    // to this camera. Refuse the pick, never recast it.
                    self.hovered = None;
                    break;
                }
            }
            self.accept(candidate, same_vehicle, allowed);
        }
        candidates
    }

    fn accept(&mut self, candidate: PickCandidate, same_vehicle: bool, allowed: bool) -> bool {
        let distance = candidate.distance;
        if distance == 0.0 {
            if self.nearest < 0.0 {
                return false;
            }
            self.nearest = 0.0;
        } else if distance < self.nearest || self.nearest == 0.0 {
            if same_vehicle && !allowed {
                if self.nearest != 0.0 {
                    return false;
                }
            } else {
                self.nearest = distance;
            }
        } else {
            return false;
        }
        self.hovered = Some(candidate);
        true
    }
}

impl Game<'_> {
    /// `Entity.canBePickedFromInside`, false only for a sulfur cube holding a body item
    /// (`SulfurCube.canBePickedFromInside`: `!hasBodyItem()`).
    pub(super) fn pickable_from_inside(&self, id: u32) -> bool {
        !self.mobs.iter().any(|m| {
            m.id == id
                && m.species() == sunrise_client::mobs::Species::SulfurCube
                && sunrise_client::mobs::sulfur_cube::has_body(m)
        })
    }

    pub(super) fn pick_camera_id(&self) -> Option<u32> {
        self.remote_camera_id()
            .or_else(|| self.remote.as_ref().map(|r| r.entity_id as u32))
    }

    fn pick_vehicle_root(&self, id: u32) -> Option<u32> {
        let mut root = id;
        let limit = self
            .remote
            .as_ref()
            .map_or(self.mobs.len(), |r| r.passenger_vehicles.len());
        for _ in 0..=limit + 1 {
            let vehicle =
                if self.pick_camera_id() == Some(root) && self.remote_camera_id().is_none() {
                    self.riding
                } else if let Some(remote) = &self.remote {
                    remote.passenger_vehicle(root)
                } else {
                    self.vehicle_of_any(root)
                };
            let Some(vehicle) = vehicle else {
                return Some(root);
            };
            root = vehicle;
        }
        None
    }

    pub(super) fn pick_shares_vehicle(&self, id: u32) -> bool {
        let camera_root = self
            .pick_camera_id()
            .and_then(|camera| self.pick_vehicle_root(camera))
            .or_else(|| {
                self.riding
                    .and_then(|vehicle| self.pick_vehicle_root(vehicle))
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
        let mut pick = PickAccumulator::new(5.0);
        assert!(!pick.accept(
            PickCandidate {
                id: 2,
                distance: 1.0
            },
            true,
            false
        ));
        assert!(pick.accept(
            PickCandidate {
                id: 1,
                distance: 0.0
            },
            true,
            false
        ));
        assert!(pick.accept(
            PickCandidate {
                id: 2,
                distance: 3.0
            },
            true,
            false
        ));
        assert_eq!(pick.hovered.unwrap().id, 2);
        assert_eq!(pick.nearest, 0.0);
        assert!(pick.accept(
            PickCandidate {
                id: 3,
                distance: 4.0
            },
            false,
            false
        ));
        assert_eq!(pick.nearest, 4.0);
        assert!(!pick.accept(
            PickCandidate {
                id: 4,
                distance: 4.5
            },
            false,
            false
        ));
    }
}
