//! Where a portal leads, asked through a level handle (`LevelRef::portal_destination`,
//! `LevelRef::find_or_create_portal_exit`). Both read the level in place and the parked
//! levels only: a level that is not open, or a chunk the search reads that is not loaded,
//! is `ChunksNotLoaded` (vanilla loads them), and nothing is read from the save. The search is
//! the one dimension travel runs, over a side of it (`PortalSide`).
use super::Game;
use crate::game::{
    Dim,
    end::END_SPAWN,
    nether_portal::{Plan, PortalSide, end_portal_rotation},
};
use mod_api::{LevelRef, PortalAngle, PortalError, PortalExit, PortalRotation, PortalShape};
use sunrise_client::containers::BlockEntity;
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_world::level::portal_travel::FoundRectangle;

/// How far `largest_rectangle_around` reads from a block: the portal's limit in both directions.
const RECTANGLE: i32 = 21;
/// Blocks past a portal that its collision-free arrival reads (an entity box over 4 blocks is
/// not moved).
const COLLISION: i32 = 5;

/// What a probe of a portal block found.
enum Probe {
    /// An End portal or gateway: the answer.
    Fixed(Dim, Vec3, PortalRotation),
    /// A nether portal: the plan, and the exit portal when one stands.
    Nether(Plan, Option<FoundRectangle>),
}

impl Game<'_> {
    fn portal_error(error: mod_api::LevelReadError) -> PortalError {
        match error {
            mod_api::LevelReadError::Stale => PortalError::LevelMissing,
            _ => PortalError::Unavailable,
        }
    }

    /// Every chunk holding a block within `reach` blocks of column (`x`, `z`) is loaded in `side`.
    fn blocks_loaded(side: &PortalSide<'_>, x: i32, z: i32, reach: i32) -> bool {
        // `x` and `z` are a mod's: a sum past the ends of `i32` stops at them, in a chunk that
        // is not loaded.
        let (min_x, max_x) = (x.saturating_sub(reach) >> 4, x.saturating_add(reach) >> 4);
        let (min_z, max_z) = (z.saturating_sub(reach) >> 4, z.saturating_add(reach) >> 4);
        (min_z..=max_z)
            .all(|cz| (min_x..=max_x).all(|cx| side.world.chunks.contains_key(&(cx, cz))))
    }

    /// Every chunk the search of `radius` around `at` reads is loaded in `side`.
    fn portal_chunks_loaded(side: &PortalSide<'_>, at: BlockPos, radius: i32) -> bool {
        PortalSide::search_chunks(at.x >> 4, at.z >> 4, radius)
            .all(|chunk| side.world.chunks.contains_key(&chunk))
    }

    /// The portal block at `pos` of `level` and what it leads to, found without changing
    /// anything.
    fn probe_portal(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        size: [f64; 2],
    ) -> Result<Probe, PortalError> {
        // The size is validated once, here: a width or height a float cannot hold, or a
        // negative one, would make the placement arithmetic non-finite.
        if !size
            .iter()
            .all(|v| v.is_finite() && (0.0..=f64::from(f32::MAX)).contains(v))
        {
            return Err(PortalError::InvalidSize);
        }
        self.native_level_world_result(level)
            .map_err(Self::portal_error)?;
        let source = self
            .portal_side(Dim::from_id(level.dimension_id()))
            .ok_or(PortalError::LevelMissing)?;
        let [x, y, z] = pos;
        // `largest_rectangle_around` reads up to `RECTANGLE` blocks each way around the entry.
        if !Self::blocks_loaded(&source, x, z, RECTANGLE) {
            return Err(PortalError::ChunksNotLoaded);
        }
        let state = source.world.block(x, y, z);
        let name = &*self.res.facts().get(state).name;
        if self.res.behaviours().portal_axis(state).is_some() {
            let entry = BlockPos::new(x, y, z);
            let at = Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5);
            let plan =
                self.portal_plan_from(&source, entry, at, size[0] as f32, size[1] as f32, true);
            if !self.may_enter(plan.dim) {
                return Err(PortalError::EnteringNotAllowed);
            }
            let target = self
                .portal_side(plan.dim)
                .filter(|target| {
                    Self::portal_chunks_loaded(
                        target,
                        plan.approximate,
                        PortalSide::search_radius(plan.dim == Dim::Nether),
                    )
                })
                .ok_or(PortalError::ChunksNotLoaded)?;
            let found = self.find_portal_in(&target, None, &plan);
            // The exit rectangle and the collision-free placement read around the portal found.
            if let Some((at, _)) = found
                && !Self::blocks_loaded(&target, at.x, at.z, RECTANGLE + COLLISION)
            {
                return Err(PortalError::ChunksNotLoaded);
            }
            return Ok(Probe::Nether(plan, found.map(|(_, rectangle)| rectangle)));
        }
        match name {
            "minecraft:end_portal" => self.probe_end_portal(&source),
            "minecraft:end_gateway" => self.probe_end_gateway(&source, pos),
            _ => Err(PortalError::NotAPortal),
        }
    }

    /// `EndPortalBlock.getPortalDestination` for an entity other than a player, without the
    /// platform: the End's spawn point, or out of the End the world spawn in its level.
    fn probe_end_portal(&self, source: &PortalSide<'_>) -> Result<Probe, PortalError> {
        if source.dim != Dim::End {
            let [x, y, z] = END_SPAWN;
            self.portal_side(Dim::End)
                .ok_or(PortalError::ChunksNotLoaded)?;
            // Facing west, whatever the entity faced.
            let [yaw, pitch] = end_portal_rotation(true, [0.0; 2], self.level.world_spawn_angle);
            return Ok(Probe::Fixed(
                Dim::End,
                Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5),
                PortalRotation::new(PortalAngle::Set(yaw), PortalAngle::Turn(pitch)),
            ));
        }
        let [x, _, z] = self.level.world_spawn.unwrap_or_else(|| {
            self.spawn.map_or([0, 64, 0], |s| {
                [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32]
            })
        });
        let level = self.world_spawn_level();
        let side = self
            .portal_side(level)
            .filter(|side| side.world.is_loaded_at(x, 64, z))
            .ok_or(PortalError::ChunksNotLoaded)?;
        let y = self.motion_top_in(side.world, level, x, z);
        // Turned by the world spawn's angle.
        let [yaw, pitch] = end_portal_rotation(false, [0.0; 2], self.level.world_spawn_angle);
        Ok(Probe::Fixed(
            level,
            Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5),
            PortalRotation::new(PortalAngle::Turn(yaw), PortalAngle::Turn(pitch)),
        ))
    }

    /// `TheEndGatewayBlockEntity.getPortalPosition` of a gateway that already has its exit.
    fn probe_end_gateway(
        &self,
        source: &PortalSide<'_>,
        pos: [i32; 3],
    ) -> Result<Probe, PortalError> {
        let Some(BlockEntity::EndGateway(gateway)) =
            source.world.block_entities.get(&(pos[0], pos[1], pos[2]))
        else {
            return Err(PortalError::NotAPortal);
        };
        let exit = gateway.exit_portal.ok_or(PortalError::NoExit)?;
        let target = if gateway.exact_teleport {
            exit
        } else {
            let (cx, cz) = (exit[0] >> 4, exit[2] >> 4);
            let loaded = (cz - 1..=cz + 1)
                .all(|z| (cx - 1..=cx + 1).all(|x| source.world.chunks.contains_key(&(x, z))));
            if !loaded {
                return Err(PortalError::ChunksNotLoaded);
            }
            let p = self.tallest_block_in(
                source.world,
                [exit[0], exit[1].saturating_add(2), exit[2]],
                5,
                false,
            );
            [p[0], p[1].saturating_add(1), p[2]]
        };
        Ok(Probe::Fixed(
            source.dim,
            sunrise_client::end_gateway::bottom_center(target),
            PortalRotation::new(PortalAngle::Turn(0.0), PortalAngle::Turn(0.0)),
        ))
    }

    /// The answer for the destination `exit` of `plan` in the loaded level of `plan.dim`.
    fn nether_exit(
        &self,
        level: LevelRef,
        plan: &Plan,
        exit: FoundRectangle,
    ) -> Result<PortalExit, PortalError> {
        let side = self.portal_side(plan.dim).ok_or(PortalError::Unavailable)?;
        let (at, yaw_turn) = self.arrival_in(&side, plan, exit);
        let corner = exit.min_corner;
        let x_axis = self
            .res
            .behaviours()
            .portal_axis(side.world.block(corner.x, corner.y, corner.z))
            .unwrap_or(true);
        let shape = PortalShape::from(mod_api::__private::built::PortalShape {
            x_axis,
            bottom_left: [corner.x, corner.y, corner.z],
            width: exit.axis1_size,
            height: exit.axis2_size,
        });
        Ok(PortalExit::new(
            self.level_ref(level.epoch(), plan.dim.key()),
            [at.x, at.y, at.z],
            PortalRotation::new(PortalAngle::Turn(yaw_turn), PortalAngle::Turn(0.0)),
            Some(shape),
        ))
    }

    fn fixed_exit(
        &self,
        level: LevelRef,
        dim: Dim,
        at: Vec3,
        rotation: PortalRotation,
    ) -> PortalExit {
        PortalExit::new(
            self.level_ref(level.epoch(), dim.key()),
            [at.x, at.y, at.z],
            rotation,
            None,
        )
    }

    /// Where an entity of `size` arrives through the portal at `portal_pos`, changing nothing
    /// (`getPortalDestination`). Vanilla places the exit from the two portals' axes
    /// alone.
    pub(super) fn level_portal_destination(
        &self,
        level: LevelRef,
        portal_pos: [i32; 3],
        size: [f64; 2],
    ) -> Result<PortalExit, PortalError> {
        match self.probe_portal(level, portal_pos, size)? {
            Probe::Fixed(dim, at, rotation) => Ok(self.fixed_exit(level, dim, at, rotation)),
            Probe::Nether(plan, Some(exit)) => self.nether_exit(level, &plan, exit),
            Probe::Nether(_, None) => Err(PortalError::NoExit),
        }
    }

    /// [`Self::level_portal_destination`], building the exit portal when none stands
    /// (`PortalForcer.createPortal`) and telling `portal_create` as travel does.
    pub(super) fn level_find_or_create_portal_exit(
        &mut self,
        level: LevelRef,
        portal_pos: [i32; 3],
        size: [f64; 2],
    ) -> Result<PortalExit, PortalError> {
        if !self.transactions.is_empty() {
            return Err(PortalError::InTransaction);
        }
        let plan = match self.probe_portal(level, portal_pos, size)? {
            Probe::Nether(plan, None) => plan,
            _ => return self.level_portal_destination(level, portal_pos, size),
        };
        // Building works on the level in place: the destination goes in place for it, and the
        // level that was in place comes back.
        let source = self.dim;
        self.enter_level(plan.dim)
            .map_err(|_| PortalError::LevelMissing)?;
        let built = self.create_exit_portal(&plan);
        let answer = built.map(|exit| self.nether_exit(level, &plan, exit));
        // The source level was parked by the swap above, so entering it again only swaps back:
        // opening a level is the one way `enter_level` fails.
        let back = self.enter_level(source);
        debug_assert!(back.is_ok(), "the parked source level re-enters");
        answer.unwrap_or(Err(PortalError::NoExit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use sunrise_client::chunk::Chunk;

    const SIZE: [f64; 2] = [0.98, 0.98];
    const ENTRY: [i32; 3] = [8, 70, 8];

    fn nether_portal_blocks(game: &Game<'_>) -> usize {
        let b = game.res.behaviours();
        let side = game.portal_side(Dim::Nether).expect("the Nether is open");
        side.world
            .chunks
            .values()
            .map(|c| c.blocks_where(|s| b.portal_axis(s).is_some()).len())
            .sum()
    }

    fn prepare(game: &mut Game<'_>) {
        game.build_registries();
        game.set_allow_commands(true);
        game.options.dedicated = Some(Vec::new());
        game.native_lifecycle.world_open_ready().unwrap();
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
    }

    /// The chunks of the level in place that every answer reads around the entry, but `skip`.
    fn load_around(game: &mut Game<'_>, skip: Option<(i32, i32)>) {
        for z in -3..=3 {
            for x in -3..=3 {
                if Some((x, z)) != skip && !game.world.chunks.contains_key(&(x, z)) {
                    game.streaming
                        .insert_remote(&mut game.world, Chunk::empty(x, z));
                }
            }
        }
    }

    fn set(game: &mut Game<'_>, name: &str, [x, y, z]: [i32; 3]) {
        let state = game.pack.assets.registry.default_state(name).unwrap();
        game.world.set(x, y, z, state).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_portal_is_asked_where_it_leads_and_find_or_create_builds_its_exit() {
        with_game(|game| {
            prepare(game);
            set(game, "minecraft:nether_portal", ENTRY);
            let overworld = game.level("minecraft:overworld").unwrap();
            let ask = |game: &Game<'_>| game.level_portal_destination(overworld, ENTRY, SIZE);
            load_around(game, None);
            // The Nether is not open.
            assert_eq!(ask(game), Err(PortalError::ChunksNotLoaded));
            // Open with none of the chunks the search reads.
            game.enter_level(Dim::Nether).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(ask(game), Err(PortalError::ChunksNotLoaded));
            // Its chunks loaded, nothing stands there: the read finds none and builds none.
            game.enter_level(Dim::Nether).unwrap();
            load_around(game, None);
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(ask(game), Err(PortalError::NoExit));
            assert_eq!(nether_portal_blocks(game), 0);
            // A block that is no portal; sizes no float placement can use.
            assert_eq!(
                game.level_portal_destination(overworld, [9, 70, 9], SIZE),
                Err(PortalError::NotAPortal)
            );
            for pos in [
                [i32::MAX, 70, 8],
                [i32::MIN, 70, 8],
                [8, 70, i32::MAX],
                [8, 70, i32::MIN],
            ] {
                assert_eq!(
                    game.level_portal_destination(overworld, pos, SIZE),
                    Err(PortalError::ChunksNotLoaded)
                );
            }
            for size in [
                [f64::NAN, 1.0],
                [1.0, f64::INFINITY],
                [-0.5, 1.0],
                [1.0, f64::from(f32::MAX) * 2.0],
            ] {
                assert_eq!(
                    game.level_portal_destination(overworld, ENTRY, size),
                    Err(PortalError::InvalidSize)
                );
            }
            // Not inside a transaction.
            let seal = game.transaction_open();
            assert_eq!(
                game.level_find_or_create_portal_exit(overworld, ENTRY, SIZE),
                Err(PortalError::InTransaction)
            );
            game.transaction_close(&seal, false);
            // Find or create builds one, and the read then finds it.
            let built = game
                .level_find_or_create_portal_exit(overworld, ENTRY, SIZE)
                .expect("the exit is built");
            assert_eq!(game.dim, Dim::Overworld);
            assert!(nether_portal_blocks(game) > 0);
            assert_eq!(built.level, game.level("minecraft:the_nether").unwrap());
            assert!(built.exit_portal.is_some());
            assert_eq!(ask(game), Ok(built));
        });
    }

    /// A two-wide portal across the edge of chunks (0, 0) and (1, 0) is read whole or not at
    /// all; the exit is the middle of the Nether portal's two blocks.
    #[test]
    #[ignore = "daily"]
    fn a_portal_across_a_chunk_edge_waits_for_its_far_chunk() {
        with_game(|game| {
            prepare(game);
            let overworld = game.level("minecraft:overworld").unwrap();
            let entry = [15, 70, 8];
            load_around(game, Some((1, 0)));
            for y in 70..73 {
                set(game, "minecraft:nether_portal", [15, y, 8]);
            }
            game.enter_level(Dim::Nether).unwrap();
            load_around(game, None);
            for x in 0..2 {
                for y in 70..73 {
                    set(game, "minecraft:nether_portal", [x, y, 1]);
                }
            }
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(
                game.level_portal_destination(overworld, entry, SIZE),
                Err(PortalError::ChunksNotLoaded)
            );
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(1, 0));
            for y in 70..73 {
                set(game, "minecraft:nether_portal", [16, y, 8]);
            }
            let exit = game
                .level_portal_destination(overworld, entry, SIZE)
                .expect("the portal is read whole");
            // The entity's 0.98 box centred at x 15.5 in the 2-wide frame starting at x 15 sits
            // 0.01 right of the frame's left end: (0.01 / 1.02) of the free width, which is
            // the middle of the 2-wide exit (0.5); forward 0.5 of the exit's z of 1.
            let [x, y, z] = exit.pos;
            assert!((x - 0.5).abs() < 1e-4 && (y - 70.0).abs() < 1e-4 && (z - 1.5).abs() < 1e-4);
            assert_eq!(
                exit.rotation,
                PortalRotation::new(PortalAngle::Turn(0.0), PortalAngle::Turn(0.0))
            );
            let shape = exit.exit_portal.expect("a nether portal");
            assert_eq!(
                (shape.bottom_left, shape.width, shape.height, shape.x_axis),
                ([0, 70, 1], 2, 3, true)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn end_portals_answer_the_fixed_spawn_with_the_travel_yaw() {
        with_game(|game| {
            prepare(game);
            game.level.world_spawn = Some([4, 64, 4]);
            game.level.world_spawn_angle = [30.0, 12.0];
            load_around(game, None);
            set(game, "minecraft:stone", [4, 63, 4]);
            set(game, "minecraft:end_portal", [8, 70, 8]);
            game.enter_level(Dim::End).unwrap();
            load_around(game, None);
            set(game, "minecraft:end_portal", [0, 60, 0]);
            game.enter_level(Dim::Overworld).unwrap();
            let overworld = game.level("minecraft:overworld").unwrap();
            let end = game.level("minecraft:the_end").unwrap();
            // Into the End: the End's spawn point, facing west.
            let into = game
                .level_portal_destination(overworld, [8, 70, 8], SIZE)
                .unwrap();
            assert_eq!(into.level, end);
            assert_eq!(into.pos, [100.5, 50.0, 0.5]);
            assert_eq!(
                into.rotation,
                PortalRotation::new(PortalAngle::Set(90.0), PortalAngle::Turn(0.0))
            );
            // Out of it: the top of the world spawn's column, turned by its angle.
            let out = game
                .level_portal_destination(end, [0, 60, 0], SIZE)
                .unwrap();
            assert_eq!(out.level, overworld);
            assert_eq!(out.pos, [4.5, 64.0, 4.5]);
            assert_eq!(
                out.rotation,
                PortalRotation::new(PortalAngle::Turn(30.0), PortalAngle::Turn(12.0))
            );
        });
    }
}
