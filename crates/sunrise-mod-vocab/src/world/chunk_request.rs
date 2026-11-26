//! Non-blocking server chunk load/generation requests (`LevelRef::request_chunk`, #5234).
use crate::{ChunkLoadError, ChunkSnapshot, LevelRef};

/// Handle to a non-blocking chunk load or generation request queued on a server level
/// (`LevelRef::request_chunk`).
///
/// The handle identifies one request made by the calling mod; pass it to
/// [`ChunkRequestRef::cancel`] to cancel the request and release its level-33
/// (`ChunkLevel.FULL_CHUNK_LEVEL`) ticket before completion, or match it against
/// [`CompletedChunkRequest::request`] returned by `LevelRef::drain_chunk_requests`.
// exhaustive: opaque handle with crate-private fields; constructed only by the engine.
// no wire: process-local request handle bound to a running server level, not a network payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkRequestRef {
    pub(crate) level: LevelRef,
    pub(crate) id: u64,
}

impl ChunkRequestRef {
    /// The server level this request targets.
    pub fn level(self) -> LevelRef {
        self.level
    }

    /// Side: server.
    /// Cancels a pending (or undrained) request owned by the calling mod and releases its
    /// level-33 (`ChunkLevel.FULL_CHUNK_LEVEL`) ticket immediately. Returns `true` if the
    /// request was still active.
    pub fn cancel(self, host: &mut dyn crate::ServerHost) -> bool {
        crate::backend_mut_of(host).is_some_and(|backend| backend.chunk_request_cancel(self))
    }
}

/// One finished or failed non-blocking chunk request returned by `LevelRef::drain_chunk_requests`.
// no wire: holds an engine-backed `ChunkSnapshot` and process-local request handle.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct CompletedChunkRequest {
    /// Handle returned when the request was queued by `LevelRef::request_chunk`.
    pub request: ChunkRequestRef,
    /// Chunk coordinates `[x, z]` that were requested.
    pub pos: [i32; 2],
    /// Immutable snapshot of the `FULL` (lit) chunk, or the load/generation failure.
    pub result: Result<ChunkSnapshot, ChunkLoadError>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DimensionKey;

    #[test]
    fn chunk_request_ref_equality_and_level_accessor() {
        let level = LevelRef::new(crate::world::types::WorldEpoch(3), DimensionKey::OVERWORLD);
        let a = crate::__private::game::chunk_request_ref(level, 1);
        let b = crate::__private::game::chunk_request_ref(level, 1);
        let c = crate::__private::game::chunk_request_ref(level, 2);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.level(), level);

        let completed = crate::__private::game::completed_chunk_request(
            a,
            [4, -2],
            Err(ChunkLoadError::Failed),
        );
        assert_eq!(completed.request, a);
        assert_eq!(completed.pos, [4, -2]);
        assert!(matches!(completed.result, Err(ChunkLoadError::Failed)));
    }
}
