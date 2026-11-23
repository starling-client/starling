//! Transport-owned credit for ordered remote backlog slots.
use super::remote_chunks::ChunkFormat;
use sunrise_client::net::Event;
use std::sync::Arc;

/// What the network thread delivered, in order: an event, or a chunk packet
/// handed to the decoder (with the format it decodes with), whose column
/// comes back from `ChunkDecoder::next`.
pub(super) enum Received {
    Event(Event, sunrise_client::net::ingress::ReceiveLease<Event>),
    Chunk(
        Arc<ChunkFormat>,
        sunrise_client::net::ingress::ReceiveLease<Event>,
    ),
}

// Keep net::Event's transport-owned staging slot bound honest as the actor evolves.
const _: () = assert!(
    std::mem::size_of::<Received>()
        <= std::mem::size_of::<(
            Event,
            sunrise_client::net::ingress::ReceiveLease<Event>,
            usize,
            usize,
        )>()
);

impl Received {
    pub(super) fn into_event_credit(self) -> sunrise_client::net::ingress::ReceiveLease<Event> {
        let Self::Event(_, credit) = self else {
            unreachable!()
        };
        credit
    }
}
