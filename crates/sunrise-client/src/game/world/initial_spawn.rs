//! A new world's spawn and its first player's position: MinecraftServer.
//! setInitialSpawn walks a chunk spiral around NoiseBasedChunkGenerator.
//! getOrigin, then PlayerSpawnFinder.findSpawn picks a free column around
//! the world spawn within the respawn radius.
use super::Game;
use sunrise_client::{
    Result,
    collisions::{SceneCollisions, no_collision_or_liquid},
    random_source,
    world_spawn::{
        PlayerSpawnFinder, SpawnCandidate, SpawnLevel, level_respawn_pos, spawn_pos_in_chunk,
    },
};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
pub(super) use sunrise_gameplay::initial_spawn::{Columns, bottom_center};
use sunrise_world::{
    collision_context::CollisionContext, heightmap::HeightmapType, level::flags::UPDATE_CLIENTS,
    registry::BlockStateId,
};

impl Game<'_> {
    fn spawn_columns(&self) -> Columns<'_, '_> {
        Columns {
            world: &self.world,
            properties: &self.pack.assets.properties,
            collision: self.collision,
            // PlayerSpawnFinder.getLevelRespawnPos: a ceiling starts the walk
            // at the destination generator's spawn height.
            has_ceiling: self.dimension_type().height.has_ceiling,
            spawn_height: self.streaming.spawn_height(),
        }
    }
    /// ChunkGenerator.getOrigin and getSpawnHeight of the overworld.
    pub(super) fn spawn_origin(&self) -> Result<((i32, i32), i32)> {
        if let Some(flat) = self.level.generator.as_ref().and_then(|g| {
            sunrise_client::flatgen::FlatTerrain::from_settings(
                g.settings(),
                self.pack.assets.registry,
                world_biomes!(self),
                self.level_shapes.overworld,
            )
        }) {
            return Ok(((0, 0), flat.spawn_height()));
        }
        if self.options.test_terrain {
            return Ok(((0, 0), 64));
        }
        let origin = match self.end.locator() {
            Some(terrain) => terrain.origin()?,
            None => sunrise_core::position::ChunkPos::new(0, 0),
        };
        Ok(((origin.x, origin.z), 64))
    }
    /// MinecraftServer.createLevels initializes the Overworld after its Load
    /// event and before constructing other levels. The owner's player spawn
    /// is resolved separately, after level preparation.
    pub(super) fn initialize_overworld_spawn(&mut self) -> Result<()> {
        let dedicated = self.is_dedicated_server();
        if self.level.world_spawn.is_none() && self.world_debug {
            // MinecraftServer.setInitialSpawn takes this branch before NeoForge's
            // CreateSpawnPosition and the normal chunk search (including bonus chest).
            self.level.world_spawn = Some([0, 80, 0]);
            self.level.world_spawn_angle = [0.0, 0.0];
            self.level.spawn = Some([0.0, 80.0, 0.0]);
        }
        if self.level.world_spawn.is_none() {
            if dedicated {
                self.streaming.radius = -1;
            }
            self.try_initialize_world_spawn(true)?;
        }
        if dedicated {
            self.spawn = self
                .level
                .world_spawn
                .map(|[x, y, z]| Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5));
        }
        Ok(())
    }

    pub(super) fn try_initialize_world_spawn(&mut self, wait: bool) -> Result<bool> {
        let Some([x, y, z]) = self.find_world_spawn(wait)? else {
            return Ok(false);
        };
        self.spawn = Some(Vec3::new(
            f64::from(x) + 0.5,
            f64::from(y),
            f64::from(z) + 0.5,
        ));
        self.level.spawn = Some([f64::from(x), f64::from(y), f64::from(z)]);
        // A personal spawnpoint must not replace the world's spawn.
        self.level.world_spawn.get_or_insert([x, y, z]);
        if !self.spawn_hook_cancelled
            && sunrise_client::save::WorldGen::read(&self.world_root).is_some_and(|g| g.bonus_chest)
        {
            self.place_bonus_chest([x, y, z])?;
        }
        Ok(true)
    }

    /// MinecraftServer.setInitialSpawn: the first chunk of the spiral with a
    /// valid respawn column gives the world spawn, else the origin chunk's
    /// centre at the spawn height. The server's creation pass loads each tested
    /// column to FULL as `ServerChunkCache.getChunk` does. The nonblocking path
    /// can return None while a chunk to test is loading.
    fn find_world_spawn(&mut self, wait: bool) -> Result<Option<[i32; 3]>> {
        if !self.spawn_hook_fired {
            self.spawn_hook_fired = true;
            let mut position = self.level.world_spawn.unwrap_or([0, 0, 0]);
            let mut rotation = self.level.world_spawn_angle;
            let mut cancel = false;
            self.native_create_spawn_position(&mut position, &mut rotation, &mut cancel);
            if cancel {
                self.spawn_hook_cancelled = true;
                self.level.world_spawn = Some(position);
                self.level.world_spawn_angle = rotation;
                return Ok(Some(position));
            }
        }
        let ((ox, oz), height) = match self.spawn_origin {
            Some(origin) => origin,
            None => *self.spawn_origin.insert(self.spawn_origin()?),
        };
        while let Some((dx, dz)) = self.spawn_spiral.current() {
            if self.spawn_spiral.tested() {
                let (cx, cz) = (ox + dx, oz + dz);
                if wait {
                    if self.load_full_chunk(None, (cx, cz)).is_err() {
                        return Err(format!("could not load initial spawn chunk {cx},{cz}").into());
                    }
                } else if !self.streaming.is_lit(cx, cz) {
                    self.streaming.center = (cx, cz);
                    return Ok(None);
                }
                if let Some(pos) = spawn_pos_in_chunk(&self.spawn_columns(), cx, cz) {
                    return Ok(Some(pos));
                }
            }
            self.spawn_spiral.advance();
        }
        Ok(Some([ox * 16 + 8, height, oz * 16 + 8]))
    }
    /// MinecraftServer.setInitialSpawn's bonus chest: BonusChestFeature.place
    /// in the spawn chunk, a chest with the spawn bonus loot and a torch on
    /// each side that can hold one, on the first free column of a shuffled
    /// walk over the chunk. Skipped while the chunk is not loaded.
    pub(super) fn place_bonus_chest(&mut self, origin: [i32; 3]) -> Result<()> {
        use sunrise_client::containers::BlockEntity;
        let (cx, cz) = (origin[0] >> 4, origin[2] >> 4);
        let registry = &self.pack.assets.registry;
        let (Some(chest), Some(torch)) = (
            registry.default_state("minecraft:chest"),
            registry.default_state("minecraft:torch"),
        ) else {
            return Ok(());
        };
        if !self.streaming.is_lit(cx, cz) {
            return Ok(());
        }
        // `level.getRandom()`. Util.toShuffledList of the chunk's x and z.
        let random = &mut self.world.random;
        let mut shuffled = |min: i32| -> [i32; 16] {
            let mut values: [i32; 16] = std::array::from_fn(|i| min + i as i32);
            for end in (2..=16usize).rev() {
                let other = random.next_int_bound(end as i32).unwrap_or(0) as usize;
                values.swap(end - 1, other);
            }
            values
        };
        let (xs, zs) = (shuffled(cx * 16), shuffled(cz * 16));
        for x in xs {
            for z in zs {
                let y = self
                    .spawn_columns()
                    .height(HeightmapType::MotionBlockingNoLeaves, x, z);
                if self.world.block(x, y, z) != self.world.air && !self.collision_empty([x, y, z]) {
                    continue;
                }
                self.set_block_clients([x, y, z], chest)?;
                let seed = self.world.random.next_long();
                self.world
                    .block_entities
                    .insert((x, y, z), BlockEntity::Chest(Vec::new()));
                self.load_block_entity_nbt(
                    [x, y, z],
                    &format!(
                        "{{LootTable:\"minecraft:chests/spawn_bonus_chest\",LootTableSeed:{seed}L}}"
                    ),
                );
                for [dx, dz] in [[0, -1], [1, 0], [0, 1], [-1, 0]] {
                    let at = [x + dx, y, z + dz];
                    let survives = self.with_level(|b, level, ticks| {
                        b.can_survive(level, ticks, BlockPos::new(at[0], at[1], at[2]), torch)
                    });
                    if survives {
                        self.set_block_clients(at, torch)?;
                    }
                }
                return Ok(());
            }
        }
        Ok(())
    }
    /// `Level.setBlock(pos, state, UPDATE_CLIENTS)`: no neighbour updates.
    fn set_block_clients(&mut self, pos: [i32; 3], state: BlockStateId) -> Result<()> {
        self.with_level(|b, level, ticks| {
            b.set_block(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                state,
                UPDATE_CLIENTS,
            );
        });
        self.process_level_events()
    }
    /// The block at `pos` has no collision shape.
    pub(super) fn collision_empty(&self, pos: [i32; 3]) -> bool {
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        self.collision
            .collision_shape_with_context(
                &mut scene,
                self.world.block(pos[0], pos[1], pos[2]),
                BlockPos::new(pos[0], pos[1], pos[2]),
                &CollisionContext::empty(false),
            )
            .is_ok_and(|shape| shape.is_empty())
    }
    /// PlayerSpawnFinder.findSpawn around the world spawn, as far as the loaded
    /// chunks allow: the feet of the first free column within the respawn
    /// radius, else the suggestion's own column. The finder waits in `state`
    /// (None to begin); Err gives the chunk of the candidate being waited for,
    /// which the caller must have loaded (the finder tries at most 1024
    /// columns).
    pub(super) fn search_spawn(
        &self,
        suggestion: [i32; 3],
        state: &mut Option<PlayerSpawnFinder>,
    ) -> std::result::Result<Vec3, (i32, i32)> {
        let mut finder = state
            .take()
            .unwrap_or_else(|| self.spawn_finder(suggestion));
        let suggestion = finder.suggestion;
        loop {
            let Some(candidate) = finder.next_candidate() else {
                return Ok(self.fixup_spawn_height(suggestion));
            };
            let (x, z) = match candidate {
                SpawnCandidate::Column(x, z) => (x, z),
                SpawnCandidate::Suggestion => (suggestion[0], suggestion[2]),
            };
            if !self.streaming.is_lit(x >> 4, z >> 4) {
                finder.retry();
                *state = Some(finder);
                return Err((x >> 4, z >> 4));
            }
            match candidate {
                SpawnCandidate::Column(x, z) => {
                    if let Some(pos) = level_respawn_pos(&self.spawn_columns(), x, z)
                        && self.player_fits(pos)
                    {
                        return Ok(bottom_center(pos));
                    }
                }
                SpawnCandidate::Suggestion => return Ok(self.fixup_spawn_height(suggestion)),
            }
        }
    }
    /// PlayerSpawnFinder.findSpawn around the world spawn: the feet of a
    /// player who has no position yet. None while a candidate's chunk loads.
    pub(super) fn find_player_spawn(&mut self, suggestion: [i32; 3]) -> Result<Option<Vec3>> {
        let mut state = self.player_spawn.take();
        match self.search_spawn(suggestion, &mut state) {
            Ok(feet) => Ok(Some(feet)),
            Err(chunk) => {
                self.streaming.center = chunk;
                self.player_spawn = state;
                Ok(None)
            }
        }
    }
    /// A new PlayerSpawnFinder around `suggestion`: the respawn radius, cut
    /// by the world border; the suggestion itself in adventure mode.
    fn spawn_finder(&self, suggestion: [i32; 3]) -> PlayerSpawnFinder {
        if self.level.game_mode == 2 {
            let mut finder = PlayerSpawnFinder::new(suggestion, 0, |_| 0);
            finder.next_candidate();
            return finder;
        }
        let mut radius = self
            .ui
            .rules
            .int_at(sunrise_client::gamerules::vanilla::RESPAWN_RADIUS)
            .max(0);
        let to_border = self
            .border
            .distance(f64::from(suggestion[0]), f64::from(suggestion[2]))
            .floor() as i32;
        if to_border < radius {
            radius = to_border;
        }
        if to_border <= 1 {
            radius = 1;
        }
        PlayerSpawnFinder::new(suggestion, radius, |n| {
            random_source::entity().next_int_bound(n).unwrap_or(0)
        })
    }
    /// PlayerSpawnFinder.noCollisionNoLiquid: a player's box standing at the
    /// block's bottom centre touches no block or liquid.
    fn player_fits(&self, pos: [i32; 3]) -> bool {
        let feet = bottom_center(pos);
        let bounds = Aabb::new(
            Vec3::new(feet.x - 0.3, feet.y, feet.z - 0.3),
            Vec3::new(feet.x + 0.3, feet.y + 1.8, feet.z + 0.3),
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        no_collision_or_liquid(&mut scene, bounds)
    }
    /// PlayerSpawnFinder.fixupSpawnHeight: up out of anything solid, then
    /// down onto the ground.
    fn fixup_spawn_height(&self, pos: [i32; 3]) -> Vec3 {
        let [x, mut y, z] = pos;
        while !self.player_fits([x, y, z]) && y < self.world.top_y() - 1 {
            y += 1;
        }
        y -= 1;
        while self.player_fits([x, y, z]) && y > self.world.bottom_y() {
            y -= 1;
        }
        bottom_center([x, y + 1, z])
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
    use crate::game::world_loading::WorldRequest;
    use mod_api::{ChunkLoadEvent, Hooks, LevelLoadEvent, ModEntry, ServerHost};
    use std::sync::{Arc, Mutex};
    use sunrise_client::{
        save::WorldPreset,
        title::{FlatSettings, WorldOptions},
    };

    #[test]
    #[ignore = "daily"]
    fn integrated_creation_finishes_world_spawn_before_opening_the_nether() {
        if run_fresh_cases(
            "game::initial_spawn::tests::integrated_creation_finishes_world_spawn_before_opening_the_nether",
            "SUNRISE_INITIAL_WORLD_SPAWN_CASE",
            &["flat"],
        ) {
            return;
        }

        struct Log(Arc<Mutex<Vec<&'static str>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Log {
            fn level_load(&self, _: &mut dyn ServerHost, event: &mut LevelLoadEvent<'_>) {
                match event.dimension {
                    "minecraft:overworld" => self.0.lock().unwrap().push("overworld"),
                    "minecraft:the_nether" => self.0.lock().unwrap().push("nether"),
                    _ => {}
                }
            }
            fn chunk_load(&self, _: &mut dyn ServerHost, event: &mut ChunkLoadEvent<'_>) {
                if event.dimension == "minecraft:overworld" && event.chunk == [0, 0] {
                    self.0.lock().unwrap().push("spawn column");
                }
            }
        }
        let log = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new("spawn_order", Box::new(Log(log.clone())))].into(),
            || {
                with_game(|game| {
                    let root = tempfile::tempdir().unwrap();
                    game.join_world_rest().unwrap();
                    let options = WorldOptions {
                        world_type: WorldPreset::Flat,
                        flat_settings: Some(FlatSettings {
                            structure_overrides: Vec::new(),
                            ..FlatSettings::default()
                        }),
                        structures: false,
                        ..WorldOptions::default()
                    };
                    let mut request = WorldRequest::new(
                        "spawn_order",
                        Some("Flat spawn"),
                        Some(1),
                        false,
                        Some(&options),
                        game.level.rule_registry.clone(),
                    )
                    .unwrap();
                    request.save_root = root.path().to_path_buf();
                    request.stem_order = Some(&[
                        "minecraft:overworld",
                        "minecraft:the_nether",
                        "minecraft:the_end",
                    ]);
                    let prepared = game.prepare_world_and_rules(request).unwrap();
                    assert!(prepared.level.world_spawn.is_none());
                    game.native_lifecycle_open_started();
                    game.finish_open_world(prepared).unwrap();
                    // The Flat preset's column has four solid blocks from -64. The spawn
                    // search takes the first usable column in the origin chunk.
                    assert_eq!(game.level.world_spawn, Some([0, -60, 0]));
                    assert!(
                        game.player.is_none(),
                        "the owner's player is prepared separately"
                    );
                })
            },
        );
        assert_eq!(
            *log.lock().unwrap(),
            ["overworld", "spawn column", "nether"]
        );
    }
}
