//! A server's chunk columns decoded off the frame thread: the format a packet decodes with and the in-order decoder.
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};
use sunrise_client::{
    chunk::{
        BIOME_STRATEGY, Chunk, ColumnLayout, Light, Section, block_strategy, known_blocks_only,
    },
    net::Event,
};
use sunrise_core::paletted::{PalettedContainer, Strategy};
use sunrise_protocol::{
    chunk::{LightData, Paletted, SectionLight},
    packets::play::{self, Clientbound as C},
    registry::DimensionType,
};

/// What turning a chunk packet into a client column depends on: the
/// dimension's height and sky, and the server's biome registry mapped to
/// ours. The connection replaces it when either changes, so a column decoded
/// with an older one is decoded again.
#[derive(Default)]
pub struct ChunkFormat {
    /// The column decoded chunks hold: the client's for the level.
    column: ColumnLayout,
    min_y: i32,
    sections: usize,
    pub has_skylight: bool,
    biome_map: Vec<u16>,
    biome_bits: u8,
}

impl ChunkFormat {
    pub fn new(dimension: &DimensionType, biome_map: Vec<u16>, biome_bits: u8) -> Self {
        Self {
            column: ColumnLayout::VANILLA,
            min_y: dimension.min_y,
            sections: dimension.sections(),
            has_skylight: dimension.has_skylight,
            biome_map,
            biome_bits,
        }
    }
    /// Decodes into chunks of `column`.
    pub fn in_column(self, column: ColumnLayout) -> Self {
        Self { column, ..self }
    }
    /// The column decoded chunks hold.
    pub fn layout(&self) -> ColumnLayout {
        self.column
    }
    pub fn with_dimension(&self, dimension: &DimensionType) -> Self {
        Self {
            column: self.column,
            ..Self::new(dimension, self.biome_map.clone(), self.biome_bits)
        }
    }

    /// LevelChunkSection data into the client's column.
    pub fn column(&self, p: &play::LevelChunkWithLight) -> Result<Chunk, String> {
        let sections = p
            .chunk
            .sections(self.sections, 16, self.biome_bits)
            .map_err(|e| e.to_string())?;
        let mut chunk = Chunk::with_layout(p.x, p.z, self.column);
        let offset = (self.min_y - self.column.min_y()).div_euclid(16);
        let column = self.column.sections() as i32;
        for (i, s) in sections.iter().enumerate() {
            let target = offset + i as i32;
            if !(0..column).contains(&target) {
                continue;
            }
            // A global section is as wide as the server's registry: 16 bits
            // from a vanilla server, wider from a Sunrise host with the same
            // content mods. Read it at that width, then widen it to ours.
            let ours = block_strategy();
            let strategy = if ours.wire_has_palette(s.states.bits) {
                ours
            } else {
                Strategy::blocks(s.states.bits)
            };
            let palette = s.states.palette.iter().map(|&p| p as u32).collect();
            let words = s.states.data.iter().map(|&w| w as u64).collect();
            let blocks = known_blocks_only(
                PalettedContainer::from_wire(strategy, s.states.bits, palette, words)
                    .map_err(|e| e.to_string())?,
            );
            let mut section = Section {
                blocks,
                biomes: PalettedContainer::new(BIOME_STRATEGY, 0),
                light: Light::Uniform(0xF0),
            };
            section.compact();
            chunk.sections[target as usize] = Arc::new(section);
            // `LevelChunkSection.read` publishes this count, and `hasOnlyAir` reads it.
            if s.non_empty != 0 {
                chunk.add_light_support(target as usize);
            }
        }
        let biomes: Vec<_> = sections.iter().map(|s| &s.biomes).collect();
        self.put_biomes(&mut chunk, &biomes);
        // ClientPacketListener.applyLightData: the column keeps the light
        // the server sent; nothing is relit.
        let layers = self.light(&p.light);
        sunrise_client::light::light_from_layers(&mut chunk, &layers, self.has_skylight);
        // From the single-player server thread: its own sections where they
        // are the same.
        sunrise_client::lan::handoff::take(&mut chunk);
        Ok(chunk)
    }

    /// The dimension's sections' biome containers (in the server's
    /// registry) into the column's sections. Sections outside the
    /// dimension's range take its nearest biomes (the column's top or bottom
    /// cell layer repeated).
    fn put_biomes(&self, chunk: &mut Chunk, biomes: &[&Paletted]) {
        if biomes.is_empty() {
            return;
        }
        let offset = (self.min_y - self.column.min_y()).div_euclid(16);
        // Biome table index per quart cell of the dimension's own sections.
        let biome_cells: Vec<[u32; 64]> = biomes
            .iter()
            .map(|b| {
                std::array::from_fn(|j| {
                    u32::from(self.biome_map.get(b.get(j) as usize).copied().unwrap_or(0))
                })
            })
            .collect();
        let last = biome_cells.len() - 1;
        for si in 0..self.column.sections() as i32 {
            let i = si - offset;
            let cells = if (0..biome_cells.len() as i32).contains(&i) {
                biome_cells[i as usize]
            } else {
                let (edge, layer) = if i < 0 { (0, 0) } else { (last, 3) };
                std::array::from_fn(|j| biome_cells[edge][layer * 16 + j % 16])
            };
            let mut container = PalettedContainer::from_values(BIOME_STRATEGY, &cells);
            if let Some(v) = container.uniform() {
                container = PalettedContainer::new(BIOME_STRATEGY, v);
            }
            Arc::make_mut(&mut chunk.sections[si as usize]).biomes = container;
        }
    }

    /// `LevelChunk.replaceBiomes`: each of the dimension's sections reads
    /// its biome container from the packet's buffer, in order.
    pub fn replace_biomes(
        &self,
        chunk: &mut Chunk,
        data: &play::ChunkBiomeData,
    ) -> Result<(), String> {
        let biomes = data
            .biomes(self.sections, self.biome_bits)
            .map_err(|e| e.to_string())?;
        self.put_biomes(chunk, &biomes.iter().collect::<Vec<_>>());
        Ok(())
    }

    /// `ClientPacketListener.readSectionList`: the sky and block layers of a
    /// light packet, by light section of the client's column: the column's
    /// sections with the padding section one below and one above them, so
    /// index `i` is the column's section `i - 1` (`light_from_layers`).
    pub fn light(&self, light: &LightData) -> Vec<SectionLight> {
        let column = self.column.sections() as i32;
        let mut layers: Vec<SectionLight> = vec![(None, None); self.column.sections() + 2];
        // Light section b is the dimension's section b - 1.
        let first = (self.min_y - self.column.min_y()).div_euclid(16) - 1;
        for (b, layer) in light.layers(self.sections + 2).into_iter().enumerate() {
            let si = first + b as i32;
            if (-1..=column).contains(&si) {
                layers[(si + 1) as usize] = layer;
            }
        }
        layers
    }
}

/// `ClientboundSectionBlocksUpdatePacket.runUpdates`: each changed block's position (the
/// packed local position within the section) and its state.
pub fn section_blocks(p: &play::SectionBlocksUpdate) -> impl Iterator<Item = ([i32; 3], i32)> + '_ {
    let [sx, sy, sz] = p.section;
    p.changes.iter().map(move |&change| {
        let local = (change & 0xFFF) as i32;
        let pos = [
            sx * 16 + (local >> 8 & 15),
            sy * 16 + (local & 15),
            sz * 16 + (local >> 4 & 15),
        ];
        (pos, (change >> 12) as i32)
    })
}

pub type Decoded = (Box<C>, Result<Chunk, String>);

struct Job {
    seq: u64,
    format: Arc<ChunkFormat>,
    packet: Box<C>,
}

struct WorkResult {
    seq: u64,
    packet: Box<C>,
    column: Result<Chunk, String>,
}

struct DecoderQueue {
    jobs: VecDeque<Job>,
    stopped: bool,
}

struct Retirement {
    next_submit: u64,
    next_deliver: u64,
    ready: BTreeMap<u64, Decoded>,
    done: mpsc::Receiver<WorkResult>,
}

/// Worker threads that decode chunk packets in parallel while returning
/// results strictly in the order they arrived, one per `submit`.
pub struct ChunkDecoder {
    queue: Arc<(Mutex<DecoderQueue>, Condvar)>,
    retirement: Mutex<Retirement>,
    workers_alive: Arc<AtomicUsize>,
}

impl ChunkDecoder {
    pub fn new() -> Self {
        let (finished, done) = mpsc::channel();
        let queue = Arc::new((
            Mutex::new(DecoderQueue {
                jobs: VecDeque::new(),
                stopped: false,
            }),
            Condvar::new(),
        ));
        let worker_count = sunrise_threads::worker_count().clamp(1, 4);
        let workers_alive = Arc::new(AtomicUsize::new(0));

        let mut spawned = 0;
        for i in 0..worker_count {
            let queue = Arc::clone(&queue);
            let finished = finished.clone();
            let alive = Arc::clone(&workers_alive);
            let builder = std::thread::Builder::new()
                .name(format!("Chunk decoder {i}"))
                .spawn(move || {
                    sunrise_threads::favour_current();
                    'worker: loop {
                        let job = {
                            let mut q = match queue.0.lock() {
                                Ok(q) => q,
                                Err(_) => break 'worker,
                            };
                            loop {
                                if q.stopped && q.jobs.is_empty() {
                                    break 'worker;
                                }
                                if let Some(job) = q.jobs.pop_front() {
                                    break job;
                                }
                                q = match queue.1.wait(q) {
                                    Ok(guard) => guard,
                                    Err(_) => break 'worker,
                                };
                            }
                        };
                        let column =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &*job
                                .packet
                            {
                                C::LevelChunkWithLight(p) => job.format.column(p),
                                _ => Err("not a chunk packet".into()),
                            }))
                            .unwrap_or_else(|_| Err("chunk decoding panicked".into()));
                        if finished
                            .send(WorkResult {
                                seq: job.seq,
                                packet: job.packet,
                                column,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    alive.fetch_sub(1, Ordering::Relaxed);
                });
            if builder.is_ok() {
                spawned += 1;
            } else if let Err(e) = builder {
                log::warn!("Sunrise: no chunk decoder thread ({e}); chunks decode in the frame");
            }
        }
        workers_alive.store(spawned, Ordering::Relaxed);
        Self {
            queue,
            retirement: Mutex::new(Retirement {
                next_submit: 0,
                next_deliver: 0,
                ready: BTreeMap::new(),
                done,
            }),
            workers_alive,
        }
    }

    /// Start decoding a chunk packet with the format it arrived under.
    /// Without a decoder thread the packet comes back undecoded.
    pub fn submit(&self, format: Arc<ChunkFormat>, packet: Box<C>) -> Option<Box<C>> {
        if self.workers_alive.load(Ordering::Relaxed) == 0 {
            return Some(packet);
        }
        let mut retirement = match self.retirement.lock() {
            Ok(r) => r,
            Err(_) => return Some(packet),
        };
        let seq = retirement.next_submit;
        retirement.next_submit += 1;
        let mut q = match self.queue.0.lock() {
            Ok(q) => q,
            Err(_) => {
                retirement.next_submit -= 1;
                return Some(packet);
            }
        };
        if q.stopped {
            retirement.next_submit -= 1;
            return Some(packet);
        }
        q.jobs.push_back(Job {
            seq,
            format,
            packet,
        });
        self.queue.1.notify_one();
        None
    }

    /// The oldest submitted packet and its column, unless it is still
    /// decoding: a frame never waits for the decoder.
    pub fn next(&self) -> Next {
        let mut ret = match self.retirement.lock() {
            Ok(r) => r,
            Err(_) => return Next::Lost,
        };
        while let Ok(res) = ret.done.try_recv() {
            ret.ready.insert(res.seq, (res.packet, res.column));
        }
        let next_deliver = ret.next_deliver;
        if let Some(decoded) = ret.ready.remove(&next_deliver) {
            ret.next_deliver += 1;
            return Next::Ready(decoded);
        }
        if self.workers_alive.load(Ordering::Relaxed) == 0 {
            return Next::Lost;
        }
        Next::Decoding
    }

    /// [`Self::next`], waiting up to `timeout` for the oldest packet's column:
    /// a tick about to run takes every chunk that came before it, as vanilla's
    /// handles every packet before its tick.
    pub fn wait_next(&self, timeout: std::time::Duration) -> Next {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.next() {
                Next::Decoding => {}
                other => return other,
            }
            let mut ret = match self.retirement.lock() {
                Ok(r) => r,
                Err(_) => return Next::Lost,
            };
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match ret.done.recv_timeout(left) {
                Ok(res) => {
                    ret.ready.insert(res.seq, (res.packet, res.column));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => return Next::Decoding,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Next::Lost,
            }
        }
    }
}

impl Default for ChunkDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ChunkDecoder {
    fn drop(&mut self) {
        if let Ok(mut q) = self.queue.0.lock() {
            q.stopped = true;
            self.queue.1.notify_all();
        }
    }
}

pub enum Next {
    Ready(Decoded),
    Decoding,
    Lost,
}

/// What the network thread delivered, in order: an event, or a chunk packet
/// handed to the decoder (with the format it decodes with), whose column
/// comes back from `ChunkDecoder::next`.
pub enum Received {
    Event(Event, sunrise_client::net::ingress::ReceiveLease<Event>),
    Chunk(
        Arc<ChunkFormat>,
        sunrise_client::net::ingress::ReceiveLease<Event>,
    ),
}

// Keep net::Event's transport-owned staging slot bound honest as the actor evolves.
const _: () = assert!(
    std::mem::size_of::<Received>()
        <= std::mem::size_of::<(
            Event,
            sunrise_client::net::ingress::ReceiveLease<Event>,
            usize,
            usize,
        )>()
);

impl Received {
    pub fn into_event_credit(self) -> sunrise_client::net::ingress::ReceiveLease<Event> {
        let Self::Event(_, credit) = self else {
            unreachable!()
        };
        credit
    }
}
