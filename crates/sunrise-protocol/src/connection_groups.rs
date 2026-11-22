//! The optional clientbound group data for one section. The associated vanilla
//! column or block update comes first in the same bundle. For a column these
//! sections describe its complete sparse group map; for a block update an
//! entry's `None` clears that cell.
use crate::{
    Error, Result,
    buf::{Codec, Reader, SectionPos},
};
use sunrise_core::vanilla_bytes::Writer;

pub const CHANNEL: &str = "sunrise:connection_groups";
pub const VERSION: &str = "2";
const CELLS: usize = 16 * 16 * 16;

/// The 12-bit relative position in a `SectionBlocksUpdate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell(u16);

impl Cell {
    pub const fn of([x, y, z]: [i32; 3]) -> Self {
        Self(((x & 15) << 8 | (z & 15) << 4 | (y & 15)) as u16)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub fn at(self, [x, y, z]: [i32; 3]) -> [i32; 3] {
        let cell = i32::from(self.0);
        [
            x << 4 | cell >> 8,
            y << 4 | cell & 15,
            z << 4 | (cell >> 4) & 15,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub cell: Cell,
    pub group: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub pos: [i32; 3],
    pub entries: Vec<Entry>,
}

impl Section {
    /// Worst case is 11 bytes per cell, well under the Play payload limit.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.entries.len() > CELLS {
            return Err(Error::new("connection groups: too many section entries"));
        }
        let mut w = Writer::new();
        SectionPos::write(&self.pos, &mut w);
        w.varint(self.entries.len() as i32);
        for entry in &self.entries {
            w.u16(entry.cell.0);
            w.bool(entry.group.is_some());
            if let Some(group) = entry.group {
                w.i64(group as i64);
            }
        }
        Ok(w.data)
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let pos = SectionPos::read(&mut r)?;
        let count = r.count(CELLS)?;
        let mut entries = Vec::with_capacity(count.min(r.remaining() / 3));
        let mut seen = [0_u64; CELLS / 64];
        for _ in 0..count {
            let cell = r.u16()?;
            let index = usize::from(cell);
            let Some(word) = seen.get_mut(index / 64) else {
                return Err(Error::new("connection groups: cell outside section"));
            };
            let bit = 1 << (index % 64);
            if *word & bit != 0 {
                return Err(Error::new("connection groups: repeated cell"));
            }
            *word |= bit;
            let group = if r.bool()? {
                Some(r.i64()? as u64)
            } else {
                None
            };
            entries.push(Entry {
                cell: Cell(cell),
                group,
            });
        }
        if r.remaining() != 0 {
            return Err(Error::new("connection groups: trailing section bytes"));
        }
        Ok(Self { pos, entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_preserves_zero_and_clears_and_rejects_repeated_cells() {
        let pos = [-1, -4, 2];
        let mut section = Section {
            pos,
            entries: vec![
                Entry {
                    cell: Cell::of([-1, -64, 47]),
                    group: Some(0),
                },
                Entry {
                    cell: Cell::of([-16, -49, 32]),
                    group: None,
                },
            ],
        };
        let decoded = Section::decode(&section.encode().unwrap()).unwrap();
        assert_eq!(decoded, section);
        assert_eq!(decoded.entries[0].cell.at(pos), [-1, -64, 47]);
        assert_eq!(decoded.entries[1].cell.at(pos), [-16, -49, 32]);
        section.entries.push(section.entries[0]);
        assert!(Section::decode(&section.encode().unwrap()).is_err());
    }
}
