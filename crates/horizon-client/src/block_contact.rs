//! Synchronous mod-block contacts from the server's simulation code.
use crate::health::DamageLevel;
use horizon_core::position::BlockPos;
use horizon_world::{level::LandingWrite, registry::BlockStateId};
use std::{
    cell::{Cell, RefCell},
    sync::OnceLock,
};

#[derive(Clone, Copy, Debug)]
pub struct Contact<'a> {
    pub entity: i32,
    pub kind: &'a str,
    pub state: BlockStateId,
    pub position: BlockPos,
}

/// Where the contacts are answered: the game binds these to the hooks of the block
/// parts, and says which block states have a part for each.
pub struct Hooks {
    /// `Block.stepOn` for a mod block, in the dimension.
    pub step_on: fn(&str, Contact<'_>),
    pub any_inside: fn() -> bool,
    pub inside_listens: fn(BlockStateId) -> bool,
    pub pass_listens: fn(BlockStateId) -> bool,
    pub fall_listens: fn(BlockStateId) -> bool,
    pub bounce_listens: fn(BlockStateId) -> bool,
    /// `Block.fallOn`: change `damage`, the fall damage `causeFallDamage` calculated, and
    /// push the writes the part asks for.
    pub fall_on: fn(&str, &mut DamageLevel<'_>, Contact<'_>, f64, &mut f32, &mut Vec<LandingWrite>),
    /// `Block.bounceOn`: change `movement`, the movement the bounce leaves the entity.
    pub bounce_on:
        fn(&str, &mut DamageLevel<'_>, Contact<'_>, f64, &mut horizon_core::geometry::Vec3),
}

pub static HOOKS: OnceLock<Hooks> = OnceLock::new();

/// A contact kept until the entity's batch ends, delivered in the order it happened.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    Inside(Inside),
    Landed(Landed),
    MinecartPass(MinecartPass),
}

/// An entity that landed on a block whose Part asked for writes (`Block.fallOn`).
#[derive(Clone, Debug, PartialEq)]
pub struct Landed {
    pub state: BlockStateId,
    pub position: BlockPos,
    pub writes: Vec<LandingWrite>,
}

/// An entity inside a block whose Part listens to `entity_inside`
/// (`Block.entityInside`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inside {
    pub entity: i32,
    pub state: BlockStateId,
    pub position: BlockPos,
    /// `Entity.checkInsideBlocks`' `isPrecise`: the entity moved far, or its box
    /// at the target intersects the block.
    pub precise: bool,
}

/// A minecart on a rail whose Part listens to `minecart_pass`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinecartPass {
    pub entity: i32,
    pub state: BlockStateId,
    pub position: BlockPos,
}

thread_local! {
    static LEVEL: Cell<Option<&'static str>> = const { Cell::new(None) };
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
}

/// Scope authority to this thread and restore it even on an error or panic.
/// Nested guest actions may temporarily enter a different dimension.
pub fn in_level<R>(dimension: Option<&'static str>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<&'static str>);
    impl Drop for Restore {
        fn drop(&mut self) {
            LEVEL.set(self.0);
        }
    }
    let _restore = Restore(LEVEL.replace(dimension));
    f()
}

/// `Block.stepOn` for a mod block: the entity stands on `contact`'s block.
pub fn step_on(block: &str, contact: Contact<'_>) {
    if block.starts_with("minecraft:") {
        return;
    }
    if let Some(dimension) = LEVEL.get()
        && let Some(hooks) = HOOKS.get()
    {
        (hooks.step_on)(dimension, contact);
    }
}

/// Whether a part of the block `state` answers `fall_on`.
pub fn fall_listens(state: BlockStateId) -> bool {
    HOOKS.get().is_some_and(|hooks| (hooks.fall_listens)(state))
}

/// `Block.fallOn`: the fall damage `damage` calculated for the entity that landed on
/// `contact`'s block, after any part of that block changed it. A non-finite answer is
/// ignored and a negative one is zero. The writes the part asked for are kept until the
/// entity's batch ends. Nothing runs off the server or without a part.
pub fn fall(level: &mut DamageLevel<'_>, contact: Contact<'_>, distance: f64, damage: f32) -> f32 {
    let (Some(dimension), Some(hooks)) = (LEVEL.get(), HOOKS.get()) else {
        return damage;
    };
    if !(hooks.fall_listens)(contact.state) {
        return damage;
    }
    let mut answer = damage;
    let mut writes = Vec::new();
    (hooks.fall_on)(
        dimension,
        level,
        contact,
        distance,
        &mut answer,
        &mut writes,
    );
    landed(contact.state, contact.position, writes);
    accepted_damage(answer, damage)
}

/// A part's fall damage answer: negative is zero, and a non-finite one is ignored.
fn accepted_damage(answer: f32, kept: f32) -> f32 {
    if answer.is_finite() {
        answer.max(0.0)
    } else {
        kept
    }
}

/// A part's bounce answer, ignored unless every component is finite.
fn accepted_movement(
    answer: horizon_core::geometry::Vec3,
    kept: horizon_core::geometry::Vec3,
) -> horizon_core::geometry::Vec3 {
    if answer.x.is_finite() && answer.y.is_finite() && answer.z.is_finite() {
        answer
    } else {
        kept
    }
}

/// `Block.bounceOn`: the movement a bounce off `contact`'s block leaves, after any part of
/// that block changed it. A non-finite answer is ignored.
pub fn bounce(
    level: &mut DamageLevel<'_>,
    contact: Contact<'_>,
    fall_distance: f64,
    movement: horizon_core::geometry::Vec3,
) -> horizon_core::geometry::Vec3 {
    let (Some(dimension), Some(hooks)) = (LEVEL.get(), HOOKS.get()) else {
        return movement;
    };
    if !(hooks.bounce_listens)(contact.state) {
        return movement;
    }
    let mut answer = movement;
    (hooks.bounce_on)(dimension, level, contact, fall_distance, &mut answer);
    accepted_movement(answer, movement)
}

/// Keep the writes a landing on the block `state` at `position` asked for until the
/// entity's batch ends; nothing is kept when none were asked.
pub fn landed(state: BlockStateId, position: BlockPos, writes: Vec<LandingWrite>) {
    if !writes.is_empty() {
        PENDING.with_borrow_mut(|pending| {
            pending.push(Pending::Landed(Landed {
                state,
                position,
                writes,
            }));
        });
    }
}

/// Note an entity inside a block, if a part there listens; nothing is kept
/// otherwise, so a world without such a part pays one test per block.
pub fn inside(entity: i32, state: BlockStateId, position: BlockPos, precise: bool) {
    if LEVEL.get().is_some()
        && HOOKS
            .get()
            .is_some_and(|hooks| (hooks.inside_listens)(state))
    {
        PENDING.with_borrow_mut(|pending| {
            pending.push(Pending::Inside(Inside {
                entity,
                state,
                position,
                precise,
            }));
        });
    }
}

/// Note a minecart on the rail `state` at `position`, if a part there listens; a world
/// without such a part pays one test per rail step.
pub fn minecart_pass(entity: i32, state: BlockStateId, position: BlockPos) {
    if LEVEL.get().is_some() && HOOKS.get().is_some_and(|hooks| (hooks.pass_listens)(state)) {
        PENDING.with_borrow_mut(|pending| {
            pending.push(Pending::MinecartPass(MinecartPass {
                entity,
                state,
                position,
            }));
        });
    }
}

/// The contacts noted since the last call, in the order they happened.
pub fn take_pending() -> Vec<Pending> {
    PENDING.with_borrow_mut(std::mem::take)
}

/// `Entity.checkInsideBlocks`' `isPrecise` for the block at `position`: the
/// move from `from` to `to` was long, or the box at `to` touches the block.
pub fn is_precise(
    from: horizon_core::geometry::Vec3,
    to: horizon_core::geometry::Vec3,
    at_target: horizon_core::geometry::Aabb,
    position: BlockPos,
) -> bool {
    let far = 0.999_990_000_000_252_6_f64;
    from.distance_squared(to) > far * far
        || at_target.intersects(horizon_core::geometry::Aabb::block(position))
}

/// Reuse the entity block sweep for item and projectile ticks without an effect collector.
pub fn swept_inside(
    entity: i32,
    from: horizon_core::geometry::Vec3,
    to: horizon_core::geometry::Vec3,
    bounds: horizon_core::geometry::Aabb,
    mut block: impl FnMut(BlockPos) -> BlockStateId,
) {
    if LEVEL.get().is_none() || !HOOKS.get().is_some_and(|hooks| (hooks.any_inside)()) {
        return;
    }
    let bounds = bounds.deflate(horizon_core::geometry::Vec3::new(1e-5, 1e-5, 1e-5));
    let mut visited = std::collections::HashSet::new();
    let _ =
        horizon_core::geometry::sweep_intersected_blocks(from, to, bounds, 1_000_000, |p, step| {
            if step >= 16 {
                return false;
            }
            if visited.insert(p) {
                inside(entity, block(p), p, is_precise(from, to, bounds, p));
            }
            true
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    /// `checkInsideBlocks`' `isPrecise`: a long move, or the box at the target
    /// touching the block.
    #[test]
    fn precision_follows_the_move_length_and_the_box() {
        use horizon_core::geometry::{Aabb, Vec3};
        let at = Aabb::new(Vec3::new(2.25, 2.0, 3.25), Vec3::new(2.75, 2.5, 3.75));
        let block = BlockPos::new(2, 2, 3);
        let from = Vec3::new(2.5, 2.0, 3.5);
        assert!(is_precise(from, from, at, block));
        assert!(!is_precise(from, from, at, BlockPos::new(1, 2, 3)));
        assert!(is_precise(
            Vec3::new(0.5, 2.0, 3.5),
            from,
            at,
            BlockPos::new(1, 2, 3)
        ));
    }

    /// A fall answer is clamped at zero and a non-finite one ignored; a bounce answer is
    /// used unless one component is non-finite.
    #[test]
    fn landing_answers_are_checked() {
        use horizon_core::geometry::Vec3;
        assert_eq!(accepted_damage(-2.0, 3.0), 0.0);
        assert_eq!(accepted_damage(5.0, 3.0), 5.0);
        assert_eq!(accepted_damage(f32::NAN, 3.0), 3.0);
        let kept = Vec3::new(0.0, 0.8, 0.0);
        assert_eq!(
            accepted_movement(Vec3::new(0.0, 0.4, 0.0), kept),
            Vec3::new(0.0, 0.4, 0.0)
        );
        assert_eq!(
            accepted_movement(Vec3::new(0.0, f64::INFINITY, 0.0), kept),
            kept
        );
    }
}
