//! Explicit client attack batches; the stored tick/frame pick lives in client_pick.
use super::{Game, commands::Target};
use mod_api::{CrosshairTarget, EntityRef};

fn target_point(eye: [f64; 3], bounds: [[f64; 3]; 2], reach: f64) -> Option<([f64; 3], f64)> {
    if !reach.is_finite()
        || reach < 0.0
        || (0..3).any(|i| {
            !eye[i].is_finite()
                || !bounds[0][i].is_finite()
                || !bounds[1][i].is_finite()
                || bounds[0][i] > bounds[1][i]
        })
    {
        return None;
    }
    let point = std::array::from_fn(|i| eye[i].clamp(bounds[0][i], bounds[1][i]));
    let distance = (0..3)
        .map(|i| (point[i] - eye[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    (distance <= reach).then_some((point, distance))
}

impl Game<'_> {
    pub(super) fn targeting_entity_hit(&self, entity: EntityRef) -> Option<([f64; 3], f64)> {
        let remote = self.remote.as_ref().filter(|r| r.logged_in)?;
        if !self.native_window() || remote.entity_id as u32 == entity.id() {
            return None;
        }
        let target = self.native_client_entity_target(entity)?;
        let attackable = match target {
            Target::Mob(_)
            | Target::Stand(_)
            | Target::Frame(_)
            | Target::Painting(_)
            | Target::Boat(_)
            | Target::Cart(_)
            | Target::Crystal(_)
            | Target::DragonPart(_)
            | Target::Cushion(_) => true,
            Target::Other(id) => self.interactions.contains_key(&(id as i32)),
            Target::Projectile(id) => self
                .creatures
                .projectiles
                .iter()
                .any(|p| p.id == id && self.projectile_is_pickable(p)),
            Target::Trident(id) => self
                .tridents
                .iter()
                .any(|t| t.id == id && self.trident_is_pickable(t)),
            _ => false,
        };
        if !attackable {
            return None;
        }
        let bounds = self.entity_bounding_box(entity).or_else(|| {
            self.tracked_special_box(entity.id() as i32).map(|b| {
                let (a, b) = (b.min_position(), b.max_position());
                [[a.x, a.y, a.z], [b.x, b.y, b.z]]
            })
        })?;
        let eye = self.eye_position()?;
        let (point, distance) = target_point(eye, bounds, self.entity_reach)?;
        if !self.held_range_allows(distance) {
            return None;
        }
        if distance > 0.0 {
            let dir = std::array::from_fn(|i| (point[i] - eye[i]) / distance);
            if self.clip_ray(eye, dir, distance).is_some() {
                return None;
            }
        }
        Some((point, distance))
    }
    /// The optional batch is resolved once for this click, then each hit uses vanilla's send path.
    pub(super) fn mod_attack_targets(&mut self) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ClientAttackTarget) {
            return false;
        }
        let committed = self.crosshair_target();
        let selected = match committed {
            Some(CrosshairTarget::Entity { entity, point }) => Some((entity, point)),
            _ => None,
        };
        let mut targets: Vec<_> = selected.map(|(entity, _)| entity).into_iter().collect();
        let mut canceled = false;
        self.native_client_attack_target(&mut targets, &mut canceled);
        if canceled {
            return true;
        }
        let mut seen = std::collections::HashSet::new();
        for &entity in &targets {
            if !seen.insert(entity) {
                continue;
            }
            let distance =
                if let Some((_, point)) = selected.filter(|(selected, _)| *selected == entity) {
                    // This is the stored pick, not an independently requested batch target.
                    // Preserve its point and occlusion policy for this stored decision.
                    if self.crosshair_target() == committed {
                        self.attack_hit_distance(point)
                    } else {
                        None
                    }
                } else {
                    self.targeting_entity_hit(entity)
                        .map(|(_, distance)| distance)
                };
            if let Some(distance) = distance {
                self.remote_attack_entity(entity.id() as i32, distance);
            }
        }
        selected.is_some() || !targets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_range_uses_nearest_box_point_and_rejects_invalid_geometry() {
        let bounds = [[2.0, -1.0, -1.0], [4.0, 1.0, 1.0]];
        assert_eq!(
            target_point([0.0; 3], bounds, 2.0),
            Some(([2.0, 0.0, 0.0], 2.0))
        );
        assert_eq!(target_point([0.0; 3], bounds, 1.999), None);
        assert_eq!(
            target_point([3.0, 0.0, 0.0], bounds, 0.0),
            Some(([3.0, 0.0, 0.0], 0.0))
        );
        for reach in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(target_point([0.0; 3], bounds, reach), None);
        }
        assert_eq!(target_point([f64::NAN; 3], bounds, 3.0), None);
        assert_eq!(target_point([0.0; 3], [bounds[1], bounds[0]], 3.0), None);
    }

    #[derive(Default)]
    struct AttackBatch {
        targets: std::sync::Mutex<Vec<EntityRef>>,
        cancel: std::sync::atomic::AtomicBool,
    }
    struct Observer(std::sync::Arc<AttackBatch>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Observer {
        fn client_attack_target(
            &self,
            _: &mut dyn mod_api::ClientHost,
            event: &mut mod_api::ClientAttackTargetEvent<'_>,
        ) -> mod_api::Flow {
            *event.targets = self.0.targets.lock().unwrap().clone();
            if self.0.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    fn finish_login(
        game: &mut crate::game::Game<'_>,
        server: &mut sunrise_protocol::connection::Connection,
    ) {
        use sunrise_protocol::packets::{Empty, configuration as cfg, login};
        server.recv().unwrap(); // Intention.
        server.recv().unwrap(); // Hello.
        server
            .send(
                &login::Clientbound::LoginFinished(login::LoginFinished {
                    profile: login::GameProfile {
                        uuid: 42,
                        name: "SessionReadTest".into(),
                        properties: vec![],
                    },
                    session: 1,
                })
                .encode(),
            )
            .unwrap();
        let (id, payload) = server.recv().unwrap();
        assert!(matches!(
            login::Serverbound::decode(id, &payload).unwrap(),
            login::Serverbound::LoginAcknowledged(_)
        ));
        server.recv().unwrap(); // Brand.
        server.recv().unwrap(); // Client information.
        server
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        game.acknowledge_test_configuration();
        let (id, payload) = server.recv().unwrap();
        assert!(matches!(
            cfg::Serverbound::decode(id, &payload).unwrap(),
            cfg::Serverbound::FinishConfiguration(_)
        ));
    }

    /// A FIFO swing packet fences the real connection's outgoing attack packets.
    fn sent_attacks(
        game: &Game<'_>,
        server: &mut sunrise_protocol::connection::Connection,
    ) -> Vec<i32> {
        use sunrise_protocol::packets::{Empty, play::Serverbound as S};
        game.send(S::Punch(Empty));
        receive_attacks(server)
    }

    fn receive_attacks(server: &mut sunrise_protocol::connection::Connection) -> Vec<i32> {
        use sunrise_protocol::packets::play::Serverbound as S;
        let mut ids = Vec::new();
        loop {
            let (id, payload) = server.recv().unwrap();
            match S::decode(id, &payload).unwrap() {
                S::Attack(attack) => ids.push(attack.id),
                S::Punch(_) => return ids,
                _ => {}
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn client_attack_batch_keeps_order_deduplication_and_server_constraints() {
        use crate::game::{
            native_client_session::tests::logged_client,
            native_entity_refs::tests::with_client_game,
        };
        use sunrise_client::{chunk::Chunk, motion::PlayerMotion};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::play::{self, Clientbound as C};
        let state = std::sync::Arc::new(AttackBatch::default());
        let mods = vec![mod_api::ModEntry::new(
            "attack_batch_test",
            Box::new(Observer(state.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let mut server =
                    sunrise_protocol::connection::Connection::local(logged_client(game));
                server
                    .reader
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                finish_login(game, &mut server);
                game.native_lifecycle.begin_open(None);
                game.title = None;
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 1.0, 0.5)).unwrap());
                game.entity_reach = 3.0;
                game.world
                    .chunks
                    .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
                let kind = game
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id("minecraft:pig")
                    .unwrap();
                for (id, position) in [(31, [2.5, 2.0, 0.5]), (32, [0.5, 2.0, 2.5])] {
                    game.handle_packet(C::AddEntity(play::AddEntity {
                        id,
                        uuid: id as u128,
                        kind,
                        position,
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    }))
                    .unwrap();
                }
                let first = game.entity(31).unwrap();
                let second = game.entity(32).unwrap();
                *state.targets.lock().unwrap() = vec![first, second, first];
                assert!(game.mod_attack_targets());
                assert_eq!(sent_attacks(game, &mut server), [31, 32]);
                *state.targets.lock().unwrap() = (0..65).flat_map(|_| [second, first]).collect();
                assert!(game.mod_attack_targets());
                assert_eq!(sent_attacks(game, &mut server), [32, 31]);
                state
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                assert!(game.mod_attack_targets());
                assert!(sent_attacks(game, &mut server).is_empty());
                state
                    .cancel
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                game.entity_reach = 1.0;
                assert!(game.mod_attack_targets());
                assert!(sent_attacks(game, &mut server).is_empty());
                game.entity_reach = 3.0;
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(1, 2, 0, stone).unwrap();
                *state.targets.lock().unwrap() = vec![first];
                assert!(game.mod_attack_targets());
                assert!(
                    sent_attacks(game, &mut server).is_empty(),
                    "explicit extra batch targets still obey occlusion"
                );
                game.native_lifecycle.begin_open(None);
                assert!(game.mod_attack_targets());
                assert!(sent_attacks(game, &mut server).is_empty());
            })
        });
    }
}
