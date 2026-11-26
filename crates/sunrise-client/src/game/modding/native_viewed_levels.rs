//! A client mod's reads of the levels the server opened for viewing (#3981, gap 10), as Immersive
//! Portals' `ClientWorldLoader.getWorld(dim)` hands out the far `ClientLevel`: on a client,
//! `host.level(dim)` also names each level the server keeps open on `sunrise:levels`, and
//! `block_state`, `brightness`, `clip`, `visit_block_collision(s)` and `has_collisions` read its world
//! (`viewed_worlds`), as do the reads built on `level_block_state`: `block`, `block_property`
//! and the both-sided fluid reads (`fluid_shape`, `fluid_height`, `fluid_height_for_camera`,
//! `fluid_can_be_replaced_with`). `get_enchant_power_bonus` uses the viewed level as context. A read never writes and never loads a column: direct block
//! reads return None for missing columns, `clip` reads missing columns and out-of-height cells
//! as air (`ClientChunkCache.getChunk(..., true)`), and the collision scan skips missing columns.
//! The handle answers
//! None after the server closes the level or a Login, Respawn or new configuration phase drops
//! every viewed world. A `LevelRef` names a level and the client level's epoch, not one open of
//! the view. A close and a later open within the same epoch make the old handle read again. A
//! Login, reconfiguration or Respawn into another level starts a new epoch, so the old handle stays
//! stale and a mod asks `host.level` again (a same-level Respawn keeps the epoch). Writes,
//! server-only reads, the other reads and the entity reads (until step 9d) refuse it.
use super::Game;
use mod_api::{DimensionKey, LevelReadError, LevelRef};
use sunrise_client::world::World;

impl Game<'_> {
    /// Resolve the constructor's level by name during Load; otherwise the installed level or
    /// a view the server opened. The constructor and installed level have distinct epochs.
    pub(super) fn client_level_by_name(&self, dimension: &str) -> Option<LevelRef> {
        let remote = self.remote.as_ref().filter(|r| r.logged_in)?;
        if let Some(level) = &remote.constructing_level
            && level.handle.dimension_id() == dimension
        {
            return Some(level.handle);
        }
        let key: DimensionKey = if remote.level == dimension {
            remote.level_key
        } else {
            remote.viewed.key_of(dimension)?
        };
        Some(LevelRef::new(self.native_lifecycle.epoch()?, key))
    }
    /// On a client, the world a handle names now: the player's own, or a viewed level's while
    /// the server keeps it open. None before login, for a stale handle and for any other level.
    pub(super) fn client_view_world(&self, level: LevelRef) -> Option<&World> {
        self.client_view_world_result(level).ok()
    }

    pub(super) fn client_view_world_result(
        &self,
        level: LevelRef,
    ) -> Result<&World, LevelReadError> {
        let remote = self
            .remote
            .as_ref()
            .filter(|r| r.logged_in)
            .ok_or(LevelReadError::Unavailable)?;
        if let Some(constructing) = remote.constructing(level) {
            return Ok(&constructing.world);
        }
        if self.native_lifecycle.epoch() != Some(level.epoch()) {
            return Err(LevelReadError::Stale);
        }
        if level.dimension() == remote.level_key {
            return Ok(&self.world);
        }
        remote
            .viewed
            .world_of(level.dimension())
            .ok_or(LevelReadError::Stale)
    }
    /// On a client, the handle the player's own level answers to now (its key and the client
    /// level's epoch): what a world view naming a level is compared with, resolved with the
    /// same rules as `client_view_world`. None before login.
    pub(super) fn client_own_level(&self) -> Option<LevelRef> {
        let remote = self.remote.as_ref().filter(|r| r.logged_in)?;
        Some(LevelRef::new(
            self.native_lifecycle.epoch()?,
            remote.level_key,
        ))
    }
    /// The frusta of the frame's camera `[fov_degrees, yaw, pitch, roll]` and its world views,
    /// which are asked for here: each view's level handle is resolved once, first, against the
    /// player's own.
    pub(in crate::game) fn world_view_frustum(
        &mut self,
        camera: glam::DVec3,
        [fov_degrees, yaw, pitch, roll]: [f32; 4],
        view_transform: glam::Mat4,
        depth_far: f32,
    ) -> Option<sunrise_client::gfx::JoinedFrusta> {
        let own_level = self.client_own_level();
        let renderer = self.renderer.as_mut()?;
        renderer.set_world_view_own_level(own_level);
        Some(renderer.camera_frustum(
            camera,
            fov_degrees,
            yaw,
            pitch,
            roll,
            view_transform,
            depth_far,
        ))
    }
    /// The world for reads such as `block_state`, `brightness` and `clip`: on a server an open
    /// level, in place or parked (`native_level_world`), on a client `client_view_world`.
    pub(super) fn native_level_view_world(&self, level: LevelRef) -> Option<&World> {
        if self.is_logical_server() {
            self.native_level_world(level)
        } else {
            self.client_view_world(level)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{
        Game,
        api_values::TryIntoApi,
        native_entity_refs::tests::{collect_entities_in, run_fresh_cases},
        viewed_worlds::tests::{client, close, column, layered, open, run, spawn},
    };
    use mod_api::{
        BlockFace, BlockState, BlockUpdateFlags, ClipBlock, ClipError, ClipFluid, ClipHit,
        ClipOutcome, CollisionContext, Host, LevelReadError, LevelRef, LevelWriteError, LightLayer,
        WorldEpoch,
    };
    use std::ops::ControlFlow;
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    use sunrise_world::registry::BlockStateId;

    const NETHER: &str = "minecraft:the_nether";
    /// The distinguishing block of 9c's fixture: air in the viewed Nether, stone in the
    /// player's own Overworld.
    const POCKET: [i32; 3] = [3, 70, 5];

    fn set(pos: [i32; 3], state: u32) -> C {
        C::BlockUpdate(play::BlockUpdate {
            pos,
            state: state as i32,
        })
    }

    /// A client in the Overworld whose own column (0, 0) is air with stone at `POCKET`, and
    /// which views the Nether (level 1): its column (0, 0) stone below y 80 and air above, with
    /// air at `POCKET`.
    fn viewing(game: &mut Game) -> (LevelRef, LevelRef) {
        game.handle_packet(layered(0, 0, 0)).unwrap();
        game.handle_packet(set(POCKET, 1)).unwrap();
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[layered(0, 0, 9), set(POCKET, 0)]))
            .unwrap();
        let own = game.level("minecraft:overworld").unwrap();
        let viewed = game.level(NETHER).unwrap();
        (own, viewed)
    }

    /// The block at `pos` through the handle, as its state id: 0 air, 1 stone.
    fn state(host: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<u32> {
        let read = level.block_state(host, pos)?;
        Some(
            [0, 1]
                .into_iter()
                .find(|&id| api(id) == read)
                .expect("air or stone"),
        )
    }

    /// The hit of a ray that found one.
    fn hit_of(outcome: Result<ClipOutcome, ClipError>) -> ClipHit {
        match outcome {
            Ok(ClipOutcome::Hit(hit)) => hit,
            other => panic!("expected a hit, got {other:?}"),
        }
    }

    fn api(id: u32) -> BlockState {
        BlockStateId(id).try_into_api().unwrap()
    }

    /// The viewed level's handle reads the viewed world's block where the player's own level
    /// holds another: air against stone at the same position.
    #[test]
    #[ignore = "daily"]
    fn a_viewed_block_state_differs_from_the_players_own() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (own, viewed) = viewing(&mut game);
        assert_ne!(own, viewed);
        assert_eq!(state(&game, viewed, POCKET), Some(0));
        assert_eq!(state(&game, own, POCKET), Some(1));
        assert_eq!(state(&game, viewed, [3, 71, 5]), Some(1));
        assert_eq!(state(&game, viewed, [3, 80, 5]), Some(0));
        // A level the server did not open has no handle.
        assert_eq!(game.level("minecraft:the_end"), None);
    }

    #[test]
    #[ignore = "daily"]
    fn brightness_reads_the_requested_client_world() {
        if run_fresh_cases(
            "game::native_viewed_levels::tests::brightness_reads_the_requested_client_world",
            "SUNRISE_TEST_VIEWED_LEVEL_BRIGHTNESS",
            &["brightness"],
        ) {
            return;
        }
        let mut resources = None;
        let mut game = client(&mut resources);
        let lit_column = |sky, block| {
            let C::LevelChunkWithLight(mut packet) = layered(0, 0, 0) else {
                unreachable!("layered returns a chunk with light")
            };
            // Light section 1 is the first block section, above the bottom padding.
            packet.light.sky_mask = vec![0b10];
            packet.light.block_mask = vec![0b10];
            packet.light.sky = vec![vec![sky; 2048]];
            packet.light.block = vec![vec![block; 2048]];
            C::LevelChunkWithLight(packet)
        };
        game.handle_packet(lit_column(0xCC, 0x55)).unwrap();
        game.handle_packet(open(1, "test:void")).unwrap();
        game.handle_packet(run(1, &[lit_column(0x77, 0x22)]))
            .unwrap();
        let own = game.level("minecraft:overworld").unwrap();
        let viewed = game.level("test:void").unwrap();
        let read = |host: &dyn Host, level: LevelRef, pos| {
            (
                level.brightness(host, LightLayer::Sky, pos),
                level.brightness(host, LightLayer::Block, pos),
            )
        };

        let lit = [3, -60, 5];
        assert_eq!(read(&game, own, lit), (Some(12), Some(5)));
        assert_eq!(read(&game, viewed, lit), (Some(7), Some(2)));
        let missing = [16, -60, 5];
        assert_eq!(read(&game, own, missing), (None, None));
        assert_eq!(read(&game, viewed, missing), (None, None));
        assert!(!game.world.chunks.contains_key(&(1, 0)));
        let world = &game.remote.as_ref().unwrap().viewed.get(1).unwrap().world;
        assert!(!world.chunks.contains_key(&(1, 0)));

        let stale = LevelRef::new(
            WorldEpoch::new(viewed.epoch().get() + 1),
            viewed.dimension(),
        );
        assert_eq!(read(&game, stale, lit), (None, None));
        game.handle_packet(close(1)).unwrap();
        assert_eq!(read(&game, viewed, lit), (None, None));
        assert_eq!(read(&game, own, lit), (Some(12), Some(5)));
    }

    /// A ray down onto the viewed level's stone hits its top, while the same ray in the
    /// player's own level, all air there, misses.
    #[test]
    #[ignore = "daily"]
    fn clip_hits_the_viewed_block_while_the_players_own_ray_misses() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (own, viewed) = viewing(&mut game);
        let (from, to) = ([8.5, 90.5, 8.5], [8.5, 75.5, 8.5]);
        for mode in [ClipBlock::Collider, ClipBlock::Outline] {
            let hit = hit_of(viewed.clip(
                &game,
                from,
                to,
                mode,
                ClipFluid::None,
                CollisionContext::Empty,
            ));
            assert!(!hit.miss);
            assert_eq!(hit.block, [8, 79, 8]);
            assert_eq!(hit.face, BlockFace::Up);
            assert_eq!(hit.location, [8.5, 80.0, 8.5]);
            assert!(!hit.inside && !hit.world_border);
            assert_eq!(
                own.clip(
                    &game,
                    from,
                    to,
                    mode,
                    ClipFluid::None,
                    CollisionContext::Empty
                ),
                Ok(ClipOutcome::Miss)
            );
        }
    }

    fn collect_collision_boxes(
        game: &Game<'_>,
        level: LevelRef,
        bounds: [f64; 6],
    ) -> Vec<[f64; 6]> {
        let mut boxes = Vec::new();
        let result = level.visit_block_collisions(game, bounds, &mut |shape| {
            boxes.extend(shape.boxes());
            ControlFlow::Continue(())
        });
        assert_eq!(result, Some(ControlFlow::Continue(())));
        boxes
    }

    /// The viewed level's collision boxes are its own: none in the pocket the player's own
    /// level fills with stone, the stone's box where the player's level is air.
    #[test]
    #[ignore = "daily"]
    fn collision_boxes_read_the_viewed_world() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (own, viewed) = viewing(&mut game);
        let pocket = [3.25, 70.25, 5.25, 3.75, 70.75, 5.75];
        assert!(collect_collision_boxes(&game, viewed, pocket).is_empty());
        assert_eq!(viewed.has_collisions(&game, pocket), Some(false));
        assert_eq!(
            collect_collision_boxes(&game, own, pocket),
            [[3.0, 70.0, 5.0, 4.0, 71.0, 6.0]]
        );
        assert_eq!(own.has_collisions(&game, pocket), Some(true));
        let top = [8.25, 79.25, 8.25, 8.75, 79.75, 8.75];
        let cube = [8.0, 79.0, 8.0, 9.0, 80.0, 9.0];
        assert_eq!(collect_collision_boxes(&game, viewed, top), [cube]);
        assert_eq!(viewed.has_collisions(&game, top), Some(true));
        assert!(collect_collision_boxes(&game, own, top).is_empty());
        assert_eq!(own.has_collisions(&game, top), Some(false));

        // More than a million cursor cells, mostly after the first hit along z.
        // An early hit exercises the removed cap without walking those cells.
        let long = [8.25, 79.25, 8.25, 8.75, 79.75, 120_000.75];
        let mut visits = 0;
        assert_eq!(
            viewed.visit_block_collisions(&game, long, &mut |shape| {
                visits += 1;
                let mut boxes = shape.boxes();
                assert_eq!(boxes.next(), Some(cube));
                assert_eq!(boxes.next(), None);
                ControlFlow::Break(())
            }),
            Some(ControlFlow::Break(()))
        );
        assert_eq!(visits, 1);
        assert_eq!(viewed.has_collisions(&game, long), Some(true));
    }

    #[test]
    #[ignore = "daily"]
    fn block_collision_visit_uses_the_named_level_and_stops_on_break() {
        use crate::game::native_entity_refs::tests::run_fresh_cases;
        use core::ops::ControlFlow::{Break, Continue};

        if run_fresh_cases(
            "game::native_viewed_levels::tests::block_collision_visit_uses_the_named_level_and_stops_on_break",
            "SUNRISE_BLOCK_COLLISION_VISIT_CHILD",
            &["viewed"],
        ) {
            return;
        }
        let mut resources = None;
        let mut game = client(&mut resources);
        let (own, viewed) = viewing(&mut game);
        let mut boxes = Vec::new();
        assert_eq!(
            own.visit_block_collision(&game, POCKET, &mut |bounds| {
                boxes.push(bounds);
                Continue(())
            }),
            Ok(Continue(()))
        );
        assert_eq!(boxes, [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]);
        assert_eq!(
            viewed.visit_block_collision(&game, POCKET, &mut |_| panic!("viewed air is empty")),
            Ok(Continue(()))
        );

        // A straight stair has a bottom slab and an upper half-block.
        let stair = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone_stairs")
            .unwrap();
        game.world
            .set(POCKET[0], POCKET[1], POCKET[2], stair)
            .unwrap();
        boxes.clear();
        assert_eq!(
            own.visit_block_collision(&game, POCKET, &mut |bounds| {
                boxes.push(bounds);
                Continue(())
            }),
            Ok(Continue(()))
        );
        assert_eq!(boxes.len(), 2);
        let mut visited = 0;
        assert_eq!(
            own.visit_block_collision(&game, POCKET, &mut |_| {
                visited += 1;
                Break(())
            }),
            Ok(Break(()))
        );
        assert_eq!(visited, 1);

        game.handle_packet(close(1)).unwrap();
        assert_eq!(
            viewed.visit_block_collision(&game, POCKET, &mut |_| panic!("closed level is stale")),
            Err(LevelReadError::Stale)
        );
    }

    /// Missing columns refuse direct reads, stop a `clip` at the first cell inside them and are
    /// skipped by the block collision scan, without mutating either client world. Cells out of
    /// build height are air whatever the column holds.
    #[test]
    #[ignore = "daily"]
    fn a_column_the_viewed_world_does_not_hold_is_none() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (own, viewed) = viewing(&mut game);
        assert_eq!(state(&game, viewed, [20, 70, 5]), None);
        let ray = |level: LevelRef, game: &Game<'_>, y: f64, to: f64| {
            level.clip(
                game,
                [8.5, y, 8.5],
                [to, y, 8.5],
                ClipBlock::Collider,
                ClipFluid::None,
                CollisionContext::Empty,
            )
        };
        // Through the air above the stone: a miss inside column 0, and a stop at the first cell
        // of missing column 1.
        assert_eq!(ray(viewed, &game, 85.5, 15.5), Ok(ClipOutcome::Miss));
        assert_eq!(
            ray(viewed, &game, 85.5, 24.5),
            Ok(ClipOutcome::Unloaded { at: [16, 85, 8] })
        );
        assert_eq!(
            ray(own, &game, 85.5, 24.5),
            Ok(ClipOutcome::Unloaded { at: [16, 85, 8] })
        );
        // Above build height every cell is air, so the unloaded column is never read.
        assert_eq!(ray(viewed, &game, 325.5, 24.5), Ok(ClipOutcome::Miss));
        let across = [15.25, 85.25, 8.25, 32.75, 85.75, 8.75];
        assert!(collect_collision_boxes(&game, viewed, across).is_empty());
        assert_eq!(viewed.has_collisions(&game, across), Some(false));

        game.handle_packet(run(
            1,
            &[layered(1, 0, 0), layered(2, 0, 0), set([32, 85, 8], 1)],
        ))
        .unwrap();
        assert_eq!(
            collect_collision_boxes(&game, viewed, across),
            [[32.0, 85.0, 8.0, 33.0, 86.0, 9.0]]
        );
        assert_eq!(viewed.has_collisions(&game, across), Some(true));
        for mode in [ClipBlock::Collider, ClipBlock::Outline] {
            let after_gap = hit_of(viewed.clip(
                &game,
                [8.5, 85.5, 8.5],
                [34.5, 85.5, 8.5],
                mode,
                ClipFluid::None,
                CollisionContext::Empty,
            ));
            assert!(!after_gap.miss);
            assert_eq!(after_gap.block, [32, 85, 8]);
            assert_eq!(after_gap.face, BlockFace::West);
            assert_eq!(after_gap.location, [32.0, 85.5, 8.5]);
            assert!(!after_gap.inside && !after_gap.world_border);
        }
        let from_east = hit_of(viewed.clip(
            &game,
            [34.5, 85.5, 8.5],
            [8.5, 85.5, 8.5],
            ClipBlock::Collider,
            ClipFluid::None,
            CollisionContext::Empty,
        ));
        assert!(!from_east.miss);
        assert_eq!(from_east.block, [32, 85, 8]);
        assert_eq!(from_east.face, BlockFace::East);
        assert_eq!(from_east.location, [33.0, 85.5, 8.5]);

        // A straight ray through more than 4096 cells has no length cap: it hits the stone at the
        // end of 300 loaded columns, and misses the row above it.
        let mut columns: Vec<_> = (1..=300).map(|x| layered(x, 0, 0)).collect();
        columns.push(set([4805, 85, 8], 1));
        game.handle_packet(run(1, &columns)).unwrap();
        let far_hit = hit_of(viewed.clip(
            &game,
            [33.5, 85.5, 8.5],
            [4810.5, 85.5, 8.5],
            ClipBlock::Collider,
            ClipFluid::None,
            CollisionContext::Empty,
        ));
        assert!(!far_hit.miss);
        assert_eq!(far_hit.block, [4805, 85, 8]);
        assert_eq!(far_hit.face, BlockFace::West);
        assert_eq!(far_hit.location, [4805.0, 85.5, 8.5]);
        assert!(!far_hit.inside && !far_hit.world_border);
        assert_eq!(
            viewed.clip(
                &game,
                [33.5, 86.5, 8.5],
                [4810.5, 86.5, 8.5],
                ClipBlock::Collider,
                ClipFluid::None,
                CollisionContext::Empty,
            ),
            Ok(ClipOutcome::Miss)
        );
        let world = &game.remote.as_ref().unwrap().viewed.get(1).unwrap().world;
        assert!(!world.chunks.contains_key(&(301, 0)));
        assert_eq!(world.chunks.len(), 301);

        // The client's own world also hits once the gap is loaded, and stops at it before.
        game.handle_packet(layered(2, 0, 0)).unwrap();
        game.handle_packet(set([32, 85, 8], 1)).unwrap();
        assert_eq!(
            own.clip(
                &game,
                [8.5, 85.5, 8.5],
                [34.5, 85.5, 8.5],
                ClipBlock::Collider,
                ClipFluid::None,
                CollisionContext::Empty,
            ),
            Ok(ClipOutcome::Unloaded { at: [16, 85, 8] })
        );
        game.handle_packet(layered(1, 0, 0)).unwrap();
        let own_hit = hit_of(own.clip(
            &game,
            [8.5, 85.5, 8.5],
            [34.5, 85.5, 8.5],
            ClipBlock::Collider,
            ClipFluid::None,
            CollisionContext::Empty,
        ));
        assert!(!own_hit.miss);
        assert_eq!(own_hit.block, [32, 85, 8]);
        assert_eq!(own_hit.face, BlockFace::West);
        assert_eq!(own_hit.location, [32.0, 85.5, 8.5]);
        assert!(!game.world.chunks.contains_key(&(3, 0)));
        assert_eq!(game.world.chunks.len(), 3);
    }

    /// Writes, server-only reads and the entity reads refuse a viewed level, which keeps its
    /// block.
    #[test]
    #[ignore = "daily"]
    fn writes_and_entity_reads_refuse_a_viewed_level() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (_, viewed) = viewing(&mut game);
        assert_eq!(
            viewed.set_block(&mut game, POCKET, api(1), BlockUpdateFlags::CLIENTS),
            Err(LevelWriteError::Unavailable)
        );
        assert_eq!(state(&game, viewed, POCKET), Some(0));
        assert_eq!(viewed.fluid_state(&game, POCKET), None);
        assert_eq!(
            collect_entities_in(&game, viewed, [0.0; 3], [16.0; 3]),
            Err(LevelReadError::Unavailable)
        );
    }

    /// The server's close, and a Respawn, end the handle: it and `host.level` answer None until
    /// the server opens the level again.
    #[test]
    #[ignore = "daily"]
    fn a_close_and_a_respawn_end_the_viewed_handle() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (_, viewed) = viewing(&mut game);
        game.handle_packet(close(1)).unwrap();
        assert_eq!(game.level(NETHER), None);
        assert_eq!(state(&game, viewed, [3, 71, 5]), None);
        assert_eq!(
            viewed.visit_block_collisions(
                &game,
                [8.0, 79.0, 8.0, 9.0, 80.0, 9.0],
                &mut |_| panic!("stale level")
            ),
            None
        );
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        assert_eq!(state(&game, viewed, [3, 71, 5]), Some(1));
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:overworld"),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.level(NETHER), None);
        assert_eq!(state(&game, viewed, [3, 71, 5]), None);
        let ray = viewed.clip(
            &game,
            [8.5, 90.5, 8.5],
            [8.5, 75.5, 8.5],
            ClipBlock::Collider,
            ClipFluid::None,
            CollisionContext::Empty,
        );
        assert_eq!(ray, Err(ClipError::StaleLevel));
        // The same-level Respawn retains the epoch, but not the previous world's blocks.
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[layered(0, 0, 0)])).unwrap();
        assert_eq!(state(&game, viewed, [3, 71, 5]), Some(0));

        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:the_end"),
            keep: 0,
        }))
        .unwrap();
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        assert_eq!(state(&game, viewed, [3, 71, 5]), None);
        let reopened = game.level(NETHER).unwrap();
        assert_eq!(state(&game, reopened, [3, 71, 5]), Some(1));
    }
}
