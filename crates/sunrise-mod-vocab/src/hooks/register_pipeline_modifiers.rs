//! Client pipeline modifier registration.

pub trait Hooks {
    /// Register pipeline modifiers once at client startup (NeoForge's
    /// `RegisterPipelineModifiersEvent`): call `event.register(modifier)` with each
    /// [`PipelineModifier`]. Modifiers are plain data state overrides (blend, depth compare,
    /// depth write, depth bias, face culling, colour write mask, polygon mode, primitive
    /// topology) applied during world drawing via [`WorldDrawContext::with_pipeline_modifier`].
    /// Duplicate modifier IDs and non-finite depth-bias slope scales fail startup.
    #[notes = r#"## Pipeline modifiers

`Hooks::register_pipeline_modifiers` is the client registration-phase counterpart of NeoForge's
`RegisterPipelineModifiersEvent`. A modifier is plain data (`PipelineModifier`): each field is an
optional override of `PipelineState` (`blend`, `depth_compare`, `depth_write`, `depth_bias`,
`cull`, `color_write`, `polygon`, `topology`). Because modifiers are data rather than closures,
applying them is idempotent by construction and the renderer caches each `(pipeline, modifier
stack)` variant lazily.

```rust,ignore
fn register_pipeline_modifiers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterPipelineModifiersEvent<'_>,
) {
    event.register(
        PipelineModifier::new(Identifier::parse("example:xray").unwrap())
            .depth(WorldDepth::ThroughWalls)
            .depth_write(false),
    );
}
```

Resolve a modifier once in `client_setup` (or `client_resources_reloaded`) with
`host.render_assets().pipeline_modifier(&id)` and apply it around draws with
`WorldDrawContext::with_pipeline_modifier(modifier, &mut |draw| { ... })`. Scopes nest: the most
recently pushed modifier applies first, then older ones, so for any field set by two modifiers
the outermost wins (`PipelineModifierStack.apply`). Modifiers apply to `lines`, `quads`, `pipeline_quads`, and `draw_model_instances` (not
`name_tag` labels), but not all of a modifier applies everywhere. The built-in `lines`, `quads`
and `draw_model_instances` have no pipeline of their own to vary, so only `depth` applies to them
(it picks the through-walls or tested copy). The full `PipelineModifier` (`blend`, `cull`,
`color_write`, `depth_write`, `polygon`, `depth`) applies to pipelines a mod registers and draws
with `pipeline_quads`. A field a modifier leaves unset keeps the pipeline's own value, which
defaults to what a mod pipeline already had before `PipelineState` existed. On OIT variants, `blend`
and `depth_write` overrides are ignored because OIT owns target blending and read-only depth.
Variants the GPU refuses (such as `PolygonMode::Line` on a device without wireframe support) are
logged once with both IDs and skipped while the rest of the frame draws."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "modifiers",
        registration_append = "attributed",
        context = "registration",
        neoforge = "RegisterPipelineModifiersEvent"
    )]
    fn register_pipeline_modifiers(modifiers: &mut Vec<mod_api::PipelineModifier>);
}
