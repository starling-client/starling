//! `SectionOcclusionGraph`: which sections the camera can see through open
//! faces. Breadth-first from the camera section through the faces each
//! section's VisGraph connects, never stepping back towards the camera. As in
//! vanilla's `runUpdates`, a section leads on through whatever any of the faces
//! it was entered through connects to, and beyond 60 blocks a step is taken
//! only if the line from the section towards the camera stays in sections the
//! search has reached.
//!
//! As in vanilla, the full search runs off the render thread: a worker keeps
//! its own copy of every section's connectivity and searches over dense
//! arrays around the camera. Frames use the last finished graph and never wait
//! for the next one. A section meshed in between propagates into the graph the
//! frame has (vanilla's partial update), so what it opens shows at once; what
//! it closes goes at the next full search.
//!
//! GPU culling (gfx/gpu_culling.rs) answers as `contains` does from a copy of
//! the graph (`Reach`): the cells it reached, kept as bits too, sent again
//! whenever the generation moves, which it does whenever an answer may change.
use super::SectionKey;
use super::resident::{block_to_section_y, vertical_section_range};
use crate::chunk_map::{ChunkMap, ChunkMembershipDelta};
use crate::fast_hash::FxHashSet;
use crate::mesher::ALL_VISIBLE;
use crate::streaming::in_view_distance;
use glam::DVec3;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use sunrise_core::mth::floor;

/// Vanilla's `Direction` order: down, up, north, south, west, east.
const STEPS: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];
/// In `Graph::directions`: the cell has a node (`sectionToNodeMap`).
const REACHED: u8 = 1 << 7;
/// In `Graph::directions`: the node was expanded while its column was loaded (`sectionTree`).
const EXPANDED: u8 = 1 << 6;
/// `MINIMUM_ADVANCED_CULLING_SECTION_DISTANCE`: `blockToSectionCoord(60)`.
const DISTANT_SECTIONS: i32 = 60 >> 4;
/// `MINIMUM_ADVANCED_CULLING_DISTANCE` (60 blocks), squared.
const DISTANT_SQR: f64 = 3600.0;
/// `CEILED_SECTION_DIAGONAL`: `ceil(sqrt(3) * 16)`.
const SECTION_DIAGONAL: f64 = 28.0;
/// In `Graph::rays`: the line from this corner stays in reached sections.
const SEES: u8 = u8::MAX;
/// Edits kept before a full resend.
const EDIT_CAP: usize = 1 << 16;

/// VisGraph connectivity and loaded-chunk membership per column.
/// A missing connectivity entry connects every face; an unloaded column blocks expansion.
struct Columns {
    /// The lowest block y of section 0 (`Level.getMinY`).
    min_y: i32,
    sections: i32,
    map: HashMap<(i32, i32), Box<[u64]>>,
    loaded: FxHashSet<(i32, i32)>,
}

impl Columns {
    fn new(min_y: i32, sections: i32) -> Self {
        Self {
            min_y,
            sections,
            map: HashMap::new(),
            loaded: FxHashSet::default(),
        }
    }
    /// `SectionPos.of`: the section holding `pos`.
    fn section_of(&self, pos: DVec3) -> SectionKey {
        (
            floor(pos.x) >> 4,
            block_to_section_y(floor(pos.y), self.min_y),
            floor(pos.z) >> 4,
        )
    }
    fn is_loaded(&self, x: i32, z: i32) -> bool {
        self.loaded.contains(&(x, z))
    }
    /// Whether the section's connectivity changed.
    fn set(&mut self, key: SectionKey, vis: u64) -> bool {
        if !(0..self.sections).contains(&key.1) {
            return false;
        }
        let column = match self.map.get_mut(&(key.0, key.2)) {
            Some(column) => column,
            None if vis == ALL_VISIBLE => return false,
            None => self
                .map
                .entry((key.0, key.2))
                .or_insert_with(|| vec![ALL_VISIBLE; self.sections as usize].into()),
        };
        let slot = &mut column[key.1 as usize];
        if *slot == vis {
            return false;
        }
        *slot = vis;
        if vis == ALL_VISIBLE && column.iter().all(|&v| v == ALL_VISIBLE) {
            self.map.remove(&(key.0, key.2));
        }
        true
    }
    fn get(&self, key: SectionKey) -> u64 {
        self.map
            .get(&(key.0, key.2))
            .and_then(|column| column.get(usize::try_from(key.1).ok()?))
            .copied()
            .unwrap_or(ALL_VISIBLE)
    }
    fn apply(&mut self, edit: Edit) {
        match edit {
            Edit::Set(key, vis) => {
                self.set(key, vis);
            }
            Edit::LoadColumn(x, z) => {
                self.loaded.insert((x, z));
            }
            Edit::UnloadColumn(x, z) => {
                self.loaded.remove(&(x, z));
                self.map.remove(&(x, z));
            }
            Edit::Clear => {
                self.map.clear();
                self.loaded.clear();
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Edit {
    Set(SectionKey, u64),
    LoadColumn(i32, i32),
    UnloadColumn(i32, i32),
    Clear,
}

/// One search's result over the box around the camera: vanilla's
/// `GraphStorage.sectionToNodeMap`, as dense arrays.
#[derive(Default)]
struct Graph {
    from: SectionKey,
    /// Center column of dense storage (initially `(from.0, from.2)`, shifted on partial rebase).
    origin_x: i32,
    origin_z: i32,
    radius: i32,
    /// The world: lowest block y and height in sections.
    min_y: i32,
    sections: i32,
    /// The searched box: lowest section y and its height in sections.
    y0: i32,
    height: i32,
    /// Per cell: `REACHED`, `EXPANDED`, and travel directions (`Node.directions`).
    directions: Vec<u8>,
    /// Per cell: the faces it was entered through, one bit per face
    /// (`Node.sourceDirections`, as the faces rather than the steps); none for
    /// the camera's.
    entered: Vec<u8>,
    /// `EXPANDED` of each cell again, a bit each at `index` (32 a word, lowest
    /// bit first): the GPU's copy (`Occlusion::reach`), written with it
    /// (`set_expanded`).
    bits: Vec<u32>,
    /// Per section corner, during a full search: `SEES`, or how far the line
    /// from it towards the camera is known to stay in reached sections (the
    /// first sample that was not). Sections are only ever added during a
    /// search, so a line that stays in reached sections keeps doing so, and
    /// one that didn't need only be checked again from where it failed.
    rays: Vec<u8>,
    /// Unloaded columns holding reached nodes waiting to expand (`sectionsWaitingForChunkLoads`).
    waiting: HashMap<(i32, i32), Vec<SectionKey>>,
}

impl Graph {
    fn index(&self, key: SectionKey) -> Option<usize> {
        let (dx, dz, dy) = (
            key.0 - self.origin_x,
            key.2 - self.origin_z,
            key.1 - self.y0,
        );
        let side = 2 * self.radius + 1;
        if dx.abs() > self.radius || dz.abs() > self.radius || !(0..self.height).contains(&dy) {
            return None;
        }
        Some((((dz + self.radius) * side + dx + self.radius) * self.height + dy) as usize)
    }
    fn key(&self, index: usize) -> SectionKey {
        let side = 2 * self.radius + 1;
        let i = index as i32;
        let column = i / self.height;
        (
            self.origin_x + column % side - self.radius,
            self.y0 + i % self.height,
            self.origin_z + column / side - self.radius,
        )
    }
    fn contains(&self, key: SectionKey) -> bool {
        self.index(key)
            .is_some_and(|i| self.directions[i] & EXPANDED != 0)
    }
    fn has_node(&self, key: SectionKey) -> bool {
        self.index(key)
            .is_some_and(|i| self.directions[i] & REACHED != 0)
    }
    /// Marks cell `i` expanded into the visible tree and updates `bits`.
    fn set_expanded(&mut self, i: usize) -> bool {
        if self.directions[i] & EXPANDED != 0 {
            return false;
        }
        self.directions[i] |= EXPANDED;
        self.bits[i / 32] |= 1 << (i % 32);
        true
    }
    /// Expands/recenters dense storage to cover `eye`'s view area during partial propagation.
    fn rebase_for_partial(&mut self, eye: SectionKey) -> bool {
        let Some((need_lo, need_hi)) = vertical_section_range(eye.1, self.radius, self.sections)
        else {
            return false;
        };
        let (new_y0, new_height) = if self.height == 0 {
            (need_lo, need_hi - need_lo + 1)
        } else {
            let y0 = self.y0.min(need_lo);
            let y1 = (self.y0 + self.height - 1).max(need_hi);
            (y0, y1 - y0 + 1)
        };
        if eye.0 == self.origin_x
            && eye.2 == self.origin_z
            && new_y0 == self.y0
            && new_height == self.height
        {
            return false;
        }
        let side = (2 * self.radius + 1) as usize;
        let new_size = side * side * new_height as usize;
        let mut directions = vec![0u8; new_size];
        let mut entered = vec![0u8; new_size];
        let mut bits = vec![0u32; new_size.div_ceil(32)];
        let old_side = 2 * self.radius + 1;
        for dz in -self.radius..=self.radius {
            let z = self.origin_z + dz;
            let ndz = z - eye.2;
            if ndz.abs() > self.radius {
                continue;
            }
            for dx in -self.radius..=self.radius {
                let x = self.origin_x + dx;
                let ndx = x - eye.0;
                if ndx.abs() > self.radius {
                    continue;
                }
                let old_base =
                    (((dz + self.radius) * old_side + dx + self.radius) * self.height) as usize;
                let new_base =
                    (((ndz + self.radius) * old_side + ndx + self.radius) * new_height) as usize;
                for dy in 0..self.height {
                    let old_i = old_base + dy as usize;
                    let dir = self.directions[old_i];
                    if dir == 0 {
                        continue;
                    }
                    let y = self.y0 + dy;
                    let new_i = new_base + (y - new_y0) as usize;
                    directions[new_i] = dir;
                    entered[new_i] = self.entered[old_i];
                    if dir & EXPANDED != 0 {
                        bits[new_i / 32] |= 1 << (new_i % 32);
                    }
                }
            }
        }
        self.origin_x = eye.0;
        self.origin_z = eye.2;
        self.y0 = new_y0;
        self.height = new_height;
        self.directions = directions;
        self.entered = entered;
        self.bits = bits;
        let (ox, oz, r, y0, h) = (
            self.origin_x,
            self.origin_z,
            self.radius,
            self.y0,
            self.height,
        );
        self.waiting.retain(|&(cx, cz), keys| {
            if (cx - ox).abs() > r || (cz - oz).abs() > r {
                return false;
            }
            keys.retain(|k| (y0..y0 + h).contains(&k.1));
            !keys.is_empty()
        });
        true
    }
    /// The full search from the camera (vanilla's `scheduleFullUpdate`).
    fn search(
        &mut self,
        columns: &Columns,
        camera: DVec3,
        radius: i32,
        smart_cull: bool,
        vis: &mut Vec<u64>,
        queue: &mut VecDeque<u32>,
    ) {
        let from = columns.section_of(camera);
        self.from = from;
        self.origin_x = from.0;
        self.origin_z = from.2;
        self.radius = radius;
        self.min_y = columns.min_y;
        self.sections = columns.sections;
        self.waiting.clear();
        let Some((lo_y, hi_y)) = vertical_section_range(from.1, radius, columns.sections) else {
            self.y0 = 0;
            self.height = 0;
            self.directions.clear();
            self.bits.clear();
            self.entered.clear();
            self.rays.clear();
            return;
        };
        self.y0 = lo_y;
        self.height = hi_y - lo_y + 1;
        let side = (2 * radius + 1) as usize;
        let size = side * side * self.height as usize;
        self.directions.clear();
        self.directions.resize(size, 0);
        self.bits.clear();
        self.bits.resize(size.div_ceil(32), 0);
        self.entered.clear();
        self.entered.resize(size, 0);
        self.rays.clear();
        self.rays
            .resize((side + 1) * (side + 1) * (self.height + 1) as usize, 0);
        vis.clear();
        vis.resize(size, ALL_VISIBLE);
        let src_start = lo_y as usize;
        let src_end = hi_y as usize + 1;
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let Some(column) = columns.map.get(&(from.0 + dx, from.2 + dz)) else {
                    continue;
                };
                let base = self
                    .index((from.0 + dx, lo_y, from.2 + dz))
                    .expect("column inside the searched box");
                let end = src_end.min(column.len());
                if src_start < end {
                    vis[base..base + (end - src_start)].copy_from_slice(&column[src_start..end]);
                }
            }
        }
        queue.clear();
        if (0..columns.sections).contains(&from.1) {
            let start = self.index(from).expect("camera inside the searched box");
            self.directions[start] = REACHED;
            queue.push_back(start as u32);
        } else {
            self.seed_outside(camera, queue);
        }
        self.propagate(columns, queue, camera, true, smart_cull, |i, _| vis[i]);
    }
    /// `initializeQueueForFullUpdate` when camera is outside build height: seed boundary layer columns in view distance.
    fn seed_outside(&mut self, camera: DVec3, queue: &mut VecDeque<u32>) {
        // STEPS indexes: down, up, north, south, west, east.
        let (y, travel) = if self.from.1 < 0 {
            (0, 1)
        } else {
            (self.sections - 1, 0)
        };
        let block = (i64::from(floor(camera.x)), i64::from(floor(camera.z)));
        let mut seeds = Vec::with_capacity(((2 * self.radius + 1) as usize).pow(2));
        for dx in -self.radius..=self.radius {
            for dz in -self.radius..=self.radius {
                if !in_view_distance(
                    (self.from.0, self.from.2),
                    self.radius,
                    self.from.0 + dx,
                    self.from.2 + dz,
                ) {
                    continue;
                }
                let mut directions = REACHED | 1 << travel;
                directions |= match dx.signum() {
                    1 => 1 << 5,
                    -1 => 1 << 4,
                    _ => 0,
                };
                directions |= match dz.signum() {
                    1 => 1 << 3,
                    -1 => 1 << 2,
                    _ => 0,
                };
                // The y offset is the same for every seed.
                let centre = |section: i32| i64::from(section) * 16 + 8;
                let (ox, oz) = (
                    block.0 - centre(self.from.0 + dx),
                    block.1 - centre(self.from.2 + dz),
                );
                seeds.push((ox * ox + oz * oz, dx, dz, directions));
            }
        }
        seeds.sort_by_key(|seed| seed.0);
        for (_, dx, dz, directions) in seeds {
            let i = self
                .index((self.from.0 + dx, y, self.from.2 + dz))
                .expect("seed inside the searched box");
            self.directions[i] = directions;
            // Entered through the face opposite the travel (`sourceDirection`).
            self.entered[i] = 1 << (travel ^ 1);
            queue.push_back(i as u32);
        }
    }
    /// Vanilla's `runUpdates`: take nodes from the queue, stop at unloaded columns, and step
    /// through open faces (or every face when `!smart_cull`) bounded by the current eye.
    fn propagate(
        &mut self,
        columns: &Columns,
        queue: &mut VecDeque<u32>,
        camera: DVec3,
        full: bool,
        smart_cull: bool,
        vis: impl Fn(usize, SectionKey) -> u64,
    ) -> bool {
        let mut grew = false;
        let abs_eye_y = floor(camera.y) >> 4;
        let eye = (
            floor(camera.x) >> 4,
            abs_eye_y - (self.min_y >> 4),
            floor(camera.z) >> 4,
        );
        // `SectionPos.center` of the camera's section.
        let centre = [(eye.0 << 4) + 8, (abs_eye_y << 4) + 8, (eye.2 << 4) + 8];
        while let Some(index) = queue.pop_front() {
            let i = index as usize;
            let key = self.key(i);
            if !columns.is_loaded(key.0, key.2) {
                let wait = self.waiting.entry((key.0, key.2)).or_default();
                if !wait.contains(&key) {
                    wait.push(key);
                }
                continue;
            }
            if self.set_expanded(i) {
                grew = true;
            }
            let directions = self.directions[i];
            let entered = self.entered[i];
            let exits = if !smart_cull || entered == 0 {
                0x3f
            } else {
                let vis = vis(i, key);
                (0..6)
                    .filter(|face| entered & (1 << face) != 0)
                    .fold(0, |exits, face| exits | (vis >> (face * 6)) as u8 & 0x3f)
            };
            let distant = smart_cull
                && ((key.0 - eye.0).abs() > DISTANT_SECTIONS
                    || (key.1 - eye.1).abs() > DISTANT_SECTIONS
                    || (key.2 - eye.2).abs() > DISTANT_SECTIONS);
            let reuse = full && eye == self.from;
            let mut seen = [None; 3];
            for (d, step) in STEPS.iter().enumerate() {
                if smart_cull && (exits & (1 << d) == 0 || directions & (1 << (d ^ 1)) != 0) {
                    continue;
                }
                let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                if !(0..self.sections).contains(&next.1)
                    || (next.1 - eye.1).abs() > self.radius
                    || !in_view_distance((eye.0, eye.2), self.radius, next.0, next.2)
                {
                    continue;
                }
                let Some(n) = self.index(next) else { continue };
                if distant {
                    let sees = match seen[d / 2] {
                        Some(sees) if reuse => sees,
                        _ => self.sees_camera(key, d, centre, camera, full),
                    };
                    seen[d / 2] = Some(sees);
                    if !sees {
                        continue;
                    }
                }
                if self.directions[n] & REACHED == 0 {
                    self.directions[n] = REACHED | (directions & 0x3f) | (1 << d);
                    self.entered[n] = 1 << (d ^ 1);
                    queue.push_back(n as u32);
                } else {
                    self.entered[n] |= 1 << (d ^ 1);
                }
            }
        }
        grew
    }
    /// Vanilla's distant step test: from the corner of `key` that the step `d`
    /// picks, walk towards the camera in section diagonals while farther than
    /// 60 blocks. Every point inside the world has to fall in a section the
    /// search has reached. Same arithmetic as JOML's `Vector3d` (no FMA).
    /// `full`: remember the answer per corner in `rays`.
    fn sees_camera(
        &mut self,
        key: SectionKey,
        d: usize,
        centre: [i32; 3],
        camera: DVec3,
        full: bool,
    ) -> bool {
        let origin = [key.0 << 4, (key.1 + (self.min_y >> 4)) << 4, key.2 << 4];
        // The step's axis in x, y, z order: down/up, north/south, west/east.
        let axis = [1, 2, 0][d / 2];
        let far = |c: usize| {
            i32::from(if c == axis {
                centre[c] > origin[c]
            } else {
                centre[c] < origin[c]
            })
        };
        let corner = (key.0 + far(0), key.1 + far(1), key.2 + far(2));
        let slot = full.then(|| {
            let side = 2 * self.radius + 2;
            let (x, z) = (
                corner.0 - self.origin_x + self.radius,
                corner.2 - self.origin_z + self.radius,
            );
            ((z * side + x) * (self.height + 1) + corner.1 - self.y0) as usize
        });
        let known = slot.map_or(0, |slot| self.rays[slot]);
        if known == SEES {
            return true;
        }
        let failed = self.walk(corner, camera, known);
        if let Some(slot) = slot {
            self.rays[slot] = failed.unwrap_or(SEES);
        }
        failed.is_none()
    }
    /// The line from the section corner `corner` towards the camera; `None`
    /// if every sample is in a reached section, else the first one that is
    /// not (counted from 1). The first `known - 1` samples are known to be
    /// reached: they are stepped over (the same additions, so the same
    /// points) without looking them up.
    fn walk(&self, corner: SectionKey, camera: DVec3, known: u8) -> Option<u8> {
        let mut check = [
            (i64::from(corner.0) * 16) as f64,
            (i64::from(self.min_y) + i64::from(corner.1) * 16) as f64,
            (i64::from(corner.2) * 16) as f64,
        ];
        let camera = [camera.x, camera.y, camera.z];
        let to = [
            camera[0] - check[0],
            camera[1] - check[1],
            camera[2] - check[2],
        ];
        let inverse = 1.0 / (to[0] * to[0] + (to[1] * to[1] + to[2] * to[2])).sqrt();
        let step = to.map(|t| t * inverse * SECTION_DIAGONAL);
        let (min_y, max_y) = (
            f64::from(self.min_y),
            f64::from(self.min_y + self.sections * 16 - 1),
        );
        let far = |p: &[f64; 3]| {
            let (x, y, z) = (p[0] - camera[0], p[1] - camera[1], p[2] - camera[2]);
            x * x + (y * y + z * z) > DISTANT_SQR
        };
        let mut sample = 0u8;
        while sample + 1 < known {
            for (c, s) in check.iter_mut().zip(step) {
                *c += s;
            }
            sample += 1;
        }
        while far(&check) {
            for (c, s) in check.iter_mut().zip(step) {
                *c += s;
            }
            sample = sample.saturating_add(1);
            if check[1] > max_y || check[1] < min_y {
                break;
            }
            let section = (
                floor(check[0]) >> 4,
                block_to_section_y(floor(check[1]), self.min_y),
                floor(check[2]) >> 4,
            );
            if !self.has_node(section) {
                // A count too long to keep (`SEES`) starts over next time.
                return Some(if sample == SEES { 0 } else { sample });
            }
        }
        None
    }
}

/// The graph `Occlusion::contains` reads, for GPU culling's copy (gfx/gpu_culling.rs).
#[derive(Clone, Copy)]
pub struct Reach<'a> {
    /// `Occlusion::generation` when it was taken.
    pub generation: u64,
    pub from_x: i32,
    pub from_z: i32,
    pub radius: i32,
    pub y0: i32,
    pub height: i32,
    pub bits: &'a [u32],
}

impl Reach<'_> {
    /// gpu_culling.wgsl's `reached`, statement for statement: what `Graph::contains` answers.
    pub fn contains(&self, key: SectionKey) -> bool {
        let r = self.radius;
        let (dx, dz, dy) = (key.0 - self.from_x, key.2 - self.from_z, key.1 - self.y0);
        if dx < -r || dx > r || dz < -r || dz > r || dy < 0 || dy >= self.height {
            return false;
        }
        let i = (((dz + r) * (2 * r + 1) + dx + r) * self.height + dy) as usize;
        (self.bits[i / 32] >> (i % 32)) & 1 != 0
    }
}

struct Job {
    edits: Vec<Edit>,
    camera: DVec3,
    radius: i32,
    smart_cull: bool,
    /// Buffers to reuse.
    graph: Graph,
}

/// The thread that runs full searches over its own copy of the columns.
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<(Graph, Vec<Edit>)>,
}

impl Worker {
    fn spawn(min_y: i32, sections: i32) -> Option<Self> {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("occlusion".into())
            .spawn(move || {
                let mut columns = Columns::new(min_y, sections);
                let (mut vis, mut queue) = (Vec::new(), VecDeque::new());
                while let Ok(mut job) = job_rx.recv() {
                    for edit in job.edits.drain(..) {
                        columns.apply(edit);
                    }
                    job.graph.search(
                        &columns,
                        job.camera,
                        job.radius,
                        job.smart_cull,
                        &mut vis,
                        &mut queue,
                    );
                    if done_tx.send((job.graph, job.edits)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { jobs, done })
    }
}

/// A new value for `Occlusion::generation`, unique in the process: a copy made
/// from an older `Occlusion` (another level's) never matches. As gfx.rs
/// `next_block_entity_revision`.
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Occlusion {
    /// The render thread's copy, for partial updates (and searches when the
    /// worker is gone).
    columns: Columns,
    /// Edits the worker has not been sent yet.
    edits: Vec<Edit>,
    /// A full search is due: sections changed since the last one started.
    dirty: bool,
    /// The last finished search; `None` until one finishes, or after the
    /// columns were cleared.
    graph: Option<Graph>,
    spare: Graph,
    worker: Option<Worker>,
    /// The camera section, radius, and smart_cull flag of the running search.
    running: Option<(SectionKey, i32, bool)>,
    /// The running search started before the columns were cleared: drop it.
    discard: bool,
    /// The camera section, radius, and smart_cull flag `graph`'s search started from.
    searched: Option<(SectionKey, i32, bool)>,
    /// Sections changed since `graph`'s search started, to propagate from.
    changed: Vec<SectionKey>,
    /// Sections changed since the running search started: they propagate into
    /// its graph when it lands.
    changed_late: Vec<SectionKey>,
    /// Columns loaded since `graph`'s search started, to resume waiting nodes.
    loaded_since: Vec<(i32, i32)>,
    /// Columns loaded since the running search started: they resume waiting nodes when it lands.
    loaded_late: Vec<(i32, i32)>,
    /// Last synced `ChunkMap::membership_revision`.
    loaded_revision: u64,
    vis: Vec<u64>,
    queue: VecDeque<u32>,
    /// Changes whenever `contains` may answer differently (`generation`).
    generation: u64,
    /// Set when edits were dropped; the next search resends every column.
    resync: bool,
}

impl Occlusion {
    /// The world's lowest block y and its height in sections.
    pub fn new(min_y: i32, sections: i32) -> Self {
        Self {
            columns: Columns::new(min_y, sections),
            edits: Vec::new(),
            dirty: true,
            graph: None,
            spare: Graph::default(),
            worker: Worker::spawn(min_y, sections),
            running: None,
            discard: false,
            searched: None,
            changed: Vec::new(),
            changed_late: Vec::new(),
            loaded_since: Vec::new(),
            loaded_late: Vec::new(),
            loaded_revision: 0,
            vis: Vec::new(),
            queue: VecDeque::new(),
            generation: next_generation(),
            resync: false,
        }
    }
    fn push_edit(&mut self, edit: Edit) {
        if self.edits.len() >= EDIT_CAP
            || self.changed.len() >= EDIT_CAP
            || self.loaded_since.len() >= EDIT_CAP
        {
            self.resync = true;
            self.dirty = true;
            self.edits.clear();
            self.changed.clear();
            self.changed_late.clear();
            self.loaded_since.clear();
            self.loaded_late.clear();
        }
        if !self.resync {
            self.edits.push(edit);
        }
    }
    /// Marks chunk column `(x, z)` loaded (`SectionOcclusionGraph.loadedChunks`).
    pub fn load_column(&mut self, x: i32, z: i32) {
        if !self.columns.loaded.insert((x, z)) {
            return;
        }
        self.push_edit(Edit::LoadColumn(x, z));
        if !self.resync {
            self.loaded_since.push((x, z));
            if self.running.is_some() {
                self.loaded_late.push((x, z));
            }
        }
    }
    /// Synchronizes loaded column membership with `chunks` using its delta ring.
    pub fn sync_loaded_columns(&mut self, chunks: &ChunkMap) {
        let rev = chunks.membership_revision();
        if rev == self.loaded_revision {
            return;
        }
        if let Some(deltas) = chunks.deltas_since(self.loaded_revision) {
            for (_, delta) in deltas {
                match delta {
                    ChunkMembershipDelta::Added((x, z)) => self.load_column(x, z),
                    ChunkMembershipDelta::Removed((x, z)) => self.remove_column(x, z),
                    ChunkMembershipDelta::Reset => unreachable!(),
                }
            }
        } else {
            let gone: Vec<(i32, i32)> = self
                .columns
                .loaded
                .iter()
                .copied()
                .filter(|col| !chunks.contains_key(col))
                .collect();
            for (x, z) in gone {
                self.remove_column(x, z);
            }
            for &(x, z) in chunks.keys() {
                self.load_column(x, z);
            }
        }
        self.loaded_revision = rev;
    }
    pub fn set(&mut self, key: SectionKey, vis: u64) {
        if self.columns.set(key, vis) {
            self.changed(key, vis);
        }
    }
    fn changed(&mut self, key: SectionKey, vis: u64) {
        self.dirty = true;
        self.push_edit(Edit::Set(key, vis));
        if self.resync {
            return;
        }
        self.changed.push(key);
        if self.running.is_some() {
            self.changed_late.push(key);
        }
    }
    fn resync_edits(&mut self) {
        self.resync = false;
        self.edits.clear();
        self.edits.push(Edit::Clear);
        for &(x, z) in &self.columns.loaded {
            self.edits.push(Edit::LoadColumn(x, z));
        }
        for (&(x, z), column) in &self.columns.map {
            for (y, &vis) in column.iter().enumerate() {
                if vis != ALL_VISIBLE {
                    self.edits.push(Edit::Set((x, y as i32, z), vis));
                }
            }
        }
    }
    /// Marks chunk column `(x, z)` unloaded and clears its section connectivity.
    pub fn remove_column(&mut self, x: i32, z: i32) {
        let was_loaded = self.columns.loaded.remove(&(x, z));
        let had_vis = self.columns.map.remove(&(x, z)).is_some();
        if was_loaded || had_vis {
            self.dirty = true;
            self.push_edit(Edit::UnloadColumn(x, z));
        }
    }
    /// `remove_column` for each of `gone`. When they are every column it holds (a
    /// teleport drops the whole view), the old graph says nothing about what arrives
    /// next: it starts over as `clear` does instead of seeing through each removed
    /// section one edit at a time (some 90 000 at render distance 32).
    pub fn remove_columns(&mut self, gone: &FxHashSet<(i32, i32)>) {
        if self
            .columns
            .loaded
            .iter()
            .all(|column| gone.contains(column))
            && self.columns.map.keys().all(|column| gone.contains(column))
        {
            self.clear();
            return;
        }
        for &(x, z) in gone {
            self.remove_column(x, z);
        }
    }
    pub fn clear(&mut self) {
        self.columns.map.clear();
        self.columns.loaded.clear();
        self.edits.clear();
        self.edits.push(Edit::Clear);
        self.resync = false;
        self.dirty = true;
        // Another world: the old graph says nothing about it. Without one, every
        // section was visible already, and stays so.
        if let Some(graph) = self.graph.take() {
            self.spare = graph;
            self.generation = next_generation();
        }
        self.searched = None;
        self.discard = self.running.is_some();
        self.changed.clear();
        self.changed_late.clear();
        self.loaded_since.clear();
        self.loaded_late.clear();
        self.loaded_revision = 0;
    }
    /// Whether `key` is visible from the camera. Without a usable graph
    /// (before the first search lands, or with the camera somewhere the graph
    /// never reached) every section is.
    pub fn contains(&self, key: SectionKey) -> bool {
        self.graph.as_ref().is_none_or(|g| g.contains(key))
    }
    /// Whether the current graph knows that `key` is visible from
    /// `camera_section`. Unknown is returned when the graph is absent, was
    /// searched from another section, or does not cover the queried section.
    pub fn visible_for_camera(&self, key: SectionKey, camera_section: SectionKey) -> Option<bool> {
        let graph = self.graph.as_ref()?;
        if graph.from != camera_section {
            return None;
        }
        if graph.index(key).is_none()
            || (key.1 - graph.from.1).abs() > graph.radius
            || !in_view_distance((graph.from.0, graph.from.2), graph.radius, key.0, key.2)
        {
            return None;
        }
        Some(graph.contains(key))
    }
    /// Changes whenever `contains` may answer differently: a search landed, a
    /// partial update reached more sections, or the graph went.
    #[cfg(any(test, debug_assertions))]
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// The graph `contains` reads; `None` while every section counts as
    /// visible.
    pub fn reach(&self) -> Option<Reach<'_>> {
        self.graph.as_ref().map(|g| Reach {
            generation: self.generation,
            from_x: g.origin_x,
            from_z: g.origin_z,
            radius: g.radius,
            y0: g.y0,
            height: g.height,
            bits: &g.bits,
        })
    }
    /// The bits `reach` exports are the cells' `EXPANDED`, cell for cell.
    #[cfg(any(test, debug_assertions))]
    pub fn reach_is_exact(&self) -> bool {
        self.graph.as_ref().is_none_or(|g| {
            g.bits.len() == g.directions.len().div_ceil(32)
                && g.directions
                    .iter()
                    .enumerate()
                    .all(|(i, &d)| (d & EXPANDED != 0) == ((g.bits[i / 32] >> (i % 32)) & 1 != 0))
        })
    }
    /// Take a finished search, propagate changed sections and newly loaded columns into the
    /// graph, and start a search when sections changed, smart_cull changed, or the camera moved.
    pub fn update(&mut self, camera: DVec3, radius: i32, smart_cull: bool) {
        let from = self.columns.section_of(camera);
        self.collect(false);
        self.propagate_changes(camera, smart_cull);
        if self.running.is_none()
            && (self.dirty || self.searched != Some((from, radius, smart_cull)))
        {
            self.start(camera, radius, smart_cull);
        }
        // A graph that never reached the camera's section hides everything
        // around it: until the new one lands, cull by the frustum alone.
        if self
            .graph
            .as_ref()
            .is_some_and(|g| g.from != from && !g.contains(from))
            && let Some(graph) = self.graph.take()
        {
            self.spare = graph;
            self.generation = next_generation();
        }
    }
    fn start(&mut self, camera: DVec3, radius: i32, smart_cull: bool) {
        let from = self.columns.section_of(camera);
        self.dirty = false;
        self.searched = Some((from, radius, smart_cull));
        if self.resync {
            self.resync_edits();
        }
        let graph = std::mem::take(&mut self.spare);
        if let Some(worker) = &self.worker {
            let job = Job {
                edits: std::mem::take(&mut self.edits),
                camera,
                radius,
                smart_cull,
                graph,
            };
            if worker.jobs.send(job).is_ok() {
                self.running = Some((from, radius, smart_cull));
                self.changed_late.clear();
                self.loaded_late.clear();
                return;
            }
            log::warn!("Sunrise: the occlusion worker stopped; culling on the render thread");
            self.worker = None;
        }
        self.edits.clear();
        let mut graph = Graph::default();
        graph.search(
            &self.columns,
            camera,
            radius,
            smart_cull,
            &mut self.vis,
            &mut self.queue,
        );
        self.land(graph);
    }
    fn collect(&mut self, wait: bool) {
        let Some(worker) = &self.worker else { return };
        if self.running.is_none() {
            return;
        }
        let result = if wait {
            worker.done.recv().map_err(|_| TryRecvError::Disconnected)
        } else {
            worker.done.try_recv()
        };
        match result {
            Ok((graph, edits)) => {
                self.running = None;
                if self.edits.is_empty() {
                    self.edits = edits;
                }
                if std::mem::take(&mut self.discard) {
                    self.spare = graph;
                    return;
                }
                self.land(graph);
                self.changed.clear();
                self.changed.append(&mut self.changed_late);
                self.loaded_since.clear();
                self.loaded_since.append(&mut self.loaded_late);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                // The edits it held are lost with it: search here from now on.
                self.worker = None;
                self.running = None;
                self.discard = false;
                self.dirty = true;
            }
        }
    }
    fn land(&mut self, graph: Graph) {
        if let Some(old) = self.graph.replace(graph) {
            self.spare = old;
        }
        self.generation = next_generation();
    }
    /// Vanilla's `runPartialUpdate`: resume waiting nodes in newly loaded columns and propagate
    /// changed sections from the current eye.
    fn propagate_changes(&mut self, camera: DVec3, smart_cull: bool) {
        let Some(graph) = &mut self.graph else {
            self.changed.clear();
            self.loaded_since.clear();
            return;
        };
        if self.changed.is_empty() && self.loaded_since.is_empty() {
            return;
        }
        let mut keys = Vec::new();
        for col in self.loaded_since.drain(..) {
            if self.columns.is_loaded(col.0, col.1)
                && let Some(waiting) = graph.waiting.remove(&col)
            {
                keys.extend(waiting);
            }
        }
        for key in self.changed.drain(..) {
            if graph.has_node(key) {
                keys.push(key);
            }
        }
        if keys.is_empty() {
            return;
        }
        let eye = self.columns.section_of(camera);
        let rebased = graph.rebase_for_partial(eye);
        self.queue.clear();
        for key in keys {
            if let Some(i) = graph.index(key)
                && graph.directions[i] & REACHED != 0
            {
                self.queue.push_back(i as u32);
            }
        }
        let columns = &self.columns;
        let grew = graph.propagate(
            columns,
            &mut self.queue,
            camera,
            false,
            smart_cull,
            |_, key| columns.get(key),
        );
        if rebased || grew {
            self.generation = next_generation();
        }
    }
    /// Camera's 0-based section key for this graph's vertical layout.
    pub(super) fn section_of(&self, camera: DVec3) -> SectionKey {
        self.columns.section_of(camera)
    }
    /// Wait for the running search: tests, parity probe, and A/B timing.
    pub(super) fn settle(&mut self, camera: DVec3, radius: i32, smart_cull: bool) {
        self.update(camera, radius, smart_cull);
        self.collect(true);
        self.propagate_changes(camera, smart_cull);
    }
    /// Runs only partial propagation from `camera` as if a full search were still in flight.
    pub(super) fn propagate_partial_in_flight(
        &mut self,
        camera: DVec3,
        radius: i32,
        smart_cull: bool,
    ) {
        let prev = self.running;
        if self.running.is_none() {
            self.running = Some((self.columns.section_of(camera), radius, smart_cull));
        }
        self.update(camera, radius, smart_cull);
        self.running = prev;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The centre of a section, in a world whose section 0 starts at y 0.
    fn at(key: SectionKey) -> DVec3 {
        DVec3::new(
            f64::from(key.0) * 16.0 + 8.0,
            f64::from(key.1) * 16.0 + 8.0,
            f64::from(key.2) * 16.0 + 8.0,
        )
    }

    fn load_square(o: &mut Occlusion, cx: i32, cz: i32, radius: i32) {
        for x in cx - radius..=cx + radius {
            for z in cz - radius..=cz + radius {
                o.load_column(x, z);
            }
        }
    }

    #[test]
    fn constants_are_vanillas() {
        assert_eq!(SECTION_DIAGONAL, (3f64.sqrt() * 16.0).ceil());
        assert_eq!(DISTANT_SECTIONS, 3);
    }

    #[test]
    fn floor_is_std_floor() {
        let mut x = -40_000.0_f64;
        while x < 40_000.0 {
            for v in [x, x + 0.5, x - 1e-9, x + 1e-9, -x] {
                assert_eq!(floor(v), v.floor() as i32, "{v}");
            }
            x += 0.37;
        }
    }

    #[test]
    fn open_space_reaches_the_whole_box() {
        let mut o = Occlusion::new(0, 4);
        load_square(&mut o, 0, 0, 2);
        o.settle(at((0, 1, 0)), 2, true);
        for x in -2..=2 {
            for z in -2..=2 {
                for y in 0..4 {
                    assert!(o.contains((x, y, z)), "{x} {y} {z}");
                }
            }
        }
        assert!(!o.contains((0, -1, 0)));
        assert!(!o.contains((0, 4, 0)));
        assert!(!o.contains((3, 1, 0)));
    }

    #[test]
    fn closed_neighbours_hide_what_is_behind_them() {
        let mut o = Occlusion::new(0, 4);
        load_square(&mut o, 0, 0, 3);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), 3, true);
        assert!(o.contains((1, 1, 0)), "a neighbour is seen");
        assert!(!o.contains((2, 1, 0)), "nothing shows through it");
        o.set((1, 1, 0), ALL_VISIBLE);
        o.settle(at((0, 1, 0)), 3, true);
        assert!(o.contains((2, 1, 0)));
    }

    /// `visible_for_camera` answers only for the box of the graph it holds, and
    /// only for the camera that search started from. Everywhere else it says
    /// unknown, so the caller keeps the geometry.
    #[test]
    fn visible_for_camera_is_unknown_outside_its_box_and_its_camera() {
        // No graph yet: nothing to answer with.
        assert_eq!(
            Occlusion::new(0, 4).visible_for_camera((0, 1, 0), (0, 1, 0)),
            None
        );
        let mut o = Occlusion::new(0, 4);
        load_square(&mut o, 0, 0, 2);
        o.settle(at((0, 1, 0)), 2, true);
        let from = o.columns.section_of(at((0, 1, 0)));
        assert_eq!(from, (0, 1, 0), "the camera section searched from");
        // Inside the searched box the answer is `contains`, its edges too.
        for x in -2..=2 {
            for z in -2..=2 {
                for y in 0..4 {
                    assert_eq!(
                        o.visible_for_camera((x, y, z), from),
                        Some(o.contains((x, y, z))),
                        "{x} {y} {z}"
                    );
                }
            }
        }
        assert_eq!(
            o.visible_for_camera((2, 1, 0), from),
            Some(true),
            "the box's edge"
        );
        // Off the box's sides, below it, or above the level: unknown.
        for key in [
            (3, 1, 0),
            (-3, 1, 0),
            (0, 1, 3),
            (0, 1, -3),
            (0, -1, 0),
            (0, -2, 0),
            (0, 4, 0),
            (0, 5, 0),
        ] {
            assert_eq!(o.visible_for_camera(key, from), None, "{key:?}");
        }
        // Another camera section: this graph is not about it.
        assert_eq!(o.visible_for_camera((0, 1, 0), (1, 1, 0)), None);
    }

    /// The answer follows the graph: a closed wall hides the section behind it,
    /// and opening the wall shows it again.
    #[test]
    fn visible_for_camera_reports_what_the_graph_hides() {
        let mut o = Occlusion::new(0, 4);
        load_square(&mut o, 0, 0, 3);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), 3, true);
        let from = (0, 1, 0);
        assert_eq!(
            o.visible_for_camera((1, 1, 0), from),
            Some(true),
            "the wall itself"
        );
        assert_eq!(
            o.visible_for_camera((2, 1, 0), from),
            Some(false),
            "hidden behind it"
        );
        o.set((1, 1, 0), ALL_VISIBLE);
        o.settle(at((0, 1, 0)), 3, true);
        assert_eq!(o.visible_for_camera((2, 1, 0), from), Some(true));
    }

    /// Faces `a` and `b` of a section see each other, nothing else does.
    fn connects(a: usize, b: usize) -> u64 {
        1 << (a * 6 + b) | 1 << (b * 6 + a)
    }

    /// #629 / #782 rule 1: a section first entered through a face that leads
    /// nowhere still leads on through the face it is entered through later.
    #[test]
    fn a_section_leads_on_through_every_face_it_is_entered_through() {
        let mut o = Occlusion::new(0, 4);
        load_square(&mut o, 0, 0, 3);
        // (1, 1, 1) is reached through its west face first (from (0, 1, 1),
        // queued before (1, 1, 0)), then through its north face. Only north
        // leads on, to the east.
        o.set((1, 1, 1), connects(2, 5));
        // Every other way into (2, 1, 1) is closed.
        for key in [(2, 1, 0), (2, 0, 1), (2, 2, 1)] {
            o.set(key, 0);
        }
        o.settle(at((0, 1, 0)), 3, true);
        let graph = o.graph.as_ref().expect("searched");
        let i = graph.index((1, 1, 1)).unwrap();
        assert_eq!(
            graph.entered[i],
            1 << 2 | 1 << 4,
            "entered from north and west"
        );
        assert!(o.contains((2, 1, 1)), "seen through the north face");
    }

    #[test]
    fn frames_never_wait_and_partial_updates_open_at_once() {
        let mut o = Occlusion::new(0, 4);
        load_square(&mut o, 0, 0, 3);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        let camera = at((0, 1, 0));
        o.settle(camera, 3, true);
        assert!(!o.contains((2, 1, 0)));
        // Opening the wall shows what is behind it in the same frame, from the
        // graph the frame has, whether or not the full search has finished.
        o.set((1, 1, 0), ALL_VISIBLE);
        o.update(camera, 3, true);
        assert!(o.contains((2, 1, 0)));
        assert!(o.contains((3, 1, 0)));
        // Closing it again lasts until the next full search lands.
        o.set((1, 1, 0), 0);
        o.settle(camera, 3, true);
        o.settle(camera, 3, true);
        assert!(!o.contains((2, 1, 0)));
    }

    #[test]
    fn a_graph_that_lost_the_camera_culls_nothing() {
        let mut o = Occlusion::new(0, 4);
        load_square(&mut o, 0, 0, 6);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), 3, true);
        assert!(!o.contains((3, 1, 0)));
        // The camera jumped past the wall; the next search is still running.
        let o = &mut o;
        o.update(at((3, 1, 0)), 3, true);
        if o.running.is_some() {
            assert!(o.contains((3, 1, 0)));
        }
        o.settle(at((3, 1, 0)), 3, true);
        assert!(o.contains((3, 1, 0)));
        assert!(
            !o.contains((0, 1, 0)),
            "the old camera section is walled in"
        );
    }

    /// #1037: from far above or below the world, the search starts from the
    /// world's top or bottom layer (`initializeQueueForFullUpdate`), so the
    /// ground stays drawn at any height.
    #[test]
    fn a_camera_outside_the_world_sees_it_from_its_top_or_bottom() {
        for from in [(0, 6, 0), (0, 9, 0), (2, 40, -1), (0, -1, 0), (1, -7, 0)] {
            let mut o = Occlusion::new(0, 6);
            load_square(&mut o, from.0, from.2, 3);
            o.settle(at(from), 3, true);
            let expected = reference(&o.columns, at(from), 3, true, true);
            for x in -1..=5 {
                for z in -4..=2 {
                    for y in 0..6 {
                        assert_eq!(
                            o.contains((x, y, z)),
                            expected.contains(&(x, y, z)),
                            "{from:?}: {x} {y} {z}"
                        );
                    }
                }
            }
            assert!(
                !o.contains(from),
                "{from:?}: the camera's own section is not in the world"
            );
        }
    }

    /// Seeds enter through the outer face and never turn back up (down)
    /// or towards the camera's column.
    #[test]
    fn a_closed_top_layer_hides_the_world_from_above() {
        let mut o = Occlusion::new(0, 6);
        load_square(&mut o, 0, 0, 6);
        for x in -3..=3 {
            for z in -3..=3 {
                o.set((x, 5, z), 0);
            }
        }
        for y in [6, 30] {
            o.settle(at((0, y, 0)), 3, true);
            assert!(
                o.contains((0, 5, 0)) && o.contains((3, 5, -3)),
                "the top is seen from y={y}"
            );
            assert!(
                !o.contains((0, 4, 0)) && !o.contains((2, 1, 2)),
                "nothing under it from y={y}"
            );
            assert!(!o.contains((0, 6, 0)), "seeds don't step back up");
        }
        // From below within vertical range, the open bottom layer shows everything up to the lid.
        o.settle(at((0, -1, 0)), 6, true);
        assert!(o.contains((0, 0, 0)) && o.contains((1, 4, 1)) && o.contains((1, 5, 1)));
        assert!(!o.contains((0, -1, 0)), "seeds don't step back down");
    }

    /// Independent vanilla `SectionOcclusionGraph.runUpdates` model using vanilla's literal formulas
    /// (without calling production `vertical_section_range`, `in_view_distance`, or `Graph::index`).
    fn reference(
        columns: &Columns,
        camera: DVec3,
        radius: i32,
        rays: bool,
        smart_cull: bool,
    ) -> FxHashSet<SectionKey> {
        struct Node {
            directions: u8,
            sources: u8,
        }
        let min_sec_y = columns.min_y >> 4;
        let max_sec_y = (columns.min_y + columns.sections * 16 - 1) >> 4;
        let cam_sx = floor(camera.x) >> 4;
        let cam_sy_abs = floor(camera.y) >> 4;
        let cam_sz = floor(camera.z) >> 4;
        let from = (cam_sx, cam_sy_abs - min_sec_y, cam_sz);
        // Literal `ChunkTrackingView.isWithinDistance(cam_sx, cam_sz, radius, x, z, false)`.
        let within_xz = |x: i32, z: i32| {
            let gx = ((i64::from(x) - i64::from(cam_sx)).abs() - 1).max(0);
            let gz = ((i64::from(z) - i64::from(cam_sz)).abs() - 1).max(0);
            let r = i64::from(radius);
            r > 0 && gx < r && gz < r && gx * gx + gz * gz < r * r
        };
        let block = |k: SectionKey| {
            [
                i64::from(k.0) * 16,
                i64::from(k.1 + min_sec_y) * 16,
                i64::from(k.2) * 16,
            ]
        };
        // Direction.getAxis in x, y, z order.
        let axis = |d: usize| [1, 2, 0][d / 2];
        let centre = [
            i64::from(cam_sx) * 16 + 8,
            i64::from(cam_sy_abs) * 16 + 8,
            i64::from(cam_sz) * 16 + 8,
        ];
        let mut nodes: HashMap<SectionKey, Node> = HashMap::new();
        let mut expanded: FxHashSet<SectionKey> = FxHashSet::default();
        let mut queue = VecDeque::new();
        if cam_sy_abs >= min_sec_y && cam_sy_abs <= max_sec_y {
            nodes.insert(
                from,
                Node {
                    directions: 0,
                    sources: 0,
                },
            );
            queue.push_back(from);
        } else {
            let above = cam_sy_abs > max_sec_y;
            let (y, travel) = if above {
                (max_sec_y - min_sec_y, 0)
            } else {
                (0, 1)
            };
            let camera_block = [i64::from(floor(camera.x)), i64::from(floor(camera.z))];
            let mut seeds = Vec::new();
            for x in -radius..=radius {
                for z in -radius..=radius {
                    if !within_xz(cam_sx + x, cam_sz + z) {
                        continue;
                    }
                    let key = (cam_sx + x, y, cam_sz + z);
                    let mut directions = 1 << travel;
                    if x > 0 {
                        directions |= 1 << 5;
                    } else if x < 0 {
                        directions |= 1 << 4;
                    }
                    if z > 0 {
                        directions |= 1 << 3;
                    } else if z < 0 {
                        directions |= 1 << 2;
                    }
                    let origin = block(key);
                    let dx = camera_block[0] - (origin[0] + 8);
                    let dz = camera_block[1] - (origin[2] + 8);
                    seeds.push((dx * dx + dz * dz, key, directions));
                }
            }
            seeds.sort_by_key(|seed| seed.0);
            for (_, key, directions) in seeds {
                nodes.insert(
                    key,
                    Node {
                        directions,
                        sources: 1 << travel,
                    },
                );
                queue.push_back(key);
            }
        }
        while let Some(key) = queue.pop_front() {
            if !columns.is_loaded(key.0, key.2) {
                continue;
            }
            expanded.insert(key);
            let node = &nodes[&key];
            let (directions, sources) = (node.directions, node.sources);
            let vis = columns.get(key);
            let key_sy_abs = key.1 + min_sec_y;
            let distant = (key.0 - cam_sx).abs() > 3
                || (key_sy_abs - cam_sy_abs).abs() > 3
                || (key.2 - cam_sz).abs() > 3;
            for (d, step) in STEPS.iter().enumerate() {
                let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
                let next_sy_abs = next.1 + min_sec_y;
                if next_sy_abs < min_sec_y
                    || next_sy_abs > max_sec_y
                    || (next_sy_abs - cam_sy_abs).abs() > radius
                    || !within_xz(next.0, next.2)
                {
                    continue;
                }
                if smart_cull && directions & (1 << (d ^ 1)) != 0 {
                    continue;
                }
                // facesCanSeeEachother(source.getOpposite(), direction).
                if smart_cull
                    && sources != 0
                    && !(0..6)
                        .any(|s| sources & (1 << s) != 0 && vis & (1 << ((s ^ 1) * 6 + d)) != 0)
                {
                    continue;
                }
                if smart_cull && rays && distant {
                    let origin = block(key);
                    let max = |c: usize| {
                        if c == axis(d) {
                            centre[c] > origin[c]
                        } else {
                            centre[c] < origin[c]
                        }
                    };
                    let mut check = DVec3::new(
                        (origin[0] + if max(0) { 16 } else { 0 }) as f64,
                        (origin[1] + if max(1) { 16 } else { 0 }) as f64,
                        (origin[2] + if max(2) { 16 } else { 0 }) as f64,
                    );
                    let length = |v: DVec3| (v.x * v.x + (v.y * v.y + v.z * v.z)).sqrt();
                    let to = camera - check;
                    let step = to * (1.0 / length(to)) * 28.0;
                    let mut visible = true;
                    while length(check - camera).powi(2) > 3600.0 {
                        check += step;
                        let max_y = columns.min_y + columns.sections * 16 - 1;
                        if check.y > f64::from(max_y) || check.y < f64::from(columns.min_y) {
                            break;
                        }
                        let section = (
                            floor(check.x) >> 4,
                            (floor(check.y) >> 4) - min_sec_y,
                            floor(check.z) >> 4,
                        );
                        if !nodes.contains_key(&section) {
                            visible = false;
                            break;
                        }
                    }
                    if !visible {
                        continue;
                    }
                }
                match nodes.get_mut(&next) {
                    Some(existing) => existing.sources |= 1 << d,
                    None => {
                        nodes.insert(
                            next,
                            Node {
                                directions: directions | 1 << d,
                                sources: 1 << d,
                            },
                        );
                        queue.push_back(next);
                    }
                }
            }
        }
        expanded
    }

    fn random_world(o: &mut Occlusion, seed: u64, count: usize, spread: i32) {
        let mut seed = seed;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let span = (2 * spread + 1) as u64;
        for _ in 0..count {
            let r = next();
            let key = (
                (r % span) as i32 - spread,
                ((r >> 8) % 6) as i32,
                ((r >> 16) % span) as i32 - spread,
            );
            // Open, half open or nearly closed sections.
            let vis = match r >> 40 & 3 {
                0 => next() & next() & next(),
                1 => next(),
                _ => next() | next(),
            };
            o.set(key, vis & ALL_VISIBLE);
        }
    }

    /// The graph is vanilla's, cell for cell, near the camera and beyond the
    /// distant step test's 60 blocks.
    #[test]
    #[ignore = "daily"]
    fn same_sections_as_vanilla() {
        let (mut hidden, mut rayed) = (0, 0);
        for seed in [0x9e37_79b9, 7, 0xdead_beef] {
            for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
                let mut o = Occlusion::new(0, 6);
                load_square(&mut o, 0, 0, spread + radius);
                random_world(&mut o, seed, count, spread);
                for from in [(0, 2, 0), (3, 0, -2), (-5, 5, 4), (1, -1, 1), (0, 7, 0)] {
                    // Off-centre, so the rays are not axis-aligned.
                    let camera = at(from) + DVec3::new(3.3, -5.7, 1.9);
                    o.settle(camera, radius, true);
                    o.settle(camera, radius, true);
                    let expected = reference(&o.columns, camera, radius, true, true);
                    let no_rays = reference(&o.columns, camera, radius, false, true);
                    for x in from.0 - radius - 1..=from.0 + radius + 1 {
                        for z in from.2 - radius - 1..=from.2 + radius + 1 {
                            for y in -1..=o.columns.sections {
                                let key = (x, y, z);
                                let actual = o.contains(key);
                                assert_eq!(
                                    actual,
                                    expected.contains(&key),
                                    "{seed} {from:?} {radius} {key:?}"
                                );
                                rayed += usize::from(no_rays.contains(&key) && !actual);
                                hidden +=
                                    usize::from(!actual && (0..o.columns.sections).contains(&y));
                            }
                        }
                    }
                }
            }
        }
        assert!(hidden > 1000, "the worlds hide sections ({hidden})");
        assert!(
            rayed > 100,
            "the distant step test culls sections ({rayed})"
        );
    }

    /// Vertical clamping matches vanilla's `SectionOcclusionGraph` exactly with no sky-band overdraw.
    #[test]
    #[ignore = "daily"]
    fn vertical_clamping_matches_vanilla_without_overdraw() {
        let mut compared = 0;
        let mut clamped = 0;
        for seed in [0x9e37_79b9, 7, 0xdead_beef, 0x2545_f491] {
            for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
                let mut o = Occlusion::new(0, 24);
                load_square(&mut o, 0, 0, spread + radius);
                random_world(&mut o, seed, count, spread);
                let camera = at((0, 2, 0)) + DVec3::new(3.3, -5.7, 1.9);
                o.settle(camera, radius, true);
                let graph = o.graph.as_ref().expect("searched");
                clamped += usize::from(graph.height < o.columns.sections);
                let full = reference(&o.columns, camera, radius, true, true);
                let from = o.columns.section_of(camera);
                for x in from.0 - radius - 1..=from.0 + radius + 1 {
                    for z in from.2 - radius - 1..=from.2 + radius + 1 {
                        for y in -1..=o.columns.sections {
                            let key = (x, y, z);
                            let expected = full.contains(&key);
                            let actual = o.contains(key);
                            compared += 1;
                            assert_eq!(
                                actual, expected,
                                "seed {seed:#x}, spread {spread}, radius {radius}, key={key:?}"
                            );
                        }
                    }
                }
            }
        }
        assert!(clamped > 0, "the comparison exercised a clamped search");
        assert!(compared > 50_000, "compared {compared} sections");
    }

    /// A line checked again from the sample it failed at, after more
    /// sections were reached, ends where checking it from its corner does.
    #[test]
    fn a_line_checked_again_resumes_where_it_failed() {
        let mut seed = 0x2545_f491_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut columns = Columns::new(-64, 24);
        for x in -20..=20 {
            for z in -20..=20 {
                columns.loaded.insert((x, z));
            }
        }
        let mut graph = Graph::default();
        let (mut vis, mut queue) = (Vec::new(), VecDeque::new());
        let (mut resumed, mut seen) = (0, 0);
        for _ in 0..40 {
            let camera = DVec3::new(
                (next() % 3000) as f64 / 100.0 - 15.0,
                (next() % 30000) as f64 / 100.0 - 60.0,
                (next() % 3000) as f64 / 100.0 - 15.0,
            );
            // Open air: every section reached; then holes along the lines.
            graph.search(&columns, camera, 12, true, &mut vis, &mut queue);
            for cell in graph.directions.iter_mut() {
                if next() % 4 == 0 {
                    *cell = 0;
                }
            }
            let from = graph.from;
            for _ in 0..25 {
                let corner = (
                    from.0 + (next() % 25) as i32 - 12,
                    graph.y0 + (next() % graph.height as u64) as i32,
                    from.2 + (next() % 25) as i32 - 12,
                );
                let mut known = 0;
                while let Some(failed) = graph.walk(corner, camera, known) {
                    for earlier in 0..=failed {
                        assert_eq!(graph.walk(corner, camera, earlier), Some(failed));
                    }
                    known = failed;
                    resumed += 1;
                    // More sections reached, as the search goes on.
                    for cell in graph.directions.iter_mut() {
                        if next() % 3 == 0 {
                            *cell |= REACHED | EXPANDED;
                        }
                    }
                    assert_eq!(
                        graph.walk(corner, camera, known),
                        graph.walk(corner, camera, 0)
                    );
                }
                seen += usize::from(known > 1);
            }
        }
        assert!(resumed > 200 && seen > 50, "{resumed} {seen}");
    }

    /// #782 rule 2: behind a wall, a section the search reaches around its
    /// edge is culled when the line to the camera crosses the wall's shadow.
    #[test]
    fn distant_sections_behind_a_wall_are_culled() {
        let mut o = Occlusion::new(0, 4);
        load_square(&mut o, 0, 0, 12);
        // A closed wall at x 2, z -2..=2, the whole height.
        for z in -2..=2 {
            for y in -1..=4 {
                o.set((2, y, z), 0);
            }
        }
        let camera = at((0, 1, 0));
        o.settle(camera, 12, true);
        let no_rays = reference(&o.columns, camera, 12, false, true);
        assert!(
            no_rays.contains(&(9, 1, 3)),
            "reached around the wall's edge"
        );
        assert!(
            !o.contains((9, 1, 3)),
            "but its line to the camera is hidden"
        );
        assert!(o.contains((9, 1, 9)), "a section off to the side is seen");
    }

    /// `cargo test --profile fast -p sunrise-client --lib occlusion::tests::search_cost -- --ignored --nocapture`:
    /// full searches at render distance 32 over a world with open sky above
    /// y 64 and caves below it.
    #[test]
    #[ignore]
    fn search_cost() {
        let mut o = Occlusion::new(-64, 24);
        load_square(&mut o, 0, 0, 40);
        let mut seed = 0x9e37_79b9_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for x in -40..=40 {
            for z in -40..=40 {
                for y in 0..24 {
                    let vis = if y >= 8 {
                        ALL_VISIBLE
                    } else if next() % 5 == 0 {
                        next() & ALL_VISIBLE
                    } else {
                        0
                    };
                    o.set((x, y, z), vis);
                }
            }
        }
        let camera = DVec3::new(3.3, 70.2, -5.6);
        let (mut vis, mut queue) = (Vec::new(), VecDeque::new());
        let mut graph = Graph::default();
        let mut best = f64::MAX;
        for _ in 0..12 {
            let started = std::time::Instant::now();
            graph.search(&o.columns, camera, 34, true, &mut vis, &mut queue);
            best = best.min(started.elapsed().as_secs_f64() * 1000.0);
        }
        let reached = graph
            .directions
            .iter()
            .filter(|&&d| d & EXPANDED != 0)
            .count();
        println!("search_cost: best {best:.2} ms over 12, {reached} reached");
    }

    /// Removed columns unload and drop connectivity: the graph matches a fresh world without them,
    /// whether some columns go or all of them (a teleport).
    #[test]
    fn removed_columns_leave_the_graph_of_a_world_without_them() {
        fn without(o: &Occlusion, gone: &FxHashSet<(i32, i32)>) -> Occlusion {
            let mut fresh = Occlusion::new(0, 6);
            for &(x, z) in &o.columns.loaded {
                if !gone.contains(&(x, z)) {
                    fresh.load_column(x, z);
                }
            }
            for (&(x, z), column) in &o.columns.map {
                if !gone.contains(&(x, z)) {
                    for (y, &vis) in column.iter().enumerate() {
                        fresh.set((x, y as i32, z), vis);
                    }
                }
            }
            fresh
        }
        let camera = at((0, 2, 0));
        let mut o = Occlusion::new(0, 6);
        load_square(&mut o, 0, 0, 6);
        random_world(&mut o, 0x51ed_2701, 900, 6);
        o.settle(camera, 6, true);
        let some: FxHashSet<(i32, i32)> = (-6..=6)
            .flat_map(|x| (-1..=1).map(move |z| (x, z)))
            .collect();
        let mut expected = without(&o, &some);
        o.remove_columns(&some);
        o.settle(camera, 6, true);
        expected.settle(camera, 6, true);
        let graph = |o: &Occlusion| o.graph.as_ref().expect("searched").directions.clone();
        assert_eq!(graph(&o), graph(&expected));

        let all: FxHashSet<(i32, i32)> = o
            .columns
            .loaded
            .iter()
            .copied()
            .chain(o.columns.map.keys().copied())
            .collect();
        o.remove_columns(&all);
        assert!(o.contains((40, 3, 40)), "no graph from the old view culls");
        load_square(&mut o, 0, 0, 6);
        random_world(&mut o, 0x2545_f491, 900, 6);
        let mut arrived = Occlusion::new(0, 6);
        load_square(&mut arrived, 0, 0, 6);
        random_world(&mut arrived, 0x2545_f491, 900, 6);
        o.settle(camera, 6, true);
        arrived.settle(camera, 6, true);
        assert_eq!(graph(&o), graph(&arrived));
    }

    /// The worker's graph is the one the render thread would compute.
    #[test]
    fn the_worker_matches_a_search_on_the_render_thread() {
        let mut o = Occlusion::new(0, 6);
        load_square(&mut o, 0, 0, 12);
        random_world(&mut o, 0x9e37_79b9, 800, 6);
        for from in [(0, 2, 0), (3, 0, -2), (-5, 5, 4)] {
            let camera = at(from);
            o.settle(camera, 6, true);
            o.settle(camera, 6, true);
            let mut local = Graph::default();
            local.search(
                &o.columns,
                camera,
                6,
                true,
                &mut Vec::new(),
                &mut VecDeque::new(),
            );
            let graph = o.graph.as_ref().expect("searched");
            assert_eq!(graph.directions, local.directions);
            assert_eq!(graph.entered, local.entered);
        }
    }

    #[test]
    fn vertical_clamping_bounds_height_to_render_distance() {
        let mut o = Occlusion::new(0, 24);
        load_square(&mut o, 0, 0, 4);
        o.set((0, 5, 0), 0); // Wall at section 5
        o.settle(at((0, 2, 0)), 4, true);
        let graph = o.graph.as_ref().expect("searched");
        assert_eq!((graph.y0, graph.height), (0, 7));
        assert!(o.contains((0, 2, 0)), "camera section is reached");
        assert!(o.contains((1, 6, 1)), "section at vertical edge is reached");
        assert!(!o.contains((1, 7, 1)), "past vertical render distance");
        assert!(
            !o.contains((1, 20, 1)),
            "far above vertical render distance"
        );

        // An enclosed cave cannot reach past its walls.
        let mut cave = Occlusion::new(0, 24);
        load_square(&mut cave, 0, 0, 4);
        for (dx, dy, dz) in STEPS {
            cave.set((dx, 1 + dy, dz), 0);
        }
        cave.settle(at((0, 1, 0)), 4, true);
        assert!(
            !cave.contains((0, 5, 0)),
            "enclosed cave does not see past its roof"
        );
    }

    /// `contains` over the columns `radius` + 3 around `from`, checked against `reach`.
    fn answers(o: &Occlusion, from: SectionKey, radius: i32) -> Vec<bool> {
        let reach = o.reach();
        assert_eq!(reach.is_some(), o.graph.is_some(), "a graph exported");
        assert!(o.reach_is_exact(), "the bits are the cells'");
        let past = |key: SectionKey| o.graph.as_ref().is_some_and(|g| g.index(key).is_none());
        let mut answers = Vec::new();
        for x in from.0 - radius - 3..=from.0 + radius + 3 {
            for z in from.2 - radius - 3..=from.2 + radius + 3 {
                for y in -2..=o.columns.sections + 1 {
                    let key = (x, y, z);
                    let contains = o.contains(key);
                    assert_eq!(reach.is_none_or(|r| r.contains(key)), contains, "{key:?}");
                    assert!(!(contains && past(key)), "{key:?} past the box");
                    answers.push(contains);
                }
            }
        }
        answers
    }

    /// The graph `reach` exports answers as `contains` (`answers`) across updates, clears, and clamped boxes.
    #[test]
    fn the_exported_graph_answers_as_contains() {
        let mut o = Occlusion::new(0, 4);
        load_square(&mut o, 0, 0, 3);
        let first = o.generation();
        answers(&o, (0, 1, 0), 3);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), 3, true);
        assert_ne!(o.generation(), first, "a search landed");
        let walled = answers(&o, (0, 1, 0), 3);
        let searched = o.generation();
        o.set((0, 1, 1), ALL_VISIBLE);
        o.update(at((0, 1, 0)), 3, true);
        assert_ne!(o.generation(), searched, "a partial update reached more");
        assert_ne!(answers(&o, (0, 1, 0), 3), walled);
        let opened = o.generation();
        o.clear();
        assert_ne!(o.generation(), opened, "the graph went");
        answers(&o, (0, 1, 0), 3);
        // Another level's graph never takes one of this one's generations, or
        // the pass would keep its copy of this one.
        let mut other = Occlusion::new(0, 4);
        load_square(&mut other, 0, 0, 3);
        other.settle(at((0, 1, 0)), 3, true);
        assert_ne!(other.generation(), searched, "another level's graph");

        let mut clamped = 0;
        let mut check = |mut o: Occlusion, camera: DVec3, radius: i32| {
            o.settle(camera, radius, true);
            answers(&o, o.columns.section_of(camera), radius);
            let g = o.graph.as_ref().expect("searched");
            if g.height < o.columns.sections {
                clamped += 1;
                assert!(!o.contains((g.from.0, g.y0 + g.height, g.from.2)));
            }
        };
        for seed in [0x9e37_79b9, 7, 0xdead_beef, 0x2545_f491] {
            for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
                let mut o = Occlusion::new(0, 24);
                load_square(&mut o, 0, 0, radius);
                random_world(&mut o, seed, count, spread);
                check(o, at((0, 2, 0)) + DVec3::new(3.3, -5.7, 1.9), radius);
            }
        }
        let mut cave = Occlusion::new(0, 24);
        load_square(&mut cave, 0, 0, 4);
        for (dx, dy, dz) in STEPS {
            cave.set((dx, 1 + dy, dz), 0);
        }
        check(cave, at((0, 1, 0)), 4);
        let mut above = Occlusion::new(0, 24);
        load_square(&mut above, 0, 0, 6);
        random_world(&mut above, 7, 900, 6);
        check(above, DVec3::new(3.3, 400.0, -5.6), 6);
        assert!(clamped > 0, "expected vertically clamped graphs");
    }

    /// Whenever an answer of `contains` changes, so does the generation: over
    /// 300 random steps, each a section set (open, closed or anything between),
    /// a column removed or loaded, the camera moved to another section (now and then
    /// above or below the world), or now and then another world, after a
    /// `clear` or a teleport that drops every column (`remove_columns`), each
    /// followed by a frame's `update` or a settled search; then an update from
    /// a section the graph never reached. The exported graph answers as
    /// `contains` after every step.
    #[test]
    fn every_change_of_the_answers_changes_the_generation() {
        let mut seed = 0x0bad_5eed_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let (spread, radius, around) = (5, 3, (0, 0, 0));
        let offset = DVec3::new(3.3, -5.7, 1.9);
        let mut o = Occlusion::new(0, 24);
        load_square(&mut o, 0, 0, spread + radius);
        random_world(&mut o, 0x2545_f491, 400, spread);
        let mut camera = at((0, 2, 0)) + offset;
        o.settle(camera, radius, true);
        let mut last = (o.generation(), answers(&o, around, spread + radius));
        let mut changed = 0;
        for step in 0..300 {
            let r = next();
            let key = (
                (r % 11) as i32 - spread,
                ((r >> 8) % 7) as i32,
                ((r >> 16) % 11) as i32 - spread,
            );
            match r >> 24 & 15 {
                0..=6 => {
                    let vis = match r >> 28 & 3 {
                        0 => ALL_VISIBLE,
                        1 => 0,
                        _ => next() & ALL_VISIBLE,
                    };
                    o.set(key, vis);
                }
                7 => o.load_column(key.0, key.2),
                8 | 9 => o.remove_column(key.0, key.2),
                10..=13 => {
                    let y = match r >> 32 & 7 {
                        0 => 30,
                        1 => -3,
                        _ => key.1,
                    };
                    camera = at((key.0, y, key.2)) + offset;
                }
                _ if r >> 36 & 7 == 0 => {
                    o.clear();
                    load_square(&mut o, 0, 0, spread + radius);
                    random_world(&mut o, next(), 400, spread);
                }
                _ if r >> 36 & 7 == 1 => {
                    // A teleport: every column goes at once.
                    let all: FxHashSet<_> = o
                        .columns
                        .loaded
                        .iter()
                        .copied()
                        .chain(o.columns.map.keys().copied())
                        .collect();
                    o.remove_columns(&all);
                    load_square(&mut o, 0, 0, spread + radius);
                    random_world(&mut o, next(), 400, spread);
                }
                _ => {}
            }
            if r >> 40 & 1 == 0 {
                o.update(camera, radius, true);
            } else {
                o.settle(camera, radius, true);
            }
            let now = (o.generation(), answers(&o, around, spread + radius));
            if now.1 != last.1 {
                assert_ne!(
                    now.0, last.0,
                    "step {step}: answers changed, the generation not"
                );
                changed += 1;
            }
            last = now;
        }
        assert!(changed >= 50, "{changed} steps changed answers");
        // Walled in, then past the wall: the graph never reached the camera's
        // section, and goes until the next search lands. (The first settle may
        // land a search that started before the wall.)
        load_square(&mut o, 0, 0, spread + radius);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 1 + dy, dz), 0);
        }
        o.settle(at((0, 1, 0)), radius, true);
        o.settle(at((0, 1, 0)), radius, true);
        let walled = (o.generation(), answers(&o, around, spread + radius));
        assert!(!o.contains((3, 1, 0)), "past the wall");
        o.update(at((3, 1, 0)), radius, true);
        assert_ne!(answers(&o, around, spread + radius), walled.1);
        assert_ne!(o.generation(), walled.0, "the graph went");
    }

    #[test]
    fn edit_queue_stays_bounded_without_searches() {
        let mut o = Occlusion::new(0, 4);
        load_square(&mut o, 0, 0, 6);
        // Fill the queue, trigger the resend, then edit once while it is pending.
        // Further edits repeat that last branch without exercising a new boundary.
        let total = EDIT_CAP + 2;
        for i in 0..total {
            o.set(((i % 700) as i32, 1, (i / 700) as i32), 1 + (i as u64 % 5));
        }
        assert!(o.edits.len() <= 1 << 16, "{} edits queued", o.edits.len());
        assert!(
            o.changed.len() <= 1 << 16,
            "{} changed queued",
            o.changed.len()
        );

        let mut fresh = Occlusion::new(0, 4);
        for &(x, z) in &o.columns.loaded {
            fresh.load_column(x, z);
        }
        for (&(x, z), column) in &o.columns.map {
            for (y, &vis) in column.iter().enumerate() {
                // Missing sections in the fresh world already connect every face.
                if vis != ALL_VISIBLE {
                    fresh.set((x, y as i32, z), vis);
                }
            }
        }
        let camera = at((0, 1, 0));
        for _ in 0..2 {
            o.settle(camera, 6, true);
            fresh.settle(camera, 6, true);
        }
        for x in -6..=6 {
            for z in -6..=6 {
                for y in 0..4 {
                    assert_eq!(
                        o.contains((x, y, z)),
                        fresh.contains((x, y, z)),
                        "the worker saw the same world at ({x}, {y}, {z})"
                    );
                }
            }
        }
    }

    #[test]
    fn extreme_y_camera_does_not_overflow() {
        let mut o = Occlusion::new(-64, 24);
        load_square(&mut o, 0, 0, 2);
        o.settle(DVec3::new(8.0, f64::from(i32::MAX), 8.0), 2, true);
        assert!(o.contains((0, 23, 0)));
        assert!(!o.contains((0, 22, 0)));
        o.settle(DVec3::new(8.0, f64::from(i32::MIN), 8.0), 2, true);
        assert!(o.contains((0, 0, 0)));
        assert!(!o.contains((0, 1, 0)));
    }

    #[test]
    fn smart_cull_off_keeps_bounds_and_ignores_barriers() {
        let mut o = Occlusion::new(0, 24);
        load_square(&mut o, 0, 0, 4);
        for (dx, dy, dz) in STEPS {
            o.set((dx, 10 + dy, dz), 0);
        }
        let camera = at((0, 10, 0));
        o.settle(camera, 2, true);
        assert!(!o.contains((2, 10, 0)), "barrier hides with smart_cull on");
        o.settle(camera, 2, false);
        assert!(
            o.contains((2, 10, 0)),
            "barrier ignored with smart_cull off"
        );
        assert!(
            !o.contains((0, 13, 0)),
            "vertical render distance still clamps with smart_cull off"
        );
        assert!(
            !o.contains((3, 10, 0)),
            "horizontal render distance still bounds with smart_cull off"
        );
    }

    #[test]
    fn unloaded_stripe_stops_propagation_and_resumes_when_loaded() {
        for smart_cull in [true, false] {
            let mut o = Occlusion::new(0, 24);
            for x in -2..=2 {
                for z in -2..=2 {
                    if x != 1 {
                        o.load_column(x, z);
                    }
                }
            }
            let camera = at((0, 10, 0));
            o.settle(camera, 2, smart_cull);
            assert!(o.contains((0, 10, 0)), "camera column is loaded");
            assert!(
                !o.contains((1, 10, 0)),
                "unloaded stripe not expanded (smart_cull={smart_cull})"
            );
            assert!(
                !o.contains((2, 10, 0)),
                "loaded target behind unloaded stripe blocked (smart_cull={smart_cull})"
            );
            assert!(
                !o.reach().expect("graph").contains((2, 10, 0)),
                "Reach matches blocked target (smart_cull={smart_cull})"
            );

            // Loading (1, 0) resumes waiting nodes immediately on partial update.
            o.load_column(1, 0);
            o.update(camera, 2, smart_cull);
            assert!(
                o.contains((1, 10, 0)) && o.contains((2, 10, 0)),
                "loading stripe resumes propagation into x=2 (smart_cull={smart_cull})"
            );
            assert!(
                o.reach().expect("graph").contains((2, 10, 0)),
                "Reach updates immediately on column load (smart_cull={smart_cull})"
            );
            assert!(o.reach_is_exact());
        }
    }

    #[test]
    fn partial_update_uses_current_eye_across_shifted_vertical_band() {
        let mut o = Occlusion::new(0, 24);
        load_square(&mut o, 0, 0, 3);
        o.set((0, 12, 0), 0);
        o.settle(at((0, 10, 0)), 2, true);
        assert!(o.contains((0, 12, 0)));
        assert!(!o.contains((0, 13, 0)));
        assert!(!o.reach().expect("graph").contains((0, 13, 0)));

        // Move camera to Y=11 and open (0, 12, 0) while the next full search is still in flight.
        let before_gen = o.generation();
        o.running = Some(((0, 11, 0), 2, true));
        o.set((0, 12, 0), ALL_VISIBLE);
        o.update(at((0, 11, 0)), 2, true);
        assert!(
            o.contains((0, 13, 0)),
            "partial update reaches (0, 13, 0) from current eye Y=11 before full search lands"
        );
        assert!(
            o.reach().expect("graph").contains((0, 13, 0)),
            "exported Reach reflects rebased partial propagation immediately"
        );
        assert!(o.reach_is_exact());
        assert_ne!(o.generation(), before_gen);

        o.running = None;
        o.settle(at((0, 11, 0)), 2, true);
        assert!(o.contains((0, 13, 0)));
    }

    #[test]
    fn reference_includes_adjacent_columns_at_distance_one() {
        let mut o = Occlusion::new(0, 6);
        load_square(&mut o, 0, 0, 3);
        let camera = at((0, 2, 0));
        o.settle(camera, 1, true);
        let expected = reference(&o.columns, camera, 1, true, true);
        for x in -2i32..=2 {
            for z in -2i32..=2 {
                for y in 0i32..6 {
                    let key = (x, y, z);
                    let in_box = x.abs() <= 1 && z.abs() <= 1 && (y - 2).abs() <= 1;
                    assert_eq!(expected.contains(&key), in_box, "reference at {key:?}");
                    assert_eq!(o.contains(key), in_box, "occlusion at {key:?}");
                }
            }
        }
    }
}
