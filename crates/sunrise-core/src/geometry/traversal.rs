//! Ordered block visits for a finite ray, as in BlockGetter.traverseBlocks.
use super::{Vec3, floor_i32};
use crate::position::BlockPos;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraversalError {
    NonFinite,
    VisitLimit,
    InvalidBounds,
}
impl fmt::Display for TraversalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "block traversal failed: {self:?}")
    }
}
impl std::error::Error for TraversalError {}

/// Visit blocks in vanilla ray order until the visitor returns Some, or the
/// segment is exhausted. `None` corresponds to vanilla's miss callback.
///
/// Non-finite geometry and exceeding the caller's visit budget return typed
/// errors. Previously invoked callbacks are not rolled back on a budget error.
pub fn traverse_blocks<T>(
    start: Vec3,
    end: Vec3,
    max_visits: usize,
    visitor: impl FnMut(BlockPos) -> Option<T>,
) -> Result<Option<T>, TraversalError> {
    traverse_blocks_window(start, end, (0.0, 1.0), max_visits, visitor)
}

/// [`traverse_blocks`] over the whole segment `start..end`, but visiting only the cells it
/// enters within `window`, a span `(first, last)` of that segment's length as fractions in
/// `0..=1`. The walk's nudge, step state and cell order are the whole segment's: cells before
/// `first` are passed without calling the visitor (and without being counted against
/// `max_visits`), so a caller that knows where the data is loaded skips the gap and gets the
/// candidates, in the order, a walk of the full segment would give.
pub fn traverse_blocks_window<T>(
    start: Vec3,
    end: Vec3,
    window: (f64, f64),
    max_visits: usize,
    mut visitor: impl FnMut(BlockPos) -> Option<T>,
) -> Result<Option<T>, TraversalError> {
    if !start.is_finite() || !end.is_finite() {
        return Err(TraversalError::NonFinite);
    }
    if start == end {
        return Ok(None);
    }
    let finish = end.lerp(start, -1e-7);
    let origin = start.lerp(end, -1e-7);
    let delta = origin.vector_to(finish).components();
    if !origin.is_finite() || !finish.is_finite() || delta.iter().any(|v| !v.is_finite()) {
        return Err(TraversalError::NonFinite);
    }
    // A BlockPos has signed 32-bit coordinates. Do not let Mth.floor saturation
    // or wrapping turn an out-of-domain ray into a walk through unrelated cells.
    if [origin, finish].into_iter().any(|point| {
        point
            .components()
            .into_iter()
            .any(|v| v < f64::from(i32::MIN) || v >= f64::from(i32::MAX) + 1.0)
    }) {
        return Err(TraversalError::InvalidBounds);
    }
    let mut position = origin.components().map(floor_i32);
    let sign = delta.map(|v| {
        if v == 0.0 {
            0i32
        } else if v > 0.0 {
            1
        } else {
            -1
        }
    });
    let step = std::array::from_fn::<_, 3, _>(|i| {
        if sign[i] == 0 {
            f64::MAX
        } else {
            sign[i] as f64 / delta[i]
        }
    });
    let mut boundary = std::array::from_fn::<_, 3, _>(|i| {
        let value = origin.components()[i];
        let fraction = value - (value.floor() as i64) as f64;
        step[i]
            * if sign[i] > 0 {
                1.0 - fraction
            } else {
                fraction
            }
    });
    // The walk's own parameter spans `origin..finish`, the segment widened by the nudge.
    let in_walk = |fraction: f64| (fraction + 1e-7) / (1.0 + 2e-7);
    let first = if window.0 > 0.0 {
        in_walk(window.0)
    } else {
        0.0
    };
    let last = if window.1 >= 1.0 {
        1.0
    } else {
        in_walk(window.1)
    };
    if first > 0.0 {
        // Pass every crossing at or before `first` in one jump per axis.
        for i in 0..3 {
            if sign[i] != 0 && boundary[i] <= first {
                let crossings = ((first - boundary[i]) / step[i]).floor() + 1.0;
                position[i] =
                    position[i].wrapping_add((crossings as i64 as i32).wrapping_mul(sign[i]));
                boundary[i] += crossings * step[i];
            }
        }
    }
    let mut visited = 0;
    loop {
        if visited == max_visits {
            return Err(TraversalError::VisitLimit);
        }
        visited += 1;
        if let Some(result) = visitor(BlockPos::new(position[0], position[1], position[2])) {
            return Ok(Some(result));
        }
        if !(boundary[0] <= last || boundary[1] <= last || boundary[2] <= last) {
            return Ok(None);
        }
        // Strict comparisons give Z priority on a three-way corner tie.
        let axis = if boundary[0] < boundary[1] {
            if boundary[0] < boundary[2] { 0 } else { 2 }
        } else if boundary[1] < boundary[2] {
            1
        } else {
            2
        };
        position[axis] = position[axis].wrapping_add(sign[axis]);
        boundary[axis] += step[axis];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window skips only the front of the walk: the cells and their order after it are the
    /// whole segment's, however far the window starts along a long, nudged segment.
    #[test]
    fn a_window_keeps_the_whole_walks_cells_and_order() {
        let start = Vec3::new(8.5, 70.5, 8.5);
        for length in [40.0, 20_000_000.0] {
            let end = Vec3::new(8.5 + length, 70.5, 8.5);
            let mut all = Vec::new();
            let limit = 40.0 / length;
            traverse_blocks_window::<()>(start, end, (0.0, limit), usize::MAX, |p| {
                all.push(p);
                None
            })
            .unwrap();
            let first = 8.0 / length;
            let mut windowed = Vec::new();
            traverse_blocks_window::<()>(start, end, (first, limit), usize::MAX, |p| {
                windowed.push(p);
                None
            })
            .unwrap();
            let skipped = all
                .iter()
                .position(|p| windowed.first() == Some(p))
                .unwrap();
            assert_eq!(windowed, all[skipped..], "length {length}");
            assert!(windowed.first().unwrap().x >= 16, "length {length}");
        }
    }
    #[test]
    fn corner_order_and_early_exit() {
        let mut visited = Vec::new();
        assert_eq!(
            traverse_blocks::<()>(
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(1.5, 1.5, 1.5),
                10,
                |pos| {
                    visited.push(pos);
                    None
                }
            ),
            Ok(None)
        );
        assert_eq!(
            visited,
            vec![
                BlockPos::new(0, 0, 0),
                BlockPos::new(0, 0, 1),
                BlockPos::new(0, 1, 1),
                BlockPos::new(1, 1, 1)
            ]
        );
        let result = traverse_blocks(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(1.5, 1.5, 1.5),
            10,
            |pos| (pos.z == 1).then_some(pos),
        );
        assert_eq!(result, Ok(Some(BlockPos::new(0, 0, 1))));
    }
    #[test]
    fn miss_zero_identity_and_budget_errors() {
        let mut count = 0;
        assert_eq!(
            traverse_blocks::<()>(Vec3::ZERO, Vec3::ZERO, 0, |_| panic!(
                "no visits for equal endpoints"
            )),
            Ok(None)
        );
        assert_eq!(
            traverse_blocks::<()>(Vec3::new(-0.0, 0.0, 0.0), Vec3::ZERO, 10, |_| {
                count += 1;
                None
            }),
            Ok(None)
        );
        assert_eq!(count, 4);
        assert_eq!(
            traverse_blocks::<()>(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0), 1, |_| None),
            Err(TraversalError::VisitLimit)
        );
        assert_eq!(
            traverse_blocks::<()>(Vec3::ZERO, Vec3::new(f64::NAN, 1.0, 1.0), 1, |_| None),
            Err(TraversalError::NonFinite)
        );
    }
}
