//! The `entity_tick_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0155

pub trait Hooks {
    /// The entities `entities` ticked in the level `dimension`, and `kinds` holds the type of
    /// each, in the same order (NeoForge's `EntityTickEvent.Post`): those `entity_tick_pre`
    /// was told and did not cancel, also one that died or was removed in its tick. An entity
    /// `entity_tick_pre` was told alone is told alone here too, right after its own tick; a
    /// family is told once, after its last tick. Told where `entity_tick_pre` is, on the
    /// server and on a client (`host` says which side). Version 3 observes additional families
    /// without changing the event signature.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_entity_tick_post",
        dispatch = "flat",
        side = "both",
        version = 3,
        contract = "listen",
        neoforge = "EntityTickEvent.Post",
        cancellable = false
    )]
    fn entity_tick_post(dimension: &str, entities: &[u32], kinds: &[&str]);
}
