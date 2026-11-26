//! The `sleepers_needed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// How many players must sleep in level `dimension` to skip the night: vanilla's
    /// `SleepStatus.sleepersNeeded`. `needed` arrives with vanilla's answer, `percentage`
    /// (the `players_sleeping_percentage` game rule) of the `active` players that count toward
    /// sleeping (not spectators), rounded up and at least 1; set it to your own count for this
    /// level and tick. An answer below 1 counts as 1.
    /// Asked on the server wherever vanilla reads the count: the night-skip check of
    /// `ServerLevel.tick` (`areEnoughSleeping` and `areEnoughDeepSleeping`, before
    /// `sleep_skip_night`) and the "n/m players sleeping" announcement, which shows the answered
    /// count. Not asked when `percentage` is over 100, where vanilla never skips the night. Asked
    /// each tick, so answer from what you already hold; don't change the world here.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "none (vanilla SleepStatus.sleepersNeeded; NeoForge adds no event)"
    )]
    fn sleepers_needed(dimension: &str, percentage: i32, active: i64, needed: &mut i64);
}
