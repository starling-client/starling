//! The `extract_block_screen_effect` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before an overlay from `ScreenEffectRenderer` is drawn, once a frame per eligible overlay:
    /// `Block` with the view-blocking `state` at `pos` the camera is
    /// inside, `Water` with water's default `state` at the eye's block `pos`, and `Fire` with
    /// fire's default `state` at the player's block `pos`. Never for a spectator; `Water` and
    /// `Fire` only in first person. Water eligibility comes from the player's eye fluid, sampled
    /// through death tick 20. While the death screen stays open after removal, extraction keeps
    /// that last sample. `camera_submersion` can still override it at the eye.
    /// Cancel to leave the vanilla overlay out, except that a selected fire renderer still decides
    /// whether its vanilla quads follow it. Use [`ExtractBlockScreenEffectEvent::set_custom_renderer`]
    /// to prepend or replace the quads. A failed listener leaves the previous choice and
    /// cancellation intact so later eligible listeners still run. A panic disables that mod's
    /// listener for the session and is reported in the log and the client error toast.
    /// `partial_tick` is world interpolation; `player_partial_tick` is local-player interpolation,
    /// which continues when the level is frozen. Client-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ExtractBlockScreenEffectEvent",
        cancellable = true,
        version = 3
    )]
    #[notes = r##"## Block screen effects

`extract_block_screen_effect` fires before block, water and fire overlays are drawn. Water and fire are first-person only. Water uses the player's eye fluid sampled through death tick 20 and kept after removal until respawn. `camera_submersion` can override that sample. The event carries the default water state at the player's eye block, including inside a waterlogged block. The event's `set_custom_renderer` is NeoForge's `setCustomRenderer`: the last eligible listener to set a renderer for an overlay wins. Cancellation skips listeners that did not opt in with `#[listener(receive_cancelled)]`, but it does not clear a renderer. A canceled overlay with no renderer draws nothing. For block and water, cancellation suppresses the vanilla quads even if a renderer was selected. For fire, cancellation uses a no-op only if no renderer was selected; otherwise a renderer that returns `false` still precedes vanilla fire.

After extraction, the game calls each selected renderer's `submit` in block, water, fire order. It submits just before its own overlay and can return `true` to suppress that overlay's vanilla quads. A failed submission contributes no quads and does not change extraction's vanilla drawing decision. A panic disables that mod's submission for that overlay kind for the session, not its extraction hook or its other overlay submissions. Panics name the mod in the log and client error toast. Earlier overlays, later overlays and the HUD's camera overlays are unaffected. `CameraDrawContext` takes camera-space quads under the first-person hand's projection, after the level and hand and before the GUI. The player and camera can be read on `ClientHost`; `partial_tick` is `getWorldPartialTick` at both stages, and `player_partial_tick` is `getPlayerPartialTick` at extraction. Nothing drawn outlives the frame, and no GPU handle is exposed."##]
    fn extract_block_screen_effect(
        effect: mod_api::BlockScreenEffect,
        state: mod_api::BlockState,
        pos: BlockPos,
        partial_tick: f32,
        player_partial_tick: f32,
        canceled: &mut bool,
        renderer: &mut Option<mod_api::__private::SelectedBlockScreenEffectRenderer>,
    );
}
