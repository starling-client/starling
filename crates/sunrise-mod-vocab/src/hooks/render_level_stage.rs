//! The `render_level_stage` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait DrawContexts {
    #[draw_context(
        surface = "crate::world_geometry::StageSurface<'_>",
        name = "RenderLevelStageContext"
    )]
    fn render_level_stage_context();
}

pub trait Hooks {
    /// Observe or draw world geometry at an active stage of a world view (NeoForge 26.3's
    /// `RenderLevelStageEvent`). NeoForge's `RenderLevelStageEvent.AfterSky` runs once when
    /// `LevelRenderer.addSkyPass` adds the sky pass, after the engine or classic pack's sky
    /// work even if no geometry was visible. A classic pack's `AfterLevel` follows its world
    /// passes, before composition and the engine hand overlay. `context` exposes
    /// `StagePass::None` for those two stages, `StagePass::World` for standard pass stages,
    /// and `StagePass::Oit` for `OitTranslucent` sub-passes. Client-only, noncancellable.
    #[notes = r#"## World render stages

`Hooks::render_level_stage` is the client counterpart of NeoForge 26.3's
`RenderLevelStageEvent`, covering all 9 stages as one hook with a `RenderLevelStage`:
- `AfterSky` (no active render pass, immediately after the logical sky pass's drawing work)
- `AfterOpaqueBlocks` (after solid and cutout chunk geometry, before opaque features)
- `AfterOpaqueFeatures` (after opaque entities, block entities and particles)
- When classic transparency is active (`!view.improved_transparency`):
  - `AfterTranslucentFeatures` (after translucent entities and block entities)
  - `AfterTranslucentBlocks` (after translucent chunk geometry)
  - `AfterTranslucentParticles` (after translucent particles)
  - `AfterWeather` (after rain and snow, before the world border)
- When improved transparency / OIT is active (`view.improved_transparency`):
  - `OitTranslucent(OitStage::DepthBounds)`
  - `OitTranslucent(OitStage::Transmittance)`
  - `OitTranslucent(OitStage::Accumulate)`
- `AfterLevel` (no active render pass, after the level finishes rendering)

Each frame asks `Hooks::submit_custom_geometry` and registered debug renderers first (with
`WorldDrawContext`, for `name_tag`, `draw_model_instances`, `draw_instance_group`, and custom
world primitives drawn at the end of the world pass). `Hooks::prepare_render_buffers` comes before
primitive, entity, prepared-instance, and glyph uploads so group updates apply to this frame.
`AfterSky` runs when `LevelRenderer.addSkyPass` adds a pass: the dimension has a skybox,
there is no lava or powder-snow camera fog, and no camera mob effect blocks the sky. It fires
once at the end of that pass's work even when no sky geometry is visible. Sunrise's sky setting
and dragon boss fog affect drawing, not admission. A classic pack owning either the sky disc
or the celestial work completes the logical sky after its world import, with the engine still
drawing the unclaimed portions. Each active pass-bearing stage fires at its own render-pass
boundary. `AfterLevel` follows the world and precedes the engine's first-person hand and other
overlays. A classic pack's own hand remains inside its world passes for the pack's depth
snapshots; `AfterLevel` runs before pack composition, not before that hand. These hooks never
fire on title or connect screens.

`event.context` is a `RenderLevelStageContext`: `AfterSky` and `AfterLevel` return
`StagePass::None` (`has_render_pass() == false`); standard pass stages return
`StagePass::World(&mut dyn WorldPassDrawContext)`, and `OitTranslucent(oit)` returns
`StagePass::Oit(oit, &mut dyn WorldOitPassDrawContext)`. Call `pass.draw(batch)` with a
`PreparedWorldDraw` returned by `event.context` in `prepare_render_buffers` for this view and
stage. Both pass kinds draw prepared engine lines and quads in call order. `RenderLevelStage`
and `OitStage` are process-local selectors, not `Wire` values.

Stages that do not run on the active route are not synthesized. When a classic shader pack owns
terrain, water, features, or weather and exposes a float-blendable world color buffer
(`colortex0`), `AfterOpaqueBlocks`, `AfterOpaqueFeatures`, `AfterTranslucentFeatures`,
`AfterTranslucentBlocks`, and `AfterWeather` fire at the pack's own passes (before or after
deferred according to the pack's feature schedule) against the pack's color and depth targets.
On an incompatible (integer or unblendable) `colortex0`, those pack-owned stages are omitted
rather than firing early on a mismatched target.

`PrepareRenderBuffersContext::lines`, `quads`, and `pipeline_quads` prepare batches for any
pass-bearing stage (including `OitTranslucent`). At `OitTranslucent`, a mod pipeline must be
registered with `.order_independent()` and a non-`Opaque` blend mode; others are refused with
`PipelineDrawError::Pipeline(RenderPipelineError::Invalid)` and logged once per mod and pipeline.

A panic disables that mod's listener for this hook for the session, with an error log and toast.
Draws already accepted are not rolled back."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "RenderLevelStageEvent",
        cancellable = false,
        version = 4
    )]
    fn render_level_stage(
        stage: mod_api::RenderLevelStage,
        view: &mod_api::WorldRenderView<'_>,
        context: &mut dyn RenderLevelStageContext,
    );
}
