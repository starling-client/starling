//! Chunk attachment snapshot encoding boundary.

pub trait Hooks {
    /// A chunk snapshot has been captured on the owning server thread, before
    /// background encoding. The callback runs on that server thread.
    /// `data` belongs only to that snapshot: typed edits change the disk record,
    /// never the live chunk. Exactly once per captured record, for full and proto
    /// chunks alike. The restricted Host cannot resolve live chunks or mutate the
    /// world; `host.job()` borrows the owning level's identity and this record's
    /// metadata. A failed job, including invalid or over-budget disk NBT, retains
    /// its snapshot and the prior disk record.
    /// After a callback panics or hits a codec error, later listeners still run on
    /// the last accepted envelopes, but the job cannot publish. A phase refusal
    /// discards only that callback's proposal; only a panic disables a callback
    /// for this hook for the session.
    #[hook(
        side = "server",
        contract = "listen",
        context = "chunk_disk",
        target = "mod_api::chunk_disk::native_chunk_data_save",
        neoforge = "ChunkDataEvent.Save",
        version = 4
    )]
    fn chunk_data_save(data: &mut ChunkDiskData);
}
