//! The `player_bed_enter` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0278

pub trait Hooks {
    /// A server player is about to enter a bed after the sleep checks pass. Set `cancel` to keep them
    /// out. The player's state is swapped in.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_player_bed_enter",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerBedEnterEvent)",
        cancellable = true
    )]
    fn player_bed_enter(player: u128, name: &str, pos: [i32; 3], cancel: &mut bool);
}
