//! Typed refusals of state-bearing writes through a level handle.
use crate::{BlockStateError, LevelReadError, StackWriteError};

/// Why a level operation cannot reach its level.
/// `Ok(false)` instead means the operation did not apply, for example at an
/// unloaded or out-of-height position or an unchanged block.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum LevelWriteError {
    /// The level is gone or its handle belongs to an earlier lifetime.
    Stale,
    /// The operation is unavailable on this side, phase or current level.
    Unavailable,
    /// The supplied block state does not exist in the owning registry.
    InvalidState(BlockStateError),
    /// The supplied detached item value cannot be converted for placement.
    InvalidStack(StackWriteError),
    /// This placement completes a wither pattern and cannot run inside a Host transaction.
    InTransaction,
}
impl From<LevelReadError> for LevelWriteError {
    fn from(error: LevelReadError) -> Self {
        match error {
            LevelReadError::Stale => Self::Stale,
            LevelReadError::Unavailable => Self::Unavailable,
        }
    }
}
impl From<BlockStateError> for LevelWriteError {
    fn from(error: BlockStateError) -> Self {
        match error {
            BlockStateError::Unavailable => Self::Unavailable,
            error => Self::InvalidState(error),
        }
    }
}
impl From<StackWriteError> for LevelWriteError {
    fn from(error: StackWriteError) -> Self {
        match error {
            StackWriteError::Unavailable => Self::Unavailable,
            error => Self::InvalidStack(error),
        }
    }
}
impl crate::NoBackend for LevelWriteError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
