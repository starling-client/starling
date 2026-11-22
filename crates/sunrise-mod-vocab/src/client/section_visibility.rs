//! Per-section and per-cell mesh visibility (`section_visibility`, the mesh visibility design):
//! the raw section a client mod reads and the frozen answer it gives before the section meshes.
//!
//! The game captures the input and asks on the client world thread, never on a meshing worker; the
//! workers read only the checked answers.
use crate::{BiomeId, BlockState};

/// Cells along each axis of a [`SectionVisibilityInput`]: the section's 16 and one each side.
pub const SECTION_INPUT_SIDE: usize = 18;
/// Cells in a [`SectionVisibilityInput`]: 18 cubed, edges and corners included.
pub const SECTION_INPUT_CELLS: usize = SECTION_INPUT_SIDE * SECTION_INPUT_SIDE * SECTION_INPUT_SIDE;
/// The most cell records one [`SectionVisibility::keep`] may hold: one per cell of a section.
pub const MAX_CELL_RECORDS: usize = 4096;
/// A face mask with every face kept: bits down, up, north, south, west, east (0 to 5).
pub const ALL_FACES: u8 = 0b11_1111;

/// One cell of a [`SectionVisibilityInput`].
// no wire: a cell is read only from the in-process section snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SectionCell {
    /// The block state there as the world holds it, before any mod hid it: a state a
    /// `block_render_visibility` part hides still reads here as itself. Above or below the
    /// world it is air.
    State(BlockState),
    /// A neighbouring column that is not loaded. It is not air: decide as if it neither occludes
    /// nor matches anything, and expect the section to be asked again when the column arrives.
    Missing,
}

/// The raw section `section_visibility` reads: the 16-cubed section and a one-block border on
/// every side (edges and corners too), with its light and raw biomes, captured on the client world
/// thread before the section meshes. Nothing beyond the border is available.
// no wire: the client thread lends this snapshot directly to native listeners.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SectionVisibilityInput {
    /// The block position of the section's lowest corner: a multiple of 16 on each axis.
    pub origin: [i32; 3],
    cells: Box<[SectionCell]>,
    light: Box<[u8]>,
    biomes: Box<[Option<BiomeId>]>,
}

impl SectionVisibilityInput {
    /// The game captures one raw state, packed light and stored biome per position in the halo.
    // hidden: engine-only snapshot construction; listeners borrow the capture supplied by the game.
    #[doc(hidden)]
    pub fn capture(
        origin: [i32; 3],
        mut cell_at: impl FnMut([i32; 3]) -> (SectionCell, u8, Option<BiomeId>),
    ) -> Self {
        let mut cells = Vec::with_capacity(SECTION_INPUT_CELLS);
        let mut light = Vec::with_capacity(SECTION_INPUT_CELLS);
        let mut biomes = Vec::with_capacity(SECTION_INPUT_CELLS);
        for y in -1..=16 {
            for z in -1..=16 {
                for x in -1..=16 {
                    let (cell, packed, biome) = cell_at([x, y, z]);
                    cells.push(cell);
                    light.push(packed);
                    biomes.push(biome);
                }
            }
        }
        Self {
            origin,
            cells: cells.into_boxed_slice(),
            light: light.into_boxed_slice(),
            biomes: biomes.into_boxed_slice(),
        }
    }
    /// The index of `local` (from the section's lowest corner), x fastest, then z, then y; each
    /// axis -1 to 16; refused with [`OutsideInput`](SectionVisibilityRefusal::OutsideInput)
    /// beyond that border.
    fn index(local: [i32; 3]) -> Result<usize, SectionVisibilityRefusal> {
        if local.iter().any(|v| !(-1..=16).contains(v)) {
            return Err(SectionVisibilityRefusal::OutsideInput);
        }
        let [x, y, z] = local.map(|v| (v + 1) as usize);
        Ok((y * SECTION_INPUT_SIDE + z) * SECTION_INPUT_SIDE + x)
    }
    /// The cell at `local` (from the section's lowest corner, -1 to 16 on each axis).
    pub fn cell(&self, local: [i32; 3]) -> Result<SectionCell, SectionVisibilityRefusal> {
        Ok(self.cells[Self::index(local)?])
    }
    /// The sky and block light at `local`, 0 to 15 each. A missing cell reads `[0, 0]`.
    pub fn light(&self, local: [i32; 3]) -> Result<[u8; 2], SectionVisibilityRefusal> {
        let packed = self.light[Self::index(local)?];
        Ok([packed >> 4, packed & 15])
    }
    /// The raw, stored biome at `local`, before biome zoom and blending; `None` in a missing column.
    pub fn biome(&self, local: [i32; 3]) -> Result<Option<BiomeId>, SectionVisibilityRefusal> {
        Ok(self.biomes[Self::index(local)?])
    }
    /// The position seed used for seeded block-model selection at `local`.
    pub fn model_seed(&self, [x, y, z]: [i32; 3]) -> Result<i64, SectionVisibilityRefusal> {
        Self::index([x, y, z])?;
        let [ox, oy, oz] = self.origin;
        Ok(sunrise_core::random::position_seed(
            sunrise_core::position::BlockPos::new(
                ox.wrapping_add(x),
                oy.wrapping_add(y),
                oz.wrapping_add(z),
            ),
        ))
    }
}

/// What one cell of a section draws. Every field only takes away: a face a neighbour hides stays
/// hidden, and what one mod hides no other mod brings back.
// no wire: a cell restriction is part of a native listener's in-process return value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CellVisibility {
    /// The cell, from the section's lowest corner: 0 to 15 on each axis.
    pub pos: [u8; 3],
    /// Whether its block model draws. A model that does not draw occludes nothing, and its
    /// unculled quads go with it.
    pub model: bool,
    /// Whether its fluid draws.
    pub fluid: bool,
    /// The faces kept, [`ALL_FACES`] for all: bit 0 down, 1 up, 2 north, 3 south, 4 west, 5
    /// east. A removed face is not drawn, for the model's and the fluid's faces toward it, and
    /// no longer hides the neighbour's face against it. Only faces that would draw can stay.
    pub faces: u8,
}

impl CellVisibility {
    /// The cell at `pos` with its model, its fluid and the faces in `faces` kept.
    pub fn new(pos: [u8; 3], model: bool, fluid: bool, faces: u8) -> Self {
        Self {
            pos,
            model,
            fluid,
            faces,
        }
    }
    /// The cell at `pos` drawn not at all: as with a hidden state, it no longer occludes, so its
    /// neighbours' faces toward it show. Its block entity still draws.
    pub fn hidden(pos: [u8; 3]) -> Self {
        Self::new(pos, false, false, 0)
    }
}

/// A mod's answer for one section in `section_visibility`.
// no wire: a native listener returns this directly to the client thread.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SectionVisibility {
    cells: Option<Vec<CellVisibility>>,
}

impl SectionVisibility {
    /// The section draws, with these cells restricted. Each cell may be named once, and an empty
    /// list changes nothing. Refuses the first record past [`MAX_CELL_RECORDS`] without keeping
    /// it or reading the rest. The answer owns bounded storage, not the iterator's allocation.
    pub fn keep(
        cells: impl IntoIterator<Item = CellVisibility>,
    ) -> Result<Self, SectionVisibilityRefusal> {
        let mut cells = cells.into_iter().peekable();
        let mut kept = if cells.peek().is_some() {
            Vec::with_capacity(MAX_CELL_RECORDS)
        } else {
            Vec::new()
        };
        for cell in cells {
            if kept.len() == MAX_CELL_RECORDS {
                return Err(SectionVisibilityRefusal::TooManyCells {
                    count: (MAX_CELL_RECORDS + 1) as u32,
                });
            }
            kept.push(cell);
        }
        Ok(Self { cells: Some(kept) })
    }

    /// None of the section's terrain draws: no block model, fluid or section geometry quads,
    /// and it hides nothing behind it. Block entities that draw themselves, entities and
    /// collision are untouched.
    pub fn hide_terrain() -> Self {
        Self { cells: None }
    }

    /// Cell restrictions, or `None` when all terrain is hidden.
    pub fn cells(&self) -> Option<&[CellVisibility]> {
        self.cells.as_deref()
    }

    /// Checks the answer as the game does before it uses any of it: the first fault refuses the
    /// whole answer, and the game then keeps the section as if this mod had not answered.
    pub fn check(&self) -> Result<(), SectionVisibilityRefusal> {
        let Some(cells) = self.cells() else {
            return Ok(());
        };
        let mut seen = [0u64; MAX_CELL_RECORDS / 64];
        for cell in cells {
            let [x, y, z] = cell.pos;
            if x > 15 || y > 15 || z > 15 {
                return Err(SectionVisibilityRefusal::OutsideSection { pos: cell.pos });
            }
            if cell.faces > ALL_FACES {
                return Err(SectionVisibilityRefusal::FaceMask {
                    pos: cell.pos,
                    faces: cell.faces,
                });
            }
            let index = usize::from(y) << 8 | usize::from(z) << 4 | usize::from(x);
            let bit = 1u64 << (index & 63);
            if seen[index >> 6] & bit != 0 {
                return Err(SectionVisibilityRefusal::Duplicate { pos: cell.pos });
            }
            seen[index >> 6] |= bit;
        }
        Ok(())
    }
}

/// Why a section visibility read or answer was refused. An answer refused for any of these is
/// refused whole, logged once with the mod named, and the section draws as if it had not answered.
// no wire: refusals are local read errors or diagnostics for a native listener's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SectionVisibilityRefusal {
    /// `E_SECTION_VISIBILITY_OUTSIDE_INPUT`: a read beyond the one-block border.
    OutsideInput,
    /// `E_SECTION_VISIBILITY_OUTSIDE_SECTION`: a cell record outside 0 to 15.
    OutsideSection {
        /// The record's position.
        pos: [u8; 3],
    },
    /// `E_SECTION_VISIBILITY_FACE_MASK`: a face mask above [`ALL_FACES`].
    FaceMask {
        /// The record's position.
        pos: [u8; 3],
        /// The mask given.
        faces: u8,
    },
    /// `E_SECTION_VISIBILITY_DUPLICATE`: two records for one cell.
    Duplicate {
        /// The cell named twice.
        pos: [u8; 3],
    },
    /// `E_SECTION_VISIBILITY_TOO_MANY`: more than [`MAX_CELL_RECORDS`] records.
    TooManyCells {
        /// How many were read before stopping.
        count: u32,
    },
    /// `E_SECTION_VISIBILITY_PANIC`: the mod's listener panicked. It is not asked again
    /// during this session.
    Failed,
}

impl SectionVisibilityRefusal {
    /// The refusal's stable code, `E_SECTION_VISIBILITY_...`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::OutsideInput => "E_SECTION_VISIBILITY_OUTSIDE_INPUT",
            Self::OutsideSection { .. } => "E_SECTION_VISIBILITY_OUTSIDE_SECTION",
            Self::FaceMask { .. } => "E_SECTION_VISIBILITY_FACE_MASK",
            Self::Duplicate { .. } => "E_SECTION_VISIBILITY_DUPLICATE",
            Self::TooManyCells { .. } => "E_SECTION_VISIBILITY_TOO_MANY",
            Self::Failed => "E_SECTION_VISIBILITY_PANIC",
        }
    }
}

impl std::fmt::Display for SectionVisibilityRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = self.code();
        match self {
            Self::OutsideInput => write!(f, "{code}: a read beyond the section's one-block border"),
            Self::OutsideSection { pos } => write!(f, "{code}: cell {pos:?} is outside 0 to 15"),
            Self::FaceMask { pos, faces } => {
                write!(f, "{code}: cell {pos:?} keeps faces {faces}, above 63")
            }
            Self::Duplicate { pos } => write!(f, "{code}: cell {pos:?} is named twice"),
            Self::TooManyCells { count } => {
                write!(f, "{code}: {count} cell records, more than 4096")
            }
            Self::Failed => write!(f, "{code}: the listener panicked"),
        }
    }
}

impl std::error::Error for SectionVisibilityRefusal {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_answer_stops_reading_at_the_first_extra_cell() {
        let cell = |i: usize| {
            CellVisibility::hidden([(i & 15) as u8, (i >> 4 & 15) as u8, (i >> 8) as u8])
        };
        let full = SectionVisibility::keep((0..MAX_CELL_RECORDS).map(cell)).unwrap();
        assert_eq!(full.cells().unwrap().len(), MAX_CELL_RECORDS);
        let mut supplied = (0..).map(cell);
        assert_eq!(
            SectionVisibility::keep(&mut supplied),
            Err(SectionVisibilityRefusal::TooManyCells { count: 4097 }),
        );
        assert_eq!(supplied.next(), Some(cell(MAX_CELL_RECORDS + 1)));
    }
}
