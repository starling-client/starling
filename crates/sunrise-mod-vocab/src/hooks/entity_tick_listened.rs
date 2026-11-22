//! The `entity_tick_listened` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0156

pub trait Hooks {
    /// Whether a mod listens to `entity_tick_pre` and `entity_tick_post`: a mod
    /// that hooks them hooks this one too and sets `listened` (HEAD, `*listened = true`).
    /// Without it the game tells no entity's tick, and keeps no list of them. Asked once for
    /// each family each tick, on the server and on a client (`host` says which side). Contract
    /// version 2: version 1 was asked on the server only.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_entity_tick_listened",
        dispatch = "flat",
        side = "both",
        version = 2,
        contract = "collect",
        out = "listened",
        cancellable = false,
        neoforge = "none (Sunrise-only; NeoForge has no counterpart); an opt-in gate for a hook"
    )]
    fn entity_tick_listened(listened: &mut bool);
}
