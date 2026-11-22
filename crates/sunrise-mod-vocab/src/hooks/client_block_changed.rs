//! The `client_block_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The block at `pos` in `dimension` went from `old` to `new` in this client's level
    /// (vanilla's `Level.setBlock` on a `ClientLevel`), after the write: the level already reads
    /// `new`. Told once per block whose state changed, on every route a client writes a block by:
    /// the server's block update, each block of its batched section update, the client's own
    /// predictions (a break, a placement, a use; `predicted` is true) and their reconciliation
    /// when the server acknowledges them (a refused prediction comes back, `predicted` false).
    /// A write that keeps the state is not told, and neither is a server update held under a
    /// pending prediction until the acknowledgement applies it. Whole columns arriving and leaving
    /// are `client_chunk_load` and `client_chunk_unload`. Only on a client of a server, a LAN
    /// guest included, and without a server-side mod; an integrated world's own blocks are the
    /// server's `block_changed`. A level a server opens for viewing (`sunrise:levels`) is not
    /// told. Client-side; with no listener a write costs one flag check. Beyond NeoForge: there is
    /// no counterpart. Diverges from upstream: new hook.
    #[notes = r##"## Client block changes

`client_block_changed(dimension, pos, old, new, predicted)` runs on a client of a server
after each block of the client level changes state, with the level already reading `new`. It
is the per-block counterpart of `client_chunk_load` and `client_chunk_unload`, so a client map
can invalidate just the chunk that holds `pos` instead of rescanning the loaded terrain. Every
route a client writes a block by tells it: the server's `BlockUpdate`, each block of a
`SectionBlocksUpdate`, the client's own predictions (`predicted` true) and, when the server
acknowledges them (`BlockChangedAck`), each block that takes the server's state back
(`predicted` false). A server update for a block still under a pending prediction is held, as in
vanilla, and told when the acknowledgement applies it. A write that keeps the state is not told.
An integrated world's own blocks are the server's `block_changed`; a level the server opens for
viewing is not told."##]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla ClientLevel.setBlock; NeoForge adds no client event for it)"
    )]
    fn client_block_changed(
        dimension: &str,
        pos: BlockPos,
        old: BlockState,
        new: BlockState,
        predicted: bool,
    );
}
