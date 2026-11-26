//! The `client_break_progress` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The client level of `dimension` was told that `breaker` (an entity id) cracked the block at
    /// `pos` to `progress` (vanilla's `ClientLevel.destroyBlockProgress`): 0 to 9 shows that crack
    /// stage, any other value takes the breaker's crack away. Told on both routes the game tells
    /// the client level by: the server's block destruction packet, and the local player's own dig
    /// (its stage when it starts and on every tick after, -1 when it stops or the block breaks).
    /// A repeated stage is told again, as vanilla tells its level every tick of a dig. A mod's own
    /// `set_client_break_progress` is not told, so a listener may call it. Only on a client of a
    /// server, single player's window included. Client-side; with no listener a crack costs one
    /// flag check. Beyond NeoForge: there is no counterpart. Diverges from upstream: new hook.
    #[notes = r##"## Client break progress

`client_break_progress(dimension, breaker, pos, progress)` runs on a client of a server each time
the game tells its client level of a crack (`ClientLevel.destroyBlockProgress`). `breaker` is the
entity id of whoever cracks the block; `progress` 0 to 9 is the crack stage, and any other value
takes that breaker's crack away. Two routes tell it, as in vanilla: the server's
`BlockDestruction` packet (`ClientPacketListener.handleBlockDestruction`), and the local player's
own dig, with `breaker` the player's own entity id (`MultiPlayerGameMode.startDestroyBlock` and
`continueDestroyBlock` with the dig's stage, -1 before it has progress; `stopDestroyBlock` and a
finished break with -1). A server that runs mining itself shows the player's own crack only
through the packet, so this is how a client mod hears how far that block has cracked. A mod's own
`set_client_break_progress` is not told back to it. The hook only reports: the crack is shown as
vanilla shows it whatever the listener does."##]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla ClientLevel.destroyBlockProgress; NeoForge adds no client event for it)"
    )]
    fn client_break_progress(dimension: &str, breaker: i32, pos: BlockPos, progress: i32);
}
