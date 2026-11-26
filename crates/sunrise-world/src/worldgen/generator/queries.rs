//! Requests and results of the generator's queries (`/place jigsaw`, `/locate biome`) and
//! the search orders they walk, moved out of `generator.rs` unchanged.
use std::{collections::BTreeSet, sync::Arc};

/// A `JigsawPlacement.generateJigsaw` request (`/place jigsaw`).
pub struct JigsawQuery<'a> {
    pub pool: &'a str,
    pub target: &'a str,
    pub max_depth: i32,
    pub position: [i32; 3],
}
/// The assembled pieces of a jigsaw and the templates they place.
pub struct JigsawPieces {
    pub pieces: Vec<super::super::structure::jigsaw::PoolPiece>,
    pub resources: Arc<super::super::structure::StructureResources>,
}

/// A `findClosestBiome3d` query.
pub struct BiomeSearch<'a> {
    pub allowed: &'a dyn Fn(&str) -> bool,
    pub origin: [i32; 3],
    pub radius: i32,
    pub horizontal: i32,
    pub vertical: i32,
    /// `level.getMinY()` and `level.getMaxY()`.
    pub min_y: i32,
    pub max_y: i32,
}

/// `Mth.outFromOrigin(origin, lower, upper, step)`.
pub fn out_from_origin(origin: i32, lower: i32, upper: i32, step: i32) -> Vec<i32> {
    assert!(lower <= upper, "outFromOrigin requires ordered bounds");
    assert!(step > 0, "outFromOrigin requires a positive step");
    // Public generator callers may supply any positive i32 step. Intermediates can
    // exceed the coordinate range even though every emitted height stays within it.
    let (lower, upper, step) = (i64::from(lower), i64::from(upper), i64::from(step));
    let center = i64::from(origin).clamp(lower, upper);
    let mut out = Vec::new();
    let mut cursor = center;
    loop {
        let distance = (center - cursor).abs();
        if !(center - distance >= lower || center + distance <= upper) {
            return out;
        }
        out.push(cursor as i32);
        let negative = cursor <= center;
        let can_move_positive = center + distance + step <= upper;
        let mut next = center + distance + step;
        if !negative || !can_move_positive {
            let attempt = center - distance - if negative { step } else { 0 };
            if attempt >= lower {
                next = attempt;
            }
        }
        cursor = next;
    }
}

/// `BlockPos.spiralAround(ZERO, radius, EAST, SOUTH)` as (x, z) offsets.
pub fn spiral_around(radius: i32) -> impl Iterator<Item = [i32; 2]> {
    const DIRS: [[i32; 2]; 4] = [[1, 0], [0, 1], [-1, 0], [0, -1]];
    let legs = 4 * i64::from(radius);
    let (mut leg, mut leg_size, mut leg_index) = (-1i64, 0i64, 0i64);
    let mut last = [0i32, 1i32];
    std::iter::from_fn(move || {
        let d = DIRS[((leg + 4) % 4) as usize];
        let cursor = [last[0].wrapping_add(d[0]), last[1].wrapping_add(d[1])];
        last = cursor;
        if leg_index >= leg_size {
            if leg >= legs {
                return None;
            }
            leg += 1;
            leg_index = 0;
            leg_size = leg / 2 + 1;
        }
        leg_index += 1;
        Some(cursor)
    })
}

#[cfg(test)]
mod tests {
    use super::spiral_around;

    #[test]
    fn spiral_around_handles_zero_negative_and_max_radius_without_overflow() {
        assert_eq!(spiral_around(0).collect::<Vec<_>>(), vec![[0, 0]]);
        assert!(spiral_around(-1).next().is_none());
        assert_eq!(
            spiral_around(i32::MAX).take(5).collect::<Vec<_>>(),
            vec![[0, 0], [1, 0], [1, 1], [0, 1], [-1, 1]]
        );
    }
}

/// Counters from one FEATURES pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecorateStats {
    /// Placed-feature roots executed (including those that placed nothing).
    pub placed: usize,
    /// Roots skipped because their algorithm or bindings are not implemented.
    pub unsupported: usize,
    /// Roots that started but failed on an unavailable world query or error.
    /// Their partial writes are kept, as each root reseeds its own random.
    pub failed: usize,
    /// Identifiers of skipped or failed roots, for diagnostics.
    pub skipped: BTreeSet<String>,
}
