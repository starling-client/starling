//! A server's chunk columns (`ClientboundLevelChunkWithLightPacket`): the
//! sections and light of each packet become a client column on a decoder
//! thread as soon as the packet arrives, so the frame that handles it in
//! order only inserts the result (`ClientPacketListener.handleLevelChunkWithLight`).
use super::Game;
use std::sync::Arc;
use sunrise_client::{chunk::Chunk, streaming::Streaming};
use sunrise_mod_runtime::attachments::Holder;
use sunrise_protocol::packets::play;

pub(in crate::game) use sunrise_client_gameplay::remote::chunks::{
    ChunkDecoder, ChunkFormat, Next, section_blocks,
};

impl Game<'_> {
    /// A chunk packet's column was adopted, with its block entities in the
    /// level: the client chunk seam (milestone 8) tells `client_chunk_load`.
    pub(in crate::game) fn client_chunk_load(&mut self, x: i32, z: i32) {
        if !mod_api::__private::listened(mod_api::HookId::ClientChunkLoad) {
            return;
        }
        if let Some(dimension) = self.client_chunk_dimension() {
            self.native_client_chunk_load(&dimension, [x, z]);
        }
    }

    /// A resident column was freed, whichever route freed it — the last
    /// event for it, its data still there. Tells `client_chunk_unload` and,
    /// when the level stays (`level_stays`), `on_chunk_unloaded`: a level that
    /// goes takes its block entities with it. Its passages go with it.
    pub(in crate::game) fn client_chunk_unload(&mut self, x: i32, z: i32, level_stays: bool) {
        self.forget_chunk_passages(x, z);
        if mod_api::__private::listened(mod_api::HookId::ClientChunkUnload)
            && let Some(dimension) = self.client_chunk_dimension()
        {
            self.native_client_chunk_unload(&dimension, [x, z]);
        }
        if level_stays && let Some(level) = self.native_client_level() {
            self.notify_chunk_block_entities_unloaded(level, (x, z));
        }
    }

    /// The client level's dimension, as the server named it, for the client
    /// chunk hooks: none off a server (an integrated world's chunks are the
    /// server's `chunk_load` and `chunk_unload`).
    fn client_chunk_dimension(&self) -> Option<String> {
        self.remote.as_ref()?.heard_level.clone()
    }
}

impl Game<'_> {
    /// `handleLevelChunkWithLight`: the column replaces any loaded one, and
    /// its block entities load their update tags. `decoded` is the column the
    /// decoder made with `format`; another format since means decode again.
    pub(super) fn remote_chunk(
        &mut self,
        p: &play::LevelChunkWithLight,
        decoded: Option<(&Arc<ChunkFormat>, Result<Chunk, String>)>,
    ) {
        // ClientChunkCache.replaceWithPacketData: a column outside the
        // storage around the view centre is ignored.
        if !self.remote_chunk_in_range(p.x, p.z) {
            log::warn!(
                "Ignoring chunk since it's not in the view range: {}, {}",
                p.x,
                p.z
            );
            return;
        }
        let Some(format) = self.remote.as_ref().map(|r| r.chunk_format.clone()) else {
            return;
        };
        let column = match decoded {
            Some((used, column)) if Arc::ptr_eq(used, &format) => column,
            _ => format.column(p),
        };
        let chunk = match column {
            Ok(chunk) => chunk,
            Err(e) => {
                log::warn!("Sunrise: chunk {} {}: {e}", p.x, p.z);
                return;
            }
        };
        let (x, z) = (p.x, p.z);
        self.supersede_group_predictions(&chunk);
        // A column sent again while it is resident (a bulk edit): only what changed is lit and meshed again.
        let resident = self.world.chunks.contains_key(&(x, z));
        if resident {
            sunrise_client::spans::time("replace_remote", || {
                self.streaming.replace_remote(&mut self.world, chunk)
            });
        } else {
            self.streaming.insert_remote(&mut self.world, chunk);
        }
        sunrise_client::spans::time("unload_block_entities", || {
            self.world.block_entities.unload_column((x, z));
        });
        self.load_block_entities((x, z), &p.chunk.block_entities);
        // After the column's block entities load: an attachment keeps its block entity if the
        // packet sent it again, and one left from a forgotten column goes.
        if let Some(r) = self.remote.as_mut() {
            r.attachments.retain(|holder, _| match *holder {
                Holder::BlockEntity([bx, by, bz]) => {
                    (bx >> 4, bz >> 4) != (x, z)
                        || self.world.block_entities.contains_key(&(bx, by, bz))
                }
                _ => true,
            });
        }
        // Asked once the packet's block entities and attachments are in, so a part reading them
        // sees this column's data.
        self.ask_chunk_light_emission((x, z));
        // The client chunk seam: one load per packet adoption, a re-sent position included,
        // once the column and its block entities are in. A biome packet takes neither path.
        self.client_chunk_load(x, z);
    }

    /// `LevelChunk.replaceWithPacketData`'s block entities of column `(x, z)`, as its chunk
    /// packet lists them: each made for its block, then loading its update tag.
    pub(in crate::game) fn load_block_entities(
        &mut self,
        (x, z): (i32, i32),
        infos: &[sunrise_protocol::chunk::BlockEntityInfo],
    ) {
        let _load = if infos.is_empty() {
            None
        } else {
            sunrise_client::spans::raw_flight_guard("load_block_entities")
        };
        for info in infos {
            let pos = (
                x * 16 + i32::from(info.packed_xz as u8 >> 4),
                i32::from(info.y),
                z * 16 + i32::from(info.packed_xz as u8 & 15),
            );
            let state = self.world.block(pos.0, pos.1, pos.2);
            self.sync_block_entity(pos, None, state);
            // LevelChunk.replaceWithPacketData: the block entity loads its update tag.
            self.remote_block_entity_data(
                [pos.0, pos.1, pos.2],
                info.kind,
                mod_api::BlockEntityDataSource::Chunk,
                &info.tag,
            );
            self.bind_remote_attachment(Holder::BlockEntity([pos.0, pos.1, pos.2]));
        }
    }

    /// `ClientPacketListener.handleChunksBiomes`: each loaded column takes
    /// its new biomes (`ClientChunkCache.replaceBiomes`), then every section
    /// of it and of its neighbours is drawn again, as their colours blend
    /// across columns (`LevelExtractor.setSectionDirty`).
    pub(super) fn remote_chunks_biomes(&mut self, p: &play::ChunksBiomes) {
        let Some(format) = self.remote.as_ref().map(|r| r.chunk_format.clone()) else {
            return;
        };
        for data in &p.chunks {
            let [x, z] = data.pos;
            if !self.remote_chunk_in_range(x, z) {
                log::warn!("Ignoring chunk since it's not in the view range: {x}, {z}");
                continue;
            }
            let Some(chunk) = self.world.chunks.get_mut(&(x, z)) else {
                log::warn!("Ignoring chunk since it's not present: {x}, {z}");
                continue;
            };
            if let Err(e) = format.replace_biomes(Arc::make_mut(chunk), data) {
                log::warn!("Sunrise: biomes of chunk {x} {z}: {e}");
            }
        }
        let mask = sunrise_client::section_mask::SectionMask::below(self.world.layout().sections());
        for data in &p.chunks {
            let [x, z] = data.pos;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let n = (x + dx, z + dz);
                    if self.world.chunks.contains_key(&n) {
                        self.world.dirty_sections.insert_column_mask(n, mask);
                    }
                }
            }
        }
    }

    /// `handleLightUpdatePacket`: the sections it names take its light.
    pub(super) fn remote_light_update(&mut self, p: &play::LightUpdate) {
        let Some(format) = self.remote.as_ref().map(|r| r.chunk_format.clone()) else {
            return;
        };
        if !self.world.chunks.contains_key(&(p.x, p.z)) {
            return;
        }
        let layers = format.light(&p.light);
        let Some(chunk) = self.world.chunks.get_mut(&(p.x, p.z)) else {
            return;
        };
        let changed = sunrise_client::light::update_light_layers(
            Arc::make_mut(chunk),
            &layers,
            format.has_skylight,
        );
        Streaming::light_updated(&mut self.world, p.x, p.z, &changed);
        self.ask_updated_light_emission((p.x, p.z), &changed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_protocol::chunk::{ChunkData, Paletted, Section as Wire, Strategy as Wiring};
    use sunrise_protocol::{
        chunk::LightData, packets::play::Clientbound as C, registry::DimensionType,
    };

    /// A column of `state` with biome 1 everywhere, unlit.
    fn packet(x: i32, state: i32) -> Box<C> {
        let section = Wire {
            non_empty: 4096,
            fluids: 0,
            states: Paletted::from_values(&[state; 4096], Wiring::Blocks, 16),
            biomes: Paletted::from_values(&[1; 64], Wiring::Biomes, 2),
        };
        let none = LightData {
            sky_mask: Vec::new(),
            block_mask: Vec::new(),
            empty_sky_mask: Vec::new(),
            empty_block_mask: Vec::new(),
            sky: Vec::new(),
            block: Vec::new(),
        };
        Box::new(C::LevelChunkWithLight(play::LevelChunkWithLight {
            x,
            z: -3,
            chunk: ChunkData {
                heightmaps: Vec::new(),
                buffer: ChunkData::from_sections(&vec![section; 24]),
                block_entities: Vec::new(),
            },
            light: none,
        }))
    }

    /// Columns come back one per packet, in the order the packets went in,
    /// as the frame would have decoded them itself.
    #[test]
    fn the_decoder_returns_columns_in_order() {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let format = Arc::new(ChunkFormat::new(&overworld, vec![0, 7, 9], 2));
        let decoder = ChunkDecoder::new();
        for x in 0..20 {
            assert!(decoder.submit(format.clone(), packet(x, 1 + x)).is_none());
        }
        for x in 0..20 {
            let (packet, column) = loop {
                match decoder.next() {
                    Next::Ready(decoded) => break decoded,
                    Next::Decoding => std::thread::yield_now(),
                    Next::Lost => panic!("the decoder stopped"),
                }
            };
            let C::LevelChunkWithLight(p) = *packet else {
                panic!("not a chunk packet");
            };
            let (column, direct) = (column.unwrap(), format.column(&p).unwrap());
            assert_eq!((column.x, column.z), (x, -3));
            for (cx, y, cz) in [(0, -64, 0), (5, 70, 9), (15, 319, 15)] {
                assert_eq!(column.get(cx, y, cz), 1 + x as u32);
                assert_eq!(column.get(cx, y, cz), direct.get(cx, y, cz));
                // Wire biome 1 is our biome 7.
                assert_eq!(column.biome(cx, y, cz), 7);
            }
        }
        assert!(matches!(decoder.next(), Next::Decoding));
    }

    /// Large bursts of chunk packets (e.g. 128 columns arriving together)
    /// decode concurrently and retire in strict sequential FIFO order.
    #[test]
    fn the_decoder_handles_large_bursts_in_strict_fifo() {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let format = Arc::new(ChunkFormat::new(&overworld, vec![0, 7, 9], 2));
        let decoder = ChunkDecoder::new();
        for x in 0..128 {
            assert!(decoder.submit(format.clone(), packet(x, 1 + x)).is_none());
        }
        for x in 0..128 {
            let (packet, column) = loop {
                match decoder.next() {
                    Next::Ready(decoded) => break decoded,
                    Next::Decoding => std::thread::yield_now(),
                    Next::Lost => panic!("the decoder stopped"),
                }
            };
            let C::LevelChunkWithLight(p) = *packet else {
                panic!("not a chunk packet");
            };
            assert_eq!(p.x, x);
            let column = column.unwrap();
            assert_eq!((column.x, column.z), (x, -3));
        }
        assert!(matches!(decoder.next(), Next::Decoding));
    }

    /// A tick about to run waits for each column still decoding instead of
    /// leaving it for the next frame: every packet comes back in order, and
    /// with none left the wait ends as `Decoding`.
    #[test]
    fn a_tick_waits_for_each_column_in_order() {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let format = Arc::new(ChunkFormat::new(&overworld, vec![0, 7, 9], 2));
        let decoder = ChunkDecoder::new();
        for x in 0..32 {
            assert!(decoder.submit(format.clone(), packet(x, 1 + x)).is_none());
        }
        for x in 0..32 {
            let Next::Ready((packet, column)) =
                decoder.wait_next(std::time::Duration::from_secs(10))
            else {
                panic!("column {x} did not come");
            };
            let C::LevelChunkWithLight(p) = *packet else {
                panic!("not a chunk packet");
            };
            assert_eq!(p.x, x);
            assert_eq!(column.unwrap().get(5, 70, 9), 1 + x as u32);
        }
        assert!(matches!(
            decoder.wait_next(std::time::Duration::from_millis(1)),
            Next::Decoding
        ));
    }

    /// `LevelChunk.replaceBiomes`: the packet's containers go to the
    /// dimension's sections (y 0 to 255 in the Nether), mapped to our biome
    /// table; the blocks stay, and the sections outside the dimension take
    /// its nearest biomes as a chunk packet's do.
    #[test]
    fn chunks_biomes_replace_a_columns_biomes() {
        let nether = DimensionType::vanilla("minecraft:the_nether");
        let format = ChunkFormat::new(&nether, vec![0, 7, 9], 2);
        let mut chunk = Chunk::empty(2, -3);
        chunk.set(3, 70, 4, 5);
        // Wire biome 1 everywhere, with 2 in the lowest and the highest cells.
        let containers: Vec<Paletted> = (0..nether.sections())
            .map(|i| {
                let mut cells = [1; 64];
                if i == 0 {
                    cells[0] = 2;
                }
                if i == nether.sections() - 1 {
                    cells[63] = 2;
                }
                Paletted::from_values(&cells, Wiring::Biomes, 2)
            })
            .collect();
        let data = play::ChunkBiomeData::from_biomes([2, -3], &containers);
        format.replace_biomes(&mut chunk, &data).unwrap();
        assert_eq!(chunk.get(3, 70, 4), 5);
        assert_eq!(chunk.biome(8, 70, 8), 7);
        assert_eq!(chunk.biome(0, 0, 0), 9);
        assert_eq!(chunk.biome(4, 0, 0), 7);
        assert_eq!(chunk.biome(15, 255, 15), 9);
        // Below and above the Nether: its bottom and top cell layers.
        assert_eq!(chunk.biome(0, -64, 0), 9);
        assert_eq!(chunk.biome(15, 319, 15), 9);
        assert_eq!(chunk.biome(0, 319, 0), 7);
        // A buffer short of the dimension's sections changes nothing.
        let short = play::ChunkBiomeData::from_biomes([2, -3], &containers[..3]);
        let mut other = Chunk::empty(2, -3);
        assert!(format.replace_biomes(&mut other, &short).is_err());
        assert_eq!(other.biome(8, 70, 8), 0);
    }
}
