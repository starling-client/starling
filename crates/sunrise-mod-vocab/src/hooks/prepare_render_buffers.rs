//! The `prepare_render_buffers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait DrawContexts {
    #[draw_context(
        surface = "crate::world_geometry::WorldGeometry",
        name = "PrepareRenderBuffersContext"
    )]
    fn prepare_render_buffers_context();
}

pub trait Hooks {
    /// Prepare geometry and update retained instance data before pass-bearing world stages execute
    /// (NeoForge 26.3's `PrepareRenderBuffersEvent`, vanilla `LevelRenderer.prepareTranslucents`).
    /// Runs before a classic shader pack's sky or celestial work; `AfterSky` is not a preparation hook.
    /// Client-only, noncancellable. `view` lends the frame's camera, world partial tick, game
    /// time, improved-transparency state and visible sections.
    #[notes = r#"## Preparing render buffers

`Hooks::prepare_render_buffers` is the client counterpart of NeoForge 26.3's
`PrepareRenderBuffersEvent`. It fires once per world view before any pass-bearing world render
stages execute, whether or not the frame has an `AfterSky` stage. When a classic shader pack
owns the sky or celestial work, `AfterSky` follows preparation and the engine world import. Preparation never
fires on title or connect screens.

Use `event.context.lines(stage, lines)` or `.quads(stage, quads)` to prepare a batch for a
pass-bearing stage. Keep the
returned `PreparedWorldDraw` until `render_level_stage`, then give it to the active pass's `draw`.
A handle is valid only for its own view and stage. Nothing is uploaded from a stage callback.

Changes made here to `InstanceGroup` records, active length, or group transform apply to groups
already submitted for this frame. Its `WorldRenderView`
lends the frame's `Camera`, world partial tick, game time, whether vanilla's improved transparency
(OIT) is active this frame, and the primary camera's canonical visible `RenderableSection` slice."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "PrepareRenderBuffersEvent",
        cancellable = false
    )]
    fn prepare_render_buffers(
        view: &mod_api::WorldRenderView<'_>,
        context: &mut dyn PrepareRenderBuffersContext,
    );
}
