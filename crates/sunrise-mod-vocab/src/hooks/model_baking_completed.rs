//! The `model_baking_completed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Every model of a resource load is baked and published (`ModelEvent.BakingCompleted`): the
    /// block and item models and the standalone ones mods asked for (`standalone_models`), in the
    /// store `model` now reads. Fires once per successful model load, after both
    /// `texture_atlas_stitched` calls, at the model manager's apply step on first load and reload.
    /// Listeners ordered after the model manager apply after this hook. It precedes `client_setup`
    /// on startup and `client_resources_reloaded` on later reloads. A refused prepare fires
    /// nothing, and the old handles stay valid. Resolve models once through `render_assets().model`
    /// and keep the `ModelRef`s until the next `model_baking_completed`, when the old ones answer
    /// `ModelError::StaleModel`. Use `host.render_assets().baked_model(key)` to borrow published
    /// geometry during this callback; it does not edit the store. Never look them up by name every
    /// frame. Client-side. A panic rolls back the callback's journaled writes; the mod is reported
    /// and its callback skipped for the session without suppressing other mods' notifications.
    #[notes = r#"## Model baking completed

`model_baking_completed` (NeoForge's `ModelEvent.BakingCompleted`) fires on the render thread once per successful model
load at the model manager's apply step, right after both atlases' `texture_atlas_stitched`. Listeners ordered after
the model manager apply after it. The store `render_assets().model(&id)` reads is already the new one, the standalone
models included. Resolve the models a mod draws there, once, and keep the `ModelRef`s until
the next `model_baking_completed`, when the old ones answer `ModelError::StaleModel`; a reload that fails fires nothing
and leaves them valid. Use `host.render_assets().baked_model(key)` during the callback to borrow the published geometry
without editing it. A dedicated server never fires it."#]
    #[hook(
        dispatch = "direct",
        staged_surface = "host",
        side = "client",
        contract = "listen",
        cancellable = false,
        neoforge = "ModelEvent.BakingCompleted",
        version = 3
    )]
    fn model_baking_completed();
}
