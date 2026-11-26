//! The `player_xp_level_change` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player's level is about to change through
    /// `Player.giveExperienceLevels`: once per level gained or lost by points,
    /// and for the anvil, the enchanting table's debit, `/experience add ... levels`
    /// and `PlayerRef::give_levels`. Absolute `setExperienceLevels` writes do not ask.
    /// Change `levels`, or set `cancel` to leave the level unchanged. The
    /// player's state is swapped in, so `host.players().acting_player()`, `PlayerRef::tell`
    /// and the item helpers reach them. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PlayerXpEvent.LevelChange",
        cancellable = true,
        version = 2
    )]
    fn player_xp_level_change(player: u128, name: &str, levels: &mut i32, cancel: &mut bool);
}
