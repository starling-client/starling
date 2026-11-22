//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// How many `Droppable` sends the engine dropped, readable on either side through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait PayloadDropsHost {
    /// Side: both.
    /// The drops of `Droppable` channel `id` so far, by cause, over every connection of this
    /// game (a server's clientbound drops, a client's serverbound ones, and the server's
    /// refusals of a client over its byte budget). `None` for a channel that is not `Droppable`
    /// or not declared by an enabled mod. `Err(Unavailable)` when the host has no backend.
    fn payload_drop_counts(
        &self,
        id: &PayloadId,
    ) -> Result<Option<PayloadDropCounts>, LevelReadError>;
}
