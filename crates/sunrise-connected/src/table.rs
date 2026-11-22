//! The `table` method: a tile chosen by a lookup, so a format (a mod's asset file) can declare
//! its own connected-texture type without code.

use crate::{D, DL, DR, L, R, U, UL, UR};
use std::sync::Arc;

/// The names of the 8 tests, in a face's texture directions, and their mask bits.
const TESTS: [(&str, u8); 8] = [
    ("up", U),
    ("up_right", UR),
    ("right", R),
    ("down_right", DR),
    ("down", D),
    ("down_left", DL),
    ("left", L),
    ("up_left", UL),
];

/// The sheets a table may cut: columns and rows, each at most this.
const MAX_GRID: u32 = 16;

/// Which neighbours a face tests, and the tile each outcome draws from a sheet cut into a grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// The mask bit of each test, in the order the outcome counts them: the first is the lowest
    /// bit.
    tests: Vec<u8>,
    /// A neighbour whose front cell (the cell in front of it, on the face's side) also joins
    /// does not join: a covered step.
    cover: bool,
    columns: u32,
    rows: u32,
    /// By outcome: the tile, counted row by row from the sheet's top left.
    tiles: Vec<u16>,
}

impl Table {
    /// The mask bit of the test `name` (`up`, `up_right`, `right`...).
    pub fn test(name: &str) -> Option<u8> {
        TESTS.iter().find(|t| t.0 == name).map(|t| t.1)
    }

    /// A table of `tests` (mask bits, each once), a sheet of `columns` x `rows` tiles and one
    /// tile per outcome of the tests (`2^tests` of them), each inside the sheet.
    pub fn new(
        tests: Vec<u8>,
        cover: bool,
        (columns, rows): (u32, u32),
        tiles: Vec<u16>,
    ) -> Result<Self, String> {
        if !(1..=MAX_GRID).contains(&columns) || !(1..=MAX_GRID).contains(&rows) {
            return Err(format!(
                "a {columns} x {rows} sheet (each side is 1 to {MAX_GRID})"
            ));
        }
        let bits = tests.iter().fold(0u8, |bits, &t| bits | t);
        if tests.iter().any(|&t| !TESTS.iter().any(|x| x.1 == t))
            || bits.count_ones() as usize != tests.len()
        {
            return Err("tests are a list of the 8 neighbours, each at most once".into());
        }
        let needed = 1usize << tests.len();
        if tiles.len() != needed {
            return Err(format!(
                "{} tiles for {} tests (needs {needed})",
                tiles.len(),
                tests.len()
            ));
        }
        if let Some(tile) = tiles.iter().find(|&&t| u32::from(t) >= columns * rows) {
            return Err(format!(
                "tile {tile} is outside the {columns} x {rows} sheet"
            ));
        }
        Ok(Self {
            tests,
            cover,
            columns,
            rows,
            tiles,
        })
    }

    /// The mask bits the tests read: the only neighbours a face needs to probe.
    pub fn tested(&self) -> u8 {
        self.tests.iter().fold(0, |bits, &t| bits | t)
    }

    pub fn cover(&self) -> bool {
        self.cover
    }

    /// The sheet's columns and rows.
    pub fn grid(&self) -> (u32, u32) {
        (self.columns, self.rows)
    }

    /// The tile (counted row by row from the sheet's top left) for the neighbours of `mask` that
    /// join.
    pub fn tile(&self, mask: u8) -> usize {
        let outcome = self
            .tests
            .iter()
            .enumerate()
            .fold(0, |n, (i, &test)| n | usize::from(mask & test != 0) << i);
        usize::from(self.tiles[outcome])
    }
}

/// A table and the sprite its tiles are cut from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSheet {
    /// The sprite's name (`mod:block/casing_connected`).
    pub sprite: String,
    pub table: Arc<Table>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_per_outcome_of_the_tests() {
        // Left, then right: the outcome is left + 2 * right.
        let row = Table::new(vec![L, R], false, (4, 1), vec![3, 2, 0, 1]).unwrap();
        // Alone (3), joined on the left only (2), on the right only (0), both (1).
        assert_eq!([0, L, R, L | R].map(|m| row.tile(m)), [3, 2, 0, 1]);
        // Bits that are not tested change nothing.
        assert_eq!(row.tile(U | D | UL | L), 2);
        // Three tests in the order the file lists them.
        let corner = Table::new(vec![UR, U, R], false, (3, 3), (0..8).collect()).unwrap();
        assert_eq!(corner.tile(UR), 1);
        assert_eq!(corner.tile(U), 2);
        assert_eq!(corner.tile(R), 4);
        assert_eq!(corner.tile(UR | U | R | D), 7);
    }

    #[test]
    fn refusals_name_what_is_wrong() {
        let new = |tests: Vec<u8>, grid, tiles: Vec<u16>| Table::new(tests, false, grid, tiles);
        // The length must be 2^tests.
        let err = new(vec![L, R], (2, 2), vec![0, 1, 2]).unwrap_err();
        assert!(err.contains("3 tiles for 2 tests (needs 4)"), "{err}");
        assert!(new(vec![], (1, 1), vec![0]).is_ok());
        // A tile past the sheet, a sheet with no tiles or too many.
        let err = new(vec![L], (2, 1), vec![0, 2]).unwrap_err();
        assert!(err.contains("tile 2 is outside the 2 x 1 sheet"), "{err}");
        assert!(new(vec![L], (0, 1), vec![0, 0]).is_err());
        assert!(new(vec![L], (17, 1), vec![0, 0]).is_err());
        // A test twice, or not a single neighbour.
        assert!(new(vec![L, L], (2, 2), vec![0; 4]).is_err());
        assert!(new(vec![L | R], (2, 2), vec![0, 0]).is_err());
        assert_eq!(Table::test("down_left"), Some(DL));
        assert_eq!(Table::test("middle"), None);
    }
}
