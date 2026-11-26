//! BlockGetter.forEachBlockIntersectedBetween's ordered movement-path visits.
use super::{Aabb, TraversalError, Vec3, clamp, floor_i32};
use crate::{
    fast_hash::FxHashSet,
    position::{BlockBoxIter, BlockPos},
};

#[derive(Debug)]
struct Work {
    used: usize,
    limit: Option<usize>,
}

impl Work {
    fn spend(&mut self) -> Result<(), TraversalError> {
        if let Some(limit) = self.limit {
            if self.used == limit {
                return Err(TraversalError::VisitLimit);
            }
            self.used += 1;
        }
        Ok(())
    }
}

/// An owned moving-box traversal. Each advance yields a cell and its step
/// without borrowing a world or enumerating the remaining cells.
#[derive(Debug)]
pub struct SweepCursor {
    movement: Vec3,
    final_box: Aabb,
    phase: Phase,
    blocks: BlockBoxIter,
    visited: FxHashSet<i64>,
    work: Work,
    step: i32,
}

#[derive(Debug)]
enum Phase {
    Tiny,
    Initial,
    Travel(MovementRay),
    Final,
    Done,
}

impl SweepCursor {
    /// `final_box` is the box at `end`. Only finite geometry and block boxes
    /// within the positive Java-int iteration domain are supported. A work
    /// limit, when given, counts candidate cells (even duplicates) and ray
    /// steps; vanilla's movement sweep has none, so its callers pass `None`.
    pub fn new(
        start: Vec3,
        end: Vec3,
        final_box: Aabb,
        max_work: Option<usize>,
    ) -> Result<Self, TraversalError> {
        if !start.is_finite()
            || !end.is_finite()
            || !final_box.min_position().is_finite()
            || !final_box.max_position().is_finite()
        {
            return Err(TraversalError::NonFinite);
        }
        let movement = end.subtract(start);
        if !movement.is_finite() {
            return Err(TraversalError::NonFinite);
        }
        let (phase, blocks, visited) = if movement.length_squared()
            < (1.0e-5_f32 * 1.0e-5_f32) as f64
        {
            // Vanilla's stationary path has no deduplication or visited set.
            (
                Phase::Tiny,
                BlockPos::between_closed_box(final_box),
                FxHashSet::default(),
            )
        } else {
            let initial_box = final_box.move_by(movement.scale(-1.0));
            if !initial_box.min_position().is_finite() || !initial_box.max_position().is_finite() {
                return Err(TraversalError::NonFinite);
            }
            (
                Phase::Initial,
                BlockPos::between_box_corners_in_direction(initial_box, movement),
                FxHashSet::with_capacity_and_hasher(32, Default::default()),
            )
        };
        Ok(Self {
            movement,
            final_box,
            phase,
            blocks: blocks.map_err(|_| TraversalError::InvalidBounds)?,
            visited,
            work: Work {
                used: 0,
                limit: max_work,
            },
            step: 0,
        })
    }

    fn next_cell(&mut self) -> Result<Option<(BlockPos, i32)>, TraversalError> {
        loop {
            if matches!(self.phase, Phase::Done) {
                return Ok(None);
            }
            if let Some(pos) = self.blocks.next() {
                self.work.spend()?;
                match self.phase {
                    Phase::Tiny => {}
                    Phase::Initial => {
                        self.visited.insert(pos.pack());
                    }
                    _ if !self.visited.insert(pos.pack()) => continue,
                    _ => {}
                }
                return Ok(Some((pos, self.step)));
            }
            match &mut self.phase {
                Phase::Initial => {
                    self.phase = Phase::Travel(MovementRay::new(self.final_box, self.movement)?);
                }
                Phase::Travel(ray) => {
                    self.step = self.step.wrapping_add(1);
                    if let Some(blocks) = ray.next_box(self.movement, &mut self.work)? {
                        self.blocks = blocks;
                    } else {
                        self.blocks = BlockPos::between_box_corners_in_direction(
                            self.final_box,
                            self.movement,
                        )
                        .map_err(|_| TraversalError::InvalidBounds)?;
                        self.phase = Phase::Final;
                    }
                }
                Phase::Tiny | Phase::Final | Phase::Done => {
                    self.phase = Phase::Done;
                    return Ok(None);
                }
            }
        }
    }
}

impl Iterator for SweepCursor {
    type Item = Result<(BlockPos, i32), TraversalError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.next_cell();
        if next.is_err() {
            self.phase = Phase::Done;
        }
        next.transpose()
    }
}

/// Visit blocks and step indices for a moving box, stopping if the callback
/// returns false. Prior callbacks remain applied on a traversal error.
pub fn sweep_intersected_blocks(
    start: Vec3,
    end: Vec3,
    final_box: Aabb,
    max_work: usize,
    mut visitor: impl FnMut(BlockPos, i32) -> bool,
) -> Result<bool, TraversalError> {
    for cell in SweepCursor::new(start, end, final_box, Some(max_work))? {
        let (pos, step) = cell?;
        if !visitor(pos, step) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug)]
struct MovementRay {
    origin: Vec3,
    finish: Vec3,
    size: [f64; 3],
    corner: [i32; 3],
    position: [i32; 3],
    sign: [i32; 3],
    increment: [f64; 3],
    boundary: [f64; 3],
}

impl MovementRay {
    fn new(final_box: Aabb, movement: Vec3) -> Result<Self, TraversalError> {
        let size = final_box.size().components();
        let corner = furthest_corner(movement);
        let center = final_box.center().components();
        let finish = Vec3::new(
            center[0] + size[0] * 0.5 * corner[0] as f64,
            center[1] + size[1] * 0.5 * corner[1] as f64,
            center[2] + size[2] * 0.5 * corner[2] as f64,
        );
        let origin = finish.subtract(movement);
        if !origin.is_finite() || !finish.is_finite() || size.iter().any(|v| !v.is_finite()) {
            return Err(TraversalError::NonFinite);
        }
        let position = origin.components().map(floor_i32);
        let delta = movement.components();
        let sign = delta.map(|v| {
            if v == 0.0 {
                0i32
            } else if v > 0.0 {
                1
            } else {
                -1
            }
        });
        let increment = std::array::from_fn::<_, 3, _>(|i| {
            if sign[i] == 0 {
                f64::MAX
            } else {
                sign[i] as f64 / delta[i]
            }
        });
        let boundary = std::array::from_fn::<_, 3, _>(|i| {
            let value = origin.components()[i];
            let fraction = value - (value.floor() as i64) as f64;
            increment[i]
                * if sign[i] > 0 {
                    1.0 - fraction
                } else {
                    fraction
                }
        });
        Ok(Self {
            origin,
            finish,
            size,
            corner,
            position,
            sign,
            increment,
            boundary,
        })
    }

    fn next_box(
        &mut self,
        movement: Vec3,
        work: &mut Work,
    ) -> Result<Option<BlockBoxIter>, TraversalError> {
        let Self {
            origin,
            finish,
            size,
            corner,
            position,
            sign,
            increment,
            boundary,
        } = self;
        while boundary.iter().any(|v| *v <= 1.0) {
            work.spend()?;
            let axis = if boundary[0] < boundary[1] {
                if boundary[0] < boundary[2] { 0 } else { 2 }
            } else if boundary[1] < boundary[2] {
                1
            } else {
                2
            };
            position[axis] = position[axis].wrapping_add(sign[axis]);
            boundary[axis] += increment[axis];
            let pos = BlockPos::new(position[0], position[1], position[2]);
            // The static Java clip takes raw endpoints, including wrapped +1 ints.
            let block_box = Aabb {
                min: Vec3::from_block(pos),
                max: Vec3::from_block(pos.offset(1, 1, 1)),
            };
            let Some(point) = block_box.clip(*origin, *finish) else {
                continue;
            };
            let clamped = std::array::from_fn::<_, 3, _>(|i| {
                clamp(
                    point.components()[i],
                    position[i] as f64 + 1.0e-5_f32 as f64,
                    (position[i] as f64 + 1.0) - 1.0e-5_f32 as f64,
                )
            });
            let opposite = std::array::from_fn::<_, 3, _>(|i| {
                floor_i32(clamped[i] - size[i] * corner[i] as f64)
            });
            return BlockPos::between_corners_in_direction(
                pos,
                BlockPos::new(opposite[0], opposite[1], opposite[2]),
                movement,
            )
            .map(Some)
            .map_err(|_| TraversalError::InvalidBounds);
        }
        Ok(None)
    }
}

fn furthest_corner(movement: Vec3) -> [i32; 3] {
    let abs = [
        Vec3::new(1.0, 0.0, 0.0).dot(movement).abs(),
        Vec3::new(0.0, 1.0, 0.0).dot(movement).abs(),
        Vec3::new(0.0, 0.0, 1.0).dot(movement).abs(),
    ];
    let [x, y, z] = movement.components().map(|v| if v >= 0.0 { 1 } else { -1 });
    if abs[0] <= abs[1] && abs[0] <= abs[2] {
        [-x, -z, y]
    } else if abs[1] <= abs[2] {
        [z, -y, -x]
    } else {
        [-y, x, -z]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moving_cursor_retains_its_step_and_work_across_visits() {
        let start = Vec3::new(0.25, 0.25, 0.25);
        let end = Vec3::new(1.25, 0.25, 0.25);
        let bounds = Aabb::new(end, end.add_vector(Vec3::new(0.5, 0.5, 0.5)));
        let mut cells = SweepCursor::new(start, end, bounds, Some(4)).unwrap();
        assert_eq!(cells.next(), Some(Ok((BlockPos::new(0, 0, 0), 0))));
        assert_eq!(cells.next(), Some(Ok((BlockPos::new(1, 0, 0), 1))));
        // The final box contains only an already-emitted cell, but it still costs work.
        assert_eq!(cells.next(), Some(Err(TraversalError::VisitLimit)));
        assert_eq!(cells.next(), None);
    }

    #[test]
    fn tiny_movement_includes_maximum_faces_and_stops_immediately() {
        let box_ = Aabb::unit_cube(Vec3::ZERO);
        let mut visits = Vec::new();
        assert_eq!(
            sweep_intersected_blocks(Vec3::ZERO, Vec3::ZERO, box_, 20, |p, step| {
                visits.push((p, step));
                true
            }),
            Ok(true)
        );
        assert_eq!(visits.len(), 8);
        assert!(visits.iter().all(|(_, step)| *step == 0));
        assert_eq!(
            sweep_intersected_blocks(Vec3::ZERO, Vec3::ZERO, box_, 20, |_, _| false),
            Ok(false)
        );
        assert_eq!(
            sweep_intersected_blocks(Vec3::ZERO, Vec3::ZERO, box_, 1, |_, _| true),
            Err(TraversalError::VisitLimit)
        );
    }
}
