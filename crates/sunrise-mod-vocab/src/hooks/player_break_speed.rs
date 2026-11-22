//! The `player_break_speed` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player's destroy speed for the block at `pos` may be changed or canceled.
    /// A client without this mod predicts vanilla destroy speed; the server remains authoritative
    /// and corrects the resulting block-breaking progress.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerEvent.BreakSpeed",
        cancellable = true
    )]
    fn player_break_speed(
        player: u32,
        pos: [i32; 3],
        state: u32,
        original_speed: f32,
        new_speed: &mut f32,
        canceled: &mut bool,
    );
}
