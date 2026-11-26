//! Read-only projections of the connected client's existing state.

use super::{Game, remote::Remote};
#[cfg(test)]
use mod_api::prelude::*;
use mod_api::{
    BlockBreakingState, BlockPos, ClientFrameStats, ClientServer, ClientTabList,
    ClientTabListEntry, PlayerRef,
};

impl Game<'_> {
    fn native_session(&self) -> Option<&Remote> {
        self.native_window().then_some(())?;
        self.native_lifecycle.epoch()?;
        self.remote.as_ref().filter(|remote| remote.logged_in)
    }

    /// The connected server, not a stale reconnect target. None outside a
    /// logged-in client world. External display names are not retained; memory
    /// connections have no address. Friend-invite secrets are redacted.
    pub(super) fn native_client_server(&self) -> Option<ClientServer> {
        let remote = self.native_session()?;
        Some(ClientServer {
            name: self
                .integrated
                .as_ref()
                .map(|server| server.world_name.clone()),
            address: (remote.session.address != "local")
                .then(|| horizon_client::net::shown_address(&remote.session.address)),
            integrated_local: self.integrated.is_some(),
            save_folder: self
                .integrated
                .as_ref()
                .map(|server| server.save_folder.clone()),
        })
    }

    /// The applied language code, not a pending options selection. None outside
    /// a logged-in client world. Read is_paused for the applied pause state.
    pub(super) fn native_client_language(&self) -> Option<String> {
        self.native_session()?;
        Some(horizon_client::lang::code())
    }

    /// `PlayerInfo.getLatency` of the local player in milliseconds, 0 before the server's first
    /// measurement; None outside a logged-in client world or before the server lists the player.
    pub(super) fn native_client_latency(&self) -> Option<u32> {
        self.native_session()?;
        let latency = self.own_latency()?;
        Some(u32::try_from(latency).unwrap_or(0))
    }

    /// The locator bar's tracked waypoints; None outside a logged-in client world.
    pub(super) fn native_client_waypoints(&self) -> Option<mod_api::ClientWaypoints> {
        self.native_session()?;
        Some(waypoints(&self.waypoints, self.locator_camera.map(|c| c.0)))
    }

    /// The listed players' skins as the Tab overlay draws them; None outside a logged-in
    /// client world.
    pub(super) fn native_client_player_skins(&self) -> Option<mod_api::ClientPlayerSkins> {
        let remote = self.native_session()?;
        Some(player_skins(&remote.listed_player_skins()))
    }

    /// The local player's item use (`isUsingItem`, `getUsedItemHand`, `getTicksUsingItem`,
    /// `getUseItemRemainingTicks`); None when no item is in use or outside a logged-in client
    /// world.
    pub(in crate::game) fn local_item_use(&self) -> Option<mod_api::ItemUse> {
        self.native_session()?;
        let (_, ticks) = self.using?;
        let ticks_used = i32::try_from(ticks).unwrap_or(i32::MAX);
        let hand = match self.used_hand()? {
            horizon_client::first_person::Hand::Main => mod_api::Hand::Main,
            horizon_client::first_person::Hand::Off => mod_api::Hand::Off,
        };
        Some(mod_api::ItemUse::new(
            hand,
            ticks_used,
            self.use_duration.saturating_sub(ticks_used),
        ))
    }

    /// `setYRot` / `setXRot` of the local player by a client mod: yaw as given, pitch `% 360`
    /// clamped to -90..90, the previous angles moved by as much (`Entity.turn`). Sends nothing:
    /// the tick's player sync (`remote_send_state`) carries the look.
    pub(in crate::game) fn set_local_rotation(
        &mut self,
        yaw: f32,
        pitch: f32,
    ) -> Result<(), mod_api::LocalRotationError> {
        if self.native_session().is_none() || self.player.is_none() {
            return Err(mod_api::LocalRotationError::NoWorld);
        }
        if !(yaw.is_finite() && pitch.is_finite()) {
            return Err(mod_api::LocalRotationError::NotFinite);
        }
        let pitch = (pitch % 360.0).clamp(-90.0, 90.0);
        let (turned_yaw, turned_pitch) = (yaw - self.yaw, pitch - self.pitch);
        self.rope_rot_old.0 += turned_yaw;
        self.rope_rot_old.1 += turned_pitch;
        self.yaw = yaw;
        self.pitch = pitch;
        if let Some(model) = self.player_model.as_mut() {
            model.previous_head_pitch += turned_pitch;
            model.head_pitch = pitch;
        }
        Ok(())
    }

    /// Existing local frame counters; None outside a logged-in client world.
    /// This read collects no history and sends nothing anywhere.
    pub(super) fn native_client_frame_stats(&self) -> Option<ClientFrameStats> {
        self.native_session()?;
        Some(ClientFrameStats {
            fps: self.fps,
            rendered_frames: self.frames,
            frame_time_ns: self
                .debug_charts
                .frame_times
                .latest()
                .and_then(|time| u64::try_from(time).ok()),
        })
    }

    /// `PlayerTabOverlay`'s list, shown or not: the listed players in `getPlayerInfos` order (at
    /// most 80), each with the name `getNameForDisplay` draws, and the header and footer.
    /// None outside a logged-in client world.
    pub(super) fn native_client_tab_list(&self) -> Option<ClientTabList> {
        use super::api_values::TryIntoApi;
        use horizon_client::tab_list;
        let players: Vec<_> = self
            .native_session()?
            .listed_tab_players()
            .cloned()
            .collect();
        let board = &self.score.board;
        let entries = tab_list::sorted_players(&players, board)
            .into_iter()
            .map(|p| {
                let display = tab_list::name_for_display(p, board)
                    .try_into_api()
                    .unwrap_or_else(|_| mod_api::Text::literal(p.name.clone()));
                // `GameType.byId`: an unknown id is survival.
                let mode =
                    mod_api::GameMode::from_id(p.game_mode).unwrap_or(mod_api::GameMode::Survival);
                ClientTabListEntry::new(p.name.clone(), p.uuid, mode, p.latency, display)
            })
            .collect();
        let text =
            |t: &Option<horizon_client::text::Text>| t.as_ref().and_then(|t| t.try_into_api().ok());
        Some(ClientTabList::new(
            entries,
            text(&self.tab_list.header),
            text(&self.tab_list.footer),
        ))
    }

    pub(super) fn native_local_status(&self, player: PlayerRef) -> Option<()> {
        self.native_session()?;
        (self.local_player()? == player).then_some(())
    }

    /// The local player's predicted breaking state, including Idle. None for
    /// stale, nonlocal or fake handles and outside a logged-in client world.
    pub(super) fn player_block_breaking(&self, player: PlayerRef) -> Option<BlockBreakingState> {
        self.native_local_status(player)?;
        Some(
            self.mining
                .as_ref()
                .map_or(BlockBreakingState::Idle, |mining| {
                    BlockBreakingState::Breaking {
                        position: BlockPos::new(mining.pos[0], mining.pos[1], mining.pos[2]),
                        progress: mining.progress,
                        stage: mining.stage(),
                    }
                }),
        )
    }

    /// The local player's existing cave ambience mood, normally 0 through 1.
    /// None for stale, nonlocal or fake handles and outside a client world.
    pub(super) fn player_ambient_mood(&self, player: PlayerRef) -> Option<f32> {
        self.native_local_status(player)?;
        Some(self.ambience.moodiness)
    }
}

/// `ClientWaypointManager`'s table, in `forEachWaypoint`'s read order: farthest first by
/// `distanceSquared` with a camera entity, no promised order without one, so the read cannot
/// drift from what the bar draws.
fn waypoints(
    waypoints: &horizon_client::locator_bar::ClientWaypoints,
    camera_entity_pos: Option<[f64; 3]>,
) -> mod_api::ClientWaypoints {
    use horizon_protocol::packets::play::{WaypointId, WaypointTarget};
    let rows = waypoints.for_read(camera_entity_pos);
    let entries = rows
        .iter()
        .filter_map(|w| {
            let id = match &w.id {
                WaypointId::Uuid(uuid) => mod_api::ClientWaypointId::Uuid(*uuid),
                WaypointId::Name(name) => mod_api::ClientWaypointId::Name(name.clone()),
            };
            let target = match w.target {
                WaypointTarget::Empty => mod_api::ClientWaypointTarget::Empty,
                WaypointTarget::Block([x, y, z]) => {
                    mod_api::ClientWaypointTarget::Block(mod_api::BlockPos::new(x, y, z))
                }
                WaypointTarget::Chunk([x, z]) => mod_api::ClientWaypointTarget::Chunk([x, z]),
                WaypointTarget::Azimuth(angle) => mod_api::ClientWaypointTarget::Azimuth(angle),
            };
            // A style the wire validator did not accept skips the entry instead of
            // panicking on peer data.
            let style = mod_api::Identifier::parse(&w.style).ok()?;
            Some(mod_api::ClientWaypointEntry::new(
                id,
                target,
                style,
                // The packet stores the colour as ARGB; the API takes RGB, as
                // `living_set_waypoint_color` does.
                w.color.map(|c| (c as u32) & 0x00FF_FFFF),
            ))
        })
        .collect();
    mod_api::ClientWaypoints::new(entries)
}

/// `Remote::listed_player_skins` as `ClientPlayerSkin`s, so the read cannot drift from what the
/// tab list draws.
fn player_skins(skins: &[(u128, String, bool)]) -> mod_api::ClientPlayerSkins {
    let entries = skins
        .iter()
        .map(|(uuid, skin, slim)| mod_api::ClientPlayerSkin::new(*uuid, skin.clone(), *slim))
        .collect();
    mod_api::ClientPlayerSkins::new(entries)
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::native_entity_refs::tests::with_client_game;
    use super::*;

    pub(in crate::game) fn logged_client(
        game: &mut Game<'_>,
    ) -> horizon_protocol::connection::Pipe {
        let (client, server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "SessionReadTest").unwrap();
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        server
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_session_reads_follow_connection_language_and_frame_state() {
        with_client_game(|game| {
            assert_eq!(game.client_session().client_server(), None);
            assert_eq!(game.client_session().client_language(), None);
            assert_eq!(game.client_session().client_frame_stats(), None);
            let _server = logged_client(game);
            let memory = game.client_session().client_server().unwrap();
            assert_eq!(memory.name, None);
            assert_eq!(memory.address, None);
            assert!(
                !memory.integrated_local,
                "a memory guest does not own the server"
            );

            game.remote.as_mut().unwrap().session.address = "127.0.0.1:25565".into();
            let external = game.client_session().client_server().unwrap();
            assert_eq!(external.address.as_deref(), Some("127.0.0.1:25565"));
            assert_eq!(
                external.name, None,
                "Session.name is the player, not the server"
            );
            assert!(
                !external.integrated_local,
                "loopback does not mean integrated"
            );
            game.remote.as_mut().unwrap().session.address = "friend:private-invite".into();
            assert_eq!(
                game.client_session()
                    .client_server()
                    .unwrap()
                    .address
                    .as_deref(),
                Some("Friend's world")
            );

            game.settings.language = "fr_fr".into();
            horizon_client::lang::scoped(horizon_client::lang::Language::of("es_es", []), || {
                assert_eq!(
                    game.client_session().client_language().as_deref(),
                    Some("es_es")
                );
                horizon_client::lang::scoped(
                    horizon_client::lang::Language::of("de_de", []),
                    || {
                        assert_eq!(
                            game.client_session().client_language().as_deref(),
                            Some("de_de")
                        );
                    },
                );
                assert_eq!(
                    game.client_session().client_language().as_deref(),
                    Some("es_es")
                );
            });
            game.debug_charts.frame_times.reset();
            assert_eq!(
                game.client_session()
                    .client_frame_stats()
                    .unwrap()
                    .frame_time_ns,
                None
            );
            game.fps = 63.5;
            game.frames = 99;
            game.debug_charts.frame_times.log(15_432_123);
            assert_eq!(
                game.client_session().client_frame_stats(),
                Some(ClientFrameStats {
                    fps: 63.5,
                    rendered_frames: 99,
                    frame_time_ns: Some(15_432_123),
                })
            );
            game.fps = 59.0;
            game.frames = 100;
            game.debug_charts.frame_times.log(17_123_456);
            assert_eq!(
                game.client_session().client_frame_stats(),
                Some(ClientFrameStats {
                    fps: 59.0,
                    rendered_frames: 100,
                    frame_time_ns: Some(17_123_456),
                })
            );
            game.pause = true;
            assert!(
                game.is_paused(),
                "the existing applied pause read remains canonical"
            );
            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(game.client_session().client_server(), None);
            assert_eq!(game.client_session().client_language(), None);
            assert_eq!(game.client_session().client_frame_stats(), None);
            game.remote = None;
            // A reconnect target must not be mistaken for the current session.
            game.extras_connecting("stale.example:25565", "OldPlayer", false);
            assert_eq!(game.client_session().client_server(), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn latency_and_item_use_read_the_local_player_as_the_client_sees_it() {
        with_client_game(|game| {
            assert_eq!(game.client_session().client_latency(), None);
            assert_eq!(game.local_item_use(), None);
            let _server = logged_client(game);
            assert_eq!(
                game.client_session().client_latency(),
                None,
                "not listed yet"
            );
            game.remote.as_mut().unwrap().list_self(0);
            assert_eq!(game.client_session().client_latency(), Some(0));
            game.remote.as_mut().unwrap().list_self(87);
            assert_eq!(game.client_session().client_latency(), Some(87));

            // A bow (use duration 72000) drawn in the main hand for 7 ticks.
            game.using = Some((game.selected, 7));
            game.use_duration = 72_000;
            assert_eq!(
                game.local_item_use(),
                Some(mod_api::ItemUse::new(mod_api::Hand::Main, 7, 71_993))
            );
            game.using = Some((super::super::shield::OFFHAND_SLOT, 3));
            assert_eq!(
                game.local_item_use().map(|u| u.hand),
                Some(mod_api::Hand::Off)
            );
            game.using = None;
            assert_eq!(game.local_item_use(), None, "released");

            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(game.client_session().client_latency(), None);
            game.using = Some((game.selected, 7));
            assert_eq!(game.local_item_use(), None, "no world");
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn local_player_status_tracks_mining_mood_and_refuses_other_handles() {
        with_client_game(|game| {
            let _server = logged_client(game);
            let player = game.local_player().unwrap();
            assert_eq!(player.block_breaking(game), Some(BlockBreakingState::Idle));
            game.ambience.moodiness = 0.375;
            assert_eq!(player.ambient_mood(game), Some(0.375));
            for (progress, stage) in [(0.0, None), (0.45, Some(4)), (1.0, Some(9))] {
                game.mining = Some(super::super::Mining {
                    pos: [4, 65, -2],
                    progress,
                    per_tick: 0.0,
                    ticks: 3,
                });
                assert_eq!(
                    player.block_breaking(game),
                    Some(BlockBreakingState::Breaking {
                        position: BlockPos::new(4, 65, -2),
                        progress,
                        stage,
                    })
                );
            }
            game.mining = None;
            game.ambience.moodiness = 0.0;
            assert_eq!(player.block_breaking(game), Some(BlockBreakingState::Idle));
            assert_eq!(player.ambient_mood(game), Some(0.0));
            let fields = mod_api::__private::game::player_ref;
            let epoch = player.entity().epoch();
            for other in [
                fields(mod_api::EntityRef::new(epoch, 7), 43),
                fields(mod_api::EntityRef::new(epoch, 8), 42),
                fields(mod_api::EntityRef::new(epoch, 99), 999),
            ] {
                assert_eq!(other.block_breaking(game), None);
                assert_eq!(other.ambient_mood(game), None);
            }
            game.native_lifecycle
                .begin_open(mod_api::Authority::RemoteClient);
            assert_eq!(player.block_breaking(game), None, "the old epoch is stale");
            assert_eq!(player.ambient_mood(game), None);
            let current = game.local_player().unwrap();
            assert_eq!(current.ambient_mood(game), Some(0.0));
            game.options.execution_role =
                super::super::execution_role::ExecutionRole::IntegratedServer;
            assert_eq!(current.block_breaking(game), None);
            assert_eq!(current.ambient_mood(game), None);
            assert_eq!(game.client_session().client_server(), None);
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(current.block_breaking(game), None);
            assert_eq!(current.ambient_mood(game), None);
            game.remote = None;
            assert_eq!(current.block_breaking(game), None);
        });
    }

    /// The waypoint table as `client_waypoints` reads it: every tracked entry, farthest
    /// first by squared distance from the camera entity.
    #[test]
    fn the_waypoints_read_as_the_bar_holds_them() {
        use horizon_client::locator_bar::ClientWaypoints;
        use horizon_protocol::packets::play::{
            TrackedWaypoint, WaypointId, WaypointTarget, waypoint_operation,
        };
        let tracked = |id, style: &str, color, target| TrackedWaypoint {
            operation: waypoint_operation::TRACK,
            id,
            style: style.into(),
            color,
            target,
        };
        let mut table = ClientWaypoints::default();
        // Sent nearest-first, with the azimuth in the middle: nothing here depends on sent order.
        table.apply(&tracked(
            WaypointId::Uuid(7),
            "minecraft:default",
            Some(0xFF10_2030u32 as i32),
            WaypointTarget::Block([2, 64, 3]), // nearest
        ));
        table.apply(&tracked(
            WaypointId::Name("home".into()),
            "minecraft:default",
            None,
            WaypointTarget::Azimuth(0.5),
        ));
        table.apply(&tracked(
            WaypointId::Uuid(9),
            "minecraft:default",
            Some(0xFF00_FF00u32 as i32),
            WaypointTarget::Block([0, 64, 20]), // middle
        ));
        table.apply(&tracked(
            WaypointId::Uuid(11),
            "minecraft:default",
            Some(0xFF10_2040u32 as i32),
            WaypointTarget::Block([30, 64, 0]), // farthest
        ));
        let list = waypoints(&table, Some([0.0, 64.0, 0.0]));
        assert_eq!(
            list.entries.len(),
            4,
            "every tracked waypoint, in the bar's read order"
        );
        // With a camera at [0, 64, 0] the read is farthest first by squared distance from the
        // block centres: the azimuth (no distance at all) first, then [30, 64, 0] (centre
        // 930.75), [0, 64, 20] (420.75) and [2, 64, 3] (18.75) — the reverse of sent order.
        let order: Vec<_> = list.entries.iter().map(|e| e.id.clone()).collect();
        assert_eq!(
            order,
            [
                mod_api::ClientWaypointId::Name("home".into()),
                mod_api::ClientWaypointId::Uuid(11),
                mod_api::ClientWaypointId::Uuid(9),
                mod_api::ClientWaypointId::Uuid(7),
            ],
            "the read shares the locator bar's farthest-first sort"
        );
        let azimuth = &list.entries[0];
        assert_eq!(azimuth.target, mod_api::ClientWaypointTarget::Azimuth(0.5));
        assert_eq!(azimuth.color, None, "no colour sent stays none");
        let far = &list.entries[1];
        assert_eq!(
            far.target,
            mod_api::ClientWaypointTarget::Block(mod_api::BlockPos::new(30, 64, 0))
        );
        assert_eq!(far.style.to_string(), "minecraft:default");
        assert_eq!(far.color, Some(0x10_2040), "ARGB on the wire reads as RGB");
        let mid = &list.entries[2];
        assert_eq!(
            mid.target,
            mod_api::ClientWaypointTarget::Block(mod_api::BlockPos::new(0, 64, 20))
        );
        assert_eq!(mid.color, Some(0x00_FF00));
        let near = &list.entries[3];
        assert_eq!(
            near.target,
            mod_api::ClientWaypointTarget::Block(mod_api::BlockPos::new(2, 64, 3))
        );
        assert_eq!(near.color, Some(0x10_2030));
        // Untracking drops the entry; without a camera entity no order is promised.
        table.apply(&TrackedWaypoint {
            operation: waypoint_operation::UNTRACK,
            id: WaypointId::Uuid(11),
            style: "minecraft:default".into(),
            color: None,
            target: WaypointTarget::Empty,
        });
        let list = waypoints(&table, None);
        assert_eq!(list.entries.len(), 3, "untracked waypoints leave the table");
        assert_eq!(
            list.entries[1].id,
            mod_api::ClientWaypointId::Name("home".into())
        );
    }

    /// The listed players' skins as `client_player_skins` reads them: the resolved path and the
    /// slim flag, joined on UUID.
    #[test]
    fn the_player_skins_read_as_the_tab_list_draws_them() {
        use horizon_client::tab_list::TabPlayer;
        // Listed uuids need a tab entry to be drawn; 4 has none and must not appear.
        let listed: [u128; 4] = [1, 2, 3, 4];
        let tab_players: std::collections::HashMap<u128, TabPlayer> = [
            (1, TabPlayer::new(1, "Slim")),
            (2, TabPlayer::new(2, "Wide")),
            (3, TabPlayer::new(3, "Default")),
        ]
        .into();
        let skins: std::collections::HashMap<u128, String> = [
            (1, "skins/abc123".to_owned()),
            (2, "skins/wide42".to_owned()),
        ]
        .into();
        let skin_slim: std::collections::HashSet<u128> = [1].into();
        let mut rows = crate::game::remote::listed_player_skins(
            listed.iter(),
            &tab_players,
            &skins,
            &skin_slim,
        );
        rows.sort_by_key(|row| row.0);
        assert_eq!(
            rows,
            vec![
                (1, "skins/abc123".to_owned(), true),
                (2, "skins/wide42".to_owned(), false),
                // No resolved skin: the UUID's default body and its own slim flag.
                (3, "entity/player/slim/kai".to_owned(), true),
            ],
            "every drawn player's skin, and no skin for the uuid without a tab entry"
        );
        let skins = player_skins(
            &rows
                .iter()
                .map(|(uuid, skin, slim)| (*uuid, skin.clone(), *slim))
                .collect::<Vec<_>>(),
        );
        assert_eq!(skins.entries.len(), 3);
        assert_eq!(skins.entries[0].uuid, 1);
        assert_eq!(skins.entries[0].skin, "skins/abc123");
        assert!(skins.entries[0].slim, "a downloaded slim skin stays slim");
        assert_eq!(skins.entries[1].uuid, 2);
        assert!(!skins.entries[1].slim, "a wide skin is not slim");
        assert_eq!(skins.entries[2].uuid, 3);
        assert_eq!(skins.entries[2].skin, rows[2].1.clone());
        assert_eq!(
            skins.entries[2].slim, rows[2].2,
            "the default path keeps its slim flag"
        );
    }
}
