//! Entity.collide collection order and step resolution over a collision world.
//! Travel, velocity, grounded flags and inside-block effects remain caller work.
use crate::{
    block_collision_scan::{
        BlockCollisionScan, BlockCollisionWorld, CollisionScanError, scan_cells,
    },
    collision_context::CollisionContext,
    entity_obstruction::EntityIdentity,
};
use sunrise_core::geometry::{
    Aabb, Axis, ShapeOp, Vec3, VoxelShape, entity_collision_bounds, oriented::OrientedCollider,
    try_resolve_step_movement_oriented,
};

/// How far below the feet `Entity.getAvailableSpaceBelow` looks for ground.
const GROUND_RANGE: f64 = 1.0;

pub trait EntityMovementWorld {
    type Error;
    /// Return native-selected entity shapes in query order. Selection occurs
    /// once, even for zero movement, and the list is reused during stepping.
    fn entity_collisions(
        &mut self,
        entity: EntityIdentity,
        bounds: Aabb,
    ) -> Result<Vec<VoxelShape>, Self::Error>;
    /// Obtain the border, check isInsideCloseToBorder and fetch its shape only
    /// when applicable. Called before block iteration for each collision query.
    fn movement_border(
        &mut self,
        entity: EntityIdentity,
        bounds: Aabb,
    ) -> Result<Option<VoxelShape>, Self::Error>;
    /// The moving colliders meeting `bounds` that the entity collides with: boxes that are
    /// oriented in the world (a mod entity's `set_collider`), which no `VoxelShape` holds.
    /// Selected once with the entity shapes, and appended to `out`, which the caller owns and
    /// reuses so a move allocates nothing. A world with none appends none.
    fn moving_colliders(
        &mut self,
        _entity: EntityIdentity,
        _bounds: Aabb,
        _out: &mut Vec<OrientedCollider>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

thread_local! {
    /// The colliders of the move in progress, kept between moves: a move that meets some
    /// allocates nothing once the list has grown to fit.
    static COLLIDERS: std::cell::RefCell<Vec<OrientedCollider>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A move's list of colliders, borrowed from the thread's reusable one and given back empty.
struct Colliders(Vec<OrientedCollider>);

impl Colliders {
    fn take() -> Self {
        Self(COLLIDERS.with(|list| std::mem::take(&mut *list.borrow_mut())))
    }
}

impl Drop for Colliders {
    fn drop(&mut self) {
        self.0.clear();
        COLLIDERS.with(|list| *list.borrow_mut() = std::mem::take(&mut self.0));
    }
}

pub struct MovementRequest<'a> {
    pub entity: EntityIdentity,
    /// Captured context for this entity. Experimental minecart exclusions need
    /// their separate context layer; ordinary captured contexts are supported.
    pub context: CollisionContext<'a>,
    pub bounds: Aabb,
    pub movement: Vec3,
    pub max_step: f32,
    pub on_ground: bool,
    /// Per block scan, including skipped cursor cells.
    pub candidate_budget: usize,
}

pub fn resolve_world_movement<W, E>(
    world: &mut W,
    request: MovementRequest<'_>,
) -> Result<Vec3, CollisionScanError<E>>
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    let entities = world
        .entity_collisions(
            request.entity,
            entity_collision_bounds(request.movement, request.bounds, request.max_step),
        )
        .map_err(CollisionScanError::World)?;
    let mut colliders = Colliders::take();
    world
        .moving_colliders(
            request.entity,
            entity_collision_bounds(request.movement, request.bounds, request.max_step),
            &mut colliders.0,
        )
        .map_err(CollisionScanError::World)?;
    // Vanilla walks the whole box a movement expands to with no candidate
    // counter, so a movement big enough to pass the caller's budget is slow
    // there, never fatal (#6124: a legal movement_speed of 400 crossed more
    // than a million cells on a diagonal and aborted the game). The budget
    // only has to cover the cells both queries of a step can visit: the
    // sweep, and the step's own box raised by max_step.
    let sweep = request
        .bounds
        .expand_towards(request.movement)
        .expand_towards(Vec3::new(0.0, f64::from(request.max_step.max(0.0)), 0.0));
    let candidate_budget = scan_cells(sweep).map_or(request.candidate_budget, |cells| {
        cells.max(request.candidate_budget)
    });
    try_resolve_step_movement_oriented(
        request.movement,
        request.bounds,
        request.max_step,
        request.on_ground,
        &colliders.0,
        |bounds| {
            // A box's blocks are a few dozen shapes at most: one allocation,
            // not the doubling a push-by-push list makes.
            let mut shapes = Vec::with_capacity(entities.len() + 16);
            shapes.extend_from_slice(&entities);
            if let Some(border) = world
                .movement_border(request.entity, bounds)
                .map_err(CollisionScanError::World)?
            {
                shapes.push(border);
            }
            for hit in
                BlockCollisionScan::new(world, request.context, bounds, false, candidate_budget)?
            {
                shapes.push(hit?.shape);
            }
            Ok(shapes)
        },
    )
}

/// Entity-bearing CollisionGetter.noCollision, with block/context selection
/// supplied by the caller (including an always-fluid context when requested).
/// Native short-circuits blocks, then entity selection, then border geometry.
pub struct CollisionCheckRequest<'a> {
    pub entity: EntityIdentity,
    pub context: CollisionContext<'a>,
    pub bounds: Aabb,
    pub candidate_budget: usize,
}
pub fn no_world_collision<W, E>(
    world: &mut W,
    request: CollisionCheckRequest<'_>,
) -> Result<bool, CollisionScanError<E>>
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    if BlockCollisionScan::any_collision(
        world,
        request.context,
        request.bounds,
        false,
        request.candidate_budget,
    )? {
        return Ok(false);
    }
    if !world
        .entity_collisions(request.entity, request.bounds)
        .map_err(CollisionScanError::World)?
        .is_empty()
    {
        return Ok(false);
    }
    let mut colliders = Colliders::take();
    world
        .moving_colliders(request.entity, request.bounds, &mut colliders.0)
        .map_err(CollisionScanError::World)?;
    if colliders
        .0
        .iter()
        .any(|collider| collider.intersects(request.bounds))
    {
        return Ok(false);
    }
    if let Some(border) = world
        .movement_border(request.entity, request.bounds)
        .map_err(CollisionScanError::World)?
    {
        let entity = VoxelShape::from_box(request.bounds).map_err(CollisionScanError::Shape)?;
        return border
            .join_is_not_empty(&entity, ShapeOp::AND)
            .map(|overlap| !overlap)
            .map_err(CollisionScanError::Shape);
    }
    Ok(true)
}

/// `ServerPlayerGameMode.isInRangeOfGround`: nothing collides with the body and
/// `Entity.getAvailableSpaceBelow(1.0)` is less than a block, so a player who
/// keeps flying through a game mode change lands when standing on the ground.
///
/// shortcut: entity colliders stay out, as in the rest of this collision world,
/// so standing on a boat counts as air; upgrade when that must count as ground.
pub fn in_range_of_ground<W, E>(world: &mut W, bounds: Aabb) -> bool
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    // getAvailableSpaceBelow collects the colliders of the box one block under
    // the feet up to them; the body's own colliders come with that box.
    let below = bounds.move_by(Vec3::new(0.0, -GROUND_RANGE, 0.0));
    let reach = Aabb::new(below.min_position(), bounds.max_position());
    let Ok(scan) = BlockCollisionScan::new(
        world,
        // collectAllColliders without the body's own context, as
        // no_collision_empty: it changes only powder snow and a strider's
        // fluids, neither of which holds a flying player up.
        CollisionContext::empty(false),
        reach,
        false,
        1_000_000,
    ) else {
        return false;
    };
    let mut colliders = Vec::new();
    for hit in scan {
        let Ok(hit) = hit else {
            return false;
        };
        colliders.push(hit.shape);
    }
    in_range_of_ground_among(bounds, &colliders)
}

/// The collider half: `collectAllColliders(body).isEmpty()` and `Shapes.collide`
/// one block down from the body.
fn in_range_of_ground_among(bounds: Aabb, colliders: &[VoxelShape]) -> bool {
    let Ok(body) = VoxelShape::from_box(bounds) else {
        return false;
    };
    if colliders
        .iter()
        .any(|shape| shape.join_is_not_empty(&body, ShapeOp::AND).unwrap_or(true))
    {
        return false;
    }
    -VoxelShape::collide_all(Axis::Y, bounds, colliders, -GROUND_RANGE) < GROUND_RANGE
}

#[cfg(test)]
mod ground_range_tests {
    use super::*;

    /// A full block's collision shape, in world coordinates.
    fn block(x: f64, y: f64, z: f64) -> VoxelShape {
        let corner = |dx, dy, dz| Vec3::new(x + dx, y + dy, z + dz);
        VoxelShape::from_box(Aabb::new(corner(0.0, 0.0, 0.0), corner(1.0, 1.0, 1.0)))
            .expect("a unit cube")
    }

    /// A player's box, feet at `y`.
    fn body(y: f64) -> Aabb {
        Aabb::new(Vec3::new(0.0, y, 0.0), Vec3::new(0.6, y + 1.8, 0.6))
    }

    /// The floor is within the block below the feet: in range, so a flying
    /// player lands.
    #[test]
    fn a_body_on_or_near_the_floor_is_in_range_of_ground() {
        assert!(in_range_of_ground_among(body(1.0), &[block(0.0, 0.0, 0.0)]));
        // Hovering half a block up: the ground is still within its reach.
        assert!(in_range_of_ground_among(body(1.5), &[block(0.0, 0.0, 0.0)]));
    }

    /// Nothing within a block below the feet: mid-air, so the player keeps
    /// flying.
    #[test]
    fn a_body_high_in_the_air_is_not_in_range_of_ground() {
        assert!(!in_range_of_ground_among(
            body(10.0),
            &[block(0.0, 0.0, 0.0)]
        ));
        assert!(!in_range_of_ground_among(body(10.0), &[]));
    }

    /// A body inside a block is not standing on ground.
    #[test]
    fn a_body_inside_a_block_is_not_in_range_of_ground() {
        assert!(!in_range_of_ground_among(
            body(0.5),
            &[block(0.0, 0.0, 0.0)]
        ));
    }
}
