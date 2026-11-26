pub trait DrawContexts {
    #[draw_context(
        surface = "crate::world_geometry::WorldGeometry",
        name = "WorldDrawContext"
    )]
    fn world_draw_context();
}
pub trait Hooks {
    /// Submit unlit geometry after particle submission, before opaque rendering
    /// (NeoForge 26.3 `SubmitCustomGeometryEvent`, vanilla `LevelRenderer.submitFeatures`).
    /// Client-only, noncancellable. `view` lends the frame's `Camera`, world partial tick, game
    /// time, improved-transparency state and visible `RenderableSection` slice. Submit from
    /// extracted state.
    #[notes = r#"## Custom world geometry

`Hooks::submit_custom_geometry` runs on the client after particles, before rendering. Its
`WorldRenderView` lends the frame's `Camera`, world partial tick, game time, whether vanilla's
improved transparency (OIT) is active, and the visible `RenderableSection` slice;
`WorldDrawContext::lines` and `quads` accept unlit world-space ARGB geometry with depth testing or
through walls. `lines` and `quads` fail atomically for invalid inputs; `pipeline_quads` checks
`RenderPipelineRef` validity and staleness (`PipelineDrawError::Pipeline`) and skips any quad with
non-finite coordinates or UVs individually without rejecting valid quads in the same slice.
Backing storage grows dynamically and output clears every view. Submit extracted state, without
world writes or resource loading.

`WorldDrawContext::draw_model_instances` draws one batch of a resolved `ModelRef`. Each
`ModelInstance` carries an f64 world origin, a row-major 3×4 local transform, an `0xAARRGGBB`
tint (the same convention as `WorldQuad::color`) and `LightCoords` block and sky light, each
0 to 15, in vanilla's own convention. The engine converts tint and light to the frame format;
mods never assume that format or supply a packed light word. Copies outside the frustum are
culled first, and any individual visible copy with non-finite coordinates/transforms or light
outside `0..=15` is skipped without rolling back earlier valid copies. Backing storage grows
dynamically, with copies past GPU buffer room falling back to CPU baking.
`ModelInstance::color` (`0xAARRGGBB`, white by default) multiplies every face's colour; with alpha below `0xFF`, or for a model with a translucent face, the copy is drawn in the sorted translucent material (vanilla's `translucent_moving_block`), its quads far to near. `ModelInstance::hidden_faces` leaves out faces whose `cull` side is set.
Shader packs and any route that cannot
take GPU instances bake the same prepared geometry on the CPU instead of dropping the model; a
`shaders/` pack shades those copies as block entities, as it shades `BlockEntityDrawContext::model`.

`WorldDrawContext::with_pipeline_modifier` pushes a registered `PipelineModifierRef` for the
duration of its callback (nesting outermost-wins, matching `PipelineModifierStack.apply`),
applying to `lines`, `quads`, `pipeline_quads`, and `draw_model_instances` (not `name_tag`). Only the
modifier's `depth` applies to the built-in `lines`, `quads` and `draw_model_instances`; the whole
modifier applies to a registered pipeline drawn with `pipeline_quads`.

`WorldDrawContext::name_tag(at, &text, NameTagOptions::new(see_through, background, light))` draws
a label as vanilla draws a name tag (`submitNameTag`): 0.5 above `at`, facing the camera, 0.025
blocks a font pixel, so it shrinks with distance (scale by distance yourself for a constant size),
centred, in the text's styles. `background` puts the black backdrop behind it at the player's
text background opacity; `light` is the light at the label, sky light in the high four bits and
block light in the low four. Without `see_through` it is drawn once, depth tested and translucent:
the faded text on the backdrop, in `light` as given, hidden behind what is in front. With
`see_through` it is drawn twice: opaque in the world with no backdrop, its sky and block light each
raised to at least 2, and faded on the backdrop over everything, walls included, unlit. The
backdrop's alpha is 255 times the text background opacity (0.25 when it is set for chat only), the
faded text's 255 times the larger of (opacity + 0.75) / 2 and 0.5. Labels draw with the world's
name tags whatever the stage, sorted far to near. A position that is not
finite (`DrawError::NonFinite`) is refused whole: nothing is drawn. Only visible text and styles
are used, not click, hover or opaque metadata. `see_through` can show a
label through any wall, which vanilla does only for the names its own renderers choose; the call
sends nothing and reads nothing the client does not already hold."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "SubmitCustomGeometryEvent",
        cancellable = false,
        version = 2
    )]
    fn submit_custom_geometry(view: &mod_api::WorldRenderView<'_>, draw: &mut dyn WorldDrawContext);
}
