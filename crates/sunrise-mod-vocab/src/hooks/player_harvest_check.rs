//! The `player_harvest_check` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player's ability to harvest the block at `pos` may be changed.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "collect",
        out = "can_harvest",
        neoforge = "PlayerEvent.HarvestCheck",
        cancellable = false
    )]
    fn player_harvest_check(player: u32, pos: [i32; 3], state: u32, can_harvest: &mut bool);
}
