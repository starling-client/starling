//! Immutable, copy-on-write chunk column snapshots (`ChunkRef::snapshot`, #5233).
//!
//! Each `Section` in `sunrise_client::chunk::Chunk` stores blocks, 3D quart biomes, and sky/block
//! light together behind one `Arc<Section>`, and every live mutation goes through `Arc::make_mut`.
//! Capturing a `ChunkSnapshot` therefore clones only the `Vec<Arc<Section>>` pointers plus shared
//! read-only tables (`Index`, `Biomes`, `Properties`) with zero array copies.
use super::Game;
use super::api_values::block_state::Index;
use mod_api::{BlockState, ChunkRef, ChunkSnapshot, Heightmap};
use std::sync::Arc;
use sunrise_client::assets::Properties;
use sunrise_client::biome::Biomes;
use sunrise_client::chunk::{Chunk, LevelShape, Light, Section};
use sunrise_world::heightmap::{HeightmapState, HeightmapType};
use sunrise_world::registry::BlockStateId;
use sunrise_world::worldgen::terrain::TerrainBlockStates;

pub(in crate::game) struct EngineChunkSnapshot {
    pos: [i32; 2],
    min_y: i32,
    sections: Arc<[Arc<Section>]>,
    states: Arc<Index>,
    biomes: Arc<Biomes>,
    properties: Properties,
    has_light: bool,
}

impl EngineChunkSnapshot {
    /// Wrap a chunk's copy-on-write `Arc<Section>` list into an immutable [`ChunkSnapshot`],
    /// clipped to the level's actual `LevelShape` build height rather than the widened internal
    /// `ColumnLayout`.
    pub(in crate::game) fn from_chunk(
        chunk: &Chunk,
        shape: LevelShape,
        states: Arc<Index>,
        biomes: Arc<Biomes>,
        properties: Properties,
        has_light: bool,
    ) -> ChunkSnapshot {
        let start = ((shape.min_y - chunk.min_y()) >> 4).max(0) as usize;
        let end = start
            .saturating_add(shape.section_count())
            .min(chunk.sections.len());
        let sections: Arc<[Arc<Section>]> = if start <= end {
            chunk.sections[start..end].iter().cloned().collect()
        } else {
            Arc::from([])
        };
        ChunkSnapshot::from_backend(Arc::new(Self {
            pos: [chunk.x, chunk.z],
            min_y: shape.min_y,
            sections,
            states,
            biomes,
            properties,
            has_light,
        }))
    }

    #[inline]
    fn section_and_local_y(&self, y: i32) -> Option<(&Section, usize)> {
        let rel = y.checked_sub(self.min_y)?;
        if rel < 0 {
            return None;
        }
        let si = (rel >> 4) as usize;
        let section = self.sections.get(si)?;
        Some((section, (rel & 15) as usize))
    }

    #[inline]
    fn heightmap_type(kind: Heightmap) -> Option<HeightmapType> {
        match kind {
            Heightmap::WorldSurface => Some(HeightmapType::WorldSurface),
            Heightmap::MotionBlocking => Some(HeightmapType::MotionBlocking),
            Heightmap::MotionBlockingNoLeaves => Some(HeightmapType::MotionBlockingNoLeaves),
            Heightmap::OceanFloor => Some(HeightmapType::OceanFloor),
            _ => None,
        }
    }
}

impl mod_api::__private::ChunkSnapshotBackend for EngineChunkSnapshot {
    #[inline]
    fn pos(&self) -> [i32; 2] {
        self.pos
    }

    #[inline]
    fn min_y(&self) -> i32 {
        self.min_y
    }

    #[inline]
    fn height(&self) -> i32 {
        (self.sections.len() as i32) * 16
    }

    #[inline]
    fn section_count(&self) -> usize {
        self.sections.len()
    }

    #[inline]
    fn has_light(&self) -> bool {
        self.has_light
    }

    #[inline]
    fn heightmap(&self, kind: Heightmap, lx: u8, lz: u8) -> Option<i32> {
        if lx >= 16 || lz >= 16 {
            return None;
        }
        let htype = Self::heightmap_type(kind)?;
        let xz = (usize::from(lz) << 4) | usize::from(lx);
        for (si, section) in self.sections.iter().enumerate().rev() {
            if !section
                .blocks
                .maybe_has(|id| htype.is_opaque(self.properties.heightmap_state(BlockStateId(id))))
            {
                continue;
            }
            let base_y = self.min_y + (si as i32) * 16;
            for ly in (0..16usize).rev() {
                let id = section.blocks.get((ly << 8) | xz);
                if htype.is_opaque(self.properties.heightmap_state(BlockStateId(id))) {
                    return Some(base_y + (ly as i32) + 1);
                }
            }
        }
        Some(self.min_y)
    }

    #[inline]
    fn block_state(&self, lx: u8, y: i32, lz: u8) -> Option<BlockState> {
        if lx >= 16 || lz >= 16 {
            return None;
        }
        let (section, ly) = self.section_and_local_y(y)?;
        let id = section
            .blocks
            .get((ly << 8) | (usize::from(lz) << 4) | usize::from(lx));
        self.states.to_api(BlockStateId(id)).ok()
    }

    #[inline]
    fn light(&self, lx: u8, y: i32, lz: u8) -> Option<(u8, u8)> {
        if lx >= 16 || lz >= 16 {
            return None;
        }
        let (section, ly) = self.section_and_local_y(y)?;
        let packed = section
            .light
            .get((ly << 8) | (usize::from(lz) << 4) | usize::from(lx));
        Some((packed >> 4, packed & 0x0F))
    }

    #[inline]
    fn biome(&self, lx: u8, y: i32, lz: u8) -> Option<u32> {
        if lx >= 16 || lz >= 16 {
            return None;
        }
        let (section, ly) = self.section_and_local_y(y)?;
        let qx = usize::from(lx >> 2);
        let qy = ly >> 2;
        let qz = usize::from(lz >> 2);
        Some(section.biomes.get((qy << 4) | (qz << 2) | qx))
    }

    #[inline]
    fn biome_key(&self, biome_index: u32) -> Option<&mod_api::BiomeKey> {
        self.biomes
            .registered_keys()
            .get(usize::try_from(biome_index).ok()?)
    }

    #[inline]
    fn section_y(&self, section_index: usize) -> i32 {
        (self.min_y >> 4) + (section_index as i32)
    }

    #[inline]
    fn section_is_empty(&self, section_index: usize) -> bool {
        let Some(section) = self.sections.get(section_index) else {
            return true;
        };
        if !section
            .blocks
            .maybe_has(|id| !self.properties.heightmap_state(BlockStateId(id)).is_air())
        {
            return true;
        }
        if let Some(id) = section.blocks.uniform() {
            return self.properties.heightmap_state(BlockStateId(id)).is_air();
        }
        (0..4096).all(|i| {
            self.properties
                .heightmap_state(BlockStateId(section.blocks.get(i)))
                .is_air()
        })
    }

    #[inline]
    fn section_uniform_block(&self, section_index: usize) -> Option<BlockState> {
        let id = self.sections.get(section_index)?.blocks.uniform()?;
        self.states.to_api(BlockStateId(id)).ok()
    }

    #[inline]
    fn section_uniform_light(&self, section_index: usize) -> Option<(u8, u8)> {
        match self.sections.get(section_index)?.light {
            Light::Uniform(packed) => Some((packed >> 4, packed & 0x0F)),
            _ => None,
        }
    }

    #[inline]
    fn section_uniform_biome(&self, section_index: usize) -> Option<u32> {
        self.sections.get(section_index)?.biomes.uniform()
    }

    #[inline]
    fn section_block_state(
        &self,
        section_index: usize,
        lx: u8,
        ly: u8,
        lz: u8,
    ) -> Option<BlockState> {
        if lx >= 16 || ly >= 16 || lz >= 16 {
            return None;
        }
        let section = self.sections.get(section_index)?;
        let id = section
            .blocks
            .get((usize::from(ly) << 8) | (usize::from(lz) << 4) | usize::from(lx));
        self.states.to_api(BlockStateId(id)).ok()
    }

    #[inline]
    fn section_light(&self, section_index: usize, lx: u8, ly: u8, lz: u8) -> Option<(u8, u8)> {
        if lx >= 16 || ly >= 16 || lz >= 16 {
            return None;
        }
        let section = self.sections.get(section_index)?;
        let packed = section
            .light
            .get((usize::from(ly) << 8) | (usize::from(lz) << 4) | usize::from(lx));
        Some((packed >> 4, packed & 0x0F))
    }

    #[inline]
    fn section_biome_quart(&self, section_index: usize, qx: u8, qy: u8, qz: u8) -> Option<u32> {
        if qx >= 4 || qy >= 4 || qz >= 4 {
            return None;
        }
        let section = self.sections.get(section_index)?;
        Some(
            section
                .biomes
                .get((usize::from(qy) << 4) | (usize::from(qz) << 2) | usize::from(qx)),
        )
    }
}

impl Game<'_> {
    /// Capture an immutable, copy-on-write snapshot of a resident chunk on either side (server
    /// active/parked levels, or a logged-in client's loaded level).
    pub(super) fn chunk_snapshot(&self, chunk: ChunkRef) -> Option<ChunkSnapshot> {
        let [x, z] = chunk.pos();
        let world = self.native_level_view_world(chunk.level())?;
        let column = world.chunks.get(&(x, z))?;
        let states = super::api_values::block_state::current().ok()?;
        let biomes = world_biomes!(self).clone();
        let properties = self.pack.assets.properties.clone();
        let has_light = column.lit || column.light_loaded;
        Some(EngineChunkSnapshot::from_chunk(
            column,
            world.shape,
            states,
            biomes,
            properties,
            has_light,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{Authority, ChunkRef, DimensionKey, Heightmap, LevelRef};
    use std::sync::Arc;
    use sunrise_client::chunk::{Chunk, Light};

    #[test]
    #[ignore = "daily"]
    fn chunk_snapshot_shares_cow_sections_and_isolates_blocks_biomes_and_light() {
        with_game(|game| {
            let stone_key = mod_api::__private::block_key("minecraft:stone").unwrap();
            let dirt_key = mod_api::__private::block_key("minecraft:dirt").unwrap();
            let leaves_key = mod_api::__private::block_key("minecraft:oak_leaves").unwrap();
            let water_key = mod_api::__private::block_key("minecraft:water").unwrap();
            let stone_id = super::super::api_values::default_state(stone_key)
                .unwrap()
                .0;
            let dirt_id = super::super::api_values::default_state(dirt_key).unwrap().0;
            let leaves_id = super::super::api_values::default_state(leaves_key)
                .unwrap()
                .0;
            let water_id = super::super::api_values::default_state(water_key)
                .unwrap()
                .0;

            let biomes = world_biomes!(game).clone();
            let plains_idx = biomes.try_id("minecraft:plains").unwrap();
            let desert_idx = biomes.try_id("minecraft:desert").unwrap();
            assert_ne!(plains_idx, desert_idx);

            let mut column = Chunk::empty(2, -3);
            column.lit = true;
            // Section index 4 -> block Y 0..16 (min_y = -64).
            {
                let sec = Arc::make_mut(&mut column.sections[4]);
                sec.set(0, stone_id); // (0, 0, 0)
                sec.set((3 << 8) | (1 << 4) | 2, water_id); // (2, 3, 1)
                sec.set((5 << 8) | (1 << 4) | 2, leaves_id); // (2, 5, 1)
                sec.set((1 << 8) | (1 << 4) | 3, stone_id); // (3, 1, 1)
                sec.set((3 << 8) | (1 << 4) | 3, water_id); // (3, 3, 1)
                let mut light_arr = Box::new([0xF0u8; 4096]);
                light_arr[0] = (12 << 4) | 7;
                sec.light = Light::packed(light_arr);
            }
            // Section index 5 -> block Y 16..32: stale palette entry (set then cleared back to air).
            {
                let sec = Arc::make_mut(&mut column.sections[5]);
                sec.set(10, dirt_id);
                sec.set(10, 0);
            }
            column.set_biome_cell(0, 16, 0, desert_idx); // quart (0, 0, 0) in section 4

            game.world.chunks.insert((2, -3), Arc::new(column));
            let level = game.level("minecraft:overworld").unwrap();
            let chunk_ref = game.chunk(level, [2, -3]).unwrap();

            // Record live section Arc pointer before capture to prove zero-copy COW sharing.
            let pre_sec4_ptr = Arc::as_ptr(&game.world.chunks[&(2, -3)].sections[4]);
            let snap = chunk_ref.snapshot(game).expect("snapshot of loaded chunk");
            assert_eq!(snap.pos(), [2, -3]);
            assert_eq!(snap.min_y(), -64);
            assert_eq!(snap.height(), 384);
            assert_eq!(snap.section_count(), 24);
            assert!(snap.has_light());
            // Strong count of section 4 rose from 1 (live chunk) to 2 (live chunk + snapshot).
            assert_eq!(
                Arc::strong_count(&game.world.chunks[&(2, -3)].sections[4]),
                2
            );

            // Mutate live chunk blocks, biomes, AND light after snapshot: Arc::make_mut splits
            // only section 4, leaving the snapshot completely unchanged.
            {
                let live = Arc::make_mut(game.world.chunks.get_mut(&(2, -3)).unwrap());
                {
                    let sec = Arc::make_mut(&mut live.sections[4]);
                    sec.set(0, dirt_id);
                    sec.light = Light::Uniform(0x00);
                }
                live.set_biome_cell(0, 16, 0, plains_idx);
            }
            let post_sec4_ptr = Arc::as_ptr(&game.world.chunks[&(2, -3)].sections[4]);
            assert_ne!(
                pre_sec4_ptr, post_sec4_ptr,
                "live edit to a shared section must copy-on-write"
            );

            // Move snapshot to a background worker thread and verify all reads off-thread.
            let worker_snap = snap.clone();
            let worker = std::thread::spawn(move || {
                assert_eq!(worker_snap.block_state(0, 0, 0).unwrap().block(), stone_key);
                assert_eq!(worker_snap.light(0, 0, 0), Some((12, 7)));

                let desert = mod_api::BiomeKey::parse("minecraft:desert").unwrap();
                assert_eq!(worker_snap.biome(3, 3, 3), Some(&desert));

                let sec4 = worker_snap.section(4).unwrap();
                assert_eq!(sec4.section_y(), 0);
                assert!(!sec4.is_empty());
                assert_eq!(sec4.uniform_block(), None);
                assert_eq!(sec4.uniform_light(), None);
                assert_eq!(sec4.block_state(0, 0, 0).unwrap().block(), stone_key);
                assert_eq!(sec4.light(0, 0, 0), Some((12, 7)));
                assert_eq!(sec4.biome_quart(0, 0, 0), Some(&desert));
                assert_eq!(sec4.biome_quart(4, 0, 0), None);
                assert_eq!(sec4.block_state(16, 0, 0), None);
                assert_eq!(sec4.light(0, 16, 0), None);

                // Section 5 had a stale palette entry cleared back to air: is_empty() must be true.
                let sec5 = worker_snap.section(5).unwrap();
                assert!(sec5.is_empty());
                assert_eq!(sec5.uniform_light(), Some((15, 0)));

                // Section 0 is untouched uniform air.
                let sec0 = worker_snap.section(0).unwrap();
                assert!(sec0.is_empty());
                assert!(sec0.uniform_block().is_some());
                assert!(sec0.uniform_biome().is_some());

                // Heightmaps at (2, 1): water at y=3, oak_leaves at y=5 (leaves block motion so
                // OceanFloor matches leaves; MotionBlockingNoLeaves ignores leaves and stops at water).
                assert_eq!(
                    worker_snap.heightmap(Heightmap::WorldSurface, 2, 1),
                    Some(6)
                );
                assert_eq!(
                    worker_snap.heightmap(Heightmap::MotionBlocking, 2, 1),
                    Some(6)
                );
                assert_eq!(
                    worker_snap.heightmap(Heightmap::MotionBlockingNoLeaves, 2, 1),
                    Some(4)
                );
                assert_eq!(worker_snap.heightmap(Heightmap::OceanFloor, 2, 1), Some(6));
                // Heightmaps at (3, 1): stone at y=1 under water at y=3 (OceanFloor skips fluid).
                assert_eq!(
                    worker_snap.heightmap(Heightmap::WorldSurface, 3, 1),
                    Some(4)
                );
                assert_eq!(
                    worker_snap.heightmap(Heightmap::MotionBlocking, 3, 1),
                    Some(4)
                );
                assert_eq!(
                    worker_snap.heightmap(Heightmap::MotionBlockingNoLeaves, 3, 1),
                    Some(4)
                );
                assert_eq!(worker_snap.heightmap(Heightmap::OceanFloor, 3, 1), Some(2));
                assert_eq!(worker_snap.heightmap(Heightmap::OceanFloor, 0, 0), Some(1));

                // Out-of-bounds checks never panic.
                assert!(worker_snap.section(24).is_none());
                assert_eq!(worker_snap.block_state(16, 0, 0), None);
                assert_eq!(worker_snap.block_state(0, -65, 0), None);
                assert_eq!(worker_snap.block_state(0, 320, 0), None);
                assert_eq!(worker_snap.light(0, 320, 0), None);
                assert_eq!(worker_snap.biome(0, -65, 0), None);
                assert_eq!(worker_snap.heightmap(Heightmap::WorldSurface, 16, 0), None);
            });
            worker.join().expect("worker thread reads cleanly");

            // Non-Overworld LevelShape (Nether 0..256, 16 sections) clips padded ColumnLayout
            // sections so min_y, height, section_count, heightmap, and bounds match level.height.
            let nether_shape = sunrise_client::chunk::LevelShape::NETHER;
            let mut nether_col = Chunk::with_layout(3, 4, nether_shape.column());
            nether_col.lit = true;
            // Put stone in padded below-0 section (y = -16) and inside Nether (y = 32).
            let padded_sec = ((0 - nether_col.min_y()) >> 4) as usize - 1;
            let in_range_sec = ((32 - nether_col.min_y()) >> 4) as usize;
            Arc::make_mut(&mut nether_col.sections[padded_sec]).set(0, stone_id);
            Arc::make_mut(&mut nether_col.sections[in_range_sec]).set(0, stone_id);
            let nether_snap = super::EngineChunkSnapshot::from_chunk(
                &nether_col,
                nether_shape,
                super::super::api_values::block_state::current().unwrap(),
                biomes.clone(),
                game.pack.assets.properties.clone(),
                true,
            );
            assert_eq!(nether_snap.min_y(), 0);
            assert_eq!(nether_snap.height(), 256);
            assert_eq!(nether_snap.section_count(), 16);
            assert_eq!(nether_snap.section(0).unwrap().section_y(), 0);
            assert_eq!(nether_snap.section(15).unwrap().section_y(), 15);
            assert!(nether_snap.section(16).is_none());
            assert_eq!(nether_snap.block_state(0, -16, 0), None);
            assert_eq!(nether_snap.block_state(0, 256, 0), None);
            assert_eq!(
                nether_snap.block_state(0, 32, 0).unwrap().block(),
                stone_key
            );
            assert_eq!(
                nether_snap.heightmap(Heightmap::WorldSurface, 0, 0),
                Some(33)
            );
            assert_eq!(
                nether_snap.heightmap(Heightmap::WorldSurface, 1, 0),
                Some(0)
            );

            // Unloaded chunk & stale level handles return None.
            let unloaded = level.chunk_ref(game, [99, 99]).unwrap();
            assert!(unloaded.snapshot(game).is_none());
            game.native_lifecycle
                .begin_open(Some(Authority::IntegratedServer));
            assert!(chunk_ref.snapshot(game).is_none());
            let stale = ChunkRef::new(LevelRef::new(level.epoch(), DimensionKey::NETHER), [2, -3]);
            assert!(stale.snapshot(game).is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn chunk_snapshot_works_on_client_world() {
        use crate::game::native_entity_refs::tests::with_client_game;
        with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "snapshot-client").unwrap();
            game.remote.as_mut().unwrap().logged_in = true;

            let stone_key = mod_api::__private::block_key("minecraft:stone").unwrap();
            let stone_id = super::super::api_values::default_state(stone_key)
                .unwrap()
                .0;
            let mut column = Chunk::empty(1, 1);
            column.lit = true;
            Arc::make_mut(&mut column.sections[4]).set(0, stone_id);
            game.world.chunks.insert((1, 1), Arc::new(column));

            let level = game.level("minecraft:overworld").unwrap();
            let chunk = game
                .chunk(level, [1, 1])
                .expect("client host.chunk resolves loaded chunk");
            let snap = chunk
                .snapshot(game)
                .expect("client ChunkRef::snapshot works");
            assert_eq!(snap.block_state(0, 0, 0).unwrap().block(), stone_key);
            assert!(snap.has_light());
            // Server-only ChunkRef status/loaded checks still refuse client side.
            assert!(!chunk.loaded(game));
            assert_eq!(chunk.status(game), None);
        });
    }
}
