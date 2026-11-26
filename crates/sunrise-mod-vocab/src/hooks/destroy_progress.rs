//! The `destroy_progress` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockBehaviour.getDestroyProgress(state, player, level, pos)`: the share of `state` that
    /// one tick of mining by `player` removes at `pos`, which the dig adds up until it
    /// reaches 1. Answer for your own block in place of the game's math (hardness, tool, harvest
    /// rule, efficiency, effects, block-break speed, submerged mining, the on-ground penalty and
    /// `player_break_speed`), as a block that mines like another overrides it in vanilla: leave
    /// `progress` unset to keep that math. `0.0` never breaks; `1.0` or more breaks at once.
    /// To mine as another block, ask `PlayerRef::destroy_progress` for that block's position and
    /// state from here, on the server (it takes a server host): inside this answer that query
    /// gives the game's own math for the state and asks no Part again, so a chain of answers
    /// cannot run without bound.
    /// Asked on both sides wherever the dig's progress is, so a client predicts it: register the
    /// mod on both and answer the same from both. A client without the mod predicts the game's
    /// math, and the server's dig decides. Every tick of a dig asks it: keep it short.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "replace",
        out = "progress",
        scope = "block",
        key = "state",
        neoforge = "none (vanilla BlockBehaviour.getDestroyProgress; NeoForge adds no event or extension for it)"
    )]
    fn destroy_progress(
        player: PlayerRef,
        pos: [i32; 3],
        state: BlockState,
        progress: &mut Option<f32>,
    );
}
