//! The `sleep_status_announce` hook: a declaration the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The players or the sleepers of level `dimension` changed, and the game is about to show
    /// everyone the action-bar line "n/m players sleeping", or "Sleeping through this night" once
    /// enough are in bed (vanilla's `ServerLevel.announceSleepStatus`, reached from
    /// `updateSleepingPlayerList`). Asked after vanilla's own checks passed: the
    /// `players_sleeping_percentage` rule allows sleeping through nights (`canSleepThroughNights`)
    /// and the world is not a single-player one that is not published. It is not asked when
    /// nothing changed or the line would not be sent.
    ///
    /// `sleeping` is how many players are in bed, `active` how many count toward sleeping (not
    /// spectators), `percentage` the `players_sleeping_percentage` game rule and `needed`
    /// vanilla's `SleepStatus.sleepersNeeded` for it. `skipping_night` is true when the line is
    /// "Sleeping through this night" (`areEnoughSleeping`), false for the count. `line` starts as
    /// vanilla's component and is what every player receives, so a listener may replace it; a
    /// later mod reads what an earlier one set. Server-side.
    ///
    /// Set `canceled` and nobody gets the line. Only the line is dropped: who sleeps and when the
    /// night skips are unchanged (`sleep_skip_night`). NeoForge has no such event.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn sleep_status_announce(
        dimension: &str,
        sleeping: i64,
        active: i64,
        percentage: i32,
        needed: i64,
        skipping_night: bool,
        line: &mut Text,
        canceled: &mut bool,
    );
}
