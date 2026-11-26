//! The `client_chunk_unload` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Chunk `chunk` (`[x, z]`, in chunks) of `dimension` is leaving this
    /// client's level (NeoForge's `ChunkEvent.Unload` on the client): once
    /// per column whose data is freed, with its blocks still there, just
    /// before they are freed. Told on every route that frees a column: the
    /// server's forget packet, a newer column taking the slot of one out of
    /// view, the view radius shrinking, and the client level going (a
    /// dimension change, a respawn into another level, leaving the server).
    /// The view centre moving away tells nothing: `ClientChunkCache` keeps
    /// those columns in their slots, out of view, and shows one again
    /// untold when it is back in view, with no `client_chunk_load`. That is
    /// wider than NeoForge, which tells only the forget packet, so a
    /// listener never keeps state for a column that is gone. Only on a
    /// client of a server, a LAN guest included; an integrated world's own
    /// chunks have the server's `chunk_unload`. Client-side.
    #[notes = r##"## Client chunk unload

`client_chunk_unload(dimension, chunk)` runs on a client of a server once
for each column whose data leaves the client level, while its blocks and block
entities are still readable, just before they are freed. Every route that frees
a column tells it: the server's `ForgetLevelChunk` packet, a newer column
taking the slot of one that is out of view, the view radius shrinking, and the
client level going (a dimension change, a respawn into another level, leaving
the server). The view centre moving away tells nothing: as in
`ClientChunkCache`, the column stays in its slot out of view, and it is shown
again untold when it is back in view, with no `client_chunk_load`. A column
that is already gone is not told again. NeoForge tells only the forget packet
(`ClientChunkCache.drop`); Sunrise tells every route that frees a column, so a
listener never keeps state for a column the client no longer holds. `dimension`
is the level the column belonged to: on a dimension change, the old one."##]
    #[hook(
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "listen",
        neoforge = "ChunkEvent.Unload",
        cancellable = false
    )]
    fn client_chunk_unload(dimension: &str, chunk: [i32; 2]);
}
