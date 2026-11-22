//! The `village_siege` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A village siege found a spawn position on the server. Set `canceled`
    /// to stop the siege. There is one siege per dimension.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "VillageSiegeEvent",
        cancellable = true
    )]
    fn village_siege(dimension: &str, player: u128, attempted_pos: [f64; 3], canceled: &mut bool);
}
