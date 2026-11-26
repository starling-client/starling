//! The `world_drawn` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The world is drawn (see [`WorldDrawn`]). Not called on the title screen's panorama.
    #[hook(
        side = "client",
        contract = "listen",
        target = "mod_render::render::world_drawn",
        neoforge = "RenderLevelStageEvent.AfterLevel"
    )]
    fn world_drawn(world: &mut WorldDrawn<'_>);
}
