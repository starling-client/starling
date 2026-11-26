//! Client block break sound.
pub trait Hooks {
    /// Asked before the client plays a block's break sound: level event 2001, which a server
    /// sends for a broken block and the client's own predicted break also plays; the server thread
    /// never asks it. Return `Flow::Cancel` when the part played its own sound through
    /// `ClientHost` (or wants none) and vanilla should stay silent. Otherwise vanilla plays the state's break sound at the block's centre,
    /// at `(volume + 1) / 2` and pitch `* 0.8`. Air is never asked. The destroy particles that
    /// follow are `add_destroy_effects`' business and are drawn either way.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        scope = "block",
        key = "state",
        neoforge = "IClientBlockExtensions.playBreakSound"
    )]
    fn play_break_sound(state: BlockState, pos: BlockPos, cancel: &mut bool);
}
