//! The `render_living_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After a living entity is drawn (NeoForge's `RenderLivingEvent.Post`): once a frame for each
    /// mob and player in view whose `render_living_pre` wasn't canceled, after its model, its vanilla
    /// layers and the mods' layers. `entity` is its render state, as `render_living_pre` has it;
    /// `draw` is posed at its feet, turned with its body, and what it draws is kept. `event.model`
    /// borrows the final [`crate::FrameModel`] the entity's layers saw, including custom pack
    /// models and the pose under layers-only draws. It is read-only and scoped to this callback.
    /// Use `part(name)` or the hierarchy to read a part's local `pose()` and effective `visible`
    /// flag after animation and Pre edits. A successful replacement renderer or other draw
    /// without a body pose supplies `None`.
    /// The order today: before this hook its model, its vanilla layers and the mods'
    /// layers are drawn; after it, the frame's later passes draw its name tag, its leash, and
    /// the flames of a burning entity and its shadow, each as its own pass. This hook neither
    /// sees nor replaces the flames and the shadow, and a canceled `render_living_pre` doesn't
    /// stop them. A type with a mod's registered renderer (`register_entity_renderers`) has the
    /// same later passes, the renderer in the model's place. The reference draws the leash and
    /// the name tag before this hook: that move is the follow-up in #1285. For a player, its
    /// `render_player_post` follows. In the ordered body pass, each entity's hooks finish
    /// before the next entity's submission begins, including any general entity renderer at its
    /// own level join slot.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 5,
        contract = "listen",
        neoforge = "RenderLivingEvent.Post",
        cancellable = false
    )]
    fn render_living_post(
        entity: &MobRender<'_>,
        model: Option<&dyn FrameModel>,
        draw: &mut dyn BlockEntityDrawContext,
    );
}
