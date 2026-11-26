use crate::world::chunk_snapshot::ChunkSnapshot;
use std::fmt;
use std::sync::Arc;

/// Why reading a saved chunk or region listing via [`SavedChunkReader`] failed.
// no wire: returned only to the caller of the in-process SavedChunkReader.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SavedChunkError {
    /// The server level this reader was obtained from has closed.
    Unavailable,
    /// Out-of-bounds chunk or region coordinate (`!ChunkPos.isValid`).
    InvalidPos,
    /// Region file I/O, decompression, upgrade, or corrupt NBT decode failure.
    Io(String),
}

impl fmt::Display for SavedChunkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("saved chunk storage is unavailable"),
            Self::InvalidPos => f.write_str("chunk or region position is out of bounds"),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SavedChunkError {}

// hidden: engine-only backend trait; mods interact with `SavedChunkReader`.
#[doc(hidden)]
pub trait SavedChunkReaderBackend: Send + Sync + 'static {
    fn read(&self, pos: [i32; 2]) -> Result<Option<ChunkSnapshot>, SavedChunkError>;
    fn region_chunks(&self, rx: i32, rz: i32) -> Result<Vec<[i32; 2]>, SavedChunkError>;
}

/// Thread-safe, read-only handle to a server level's saved Anvil chunk storage.
///
/// Obtained from [`crate::LevelRef::saved_chunk_reader`]. Safe to clone and call from background
/// worker threads (`Send + Sync + 'static`). Once the level closes, every call returns
/// `Err(SavedChunkError::Unavailable)` and the handle does not retain the level's storage.
// no wire: in-process thread-safe handle to the server's saved chunk storage for worker threads.
// exhaustive: opaque handle with private backend field; constructed only by the engine.
#[derive(Clone)]
pub struct SavedChunkReader {
    backend: Arc<dyn SavedChunkReaderBackend>,
}

impl SavedChunkReader {
    // hidden: engine-only constructor; mods obtain readers from `LevelRef::saved_chunk_reader`.
    #[doc(hidden)]
    pub fn from_backend(backend: Arc<dyn SavedChunkReaderBackend>) -> Self {
        Self { backend }
    }

    /// Reads the **last saved** state of full chunk `pos` from disk (or the pending save queue)
    /// on the caller's thread into an immutable [`ChunkSnapshot`].
    ///
    /// - **Last saved state only**: does **not** reflect unsaved in-memory edits to a currently
    ///   loaded chunk. For a loaded chunk's live state, use [`crate::ChunkRef::snapshot`] instead.
    /// - Never loads the chunk into the live world, never fires chunk-load hooks, never spawns
    ///   entities, and never mutates the save directory.
    /// - Returns `Ok(None)` if the chunk has not been saved as a full chunk yet.
    /// - Returns `Err(SavedChunkError::Unavailable)` once the level closes.
    pub fn read(&self, pos: [i32; 2]) -> Result<Option<ChunkSnapshot>, SavedChunkError> {
        self.backend.read(pos)
    }

    /// Lists chunk positions present in region `(rx, rz)` by inspecting the region file's 8 KiB
    /// sector header (plus any pending unflushed writes in that region), without decompressing
    /// chunk payloads.
    ///
    /// Returns an empty `Vec` if the region file does not exist and no pending writes target it.
    pub fn region_chunks(&self, rx: i32, rz: i32) -> Result<Vec<[i32; 2]>, SavedChunkError> {
        self.backend.region_chunks(rx, rz)
    }
}

impl fmt::Debug for SavedChunkReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SavedChunkReader").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync + 'static>() {}

    #[test]
    fn saved_chunk_reader_is_send_sync() {
        assert_send_sync::<SavedChunkReader>();
        assert_send_sync::<SavedChunkError>();
    }
}
