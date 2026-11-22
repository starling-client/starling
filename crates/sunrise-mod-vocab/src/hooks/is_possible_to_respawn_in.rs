//! The forced respawn column's block-space predicate.

pub trait Hooks {
    /// Whether a player can respawn inside `state` at a forced spawn point
    /// (`Block.isPossibleToRespawnInThis`). The server asks for the saved point
    /// and the block above it when no applicable bed or anchor rule handles
    /// the spawn, including when a bed or anchor cannot set spawn here.
    /// `Some(true)` accepts the space, `Some(false)` refuses it, and `None`
    /// keeps the block's vanilla predicate. The state alone determines the
    /// answer; the host is read-only. This does not choose a bed or anchor's
    /// stand-up position (see `respawn_position`).
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "replace",
        scope = "block",
        key = "state",
        out = "answer",
        host = "read",
        neoforge = "none (vanilla Block.isPossibleToRespawnInThis)"
    )]
    fn is_possible_to_respawn_in(state: BlockState, answer: &mut Option<bool>);
}
