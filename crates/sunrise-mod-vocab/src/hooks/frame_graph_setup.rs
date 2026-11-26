//! The `frame_graph_setup` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait DrawContexts {
    #[draw_context(
        surface = "crate::world_geometry::FrameGraphSurface",
        name = "FrameGraphContext"
    )]
    fn frame_graph_context();
}

pub trait Hooks {
    /// Set up data-driven mod offscreen render passes at the start of level rendering, before
    /// world passes execute (NeoForge 26.3's `FrameGraphSetupEvent`, vanilla `FrameGraphBuilder`).
    /// Client-only, noncancellable. `view` lends the frame's `Camera`, world partial tick, game
    /// time, improved-transparency state and visible `RenderableSection` slice; `graph` allocates
    /// transient per-frame targets and records offscreen passes ordered relative to
    /// `RenderLevelStage` anchors, `LevelTargetBundle` built-in targets, and imported mod
    /// `RenderTargetRef` targets.
    #[notes = r#"## Frame graph setup

`Hooks::frame_graph_setup` is the client counterpart of NeoForge 26.3's `FrameGraphSetupEvent`
and vanilla 26.3's `FrameGraphBuilder`. It fires once per world view at the start of level
rendering, before world passes run, and never fires on title or connect screens.

Mods record passes as data via `event.graph.add_pass(desc, &mut |ctx| { ... })`, reading and
writing:
- vanilla 26.3 `LevelTargetBundle` built-in targets (`FrameTarget::MainColor`,
  `FrameTarget::MainDepth`, `FrameTarget::EntityOutline`, and the OIT targets),
- imported mod-owned offscreen targets (`FrameTarget::Mod(RenderTargetRef)`, always kept and
  never culled), and
- transient per-frame targets allocated via `event.graph.create_transient(&desc)` (pooled across
  frames and released after their last reader pass; passes writing only to unread transient
  targets are culled unless `disable_culling()` is set).

Ordering anchors reuse the existing `RenderLevelStage` enum (`AfterSky`, `AfterOpaqueBlocks`,
`AfterOpaqueFeatures`, `AfterTranslucentFeatures`, `AfterTranslucentBlocks`,
`AfterTranslucentParticles`, `AfterWeather`, `OitTranslucent`, `AfterLevel`). Kept passes are
topologically sorted with deterministic registration-order tie-breaking; cycles are refused with
`FrameGraphError::Cycle` (logged once per distinct cycle, never panicking). Under an active
Iris/OptiFine shader pack, scheduled passes run at the pack's corresponding world boundaries
before `final` composition."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "FrameGraphSetupEvent",
        cancellable = false
    )]
    fn frame_graph_setup(view: &mod_api::WorldRenderView<'_>, graph: &mut dyn FrameGraphContext);
}
