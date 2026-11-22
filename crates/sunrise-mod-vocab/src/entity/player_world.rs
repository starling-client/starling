<<<<<<< sync-sunrise-1
//! Refusals of a block write a mod makes for a player (`PlayerRef::place_block`).
use crate::BlockStateError;

/// Why `player.place_block` wrote nothing. Nothing was written or sent when one comes back.
=======
//! Refusals when a mod shows, places or acknowledges blocks as a player, or asks what a player sees.

/// Why `player.sees`, `player.ack_block_changes`, `player.show_block` or `player.place_block`
/// answered with nothing done: nothing was sent, written or queued.
>>>>>>> sunrise/main
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PlayerWorldRefusal {
<<<<<<< sync-sunrise-1
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
=======
    /// A player handle (the viewer, the target or the player acted as) no longer identifies a
    /// player in the server's list: it is stale, or the player has left.
    Stale,
    /// The call was made on a client; these are the server's.
    Client,
    /// The player has no connection to send to (a fake player).
    NoConnection,
    /// The player is dead; the new player a respawn makes is the one to act as.
    Dead,
    /// `ack_block_changes` with a sequence below 0.
    NegativeSequence {
        /// The sequence given.
        sequence: i32,
    },
    /// `show_block` at a position the block update packet cannot carry: y outside
    /// -2048..=2047, x or z outside -33554432..33554432.
    PositionNotSendable {
        /// The position given.
        pos: [i32; 3],
    },
    /// The block state is not in the world's registry.
    UnknownState,
    /// `place_block`'s level handle is of an earlier open of the world.
    StaleLevel,
    /// `place_block` in a level that is not the one in place, where the player is.
    OtherLevel,
    /// `place_block` above or below the world or outside the 30,000,000 block limit.
    OutOfWorld {
        /// The position given.
        pos: [i32; 3],
    },
    /// `place_block` where the chunk is not loaded.
    NotLoaded {
        /// The position given.
        pos: [i32; 3],
    },
    /// `place_block` of the state already standing there: `setBlock` writes nothing.
    Unchanged,
    /// The origin or a cell the placement would write directly is held by a carrier transfer.
    Fenced,
    /// This placement completes a wither pattern and cannot run inside a Host transaction.
    InTransaction,
}

impl crate::NoBackend for PlayerWorldRefusal {
>>>>>>> sunrise/main
    fn no_backend() -> Self {
        Self::Stale
    }
}
<<<<<<< sync-sunrise-1
=======

impl From<crate::BlockStateError> for PlayerWorldRefusal {
    fn from(error: crate::BlockStateError) -> Self {
        match error {
            // The block state tables are not installed: no world is open.
            crate::BlockStateError::Unavailable => Self::Stale,
            _ => Self::UnknownState,
        }
    }
}
>>>>>>> sunrise/main
