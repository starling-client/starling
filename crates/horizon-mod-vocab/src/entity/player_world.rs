//! Refusals of a block write a mod makes for a player (`PlayerRef::place_block`).
use crate::BlockStateError;

/// Why `player.place_block` wrote nothing. Nothing was written or sent when one comes back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PlayerWorldRefusal {
    /// The handle no longer names an online player here: it is stale, or the player left.
    Stale,
    /// The call was made on a client; the write is the server's.
    Client,
    /// The player is dead.
    Dead,
    /// The state names no block state of this world.
    UnknownState,
    /// The level handle is stale, or its level is closed.
    StaleLevel,
    /// The level is not the one the player is in.
    OtherLevel,
    /// `pos` is above or below the level's height.
    OutOfWorld {
        /// The position asked for.
        pos: [i32; 3],
    },
    /// The chunk holding `pos` is not loaded.
    NotLoaded {
        /// The position asked for.
        pos: [i32; 3],
    },
    /// That state already stands there, or the level refused the write.
    Unchanged,
}

impl From<BlockStateError> for PlayerWorldRefusal {
    fn from(error: BlockStateError) -> Self {
        match error {
            BlockStateError::Unavailable => Self::Client,
            _ => Self::UnknownState,
        }
    }
}

impl crate::NoBackend for PlayerWorldRefusal {
    /// No game behind the host: no player is online there.
    fn no_backend() -> Self {
        Self::Stale
    }
}
