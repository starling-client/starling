//! The `player_bucket_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0280

pub trait Hooks {
    /// A server player caught a mob in a bucket, after it was removed and the filled bucket was given.
    /// Server-side.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_player_bucket_entity",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_bucket_entity(player: u128, name: &str, entity_id: i32, entity: &str);
}
