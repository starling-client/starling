//! Entity geometry helpers operating on an already selected collider list.
use super::{Aabb, Axis, COLLISION_EPSILON, Vec3, VoxelShape, java_max, java_min};

/// What Entity.collide asks of a collider: VoxelShape.collide along one axis,
/// and the coordinates its step-up heights come from.
pub trait Collides {
    fn collide(&self, axis: Axis, aabb: Aabb, amount: f64) -> f64;
    fn coordinates(&self, axis: Axis) -> &[f64];
}
impl Collides for VoxelShape {
    fn collide(&self, axis: Axis, aabb: Aabb, amount: f64) -> f64 {
        VoxelShape::collide(self, axis, aabb, amount)
    }
    fn coordinates(&self, axis: Axis) -> &[f64] {
        VoxelShape::coordinates(self, axis)
    }
}
impl<T: Collides + ?Sized> Collides for &T {
    fn collide(&self, axis: Axis, aabb: Aabb, amount: f64) -> f64 {
        (**self).collide(axis, aabb, amount)
    }
    fn coordinates(&self, axis: Axis) -> &[f64] {
        (**self).coordinates(axis)
    }
}

/// `Shapes.block().move(offset)` without building it: the moved full block
/// is an array shape over `0 + offset` and `1 + offset` on each axis, and
/// collides through the same index lookups over those two coordinates.
/// Most of a mob's colliders are full blocks; the shape would cost each one
/// several allocations every tick.
#[derive(Clone, Copy, Debug)]
pub struct MovedBlock {
    offset: Vec3,
    coords: [[f64; 2]; 3],
}
impl MovedBlock {
    pub fn new(offset: Vec3) -> Self {
        let coords = Axis::ALL.map(|axis| {
            let at = offset.component(axis);
            [0.0 + at, 1.0 + at]
        });
        Self { offset, coords }
    }
    /// The shape itself, as `VoxelShape::block().moved(offset)`.
    pub fn to_shape(&self) -> VoxelShape {
        VoxelShape::block().moved(self.offset)
    }
    /// VoxelShape.findIndex: the upper-bound search over two coordinates.
    /// A NaN value is past both, as the search's `value < coordinate` says.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn find_index(&self, axis: Axis, value: f64) -> i32 {
        let [low, high] = self.coords[axis as usize];
        if !(value < high) {
            1
        } else if value < low {
            -1
        } else {
            0
        }
    }
}
impl Collides for MovedBlock {
    /// VoxelShape.collide over the block's single full cell: a layer is
    /// occupied when the box's cell ranges on the other two axes meet it.
    fn collide(&self, axis: Axis, aabb: Aabb, amount: f64) -> f64 {
        if amount.abs() < COLLISION_EPSILON {
            return 0.0;
        }
        let i = axis as usize;
        let meets = |other: Axis| {
            self.find_index(other, aabb.min(other) + COLLISION_EPSILON)
                .max(0)
                < (self.find_index(other, aabb.max(other) - COLLISION_EPSILON) + 1).min(1)
        };
        let occupied = || meets(Axis::ALL[(i + 1) % 3]) && meets(Axis::ALL[(i + 2) % 3]);
        if amount > 0.0 {
            // The layers from the one above the box's top: the cell, if it is there.
            if self.find_index(axis, aabb.max(axis) - COLLISION_EPSILON) < 0 && occupied() {
                let distance = self.coords[i][0] - aabb.max(axis);
                return if distance >= -COLLISION_EPSILON {
                    java_min(amount, distance)
                } else {
                    amount
                };
            }
        } else if amount < 0.0
            && self.find_index(axis, aabb.min(axis) + COLLISION_EPSILON) >= 1
            && occupied()
        {
            let distance = self.coords[i][1] - aabb.min(axis);
            return if distance <= COLLISION_EPSILON {
                java_max(amount, distance)
            } else {
                amount
            };
        }
        amount
    }
    fn coordinates(&self, axis: Axis) -> &[f64] {
        &self.coords[axis as usize]
    }
}

/// One of Entity.collide's colliders: a moved full block, or any other shape.
#[derive(Clone, Debug)]
pub enum Collider {
    Block(MovedBlock),
    Shape(VoxelShape),
}
impl Collider {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Block(_) => false,
            Self::Shape(shape) => shape.is_empty(),
        }
    }
    pub fn into_shape(self) -> VoxelShape {
        match self {
            Self::Block(block) => block.to_shape(),
            Self::Shape(shape) => shape,
        }
    }
}
impl Collides for Collider {
    fn collide(&self, axis: Axis, aabb: Aabb, amount: f64) -> f64 {
        match self {
            Self::Block(block) => block.collide(axis, aabb, amount),
            Self::Shape(shape) => VoxelShape::collide(shape, axis, aabb, amount),
        }
    }
    fn coordinates(&self, axis: Axis) -> &[f64] {
        match self {
            Self::Block(block) => block.coordinates(axis),
            Self::Shape(shape) => VoxelShape::coordinates(shape, axis),
        }
    }
}

/// BlockGetter's floor-height decision for caller-selected collision shapes.
/// The lower block is queried lazily only if the current shape is empty.
pub fn block_floor_height<'s>(shape: &VoxelShape, below: impl FnOnce() -> &'s VoxelShape) -> f64 {
    if !shape.is_empty() {
        return shape.max(Axis::Y);
    }
    let height = below().max(Axis::Y);
    if height >= 1.0 {
        height - 1.0
    } else {
        f64::NEG_INFINITY
    }
}

/// Bounds used once by Entity.collide to select entity collisions. The same
/// entity list is reused for both subsequent block/world-border queries.
pub fn entity_collision_bounds(movement: Vec3, aabb: Aabb, max_step: f32) -> Aabb {
    aabb.expand_towards(movement)
        .expand_towards(Vec3::new(0.0, max_step as f64, 0.0))
}

/// Resolve Entity.collide's initial movement and complete step-up decision.
///
/// `collect` supplies vanilla-equivalent colliders for each requested bound,
/// including the previously selected entity shapes, applicable world border,
/// and context-dependent block shapes in vanilla order. It is called for the
/// initial movement (unless its squared length is zero), then at most once
/// more if stepping is eligible. Entity collision selection and movement's
/// grounding/velocity/inside-block side effects belong to the caller.
pub fn resolve_step_movement<'s>(
    movement: Vec3,
    aabb: Aabb,
    max_step: f32,
    on_ground: bool,
    mut collect: impl FnMut(Aabb) -> Vec<&'s VoxelShape>,
) -> Vec3 {
    try_resolve_step_movement(movement, aabb, max_step, on_ground, |bounds| {
        Ok::<_, std::convert::Infallible>(collect(bounds))
    })
    .unwrap_or_else(|never| match never {})
}

/// The same movement/step decision with fallible, owned or borrowed collider
/// collections. A provider failure stops before the next query and returns no
/// movement result. Existing world-query side effects are not rolled back.
pub fn try_resolve_step_movement<S: Collides, E>(
    movement: Vec3,
    aabb: Aabb,
    max_step: f32,
    on_ground: bool,
    mut collect: impl FnMut(Aabb) -> Result<Vec<S>, E>,
) -> Result<Vec3, E> {
    let resolved = if movement.length_squared() == 0.0 {
        movement
    } else {
        let shapes = collect(aabb.expand_towards(movement))?;
        resolve_borrowed_shape_movement(movement, aabb, &shapes)
    };
    let landed = movement.y != resolved.y && movement.y < 0.0;
    if max_step > 0.0
        && (landed || on_ground)
        && (movement.x != resolved.x || movement.z != resolved.z)
    {
        let step_box = if landed {
            aabb.move_by(Vec3::new(0.0, resolved.y, 0.0))
        } else {
            aabb
        };
        let mut query = step_box.expand_towards(Vec3::new(movement.x, max_step as f64, movement.z));
        if !landed {
            query = query.expand_towards(Vec3::new(0.0, -(1.0e-5_f32 as f64), 0.0));
        }
        let shapes = collect(query)?;
        for height in borrowed_step_heights(step_box, &shapes, max_step, resolved.y as f32) {
            let candidate = resolve_borrowed_shape_movement(
                Vec3::new(movement.x, height as f64, movement.z),
                step_box,
                &shapes,
            );
            if candidate.horizontal_distance_squared() > resolved.horizontal_distance_squared() {
                return Ok(candidate.subtract(Vec3::new(
                    0.0,
                    aabb.min(Axis::Y) - step_box.min(Axis::Y),
                    0.0,
                )));
            }
        }
    }
    Ok(resolved)
}

/// Resolve vanilla's Y-first, magnitude-dependent horizontal axis ordering.
///
/// The caller must supply vanilla-equivalent block/entity/world-border shapes.
/// This helper does not collect shapes, step up, set grounded flags, modify
/// velocity, or apply inside-block effects.
pub fn resolve_shape_movement(movement: Vec3, aabb: Aabb, shapes: &[&VoxelShape]) -> Vec3 {
    resolve_borrowed_shape_movement(movement, aabb, shapes)
}
fn resolve_borrowed_shape_movement<S: Collides>(movement: Vec3, aabb: Aabb, shapes: &[S]) -> Vec3 {
    if shapes.is_empty() {
        return movement;
    }
    let mut result = [0.0; 3];
    for axis in Axis::step_order(movement) {
        let amount = movement.component(axis);
        if amount == 0.0 {
            continue;
        }
        let moved = aabb.move_by(Vec3::new(result[0], result[1], result[2]));
        result[axis as usize] = collide_all(axis, moved, shapes, amount);
    }
    Vec3::new(result[0], result[1], result[2])
}
/// Shapes.collide: VoxelShape.collide_all over any colliders.
fn collide_all<S: Collides>(axis: Axis, aabb: Aabb, shapes: &[S], mut amount: f64) -> f64 {
    for shape in shapes {
        if amount.abs() < COLLISION_EPSILON {
            return 0.0;
        }
        amount = shape.collide(axis, aabb, amount);
    }
    amount
}

/// Candidate heights from Entity.collectCandidateStepUpHeights, including its
/// float narrowing, signed-zero set identity, current-Y exclusion and sorting.
/// Used by the step-up decision after collecting the appropriate world shapes.
pub fn candidate_step_heights(
    aabb: Aabb,
    shapes: &[&VoxelShape],
    max_step: f32,
    current_y: f32,
) -> Vec<f32> {
    borrowed_step_heights(aabb, shapes, max_step, current_y)
}

fn borrowed_step_heights<S: Collides>(
    aabb: Aabb,
    shapes: &[S],
    max_step: f32,
    current_y: f32,
) -> Vec<f32> {
    let mut heights = Vec::<f32>::with_capacity(4);
    for shape in shapes {
        for coordinate in shape.coordinates(Axis::Y) {
            let height = (*coordinate - aabb.min(Axis::Y)) as f32;
            if height < 0.0 || height == current_y {
                continue;
            }
            if height > max_step {
                break;
            }
            // FloatArraySet uses Float.floatToIntBits, so NaNs compare equal
            // while positive and negative zero remain separate members.
            let height = if height.is_nan() { f32::NAN } else { height };
            if !heights.iter().any(|old| old.to_bits() == height.to_bits()) {
                heights.push(height);
            }
        }
    }
    heights.sort_unstable_by(f32::total_cmp);
    heights
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::BlockPos;
    #[test]
    fn movement_slides_after_a_wall_collision() {
        let wall = VoxelShape::block();
        let b = Aabb::new(Vec3::new(-0.6, 0.0, 0.2), Vec3::new(-0.2, 1.8, 0.8));
        let requested = Vec3::new(1.0, 0.0, 0.1);
        assert_eq!(
            resolve_shape_movement(requested, b, &[&wall]),
            Vec3::new(0.2, 0.0, 0.1)
        );
        assert_eq!(
            resolve_shape_movement(Vec3::new(-0.0, -0.0, -0.0), b, &[]),
            Vec3::new(-0.0, -0.0, -0.0)
        );
        assert_eq!(
            resolve_shape_movement(Vec3::new(-0.0, -0.0, -0.0), b, &[&wall]),
            Vec3::ZERO
        );
    }
    #[test]
    fn step_heights_deduplicate_and_exclude_current_y() {
        let shape = VoxelShape::from_cube([1, 2, 1], &[true, true]).unwrap();
        let b = Aabb::block(BlockPos::default());
        assert_eq!(
            candidate_step_heights(b, &[&shape, &shape], 0.6, 0.0),
            vec![0.5]
        );
        assert_eq!(
            candidate_step_heights(b, &[&shape], 1.0, 0.5),
            vec![0.0, 1.0]
        );
    }

    #[test]
    fn stepping_requeries_and_clears_a_half_block() {
        let ledge = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0))).unwrap();
        let b = Aabb::new(Vec3::new(-0.6, 0.0, 0.2), Vec3::new(-0.2, 1.8, 0.8));
        let requested = Vec3::new(1.0, 0.0, 0.0);
        let mut queries = Vec::new();
        let result = resolve_step_movement(requested, b, 0.6, true, |query| {
            queries.push(query);
            vec![&ledge]
        });
        assert_eq!(result, Vec3::new(1.0, 0.5, 0.0));
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0], b.expand_towards(requested));
        assert_eq!(queries[1].min(Axis::Y), -(1.0e-5_f32 as f64));
        assert_eq!(
            resolve_step_movement(requested, b, 0.6, false, |_| vec![&ledge]),
            Vec3::new(0.2, 0.0, 0.0)
        );
    }

    #[test]
    fn squared_length_underflow_skips_collection_and_keeps_signed_zero() {
        let b = Aabb::block(BlockPos::default());
        let requested = Vec3::new(-0.0, 1e-200, -0.0);
        assert_eq!(
            resolve_step_movement(requested, b, 0.6, true, |_| panic!("unexpected query")),
            requested
        );
    }
    #[test]
    fn floor_height_defers_the_lower_lookup_until_empty() {
        let empty = VoxelShape::empty();
        let block = VoxelShape::block();
        assert_eq!(
            block_floor_height(&block, || panic!("unnecessary lower lookup")),
            1.0
        );
        assert_eq!(block_floor_height(&empty, || &block), 0.0);
        assert_eq!(block_floor_height(&empty, || &empty), f64::NEG_INFINITY);
    }
}

#[cfg(test)]
mod fallible_tests {
    use super::*;
    #[test]
    fn owned_collision_queries_stop_at_first_failure() {
        let bounds = Aabb::new(Vec3::new(-0.6, 0.0, 0.2), Vec3::new(-0.2, 1.8, 0.8));
        let ledge = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0))).unwrap();
        for fail in [0, 1] {
            let mut queries = 0;
            let result =
                try_resolve_step_movement(Vec3::new(1.0, 0.0, 0.0), bounds, 0.6, true, |_| {
                    let call = queries;
                    queries += 1;
                    if call == fail {
                        Err(call)
                    } else {
                        Ok(vec![ledge.clone()])
                    }
                });
            assert_eq!(result, Err(fail));
            assert_eq!(queries, fail + 1);
        }
        let result = try_resolve_step_movement(
            Vec3::new(-0.0, -0.0, -0.0),
            bounds,
            0.6,
            true,
            |_| -> Result<Vec<VoxelShape>, ()> {
                panic!("zero movement must skip shape collection")
            },
        )
        .unwrap();
        assert_eq!(
            [result.x.to_bits(), result.y.to_bits(), result.z.to_bits()],
            [(-0.0f64).to_bits(); 3]
        );
    }
}

#[cfg(test)]
mod moved_block_tests {
    use super::*;
    /// xorshift64*: deterministic inputs without a dependency.
    struct Inputs(u64);
    impl Inputs {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn pick<T: Copy>(&mut self, from: &[T]) -> T {
            from[(self.next() % from.len() as u64) as usize]
        }
        /// A coordinate near the block's faces, where the epsilon lookups turn.
        fn near(&mut self, face: f64) -> f64 {
            let nudge = self.pick(&[
                0.0, 1.0e-7, -1.0e-7, 2.0e-7, -2.0e-7, 1.0e-9, -1.0e-9, 5.0e-8, -5.0e-8, 0.3, -0.3,
                0.6, -0.6, 1.0, -1.0, 0.5,
            ]);
            match self.next() % 8 {
                0 => face + (self.next() % 1000) as f64 / 250.0 - 2.0,
                1 => f64::from_bits(face.to_bits().wrapping_add(self.next() % 5).wrapping_sub(2)),
                _ => face + nudge,
            }
        }
    }
    #[test]
    fn moved_blocks_collide_as_the_moved_shape() {
        let mut inputs = Inputs(0x9E37_79B9_7F4A_7C15);
        let offsets = [
            0.0,
            1.0,
            -1.0,
            7.0,
            -64.0,
            319.0,
            30_000_000.0,
            -30_000_000.0,
            f64::from(i32::MAX),
            f64::from(i32::MIN),
        ];
        let amounts = [
            0.0,
            -0.0,
            1.0e-7,
            -1.0e-7,
            9.9e-8,
            -9.9e-8,
            0.08,
            -0.08,
            -0.0784,
            0.6,
            -0.6,
            1.0,
            -1.0,
            3.5,
            -3.5,
            1.0e-300,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        // Cases a block stopped short of the asked amount (not just let through).
        let mut stopped = 0;
        for _ in 0..2_000_000 {
            let offset = Vec3::new(
                inputs.pick(&offsets) + (inputs.next() % 5) as f64 - 2.0,
                inputs.pick(&offsets) + (inputs.next() % 5) as f64 - 2.0,
                inputs.pick(&offsets) + (inputs.next() % 5) as f64 - 2.0,
            );
            let block = MovedBlock::new(offset);
            let shape = VoxelShape::block().moved(offset);
            let corner = |inputs: &mut Inputs, axis: usize| {
                let at = offset.components()[axis];
                let face = inputs.pick(&[at, at + 1.0]);
                inputs.near(face)
            };
            let a: [f64; 3] = std::array::from_fn(|axis| corner(&mut inputs, axis));
            let b: [f64; 3] = std::array::from_fn(|axis| corner(&mut inputs, axis));
            let aabb = Aabb::new(Vec3::new(a[0], a[1], a[2]), Vec3::new(b[0], b[1], b[2]));
            let amount = match inputs.next() % 3 {
                0 => (inputs.next() % 4001) as f64 / 1000.0 - 2.0,
                _ => inputs.pick(&amounts),
            };
            for axis in Axis::ALL {
                let expected = shape.collide(axis, aabb, amount);
                let got = Collides::collide(&block, axis, aabb, amount);
                assert_eq!(
                    got.to_bits(),
                    expected.to_bits(),
                    "{axis:?} {aabb:?} {amount} at {offset:?}: {got} != {expected}"
                );
                assert_eq!(Collides::coordinates(&block, axis), shape.coordinates(axis));
                stopped += usize::from(expected.to_bits() != amount.to_bits());
            }
        }
        assert!(stopped > 200_000, "{stopped}");
    }
    #[test]
    fn moved_blocks_resolve_and_step_as_shapes() {
        let ledge = VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0))).unwrap();
        let b = Aabb::new(Vec3::new(-0.6, 0.0, 0.2), Vec3::new(-0.2, 1.8, 0.8));
        for offset in [
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ] {
            for (movement, grounded) in [
                (Vec3::new(1.0, 0.0, 0.1), true),
                (Vec3::new(0.3, -0.08, 0.0), false),
                (Vec3::new(1.0, -0.0784, 0.2), true),
            ] {
                let shapes = [ledge.clone(), VoxelShape::block().moved(offset)];
                let colliders = [
                    Collider::Shape(ledge.clone()),
                    Collider::Block(MovedBlock::new(offset)),
                ];
                let expected =
                    resolve_step_movement(movement, b, 0.6, grounded, |_| shapes.iter().collect());
                let got = try_resolve_step_movement(movement, b, 0.6, grounded, |_| {
                    Ok::<_, ()>(colliders.iter().collect::<Vec<_>>())
                })
                .unwrap();
                assert_eq!(
                    got.components().map(f64::to_bits),
                    expected.components().map(f64::to_bits)
                );
            }
        }
    }
}
