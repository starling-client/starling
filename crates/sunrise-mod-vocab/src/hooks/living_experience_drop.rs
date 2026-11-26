//! The `living_experience_drop` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A dying mob or player `entity` of `kind` is about to drop `original`
    /// points of experience; the vanilla conditions have been checked.
    /// `attacking_player` is the player who last hurt it, if any. Change
    /// `amount` (0 or less drops nothing), or set `cancel` to drop none. A
    /// sculk catalyst that takes the experience does not call this. As in
    /// NeoForge's `dropAllDeathLoot`, it fires after the loot rolls and just
    /// before `living_drops` hears the item list. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LivingExperienceDropEvent",
        cancellable = true
    )]
    fn living_experience_drop(
        entity: u32,
        kind: &str,
        attacking_player: Option<u128>,
        original: i32,
        amount: &mut i32,
        cancel: &mut bool,
    );
}
