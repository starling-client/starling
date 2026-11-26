//! Streaming column height, block-slice and terrain-surface queries on open levels.
use super::Game;
use super::block_edits::{column_span, fill_column_blocks};
use std::ops::ControlFlow;

impl Game<'_> {
    /// Visit each column's height by `heightmap` in an active or parked open level (server) or
    /// the client's open level, `None` where its chunk is not loaded. Loads nothing.
    pub(super) fn level_visit_surface_heights(
        &self,
        level: mod_api::LevelRef,
        columns: &[(i32, i32)],
        heightmap: mod_api::Heightmap,
        visit: &mut dyn FnMut((i32, i32), Option<i32>) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, mod_api::LevelReadError> {
        let world = self.native_level_read_world_result(level)?;
        let started = std::time::Instant::now();
        let mut last: Option<((i32, i32), bool)> = None;
        let mut flow = ControlFlow::Continue(());
        let mut visited = 0usize;
        for &(x, z) in columns {
            let key = (x >> 4, z >> 4);
            let loaded = match last {
                Some((k, l)) if k == key => l,
                _ => {
                    let l = world.chunks.contains_key(&key);
                    last = Some((key, l));
                    l
                }
            };
            let height = loaded.then(|| self.world_column_height(world, heightmap, x, z));
            visited += 1;
            if visit((x, z), height).is_break() {
                flow = ControlFlow::Break(());
                break;
            }
        }
        self.native_read_logged(started, "height query", visited);
        Ok(flow)
    }

    /// Visit each column's height on `heightmap`, server-side biome JSON water colour and
    /// complete block states from `below` under to `above - 1` over that height, bottom first
    /// (air outside the chunk's vertical range; `None` where the column's chunk is not loaded).
    /// Works on active and parked server levels with no column-count or total-block cap.
    pub(super) fn level_visit_columns(
        &self,
        level: mod_api::LevelRef,
        columns: &[(i32, i32)],
        heightmap: mod_api::Heightmap,
        range: (u32, u32),
        visit: &mut dyn for<'a> FnMut(
            (i32, i32),
            Option<mod_api::ColumnSlice<'a>>,
        ) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, mod_api::LevelReadError> {
        let world = self.native_level_world_result(level)?;
        let (below, above) = range;
        let per = column_span(below, above, world.shape.height)
            .ok_or(mod_api::LevelReadError::Unavailable)?;
        let index = super::native_worldgen::prepare(self.pack.assets.registry)
            .map_err(|_| mod_api::LevelReadError::Unavailable)?;
        let started = std::time::Instant::now();
        let biomes = world_biomes!(self);
        let mut raw = Vec::with_capacity(per);
        let mut states = Vec::with_capacity(per);
        let mut flow = ControlFlow::Continue(());
        let mut visited = 0usize;
        for &(x, z) in columns {
            let slice = if let Some(chunk) = world.chunks.get(&(x >> 4, z >> 4)) {
                let base = self.world_column_height(world, heightmap, x, z);
                raw.clear();
                fill_column_blocks(
                    world.air,
                    chunk,
                    world.shape,
                    x,
                    z,
                    base,
                    below,
                    above,
                    &mut raw,
                );
                states.clear();
                for &state in &raw {
                    let api_state = index
                        .to_api(state)
                        .map_err(|_| mod_api::LevelReadError::Unavailable)?;
                    states.push(api_state);
                }
                let water_color = biomes.get(chunk.biome(x, base - 1, z)).water;
                Some(mod_api::ColumnSlice::new(base, &states, water_color))
            } else {
                None
            };
            visited += 1;
            if visit((x, z), slice).is_break() {
                flow = ControlFlow::Break(());
                break;
            }
        }
        self.native_read_logged(started, "column read", visited);
        Ok(flow)
    }
    /// A read's time logged with `SUNRISE_SPIKES`, as an edit's is.
    fn native_read_logged(&self, started: std::time::Instant, what: &str, columns: usize) {
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        if self.options.spikes.is_some_and(|threshold| ms > threshold) {
            log::info!("Sunrise: slow mod {what} {ms:.1} ms ({columns} columns)");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ops::ControlFlow;

    #[test]
    #[ignore = "daily"]
    fn read_columns_returns_server_water_colors_without_colormaps() {
        use std::path::Path;
        use std::sync::Arc;
        use sunrise_assets::resource_source::ResourceRoot;
        use sunrise_client::{biome::Biomes, chunk::Chunk};

        fn link_tree(src: &Path, dst: &Path) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(src, dst).unwrap();
            #[cfg(not(unix))]
            {
                if src.is_dir() {
                    std::fs::create_dir(dst).unwrap();
                    for entry in std::fs::read_dir(src).unwrap() {
                        let entry = entry.unwrap();
                        link_tree(&entry.path(), &dst.join(entry.file_name()));
                    }
                } else if std::fs::hard_link(src, dst).is_err() {
                    std::fs::copy(src, dst).unwrap();
                }
            }
        }

        fn mirror_omitting(src: &Path, dst: &Path, omit: &[&[&str]]) {
            for entry in std::fs::read_dir(src).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name();
                let Some(name_str) = name.to_str() else {
                    link_tree(&entry.path(), &dst.join(&name));
                    continue;
                };
                let under: Vec<&[&str]> = omit
                    .iter()
                    .filter_map(|path| {
                        path.split_first()
                            .filter(|(head, _)| **head == name_str)
                            .map(|(_, tail)| tail)
                    })
                    .collect();
                if under.iter().any(|rest| rest.is_empty()) {
                    continue;
                }
                let target = dst.join(&name);
                if under.is_empty() {
                    link_tree(&entry.path(), &target);
                } else {
                    std::fs::create_dir(&target).unwrap();
                    mirror_omitting(&entry.path(), &target, &under);
                }
            }
        }

        const OMIT: &[&[&str]] = &[
            &[sunrise_core::prepared_files::FILE_NAME],
            &["assets", "minecraft", "textures", "colormap", "grass.png"],
            &["assets", "minecraft", "textures", "colormap", "foliage.png"],
            &[
                "assets",
                "minecraft",
                "textures",
                "colormap",
                "dry_foliage.png",
            ],
        ];
        let scratch = tempfile::tempdir().unwrap();
        let resources = scratch.path().join("resources");
        std::fs::create_dir(&resources).unwrap();
        mirror_omitting(
            &super::super::native_entity_refs::tests::fixture_root(),
            &resources,
            OMIT,
        );
        for name in ["grass.png", "foliage.png", "dry_foliage.png"] {
            assert!(
                !resources
                    .join("assets/minecraft/textures/colormap")
                    .join(name)
                    .exists(),
                "private fixture must omit {name}"
            );
        }
        assert!(
            Biomes::load(&ResourceRoot::directory(resources.clone())).is_err(),
            "client biome load must fail when colormaps are absent"
        );

        super::super::native_entity_refs::tests::with_game_role(
            resources,
            crate::game::execution_role::ExecutionRole::DedicatedServer,
            |game| {
                game.options.dedicated = Some(Vec::new());
                assert!(game.world_biomes.is_none());
                let plains = world_biomes!(game).id("minecraft:plains");
                let swamp = world_biomes!(game).id("minecraft:swamp");
                let mut chunk = Chunk::empty(0, 0);
                for qy in 0..chunk.sections.len() as i32 * 4 {
                    for qz in 0..4 {
                        chunk.set_biome_cell(0, qy, qz, plains);
                        chunk.set_biome_cell(1, qy, qz, swamp);
                    }
                }
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.world.set(4, 64, 0, stone).unwrap();

                let overworld = game.level(game.dim.id()).unwrap();
                let mut bases = Vec::new();
                let mut water_colors = Vec::new();
                let mut stone_seen = Vec::new();
                let flow = overworld
                    .visit_columns(
                        game,
                        &[(0, 0), (4, 0), (32, 0)],
                        mod_api::Heightmap::WorldSurface,
                        (1, 1),
                        &mut |_, slice| {
                            match slice {
                                Some(slice) => {
                                    bases.push(Some(slice.base));
                                    water_colors.push(slice.water_color);
                                    stone_seen.push(slice.blocks[0].block());
                                }
                                None => {
                                    bases.push(None);
                                    water_colors.push(0);
                                }
                            }
                            ControlFlow::Continue(())
                        },
                    )
                    .unwrap();
                assert_eq!(flow, ControlFlow::Continue(()));
                assert_eq!(bases, vec![Some(65), Some(65), None]);
                assert_eq!(water_colors, vec![0x3F_76E4, 0x61_7B64, 0]);
                assert_eq!(
                    stone_seen,
                    vec![
                        mod_api::__private::block_key("minecraft:stone").unwrap(),
                        mod_api::__private::block_key("minecraft:stone").unwrap(),
                    ]
                );

                // Both height and column streaming work on parked open levels too.
                let here = game.dim;
                game.enter_level(crate::game::Dim::Nether).unwrap();
                let mut parked_heights = Vec::new();
                assert_eq!(
                    overworld.visit_surface_heights(
                        game,
                        &[(0, 0), (4, 0), (32, 0)],
                        mod_api::Heightmap::WorldSurface,
                        &mut |pos, height| {
                            parked_heights.push((pos, height));
                            ControlFlow::Continue(())
                        },
                    ),
                    Ok(ControlFlow::Continue(()))
                );
                assert_eq!(
                    parked_heights,
                    vec![((0, 0), Some(65)), ((4, 0), Some(65)), ((32, 0), None)]
                );
                let mut parked_bases = Vec::new();
                assert_eq!(
                    overworld.visit_columns(
                        game,
                        &[(0, 0), (32, 0)],
                        mod_api::Heightmap::WorldSurface,
                        (1, 1),
                        &mut |_, slice| {
                            parked_bases.push(slice.map(|s| (s.base, s.water_color)));
                            ControlFlow::Continue(())
                        },
                    ),
                    Ok(ControlFlow::Continue(()))
                );
                assert_eq!(parked_bases, vec![Some((65, 0x3F_76E4)), None]);
                game.enter_level(here).unwrap();
            },
        );
    }
}
