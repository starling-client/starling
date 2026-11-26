//! The effect sink lent to a block entity's pre-removal callback.

use crate::{ItemStack, StackWriteError};

/// Why one removal effect was refused. Other contributions are unchanged.
// no wire: a local refusal from the removal callback's sink.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockRemovalEffectError {
    /// The stack failed the level's normal item write checks.
    Stack(StackWriteError),
    /// At least one position coordinate was not finite.
    NonFinitePosition,
}

/// Append-only access to the retiring block entity's side effects.
///
/// The engine applies these effects before finishing removal: base contents first (unless
/// suppressed), then contributed scatter slots, positioned items, and experience. Within
/// each group, calls keep their order. A refusal affects only that call; a panicking Part
/// contributes nothing. This is not a host for arbitrary writes to the level.
pub trait BlockRemovalEffects {
    /// Whether the base container's contents are currently excluded from scattering.
    fn keep_contents(&self) -> bool;

    /// Set whether to skip scattering the base contents. Initially `false`.
    /// The last successful Part to set this decides.
    fn set_keep_contents(&mut self, keep: bool);

    /// Scatter one slot at the outgoing block via `Containers.dropItemStack`.
    /// `None` is an empty slot: it consumes the three position draws without spawning.
    /// `Some` must contain a non-air stack. A refused stack consumes no draws.
    fn drop_slot(&mut self, stack: Option<ItemStack>) -> Result<(), BlockRemovalEffectError>;

    /// Spawn a non-air stack at the given finite position with the default pickup delay.
    fn spawn_item(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
    ) -> Result<(), BlockRemovalEffectError>;

    /// Award experience at the given finite position through `ExperienceOrb.award`.
    /// An amount of zero produces no orbs or random draws.
    fn award_experience(
        &mut self,
        amount: u32,
        position: [f64; 3],
    ) -> Result<(), BlockRemovalEffectError>;
}
