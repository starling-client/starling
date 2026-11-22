//! The `render_player_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a player is drawn (NeoForge's `RenderPlayerEvent.Pre`): once a frame for each other
    /// player in view and for this window's player in third person (including front view) when it
    /// holds the camera, before its `render_living_pre`. `player` is its render state, `who` its profile name and whether it is
    /// this window's own player, and `draw` the frame `render_living_pre` gets. Set `canceled` to
    /// skip the player's own drawing this frame, `render_living_pre`, `render_living_post` and
    /// `render_player_post` included. Canceling only `render_living_pre` skips Living Post but
    /// still calls Player Post with `model: None`. Either cancel leaves eligible flames and shadows
    /// to the frame; a replacement body should not submit them again. `event.model` borrows the
    /// selected player's hierarchy and bind pose as a [`crate::FrameModelEdit`]. Look up `model.part("head")` and call
    /// `model.hide(index)` if it exists. That leaves the head and its children out for this draw
    /// after Skin Customization. Living Pre sees the same draw-local edits. Armour, held items
    /// and other independent layers, including the cape, are unaffected. In the ordered body pass,
    /// the pair wraps this player's living hooks at its slot in level join order, interleaved with
    /// general entity renderer submissions, for local and remote players alike.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 6,
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderPlayerEvent.Pre",
        cancellable = true
    )]
    fn render_player_pre(
        player: &MobRender<'_>,
        who: &PlayerRender<'_>,
        draw: &mut dyn BlockEntityDrawContext,
        model: &mut dyn FrameModelEdit,
        canceled: &mut bool,
    );
}
