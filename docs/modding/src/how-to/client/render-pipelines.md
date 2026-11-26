# Drawing with your own render pipeline

Draw world quads with your own GLSL core shader program. Register a `RenderPipeline`, resolve
its handle, and submit quads through a `WorldDrawContext` (in `submit_custom_geometry` or
a debug renderer). This is the counterpart of NeoForge's `RegisterRenderPipelinesEvent`: you
write vanilla-style GLSL core shaders (`.vsh` and `.fsh`), optionally reusing an existing vertex
shader such as `minecraft:core/position_tex_color`, and the game translates them via
`sunrise-glsl` and builds every GPU object.

## 1. Write the shaders

Put GLSL files at `assets/<namespace>/shaders/core/<path>.vsh` and `.fsh` (referred to by the
identifier `<namespace>:core/<path>`, matching vanilla's `withVertexShader` and
`withFragmentShader`). Shared includes live under `assets/<namespace>/shaders/include/` and are
brought in with `#include <namespace:file.glsl>`, `#include "relative.glsl"`, or `#moj_import`
through the active resource stack (vanilla assets, loaded mods, and resource packs).

```glsl
// assets/example/shaders/core/glow.fsh
#version 330
#include <minecraft:globals.glsl>
#include <minecraft:dynamictransforms.glsl>

in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;

void main() {
    vec4 color = vertexColor * ColorModulator;
#ifdef PULSE
    color.rgb *= 1.0 + sin(GameTime * 24000.0 * 0.2);
#endif
    fragColor = vec4(color.rgb, color.a * (1.0 - texCoord0.y));
}
```

If you do not need custom vertex math, you can omit `glow.vsh` and call
`.vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())` on registration.
When writing a custom `.vsh`, declare any subset of the vertex attributes by vanilla name:

| Input | Type / Include | Value the game binds |
| --- | --- | --- |
| `Position` | `in vec3 Position;` | Quad corner in blocks, camera-relative. |
| `UV0` | `in vec2 UV0;` | Quad texture coordinate as given on the corners. |
| `Color` | `in vec4 Color;` | Quad colour in vanilla's gamma space. |
| `Globals` | `#include <minecraft:globals.glsl>` | Frame globals (`CameraBlockPos`, `GlintAlpha`, `CameraOffset`, `GameTime`, `ScreenSize`, `MenuBlurRadius`, `UseRgss`). |
| `Projection` | `#include <minecraft:projection.glsl>` | View projection (`ProjMat`, reversed 0..1 depth; `RENDERPEARL_DEPTH_IS_ZERO_TO_ONE` is always defined). |
| `DynamicTransforms` | `#include <minecraft:dynamictransforms.glsl>` | `ModelViewMat` (camera rotation), `TextureMat` (identity), `ColorModulator` (white), `ModelOffset` (zero). |
| `Fog` | `#include <minecraft:fog.glsl>` | Frame fog (`FogColor`, environmental and render-distance starts/ends, sky and cloud ends). |
| `Sampler0` / `sampleSprite` | `#include <sunrise:pipeline_sprite.glsl>` | Sprite atlas and `vec4 sampleSprite(vec2 uv)` when `.textured(...)` is set. |

### Colour space

Mod pipeline GLSL programs work in **vanilla's gamma space**, just like vanilla core shaders:
write `texture * vertexColor * ColorModulator` directly. On frames storing vanilla's encoding the
output is stored as written; on frames holding linear light the engine automatically decodes the
fragment shader's location-0 output after `main()` (clamping negative or NaN RGB to 0 and alpha
coverage to `[0, 1]`).

### Crash and memory containment bounds

Because Sunrise translates untrusted GLSL in-process with naga rather than handing raw text to
the GPU driver, expanded shader stages are checked against crash-containment bounds before naga
parses them: 1 MiB expanded bytes, include depth 16 (1,024 pastes, 8 MiB scanned), at most 200
chained binary/member/index operators in one expression, and at most 200 nested brackets,
consecutive unary prefix operators, or brace-less control statements. Split long expressions
with local variables and add braces where needed.

## 2. Register the pipeline

Call `event.register(pipeline)` in `register_render_pipelines` at client startup.
The event adds declarations in order and does not expose earlier registrations.
Resource reloads rebuild the registered pipelines from the current shader and include files;
they do not call the registration hook again.

```rust,ignore
fn register_render_pipelines(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRenderPipelinesEvent<'_>,
) {
    event.register(
        RenderPipeline::new(
            Identifier::parse("example:glow").unwrap(),      // the pipeline's id
            Identifier::parse("example:core/glow").unwrap(), // assets/example/shaders/core/glow.{vsh,fsh}
            PipelineBlend::Additive,
            WorldDepth::Tested,
        )
        .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
        .define("PULSE")
        .define_value("INTENSITY", "2"),
    );
}
```

- `.vertex_shader(id)` reuses another program's `.vsh` (for example `minecraft:core/position_tex_color`
  or a library mod's vertex shader). Omit it when your own `<path>.vsh` sits beside `<path>.fsh`.
- `.define("NAME")` and `.define_value("NAME", "VALUE")` compile preprocessor macros into both
  stages (matching vanilla's `withShaderDefine`), letting several pipelines share one GLSL
  program. Define names must be valid non-`GL_` GLSL identifiers and values must be single-line
  tokens without comments, backslashes, or `#`; invalid defines fail startup registration.
- `PipelineBlend::Translucent` lays the result over the frame by its alpha, and `Additive` adds
  it, scaled by its alpha. `WorldDepth::Tested` hides the quads behind nearer world surfaces, and
  `ThroughWalls` draws them over everything. Neither preset enables depth writes; use
  `.depth_write(true)` to request them.

Any valid `Identifier` is accepted. Use a unique ID for every mod pipeline, regardless of its
required or optional policy. A mod ID collision fails startup. Collisions with vanilla are checked
in the corresponding required or optional table. If startup reports missing or invalid
`render-pipelines.json`, prepare the client resources again.

### Required or optional

A pipeline is required unless you register it with `.optional()`, as NeoForge's `registerPipeline`
and `registerOptionalPipeline` are:

```rust,ignore
event.register(
    RenderPipeline::new(id, shader, PipelineBlend::Translucent, WorldDepth::Tested).optional(),
);
```

| Policy | When a resource load cannot build it |
| --- | --- |
| Required (`RenderPipeline::new`) | The load fails with a message naming each required pipeline and why. Startup cannot finish with a broken required pipeline. A failed resource reload keeps the previous resources, so handles from before it still draw. |
| Optional (`.optional()`) | The log has one line naming the pipeline and why. It resolves to `RenderPipelineError::Invalid`, and every other pipeline builds and draws. |

Client resource pack recovery only runs if an optional pack was selected. It clears and saves the
failing selection before reloading the required packs and mod content. If recovery fails, the
error is reported and that selection stays cleared. If the player has already committed a newer
selection during startup, the queued load uses it instead. Create World's data-pack editor is
unaffected.

A load cannot build a pipeline when its `.vsh`, `.fsh`, or an included file is missing, the
shader is refused, or the GPU rejects it. The game never panics over a pipeline's shader.

### Checking shaders before you run the game

`cargo sunrise-mod check`, run in the mod's folder, translates and validates every GLSL core
shader program under `assets/<namespace>/shaders/core/` with the same `sunrise-mod-pipeline`
layer the game uses, including SPIR-V (Vulkan), Metal (MSL), and DirectX 12 (HLSL) backend
translation checks.

When `SUNRISE_RESOURCES` points to your prepared 26.3 resources, `minecraft:` includes and
vertex shaders are read directly from them. When prepared resources are absent (such as in CI or
registry checks), the tool falls back to Sunrise-written interface-only stubs for `minecraft:*`
includes and standard vertex shaders so every interface error is still caught offline; the game's
resource load remains the authority. Because the tool does not execute your Rust registration
code, a `.fsh` with no `.vsh` beside it is checked against `minecraft:core/position_tex_color`,
and a program referencing `sampleSprite`, `pipeline_sprite.glsl`, `SpriteRect`, or `Sampler0` is
checked as textured.

The check reads the vertex `in`s your `.vsh` declares, each on its own: it does not assume the
default format's three. It refuses only an `in` of a type no vertex attribute can have (a
`mat4`, say). Which inputs the game binds, and from which vertex bindings, the format your
pipeline registers decides when your mod registers it; the check does not run that code.

## 3. Resolve and draw

```rust,ignore
fn client_setup(&self, context: &mut dyn ClientSetupContext, _event: &mut ClientSetupEvent<'_>) {
    let id = Identifier::parse("example:glow").unwrap();
    *self.glow.lock().unwrap() = context
        .render_assets()
        .render_pipeline(&id)
        .ok();
}

fn submit_custom_geometry(&self, _host: &dyn ClientHost, event: &mut SubmitCustomGeometryEvent<'_>) {
    let Some(glow) = *self.glow.lock().unwrap() else { return };
    let quad = PipelineQuad::spanning(corners, 0xFFFF_C040);
    if let Err(error) = event.draw.pipeline_quads(glow, &[quad]) {
        log::warn!("glow not drawn: {error:?}");
    }
}
```

Resolve again in `client_resources_reloaded`: every resource reload builds the pipelines again
from the new resources, so a resource pack can replace your shaders or includes, and a handle
from before the reload answers `Stale`.

| Error | When |
| --- | --- |
| `RenderPipelineError::Unknown` | No mod registered that id. |
| `RenderPipelineError::Invalid` | The pipeline is optional and its shader file is missing, was refused or was rejected by the GPU. The log has a line `render pipeline <id> from mod <owner> refused: <file>: <reason>` or `render pipeline <id> from mod <owner> refused by the GPU: <reason>`. |
| `RenderPipelineError::Stale` | The handle is from before a resource reload. |

`pipeline_quads` returns `PipelineDrawError::Pipeline(...)` when the handle is invalid or stale,
drawing nothing for that call. Any quad with a non-finite corner, camera-relative `f32` overflow,
or non-finite texture coordinate is ignored while the valid quads in the batch still draw, and
quad storage grows dynamically into the GPU buffers without a 4,096 cap. An `Invalid` pipeline
never stops the game from starting; a required pipeline that cannot be built does.

## What a pipeline draws, and when

`WorldDrawContext::pipeline_quads` in `submit_custom_geometry` or a debug renderer submits quads
for the deferred world pass. Quads with non-finite coordinates or UVs are skipped individually
without rejecting valid quads in the same slice; accepted pipeline quads draw after the pass's
plain quads and lines, in submission order.

## Sampling a texture

A pipeline can sample one texture (vanilla's `Sampler0`). Name it when you register the pipeline:

```rust,ignore
event.register(
    RenderPipeline::new(
        id,
        Identifier::parse("example:core/charge").unwrap(),
        PipelineBlend::Additive,
        WorldDepth::Tested,
    )
    .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
    .textured(PipelineTexture::Sprite(
        Identifier::parse("example:block/charge").unwrap(),
    )),
);
```

`PipelineTexture::Sprite(id)` names a sprite of the block or item atlas by its texture id:
`example:block/charge` is `assets/example/textures/block/charge.png` as the `minecraft:blocks`
atlas holds it, and `minecraft:item/diamond` names an item sprite. Animated sprites keep their
animation. A textured pipeline samples a rectangle of the atlas; it does not sample a standalone
texture file.

In the fragment shader, include `<sunrise:pipeline_sprite.glsl>` and call `sampleSprite(uv)`:

```glsl
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <sunrise:pipeline_sprite.glsl>

in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;

void main() {
    fragColor = sampleSprite(texCoord0) * vertexColor * ColorModulator;
}
```

`sampleSprite(uv)` maps the fraction of `uv` into the sprite's own rectangle (`uv` 0 to 1 spans
it once; beyond that, negative values too, it repeats) and loads one texel by integer coordinates
at mip level 0: nearest, unfiltered, so no texel of a neighbouring sprite can enter, even at the
far edge. It returns the texel in vanilla's gamma space with its alpha. `texCoord0` is still what
the quad's corners gave, so existing quad UVs work unchanged.

A texture is part of the pipeline, so it is checked when the pipeline is built, at startup and on
each reload. A refused texture is named by `PipelineTextureError` and refuses the pipeline as a
missing shader does: a required one fails the load, and an optional one is logged with the named
cause and resolves to `Invalid`:

| Cause | When |
| --- | --- |
| `NoSuchSprite` | Neither the block nor the item atlas has the sprite (`… refused by the GPU: its texture sprite <name>: NoSuchSprite: …`) |
| `NotASpriteKey` | The id names a file (a `textures/` path or a `.png` name), not a sprite key |

A pipeline without `.textured(…)` binds neither `Sampler0` nor `SpriteRect`: referencing either
(or including `<sunrise:pipeline_sprite.glsl>`) refuses the shader with a note naming
`RenderPipeline::textured`.

## Storage buffers

A pipeline reads large, sparsely updated data (compact geometry records, per-instance tables) from
a std430 storage buffer. Create it once with `host.create_storage_buffer(&StorageBufferDescriptor::new(size))`
(a multiple of 4 bytes, up to the device's `max_buffer_size`), update ranges with
`buffer.write_range(host, offset, bytes)` (queued and applied in call order before the next frame),
and free it with `buffer.release(host)`. It survives a resource reload and a world close, and is
freed when the mod unloads.

Declare it in the layout with `BindEntry::new("Tints", BindingKind::StorageBuffer { read_only: true, min_size: 16 })`
(`min_size` is the smallest range the block accepts, 16 for a `vec4[]` block; it must be non-zero and
at most `max_storage_buffer_binding_size`, as a uniform's is) and write the block in GLSL with the same access (`layout(std430) readonly buffer Tints { vec4 tint[]; };`;
a layout that says read-write needs a block without `readonly`). Bind a range of the buffer per draw
with `DrawBindings::new().storage_buffer("Tints", buffer.slice(offset, length))`. A range is at most
the device's `max_storage_buffer_binding_size` bytes and at least `min_size`
(`BufferError::BelowMinSize` otherwise), with its offset a multiple of
`min_storage_buffer_offset_alignment`; the buffer itself may be larger. A writable block in the
vertex stage needs `VERTEX_WRITABLE_STORAGE`; a device without it refuses such a pipeline. Every
refusal is a `BufferError` or `PipelineDrawError`, never a panic.

## Mip chains and storage images on render targets

A render target made with `host.create_render_target(&desc)` can carry a mip chain, the way a
vanilla `GpuTexture` has `getMipLevels()`: `RenderTargetDescriptor::new(size)
.color_target(RenderTargetFormat::R32Float).mip_levels(n)`, with `n` from 1 to
`RenderTargetDescriptor::max_mip_levels(size)` (`32 - max(width, height).leading_zeros()`). A depth
attachment keeps one level.
`TextureSource::RenderTargetColorMip(RenderTargetMipView::new(target, attachment, base_mip,
mip_levels))` samples a range of levels, as vanilla's `GpuTextureView` has `baseMipLevel()` and
`mipLevels()`. A render pass gives each level one usage, so a level a draw writes as a storage
image cannot also be sampled, or bound with another storage access, by any draw of that pass:
a draw that does so itself is refused with `PipelineDrawError::TargetReadWriteHazard`, and the
conflicting draws of one pass are skipped with one warning per target. Sample level `k` in a draw
of a different pass than the one that writes level `k + 1`, or sample only levels the pass does not
write.

For a shader that writes a level directly (a depth pyramid, a blur chain), create the target with
`.storage(StorageImageAccess::WriteOnly)`, declare the image in the layout with
`BindEntry::new("Next", BindingKind::StorageImage { access: StorageImageAccess::WriteOnly })`, write
`layout(r32f) writeonly uniform image2D Next;` in GLSL (`rgba8`, `rgba16f` and `r32f` are the
formats a target can have; the shader's `readonly`/`writeonly` qualifiers must agree with the
access), and bind one level with `DrawBindings::new().storage_image("Next",
StorageImageBinding::new(target, 0, level))`. A target can be used as a storage image only in the
access it was created for (`ReadWrite` covers both others). A storage image format needs the
adapter's `TextureFormatFeatures::allowed_usages` to include `STORAGE_BINDING`, and its `flags` to
include `STORAGE_WRITE_ONLY`, `STORAGE_READ_ONLY` or `STORAGE_READ_WRITE` for the access; a format
or access the adapter lacks is refused when the target is created, with
`RenderTargetError::UnsupportedStorage`. `host.render_capabilities()` lists the formats with
read-write storage in `read_write_storage`, and `float32_filterable` says whether an `R32Float`
target can be sampled with `SamplerFilter::Linear`. A device refusal of the allocation itself is
`RenderTargetError::Device`.

A render target survives a resource reload and a world close, and is freed by
`target.release(host)` or when its mod unloads.

## Samplers and mipmapped textures

A set-1 sampler entry takes one `Sampler` descriptor, the shape of vanilla's
`GpuDevice.createSampler(addressU, addressV, minFilter, magFilter, maxAnisotropy, maxLod)`:
`BindGroupLayout::builder().sampler("Atlas", Sampler::new(SamplerFilter::Nearest,
SamplerAddress::ClampToEdge))` is one filter and one address mode for both axes, with no `max_lod`.
For anything else set the public fields of the descriptor: `address_u`, `address_v`, `min`, `mag` and
`max_lod: Option<f32>`. Like vanilla's, a sampler has no separate mipmap filter and no minimum level of
detail: a `max_lod` above `0.25` blends linearly between the two nearest mip levels (vanilla's
`NEAREST_MIPMAP_LINEAR` and `LINEAR_MIPMAP_LINEAR`, and Vulkan's linear mipmap mode for `maxLod > 0.25`), and
`Some(0.0)` reads only the base level. `None` is what `Sampler::new` gives: vanilla's absent `maxLod` (1000),
so it blends between levels too. A sampler that blends is a filtering one: to sample a depth target, or an
`R32Float` target where `float32_filterable` is false, set `max_lod = Some(0.0)` with `SamplerFilter::Nearest`.
A negative or non-finite `max_lod` is `BindGroupLayoutError::BadMaxLod`. Anisotropy is not offered.

A dynamic texture (see [Textures made at run time](dynamic-textures.md)) made with `mip_levels` above 1 is
bound to such a sampler by its id, `TextureSource::DynamicTexture(id)`, and the shader's `texture`,
`textureLod` and `textureGrad` read every level:

```rust,ignore
let mut atlas = Sampler::new(SamplerFilter::Nearest, SamplerAddress::ClampToEdge);
atlas.max_lod = Some(4.0);
let layout = BindGroupLayout::builder().sampler("Atlas", atlas).build()?;
```

Fill the levels from the CPU with `update_dynamic_texture_region(id, mip_level, ...)`, as vanilla's
`MipmapGenerator` builds the block atlas's levels and uploads each one; the game builds none on the GPU.
`GuiCanvas::texture` always samples level 0 of a mipmapped texture.

## Participating in translated shader packs

When an Iris/OptiFine shader pack is active and owns terrain rendering, a mod pipeline without a
pack override draws its own GLSL program into the pack's world targets before the pack's
composite chain runs, so composite and `final` passes shade it alongside the world.

To let an active Iris/OptiFine shader pack shade a pipeline's geometry with its own program, declare
its `PipelineGeometryClass` on registration via `.geometry_class(...)`:

```rust,ignore
event.register(
    RenderPipeline::new(id, shader, PipelineBlend::Translucent, WorldDepth::Tested)
        .geometry_class(PipelineGeometryClass::Terrain),
);
```

Each geometry class selects a standard Iris/OptiFine program fallback chain (for example,
`Terrain` resolves `gbuffers_terrain -> gbuffers_textured_lit -> gbuffers_textured -> gbuffers_basic`,
and `Particles` resolves `gbuffers_particles -> gbuffers_textured_lit -> gbuffers_textured -> gbuffers_basic`).
Pipelines that need pack-defined programs, preprocessor defines, an auxiliary depth target, or
per-frame pack uniforms attach an optional `.pack_binding(PackBinding::new()...)`. Startup refuses
invalid GLSL identifiers, reserved Iris/OptiFine sampler/image/uniform names, and cross-mod or
namespace collisions across samplers, uniforms, defines, and depth-target IDs.

During a world draw hook, feed declared per-frame pack uniforms with
`ctx.set_pack_uniform(name, PackUniformValue::...)` (returns `false` without panicking when `name`
or its `PackUniformType` is not registered; any `previous_name` declared on the uniform is tracked
automatically across frames). To inspect whether an active shader pack bound a specific pipeline to
one of its translated programs (`PackOverrideState::Bound`), left it on the mod's own fallback
shader (`PackOverrideState::UnboundPack`), or no pack is active (`PackOverrideState::NoPack`), call
`ctx.pack_override_state(pipeline)`. To check generally whether a shader pack is currently loaded
and rendering the world (always `false` on the title screen, matching `IrisApi.isShaderPackInUse()`),
call `host.shader_pack_active()`.

`host.render_assets().shader_pack_block_id(state)` gives the active `shaders/` pack's `block.properties`
integer id (`mc_Entity`) for a block state, as Sunrise's shader-pack loader maps it: the id the
engine's own terrain carries for it, which a mod writes into its vertices so waving, emissive, water
or translucent blocks shade like terrain. It is `None` unless a `shaders/` pack is loaded and
rendering (the title screen and a pack still loading included), under a native pack, when the pack
gives the state no id and for a state the live block-state index lacks. The answer changes when the
active pack is loaded, switched or turned off, and no pack-reload signal exists yet:
`shader_pack_active()` stays `true` when one pack replaces another, and nothing else tells a mod. Ask
again when you need the id instead of caching it across frames. Item and entity ids
(`item.properties`, `entity.properties`) are not part of it.

### Your own vertex stage, and the main depth

`PackBinding::program_mode(PackProgramMode::ModVertexPackFragment)` keeps your own vertex shader,
with its `VertexFormat` and its set-1 bindings, and links it to the fragment stage of the pack's
program for one of the pipeline's stems, so your geometry is shaded into the pack's `colortex*`
buffers by the pack's own code. The stages link by GLSL varying name, as Iris links them: the
pack's translator picks each varying's location, so name your vertex outputs after the pack's
varyings (a struct member or `out` variable named `uv` feeds the pack's `varying vec2 uv`) and the
engine renumbers them to the pack's locations; the order you declare them in does not matter. It
works only when the pack ships such a program and every varying its fragment stage reads is
written by your vertex shader under the same name, type and interpolation. It is also refused when
the pack's program has a geometry stage, which needs the pack's own vertex stage. When any of this
fails, or the device cannot hold your bind groups after the pack's (the adapter's own
`max_bind_groups`), the pipeline is unbound: `pipeline_pack_bound` reports `false` and your own
shaders draw. Your bind groups go after the pack's, which are left as the pack declared them; a
vertex shader that uses push constants is left unbound.

For example, to write world depth from `AfterTranslucentGbuffers`, use `.depth_write(true)` and
`PackBinding::depth_target(PackDepthTarget::Main)`. See `PackDepthTarget` in the
[mod API rustdoc](../../reference/api.md) for attachment selection and write rules.

A stencil test inside a pack pass is not part of this: the pack's main depth has no stencil plane.

### Shadow pass, clouds and pack changes

`host.shader_pack_state()` returns `None` when no pack is active (the same rule as
`shader_pack_active()`) and otherwise a `ShaderPackState`, taken at the start of each frame's render
and read unchanged by that frame's render callbacks:

- `hides_clouds`: the pack sets `clouds=off` or draws its own, so a custom sky or cloud renderer
  should not draw clouds over it (Iris's `DHCompatInternal.avoidRenderingClouds`).
- `shadow`: `None` when the pack has no shadow pass, otherwise the map's `resolution`, the shadow
  `distance` (half the covered side, in blocks) and the column-major `model_view` and
  `projection`. They are in the same camera-relative space as `WorldRenderView` and are the current
  frame's values. Use them to cull shadow-casting geometry you submit to `PackPassSlot::Shadow`
  against the shadow volume (Iris's `ShadowRenderer` `MODELVIEW` and `PROJECTION`).
- `generation`: changes every time a pack is loaded, switched or turned off, and at no other time.
  Compare it each frame and re-read whatever you derived from the pack when it differs. It covers a
  pack switch that fires no resource reload.

`host.render_assets().pipeline_pack_bound(pipeline)` says whether the active pack compiled a program
for the pipeline's `PackBinding`. It is `false` with no pack, for a stale or invalid handle, and when
the pipeline has a `PackBinding` but the pack has no matching program. In that last case the engine
draws the pipeline through the mod's own shaders, exactly as `PackOverrideState::UnboundPack` reports
it, so the mod's fallback shader must be able to draw it. Like the state above, it reads the frame's
snapshot during render callbacks.

## Fixed-function state (`PipelineState`)

Every `RenderPipeline` carries a `PipelineState` with vanilla's core rasterization, depth, blend,
and colour-write settings. `RenderPipeline::new` sets `blend` and `depth` and defaults the rest to
`depth_write: false`, `cull: false` (two-sided), `color_write: ColorWrite::ALL`, and
`polygon: PolygonMode::Fill`. Override any field with builder methods:

```rust,ignore
event.register(
    RenderPipeline::new(id, shader, PipelineBlend::Opaque, WorldDepth::Tested)
        .depth_write(true)
        .cull(true)
        .color_write(ColorWrite::COLOR)
        .polygon(PolygonMode::Fill),
);
```

- `PipelineBlend::Opaque` writes solid colour directly without alpha blending (letting a mod draw
  solid geometry with `depth_write(true)`).
- `PolygonMode::Line` requires the device's wireframe feature (`POLYGON_MODE_LINE`); if the GPU
  lacks it, the pipeline (or modifier variant) is refused.
- Depth is a `DepthState` (`compare`, `write`, `depth_bias_constant`, `depth_bias_slope_scale`) in
  `PipelineState::depth_stencil`. `.depth(WorldDepth)` is the coarse form (`Tested` is
  `DepthCompare::GreaterEqual` over Sunrise's reversed depth, `ThroughWalls` is `Always`);
  `.depth_compare`, `.depth_bias(constant, slope_scale)` and `.depth_stencil(Option<DepthState>)`
  set it exactly. `.depth_stencil(None)` declares a depthless pipeline.
- `PipelineBlend::Custom(BlendFunction)` takes separate colour and alpha equations (factors and
  `BlendOp`), with vanilla's presets as `BlendFunction` constants. `ConstantColor`,
  `ConstantAlpha` and their one-minus forms are refused when the pipeline is registered, because
  Sunrise sets no blend constant and a shader pack's blend table has no such factor.
- `.color_target(index, Some(ColorTargetState))` configures one of up to 8 colour targets, and
  `None` leaves it unused. Every blended target must use the same blend function.
- `.topology(PrimitiveTopology)` picks how vertices form primitives. `pipeline_quads` only draws
  `PrimitiveTopology::Quads` and refuses any other pipeline with
  `PipelineDrawError::TopologyMismatch`.

## Your own vertex layout, retained meshes and `draw_mesh` / `draw_vertices`

A pipeline reads its own vertex layout when it declares `VertexFormat` bindings. Each
`VertexFormat::builder(VertexStepMode::Vertex)` (or `Instance`) takes named attributes, and
`.build()` returns the format, or a `VertexFormatError` for an invalid or duplicate name, an offset that
breaks its alignment, or a step rate the GPU cannot do. Attribute names are the shader's `in`
names. `RenderPipeline::vertex_binding(binding, format)` attaches one format per binding index,
up to `MAX_VERTEX_BINDINGS` bindings and `MAX_VERTEX_ELEMENTS` attributes in total. A stride is
a multiple of `VERTEX_ALIGNMENT` (4 bytes).

```rust,ignore
let per_vertex = VertexFormat::builder(VertexStepMode::Vertex)
    .attribute("LocalPos", VertexAttributeFormat::Float32x2)
    .build()?;
let per_instance = VertexFormat::builder(VertexStepMode::Instance)
    .attribute("InstOffset", VertexAttributeFormat::Float32x2)
    .attribute("InstColor", VertexAttributeFormat::Unorm8x4)
    .build()?;
event.register(
    RenderPipeline::new(id, shader, PipelineBlend::Opaque, WorldDepth::ThroughWalls)
        .vertex_binding(0, per_vertex)
        .vertex_binding(1, per_instance),
);
```

Two ways feed those bindings during a world stage (`WorldDrawContext`), both validated before
anything is queued: a refused call returns a `PipelineDrawError` and queues nothing.

- **Retained mesh.** `host.create_mesh(&MeshDescriptor::new(&buffers))` uploads one buffer per
  binding (`MeshBufferInit::from_bytes(stride, bytes)`), plus an optional index buffer
  (`.with_indices(MeshIndexInit::from_bytes(IndexFormat::Uint16, bytes))`), and returns a
  `MeshRef` owned by the calling mod. Update it with `MeshRef::write_vertices` and
  `write_indices` (4-byte aligned, in bounds) or `MeshRef::replace`. Draw it each frame with
  `draw.draw_mesh(pipeline, mesh, MeshDraw::vertices(0..n), &DrawBindings::EMPTY)`; nothing is
  copied per frame. Free the GPU buffers with `MeshRef::release(host)`; the handle is stale
  (`MeshError::Stale`) afterwards. A mesh is also retired when its mod is disabled, the world
  closes, or resources reload.
- **Transient bytes.** `draw.draw_vertices(pipeline, &[vertex_bytes, instance_bytes],
  Some((IndexFormat::Uint16, index_bytes)), draw, &DrawBindings::EMPTY)` uploads this frame's
  slices and draws them. There is one slice per binding, in binding order.

`MeshDraw::vertices(range)` draws a range of vertices, `MeshDraw::indexed(range, base_vertex)`
draws a range of an index buffer, and `.with_instances(range)` sets the instance range. The
ranges are checked against what the buffers hold: a vertex-step buffer must contain every record
an indexed draw reaches (after `base_vertex`), an instance-step buffer must contain the instance
range, and `Quads`, `Lines` and `TriangleFan` draws, which the engine feeds from generated
indices, must fit the device's largest buffer.

## Pipeline modifiers (`register_pipeline_modifiers`)

Register reusable data-only state overrides in `Hooks::register_pipeline_modifiers` (NeoForge's
`RegisterPipelineModifiersEvent`):

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

Resolve the modifier once in `client_setup` (or `client_resources_reloaded`) with
`host.render_assets().pipeline_modifier(&id)` and wrap world draws in
`WorldDrawContext::with_pipeline_modifier(modifier, &mut |draw| { ... })`. Scopes nest: the most
recently pushed modifier applies first, then older ones, so the outermost modifier wins on any
field both set (`PipelineModifierStack.apply`). Modifiers apply to `lines`, `quads`, `pipeline_quads`, and `draw_model_instances` (not
`name_tag` labels), but not all of a modifier applies everywhere. The built-in `lines`, `quads`
and `draw_model_instances` have no pipeline of their own to vary, so only `depth` applies to them
(it picks the through-walls or tested copy). The full `PipelineModifier` (`blend`, `cull`,
`color_write`, `depth_write`, `polygon`, `depth`) applies to pipelines a mod registers and draws
with `pipeline_quads`. A field a modifier leaves unset keeps the pipeline's own value, which
defaults to what a mod pipeline already had before `PipelineState` existed. Modified GPU pipeline
variants are compiled lazily and cached per `(pipeline, modifier sequence)` until the next reload;
a variant the GPU refuses is logged once with both IDs and skipped while the rest of the frame
draws.

## Order-independent transparency (`OitTranslucent`)

To draw a translucent or additive pipeline in vanilla 26.3's improved-transparency passes, opt in
with `.order_independent()` and include `<minecraft:oit.glsl>` in the fragment shader:

```rust,ignore
event.register(
    RenderPipeline::new(id, shader, PipelineBlend::Translucent, WorldDepth::Tested)
        .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
        .order_independent(),
);
```

```glsl
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:oit.glsl>

in vec2 texCoord0;
in vec4 vertexColor;
#ifndef OIT_ALPHA_ONLY
out vec4 fragColor;
#endif

void main() {
    vec4 color = vertexColor * ColorModulator;
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, color.a);
#else
#ifdef OIT_ACCUMULATE
    color = sampleColorForAccumulation(color);
#endif
    fragColor = color;
#endif
}
```

At resource load the engine translates three OIT variants (`OIT_ALPHA_ONLY` + `OIT_DEPTH_BOUNDS`,
`OIT_ALPHA_ONLY` + `OIT_TRANSMITTANCE`, and `OIT_ACCUMULATE`; plus `OIT_ADDITIVE` for
`PipelineBlend::Additive`) and binds `DepthBoundsSampler`, `Coeff0`, and `Coeff1`. Prepare quads
for `RenderLevelStage::OitTranslucent(stage)` via `PrepareRenderBuffersContext::pipeline_quads`
and draw the returned `PreparedWorldDraw` in `render_level_stage`. Submitting an `Opaque` pipeline
or one without `.order_independent()` to `OitTranslucent` returns
`PipelineDrawError::Pipeline(RenderPipelineError::Invalid)` and logs once per mod and pipeline.
`cargo sunrise-mod check` validates all three OIT variants using prepared resources or Sunrise's
interface-only `oit.glsl` stub.

## Compute pipelines

A compute shader runs a grid of workgroups with no render pass: it reads and writes storage buffers,
storage images and texel buffers, which suits culling, reductions and mip building (for example a
depth-pyramid reduction that writes a mip chain and a visibility count). It is registered on the
same `register_render_pipelines` hook as a draw pipeline.

1. Write `assets/<namespace>/shaders/compute/<path>.csh` in GLSL 450. Declare
   `layout(local_size_x = 8, local_size_y = 8) in;` and bind every resource by the name its
   `BindGroupLayout` entry gives it. A compute shader has no `in` or `out` variables, no push
   constants and one bind group, so a binding is the entry you declared and nothing else.
2. Register it:

   ```rust,ignore
   event.compute_pipeline(
       ComputePipeline::builder(id("mymod:reduce"), id("mymod:reduce"))
           .bind_group_layout(layout)
           .define_value("TILE", "8")
           .build(),
   );
   ```

3. Resolve `host.render_assets().compute_pipeline(&id)` in `client_setup` and again in
   `client_resources_reloaded`; a handle from before a reload is `Stale`.
4. In `frame_graph_setup`, add a compute node and dispatch from its callback:

   ```rust,ignore
   graph.add_compute_pass(
       ComputePassDescriptor::new("reduce")
           .reads(FrameTarget::Mod(depth))
           .writes(FrameTarget::Mod(pyramid))
           .after_stage(RenderLevelStage::AfterLevel),
       &mut |ctx| {
           let _ = ctx.dispatch(pipeline, &bindings, [8, 8, 1]);
       },
   )?;
   ```

   The node runs outside any render pass, in the order its reads, writes and `after`/`before`
   give it, and is never culled. `dispatch_indirect` reads the three counts from a storage buffer
   created with `indirect: true`.

Everything a mod can get wrong comes back as an `Err`: a workgroup size, workgroup storage or
binding count over the device's limits makes the pipeline resolve `Invalid`, a shader that needs
subgroup operations resolves `UnsupportedFeature` when `RenderCapabilities::subgroup` is false,
and a dispatch over `max_compute_workgroups_per_dimension` is `TooManyWorkgroups`. Your GPU
objects survive a resource reload and a world close; free them with `release`.

## Indirect draws

A compute shader that culls can write what to draw, and the draw reads it with no CPU readback.
Create a storage buffer with `StorageBufferDescriptor::new(size).indirect(true)`, fill it from a
compute pass, and draw it from a world stage, an offscreen pass or a translated shader pack's slot:

```rust,ignore
let bindings = DrawBindings::new().storage_buffer("Pages", pages.slice(0, pages_len));
ctx.draw_indirect(
    &IndirectDraw::new(pipeline, commands, max_count, &bindings).with_mesh(mesh),
)?;
```

`draw_indirect` is on `WorldDrawContext`, `PrepareRenderBuffersContext` (with the stage, as
`draw_mesh` is) and `OffscreenDrawContext`. It draws exactly `max_count` commands from
`indirect_offset`, the same on every backend, as vanilla's `RenderPass.drawIndexedIndirect(commands,
drawCount)` does. Have the culling shader write `instance_count = 0` into each slot it drops, which
draws nothing. A `max_count` of zero is a legal no-op.

Where `render_capabilities().multi_draw_indirect_count` is set, the culling pass can instead
compact the commands it keeps and write how many there are as a `u32` into a buffer created with
`StorageBufferDescriptor::indirect`. `IndirectDraw::with_count_buffer(buffer, offset)` then draws
that many, at most `max_count` (`multi_draw_indirect_count`), so dropped slots cost nothing. The
offset is a multiple of 4 with the count's 4 bytes inside the buffer, or the draw is refused with
`PipelineDrawError::InvalidIndirect`. Where the capability is false (no
`MULTI_DRAW_INDIRECT_COUNT`, or DirectX 12, whose count draws drop an indexed command's
`base_vertex`), a draw with a count buffer is refused with
`PipelineDrawError::UnsupportedIndirectCount`. A draw with a count buffer in a translated shader
pack's slot, where the pack's program is linked to the pipeline, counts the same way.

- **Layout.** With a `MeshRef` that has an index buffer, each command is five 4-byte words
  (`index_count, instance_count, first_index, base_vertex, first_instance`; `INDIRECT_INDEXED_DRAW_SIZE`
  bytes) and the index format is the mesh's own. Without a mesh, or with a mesh that has no indices,
  each command is four words (`vertex_count, instance_count, first_vertex, first_instance`;
  `INDIRECT_DRAW_SIZE` bytes), and the pipeline's topology cannot be `Quads`, `Lines` or `TriangleFan`.
  A pipeline that pulls its vertices from a storage buffer passes no mesh and declares no vertex
  bindings.
- **First instance.** A command's `first_instance` is honoured only where
  `RenderCapabilities::indirect_first_instance` is true; keep it zero otherwise.
- **Base vertex.** An indexed command's `base_vertex` offsets the vertex fetch and `gl_VertexID`;
  the engine's daily GPU test draws a command with a non-zero `base_vertex` on the CI adapter.

The command buffer must have been created with `indirect: true`, its offset be a multiple of 4, and
`indirect_offset + max_count * stride` lie inside it. A stale or foreign buffer is
`PipelineDrawError::Buffer`, any other misuse `PipelineDrawError::InvalidIndirect`, and nothing is
recorded.

## Reading a storage buffer back

A mod that needs a compute pass's result on the CPU (a request queue, visibility counts) reads it
back without waiting, as vanilla does: `CommandEncoder.copyToBuffer(source, target)` into a
`USAGE_MAP_READ` buffer, then `GpuBuffer.map(read)` once its fence has signalled. Create the buffer
with `StorageBufferDescriptor::new(size).copy_src()` (vanilla's `USAGE_COPY_SRC`; off by default),
request the copy inside the compute node that writes it, and poll over the following frames:

```rust,ignore
// in the `record` callback of `add_compute_pass`
ctx.dispatch(pipeline, &bindings, [groups, 1, 1])?;
let ticket = counts.request_readback(host, 0, 16)?;
// kept by the mod; each later frame:
if let Some(bytes) = ticket.poll_readback(host)? {
    // the 16 bytes the dispatch above wrote
}
```

- **Where the copy happens.** `request_readback` records the copy at the call's place in the node:
  it sees the writes of the dispatches recorded before it and none recorded after. It is allowed
  only inside a frame-graph compute node's `record` callback; anywhere else it is
  `BufferError::NotInFrame`.
- **No waiting.** `poll_readback` is `Ok(None)` while the copy is in flight, then `Ok(Some(bytes))`
  once, and the staging buffer is freed with it. The map starts after the frame is submitted.
- **Bounds.** `offset` and `len` are multiples of 4, `len` is not zero, the range lies inside the
  buffer and the buffer has `copy_src` (`BufferError::{InvalidAlignment, Empty, OutOfRange,
  NotReadable}`), all refused before the GPU is touched. There is no cap on tickets beyond the
  device's memory: running out is `BufferError::ExceedsDeviceLimit`.
- **Letting go.** `ticket.release(host)` drops a ticket you no longer want; unpolled tickets are freed
  when the mod unloads. A spent or unknown ticket, a node the frame did not run, and a lost device
  are `BufferError::Unavailable`.
