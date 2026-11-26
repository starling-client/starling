//! The `spawner_active` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether a monster spawner in `dimension` is active this tick: vanilla's
    /// `BaseSpawner.isNearPlayer`, an alive, non-spectator player within its
    /// `RequiredPlayerRange` (always, for a negative range). `active` comes in as that answer.
    /// Set it true and the spawner ticks with no player near, or false and it stays idle with
    /// one beside it, without rewriting the spawner's saved `RequiredPlayerRange`. `spawner` is
    /// a spawner block (`Block`) or a spawner minecart (`Minecart`, server only); a trial spawner
    /// has its own rules and is not asked. Asked on both sides, each tick before the spawner
    /// does anything: on the server before `serverTick` counts its delay down and spawns (the
    /// `spawnerBlocksWork` rule still applies after it), on a client before `clientTick` spins
    /// the mob and adds its particles.
    /// Register the mod on both sides and answer the same on each, as the client draws what
    /// the server does. Diverges from upstream: new hook (NeoForge has no event; mods mix into
    /// `isNearPlayer`).
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        host = "read"
    )]
    fn spawner_active(dimension: &str, spawner: Spawner, active: &mut bool);
}
