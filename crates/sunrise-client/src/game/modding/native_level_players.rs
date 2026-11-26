//! Level player visits and chunk/entity tracking queries. The level visit
//! resolves UUIDs from the spawn code's player enumeration so a guest's turn
//! names its player once, excluding anyone who cannot resolve.
use super::Game;
use core::ops::ControlFlow;
use mod_api::{ChunkRef, EntityReadError, EntityRef, LevelReadError, LevelRef, PlayerRef};

impl Game<'_> {
    /// `ServerLevel.players`: the online players of an open server level, each once.
    pub(super) fn level_visit_online_players(
        &self,
        level: LevelRef,
        visit: &mut dyn FnMut(PlayerRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        self.native_level_world_result(level)?;
        Ok(self
            .level_player_uuids(level.dimension_id())
            .filter_map(|uuid| self.player(uuid))
            .try_for_each(visit))
    }

    /// `ChunkMap.TrackedEntity.seenBy`: the players of the level in place whose tracker holds
    /// the entity, other than the entity itself.
    pub(super) fn entity_visit_trackers(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(PlayerRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        let target = self.native_server_entity_target(entity)?;
        if self.target_level(target).key() != self.dim.key() {
            return Err(EntityReadError::Unavailable);
        }
        let id = entity.id() as i32;
        Ok(self
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.iter().map(move |g| (&h.hub, g)))
            .filter(|(hub, g)| hub.here(g) && g.entity_id != id && g.tracker.is_tracking(id))
            .filter_map(|(_, g)| self.player(g.uuid))
            .try_for_each(visit))
    }

    /// `ChunkMap.getPlayers(pos, false)`: the players the chunk was sent to and is
    /// inside the view of.
    pub(super) fn chunk_visit_watchers(
        &self,
        chunk: ChunkRef,
        visit: &mut dyn FnMut(PlayerRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        self.native_level_world_result(chunk.level())?;
        let dimension = chunk.level().dimension_id();
        let [x, z] = chunk.pos();
        Ok(self
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .filter(|g| g.tracks_chunk(dimension, (x, z)))
            .filter_map(|g| self.player(g.uuid))
            .try_for_each(visit))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        Dim,
        native_guest_turn_tests::{place_host, with_guests},
    };
    use super::{ControlFlow, Game};
    use mod_api::{ChunkRef, EntityReadError, LevelReadError, LevelRef, PlayerRef, WorldEpoch};

    /// Open the Nether and go back, so it is a parked level of the server.
    fn park_the_nether(game: &mut super::Game<'_>) {
        game.enter_level(Dim::Nether).unwrap();
        game.enter_level(Dim::Overworld).unwrap();
    }

    fn uuids<E>(
        enumerate: impl FnOnce(
            &mut dyn FnMut(PlayerRef) -> ControlFlow<()>,
        ) -> Result<ControlFlow<()>, E>,
    ) -> Result<Vec<u128>, E> {
        let mut uuids = Vec::new();
        assert_eq!(
            enumerate(&mut |player| {
                uuids.push(player.uuid());
                ControlFlow::Continue(())
            })?,
            ControlFlow::Continue(())
        );
        Ok(uuids)
    }

    fn collect_online_players(game: &Game<'_>, level: LevelRef) -> Vec<(u128, u32)> {
        let mut players = Vec::new();
        assert_eq!(
            level.visit_online_players(game, &mut |player| {
                players.push((player.uuid(), player.entity().id()));
                ControlFlow::Continue(())
            }),
            Ok(ControlFlow::Continue(()))
        );
        players
    }

    /// The host and two guests, each named once with their own entity, in and
    /// out of guest turns; a guest in another level is that level's.
    #[test]
    #[ignore = "daily"]
    fn level_players_name_host_and_guests_once_during_turns() {
        with_guests(|game| {
            park_the_nether(game);
            let _serving = super::super::integrated::tests::install_serving(game, false);
            place_host(game);
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                collect_online_players(game, level),
                [(99, 99), (12, 12), (13, 13)]
            );
            let mut first = Vec::new();
            assert_eq!(
                level.visit_online_players(game, &mut |player| {
                    first.push(player.uuid());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, [99]);

            game.as_guest(12, |game| {
                assert_eq!(collect_online_players(game, level), [(12, 12), (13, 13)]);
                game.as_guest(13, |game| {
                    assert_eq!(
                        collect_online_players(game, level),
                        [(13, 13)],
                        "the outer guest is displaced and cannot resolve"
                    );
                })
                .unwrap();
            })
            .unwrap();

            let nether = game.level("minecraft:the_nether").unwrap();
            let mut unexpected_visits = 0;
            let mut count = |_| {
                unexpected_visits += 1;
                ControlFlow::Continue(())
            };
            assert_eq!(
                nether.visit_online_players(game, &mut count),
                Ok(ControlFlow::Continue(()))
            );
            let host = game.lan.as_mut().unwrap();
            let moved = host.hub.guests.connection_index(13).unwrap();
            host.hub.guests.update(moved, |guest| {
                guest.dimension = "minecraft:the_nether".to_owned();
            });
            assert_eq!(collect_online_players(game, level), [(99, 99), (12, 12)]);
            assert_eq!(collect_online_players(game, nether), [(13, 13)]);

            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(
                stale.visit_online_players(game, &mut count),
                Err(LevelReadError::Stale)
            );
            assert_eq!(unexpected_visits, 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_players_leave_out_fake_players() {
        with_guests(|game| {
            let _serving = super::super::integrated::tests::install_serving(game, false);
            place_host(game);
            game.next_entity = 100;
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 1234, "machine").unwrap();
            assert_eq!(
                collect_online_players(game, level),
                [(99, 99), (12, 12), (13, 13)]
            );
            let conn = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .find(|g| g.uuid == fake.uuid())
                .unwrap()
                .conn;
            game.as_guest(conn, |game| {
                assert_eq!(collect_online_players(game, level), [(12, 12), (13, 13)]);
            })
            .unwrap();
        });
    }

    /// The players whose tracker holds the entity, in server order: a guest far away, the entity's
    /// own player and a fake player are left out, and a stale handle names nothing.
    #[test]
    #[ignore = "daily"]
    fn entity_trackers_are_the_guests_that_hold_the_entity() {
        with_guests(|game| {
            use super::super::integrated::{Integrated, Serving};

            let serving = Serving::fixture(0);
            let window = Integrated::fixture_for(&serving);
            // The tracking pass reads the owner's Entity Distance from the server handle.
            window.set_distances(
                game.settings.render_distance,
                game.settings.simulation_distance,
                game.settings.entity_distance,
            );
            game.serving = Some(serving);
            place_host(game);
            game.next_entity = 100;
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 1234, "machine").unwrap();
            for cz in -1..=1 {
                for cx in -1..=1 {
                    let mut chunk = sunrise_client::chunk::Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world
                        .chunks
                        .insert((cx, cz), std::sync::Arc::new(chunk));
                }
            }
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.ticks = 10;
            hub.guests.update_all(|g| {
                g.max_unacknowledged = 64;
                // These fixtures start in play with the origin chunk already sent.
                g.sent_chunks.insert((0, 0));
            });
            let id = game
                .spawn_mob(
                    "minecraft:pig",
                    [2.5, 64.0, 2.5],
                    true,
                    super::super::creatures::SpawnSite::Finalizing(
                        sunrise_client::mobs::spawn_rules::Reason::Command,
                    ),
                )
                .unwrap();
            let pig = game.entity(id).unwrap();
            let own = game.player(12).unwrap().entity();
            for _ in 0..10 {
                game.tick_lan(false);
            }
            assert_eq!(
                uuids(|visit| pig.visit_trackers(game, visit)),
                Ok(vec![12, 13])
            );
            let mut first = Vec::new();
            assert_eq!(
                pig.visit_trackers(game, &mut |player| {
                    first.push(player.uuid());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, [12]);
            assert_eq!(
                uuids(|visit| own.visit_trackers(game, visit)),
                Ok(vec![13]),
                "not its own player"
            );
            assert!(
                !uuids(|visit| pig.visit_trackers(game, visit))
                    .unwrap()
                    .contains(&fake.uuid())
            );

            let host = game.lan.as_mut().unwrap();
            let far = host.hub.guests.connection_index(13).unwrap();
            host.hub
                .guests
                .update(far, |g| g.position = [900.0, 64.0, 0.0]);
            for _ in 0..10 {
                game.tick_lan(false);
            }
            assert_eq!(uuids(|visit| pig.visit_trackers(game, visit)), Ok(vec![12]));
            let host = game.lan.as_mut().unwrap();
            let last = host.hub.guests.connection_index(12).unwrap();
            host.hub
                .guests
                .update(last, |g| g.position = [900.0, 64.0, 0.0]);
            for _ in 0..10 {
                game.tick_lan(false);
            }
            assert_eq!(
                pig.visit_trackers(game, &mut |_| panic!("no tracker should be visited")),
                Ok(ControlFlow::Continue(()))
            );

            let stale = mod_api::EntityRef::new(WorldEpoch::new(pig.epoch().get() + 1), pig.id());
            assert_eq!(
                stale.visit_trackers(game, &mut |_| panic!("stale tracker visit")),
                Err(EntityReadError::Stale)
            );

            // A live guest in a parked level is not in the level in place: no trackers.
            let host = game.lan.as_mut().unwrap();
            let moved = host.hub.guests.connection_index(13).unwrap();
            host.hub.guests.update(moved, |g| {
                g.dimension = "minecraft:the_nether".to_owned();
            });
            let parked = game.player(13).unwrap().entity();
            assert_eq!(
                parked.visit_trackers(game, &mut |_| panic!("parked tracker visit")),
                Err(EntityReadError::Unavailable)
            );
        });
    }

    /// A guest watches a chunk once it is sent inside its view, and stops when
    /// it moves away or the chunk falls past its view distance.
    #[test]
    #[ignore = "daily"]
    fn chunk_watchers_follow_sent_chunks_inside_the_view() {
        with_guests(|game| {
            park_the_nether(game);
            let _serving = super::super::integrated::tests::install_serving(game, false);
            let level = game.level(game.dim.id()).unwrap();
            let chunk = ChunkRef::new(level, [0, 0]);
            assert_eq!(
                uuids(|visit| chunk.visit_watchers(game, visit)),
                Ok(vec![]),
                "nothing sent yet"
            );
            let host = game.lan.as_mut().unwrap();
            for conn in [12, 13] {
                let i = host.hub.guests.connection_index(conn).unwrap();
                host.hub.guests.update(i, |g| {
                    g.center = Some((0, 0));
                    g.sent_chunks.insert((0, 0));
                });
            }
            assert_eq!(
                uuids(|visit| chunk.visit_watchers(game, visit)),
                Ok(vec![12, 13])
            );
            let mut first = Vec::new();
            assert_eq!(
                chunk.visit_watchers(game, &mut |player| {
                    first.push(player.uuid());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, [12]);
            game.as_guest(12, |game| {
                assert_eq!(
                    uuids(|visit| chunk.visit_watchers(game, visit)),
                    Ok(vec![12, 13]),
                    "the acting guest is named once, by its own UUID"
                );
            })
            .unwrap();

            // View distance 8 (the guests' client setting): ChunkTrackingView.contains
            // keeps a chunk while max(|dx| - 2, 0)^2 + max(|dz| - 2, 0)^2 < 8^2.
            let host = game.lan.as_mut().unwrap();
            let far = host.hub.guests.connection_index(13).unwrap();
            assert_eq!(host.hub.guests[far].view_distance, 8);
            host.hub.guests.update(far, |g| g.center = Some((9, 0)));
            assert_eq!(
                uuids(|visit| chunk.visit_watchers(game, visit)),
                Ok(vec![12, 13]),
                "7 past the buffer"
            );
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.update(far, |g| g.center = Some((10, 0)));
            assert_eq!(
                uuids(|visit| chunk.visit_watchers(game, visit)),
                Ok(vec![12]),
                "8 past the buffer"
            );

            let host = game.lan.as_mut().unwrap();
            let near = host.hub.guests.connection_index(12).unwrap();
            host.hub.guests.update(near, |g| {
                g.dimension = "minecraft:the_nether".to_owned();
            });
            assert_eq!(
                uuids(|visit| chunk.visit_watchers(game, visit)),
                Ok(vec![]),
                "another level's chunk"
            );
            let nether = game.level("minecraft:the_nether").unwrap();
            assert_eq!(
                uuids(|visit| ChunkRef::new(nether, [0, 0]).visit_watchers(game, visit)),
                Ok(vec![12])
            );

            let stale = ChunkRef::new(
                LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension()),
                [0, 0],
            );
            assert_eq!(
                stale.visit_watchers(game, &mut |_| panic!("stale watcher visit")),
                Err(LevelReadError::Stale)
            );
            assert_eq!(
                uuids(
                    |visit| ChunkRef::new(level, [i32::MIN, i32::MAX]).visit_watchers(game, visit)
                ),
                Ok(vec![])
            );
        });
    }
}
