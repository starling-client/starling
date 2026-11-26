//! Sourced geometry drawn on the GPU while GPU culling is on (`gpu_culling`).
//! Sourced geometry is the meshes a source hands the renderer, each over a cell of its
//! own (#2500's `Cell`), and nothing here knows what made one. The pass reads only what
//! the renderer's residency keeps for every sourced mesh: its cell, its arena range and
//! buckets by facing side, its bounds, and whether it fades in or out, stands in or goes.
//!
//! Every resident mesh with solid quads has a record, in a second table of GPU culling's
//! own pass (`GpuCulling::second`): the same `gpu_culling.wgsl` entry points, with the
//! view-distance test off (`NO_RANGE`), cave culling's graph off, and each record's box in
//! units of its cell's (`Record::units`). Its tables are GPU culling's too (`tables`): the
//! origins of the cells' corners along x and z, as the CPU's f64 difference with the
//! camera rounded to f32, the frustum planes' terms for them and the eye's keys against
//! them, so a mesh drawn here shows the quads the CPU's draw would, where it would.
//! A source's residency decides its range, so only the frustum is tested.
//!
//! The residency is this module's (`Sourced`): the table's records and flags, and the
//! meshes a GPU frame still lists the CPU (`listed`) — those fading in or out, standing
//! in for their replacement or going, and those with translucent quads, whose sort is
//! unchanged. `gfx/lod.rs` calls in as its tiles come and go.
//!
//! The table is made when the first mesh with solid quads is resident and dropped with
//! its last (`SourcedGpu::new`), and with GPU culling's pass, where that is offered:
//! with nothing resident, no pass is dispatched and no buffer is held. It draws only in
//! a frame GPU culling draws (`Sourced::prepare`). Everywhere else, sourced
//! geometry goes the CPU's way, unchanged: OpenGL, Metal, DX12 with flat page groups,
//! Intel Gen11/12's Vulkan, software adapters, a device without count draws, GPU culling
//! switched off, out of memory or after device loss. It asks the device for nothing GPU
//! culling's pass does not.
//!
//! Draw order: its solid draws go after the near sections' and the CPU's (`draw`), keeping
//! #627's near-first order: sourced geometry lies past the render distance, behind the near
//! terrain, so depth rejects its hidden pixels early. Within a page the draws come in id
//! order, the same every frame, and steady meshes do not overlap (a replaced mesh is going
//! and its replacement fades in, both the CPU's), so no pixel goes to whichever draws last.
use super::super::{
    Frustum, SectionKey,
    arena::Arena,
    gpu_culling::{
        Cells, FrameInputs, GpuCulling, PLANE_BOX_WORDS, Prepared, Record, Records, rounded_box,
    },
    lod::{self, Tile, TileGrid, Visible},
    pack_timing::PassTiming,
    resident,
    section_fade::Fades,
    submit::Queue,
    upload::Uploads,
};
use crate::{fast_hash::FxHashMap, lod::TileKey, mesher::Sides};
use glam::{DVec3, Vec3};
use std::{collections::BTreeMap, time::Instant};

/// Cells are at least a section wide (a record's key is its corner in sections), and a box's
/// unit is a sixteenth of one's width: `Record::units` holds four bits of shift.
const MIN_SIZE_LOG2: u8 = 4;
const MAX_SIZE_LOG2: u8 = MIN_SIZE_LOG2 + 15;
/// A box's step along y, in blocks, as a shift (`cell_box`): codes reach -128 to 368, past the
/// overworld's -64 to 320.
const Y_SHIFT: u32 = 4;

/// Where a sourced mesh lies (#2500's `Cell`): `1 << size_log2` blocks wide along x and z, at
/// `x` and `z` in cells of that size, its corner at y 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cell {
    pub x: i32,
    pub z: i32,
    pub size_log2: u8,
}

impl Cell {
    /// Its corner in sections, the key of its record; `None` where the pass does not take it
    /// (narrower than a section, or wider than a box's units hold, or its corner past an i32).
    fn key(self) -> Option<SectionKey> {
        let k = u32::from(self.size_log2.checked_sub(MIN_SIZE_LOG2)?);
        let corner = |c: i32| i32::try_from(i64::from(c) << k).ok();
        (self.size_log2 <= MAX_SIZE_LOG2).then_some(())?;
        Some((corner(self.x)?, 0, corner(self.z)?))
    }

    /// A box's step along x and z, as a shift: a sixteenth of its width.
    fn shift(self) -> u32 {
        u32::from(self.size_log2 - MIN_SIZE_LOG2)
    }
}

/// The cell a tile covers, and the tile of a cell.
fn cell(key: TileKey) -> Cell {
    Cell {
        x: key.x,
        z: key.z,
        size_log2: key.level + 4,
    }
}
fn tile_key(cell: Cell) -> TileKey {
    TileKey {
        x: cell.x,
        z: cell.z,
        level: cell.size_log2 - 4,
    }
}

/// Calls `list` for each tile of `cells` in view, marking the animated layers it shows in
/// `animated`. Far tiles take the frustum test a cell at a time: a cell out of view leaves every
/// tile in it out, and one wholly in view leaves them only their distance.
pub(crate) fn walk_tiles(
    cells: &TileGrid,
    lods: &FxHashMap<TileKey, Tile>,
    camera: DVec3,
    view: &Frustum,
    now: Instant,
    mut animated: Option<&mut Vec<u64>>,
    list: &mut impl FnMut(Visible),
) {
    cells.walk(camera, view, |inside, key| {
        let Some(tile) = lods.get(&key) else {
            return;
        };
        let Some(listed) = (if inside {
            tile.visible_inside(key, camera, now)
        } else {
            tile.visible(key, camera, view, now)
        }) else {
            return;
        };
        list(listed);
        if let Some(active) = &mut animated {
            super::super::draw_frame::mark_layers(active, &tile.animated);
        }
    });
}

/// What the pass reads of a resident mesh.
pub(crate) struct Mesh<'a> {
    pub cell: Cell,
    /// Its solid quads' arena page, first vertex (as a draw's base vertex) and count.
    pub page: u32,
    pub start: u64,
    pub quads: u32,
    pub sides: &'a Sides,
    /// Its quads' bounds around the cell's corner.
    pub bounds: [Vec3; 2],
}

/// A record's box: the bounds rounded out to whole steps, `rounded_box`'s packing, in units of
/// a sixteenth of the cell's width along x and z and `1 << Y_SHIFT` blocks along y; `UNBOUNDED`
/// past what its codes hold.
fn cell_box(bounds: [Vec3; 2], cell: Cell) -> u32 {
    let flat = (1u32 << cell.shift()) as f32;
    let unit = Vec3::new(flat, (1u32 << Y_SHIFT) as f32, flat);
    rounded_box(Some(bounds.map(|b| b / unit)))
}

/// The second table, its pass's buffers (`GpuCulling`) and what follows its records.
pub(crate) struct SourcedGpu {
    gpu: GpuCulling,
    /// The flagged meshes that time alone makes whole, by when.
    due: Fades,
    /// The recorded cells' corners along x and z, in sections: the tables span them.
    corners: [BTreeMap<i32, u32>; 2],
    /// Per `Cell::shift`, the records with it: the largest decides the tables' margins.
    shifts: [u32; MAX_SIZE_LOG2 as usize - MIN_SIZE_LOG2 as usize + 1],
    /// The instances' UV step, f32 bits.
    step: u32,
}

impl SourcedGpu {
    /// The table beside `gpu`'s pass, for instances drawn with UV step `step`: `None` when the
    /// memory for its buffers is missing.
    pub fn new(gpu: &GpuCulling, device: &wgpu::Device, step: f32) -> Option<Self> {
        Some(Self {
            gpu: gpu.second(device)?,
            due: Fades::default(),
            corners: Default::default(),
            shifts: Default::default(),
            step: step.to_bits(),
        })
    }

    /// Whether no mesh has a record.
    pub fn is_empty(&self) -> bool {
        self.gpu.records.live() == 0
    }

    /// The table, for the arena's compaction to move its records with their meshes.
    pub fn records(&mut self) -> &mut Records {
        &mut self.gpu.records
    }

    /// `mesh`'s record, `FADING` while the CPU draws it; `None` for a cell the pass does not
    /// take (the CPU draws it).
    pub fn record(mesh: &Mesh, fading: bool) -> Option<Record> {
        let cell = mesh.cell;
        let key = cell.key()?;
        let boxed = cell_box(mesh.bounds, cell);
        let record = Record::new(key, mesh.page, mesh.start, mesh.quads, boxed);
        Some(
            record
                .facing(mesh.sides)
                .units(cell.shift(), Y_SHIFT)
                .fading(fading),
        )
    }

    /// The mesh `mesh`, `fading` while the CPU draws it, has a record: its id, or `None` for a
    /// cell the pass does not take.
    pub fn add(&mut self, mesh: &Mesh, fading: bool) -> Option<u32> {
        let record = Self::record(mesh, fading)?;
        self.count(mesh.cell, 1);
        Some(self.gpu.records.insert(record))
    }

    /// The mesh in `cell` with record `id` went.
    pub fn remove(&mut self, cell: Cell, id: u32) {
        self.gpu.records.remove(id);
        self.count(cell, -1);
    }

    /// The mesh with record `id` began to be drawn by the CPU, or stopped.
    pub fn set_fading(&mut self, id: u32, on: bool) {
        self.gpu.records.set_fading(id, on);
    }

    /// The flagged mesh in `cell` is whole at `end` at the latest, if time alone makes it so:
    /// it is looked at again then (`take_due`).
    pub fn wait(&mut self, cell: Cell, end: Instant) {
        self.due
            .push(end, (cell.x, i32::from(cell.size_log2), cell.z));
    }

    /// Debug builds: the table, and the cells whose end is waited for.
    #[cfg(debug_assertions)]
    pub fn checked(&self) -> (&Records, impl Iterator<Item = Cell>) {
        let waits = self.due.pending().map(|(x, size, z)| Cell {
            x,
            z,
            size_log2: size as u8,
        });
        (&self.gpu.records, waits)
    }

    /// Calls `look` for each flagged mesh whose end is due at `now`, before a frame's
    /// `prepare`, which sends the flags, and no later than the frame's instances are decided.
    pub fn take_due(&mut self, now: Instant, mut look: impl FnMut(Cell)) {
        while let Some((x, size, z)) = self.due.pop_due(now) {
            look(Cell {
                x,
                z,
                size_log2: size as u8,
            });
        }
    }

    fn count(&mut self, cell: Cell, by: i32) {
        let Some((x, _, z)) = cell.key() else {
            return;
        };
        for (corners, at) in self.corners.iter_mut().zip([x, z]) {
            let n = corners.entry(at).or_insert(0);
            *n = n.wrapping_add_signed(by);
            if *n == 0 {
                corners.remove(&at);
            }
        }
        let shift = &mut self.shifts[cell.shift() as usize];
        *shift = shift.wrapping_add_signed(by);
    }

    /// Records the frame's uploads into `encoder` and clears its counts, before the walk, for a
    /// frame with the camera at `camera` over a level from `min_y`, culled with `frustum`,
    /// solid quads facing away from `eye` left out (`None`: all drawn). Then `dispatch`, and
    /// the world pass's `draw`. Unless the frame is `Prepared::Gpu`, nothing is drawn here:
    /// the CPU draws the meshes, and the records that changed wait for the next frame.
    ///
    /// A spread of records whose tables the device's buffer limits do not hold, or whose
    /// origins leave the i32 keys, is a `Prepared::Cpu` frame too: encoding them would ask
    /// for a buffer that does not fit, and that would turn GPU culling off for the near
    /// sections' table as well.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &Queue,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        camera: DVec3,
        min_y: i32,
        frustum: &Frustum,
        eye: Option<Vec3>,
        timing: Option<&mut PassTiming>,
    ) -> Prepared {
        if !camera.is_finite() {
            return Prepared::Cpu;
        }
        let (cx, cz) = resident::camera_column(camera);
        // How far the records spread from the camera's column, in sections. Widened, for
        // recorded corners reach past an i32's differences.
        let reach = |corners: &BTreeMap<i32, u32>, at: i32| {
            let ends = corners.first_key_value().zip(corners.last_key_value());
            ends.map_or(0i64, |((&low, _), (&high, _))| {
                (i64::from(at) - i64::from(low)).max(i64::from(high) - i64::from(at))
            })
        };
        let radius = reach(&self.corners[0], cx).max(reach(&self.corners[1], cz));
        // The tables' words at this spread (`gpu_culling::tables`): an origin, the six
        // planes' terms and optional eye keys per axis entry, plus the fixed box
        // coefficients and each page's two words (`GpuCulling::prepare_buffers`).
        let Some(span) = radius.checked_mul(2).and_then(|r| r.checked_add(1)) else {
            return Prepared::Cpu;
        };
        // The origins along x and z and the one row along y: `nx + ny + nz` axis entries.
        let Some(axes) = span.checked_mul(2).and_then(|s| s.checked_add(1)) else {
            return Prepared::Cpu;
        };
        let words = (7 + u64::from(eye.is_some())) * u64::try_from(axes).unwrap_or(u64::MAX);
        let pages = (self.gpu.records.page_sections().len().max(1) as u64) * 2;
        let limits = device.limits();
        let fits = |bytes: u64| {
            bytes <= limits.max_buffer_size && bytes <= limits.max_storage_buffer_binding_size
        };
        let bytes = words
            .saturating_add(PLANE_BOX_WORDS + pages)
            .saturating_mul(4);
        if !fits(bytes) {
            return Prepared::Cpu;
        }
        // The origins' keys are i32 (`tables`), and so is the frame's radius.
        let in_keys = |at: i32| {
            i64::from(at) - radius >= i64::from(i32::MIN)
                && i64::from(at) + radius <= i64::from(i32::MAX)
        };
        if !in_keys(cx) || !in_keys(cz) {
            return Prepared::Cpu;
        }
        let xz = self.shifts.iter().rposition(|&n| n > 0).unwrap_or(0) as u32;
        let cells = Cells {
            units: [xz, Y_SHIFT],
            step: self.step,
        };
        let input = FrameInputs::new(camera, min_y, 1, radius as i32, frustum)
            .with_eye(eye)
            .with_cells(cells);
        self.gpu
            .prepare(device, queue, uploads, encoder, &input, timing)
    }

    /// The pass, after `prepare`: the changed records into the table, then the draws of the
    /// meshes it keeps.
    pub fn dispatch(&self, encoder: &mut wgpu::CommandEncoder, timing: Option<&mut PassTiming>) {
        self.gpu.dispatch(encoder, timing);
    }

    /// The kept meshes' solid draws, a count draw per page, with the quad indices bound. Leaves
    /// the origins at vertex slot 1. `group`: the page's group 1 for pipelines that read the
    /// page themselves (`Flat::page_group`), else none.
    pub fn draw<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        arena: &Arena,
        group: impl Fn(u32) -> Option<&'a wgpu::BindGroup>,
    ) {
        self.gpu.draw(pass, arena, group, 0);
    }

    /// Draw calls `draw` makes.
    pub fn draw_calls(&self) -> usize {
        self.gpu.draw_calls()
    }

    /// The frame's statistics copied out with the CPU's part of frame `frame`
    /// (`GpuCulling::copy_stats`).
    pub fn copy_stats(&mut self, encoder: &mut wgpu::CommandEncoder, frame: u64, cpu: [usize; 3]) {
        self.gpu.copy_stats(encoder, frame, cpu, false);
    }

    /// The frame the submit thread submitted's readback (`GpuCulling::submitted`).
    pub fn submitted(&mut self) -> Option<impl FnOnce() + Send + 'static> {
        self.gpu.submitted()
    }

    /// The statistics that came back (`GpuCulling::collect`).
    pub fn collect(&mut self) -> Vec<(u64, [usize; 3], bool)> {
        self.gpu.collect()
    }
}

/// One resident mesh's record: its id in the table, and whether it is flagged (`FADING`):
/// the CPU draws its solid quads while it fades in or out, stands in or goes.
struct Tracked {
    id: u32,
    fading: bool,
}

/// Sourced geometry's residency: the record table while GPU culling is on, each resident
/// mesh's record in it (`tracked`), and the meshes a GPU frame still lists the CPU
/// (`listed`), which the frame walk reads. `gfx/lod.rs` calls in as its tiles come and go;
/// the flags follow the tiles' fades (`refresh`, `take_due`).
#[derive(Default)]
pub(crate) struct Sourced {
    /// The table: none while no mesh with solid quads is resident, or its buffers did
    /// not fit.
    table: Option<SourcedGpu>,
    tracked: FxHashMap<TileKey, Tracked>,
    listed: TileGrid,
}

impl Sourced {
    /// Whether there is no table, and so no frame the pass draws in.
    pub(crate) fn off(&self) -> bool {
        self.table.is_none()
    }

    /// The mesh at `key` has a record the pass draws whole: its solid quads are not the
    /// CPU's in a frame the pass draws.
    pub(crate) fn draws_solid(&self, key: TileKey) -> bool {
        self.tracked.get(&key).is_some_and(|t| !t.fading)
    }

    /// Whether a frame the pass draws in lists the mesh at `key` the CPU.
    pub(crate) fn lists(&self, key: TileKey) -> bool {
        self.listed.contains(key)
    }

    /// The records to move with their meshes, and each mesh's own (`move_off_page`).
    pub(crate) fn moves(&mut self) -> (Option<&mut Records>, impl Fn(TileKey) -> Option<u32> + '_) {
        let Self { table, tracked, .. } = self;
        let records = table.as_mut().map(|t| t.records());
        let tracked = &*tracked;
        (records, move |key| tracked.get(&key).map(|t| t.id))
    }

    /// The table's pass for a frame GPU culling draws, the camera at `camera`:
    /// `Prepared::Gpu` when it draws the steady meshes' solid quads (`SourcedGpu::prepare`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &Queue,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        camera: DVec3,
        min_y: i32,
        frustum: &Frustum,
        eye: Option<Vec3>,
        timing: Option<&mut PassTiming>,
    ) -> Prepared {
        match &mut self.table {
            Some(table) => table.prepare(
                device, queue, uploads, encoder, camera, min_y, frustum, eye, timing,
            ),
            None => Prepared::Cpu,
        }
    }

    /// The frame's pass, after its `prepare` (`SourcedGpu::dispatch`).
    pub(crate) fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timing: Option<&mut PassTiming>,
    ) {
        if let Some(table) = &self.table {
            table.dispatch(encoder, timing);
        }
    }

    /// The kept meshes' solid draws, after the near sections' and the CPU's
    /// (`SourcedGpu::draw`).
    pub(crate) fn draw<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        arena: &Arena,
        group: impl Fn(u32) -> Option<&'a wgpu::BindGroup>,
    ) {
        if let Some(table) = &self.table {
            table.draw(pass, arena, group);
        }
    }

    /// Draw calls `draw` makes.
    pub(crate) fn draw_calls(&self) -> usize {
        self.table.as_ref().map_or(0, |t| t.draw_calls())
    }

    /// The frame's statistics copied out with the CPU's part of frame `frame`
    /// (`SourcedGpu::copy_stats`).
    pub(crate) fn copy_stats(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: u64,
        cpu: [usize; 3],
    ) {
        if let Some(table) = &mut self.table {
            table.copy_stats(encoder, frame, cpu);
        }
    }

    /// The frame the submit thread submitted's readback (`SourcedGpu::submitted`).
    pub(crate) fn submitted(&mut self) -> Option<impl FnOnce() + Send + 'static> {
        self.table.as_mut().and_then(|t| t.submitted())
    }

    /// The statistics that came back (`SourcedGpu::collect`), without the near
    /// pass's sourced flag.
    pub(crate) fn collect(&mut self) -> Vec<(u64, [usize; 3])> {
        self.table.as_mut().map_or_else(Vec::new, |t| {
            t.collect()
                .into_iter()
                .map(|(frame, counts, _)| (frame, counts))
                .collect()
        })
    }

    /// The tile at `key` came in, whole or fading at `now`: its record, its flag and its
    /// place in `listed`. `Some(false)`: recorded; `Some(true)`: the table could not be
    /// made, and its buffers do not fit in video memory (the caller turns GPU culling off:
    /// `set_gpu_culling`). A tile whose cell the pass does not take, or whose mesh has no
    /// solid quads, stays the CPU's.
    pub(crate) fn record(
        &mut self,
        key: TileKey,
        tile: &mut Tile,
        gpu: Option<&GpuCulling>,
        device: &wgpu::Device,
        now: Instant,
    ) -> bool {
        if self.table.is_none() && tile.mesh.solid > 0 {
            let Some(gpu) = gpu else {
                return false;
            };
            // One table for every record, made with the first: past its buffers, GPU
            // culling goes off whole rather than a partial table lie about its records.
            match SourcedGpu::new(gpu, device, lod::instance([0.0; 3], 0)[3]) {
                Some(table) => self.table = Some(table),
                None => return true,
            }
        }
        if let Some(table) = &mut self.table
            && tile.mesh.solid > 0
        {
            let fading = !tile.steady(key, now);
            if let Some(id) = table.add(&tile.gpu_mesh(key), fading) {
                self.tracked.insert(key, Tracked { id, fading });
                if let Some(end) = tile.whole_at().filter(|_| fading) {
                    table.wait(cell(key), end);
                }
            }
        }
        if gpu.is_some() {
            let listed = match self.tracked.get(&key) {
                Some(tracked) => tracked.fading || tile.mesh.translucent > 0,
                None => true,
            };
            if listed {
                self.listed.insert(key, tile.bounds());
            } else {
                self.listed.remove(key);
            }
        }
        false
    }

    /// The tile at `key` went (or stands in as a mesh of its own): its record and its place
    /// in `listed`. The table goes with its last record.
    pub(crate) fn unrecord(&mut self, key: TileKey) {
        if let Some(tracked) = self.tracked.remove(&key)
            && let Some(table) = &mut self.table
        {
            table.remove(cell(key), tracked.id);
            if table.is_empty() {
                self.table = None;
            }
        }
        self.listed.remove(key);
    }

    /// The tiles at `keys` changed in place at `now` (they fade in, go, or their stand-in
    /// went), or time may have made them whole: their records' flags and places in `listed`
    /// follow.
    pub(crate) fn refresh(
        &mut self,
        keys: impl IntoIterator<Item = TileKey>,
        tiles: &FxHashMap<TileKey, Tile>,
        now: Instant,
    ) {
        let Some(table) = &mut self.table else {
            return;
        };
        for key in keys {
            let Some(tile) = tiles.get(&key) else {
                continue;
            };
            let Some(tracked) = self.tracked.get_mut(&key) else {
                continue;
            };
            let fading = !tile.steady(key, now);
            if fading != tracked.fading {
                tracked.fading = fading;
                table.set_fading(tracked.id, fading);
                if fading || tile.mesh.translucent > 0 {
                    self.listed.insert(key, tile.bounds());
                } else {
                    self.listed.remove(key);
                }
            }
            if let Some(end) = tile.whole_at().filter(|_| fading) {
                table.wait(cell(key), end);
            }
        }
    }

    /// The flagged tiles whose fade-in is due to end by `now`, handed to the pass if whole by
    /// then: before a frame's `prepare`, which sends the flags.
    pub(crate) fn take_due(&mut self, tiles: &FxHashMap<TileKey, Tile>, now: Instant) {
        let mut due = Vec::new();
        if let Some(table) = &mut self.table {
            table.take_due(now, |c| due.push(tile_key(c)));
        }
        self.refresh(due, tiles, now);
    }

    /// GPU culling came on with tiles resident: a table for them all, made before the walk,
    /// so one that does not fit leaves every tile the CPU's rather than a partial table.
    /// `Some(true)`: it did not fit, and the caller turns GPU culling off.
    pub(crate) fn record_all(
        &mut self,
        tiles: &mut FxHashMap<TileKey, Tile>,
        gpu: Option<&GpuCulling>,
        device: &wgpu::Device,
    ) -> bool {
        self.clear();
        let Some(gpu) = gpu else {
            return false;
        };
        if !tiles.values().any(|tile| tile.mesh.solid > 0) {
            // No table while no mesh with solid quads is resident, but GPU culling is
            // on and the frame walk reads `listed`: every tile is the CPU's, and keeps
            // its place for the table the first solid tile makes.
            for (&key, tile) in tiles.iter_mut() {
                self.listed.insert(key, tile.bounds());
            }
            return false;
        }
        match SourcedGpu::new(gpu, device, lod::instance([0.0; 3], 0)[3]) {
            Some(table) => self.table = Some(table),
            None => return true,
        }
        let now = Instant::now();
        for (&key, tile) in tiles.iter_mut() {
            if self.record(key, tile, Some(gpu), device, now) {
                return true;
            }
        }
        false
    }

    /// No residency left: no table, no records, nothing listed (`set_gpu_culling`'s off,
    /// `clear_lods`, a world left).
    pub(crate) fn clear(&mut self) {
        self.table = None;
        self.tracked.clear();
        self.listed.clear();
    }

    /// Debug builds: the records are the tiles' as they are now (compaction's moves included),
    /// the pass draws only tiles whole at `now`, a flagged tile time makes whole is waited for,
    /// and `listed` holds exactly the tiles a GPU frame lists. With GPU culling off (`gpu_on`
    /// false) nothing is listed; on, the tiles it leaves the CPU keep their places even while
    /// no solid mesh has made the table yet. A record left behind would
    /// draw quads gone or another mesh's; a tile drawn while it fades would pop in; one missing
    /// from `listed` would leave its water, or all of it, undrawn.
    #[cfg(debug_assertions)]
    pub(crate) fn check(&self, tiles: &FxHashMap<TileKey, Tile>, now: Instant, gpu_on: bool) {
        let Some(table) = &self.table else {
            assert!(self.tracked.is_empty(), "records of a dropped table");
            assert!(
                gpu_on || self.listed_is_empty(),
                "listed of GPU culling off"
            );
            return;
        };
        let (records, waits) = table.checked();
        let waits: Vec<_> = waits.collect();
        let (mut live, mut flagged) = (0, 0);
        for (&key, tile) in tiles {
            let listed = match self.tracked.get(&key) {
                Some(tracked) => tracked.fading || tile.mesh.translucent > 0,
                None => true,
            };
            assert_eq!(self.listed.contains(key), listed, "{key:?}: listed");
            let Some(tracked) = self.tracked.get(&key) else {
                let taken = SourcedGpu::record(&tile.gpu_mesh(key), false).is_some();
                assert!(tile.mesh.solid == 0 || !taken, "{key:?}: a record");
                continue;
            };
            let record = SourcedGpu::record(&tile.gpu_mesh(key), tracked.fading);
            assert_eq!(
                records.at(tracked.id),
                record.as_ref(),
                "{key:?}: its record"
            );
            assert!(
                tracked.fading || tile.steady(key, now),
                "{key:?}: drawn by the pass while not whole"
            );
            assert!(
                !tracked.fading || tile.whole_at().is_none() || waits.contains(&cell(key)),
                "{key:?}: flagged, with no end waited for"
            );
            live += 1;
            flagged += u32::from(tracked.fading);
        }
        assert_eq!(records.live(), live, "records of tiles gone");
        assert_eq!(records.fading(), flagged, "flagged records");
    }

    /// Whether nothing is listed (`check`, for the empty table's word).
    #[cfg(debug_assertions)]
    fn listed_is_empty(&self) -> bool {
        self.listed.is_empty()
    }
}

#[cfg(test)]
mod tests;
