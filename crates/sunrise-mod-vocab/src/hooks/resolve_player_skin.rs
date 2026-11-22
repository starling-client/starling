//! The `resolve_player_skin` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Select the local player's skin texture and model together, at vanilla's
    /// `AbstractClientPlayer.getSkin` moment. Asked once per frame with a local avatar,
    /// before the third-person body and skin layers, first-person arms and inventory
    /// previews are extracted. Client-side, with a read-only host; it does not change
    /// the player's profile, cape, tab-list entry or what other clients see.
    ///
    /// `selection` starts at `None`. Listeners share it in dispatch order: leave it
    /// unchanged, replace it, or clear it. The final answer applies only to this
    /// frame. An absent, freed or non-64×64 dynamic texture keeps both vanilla values.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "selection",
        version = 1,
        neoforge = "none (vanilla AbstractClientPlayer.getSkin)"
    )]
    fn resolve_player_skin(selection: &mut Option<mod_api::PlayerSkinOverride>);
}
