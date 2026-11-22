//! The seam where a block's collision shape may depend on the querying entity: a part the game
//! installed on this thread answers for the states that opt in, in place of the shape the
//! block would otherwise have. No part is installed on a worker, so a query there reads the
//! normal shape; and with none installed anywhere, a query pays one relaxed load.
use crate::{collision_context::CollisionContext, registry::BlockStateId};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};
use sunrise_core::{geometry::VoxelShape, position::BlockPos};

/// What answers a collision query for the states whose parts ask for it.
pub trait PartShapes {
    /// The shape the parts of `state` give it at `position` for `context`, or `None` to keep the
    /// shape the block would otherwise have. `standable` names the fluids the context's living
    /// entity can stand on, and is called only when a part is actually asked.
    fn shape(
        &self,
        state: BlockStateId,
        position: BlockPos,
        context: &CollisionContext<'_>,
        standable: &mut dyn FnMut() -> &'static [&'static str],
    ) -> Option<VoxelShape>;
}

/// How many [`in_parts`] scopes are open on any thread.
static OPEN: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static PARTS: RefCell<Option<Rc<dyn PartShapes>>> = const { RefCell::new(None) };
}

/// Parts answering this thread's collision queries until it drops, for the states that opt in;
/// the previous parts come back with it, even by a panic.
pub struct InParts(Option<Rc<dyn PartShapes>>);

impl InParts {
    pub fn enter(parts: Rc<dyn PartShapes>) -> Self {
        OPEN.fetch_add(1, Ordering::Relaxed);
        Self(PARTS.with_borrow_mut(|slot| slot.replace(parts)))
    }
}

impl Drop for InParts {
    fn drop(&mut self) {
        PARTS.with_borrow_mut(|parts| *parts = self.0.take());
        OPEN.fetch_sub(1, Ordering::Relaxed);
    }
}

/// The installed parts' answer for `state`, which the caller has checked opts in.
pub(crate) fn ask(
    state: BlockStateId,
    position: BlockPos,
    context: &CollisionContext<'_>,
    standable: &mut dyn FnMut() -> &'static [&'static str],
) -> Option<VoxelShape> {
    if OPEN.load(Ordering::Relaxed) == 0 {
        return None;
    }
    let parts = PARTS.with_borrow(Clone::clone)?;
    parts.shape(state, position, context, standable)
}
