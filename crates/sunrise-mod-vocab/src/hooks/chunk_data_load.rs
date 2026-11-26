//! Chunk attachment decoding boundary.

pub trait Hooks {
    /// The disk record's attachments have decoded, before the chunk is published.
    /// Runs on the owning server thread with a restricted Host. Read and edit `data` through
    /// its typed methods and borrow captured metadata from `host.job()`. Missing records
    /// and proto-to-full promotion do not fire this hook. Codec and key errors refuse
    /// only that operation. A listener that panics loses only its edits and is disabled
    /// for this hook for the session. Other listeners' edits still publish.
    #[hook(
        side = "server",
        contract = "listen",
        context = "chunk_disk",
        target = "mod_api::chunk_disk::native_chunk_data_load",
        neoforge = "ChunkDataEvent.Load",
        version = 4
    )]
    fn chunk_data_load(data: &mut ChunkDiskData);
}
