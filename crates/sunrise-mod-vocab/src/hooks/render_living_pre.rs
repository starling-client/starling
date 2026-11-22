//! The `render_living_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a living entity is drawn (NeoForge's `RenderLivingEvent.Pre`): once a frame for each
    /// mob and player in view (one the frame culls is not asked), before its pose, model and
    /// layers. `entity` is its render state (its id, kind, custom name, the frame's partial tick,
    /// its body and head turns and walk cycle, `LivingEntityRenderState`). `draw` is posed at its
    /// feet, turned with its body: +z the way it faces, +x its left, +y up, in blocks, with the
    /// helpers block entities draw with (`cube`, `quad`, `block`, `item`, `model_part`); what it
    /// draws is kept. Set `canceled` to skip all of the entity's drawing this frame: its model,
    /// armour, held items and layers, the mods' layers, its name tag, leash and glowing outline,
    /// and `render_living_post`, and the renderer a mod registered for its type
    /// (`register_entity_renderers`), which draws in the model's place; a mod drawing a model of
    /// its own draws it here and cancels. The flames of a burning entity and its shadow, which the
    /// frame queues after the entity's drawing, still draw. A player's `render_player_pre` comes first and edits the
    /// same `event.model`; a cancel here still calls `render_player_post` with `model: None`.
    /// `event.model` lends the selected body's baked hierarchy and bind pose as a
    /// [`crate::FrameModelEdit`]. Use `model.part("nose")` to get a villager's nose index, then
    /// `model.hide(index)` to leave it and its descendants out of the body and layers drawn on
    /// that model (such as a spider's eyes). Armour, held items and independent layers are
    /// unaffected. Missing names return `None`; invalid indices return `false` without an edit.
    /// Edits last for this draw only; `draw.keep()` does nothing here. In the ordered body pass,
    /// this hook follows level join order, including this window's player at its own slot,
    /// interleaved with general entity renderer submissions.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 7,
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderLivingEvent.Pre",
        cancellable = true
    )]
    fn render_living_pre(
        entity: &MobRender<'_>,
        draw: &mut dyn BlockEntityDrawContext,
        model: &mut dyn FrameModelEdit,
        canceled: &mut bool,
    );
}
