//! Chunk attachment decoding boundary.

pub trait Hooks {
    /// The disk record's attachments have decoded, before the chunk is published.
    /// Runs on a chunk worker with a restricted Host. Read and edit `data` through
    /// its typed methods; edits become the loaded store only if the entire job
    /// succeeds. Missing records and proto-to-full promotion do not fire this hook.
    /// Borrow the captured metadata from `host.job()`. A codec error, panic,
    /// or invalid or over-budget disk NBT refuses publication of the whole store.
    /// Later listeners still run on the last accepted envelopes. A phase refusal
    /// discards only that callback's proposal; only a panic disables a callback
    /// for this hook for the session.
    #[hook(
        side = "server",
        contract = "listen",
        context = "chunk_disk",
        target = "mod_api::chunk_disk::native_chunk_data_load",
        neoforge = "ChunkDataEvent.Load",
        version = 3
    )]
    fn chunk_data_load(data: &mut ChunkDiskData);
}
