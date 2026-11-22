//! Frozen mesh visibility plans (`section_visibility`, docs/design/mod-api/mesh-visibility.md).
//!
//! The world thread captures a section's raw input ([`capture_input`]), asks the mods, combines
//! their checked answers in a [`SectionPlanBuilder`] and freezes one [`SectionVisibilityPlan`]
//! per section with its one-block border from the neighbours' answers. A meshing worker only
//! reads that plan: no mod code, world or host reaches it. Without a plan the mesher runs its
//! `NoMasks` body, in which every step below is a constant and compiles away.
use super::{ALL_VISIBLE, B, Mesher, P, Region, SectionMesh, pi};
use crate::chunk::Chunk;
use mod_api::{
    BlockState, SectionCell, SectionVisibility, SectionVisibilityInput, SectionVisibilityRefusal,
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The face bits of a cell's kept bits: down, up, north, south, west, east (`Direction` order).
pub(super) const FACES: u8 = 0b0011_1111;
/// The kept bit of a cell's block model.
pub(super) const MODEL: u8 = 1 << 6;
/// The kept bit of a cell's fluid.
pub(super) const FLUID: u8 = 1 << 7;
/// Everything kept: a cell no mod changed.
const ALL: u8 = 0xFF;

/// The maximum bytes held by a combined answer's payload.
pub const ANSWER_BYTES: usize = 4096 + std::mem::size_of::<SectionAnswer>();
/// The bytes a section's frozen [`SectionVisibilityPlan`] holds.
pub const PLAN_BYTES: usize = P * P * P;

/// Bytes of visibility storage charged to the same shared in-flight pool as section geometry.
/// The bytes are returned on drop, or split off into a longer-lived lease.
#[derive(Debug)]
pub struct Lease {
    shared: &'static AtomicUsize,
    bytes: usize,
}

impl Lease {
    /// Charges `bytes` to the workers' shared in-flight bytes, or `None` at the global limit: the
    /// caller waits for room.
    pub fn charge(bytes: usize) -> Option<Self> {
        Self::charge_in(&super::mod_geometry::IN_FLIGHT, bytes)
    }
    /// Move part of a charge to another owner without competing for capacity again.
    pub(crate) fn split(&mut self, bytes: usize) -> Self {
        self.bytes -= bytes;
        Self {
            shared: self.shared,
            bytes,
        }
    }
    pub(crate) fn charge_in(shared: &'static AtomicUsize, bytes: usize) -> Option<Self> {
        super::mod_geometry::reserve_in_flight(shared, bytes).then(|| Self { shared, bytes })
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.shared.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// Resident answers are charged to that pool too, but can never consume its transient headroom.
pub(crate) const MAX_RESIDENT_VISIBILITY_BYTES: usize =
    super::mod_geometry::MAX_IN_FLIGHT_BYTES * 3 / 4;

/// What the mesher reads of a section's visibility, by padded cell index (`pi`).
pub(super) trait CellMasks {
    /// False for [`NoMasks`]: every visibility step is skipped at compile time.
    const ACTIVE: bool;
    /// The kept bits ([`FACES`], [`MODEL`], [`FLUID`]) of padded cell `padded`.
    fn at(&self, padded: usize) -> u8;
}

/// The no-mod path: everything kept, nothing read.
pub(super) struct NoMasks;
impl CellMasks for NoMasks {
    const ACTIVE: bool = false;
    #[inline(always)]
    fn at(&self, _: usize) -> u8 {
        ALL
    }
}

/// One section's frozen visibility, made on the world thread before the section is queued and
/// only read by the worker that meshes it: whether its terrain is hidden, and the kept bits of
/// its cells and of the one-block border its neighbours' answers decide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionVisibilityPlan {
    hide_terrain: bool,
    /// Kept bits by padded cell index; cells beyond the one-block border are [`ALL`].
    cells: Box<[u8]>,
}

impl CellMasks for SectionVisibilityPlan {
    const ACTIVE: bool = true;
    #[inline]
    fn at(&self, padded: usize) -> u8 {
        self.cells[padded]
    }
}

impl SectionVisibilityPlan {
    /// Whether a mod answered `HideTerrain` for the section.
    pub fn hides_terrain(&self) -> bool {
        self.hide_terrain
    }
    /// The kept bits at `local` (-1 to 16 on each axis): bits 0 to 5 the faces, 6 the model, 7
    /// the fluid. [`ALL`] (255) where nothing changed.
    pub fn kept(&self, local: [i32; 3]) -> u8 {
        debug_assert!(local.iter().all(|v| (-1..=16).contains(v)));
        self.cells[pi(local[0], local[1], local[2])]
    }
}

/// One section's own combined answer, the centre's 4096 cells: what the world thread keeps
/// between meshes to give the neighbours their border.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionAnswer {
    hide_terrain: bool,
    /// Kept bits by `y << 8 | z << 4 | x`.
    cells: Box<[u8; 4096]>,
}

impl SectionAnswer {
    /// Whether the section's terrain is hidden.
    pub fn hides_terrain(&self) -> bool {
        self.hide_terrain
    }
    /// The kept bits at `local`, 0 to 15 on each axis: none of a hidden section's.
    fn at(&self, [x, y, z]: [i32; 3]) -> u8 {
        if self.hide_terrain {
            return 0;
        }
        self.cells[(y << 8 | z << 4 | x) as usize]
    }
    /// Whether the cells on the outside of the section differ between two answers (`None` keeps
    /// everything): the neighbours' plans read them, so they must mesh again.
    pub fn border_differs(a: Option<&Self>, b: Option<&Self>) -> bool {
        if a == b {
            return false;
        }
        let at = |answer: Option<&Self>, p| answer.map_or(ALL, |s| s.at(p));
        for y in 0..16 {
            for z in 0..16 {
                let inner_row = (1..15).contains(&y) && (1..15).contains(&z);
                let step = if inner_row { 15 } else { 1 };
                for x in (0..16).step_by(step) {
                    if at(a, [x, y, z]) != at(b, [x, y, z]) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// The mods' checked answers for one section, combined by intersection: what one hides no later
/// answer brings back, and `HideTerrain` wins over every cell answer.
pub struct SectionPlanBuilder {
    hide_terrain: bool,
    cells: Box<[u8; 4096]>,
    changed: bool,
}

impl Default for SectionPlanBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SectionPlanBuilder {
    /// Nothing changed yet.
    pub fn new() -> Self {
        Self {
            hide_terrain: false,
            cells: Box::new([ALL; 4096]),
            changed: false,
        }
    }
    /// Merge one native listener's answer, only after checking every record.
    pub fn apply(&mut self, answer: &SectionVisibility) -> Result<(), SectionVisibilityRefusal> {
        answer.check()?;
        if let Some(cells) = answer.cells() {
            for cell in cells {
                self.restrict(cell.pos, cell.model, cell.fluid, cell.faces);
            }
        } else {
            self.hide_terrain();
        }
        Ok(())
    }
    /// A mod's `HideTerrain`.
    pub fn hide_terrain(&mut self) {
        self.hide_terrain = true;
        self.changed = true;
    }
    /// A mod's checked cell record: `pos` 0 to 15 on each axis, `faces` at most 63 (the answer's
    /// own check refused anything else before this).
    pub fn restrict(&mut self, pos: [u8; 3], model: bool, fluid: bool, faces: u8) {
        let [x, y, z] = pos.map(usize::from);
        debug_assert!(x < 16 && y < 16 && z < 16 && faces <= FACES);
        let kept = faces & FACES | if model { MODEL } else { 0 } | if fluid { FLUID } else { 0 };
        let cell = &mut self.cells[y << 8 | z << 4 | x];
        if *cell & kept != *cell {
            *cell &= kept;
            self.changed = true;
        }
    }
    /// The combined answer; `None` when it changes nothing, the section's no-plan path.
    pub fn finish(self) -> Option<SectionAnswer> {
        self.changed.then_some(SectionAnswer {
            hide_terrain: self.hide_terrain,
            cells: self.cells,
        })
    }
}

/// Freezes the plan of a section from its own answer and, for its one-block border, its 26
/// neighbours' answers (`neighbour([dx, dy, dz])`, each -1 to 1). `None` when neither changes
/// anything: the section meshes on the no-plan path, as it would with no mod.
pub fn plan_section<'a>(
    own: Option<&SectionAnswer>,
    mut neighbour: impl FnMut([i32; 3]) -> Option<&'a SectionAnswer>,
) -> Option<SectionVisibilityPlan> {
    let mut cells: Option<Box<[u8]>> = None;
    let mut hide_terrain = false;
    let set = |cells: &mut Option<Box<[u8]>>, local: [i32; 3], kept: u8| {
        if kept != ALL {
            let cells = cells.get_or_insert_with(|| vec![ALL; P * P * P].into_boxed_slice());
            cells[pi(local[0], local[1], local[2])] = kept;
        }
    };
    if let Some(own) = own {
        hide_terrain = own.hide_terrain;
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    set(&mut cells, [x, y, z], own.at([x, y, z]));
                }
            }
        }
    }
    let mut answers: [[[Option<&SectionAnswer>; 3]; 3]; 3] = Default::default();
    let mut any = false;
    for (dy, layer) in answers.iter_mut().enumerate() {
        for (dz, row) in layer.iter_mut().enumerate() {
            for (dx, slot) in row.iter_mut().enumerate() {
                if [dx, dy, dz] != [1, 1, 1] {
                    *slot = neighbour([dx as i32 - 1, dy as i32 - 1, dz as i32 - 1]);
                    any |= slot.is_some();
                }
            }
        }
    }
    if any {
        for y in -1i32..=16 {
            for z in -1i32..=16 {
                let border_row = !(0..16).contains(&y) || !(0..16).contains(&z);
                for x in -1i32..=16 {
                    if !border_row && (0..16).contains(&x) {
                        continue;
                    }
                    let section = [x, y, z].map(|v| v.div_euclid(16));
                    let [sx, sy, sz] = section.map(|v| (v + 1) as usize);
                    if let Some(answer) = answers[sy][sz][sx] {
                        let local = [x, y, z].map(|v| v.rem_euclid(16));
                        set(&mut cells, [x, y, z], answer.at(local));
                    }
                }
            }
        }
    }
    if cells.is_none() && !hide_terrain {
        return None;
    }
    Some(SectionVisibilityPlan {
        hide_terrain,
        cells: cells.unwrap_or_else(|| vec![ALL; P * P * P].into_boxed_slice()),
    })
}

/// Whether a cell with kept bits `kept` still occludes through its face `toward` (the face
/// toward the cell asking): its model must draw and keep that face.
#[inline]
pub(super) fn occludes(kept: u8, toward: u8) -> bool {
    kept & MODEL != 0 && kept >> toward & 1 != 0
}

/// Takes the faces a cell's answer removed from its model's `visible` faces: a mask only takes
/// away, after the normal culling.
#[inline]
pub(super) fn keep_faces(kept: u8, visible: &mut [bool; 6]) {
    for (d, visible) in visible.iter_mut().enumerate() {
        *visible &= kept >> d & 1 != 0;
    }
}

/// After the solid rows are read: a cell with anything taken away is no longer solid, so the
/// section's see-through graph and the enclosed-cell skip err toward drawing what is behind it.
pub(super) fn open_masked(masks: &impl CellMasks, rows: &mut [u32]) {
    for y in -1i32..=16 {
        for z in -1i32..=16 {
            let row = (y + B) as usize * P + (z + B) as usize;
            for x in -1i32..=16 {
                if masks.at(pi(x, y, z)) != ALL {
                    rows[row] &= !(1 << (x + B));
                }
            }
        }
    }
}

/// A section whose terrain a mod hides (`HideTerrain`): no model, fluid or section geometry
/// quads and nothing occluded, but its block entities and lights as the full mesh lists them.
pub(super) fn hidden_terrain(
    mesher: &Mesher<'_>,
    region: &[[Option<&Chunk>; 3]; 3],
    sy: i32,
    mut out: SectionMesh,
) -> SectionMesh {
    out.clear();
    let center = region[1][1].expect("centre chunk");
    let section = &center.sections[sy as usize];
    let base_y = center.min_y() + sy * 16;
    if !section.is_uniform(0) {
        let info = &mesher.tables.info;
        let mut own = Box::new([0; 4096]);
        section.unpack(&mut own);
        for (index, &id) in own.iter().enumerate() {
            let state = &info[id as usize];
            if id == 0
                || (!mesher.hidden_states.is_empty() && mesher.hidden_states[id as usize])
                || !(state.block_entity || state.emission > 0)
            {
                continue;
            }
            let (x, y, z) = (index as i32 & 15, index as i32 >> 8, index as i32 >> 4 & 15);
            let world = [center.x * 16 + x, base_y + y, center.z * 16 + z];
            if state.block_entity {
                out.block_entities.push((world, id));
            }
            if state.emission > 0 {
                out.lights.push((world, id));
            }
        }
    }
    out.visibility = ALL_VISIBLE;
    out.finish_sprite_layers();
    out
}

/// Lend native listeners a raw view captured from the same region the worker uses, before any
/// masks are applied. Above and below the world is air with the light the column gives it there.
pub fn capture_input(
    region: &Region,
    sy: i32,
    mut state_of: impl FnMut(u32) -> BlockState,
) -> SectionVisibilityInput {
    let centre = region[1][1].as_ref().expect("centre chunk");
    let base_y = centre.min_y() + sy * 16;
    SectionVisibilityInput::capture([centre.x * 16, base_y, centre.z * 16], |[x, y, z]| {
        let column = &region[(z.div_euclid(16) + 1) as usize][(x.div_euclid(16) + 1) as usize];
        let (lx, wy, lz) = (x.rem_euclid(16), base_y + y, z.rem_euclid(16));
        match column {
            Some(chunk) => (
                SectionCell::State(state_of(chunk.get(lx, wy, lz))),
                chunk.light(lx, wy, lz),
                Some(mod_api::__private::biome_id(chunk.biome(lx, wy, lz).into())),
            ),
            None => (SectionCell::Missing, 0, None),
        }
    })
}

#[cfg(test)]
#[path = "visibility_plan_mesh_tests.rs"]
mod mesh_tests;
#[cfg(test)]
#[path = "visibility_plan_tests.rs"]
mod tests;
