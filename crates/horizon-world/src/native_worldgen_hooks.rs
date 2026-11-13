//! Callbacks into the executable for hooks fired while a feature generates: on
//! the worldgen workers, and on the server thread when a sapling grows. No game
//! is at hand there, so a hook gets the server's [`WorldgenContext`] instead.
use crate::{native_load_context::WorldgenContext, registry::BlockStateId};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

/// What a mod's ground provider reads while the tree decorator asks it for
/// one ground position: the region being decorated, the decorator's own random
/// (vanilla's `TreeDecorator.Context.random()`, so draws keep vanilla's order)
/// and the decorator's own provider.
pub trait GroundAccess {
    /// The block at `pos` in the region being decorated.
    fn block_state(&mut self, pos: [i32; 3]) -> BlockStateId;
    /// The decorator's own provider at `pos` (`BlockStateProvider.getOptionalState`),
    /// drawing from the same random: None where it places nothing.
    fn decorator_state(&mut self, pos: [i32; 3]) -> Option<BlockStateId>;
    /// `nextInt(bound)`; 0 for a bound below 1.
    fn next_int_bound(&mut self, bound: i32) -> i32;
    /// `nextLong`.
    fn next_long(&mut self) -> i64;
    /// `nextFloat`.
    fn next_float(&mut self) -> f32;
    /// `nextDouble`.
    fn next_double(&mut self) -> f64;
    /// `nextBoolean`.
    fn next_boolean(&mut self) -> bool;
}

/// A ground provider a mod put in place of the tree decorator's own, as the
/// executable's adapter hands it over. `Send + Sync` because neighbouring
/// chunks decorate at once on worker threads.
pub trait ModGroundProvider: Send + Sync {
    /// The state to place at `pos`, or None to try the next position down the
    /// column (`AlterGroundEvent.StateProvider.getState`).
    fn state_at(&self, access: &mut dyn GroundAccess, pos: [i32; 3]) -> Option<BlockStateId>;
}

/// The tree whose ground `AlterGroundDecorator.place` alters: the decorator
/// context's lists (`TreeDecorator.Context`), each sorted by Y, and the
/// positions it places around (`TreeFeature.getLowestTrunkOrRootOfTree`).
pub struct AlterGroundTree<'a> {
    pub logs: &'a [[i32; 3]],
    pub leaves: &'a [[i32; 3]],
    pub roots: &'a [[i32; 3]],
    /// Never empty: the decorator fires nothing for a tree without them.
    pub positions: &'a [[i32; 3]],
}

/// Feature-generation hooks, installed before any world opens.
pub struct NativeWorldgenHooks {
    /// `alter_ground`: leaves `provider` None to keep the decorator's own.
    pub alter_ground: fn(&AlterGroundTree<'_>, &mut Option<Arc<dyn ModGroundProvider>>),
}

static HOOKS: OnceLock<NativeWorldgenHooks> = OnceLock::new();
static CONTEXT: RwLock<Option<Arc<WorldgenContext>>> = RwLock::new(None);

/// Install the process-wide callbacks once; a second installation keeps the first.
pub fn install(hooks: NativeWorldgenHooks) -> Result<(), NativeWorldgenHooks> {
    HOOKS.set(hooks)
}

/// Publish the server's context for the features it generates from now on,
/// replacing the one before. Called where the server starts or reloads its
/// resources.
pub fn publish_context(context: Arc<WorldgenContext>) {
    *CONTEXT.write().unwrap_or_else(PoisonError::into_inner) = Some(context);
}

/// The context the server published last; None before a server published one.
pub fn context() -> Option<Arc<WorldgenContext>> {
    CONTEXT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// The provider mods put in place of the decorator's own for `tree`, or None
/// to keep the decorator's own.
pub(crate) fn alter_ground(tree: &AlterGroundTree<'_>) -> Option<Arc<dyn ModGroundProvider>> {
    let hooks = HOOKS.get()?;
    let mut provider = None;
    (hooks.alter_ground)(tree, &mut provider);
    provider
}
