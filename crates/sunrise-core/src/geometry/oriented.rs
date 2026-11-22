//! Moving colliders: boxes that are axis-aligned in a collider's own frame and oriented in
//! the world, and the moving entity's axis-aligned box swept against them.
//!
//! Vanilla has no such shape (an entity collides as one world box), so nothing here follows a
//! vanilla routine. The sweep is the separating-axis test on the 15 axes of two boxes with the
//! entity's movement along one world axis, giving the time of impact; the broad phase is a
//! uniform grid over the boxes in the collider's frame.

use super::{Aabb, Axis, COLLISION_EPSILON, Vec3};
use rustc_hash::FxHashMap;
use std::{cell::RefCell, sync::Arc};

/// The most boxes one collider holds.
pub const MAX_BOXES: usize = 32_768;
/// The farthest a box may lie from the collider's origin on any axis, in blocks.
pub const MAX_REACH: f64 = 512.0;
/// How far from length 1 a rotation may be.
pub const ROTATION_TOLERANCE: f64 = 1.0e-3;

/// Why a box of a collider is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxFault {
    NonFinite,
    OutOfRange,
}

/// Checks one box's corners against the collider contract. The one check every way a collider
/// enters the game goes through: a mod's `set_collider`, a save, and the sync wire.
pub fn check_box(min: [f64; 3], max: [f64; 3]) -> Result<(), BoxFault> {
    let values = min.iter().chain(&max);
    if values.clone().any(|v| !v.is_finite()) {
        return Err(BoxFault::NonFinite);
    }
    if values.clone().any(|v| v.abs() > MAX_REACH) {
        return Err(BoxFault::OutOfRange);
    }
    Ok(())
}

/// Whether a rotation is a finite quaternion of length 1 within [`ROTATION_TOLERANCE`].
pub fn check_rotation(q: [f64; 4]) -> bool {
    q.iter().all(|v| v.is_finite())
        && (q.iter().map(|v| v * v).sum::<f64>().sqrt() - 1.0).abs() <= ROTATION_TOLERANCE
}

thread_local! {
    /// Candidate indices of the query in progress, kept between queries so a sweep allocates
    /// nothing once it has run.
    static CANDIDATES: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

/// Bounds of the side of a grid cell in the collider's frame, in blocks: the grid is built with
/// a side of [`CELL_PER_BOX`] times the median longest edge of the boxes, kept inside them, so a
/// cell of unit boxes holds a few of them and one of whole decks is not split to dust.
const CELL_MIN: f64 = 1.0;
const CELL_MAX: f64 = 8.0;
/// The grid's cell side per block of the median box's longest edge.
const CELL_PER_BOX: f64 = 2.0;
/// A box covering more cells than this is not stored per cell; every query tests it.
const OVERSIZED_CELLS: i64 = 4096;
/// An axis whose squared length is below this (parallel edges) separates nothing new.
const DEGENERATE_AXIS: f64 = 1.0e-12;

/// A collider's frame: where its origin is in the world and where its local axes point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub origin: Vec3,
    /// The world directions of the local x, y and z axes: the columns of the rotation.
    axes: [Vec3; 3],
}

impl Frame {
    pub const fn translation(origin: Vec3) -> Self {
        Self {
            origin,
            axes: [
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
        }
    }

    /// The frame turned by a quaternion `[x, y, z, w]`. A quaternion that is not unit length
    /// is normalised; a zero or non-finite one gives no turn.
    pub fn from_quaternion(origin: Vec3, q: [f64; 4]) -> Self {
        let norm = q.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !norm.is_finite() || norm < 1.0e-12 {
            return Self::translation(origin);
        }
        let [x, y, z, w] = q.map(|v| v / norm);
        Self {
            origin,
            axes: [
                Vec3::new(
                    1.0 - 2.0 * (y * y + z * z),
                    2.0 * (x * y + z * w),
                    2.0 * (x * z - y * w),
                ),
                Vec3::new(
                    2.0 * (x * y - z * w),
                    1.0 - 2.0 * (x * x + z * z),
                    2.0 * (y * z + x * w),
                ),
                Vec3::new(
                    2.0 * (x * z + y * w),
                    2.0 * (y * z - x * w),
                    1.0 - 2.0 * (x * x + y * y),
                ),
            ],
        }
    }

    /// The frame `amount` of the way from `self` to `other` (0 is `self`): the origin in a
    /// line, the axes blended and made orthonormal again, which for the small turns between two
    /// ticks is the shortest rotation.
    pub fn interpolate(&self, other: &Self, amount: f64) -> Self {
        let a = amount.clamp(0.0, 1.0);
        let mix = |k: usize| {
            self.axes[k]
                .scale(1.0 - a)
                .add_vector(other.axes[k].scale(a))
        };
        let unit = |v: Vec3, fallback: Vec3| {
            let n = v.length();
            if n.is_finite() && n > DEGENERATE_AXIS {
                v.scale(1.0 / n)
            } else {
                fallback
            }
        };
        let x = unit(mix(0), self.axes[0]);
        let z = unit(x.cross(mix(1)), self.axes[2]);
        let y = z.cross(x);
        Self {
            origin: self.origin.lerp(other.origin, a),
            axes: [x, y, z],
        }
    }

    /// Whether the two frames point their axes the same way (they differ only in origin).
    pub fn same_turn(&self, other: &Self) -> bool {
        self.axes == other.axes
    }

    pub fn to_world(&self, local: Vec3) -> Vec3 {
        Vec3::new(
            self.origin.x
                + local.x * self.axes[0].x
                + local.y * self.axes[1].x
                + local.z * self.axes[2].x,
            self.origin.y
                + local.x * self.axes[0].y
                + local.y * self.axes[1].y
                + local.z * self.axes[2].y,
            self.origin.z
                + local.x * self.axes[0].z
                + local.y * self.axes[1].z
                + local.z * self.axes[2].z,
        )
    }

    pub fn to_local(&self, world: Vec3) -> Vec3 {
        let d = world.subtract(self.origin);
        Vec3::new(
            d.dot(self.axes[0]),
            d.dot(self.axes[1]),
            d.dot(self.axes[2]),
        )
    }

    /// The smallest axis-aligned box in the collider's frame holding a world box.
    pub fn local_bounds(&self, world: Aabb) -> Aabb {
        self.bound(world, |frame, p| frame.to_local(p))
    }

    /// The smallest axis-aligned box in the world holding a local box.
    pub fn world_bounds(&self, local: Aabb) -> Aabb {
        self.bound(local, |frame, p| frame.to_world(p))
    }

    fn bound(&self, b: Aabb, map: impl Fn(&Self, Vec3) -> Vec3) -> Aabb {
        let (lo, hi) = (b.min, b.max);
        let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut max = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for i in 0..8 {
            let p = map(
                self,
                Vec3::new(
                    if i & 1 == 0 { lo.x } else { hi.x },
                    if i & 2 == 0 { lo.y } else { hi.y },
                    if i & 4 == 0 { lo.z } else { hi.z },
                ),
            );
            min = Vec3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
            max = Vec3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
        }
        Aabb::new(min, max)
    }
}

/// The boxes of one collider, in its local frame, with a uniform grid for queries.
#[derive(Clone, Debug, Default)]
pub struct ColliderBoxes {
    boxes: Vec<Aabb>,
    cells: FxHashMap<[i32; 3], Vec<u32>>,
    /// The side of a cell, from the boxes alone.
    cell: f64,
    oversized: Vec<u32>,
    /// The indices of the boxes with volume: the only ones a query can meet.
    solid: Vec<u32>,
    bounds: Option<Aabb>,
}

/// The side of the grid cell for these boxes: [`CELL_PER_BOX`] times their median longest edge,
/// within [`CELL_MIN`] and [`CELL_MAX`]. Deterministic, and 4 blocks when there are no boxes.
fn cell_side(boxes: &[Aabb]) -> f64 {
    let mut edges: Vec<f64> = boxes
        .iter()
        .filter(|b| (0..3).all(|i| b.max(Axis::ALL[i]) > b.min(Axis::ALL[i])))
        .map(|b| {
            (0..3)
                .map(|i| b.max(Axis::ALL[i]) - b.min(Axis::ALL[i]))
                .fold(0.0, f64::max)
        })
        .collect();
    if edges.is_empty() {
        return 4.0;
    }
    let middle = edges.len() / 2;
    let (_, median, _) = edges.select_nth_unstable_by(middle, f64::total_cmp);
    (*median * CELL_PER_BOX).clamp(CELL_MIN, CELL_MAX)
}

fn cell_range(b: Aabb, cell: f64) -> ([i64; 3], [i64; 3]) {
    let (lo, hi) = (b.min, b.max);
    let f = |v: f64| (v / cell).floor() as i64;
    ([f(lo.x), f(lo.y), f(lo.z)], [f(hi.x), f(hi.y), f(hi.z)])
}

impl ColliderBoxes {
    /// Builds the grid. The boxes must be finite; callers refuse anything else first.
    pub fn new(boxes: Vec<Aabb>) -> Self {
        let cell = cell_side(&boxes);
        let mut cells: FxHashMap<[i32; 3], Vec<u32>> = FxHashMap::default();
        let mut oversized = Vec::new();
        let mut solid = Vec::new();
        let mut bounds: Option<Aabb> = None;
        for (index, b) in boxes.iter().enumerate() {
            // A box with no volume collides with nothing: it keeps its index and is in no cell.
            if (0..3).any(|i| b.max(Axis::ALL[i]) <= b.min(Axis::ALL[i])) {
                continue;
            }
            solid.push(index as u32);
            bounds = Some(bounds.map_or(*b, |all| all.union_bounds(*b)));
            let (lo, hi) = cell_range(*b, cell);
            let count = (0..3).map(|i| hi[i] - lo[i] + 1).product::<i64>();
            if count > OVERSIZED_CELLS {
                oversized.push(index as u32);
                continue;
            }
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        cells
                            .entry([x as i32, y as i32, z as i32])
                            .or_default()
                            .push(index as u32);
                    }
                }
            }
        }
        Self {
            boxes,
            cells,
            cell,
            oversized,
            solid,
            bounds,
        }
    }

    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    pub fn boxes(&self) -> &[Aabb] {
        &self.boxes
    }

    /// The box holding every box, in the collider's frame.
    pub fn bounds(&self) -> Option<Aabb> {
        self.bounds
    }

    /// Calls `f` with the index of each box that may meet `local` until it answers true; no
    /// allocation, and a box in several cells of the query may be named more than once.
    pub fn visit_candidates(&self, local: Aabb, mut f: impl FnMut(u32) -> bool) {
        if self.solid.is_empty() {
            return;
        }
        let (lo, hi) = cell_range(local, self.cell);
        let count = (0..3).map(|i| hi[i] - lo[i] + 1).product::<i64>();
        if count > self.cells.len() as i64 {
            self.solid.iter().any(|&index| f(index));
            return;
        }
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    if let Some(list) = self.cells.get(&[x as i32, y as i32, z as i32])
                        && list.iter().any(|&index| f(index))
                    {
                        return;
                    }
                }
            }
        }
        self.oversized.iter().any(|&index| f(index));
    }

    /// Indices of the boxes that may meet `local` (a superset: each box is then tested).
    pub fn candidates(&self, local: Aabb, out: &mut Vec<u32>) {
        out.clear();
        if self.solid.is_empty() {
            return;
        }
        let (lo, hi) = cell_range(local, self.cell);
        let count = (0..3).map(|i| hi[i] - lo[i] + 1).product::<i64>();
        if count > self.cells.len() as i64 {
            // A query wider than the grid: every box is cheaper than every cell.
            out.extend_from_slice(&self.solid);
            return;
        }
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    if let Some(list) = self.cells.get(&[x as i32, y as i32, z as i32]) {
                        out.extend_from_slice(list);
                    }
                }
            }
        }
        out.extend_from_slice(&self.oversized);
        out.sort_unstable();
        out.dedup();
    }
}

/// One collider placed in the world: its boxes under a frame. The boxes are shared, so a
/// placed collider is cheap to copy into a query that outlives the borrow of the game.
#[derive(Clone)]
pub struct OrientedCollider {
    pub frame: Frame,
    pub boxes: Arc<ColliderBoxes>,
}

/// The axes of one query's separating-axis tests.
struct Axes {
    list: [Vec3; 15],
    count: usize,
}

impl Axes {
    fn iter(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.list[..self.count].iter().copied()
    }
}

/// A box of the collider as a world oriented box: centre, half sizes along the frame's axes.
struct Obb {
    centre: Vec3,
    half: [f64; 3],
}

impl OrientedCollider {
    pub fn new(frame: Frame, boxes: Arc<ColliderBoxes>) -> Self {
        Self { frame, boxes }
    }

    /// The world box holding the whole collider, for the per-level index.
    pub fn world_bounds(&self) -> Option<Aabb> {
        self.boxes.bounds().map(|b| self.frame.world_bounds(b))
    }

    fn obb(&self, local: Aabb) -> Obb {
        let size = local.size();
        Obb {
            centre: self.frame.to_world(local.center()),
            half: [size.x / 2.0, size.y / 2.0, size.z / 2.0],
        }
    }

    /// The boxes whose own bounds meet the world box (tested in the collider's frame, where
    /// they are axis-aligned): the grid's cells first, then each box's bounds.
    fn with_candidates<R>(&self, world: Aabb, f: impl FnOnce(&[u32]) -> R) -> R {
        self.with_local_candidates(self.frame.local_bounds(world), Aabb::intersects, f)
    }

    fn with_local_candidates<R>(
        &self,
        local: Aabb,
        overlaps: impl Fn(Aabb, Aabb) -> bool,
        f: impl FnOnce(&[u32]) -> R,
    ) -> R {
        let Some(bounds) = self
            .boxes
            .bounds()
            .filter(|&bounds| overlaps(bounds, local))
        else {
            return f(&[]);
        };
        let mut out = CANDIDATES.with(|c| std::mem::take(&mut *c.borrow_mut()));
        self.boxes.candidates(local.intersect(bounds), &mut out);
        out.retain(|&i| overlaps(self.boxes.boxes[i as usize], local));
        let result = f(&out);
        CANDIDATES.with(|c| *c.borrow_mut() = out);
        result
    }

    /// How far the entity box `mover` may move along `axis` by `amount` (signed) before it meets
    /// a box, as `Shape.collide` answers for a block shape. A box the mover already overlaps
    /// deeper than the collision epsilon does not stop it: a collider that moved into an entity
    /// never freezes it.
    pub fn collide(&self, mover: Aabb, axis: Axis, amount: f64) -> f64 {
        if amount == 0.0 {
            return amount;
        }
        let mut direction = Vec3::ZERO;
        direction = direction.with_component(axis, amount);
        let sweep = mover.expand_towards(direction);
        let mut allowed = amount;
        let axes = self.axes(DEGENERATE_AXIS);
        self.with_candidates(sweep, |candidates| {
            for &index in candidates {
                let obb = self.obb(self.boxes.boxes[index as usize]);
                if let Some(reach) = self.time_of_impact(&axes, mover, direction, &obb)
                    && reach.abs() < allowed.abs()
                {
                    allowed = reach;
                }
            }
        });
        allowed
    }

    /// Fraction of a straight collider's travel remaining after its first contact with `mover`.
    /// Only boxes that survive at the same local index and bounds can sweep. `delta` is the
    /// collider's displacement from this pose to `after`, which has the same orientation.
    pub fn translation_push_fraction(&self, after: &Self, mover: Aabb, delta: Vec3) -> f64 {
        // PistonMath.getMovementArea sweeps the advancing face, not the old solid volume.
        // For a turned box moving diagonally there can be several advancing faces.
        let relative = delta.scale(-1.0);
        let axes = self.axes(0.0);
        let axis_aligned = self.frame.axes.iter().all(|axis| {
            matches!(
                axis.components().map(f64::abs),
                [1.0, 0.0, 0.0] | [0.0, 1.0, 0.0] | [0.0, 0.0, 1.0]
            )
        });
        let components = self.frame.axes.map(|axis| delta.dot(axis));
        let sweep = mover.expand_towards(relative);
        // Rebasing a positive world overlap into an aligned local frame can round it to
        // tangency. Keep that cell and candidate; only the world test decides contact.
        let overlaps = |a: Aabb, b: Aabb| {
            if axis_aligned {
                Axis::ALL
                    .into_iter()
                    .all(|axis| a.min(axis) <= b.max(axis) && a.max(axis) >= b.min(axis))
            } else {
                a.intersects(b)
            }
        };
        let local_sweep = self.frame.local_bounds(sweep);
        self.with_local_candidates(local_sweep, overlaps, |candidates| {
            let mut first = 1.0_f64;
            for &index in candidates {
                let index = index as usize;
                let local = self.boxes.boxes[index];
                if after.boxes.boxes.get(index) != Some(&local) {
                    continue;
                }
                if axis_aligned {
                    // A stationary envelope axis spans exactly the advancing face's bounds.
                    // Compare endpoints directly; do not project them through centre/radius.
                    let bounds = Aabb::new(
                        self.frame.to_world(local.min),
                        self.frame.to_world(local.max),
                    );
                    if Axis::ALL.into_iter().any(|axis| {
                        delta.component(axis) == 0.0
                            && (bounds.min(axis) >= mover.max(axis)
                                || bounds.max(axis) <= mover.min(axis))
                    }) {
                        continue;
                    }
                }
                let obb = self.obb(local);
                for (i, component) in components.into_iter().enumerate() {
                    if component == 0.0 {
                        continue;
                    }
                    let mut face = Obb {
                        centre: obb
                            .centre
                            .add_vector(self.frame.axes[i].scale(component.signum() * obb.half[i])),
                        half: obb.half,
                    };
                    face.half[i] = 0.0;
                    if let Some(time) = self.face_entry(&axes, mover, relative, &face, axis_aligned)
                    {
                        first = first.min(time);
                        if first == 0.0 {
                            return 1.0;
                        }
                    }
                }
            }
            1.0 - first
        })
    }

    /// First time the box crosses the advancing face. Strict overlap of the
    /// projections gives an open contact interval, including a face already
    /// inside the body at the start but not a graze that never advances into it.
    fn face_entry(
        &self,
        axes: &Axes,
        mover: Aabb,
        relative: Vec3,
        face: &Obb,
        stationary_axes_checked: bool,
    ) -> Option<f64> {
        let (mut enter, mut exit) = (0.0_f64, 1.0_f64);
        for axis in axes.iter() {
            let speed = relative.dot(axis);
            if speed == 0.0 && stationary_axes_checked {
                continue;
            }
            let (distance, radii) = self.extents(mover, face, axis);
            if speed == 0.0 {
                if distance.abs() >= radii {
                    return None;
                }
            } else {
                let t1 = (distance - radii) / speed;
                let t2 = (distance + radii) / speed;
                enter = enter.max(t1.min(t2));
                exit = exit.min(t1.max(t2));
                if enter >= exit {
                    return None;
                }
            }
        }
        Some(enter)
    }

    /// Whether the entity box overlaps any box deeper than the collision epsilon.
    pub fn intersects(&self, mover: Aabb) -> bool {
        let axes = self.axes(DEGENERATE_AXIS);
        self.with_candidates(mover, |candidates| {
            candidates.iter().any(|&index| {
                let obb = self.obb(self.boxes.boxes[index as usize]);
                self.overlap_depth(&axes, mover, &obb) > COLLISION_EPSILON
            })
        })
    }

    /// Calls `f` with the index of each box the entity box overlaps deeper than the collision
    /// epsilon, until it answers true.
    pub fn overlapping(&self, mover: Aabb, mut f: impl FnMut(u32) -> bool) {
        let axes = self.axes(DEGENERATE_AXIS);
        self.with_candidates(mover, |candidates| {
            for &index in candidates {
                let obb = self.obb(self.boxes.boxes[index as usize]);
                if self.overlap_depth(&axes, mover, &obb) > COLLISION_EPSILON && f(index) {
                    break;
                }
            }
        });
    }

    /// Whether the entity box overlaps box `index` of the collider deeper than the collision
    /// epsilon.
    pub fn overlaps_box(&self, index: u32, mover: Aabb) -> bool {
        let Some(&local) = self.boxes.boxes.get(index as usize) else {
            return false;
        };
        self.overlap_depth(&self.axes(DEGENERATE_AXIS), mover, &self.obb(local)) > COLLISION_EPSILON
    }

    /// The axes of the separating-axis test: the world axes, the box's, and their cross
    /// products (those not parallel), computed once for a query.
    fn axes(&self, cross_threshold: f64) -> Axes {
        let world = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let local = self.frame.axes;
        let mut list = [Vec3::ZERO; 15];
        let mut count = 0;
        for axis in world.into_iter().chain(local) {
            list[count] = axis;
            count += 1;
        }
        for w in world {
            for l in local {
                let c = w.cross(l);
                let n = c.length_squared();
                if n > cross_threshold {
                    list[count] = c.scale(1.0 / n.sqrt());
                    count += 1;
                }
            }
        }
        Axes { list, count }
    }

    /// Centre distance and summed radii of the two boxes on `axis`.
    fn extents(&self, mover: Aabb, obb: &Obb, axis: Vec3) -> (f64, f64) {
        let m = mover.center();
        let e = mover.size();
        let rm = e.x / 2.0 * axis.x.abs() + e.y / 2.0 * axis.y.abs() + e.z / 2.0 * axis.z.abs();
        let rb = (0..3)
            .map(|i| obb.half[i] * self.frame.axes[i].dot(axis).abs())
            .sum::<f64>();
        (obb.centre.dot(axis) - m.dot(axis), rm + rb)
    }

    /// The smallest overlap over all axes: positive when the boxes interpenetrate.
    fn overlap_depth(&self, axes: &Axes, mover: Aabb, obb: &Obb) -> f64 {
        axes.iter()
            .map(|axis| {
                let (distance, radii) = self.extents(mover, obb, axis);
                radii - distance.abs()
            })
            .fold(f64::INFINITY, f64::min)
    }

    /// How far along `direction` (one axis, signed) the box travels before meeting `obb`, if it
    /// does within the movement.
    fn time_of_impact(&self, axes: &Axes, mover: Aabb, direction: Vec3, obb: &Obb) -> Option<f64> {
        let amount = direction.x + direction.y + direction.z;
        let (mut enter, mut exit) = (f64::NEG_INFINITY, f64::INFINITY);
        // The travel distance the entering axis gives; exact when that axis is the movement's.
        let mut reach = 0.0;
        let mut embedded = true;
        for axis in axes.iter() {
            let (distance, radii) = self.extents(mover, obb, axis);
            if radii - distance.abs() <= COLLISION_EPSILON {
                embedded = false;
            }
            let speed = direction.dot(axis);
            if speed.abs() < 1.0e-12 {
                // No relative motion on this axis: it separates always or never.
                if distance.abs() >= radii - COLLISION_EPSILON {
                    return None;
                }
                continue;
            }
            // The mover's projection moves by `speed` per unit time; it overlaps while
            // `centre + speed * t` is within `radii` of the box's centre.
            let (n1, n2) = (distance - radii, distance + radii);
            let (t1, t2) = (n1 / speed, n2 / speed);
            let (lo, lo_num, hi) = if t1 < t2 { (t1, n1, t2) } else { (t2, n2, t1) };
            if lo > enter {
                enter = lo;
                let component = speed / amount;
                reach = if component.abs() == 1.0 {
                    lo_num / component
                } else {
                    amount * lo
                };
            }
            exit = exit.min(hi);
            if enter >= exit {
                return None;
            }
        }
        if embedded || exit <= 0.0 || enter >= 1.0 {
            return None;
        }
        // Float noise from a turned frame is not a movement.
        Some(if reach.abs() < 1.0e-9 { 0.0 } else { reach })
    }

    /// Heights, above the bottom of `step_box`, at which the box could stand on a face of this
    /// collider within `max_step`: the vertices' heights, and the heights of each upward face
    /// under the corners and centre of the box before and after the horizontal movement.
    pub fn step_heights(
        &self,
        step_box: Aabb,
        horizontal: Vec3,
        max_step: f64,
        out: &mut Vec<f64>,
    ) {
        let reach = step_box
            .expand_towards(Vec3::new(horizontal.x, max_step, horizontal.z))
            .expand_towards(Vec3::new(0.0, -COLLISION_EPSILON, 0.0));
        let base = step_box.min(Axis::Y);
        let mut push = |y: f64| {
            let h = y - base;
            if (0.0..=max_step).contains(&h) {
                out.push(h);
            }
        };
        let (lo, hi) = (step_box.min, step_box.max);
        // The box's corners and centre, before and after the horizontal movement, in the
        // collider's frame; and the world's up there: the same for every box.
        let mut probes = [Vec3::ZERO; 18];
        let mut count = 0;
        for dx in [0.0, horizontal.x] {
            for dz in [0.0, horizontal.z] {
                for (x, z) in [(lo.x, lo.z), (hi.x, lo.z), (lo.x, hi.z), (hi.x, hi.z)] {
                    probes[count] = self.frame.to_local(Vec3::new(x + dx, 0.0, z + dz));
                    count += 1;
                }
            }
        }
        let centre = step_box.center();
        for (dx, dz) in [(0.0, 0.0), (horizontal.x, horizontal.z)] {
            probes[count] = self
                .frame
                .to_local(Vec3::new(centre.x + dx, 0.0, centre.z + dz));
            count += 1;
        }
        let probes = &probes[..count];
        let up = self
            .frame
            .to_local(Vec3::new(0.0, 1.0, 0.0))
            .subtract(self.frame.to_local(Vec3::ZERO));
        self.with_candidates(reach, |candidates| {
            for &index in candidates {
                let local = self.boxes.boxes[index as usize];
                for i in 0..8 {
                    push(
                        self.frame
                            .to_world(Vec3::new(
                                if i & 1 == 0 {
                                    local.min(Axis::X)
                                } else {
                                    local.max(Axis::X)
                                },
                                if i & 2 == 0 {
                                    local.min(Axis::Y)
                                } else {
                                    local.max(Axis::Y)
                                },
                                if i & 4 == 0 {
                                    local.min(Axis::Z)
                                } else {
                                    local.max(Axis::Z)
                                },
                            ))
                            .y,
                    );
                }
                // Each face whose outward normal points up: its height at a sample column.
                for (axis, sign) in [
                    (0, 1.0),
                    (0, -1.0),
                    (1, 1.0),
                    (1, -1.0),
                    (2, 1.0),
                    (2, -1.0),
                ] {
                    let normal = self.frame.axes_for(axis).scale(sign);
                    if normal.y <= 1.0e-6 {
                        continue;
                    }
                    let face = Axis::ALL[axis];
                    let plane = if sign > 0.0 {
                        local.max(face)
                    } else {
                        local.min(face)
                    };
                    let d = up.component(face);
                    if d.abs() < 1.0e-9 {
                        continue;
                    }
                    for probe in probes {
                        // The point on the face's plane above the sample, then whether the box is there.
                        let t = (plane - probe.component(face)) / d;
                        let p = probe.add_vector(up.scale(t));
                        let inside = Axis::ALL.iter().all(|a| {
                            *a == face
                                || (p.component(*a) >= local.min(*a) - COLLISION_EPSILON
                                    && p.component(*a) <= local.max(*a) + COLLISION_EPSILON)
                        });
                        if inside {
                            push(t);
                        }
                    }
                }
            }
        });
    }
}

impl Frame {
    fn axes_for(&self, index: usize) -> Vec3 {
        self.axes[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(min: [f64; 3], max: [f64; 3]) -> Aabb {
        Aabb::new(
            Vec3::new(min[0], min[1], min[2]),
            Vec3::new(max[0], max[1], max[2]),
        )
    }

    fn yaw(origin: Vec3, degrees: f64) -> Frame {
        let half = degrees.to_radians() / 2.0;
        Frame::from_quaternion(origin, [0.0, half.sin(), 0.0, half.cos()])
    }

    #[test]
    fn the_frame_round_trips_and_turns_by_the_quaternion() {
        let frame = yaw(Vec3::new(10.0, 64.0, -3.0), 90.0);
        let local = Vec3::new(2.0, 1.0, 5.0);
        let back = frame.to_local(frame.to_world(local));
        assert!(back.distance(local) < 1.0e-12);
        // A quarter turn about y sends local +z to world +x.
        let w = frame
            .to_world(Vec3::new(0.0, 0.0, 1.0))
            .subtract(frame.origin);
        assert!(w.distance(Vec3::new(1.0, 0.0, 0.0)) < 1.0e-12);
        // A non-unit quaternion is normalised; a zero one is no turn.
        let big = Frame::from_quaternion(
            Vec3::ZERO,
            [0.0, 2.0 * 0.5_f64.sqrt(), 0.0, 2.0 * 0.5_f64.sqrt()],
        );
        assert!(
            big.to_world(Vec3::new(0.0, 0.0, 1.0))
                .distance(Vec3::new(1.0, 0.0, 0.0))
                < 1.0e-12
        );
        assert_eq!(
            Frame::from_quaternion(Vec3::ZERO, [0.0; 4]),
            Frame::translation(Vec3::ZERO)
        );
    }

    /// Against a box in an unturned frame the sweep answers as the block shape does.
    #[test]
    fn an_unturned_collider_stops_the_box_where_a_block_shape_would() {
        let wall = ColliderBoxes::new(vec![b([4.0, 0.0, 0.0], [5.0, 3.0, 3.0])]);
        let collider = OrientedCollider::new(Frame::translation(Vec3::ZERO), Arc::new(wall));
        let mover = b([0.0, 0.0, 1.0], [0.6, 1.8, 1.6]);
        let reference =
            super::super::VoxelShape::from_box(b([4.0, 0.0, 0.0], [5.0, 3.0, 3.0])).unwrap();
        for amount in [1.0, 3.0, 3.4, 5.0, 10.0] {
            let (got, want) = (
                collider.collide(mover, Axis::X, amount),
                reference.collide(Axis::X, mover, amount),
            );
            assert!(
                (got - want).abs() < 1.0e-12,
                "amount {amount}: {got} vs {want}"
            );
        }
        // Away from the wall nothing stops it.
        assert_eq!(collider.collide(mover, Axis::X, -5.0), -5.0);
    }

    #[test]
    fn a_translating_collider_sweeps_only_its_advancing_face() {
        let boxes = Arc::new(ColliderBoxes::new(vec![b([0.0; 3], [1.0; 3])]));
        let delta = Vec3::new(1.0, 0.0, 0.0);
        let start = OrientedCollider::new(Frame::translation(Vec3::ZERO), boxes.clone());
        let end = OrientedCollider::new(Frame::translation(delta), boxes);
        let push = |min, max| start.translation_push_fraction(&end, b(min, max), delta);
        assert_eq!(push([0.875, 0.25, 0.25], [1.125, 0.75, 0.75]), 1.0);
        assert_eq!(push([-0.125, 0.25, 0.25], [0.125, 0.75, 0.75]), 0.0);
        assert_eq!(
            push([1.25, 1.0_f64.next_down(), 0.25], [1.5, 1.25, 0.75]),
            0.75
        );
        assert_eq!(push([1.25, 1.0, 0.25], [1.5, 1.25, 0.75]), 0.0);
    }

    #[test]
    fn a_translating_face_keeps_world_overlap_after_rebasing() {
        let source = b([0.0, 1.0, 0.0], [1.0, 2.0, 1.0]);
        // With and without a second box extending the overall bounds beyond the contact.
        // The real overlap must survive both the aggregate and per-box candidate filters.
        for boxes in [
            vec![source],
            vec![source, b([0.0, 2.0, 2.0], [1.0, 3.0, 3.0])],
        ] {
            let boxes = Arc::new(ColliderBoxes::new(boxes));
            let origin = Vec3::new(0.0, -1.0, 0.0);
            let delta = Vec3::new(1.0, 0.0, 0.0);
            let start = OrientedCollider::new(Frame::translation(origin), boxes.clone());
            let end = OrientedCollider::new(Frame::translation(origin.add_vector(delta)), boxes);
            let push = |y| {
                start.translation_push_fraction(&end, b([1.25, y, 0.25], [1.5, 1.25, 0.75]), delta)
            };
            assert_eq!(push(1.0_f64.next_down()), 0.75);
            assert_eq!(push(1.0), 0.0);
        }
    }

    #[test]
    fn a_translating_face_keeps_one_ulp_of_forward_overlap() {
        let boxes = Arc::new(ColliderBoxes::new(vec![b(
            [-1.0, 0.0, 0.0],
            [2.0_f64.powi(-55), 1.0, 1.0],
        )]));
        let origin = Vec3::new(-1.0, 0.0, 0.0);
        let delta = Vec3::new(0.5, 0.0, 0.0);
        let start = OrientedCollider::new(Frame::translation(origin), boxes.clone());
        let end = OrientedCollider::new(Frame::translation(origin.add_vector(delta)), boxes);
        let push = |x| {
            start.translation_push_fraction(&end, b([x, 0.0, 0.0], [x + 0.25, 1.0, 1.0]), delta)
        };
        assert_eq!(push((-0.5_f64).next_down()), 2.0_f64.powi(-52));
        assert_eq!(push(-0.5), 0.0);
    }

    /// A box turned 45 degrees about y shows a corner to a mover coming along x: the mover
    /// stops where its face meets the corner, `centre - sqrt(2)/2 * side` away.
    #[test]
    fn a_turned_box_stops_the_mover_at_its_corner() {
        let boxes = ColliderBoxes::new(vec![b([-1.0, 0.0, -1.0], [1.0, 2.0, 1.0])]);
        let collider = OrientedCollider::new(yaw(Vec3::new(10.0, 0.0, 0.0), 45.0), Arc::new(boxes));
        let mover = b([0.0, 0.0, -0.3], [0.6, 1.8, 0.3]);
        let corner = 10.0 - 2.0_f64.sqrt();
        let expected = corner - 0.6;
        let got = collider.collide(mover, Axis::X, 20.0);
        assert!((got - expected).abs() < 1.0e-9, "{got} vs {expected}");
        // Off to the side by more than the diamond's reach there is no contact.
        let aside = mover.move_by(Vec3::new(0.0, 0.0, 2.0));
        assert_eq!(collider.collide(aside, Axis::X, 20.0), 20.0);
    }

    #[test]
    fn a_pitched_deck_is_landed_on_at_the_height_of_its_surface() {
        // A deck 4 long, 1 thick, pitched 20 degrees about x, its origin at y = 10.
        let boxes = ColliderBoxes::new(vec![b([-2.0, -1.0, -2.0], [2.0, 0.0, 2.0])]);
        let half = 20.0_f64.to_radians() / 2.0;
        let frame = Frame::from_quaternion(
            Vec3::new(0.0, 10.0, 0.0),
            [half.sin(), 0.0, 0.0, half.cos()],
        );
        let collider = OrientedCollider::new(frame, Arc::new(boxes));
        // A thin mover over the origin: the top face passes through the origin.
        let mover = b([-0.05, 12.0, -0.05], [0.05, 13.8, 0.05]);
        let fall = collider.collide(mover, Axis::Y, -5.0);
        // Its lowest corner over the face: the face rises toward -z at 20 degrees, so the
        // corner at z = -0.05 meets it highest.
        let top = |z: f64| 10.0 - z * 20.0_f64.to_radians().tan();
        let expected = top(-0.05) - 12.0;
        assert!((fall - expected).abs() < 1.0e-9, "{fall} vs {expected}");
    }

    #[test]
    fn standing_on_a_top_is_not_stopped_sideways_and_does_not_fall_through() {
        let boxes = ColliderBoxes::new(vec![b([-1.5, -1.0, -10.0], [1.5, 0.0, 10.0])]);
        let collider = OrientedCollider::new(yaw(Vec3::new(0.0, 64.0, 0.0), 30.0), Arc::new(boxes));
        let stand = collider.frame.to_world(Vec3::new(0.0, 0.0, 3.0));
        let mover = b(
            [stand.x - 0.3, stand.y, stand.z - 0.3],
            [stand.x + 0.3, stand.y + 1.8, stand.z + 0.3],
        );
        assert_eq!(collider.collide(mover, Axis::Y, -0.5), 0.0);
        assert_eq!(collider.collide(mover, Axis::X, 0.4), 0.4);
        assert_eq!(collider.collide(mover, Axis::Z, -0.4), -0.4);
        assert_eq!(collider.collide(mover, Axis::Y, 0.5), 0.5);
        assert!(!collider.intersects(mover));
    }

    #[test]
    fn an_embedded_mover_is_not_frozen_and_a_graze_is_not_embedded() {
        let boxes = ColliderBoxes::new(vec![b([0.0, 0.0, 0.0], [4.0, 1.0, 4.0])]);
        let collider = OrientedCollider::new(Frame::translation(Vec3::ZERO), Arc::new(boxes));
        let inside = b([1.0, 0.2, 1.0], [1.6, 2.0, 1.6]);
        assert!(collider.intersects(inside));
        assert_eq!(collider.collide(inside, Axis::X, 1.0), 1.0);
        let touching = b([1.0, 1.0, 1.0], [1.6, 2.8, 1.6]);
        assert!(!collider.intersects(touching));
    }

    #[test]
    fn the_grid_finds_what_a_scan_of_every_box_finds() {
        // A long train of boxes, a few of them oversized, queried at many places.
        let mut list = Vec::new();
        for i in 0..400 {
            let z = f64::from(i) * 1.0;
            list.push(b([-1.5, 0.0, z], [1.5, 1.0, z + 1.0]));
        }
        list.push(b([-600.0, 5.0, -600.0], [600.0, 6.0, 600.0]));
        let grid = ColliderBoxes::new(list.clone());
        let mut found = Vec::new();
        for z in [-5.0, 0.0, 3.5, 100.0, 399.5, 500.0] {
            let query = b([-0.5, 0.0, z], [0.5, 1.8, z + 0.6]);
            grid.candidates(query, &mut found);
            for (i, bx) in list.iter().enumerate() {
                if bx.intersects(query) {
                    assert!(found.contains(&(i as u32)), "z {z} box {i}");
                }
            }
        }
        assert!(
            found.len() < 20,
            "a point query visits few boxes: {}",
            found.len()
        );
    }

    #[test]
    fn step_heights_follow_a_tilted_top_face() {
        let boxes = ColliderBoxes::new(vec![b([-2.0, -1.0, -2.0], [2.0, 0.0, 2.0])]);
        let half = 10.0_f64.to_radians() / 2.0;
        let frame = Frame::from_quaternion(
            Vec3::new(0.0, 64.0, 0.0),
            [half.sin(), 0.0, 0.0, half.cos()],
        );
        let collider = OrientedCollider::new(frame, Arc::new(boxes));
        // The mover stands just off the low edge; the surface ahead is a little higher.
        let mover = b([-0.3, 64.0, 0.2], [0.3, 65.8, 0.8]);
        let mut heights = Vec::new();
        collider.step_heights(mover, Vec3::new(0.0, 0.0, -0.5), 0.6, &mut heights);
        let ahead = 64.0 + 0.3 * 10.0_f64.to_radians().tan() - 64.0;
        assert!(
            heights.iter().any(|h| (h - ahead).abs() < 1.0e-6),
            "{heights:?} lacks {ahead}"
        );
    }

    /// A box with no thickness is a box with no volume: nothing stops at it, unturned or turned,
    /// and nothing steps onto it. Its index stays, so the boxes after it keep theirs.
    #[test]
    fn a_box_with_no_volume_collides_with_nothing() {
        let wall = b([1.0, 0.0, 0.0], [1.0, 2.0, 2.0]);
        let solid = b([5.0, 0.0, 0.0], [6.0, 2.0, 2.0]);
        let mover = b([-0.6, 0.0, 0.5], [-0.2, 1.8, 1.0]);
        let flat = b([0.0, 1.0, 0.0], [3.0, 1.0, 3.0]);
        for frame in [
            Frame::translation(Vec3::ZERO),
            yaw(Vec3::new(0.3, 0.0, 0.1), 30.0),
        ] {
            let boxes = ColliderBoxes::new(vec![wall, flat, solid]);
            assert_eq!(boxes.len(), 3, "the indices stay");
            let collider = OrientedCollider::new(frame, Arc::new(boxes));
            let moved = collider.collide(mover, Axis::X, 4.0);
            // Only the solid box at x = 5 is met, whatever the wall's side of the frame.
            let at = frame.to_world(Vec3::new(5.5, 1.0, 1.0));
            let before = b([at.x - 3.2, 0.0, at.z - 0.3], [at.x - 2.8, 1.8, at.z + 0.3]);
            let ahead = collider.collide(before, Axis::X, 3.0);
            assert!(ahead < 3.0, "the solid box still stops: {ahead}");
            let free = OrientedCollider::new(frame, Arc::new(ColliderBoxes::new(vec![wall, flat])));
            assert_eq!(free.collide(mover, Axis::X, 4.0), 4.0, "{moved}");
            assert!(!free.intersects(b([0.5, 0.5, 0.5], [1.5, 1.5, 1.5])));
            let mut heights = Vec::new();
            free.step_heights(mover, Vec3::new(1.0, 0.0, 0.0), 2.0, &mut heights);
            assert!(heights.is_empty(), "{heights:?}");
        }
    }

    /// One check for a box and a rotation, whichever way they arrive.
    #[test]
    fn the_contract_check_refuses_by_name() {
        assert_eq!(check_box([-512.0; 3], [512.0; 3]), Ok(()));
        assert_eq!(
            check_box([0.0; 3], [512.5, 1.0, 1.0]),
            Err(BoxFault::OutOfRange)
        );
        assert_eq!(
            check_box(
                [-f64::from(f32::MAX), 0.0, 0.0],
                [f64::from(f32::MAX), 1.0, 1.0]
            ),
            Err(BoxFault::OutOfRange)
        );
        assert_eq!(
            check_box([0.0, f64::NAN, 0.0], [1.0; 3]),
            Err(BoxFault::NonFinite)
        );
        assert!(check_rotation([0.0, 0.0, 0.0, 1.0005]));
        assert!(!check_rotation([0.0, 0.0, 0.0, 1.01]));
        assert!(!check_rotation([0.0, 0.0, 0.0, 0.0]));
        assert!(!check_rotation([f64::INFINITY, 0.0, 0.0, 1.0]));
    }

    /// The grid's cell is picked from the boxes alone, inside its bounds, and whatever its size a
    /// query names every box it meets.
    #[test]
    fn the_cell_follows_the_boxes_and_the_grid_names_every_box_a_query_meets() {
        let cube = |n: usize, size: f64| {
            (0..n * n * n)
                .map(|i| {
                    let at = |k: usize| k as f64 * size;
                    let (x, y, z) = (i % n, i / n % n, i / n / n);
                    b(
                        [at(x), at(y), at(z)],
                        [at(x) + size, at(y) + size, at(z) + size],
                    )
                })
                .collect::<Vec<_>>()
        };
        let sets = [
            (cube(6, 1.0), 2.0),
            (cube(4, 0.125), CELL_MIN),
            (cube(3, 40.0), CELL_MAX),
            (vec![b([0.0, 0.0, 0.0], [3.0, 1.0, 20.0])], CELL_MAX),
            (Vec::new(), 4.0),
        ];
        let mut state = 0x1234_5678_9abc_def1u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for (boxes, side) in sets {
            let grid = ColliderBoxes::new(boxes.clone());
            assert_eq!(grid.cell, side);
            let mut out = Vec::new();
            for _ in 0..500 {
                let lo = Vec3::new(
                    next() * 30.0 - 5.0,
                    next() * 30.0 - 5.0,
                    next() * 30.0 - 5.0,
                );
                let query = Aabb::new(
                    lo,
                    lo.add_vector(Vec3::new(next() * 3.0, next() * 3.0, next() * 3.0)),
                );
                grid.candidates(query, &mut out);
                for (index, bx) in boxes.iter().enumerate() {
                    if bx.intersects(query) {
                        assert!(
                            out.contains(&(index as u32)),
                            "{index}: {bx:?} in {query:?}"
                        );
                    }
                }
            }
        }
    }
}
