//! Far meshes share the terrain arenas and passes, with their actual bounds
//! rather than the 16³ boxes used by the near-section residency walk.
use super::{Frustum, Renderer, SectionGpu};
use crate::{
    fast_hash::{FxHashMap, FxHashSet},
    lod::{MAX_LEVEL, TileKey},
    mesher::SectionMesh,
};
use glam::{DVec3, Vec3};
use smallvec::SmallVec;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Finished tile meshes not uploaded yet, the tiles they replace (removed
/// once every mesh is in) and the columns they leave out.
#[derive(Default)]
pub(crate) struct Pending {
    meshes: FxHashMap<TileKey, SectionMesh>,
    /// The order they go up in: as handed over (`Lod::update`: finer tiles
    /// first, so no tile clipped around a finer one is drawn before it).
    order: VecDeque<TileKey>,
    removed: FxHashSet<TileKey>,
    cutout: Option<FxHashSet<(i32, i32)>>,
}

impl Pending {
    /// Takes in the next job, with the same result as applying both in
    /// turn: newer meshes replace waiting ones, tiles meshed again are not
    /// removed, and tiles removed now drop their waiting meshes.
    fn add(
        &mut self,
        meshes: Vec<(TileKey, SectionMesh)>,
        removed: Vec<TileKey>,
        cutout: Option<FxHashSet<(i32, i32)>>,
    ) {
        for (key, mesh) in meshes {
            self.removed.remove(&key);
            if self.meshes.insert(key, mesh).is_none() {
                self.order.push_back(key);
            }
        }
        for key in removed {
            self.meshes.remove(&key);
            self.removed.insert(key);
        }
        if cutout.is_some() {
            self.cutout = cutout;
        }
    }
}

/// Small, its meshes boxed: `draw_frame` walks every tile each frame (thousands at long far
/// distances), and that walk is bound by the memory it reads.
pub(crate) struct Tile {
    pub mesh: Box<SectionGpu>,
    bounds: Bounds,
    /// When it began to fade in; none while the rest of its job uploads.
    shown: Option<Instant>,
    /// When its replacements began to fade in; it goes once they have.
    dying: Option<Instant>,
    /// Dying with nothing to replace it (out of range): it fades out.
    alone: bool,
    /// Its mesh before near chunks took some of its columns, drawn in its
    /// place until they have faded in (from the job's end, the instant), so
    /// no hole shows through them.
    previous: Option<Box<(SectionGpu, Option<Instant>)>>,
    /// Its slot in occlusion culling's results (`cull::TILE`), kept while
    /// meshes of the same tile replace one another.
    pub cull_slot: u32,
    /// The animated sprite layers of both meshes it holds (`animated_layers`):
    /// the frame walk marks them as it lists the tile, without reaching into
    /// its boxed meshes.
    pub animated: SmallVec<[u16; 4]>,
}

/// The animated sprite layers a tile drawing `mesh`, or `previous` in its
/// place, can show: those of either.
fn animated_layers(mesh: &SectionGpu, previous: Option<&SectionGpu>) -> SmallVec<[u16; 4]> {
    let mut layers = SmallVec::from_slice(&mesh.animated_layers);
    for &layer in previous.iter().flat_map(|p| p.animated_layers.iter()) {
        if !layers.contains(&layer) {
            layers.push(layer);
        }
    }
    layers
}

/// Puts the tiles above `key` (its parent and theirs, up to `MAX_LEVEL`) in
/// `tiles`; one already there has its own above it.
fn insert_ancestors(tiles: &mut FxHashSet<TileKey>, mut key: TileKey) {
    while key.level < MAX_LEVEL && tiles.insert(key.parent()) {
        key = key.parent();
    }
}

/// The tiles that hold any of the chunk `columns`: each column's own level-0
/// tile and those above it.
fn holding_columns(columns: impl IntoIterator<Item = (i32, i32)>) -> FxHashSet<TileKey> {
    let mut tiles = FxHashSet::default();
    for (x, z) in columns {
        let key = TileKey { x, z, level: 0 };
        if tiles.insert(key) {
            insert_ancestors(&mut tiles, key);
        }
    }
    tiles
}

/// How long a tile takes to fade in over the ones it replaces.
pub(super) const FADE: Duration = Duration::from_millis(600);

/// A tile or section fading in: 0 drawn whole, else 1 + `level` * 129 +
/// `shown`, `shown` 1 to 63 sixty-fourths of `FADE` (`fade_out_code`: 65 to
/// 128, fading out). Chunk by chunk (columns of its own grid, wider for
/// coarse tiles: 2^`level` blocks), each starting in turn over the first half
/// and fading over the second, as a fine ordered pattern fixed to the terrain
/// (terrain.wgsl's `faded`). It is drawn a hair toward the camera, the newest
/// most, over the solid tiles it replaces, which stay until it is whole
/// (`upload_lods`): no pixel fought over, no hole. None: nothing to draw yet.
pub(super) fn fade_code(shown: Option<Instant>, level: u8, now: Instant) -> Option<u32> {
    let faded = now.saturating_duration_since(shown?).as_secs_f32() / FADE.as_secs_f32();
    let shown = (faded * 64.0) as u32 + 1;
    Some(if shown >= 64 {
        0
    } else {
        1 + u32::from(level) * 129 + shown
    })
}
/// A section past the render distance that far terrain is coming in under:
/// whole, a fading-out code at its start (terrain.wgsl's `fade_depth` draws
/// leaving sections in front of far terrain).
pub(super) const HELD: u32 = 1 + 64;
/// `fade_code` for one leaving since `since`, chunk by chunk as it came, over
/// what stays behind it; once `FADE` has passed it draws nothing.
pub(super) fn fade_out_code(since: Instant, level: u8, now: Instant) -> u32 {
    let faded = now.saturating_duration_since(since).as_secs_f32() / FADE.as_secs_f32();
    1 + u32::from(level) * 129 + 64 + ((faded * 64.0) as u32 + 1).min(64)
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Bounds {
    fn mesh(mesh: &SectionMesh) -> Option<Self> {
        let [min, max] = mesh.bounds?;
        Some(Self { min, max })
    }

    /// In view: the plan alone decides which tiles are in range (a circle),
    /// so a tile leaving it fades out (`upload_lods`). Cut here at the range,
    /// tiles popped out as the player moved, before the plan dropped them.
    fn visible(self, origin: DVec3, frustum: &Frustum) -> Option<u64> {
        let min = origin + self.min.as_dvec3();
        let max = origin + self.max.as_dvec3();
        if !frustum.intersects(min.as_vec3(), max.as_vec3()) {
            return None;
        }
        Some(self.distance(origin))
    }

    /// Its centre's squared distance, as bits: the order far tiles are drawn
    /// in. The same whether its box was tested or its cell's was.
    fn distance(self, origin: DVec3) -> u64 {
        let min = origin + self.min.as_dvec3();
        let max = origin + self.max.as_dvec3();
        ((min + max) * 0.5).length_squared().to_bits()
    }

    /// The box in world coordinates, around the tile's own corner.
    fn world(self, key: TileKey) -> (DVec3, DVec3) {
        let origin = DVec3::from(key.origin().map(f64::from));
        (origin + self.min.as_dvec3(), origin + self.max.as_dvec3())
    }
}

/// The width of a cell in blocks, along x and z.
const CELL: i32 = 128;

/// Far tiles grouped by coarse cell: one frustum test per cell every frame
/// covers the tiles in it, where each tile would take six planes of its own
/// (thousands of them at long distances, against a frame budget of a fraction
/// of a millisecond). A cell's box is the union of its tiles' world boxes, so
/// it holds every tile in it: a cell out of view leaves all of them out, and
/// one wholly in view saves each its own test (`Frustum::contains`).
///
/// Not a cache: it is kept in step with `Renderer::lods` by every method that
/// changes it, so it is never a frame behind, and the camera moving changes
/// nothing it holds.
#[derive(Default)]
pub(crate) struct TileGrid {
    cells: FxHashMap<(i32, i32), Cell>,
}

struct Cell {
    /// The union of its tiles' world boxes.
    min: DVec3,
    max: DVec3,
    /// Its tiles, and the boxes (tile-relative, as `Tile` keeps them) they
    /// hold it at.
    tiles: Vec<(TileKey, Bounds)>,
}

impl TileGrid {
    /// The cell a tile sits in, from the corner its mesh is laid out at: a
    /// tile's box can spill over, which only widens the cells it widens.
    fn cell_of(key: TileKey) -> (i32, i32) {
        let [x, _, z] = key.origin();
        (x.div_euclid(CELL), z.div_euclid(CELL))
    }

    /// Puts a tile in its cell, in the place of the one it may have had
    /// there: a tile meshed again can come back with other bounds.
    fn insert(&mut self, key: TileKey, bounds: Bounds) {
        self.remove(key);
        let (min, max) = bounds.world(key);
        let at = Self::cell_of(key);
        if let Some(cell) = self.cells.get_mut(&at) {
            cell.min = cell.min.min(min);
            cell.max = cell.max.max(max);
            cell.tiles.push((key, bounds));
        } else {
            self.cells.insert(
                at,
                Cell {
                    min,
                    max,
                    tiles: vec![(key, bounds)],
                },
            );
        }
    }

    /// Takes a tile out: its cell shrinks to the tiles left, and goes when
    /// there are none.
    fn remove(&mut self, key: TileKey) {
        let at = Self::cell_of(key);
        // Its box after the tile leaves: none when nothing is left in it.
        let mut left: Option<(DVec3, DVec3)> = None;
        if let Some(cell) = self.cells.get_mut(&at) {
            let Some(slot) = cell.tiles.iter().position(|&(tile, _)| tile == key) else {
                return;
            };
            cell.tiles.swap_remove(slot);
            // A cell holds a handful of tiles: the union is built again from
            // those left rather than kept with an edge count per tile.
            for &(tile, bounds) in &cell.tiles {
                let (min, max) = bounds.world(tile);
                left = Some(match left {
                    Some((low, high)) => (low.min(min), high.max(max)),
                    None => (min, max),
                });
            }
            if let Some((min, max)) = left {
                cell.min = min;
                cell.max = max;
            }
        }
        if left.is_none() {
            self.cells.remove(&at);
        }
    }

    /// Every tile gone at once (`clear_lods`, `clear_sections`).
    pub(super) fn clear(&mut self) {
        self.cells.clear();
    }

    /// Calls `tile` for each tile whose cell meets `view`, with whether the
    /// whole cell is inside it (then so is every tile's box).
    pub(super) fn walk(&self, camera: DVec3, view: &Frustum, mut tile: impl FnMut(bool, TileKey)) {
        for cell in self.cells.values() {
            let min = (cell.min - camera).as_vec3();
            let max = (cell.max - camera).as_vec3();
            let inside = view.contains(min, max);
            if !inside && !view.intersects(min, max) {
                continue;
            }
            for &(key, _) in &cell.tiles {
                tile(inside, key);
            }
        }
    }

    /// Each cell's place and box, for tests.
    #[cfg(test)]
    fn cells(&self) -> impl Iterator<Item = ((i32, i32), DVec3, DVec3)> + '_ {
        self.cells
            .iter()
            .map(|(&at, cell)| (at, cell.min, cell.max))
    }
}

/// A tile's instance: like a section's, with the UV step of merged faces.
/// A fading tile's step is negated past its `fade_code`, which the shaders
/// read back as the whole part (they take the step as `fract(abs(w))`).
pub(super) fn instance(origin: [f32; 3], fade: u32) -> [f32; 4] {
    let step = 1.0 / crate::mesher::TILED_UV_STEPS;
    [
        origin[0],
        origin[1],
        origin[2],
        if fade == 0 {
            step
        } else {
            -(fade as f32 + step)
        },
    ]
}

pub(crate) struct Visible {
    pub key: TileKey,
    pub distance: u64,
    pub origin: [f32; 3],
    pub fade: u32,
    /// Replaced, standing until its replacements are whole: its water goes
    /// after theirs (`Renderer::render`'s water tiers), so depth leaves it only
    /// where they have none yet.
    pub old: bool,
}

impl Tile {
    /// The mesh drawn this frame: the previous one while it stands in.
    pub fn drawn(&self) -> &SectionGpu {
        self.previous
            .as_deref()
            .map_or(&self.mesh, |(mesh, _)| mesh)
    }
    pub fn drawn_mut(&mut self) -> &mut SectionGpu {
        match self.previous.as_deref_mut() {
            Some((mesh, _)) => mesh,
            None => &mut self.mesh,
        }
    }
    /// The box occlusion culling tests (`cull::Cull`), around its origin: its
    /// mesh's bounds grown by a block, while that mesh is the one drawn
    /// whole (no stand-in, no fade). A fading tile's dither moves its depth
    /// toward the camera (terrain.wgsl's `fade_depth`), out of the box.
    pub fn cull_bounds(&self, fade: u32) -> Option<[[f32; 3]; 2]> {
        (fade == 0 && self.previous.is_none()).then(|| {
            [
                (self.bounds.min - 1.0).to_array(),
                (self.bounds.max + 1.0).to_array(),
            ]
        })
    }
    /// Whether either mesh it holds is in the compact layout: a stand-in drawn in its place can
    /// hide a compact replacement (`Renderer::follow_packed_layout`).
    pub fn holds_packed(&self) -> bool {
        self.mesh.packed || self.previous.as_ref().is_some_and(|p| p.0.packed)
    }
    /// Both meshes it holds in the arena: its own and the stand-in, if any.
    pub fn meshes_mut(&mut self) -> impl Iterator<Item = &mut SectionGpu> {
        std::iter::once(&mut *self.mesh).chain(self.previous.as_deref_mut().map(|(mesh, _)| mesh))
    }
    /// The tile as drawn this frame, if its own box is in view.
    pub fn visible(
        &self,
        key: TileKey,
        camera: DVec3,
        frustum: &Frustum,
        now: Instant,
    ) -> Option<Visible> {
        let at = DVec3::from(key.origin().map(f64::from)) - camera;
        let mut visible = self.draw_state(key, at, now)?;
        visible.distance = self.bounds.visible(at, frustum)?;
        Some(visible)
    }

    /// `visible` for a tile whose cell is wholly in view (`TileGrid::walk`):
    /// its own box is wholly in view too, so only its distance is left.
    pub fn visible_inside(&self, key: TileKey, camera: DVec3, now: Instant) -> Option<Visible> {
        let at = DVec3::from(key.origin().map(f64::from)) - camera;
        let mut visible = self.draw_state(key, at, now)?;
        visible.distance = self.bounds.distance(at);
        Some(visible)
    }

    /// The tile as drawn this frame, short of its distance: none while it is
    /// still fading in.
    fn draw_state(&self, key: TileKey, at: DVec3, now: Instant) -> Option<Visible> {
        let fade = match self.dying {
            Some(since) if self.alone => fade_out_code(since, key.level, now),
            _ => fade_code(self.shown, key.level, now)?,
        };
        Some(Visible {
            key,
            distance: 0,
            origin: at.as_vec3().to_array(),
            fade,
            old: self.dying.is_some() || self.previous.is_some(),
        })
    }
}

impl Renderer {
    /// A far tile in, with its cell. `lods` and `lod_cells` change only here
    /// and in `take_lod_tile`, so the cells never name a tile that is gone and
    /// never miss one that is there (the frame walk trusts them).
    fn add_lod_tile(&mut self, key: TileKey, tile: Tile) {
        self.lod_cells.insert(key, tile.bounds);
        self.lod_vertex_bytes += tile.mesh.vertex_bytes();
        if let Some(old) = self.lods.insert(key, tile) {
            self.lod_vertex_bytes -= old.mesh.vertex_bytes();
        }
    }

    /// A far tile out, with its cell.
    fn take_lod_tile(&mut self, key: TileKey) -> Option<Tile> {
        let tile = self.lods.remove(&key)?;
        self.lod_cells.remove(key);
        self.lod_vertex_bytes -= tile.mesh.vertex_bytes();
        Some(tile)
    }

    /// A tile meshed again (finer detail) takes its place at once, but for
    /// one meshed around near chunks or finer tiles that are new (`keep`):
    /// its old mesh stands in while they fade in. A new one waits for the rest of its job
    /// (`upload_lods`), then fades in.
    pub fn upload_lod(&mut self, key: TileKey, mesh: &SectionMesh, keep: bool) {
        if let Some(copies) = &mut self.capture_copies {
            copies.tile(key, mesh);
        }
        let old_slot = self.lods.get(&key).map(|tile| tile.cull_slot);
        let (shown, previous) = match self.take_lod_tile(key) {
            Some(old) if old.dying.is_none() => match old.previous {
                Some(previous) => {
                    self.release_mesh(*old.mesh);
                    (old.shown, Some(previous))
                }
                None if keep => (old.shown, Some(Box::new((*old.mesh, None)))),
                None => {
                    self.release_mesh(*old.mesh);
                    (old.shown, None)
                }
            },
            Some(old) => {
                self.release_tile(old);
                (None, None)
            }
            None => (None, None),
        };
        match Bounds::mesh(mesh).zip(self.upload_mesh(mesh)) {
            Some((bounds, mesh)) => {
                let cull_slot = old_slot.unwrap_or_else(|| {
                    self.lod_cull_free.pop().unwrap_or_else(|| {
                        self.lod_cull_next += 1;
                        self.lod_cull_next - 1
                    })
                });
                let animated = animated_layers(&mesh, previous.as_deref().map(|(p, _)| p));
                self.add_lod_tile(
                    key,
                    Tile {
                        animated,
                        mesh: Box::new(mesh),
                        bounds,
                        shown,
                        dying: None,
                        alone: false,
                        previous,
                        cull_slot,
                    },
                );
            }
            None => {
                if let Some((previous, _)) = previous.map(|p| *p) {
                    self.release_mesh(previous);
                }
                self.lod_cull_free.extend(old_slot);
            }
        }
    }

    fn release_tile(&mut self, tile: Tile) {
        self.release_mesh(*tile.mesh);
        if let Some((previous, _)) = tile.previous.map(|p| *p) {
            self.release_mesh(previous);
        }
    }

    pub fn remove_lod(&mut self, key: TileKey) {
        if let Some(copies) = &mut self.capture_copies {
            copies.forget_tile(key);
        }
        if let Some(tile) = self.take_lod_tile(key) {
            self.lod_cull_free.push(tile.cull_slot);
            self.release_tile(tile);
        }
    }

    pub fn clear_lods(&mut self) {
        if let Some(copies) = &mut self.capture_copies {
            copies.clear_tiles();
        }
        self.lod_pending = None;
        // No far terrain left to fade out over.
        for (key, since) in std::mem::take(&mut self.lod_leaving) {
            if self
                .sections
                .find(key)
                .is_some_and(|s| s.leaving == Some(since))
            {
                self.release_section(key);
            }
        }
        self.lod_vertex_bytes = 0;
        for (_, tile) in std::mem::take(&mut self.lods) {
            self.release_tile(tile);
        }
        self.lod_cells.clear();
        self.lod_cull_free.clear();
        self.lod_cull_next = 0;
        self.set_lod_cutout(Default::default());
    }

    /// Queues a finished mesh job (`Lod::update`, `Lod::removed`,
    /// `Lod::take_cutout`) for `upload_lods`, joining one still uploading
    /// (`Pending::add`).
    pub fn queue_lods(
        &mut self,
        meshes: Vec<(TileKey, SectionMesh)>,
        removed: Vec<TileKey>,
        cutout: Option<FxHashSet<(i32, i32)>>,
    ) {
        if meshes.is_empty() && removed.is_empty() && cutout.is_none() {
            return;
        }
        self.lod_pending
            .get_or_insert_default()
            .add(meshes, removed, cutout);
    }

    /// Uploads queued tile meshes for about `budget` (at least one), then,
    /// once all are in, fades them in over the tiles they replace.
    pub fn upload_lods(&mut self, budget: Duration) {
        let start = Instant::now();
        if self.lod_pending.is_some() {
            // Far tiles fill the arenas too: large translucent pages wait for them.
            self.shrink_gate.wait(start, super::SHRINK_SETTLE);
        }
        self.release_left(start);
        // Tiles go dying and stand-ins get their time only as a job ends, so
        // once the last job's fade is over no tile is left to look at.
        if let Some(settled) = self.lod_settled {
            self.release_faded(start);
            if start >= settled {
                self.lod_settled = None;
            }
        }
        if self.lod_pending.is_none() {
            return;
        }
        // The columns near chunks take in this job, and the tiles holding the
        // job's new finer ones: a tile meshed again around them keeps its
        // old mesh until they are in, so no hole shows through them.
        let mut around_new = FxHashSet::default();
        let new = self
            .lod_pending
            .iter()
            .flat_map(|pending| pending.meshes.keys())
            .filter(|key| !self.lods.contains_key(key))
            .chain(
                self.lods
                    .iter()
                    .filter(|(_, tile)| tile.shown.is_none())
                    .map(|(key, _)| key),
            );
        for &key in new {
            insert_ancestors(&mut around_new, key);
        }
        let taken = holding_columns(
            self.lod_pending
                .iter()
                .flat_map(|pending| pending.cutout.iter().flatten())
                .filter(|column| !self.lod_cutout.contains(column))
                .copied(),
        );
        while let Some(pending) = &mut self.lod_pending {
            // Keys meshed again keep their place; removed ones are skipped.
            let next = std::iter::from_fn(|| pending.order.pop_front())
                .find(|key| pending.meshes.contains_key(key));
            let Some(key) = next else {
                let pending = self.lod_pending.take().expect("pending tiles");
                // A tile fades in over the ones it replaces. One over nothing
                // shows whole: its hidden chunks would open holes to the sky
                // or the sea bed (dark patches, as far terrain first loads).
                let replaced: FxHashSet<TileKey> = pending
                    .removed
                    .iter()
                    .copied()
                    .filter(|key| self.lods.contains_key(key))
                    .collect();
                let mut above_replaced = FxHashSet::default();
                for &key in &replaced {
                    insert_ancestors(&mut above_replaced, key);
                }
                let over_replaced = |key: TileKey| {
                    let mut ancestor = key;
                    replaced.contains(&key)
                        || above_replaced.contains(&key)
                        || (key.level..MAX_LEVEL).any(|_| {
                            ancestor = ancestor.parent();
                            replaced.contains(&ancestor)
                        })
                };
                // A removed tile no new tile overlaps, and no near chunk takes
                // a column of, leaves the plan (out of range): it fades out.
                // Over near chunks fading in it stays whole: two patterns
                // fading at once left holes between them.
                // Two tiles overlap when one is the other or its ancestor:
                // `kept` holds the new tiles and the near chunks' columns
                // (level 0), `above` their ancestors.
                let mut kept: FxHashSet<TileKey> = self
                    .lods
                    .iter()
                    .filter(|(_, tile)| tile.shown.is_none())
                    .map(|(&key, _)| key)
                    .collect();
                let cut = pending.cutout.as_ref().unwrap_or(&self.lod_cutout);
                kept.extend(cut.iter().map(|&(x, z)| TileKey { x, z, level: 0 }));
                let mut above = FxHashSet::default();
                for &key in &kept {
                    insert_ancestors(&mut above, key);
                }
                let alone = |key: TileKey| {
                    let mut ancestor = key;
                    !kept.contains(&key)
                        && !above.contains(&key)
                        && !(key.level..MAX_LEVEL).any(|_| {
                            ancestor = ancestor.parent();
                            kept.contains(&ancestor)
                        })
                };
                let whole = start.checked_sub(FADE).unwrap_or(start);
                for (&key, tile) in &mut self.lods {
                    if tile.shown.is_none() {
                        tile.shown = Some(if over_replaced(key) { start } else { whole });
                    }
                    if let Some((_, since)) = tile.previous.as_deref_mut() {
                        since.get_or_insert(start);
                    }
                }
                for key in pending.removed {
                    let alone = alone(key);
                    if let Some(tile) = self.lods.get_mut(&key)
                        && tile.dying.is_none()
                    {
                        tile.dying = Some(start);
                        tile.alone = alone;
                    }
                }
                if let Some(cutout) = pending.cutout {
                    self.set_lod_cutout(cutout);
                }
                self.lod_settled = Some(start + FADE);
                return;
            };
            let mesh = pending.meshes.remove(&key).expect("pending tile");
            let keep = mesh.keep || around_new.contains(&key) || taken.contains(&key);
            self.upload_lod(key, &mesh, keep);
            if start.elapsed() >= budget {
                return;
            }
        }
    }

    /// Releases the tiles that have faded out and the stand-ins whose
    /// replacements have faded in, `FADE` after their job ended.
    fn release_faded(&mut self, start: Instant) {
        let faded: Vec<_> = self
            .lods
            .iter()
            .filter(|(_, tile)| tile.dying.is_some_and(|d| start - d >= FADE))
            .map(|(&key, _)| key)
            .collect();
        for key in faded {
            self.remove_lod(key);
        }
        let stood_in: Vec<_> = self
            .lods
            .values_mut()
            .filter_map(|tile| match tile.previous.as_deref() {
                Some((_, Some(since))) if start - *since >= FADE => {
                    tile.animated = animated_layers(&tile.mesh, None);
                    tile.previous.take().map(|p| *p)
                }
                _ => None,
            })
            .collect();
        for (previous, _) in stood_in {
            self.release_mesh(previous);
        }
    }

    /// The tiles drawn for good, not fading out (`render-capture`).
    pub(super) fn standing_lods(&self) -> FxHashSet<TileKey> {
        self.lods
            .iter()
            .filter(|(_, tile)| tile.dying.is_none())
            .map(|(&key, _)| key)
            .collect()
    }

    /// The chunk columns the resident tiles now leave out (`Lod::take_cutout`).
    /// Removed sections they no longer leave out are released.
    pub fn set_lod_cutout(&mut self, cutout: crate::fast_hash::FxHashSet<(i32, i32)>) {
        self.lod_cutout = cutout;
        let released: Vec<_> = self
            .lod_retiring
            .iter()
            .copied()
            .filter(|key| !self.lod_cutout.contains(&(key.0, key.2)))
            .collect();
        for key in released {
            self.lod_retiring.remove(&key);
            self.leave_section(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::TerrainVertex;

    #[test]
    fn tiles_fade_in_on_their_own_grid() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        // terrain.wgsl's `faded` reads the level and sixty-fourths back.
        let decode = |code: u32| ((code - 1) / 129, (code - 1) % 129);
        assert_eq!(fade_code(None, 0, at(0)), None, "waits for its job");
        assert_eq!(fade_code(Some(start), 3, at(0)).map(decode), Some((3, 1)));
        let mut last = 0;
        for ms in [0, 100, 300, 500, 590] {
            let (level, shown) = decode(fade_code(Some(start), 10, at(ms)).unwrap());
            assert_eq!(level, 10);
            assert!(shown > last && shown < 64, "{shown} at {ms} ms");
            last = shown;
        }
        assert_eq!(fade_code(Some(start), 10, at(600)), Some(0), "whole");
        // Fading out: 65 to 128, then nothing drawn (128) until released.
        let mut last = 64;
        for ms in [0, 100, 300, 500, 590] {
            let (level, out) = decode(fade_out_code(start, 3, at(ms)));
            assert_eq!(level, 3);
            assert!(out > last && out < 128, "{out} at {ms} ms");
            last = out;
        }
        assert_eq!(decode(fade_out_code(start, 3, at(700))), (3, 128));
        // A section's largest code keeps its UV step exact (terrain_uv).
        let w = 129.0f32 + 1.0 / 32768.0;
        assert_eq!(w.fract(), 1.0 / 32768.0);
    }

    #[test]
    fn joined_jobs_apply_as_if_in_turn() {
        let key = |x| TileKey { x, z: 0, level: 0 };
        let mut pending = Pending::default();
        // The first job meshes 1 and 2 and retires 3 and 4.
        pending.add(
            vec![
                (key(1), SectionMesh::default()),
                (key(2), SectionMesh::default()),
            ],
            vec![key(3), key(4)],
            Some([(0, 0)].into_iter().collect()),
        );
        // The second meshes 3 again and retires 2.
        pending.add(vec![(key(3), SectionMesh::default())], vec![key(2)], None);
        let mut meshes: Vec<_> = pending.meshes.keys().map(|k| k.x).collect();
        meshes.sort_unstable();
        let mut removed: Vec<_> = pending.removed.iter().map(|k| k.x).collect();
        removed.sort_unstable();
        assert_eq!((meshes, removed), (vec![1, 3], vec![2, 4]));
        assert!(pending.cutout.as_ref().is_some_and(|c| c.contains(&(0, 0))));
        // They go up in the order handed over, each once.
        pending.add(vec![(key(1), SectionMesh::default())], vec![], None);
        let order: Vec<_> = pending
            .order
            .iter()
            .filter(|k| pending.meshes.contains_key(k))
            .map(|k| k.x)
            .collect();
        assert_eq!(order, [1, 3]);
    }

    #[test]
    fn large_tile_bounds_include_water_and_do_not_cull_by_origin() {
        let vertex = |pos| TerrainVertex::tiled(pos, [0.0; 2], 0, 0, 0);
        let mesh = SectionMesh {
            solid: vec![vertex([0.0, -64.0, 0.0]), vertex([1024.0, 100.0, 1024.0])],
            translucent: vec![vertex([512.0, 120.0, 512.0])],
            bounds: Some([Vec3::new(0.0, -64.0, 0.0), Vec3::new(1024.0, 120.0, 1024.0)]),
            ..Default::default()
        };
        let bounds = Bounds::mesh(&mesh).unwrap();
        assert_eq!(bounds.min, Vec3::new(0.0, -64.0, 0.0));
        assert_eq!(bounds.max, Vec3::new(1024.0, 120.0, 1024.0));
        let view = Frustum::from_planes(super::super::frustum(
            super::super::world_projection(70.0, 16.0 / 9.0, 8192.0)
                * crate::camera::view_matrix(0.0, 0.0),
        ));
        // The tile origin is far behind, but its far edge reaches the camera.
        // Negative world origins therefore remain visible.
        assert!(
            bounds
                .visible(DVec3::new(-1000.0, -80.0, -512.0), &view)
                .is_some()
        );
        // Wholly behind the camera: out of view.
        assert!(
            bounds
                .visible(DVec3::new(-512.0, -80.0, -2000.0), &view)
                .is_none()
        );
        assert!(Bounds::mesh(&SectionMesh::default()).is_none());
    }

    /// The view the grid tests are written against: the camera at the origin,
    /// looking down +z, as vanilla's `view_matrix(0, 0)` does.
    fn camera_view() -> Frustum {
        Frustum::from_planes(super::super::frustum(
            super::super::world_projection(70.0, 16.0 / 9.0, 8192.0)
                * crate::camera::view_matrix(0.0, 0.0),
        ))
    }

    fn tile_box(min: [f32; 3], max: [f32; 3]) -> Bounds {
        Bounds {
            min: Vec3::from(min),
            max: Vec3::from(max),
        }
    }

    /// The tiles holding a set of chunk columns are those whose block square
    /// holds one of the columns' corners, at every level.
    #[test]
    fn tiles_holding_columns_are_those_over_them() {
        let columns = [(-1, 5), (0, 0), (37, -70)];
        let tiles = holding_columns(columns);
        let mut held = 0;
        for level in 0..=MAX_LEVEL {
            for x in -6..6 {
                for z in -6..6 {
                    let key = TileKey { x, z, level };
                    let [x0, _, z0] = key.origin();
                    let size = key.size();
                    let over = columns.iter().any(|&(cx, cz)| {
                        (x0..x0 + size).contains(&(cx * 16)) && (z0..z0 + size).contains(&(cz * 16))
                    });
                    assert_eq!(tiles.contains(&key), over, "{key:?}");
                    held += usize::from(over);
                }
            }
        }
        // The first two columns are held at every level, (37, -70) within the
        // range above only by coarser tiles.
        assert!(held > 2 * (usize::from(MAX_LEVEL) + 1));
        assert!(tiles.contains(&TileKey {
            x: 37,
            z: -70,
            level: 0
        }));
    }

    /// A tile its cell vouches for gets the same distance, and so the same
    /// place in the draw order, as one tested on its own box.
    #[test]
    fn a_vouched_for_tile_takes_the_same_distance() {
        let view = camera_view();
        let bounds = tile_box([0.0, -64.0, 0.0], [1024.0, 120.0, 1024.0]);
        for origin in [
            DVec3::new(-1000.0, -80.0, -512.0),
            DVec3::new(0.0, 100.0, 300.0),
            DVec3::new(-64.0, 0.0, 0.0),
        ] {
            assert_eq!(bounds.visible(origin, &view), Some(bounds.distance(origin)));
        }
    }

    /// A cell's box holds every tile in it (a tile it did not hold would be
    /// dropped unseen), shrinks to the tiles left when one goes, and goes with
    /// the last of them.
    #[test]
    fn a_cell_holds_the_boxes_of_its_tiles() {
        let key = |x, z| TileKey { x, z, level: 0 };
        let mut grid = TileGrid::default();
        // Two level-0 tiles in the cell at (0, 0), 128 blocks wide: the second
        // 16 along, its box reaching past its own corner.
        grid.insert(key(0, 0), tile_box([0.0, -64.0, 0.0], [16.0, 120.0, 16.0]));
        grid.insert(
            key(1, 0),
            tile_box([-2.0, -64.0, -2.0], [18.0, 320.0, 18.0]),
        );
        // A third in the next cell along.
        grid.insert(key(8, 0), tile_box([0.0, -64.0, 0.0], [16.0, 64.0, 16.0]));
        let box_of = |grid: &TileGrid, at: (i32, i32)| {
            grid.cells()
                .find(|&(cell, ..)| cell == at)
                .map(|(_, min, max)| (min, max))
        };
        assert_eq!(
            box_of(&grid, (0, 0)),
            Some((DVec3::new(0.0, -64.0, -2.0), DVec3::new(34.0, 320.0, 18.0)))
        );
        assert_eq!(
            box_of(&grid, (1, 0)),
            Some((DVec3::new(128.0, -64.0, 0.0), DVec3::new(144.0, 64.0, 16.0)))
        );
        // The overflowing tile goes: the cell is the one left over.
        grid.remove(key(1, 0));
        assert_eq!(
            box_of(&grid, (0, 0)),
            Some((DVec3::new(0.0, -64.0, 0.0), DVec3::new(16.0, 120.0, 16.0)))
        );
        grid.remove(key(0, 0));
        assert_eq!(box_of(&grid, (0, 0)), None, "the last tile takes it");
        assert!(box_of(&grid, (1, 0)).is_some(), "another cell stays");
    }

    /// The frustum is asked once per cell: a cell wholly in view vouches for
    /// its tiles, one that only crosses it hands them back for their own test,
    /// and one it misses leaves them out.
    #[test]
    fn a_cell_is_tested_once_for_all_its_tiles() {
        let key = |x, z| TileKey { x, z, level: 0 };
        let view = camera_view();
        let tile = tile_box([0.0, -64.0, 0.0], [16.0, 64.0, 16.0]);
        let mut grid = TileGrid::default();
        // On the axis 176 blocks in front of the camera.
        let inside = key(-1, 11);
        // Behind it.
        let behind = key(-1, -4);
        // Across the screen's right edge: 50 blocks out on its near side, 400
        // on its far side, over 100 blocks of depth.
        let crossing = key(3, 0);
        let edge = tile_box([2.0, -64.0, 0.0], [352.0, 64.0, 100.0]);
        grid.insert(inside, tile);
        grid.insert(behind, tile);
        grid.insert(crossing, edge);
        let seen_box = |key: TileKey, bounds: Bounds| {
            let (min, max) = bounds.world(key);
            (min.as_vec3(), max.as_vec3())
        };
        let (min, max) = seen_box(inside, tile);
        assert!(view.contains(min, max), "the box 176 in is wholly in view");
        let (min, max) = seen_box(behind, tile);
        assert!(!view.intersects(min, max), "the box behind is out of view");
        let (min, max) = seen_box(crossing, edge);
        assert!(
            view.intersects(min, max) && !view.contains(min, max),
            "the box across the edge is neither in nor out"
        );
        let mut seen = Vec::new();
        grid.walk(DVec3::ZERO, &view, |inside, at| seen.push((inside, at)));
        seen.sort_unstable();
        assert_eq!(seen, [(false, crossing), (true, inside)]);
    }
}
