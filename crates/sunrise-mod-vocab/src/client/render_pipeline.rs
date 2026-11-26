//! Render pipelines a client mod registers as plain data, and the quads it draws with them.
use crate::{Identifier, StorageBufferSlice, WorldDepth};

/// A pipeline registered by `Hooks::register_render_pipelines`: a GLSL core shader program
/// (`assets/<ns>/shaders/core/<path>.vsh` and `.fsh`, optionally reusing another program's
/// vertex stage), its preprocessor defines, optional sprite texture, and fixed blend/depth
/// state. The engine translates the GLSL via `sunrise-glsl` and builds every GPU object.
///
/// ## Colour space
/// Mod pipeline GLSL programs work in vanilla's gamma space, matching vanilla core shaders
/// (`texture * vertexColor * ColorModulator`). On frames storing vanilla's encoding the output is
/// stored as written; on frames holding linear light the engine decodes the fragment shader's
/// location-0 output after `main()` (sanitizing negative/NaN RGB to 0 and clamping alpha
/// coverage to `[0, 1]`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline registration descriptor.
pub struct RenderPipeline {
    owner: String,
    /// The pipeline's own name (`mymod:glow`), unique among mod pipelines.
    pub id: Identifier,
    /// The core shader program (`mymod:core/glow` resolves to
    /// `assets/mymod/shaders/core/glow.vsh` and `assets/mymod/shaders/core/glow.fsh`, or
    /// `mymod:glow` resolves to `assets/mymod/shaders/glow.vsh` and `.fsh`), as the mod's assets
    /// or a resource pack supply it. When [`Self::vertex_shader`] is set, this id supplies only
    /// the `.fsh` fragment stage.
    pub fragment: Identifier,
    /// Optional separate vertex shader program id (`withVertexShader`): defaults to
    /// [`Self::fragment`] when `None`. Lets a mod reuse another program's vertex stage such as
    /// `minecraft:core/position_tex_color` or a library mod's vertex shader.
    pub vertex_shader: Option<Identifier>,
    /// Preprocessor defines compiled into both stages (`withShaderDefine`): `(name, value)`.
    /// Each `name` must be a valid non-`GL_` ASCII GLSL identifier, and each `value` (when
    /// present) a non-empty single-line token without comments, backslashes, or `#`.
    pub defines: Vec<(String, Option<String>)>,
    /// Custom per-binding vertex formats (`withVertexBinding(bindingIndex, format)`). Empty by
    /// default, meaning binding 0 uses [`VertexFormat::position_tex_color`] (stride 24:
    /// `Position: Float32x3`, `UV0: Float32x2`, `Color: Unorm8x4`).
    pub vertex_bindings: Vec<(u8, VertexFormat)>,
    /// Fixed-function blend, depth, depth-write, depth-bias, culling, colour-write, polygon-mode,
    /// and primitive-topology state.
    pub state: PipelineState,
    /// Per-target colour blend and write-mask overrides for multi-target (MRT) passes, indexed
    /// `0..MAX_COLOR_TARGETS` (`ColorTargetState.MAX_COLOR_TARGETS = 8`). Empty by default,
    /// meaning a single target 0 derived from [`PipelineState::blend`] and
    /// [`PipelineState::color_write`]. Configured via [`Self::color_target`].
    pub color_targets: Vec<Option<ColorTargetState>>,
    /// First out-of-range index passed to [`Self::color_target`], kept so registration refuses
    /// `index >= MAX_COLOR_TARGETS` without panicking or over-allocating.
    invalid_color_target_index: Option<usize>,
    /// Whether this pipeline opts in to order-independent transparency
    /// (`RenderLevelStage::OitTranslucent`, NeoForge's `registerOitPipelineSet`). When `true`
    /// and `state.blend != PipelineBlend::Opaque`, the engine translates three OIT variants at
    /// load (`OIT_DEPTH_BOUNDS`, `OIT_TRANSMITTANCE`, `OIT_ACCUMULATE`, plus `OIT_ADDITIVE` for
    /// [`PipelineBlend::Additive`]).
    pub order_independent: bool,
    /// Whether a resource load may go on without this pipeline (NeoForge's
    /// `registerOptionalPipeline`). A required pipeline (NeoForge's `registerPipeline`, the
    /// default) that a load cannot build fails the load: the game does not start, and a reload
    /// fails and keeps the previous resources. An optional one is logged once by id and resolves
    /// to [`RenderPipelineError::Invalid`] while the other pipelines draw.
    pub optional: bool,
    /// What the shader samples via `Sampler0` / `<sunrise:pipeline_sprite.glsl>`'s
    /// `sampleSprite(uv)`, if anything ([`Self::textured`]). `None`, the default, binds no
    /// texture: referencing `Sampler0` or `SpriteRect` refuses the pipeline.
    pub texture: Option<PipelineTexture>,
    /// What kind of world geometry this pipeline draws ([`Self::geometry_class`]), so an active
    /// Iris/OptiFine shader pack can route it through the matching standard `gbuffers_*`
    /// fallback chain. `None`, the default, draws only with the pipeline's own shader.
    pub geometry_class: Option<PipelineGeometryClass>,
    /// Optional shader-pack integration extras ([`Self::pack_binding`]): prepended pack program
    /// names, pass slot override, auxiliary depth target, per-frame uniforms, preprocessor
    /// defines, and GLSL compatibility bindings. All names are supplied by the mod; the engine
    /// hardcodes none.
    pub pack_binding: Option<PackBinding>,
    /// Mod-declared set-1 [`BindGroupLayout`] (uniform buffers, combined image samplers, and
    /// texel buffers). Set 0 remains vanilla's shared layouts (`BindGroupLayouts.java`).
    pub bind_group_layout: BindGroupLayout,
    /// Optional per-draw push-constant size in bytes (`RenderPipeline.withPushConstantSize`,
    /// at most [`MAX_PUSH_CONSTANT_SIZE`] = 128 B, multiple of 4). Backed portably by an inline
    /// std140 uniform slot at the end of set 1; GLSL `layout(push_constant)` blocks translate
    /// onto that slot.
    pub push_constant_size: u32,
}
impl RenderPipeline {
    /// Describe a required pipeline that draws [`VertexFormat::position_tex_color`]
    /// geometry with the GLSL core program `shader` (`ns:core/path` ->
    /// `assets/<ns>/shaders/core/<path>.vsh` and `.fsh`).
    pub fn new(
        id: Identifier,
        shader: Identifier,
        blend: PipelineBlend,
        depth: WorldDepth,
    ) -> Self {
        Self {
            owner: String::new(),
            id,
            fragment: shader,
            vertex_shader: None,
            defines: Vec::new(),
            vertex_bindings: Vec::new(),
            state: PipelineState::new(blend, depth),
            color_targets: Vec::new(),
            invalid_color_target_index: None,
            order_independent: false,
            optional: false,
            texture: None,
            geometry_class: None,
            pack_binding: None,
            bind_group_layout: BindGroupLayout::default(),
            push_constant_size: 0,
        }
    }
    /// The owning mod, assigned by the host after registration.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Use a separate GLSL vertex shader program (vanilla's `withVertexShader`), such as
    /// `minecraft:core/position_tex_color` or another mod's vertex shader. Defaults to
    /// [`Self::fragment`] when omitted.
    pub fn vertex_shader(mut self, id: Identifier) -> Self {
        self.vertex_shader = Some(id);
        self
    }

    /// Define a preprocessor flag `#define NAME` for both stages of this pipeline
    /// (vanilla's `withShaderDefine(key)`). Validated at registration startup.
    pub fn define(mut self, name: impl Into<String>) -> Self {
        self.defines.push((name.into(), None));
        self
    }

    /// Define a preprocessor macro `#define NAME VALUE` for both stages of this pipeline
    /// (vanilla's `withShaderDefine(key, value)`). Validated at registration startup.
    pub fn define_value(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.defines.push((name.into(), Some(value.into())));
        self
    }

    /// Validate GLSL define names and values declared on this pipeline. Called by the engine
    /// during pipeline registration freeze.
    pub fn validate_defines(&self) -> Result<(), String> {
        for (name, value) in &self.defines {
            if !is_valid_glsl_identifier(name) || name.starts_with("GL_") {
                return Err(format!(
                    "invalid shader define name {name:?}: expected a valid non-GL_ GLSL identifier"
                ));
            }
            if let Some(val) = value
                && !is_valid_glsl_define_value(val)
            {
                return Err(format!(
                    "invalid shader define value for {name}: {val:?} must be a non-empty single-line token without comments, backslashes, or '#'"
                ));
            }
        }
        Ok(())
    }

    /// Validate fixed-function pipeline state declared on this pipeline: finite depth-bias
    /// slope scale, colour-target indices `< MAX_COLOR_TARGETS` (8), and identical blend
    /// functions across all active blended colour targets.
    /// Called by the engine during pipeline registration freeze.
    pub fn validate_state(&self) -> Result<(), String> {
        if let Some(ds) = self.state.depth_stencil
            && !ds.depth_bias_slope_scale.is_finite()
        {
            return Err(format!(
                "invalid depth_bias_slope_scale {:?}: must be finite",
                ds.depth_bias_slope_scale
            ));
        }
        if let Some(index) = self.invalid_color_target_index {
            return Err(format!(
                "color target index {index} exceeds MAX_COLOR_TARGETS ({MAX_COLOR_TARGETS})"
            ));
        }
        if self.color_targets.len() > MAX_COLOR_TARGETS {
            return Err(format!(
                "color target count {} exceeds MAX_COLOR_TARGETS ({MAX_COLOR_TARGETS})",
                self.color_targets.len()
            ));
        }
        let mut last_blend: Option<BlendFunction> = None;
        let blends = std::iter::once(self.state.blend).chain(
            self.color_targets
                .iter()
                .flatten()
                .map(|target| target.blend),
        );
        for blend_fn in blends.filter_map(PipelineBlend::to_blend_function) {
            if let Some(factor) = blend_fn.constant_factor() {
                return Err(format!("blend factor {factor:?} is not supported"));
            }
        }
        for target in self.color_targets.iter().flatten() {
            if let Some(blend_fn) = target.blend.to_blend_function() {
                match last_blend {
                    None => last_blend = Some(blend_fn),
                    Some(prev) if prev != blend_fn => {
                        return Err(
                            "blend functions must be the same for all active color targets (a pipeline's blend state is one function across its active color targets)"
                                .to_owned(),
                        );
                    }
                    Some(_) => {}
                }
            }
        }
        Ok(())
    }

    /// Declare a vertex buffer binding at `binding` (`0..MAX_VERTEX_BINDINGS`, vanilla's
    /// `RenderPipeline.Builder.withVertexBinding(int bindingIndex, VertexFormat)`).
    ///
    /// When no binding is declared, the pipeline uses the default binding 0
    /// ([`VertexFormat::position_tex_color`]). Registration refuses `binding >= 16`, duplicate
    /// binding indices, invalid formats, duplicate attribute names across bindings, or more than
    /// `MAX_VERTEX_ELEMENTS` (16) total attributes across all bindings.
    pub fn vertex_binding(mut self, binding: u8, format: VertexFormat) -> Self {
        self.vertex_bindings.push((binding, format));
        self
    }

    /// The effective vertex buffer bindings sorted by binding index: [`Self::vertex_bindings`]
    /// when non-empty, otherwise `[(0, VertexFormat::position_tex_color())]`.
    pub fn effective_vertex_bindings(&self) -> Vec<(u8, VertexFormat)> {
        if self.vertex_bindings.is_empty() {
            vec![(0, VertexFormat::position_tex_color())]
        } else {
            let mut bindings = self.vertex_bindings.clone();
            bindings.sort_by_key(|(idx, _)| *idx);
            bindings
        }
    }

    /// Whether this pipeline uses the default single vertex binding,
    /// [`VertexFormat::position_tex_color`] (compatible with the
    /// [`crate::WorldDrawContext::pipeline_quads`] shorthand).
    pub fn uses_default_vertex_format(&self) -> bool {
        self.vertex_bindings.is_empty()
            || (self.vertex_bindings.len() == 1
                && self.vertex_bindings[0].0 == 0
                && self.vertex_bindings[0].1 == VertexFormat::position_tex_color())
    }

    /// Validate all declared vertex buffer bindings on this pipeline: binding indices `< 16`,
    /// unique binding indices, valid per-binding [`VertexFormat`], unique GLSL attribute names
    /// across all bindings, and at most `MAX_VERTEX_ELEMENTS` (16) total attributes
    ///.
    pub fn validate_vertex_bindings(&self) -> Result<(), String> {
        let mut seen_bindings = std::collections::HashSet::new();
        let mut seen_attrs = std::collections::HashSet::new();
        let mut total_attrs = 0usize;
        for (binding, format) in &self.vertex_bindings {
            if usize::from(*binding) >= MAX_VERTEX_BINDINGS {
                return Err(format!(
                    "vertex binding index {binding} exceeds MAX_VERTEX_BINDINGS ({MAX_VERTEX_BINDINGS})"
                ));
            }
            if !seen_bindings.insert(*binding) {
                return Err(format!("duplicate vertex binding index {binding}"));
            }
            format
                .validate()
                .map_err(|e| format!("vertex binding {binding}: {e}"))?;
            total_attrs += format.attributes.len();
            if total_attrs > MAX_VERTEX_ELEMENTS {
                return Err(format!(
                    "total vertex attributes ({total_attrs}) exceeds MAX_VERTEX_ELEMENTS ({MAX_VERTEX_ELEMENTS})"
                ));
            }
            for attr in &format.attributes {
                if !seen_attrs.insert(attr.name.as_str()) {
                    return Err(format!(
                        "duplicate vertex attribute name {:?} across vertex bindings",
                        attr.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// Replace all fixed-function state fields at once ([`PipelineState`]).
    pub fn state(mut self, state: PipelineState) -> Self {
        self.state = state;
        self
    }

    /// Set how fragments join the colour target ([`PipelineState::blend`]).
    pub fn blend(mut self, blend: PipelineBlend) -> Self {
        self.state.blend = blend;
        if let Some(Some(slot0)) = self.color_targets.first_mut() {
            slot0.blend = blend;
        }
        self
    }

    /// Set depth testing behaviour from a coarse [`WorldDepth`] (`Tested` ->
    /// [`DepthCompare::GreaterEqual`], `ThroughWalls` -> [`DepthCompare::Always`]).
    pub fn depth(mut self, depth: WorldDepth) -> Self {
        let compare = DepthCompare::from_world_depth(depth);
        match &mut self.state.depth_stencil {
            Some(ds) => ds.compare = compare,
            None => self.state.depth_stencil = Some(DepthState::new(compare, false)),
        }
        self
    }

    /// Set the depth comparison function (`DepthStencilState.depthTest`).
    pub fn depth_compare(mut self, compare: DepthCompare) -> Self {
        match &mut self.state.depth_stencil {
            Some(ds) => ds.compare = compare,
            None => self.state.depth_stencil = Some(DepthState::new(compare, false)),
        }
        self
    }

    /// Set whether fragments that pass the comparison write to the depth buffer
    /// (`DepthStencilState.writeDepth`). Default: `false`. See [`PackDepthTarget`] for which
    /// attachments a linked shader-pack pass can write.
    pub fn depth_write(mut self, write: bool) -> Self {
        match &mut self.state.depth_stencil {
            Some(ds) => ds.write = write,
            None => {
                self.state.depth_stencil = Some(DepthState::new(DepthCompare::GreaterEqual, write))
            }
        }
        self
    }

    /// Set depth bias (`DepthStencilState.depthBiasConstant` and `depthBiasScaleFactor`).
    /// Positive values lift fragments toward the near plane in Sunrise's reversed-Z world pass
    /// (and are negated automatically on standard-Z shader-pack targets). Non-finite
    /// `slope_scale` is refused at registration.
    pub fn depth_bias(mut self, constant: i32, slope_scale: f32) -> Self {
        let ds = self
            .state
            .depth_stencil
            .get_or_insert_with(|| DepthState::new(DepthCompare::GreaterEqual, false));
        ds.depth_bias_constant = constant;
        ds.depth_bias_slope_scale = slope_scale;
        self
    }

    /// Replace the full depth-stencil state (`withDepthStencilState`). Passing `None` declares a
    /// depthless pipeline (`RenderPipeline.wantsDepthTexture() == false`) for offscreen, post, or
    /// GUI passes with no depth attachment.
    pub fn depth_stencil(mut self, depth_stencil: Option<DepthState>) -> Self {
        self.state.depth_stencil = depth_stencil;
        self
    }

    /// Set whether back faces are culled (`withCull`). Default: `false` (two-sided).
    pub fn cull(mut self, cull: bool) -> Self {
        self.state.cull = cull;
        self
    }

    /// Set which colour channels fragments write (`ColorTargetState` write mask). Default:
    /// [`ColorWrite::ALL`].
    pub fn color_write(mut self, write: ColorWrite) -> Self {
        self.state.color_write = write;
        if let Some(Some(slot0)) = self.color_targets.first_mut() {
            slot0.write = write;
        }
        self
    }

    /// Configure colour target `index` (`0..MAX_COLOR_TARGETS`, vanilla's
    /// `withColorTargetState(index, state)` / `withUnusedColorTargetState(index)`). Passing
    /// `index >= MAX_COLOR_TARGETS` (8) or differing blend functions across active targets is
    /// refused at registration.
    pub fn color_target(mut self, index: usize, state: Option<ColorTargetState>) -> Self {
        if index >= MAX_COLOR_TARGETS {
            if self.invalid_color_target_index.is_none() {
                self.invalid_color_target_index = Some(index);
            }
            return self;
        }
        if self.color_targets.len() <= index {
            self.color_targets.resize(index + 1, None);
        }
        self.color_targets[index] = state;
        if index == 0
            && let Some(ct) = state
        {
            self.state.blend = ct.blend;
            self.state.color_write = ct.write;
        }
        self
    }

    /// The effective colour target slots: [`Self::color_targets`] when explicitly configured,
    /// otherwise a single slot 0 built from [`PipelineState::blend`] and
    /// [`PipelineState::color_write`].
    pub fn effective_color_targets(&self) -> Vec<Option<ColorTargetState>> {
        if self.color_targets.is_empty() {
            vec![Some(ColorTargetState::new(
                self.state.blend,
                self.state.color_write,
            ))]
        } else {
            self.color_targets.clone()
        }
    }

    /// Set triangle fill or wireframe mode (`PolygonMode`). Default: [`PolygonMode::Fill`].
    /// [`PolygonMode::Line`] requires the device's wireframe feature.
    pub fn polygon(mut self, polygon: PolygonMode) -> Self {
        self.state.polygon = polygon;
        self
    }

    /// Set how vertices are assembled into primitives (`withPrimitiveTopology`). Default:
    /// [`PrimitiveTopology::Quads`].
    pub fn topology(mut self, topology: PrimitiveTopology) -> Self {
        self.state.topology = topology;
        self
    }

    /// Opt this pipeline in to order-independent transparency (`RenderLevelStage::OitTranslucent`,
    /// NeoForge's `registerOitPipelineSet`). The fragment shader must include
    /// `<minecraft:oit.glsl>`, declare no colour output under `OIT_ALPHA_ONLY` while calling
    /// `executeAlphaOnlyPhase(gl_FragCoord.z, alpha)`, and pass its output colour through
    /// `sampleColorForAccumulation(color)` under `OIT_ACCUMULATE`.
    pub fn order_independent(mut self) -> Self {
        self.order_independent = true;
        self
    }

    /// Let a resource load that cannot build this pipeline go on without it (NeoForge's
    /// `registerOptionalPipeline`).
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Let the shader sample `texture` via `Sampler0` or `<sunrise:pipeline_sprite.glsl>`'s
    /// `sampleSprite(uv)` (vanilla's `RenderPipeline.withSampler("Sampler0")`). A texture the
    /// load refuses ([`PipelineTextureError`]) refuses the pipeline as a missing shader does: a
    /// required one fails the load, an optional one is logged with the named cause and resolves
    /// to [`RenderPipelineError::Invalid`].
    pub fn textured(mut self, texture: PipelineTexture) -> Self {
        self.texture = Some(texture);
        self
    }

    /// Declare the world-geometry class this pipeline draws, so an active shader pack routes
    /// its draws through the matching standard `gbuffers_*` program chain before falling back
    /// to the pipeline's own shader.
    pub fn geometry_class(mut self, class: PipelineGeometryClass) -> Self {
        self.geometry_class = Some(class);
        self
    }

    /// Attach optional shader-pack integration extras (prepended program names, auxiliary
    /// depth target, frame uniforms, preprocessor defines, GLSL input bindings).
    pub fn pack_binding(mut self, binding: PackBinding) -> Self {
        self.pack_binding = Some(binding);
        self
    }

    /// Set the pipeline's mod-declared set-1 [`BindGroupLayout`] (set 0 stays vanilla's shared
    /// layouts from `BindGroupLayouts.java`).
    pub fn bind_group_layout(mut self, layout: BindGroupLayout) -> Self {
        self.bind_group_layout = layout;
        self
    }

    /// Append a std140 uniform buffer binding to this pipeline's set-1 [`BindGroupLayout`].
    pub fn uniform_buffer(mut self, name: impl Into<String>, min_size: u32) -> Self {
        self.bind_group_layout.entries.push(BindEntry {
            name: name.into(),
            kind: BindingKind::UniformBuffer { min_size },
        });
        self
    }

    /// Append a combined 2D image sampler binding to this pipeline's set-1 [`BindGroupLayout`],
    /// sampled as `sampler` describes ([`Sampler::new`] for one filter and one address mode).
    pub fn sampler(mut self, name: impl Into<String>, sampler: Sampler) -> Self {
        self.bind_group_layout.entries.push(BindEntry {
            name: name.into(),
            kind: BindingKind::CombinedImageSampler { sampler },
        });
        self
    }

    /// Append an integer texel buffer binding (`isamplerBuffer` / `usamplerBuffer`) to this
    /// pipeline's set-1 [`BindGroupLayout`].
    pub fn texel_buffer(mut self, name: impl Into<String>, format: TexelBufferFormat) -> Self {
        self.bind_group_layout.entries.push(BindEntry {
            name: name.into(),
            kind: BindingKind::TexelBuffer { format },
        });
        self
    }

    /// Declare a per-draw push-constant block size in bytes (`RenderPipeline.withPushConstantSize`,
    /// at most [`MAX_PUSH_CONSTANT_SIZE`] = 128 B, multiple of 4). Backed portably by an inline
    /// std140 uniform slot at the end of set 1.
    pub fn push_constant_size(mut self, bytes: u32) -> Self {
        self.push_constant_size = bytes;
        self
    }

    /// Validate this pipeline's set-1 [`BindGroupLayout`] and [`Self::push_constant_size`]:
    /// checks valid GLSL identifiers, duplicate names (`BindGroupLayout.ensureCompatible`),
    /// collisions with vanilla `BindGroupLayouts.java` set-0 names (plus `SpriteRect` when
    /// `.textured(...)` is set), positive uniform `min_size`, and push-constant bounds
    /// (`<= 128 B`, 4-byte aligned, `RenderPipeline.java:406`).
    pub fn validate_bind_group_layout(&self) -> Result<(), BindGroupLayoutError> {
        self.bind_group_layout.validate(self.texture.is_some())?;
        if self.push_constant_size > MAX_PUSH_CONSTANT_SIZE {
            return Err(BindGroupLayoutError::PushConstantTooLarge(
                self.push_constant_size,
            ));
        }
        if !self.push_constant_size.is_multiple_of(4) {
            return Err(BindGroupLayoutError::PushConstantUnaligned(
                self.push_constant_size,
            ));
        }
        Ok(())
    }

    /// The effective pack-program fallback chain in priority order: any names prepended in
    /// [`PackBinding::programs`], followed by [`PipelineGeometryClass::default_programs`],
    /// deduplicated while keeping the first occurrence. If none exist in the active pack or
    /// fail to translate, the pipeline falls back to its own shader.
    pub fn effective_pack_programs(&self) -> Vec<&str> {
        let mut chain = Vec::new();
        if let Some(binding) = &self.pack_binding {
            for name in &binding.programs {
                if !chain.contains(&name.as_str()) {
                    chain.push(name.as_str());
                }
            }
        }
        if let Some(class) = self.geometry_class {
            for &name in class.default_programs() {
                if !chain.contains(&name) {
                    chain.push(name);
                }
            }
        }
        chain
    }

    /// The effective shader-pack pass slot: [`PackBinding::slot`] when set, otherwise
    /// [`PipelineGeometryClass::default_slot`], or `None` when neither is declared.
    pub fn effective_pack_slot(&self) -> Option<PackPassSlot> {
        self.pack_binding
            .as_ref()
            .and_then(|binding| binding.slot)
            .or_else(|| self.geometry_class.map(PipelineGeometryClass::default_slot))
    }
}

/// What kind of world geometry a [`RenderPipeline`] draws, matching how Iris routes mod render
/// types through an active shader pack's standard `gbuffers_*` fallback chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline geometry classification.
pub enum PipelineGeometryClass {
    /// Opaque or cutout block-like terrain (`gbuffers_terrain` fallback chain).
    Terrain,
    /// Translucent terrain or fluid geometry (`gbuffers_water` fallback chain).
    Translucent,
    /// Entity, block-entity, or contraption geometry (`gbuffers_entities` fallback chain).
    Entity,
    /// Textured lit particle geometry (`gbuffers_particles` fallback chain).
    Particles,
    /// Sky dome or horizon geometry (`gbuffers_skybasic` fallback chain).
    Sky,
    /// Cloud geometry (`gbuffers_clouds` fallback chain).
    Clouds,
    /// Rain, snow, or weather geometry (`gbuffers_weather` fallback chain).
    Weather,
    /// Wireframe, outline, or line geometry (`gbuffers_line` fallback chain).
    Lines,
    /// First-person held item or hand geometry (`gbuffers_hand` fallback chain).
    Hand,
}

impl PipelineGeometryClass {
    /// The standard Iris/OptiFine pack-program fallback chain for this geometry class, in
    /// priority order. When none exist in the active pack or fail to translate, the pipeline
    /// falls back to its own shader.
    pub const fn default_programs(self) -> &'static [&'static str] {
        match self {
            Self::Terrain => &[
                "gbuffers_terrain",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Translucent => &[
                "gbuffers_water",
                "gbuffers_terrain",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Entity => &[
                "gbuffers_entities",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Particles => &[
                "gbuffers_particles",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Sky => &["gbuffers_skybasic", "gbuffers_basic"],
            Self::Clouds => &["gbuffers_clouds", "gbuffers_textured", "gbuffers_basic"],
            Self::Weather => &[
                "gbuffers_weather",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Lines => &["gbuffers_line", "gbuffers_basic"],
            Self::Hand => &[
                "gbuffers_hand",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
        }
    }

    /// The default shader-pack execution slot for this geometry class.
    pub const fn default_slot(self) -> PackPassSlot {
        match self {
            Self::Sky => PackPassSlot::AfterSky,
            Self::Terrain | Self::Entity | Self::Lines | Self::Hand => {
                PackPassSlot::AfterOpaqueGbuffers
            }
            Self::Translucent | Self::Particles | Self::Clouds => {
                PackPassSlot::AfterTranslucentGbuffers
            }
            Self::Weather => PackPassSlot::AfterWeather,
        }
    }
}

/// Execution slot inside an Iris/OptiFine shader pack's frame, ordered from earliest to latest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
// no wire: local client shader-pack execution slot.
pub enum PackPassSlot {
    /// Inside the pack's shadow pass, after vanilla shadow geometry and before `shadowcomp*`.
    Shadow,
    /// After the pack's sky pass (`gbuffers_skybasic`), before opaque terrain.
    AfterSky,
    /// After vanilla opaque gbuffers and before `deferred*`.
    AfterOpaqueGbuffers,
    /// After vanilla translucent gbuffers and before `composite*`.
    AfterTranslucentGbuffers,
    /// After weather gbuffers and before `composite*` and `final`.
    AfterWeather,
}

/// Which stages of a pipeline's pass in a shader pack come from the pack
/// ([`PackBinding::program_mode`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client shader-pack program mode.
pub enum PackProgramMode {
    /// The pack's matched program (`.vsh` and `.fsh`) replaces both of the mod's stages; its
    /// vertex stage reads the pipeline's vertices.
    #[default]
    PackVertexAndFragment,
    /// The mod's own vertex stage, with its [`VertexFormat`] and its bind group, is linked to the
    /// fragment stage of the pack's matched program, which writes the pack's draw buffers.
    ///
    /// The stages link by GLSL varying name: each varying the pack's fragment stage reads must be
    /// written by the mod's vertex stage under the same name (a struct member or `out` variable),
    /// type and interpolation, and the engine renumbers the mod's outputs to the locations the
    /// pack's translation gave them. Works only when the pack ships a program for one of the
    /// pipeline's stems without a geometry stage (that needs the pack's own vertex stage). When
    /// a varying is missing, unnamed or mistyped, the pack has no such program, or the device
    /// cannot hold the mod's bind groups after the pack's (its own `max_bind_groups`), the pipeline is unbound
    /// ([`crate::RenderAssets::pipeline_pack_bound`] is `false`) and the mod's own shaders draw. The
    /// mod's bind groups go after the pack's, which stay as the pack declared them.
    ModVertexPackFragment,
}

/// Optional Iris/OptiFine shader-pack extras attached to a [`RenderPipeline`]
/// ([`RenderPipeline::pack_binding`]). All identifiers are supplied by the mod; the engine
/// hardcodes none.
///
/// Startup refuses invalid GLSL identifiers, multi-line or comment-bearing define values,
/// collisions with reserved built-in pack samplers or uniforms, and cross-mod collisions on
/// pack samplers, uniforms, defines, or depth-target IDs (naming both mods).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline shader-pack binding descriptor.
pub struct PackBinding {
    /// Pack program stems prepended ahead of [`PipelineGeometryClass::default_programs`].
    pub programs: Vec<String>,
    /// Optional pass slot override; defaults to [`PipelineGeometryClass::default_slot`].
    pub slot: Option<PackPassSlot>,
    /// Preprocessor defines exposed while compiling the active shader pack when this pipeline
    /// is active: `(name, optional_value)`. `name` must be a valid GLSL identifier (not
    /// `GL_`-prefixed) and `value`, when present, must be a non-empty single-line token.
    /// Toggling active defines triggers at most one shader-pack reload.
    pub defines: Vec<(String, Option<String>)>,
    /// Optional depth target for non-shadow slots. See [`PackDepthTarget`] for attachment
    /// selection and write rules.
    pub depth_target: Option<PackDepthTarget>,
    /// Per-frame pack uniforms supplied by the mod under pack-side GLSL names. If a pack
    /// declares a uniform with a different GLSL type, the engine leaves it unbound and logs
    /// once without panicking.
    pub frame_uniforms: Vec<PackUniformDecl>,
    /// Mapping from the pipeline's vertex or per-draw inputs to pack-side GLSL names.
    pub glsl_bindings: Vec<PackGlslInput>,
    /// Which stages of the matched pack program run; [`PackProgramMode::PackVertexAndFragment`]
    /// by default.
    pub program_mode: PackProgramMode,
}

impl PackBinding {
    /// An empty shader-pack binding with no extra programs, defines, depth target, or uniforms.
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepend one pack program stem in front of the geometry class's default chain.
    pub fn program(mut self, name: impl Into<String>) -> Self {
        self.programs.push(name.into());
        self
    }

    /// Override the pack execution slot instead of using the geometry class default.
    pub fn slot(mut self, slot: PackPassSlot) -> Self {
        self.slot = Some(slot);
        self
    }

    /// Declare a preprocessor define `#define NAME` (no value) while this pipeline is active.
    pub fn define(mut self, name: impl Into<String>) -> Self {
        self.defines.push((name.into(), None));
        self
    }

    /// Declare a preprocessor define `#define NAME VALUE` while this pipeline is active.
    pub fn define_value(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.defines.push((name.into(), Some(value.into())));
        self
    }

    /// Declare a [`PackDepthTarget`] for the pipeline's non-shadow slot.
    pub fn depth_target(mut self, target: PackDepthTarget) -> Self {
        self.depth_target = Some(target);
        self
    }

    /// Declare a per-frame pack uniform supplied by this pipeline.
    pub fn frame_uniform(mut self, uniform: PackUniformDecl) -> Self {
        self.frame_uniforms.push(uniform);
        self
    }

    /// Choose which stages of the matched pack program run (see [`PackProgramMode`]).
    pub fn program_mode(mut self, mode: PackProgramMode) -> Self {
        self.program_mode = mode;
        self
    }

    /// Declare one mapping from a pipeline input to a pack-side GLSL input or uniform name.
    pub fn glsl_binding(mut self, input: PackGlslInput) -> Self {
        self.glsl_bindings.push(input);
        self
    }

    /// Validate GLSL identifier syntax and single-line define token rules on all declared
    /// entries. Called by the engine during pipeline registration freeze.
    pub fn validate_syntax(&self) -> Result<(), String> {
        for program in &self.programs {
            if !is_valid_pack_program_name(program) {
                return Err(format!(
                    "invalid pack program name {program:?}: expected ASCII [A-Za-z0-9_]+"
                ));
            }
        }
        for (name, value) in &self.defines {
            if !is_valid_glsl_identifier(name) || name.starts_with("GL_") {
                return Err(format!(
                    "invalid pack define name {name:?}: expected a valid non-GL_ GLSL identifier"
                ));
            }
            if let Some(val) = value
                && !is_valid_glsl_define_value(val)
            {
                return Err(format!(
                    "invalid pack define value for {name}: {val:?} must be a non-empty single-line token without comments, backslashes, or '#'"
                ));
            }
        }
        if let Some(PackDepthTarget::Aux {
            id,
            depth_samplers,
            opaque_depth_samplers,
        }) = &self.depth_target
        {
            let mut seen_samplers = std::collections::HashSet::new();
            for sampler in depth_samplers {
                if !is_valid_glsl_identifier(sampler) || sampler.starts_with("gl_") {
                    return Err(format!(
                        "invalid pack depth sampler name {sampler:?}: expected a valid non-gl_ GLSL identifier"
                    ));
                }
                if !seen_samplers.insert(sampler.as_str()) {
                    return Err(format!(
                        "duplicate pack depth sampler {sampler:?} on target {}",
                        id
                    ));
                }
            }
            for sampler in opaque_depth_samplers {
                if !is_valid_glsl_identifier(sampler) || sampler.starts_with("gl_") {
                    return Err(format!(
                        "invalid pack depth sampler name {sampler:?}: expected a valid non-gl_ GLSL identifier"
                    ));
                }
                if depth_samplers.iter().any(|s| s == sampler) {
                    return Err(format!(
                        "pack depth sampler {sampler:?} on target {} cannot be both a full depth sampler and an opaque depth sampler",
                        id
                    ));
                }
                if !seen_samplers.insert(sampler.as_str()) {
                    return Err(format!(
                        "duplicate pack opaque depth sampler {sampler:?} on target {}",
                        id
                    ));
                }
            }
        }
        let mut current_uniforms = std::collections::HashSet::new();
        let mut previous_uniforms = std::collections::HashSet::new();
        for uniform in &self.frame_uniforms {
            if !is_valid_glsl_identifier(&uniform.name) || uniform.name.starts_with("gl_") {
                return Err(format!(
                    "invalid pack uniform name {:?}: expected a valid non-gl_ GLSL identifier",
                    uniform.name
                ));
            }
            if previous_uniforms.contains(uniform.name.as_str()) {
                return Err(format!(
                    "pack uniform {:?} collides with another uniform's previous_name",
                    uniform.name
                ));
            }
            current_uniforms.insert(uniform.name.as_str());
            if let Some(prev) = &uniform.previous_name {
                if !is_valid_glsl_identifier(prev) || prev.starts_with("gl_") {
                    return Err(format!(
                        "invalid pack previous uniform name {prev:?}: expected a valid non-gl_ GLSL identifier"
                    ));
                }
                if prev == &uniform.name {
                    return Err(format!(
                        "pack uniform {:?} cannot reuse its own name as previous_name",
                        uniform.name
                    ));
                }
                if current_uniforms.contains(prev.as_str()) {
                    return Err(format!(
                        "pack previous uniform {prev:?} collides with another uniform's current name"
                    ));
                }
                previous_uniforms.insert(prev.as_str());
            }
        }
        for input in &self.glsl_bindings {
            if !is_valid_glsl_identifier(&input.source) {
                return Err(format!(
                    "invalid pack GLSL binding source {:?}: expected a valid GLSL identifier",
                    input.source
                ));
            }
            if !is_valid_glsl_identifier(&input.glsl_name) {
                return Err(format!(
                    "invalid pack GLSL binding target {:?}: expected a valid GLSL identifier",
                    input.glsl_name
                ));
            }
        }
        Ok(())
    }
}

/// Depth attachment selection for a linked shader-pack pass.
///
/// Writes require an explicit request such as [`RenderPipeline::depth_write`] with `true`.
/// The comparison is mapped to standard-Z as described by [`DepthCompare`], independently
/// of writes.
///
/// [`PackPassSlot::Shadow`] compares against and, when requested, writes to the pack's shadow
/// depth (`shadowtex0`), regardless of whether [`Self::Main`] or [`Self::Aux`] is declared.
///
/// In non-shadow slots, an explicit target permits writes. Without a target, the pass compares
/// against the pack's main depth (`depthtex0`), and only [`PackPassSlot::AfterOpaqueGbuffers`]
/// can write it. Choose [`Self::Main`] to allow writes to main depth in other non-shadow slots,
/// or [`Self::Aux`] to declare a separate depth plane.
///
/// When targeting main depth, a mod's own vertex stage can emit depth in the world's projection
/// directly, so a mod projecting its own geometry needs no separate depth copy. The fragment
/// stage is the pack's, so `gl_FragDepth` is not the mod's to write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client shader-pack depth target descriptor.
pub enum PackDepthTarget {
    /// The pack's main depth (`depthtex0`), which the pack's later passes read as the world's.
    Main,
    /// An auxiliary `Depth32Float` attachment shared by pipelines of the same mod using the same
    /// `id` (cleared to OpenGL depth `1.0`, matching `depthtex0`).
    ///
    /// The engine can select this attachment for a linked non-shadow pass when a loaded pack
    /// program references one of its samplers. It can retain the attachment even if no built
    /// pass samples it, so the linked pass can sample pack world depth without attaching
    /// `depthtex0`.
    ///
    /// If the auxiliary attachment is not used, the linked non-shadow pass compares against
    /// `depthtex0` and can still write it when requested, even in slots that are read-only
    /// without an explicit target.
    #[non_exhaustive]
    Aux {
        /// Mod-namespaced key identifying the shared depth attachment.
        id: Identifier,
        /// Sampler names exposed to pack passes for full depth after draws to this target.
        depth_samplers: Vec<String>,
        /// Optional sampler names exposed for an opaque-depth copy taken after
        /// [`PackPassSlot::AfterOpaqueGbuffers`] and before
        /// [`PackPassSlot::AfterTranslucentGbuffers`], populated only while sampled by the pack.
        opaque_depth_samplers: Vec<String>,
    },
}

impl PackDepthTarget {
    /// Describe an auxiliary depth target keyed by `id`.
    pub fn new(id: Identifier) -> Self {
        Self::Aux {
            id,
            depth_samplers: Vec::new(),
            opaque_depth_samplers: Vec::new(),
        }
    }

    /// Expose the full depth buffer under GLSL sampler name `name`. [`Self::Main`] has the
    /// pack's own samplers, so this leaves it unchanged.
    pub fn depth_sampler(mut self, name: impl Into<String>) -> Self {
        if let Self::Aux { depth_samplers, .. } = &mut self {
            depth_samplers.push(name.into());
        }
        self
    }

    /// Expose the post-opaque depth snapshot under GLSL sampler name `name`. [`Self::Main`]
    /// leaves it unchanged.
    pub fn opaque_depth_sampler(mut self, name: impl Into<String>) -> Self {
        if let Self::Aux {
            opaque_depth_samplers,
            ..
        } = &mut self
        {
            opaque_depth_samplers.push(name.into());
        }
        self
    }
}

/// A per-frame pack uniform declared by a [`PackBinding`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client shader-pack frame uniform declaration.
pub struct PackUniformDecl {
    /// GLSL uniform name read by the active shader pack.
    pub name: String,
    /// GLSL value type supplied by the mod.
    pub ty: PackUniformType,
    /// Optional previous-frame uniform alias auto-tracked by the engine.
    pub previous_name: Option<String>,
}

impl PackUniformDecl {
    /// Declare a per-frame pack uniform of type `ty`.
    pub fn new(name: impl Into<String>, ty: PackUniformType) -> Self {
        Self {
            name: name.into(),
            ty,
            previous_name: None,
        }
    }

    /// Also expose the previous frame's value under `previous_name`.
    pub fn with_previous(mut self, previous_name: impl Into<String>) -> Self {
        self.previous_name = Some(previous_name.into());
        self
    }
}

/// GLSL value type for a [`PackUniformDecl`] or [`PackGlslInput`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client shader-pack uniform GLSL type selector.
pub enum PackUniformType {
    /// `float`
    Float,
    /// `int`
    Int,
    /// `vec2`
    Vec2,
    /// `vec3`
    Vec3,
    /// `vec4`
    Vec4,
    /// `mat3`
    Mat3,
    /// `mat4`
    Mat4,
}

/// How an active shader pack binds a registered [`RenderPipeline`], reported by
/// [`crate::WorldDrawContext::pack_override_state`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client query value describing active shader-pack override state.
pub enum PackOverrideState {
    /// No translated shader pack is active.
    NoPack,
    /// A shader pack is active, but the pipeline was not bound to a translated pack program
    /// (either it has no [`PackBinding`] or no candidate program compiled); the mod's own shader
    /// runs instead.
    UnboundPack,
    /// The active shader pack translated and bound a pack program for this pipeline.
    Bound,
}

/// What the active Iris/OptiFine shader pack does that a mod's own rendering must account for,
/// read through [`crate::ClientOptionsClientHost::shader_pack_state`]. `None` there when no pack
/// is active, with [`crate::ClientOptionsClientHost::shader_pack_active`]'s rule (false on the
/// title screen and while unloading).
///
/// It is taken at the start of each frame's render and read unchanged by that frame's render
/// callbacks, so everything in it is that frame's value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client query value describing the active shader pack.
pub struct ShaderPackState {
    /// Whether the pack hides vanilla clouds (`clouds=off`, or it draws its own), as Iris's
    /// `DHCompatInternal.avoidRenderingClouds` reads the pack: a mod that draws clouds or a sky
    /// should skip its clouds while this is true.
    pub hides_clouds: bool,
    /// The shadow pass, or `None` when the pack has none.
    pub shadow: Option<ShaderPackShadowView>,
    /// Changes every time a shader pack is loaded, switched or turned off, including a reload of
    /// the same pack and a dimension change that loads the pack again, and at no other time.
    /// Compare it each frame and re-read what you derived from the pack when it differs: that
    /// covers a pack switch with no resource reload.
    pub generation: u32,
}

impl ShaderPackState {
    /// A state, built by the game.
    pub fn new(hides_clouds: bool, shadow: Option<ShaderPackShadowView>, generation: u32) -> Self {
        Self {
            hides_clouds,
            shadow,
            generation,
        }
    }
}

/// The active shader pack's shadow pass for the current frame (Iris's `ShadowRenderer`:
/// `MODELVIEW`, `PROJECTION`, shadow map resolution and half-plane distance), for a mod that
/// submits shadow-casting geometry or culls its own against the shadow volume.
///
/// `model_view` and `projection` are in the same camera-relative space as
/// [`WorldRenderView`](crate::WorldRenderView) and column-major like its matrices. They are the
/// current frame's values, valid while that frame's render callbacks run.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client query value describing the active shader pack's shadow pass.
pub struct ShaderPackShadowView {
    /// The shadow map's side in texels (`shadowMapResolution`).
    pub resolution: u32,
    /// Half the side of the area the map covers, in blocks (`shadowDistance`).
    pub distance: f32,
    /// `shadowModelView`: camera-relative world space to shadow view space, column-major.
    pub model_view: [f32; 16],
    /// `shadowProjection`: shadow view space to shadow clip space, column-major.
    pub projection: [f32; 16],
}

impl ShaderPackShadowView {
    /// A shadow view, built by the game.
    pub fn new(
        resolution: u32,
        distance: f32,
        model_view: [f32; 16],
        projection: [f32; 16],
    ) -> Self {
        Self {
            resolution,
            distance,
            model_view,
            projection,
        }
    }
}

/// Per-frame value for a mod-declared shader-pack uniform ([`PackUniformDecl`]),
/// supplied through [`crate::WorldDrawContext::set_pack_uniform`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client shader-pack per-frame uniform value.
pub enum PackUniformValue {
    /// GLSL `float`
    Float(f32),
    /// GLSL `int`
    Int(i32),
    /// GLSL `vec2`
    Vec2([f32; 2]),
    /// GLSL `vec3`
    Vec3([f32; 3]),
    /// GLSL `vec4`
    Vec4([f32; 4]),
    /// GLSL `mat3` (column-major)
    Mat3([[f32; 3]; 3]),
    /// GLSL `mat4` (column-major)
    Mat4([[f32; 4]; 4]),
}

impl PackUniformValue {
    /// The [`PackUniformType`] matching this value's variant.
    pub const fn ty(&self) -> PackUniformType {
        match self {
            Self::Float(_) => PackUniformType::Float,
            Self::Int(_) => PackUniformType::Int,
            Self::Vec2(_) => PackUniformType::Vec2,
            Self::Vec3(_) => PackUniformType::Vec3,
            Self::Vec4(_) => PackUniformType::Vec4,
            Self::Mat3(_) => PackUniformType::Mat3,
            Self::Mat4(_) => PackUniformType::Mat4,
        }
    }
}

/// Mapping from a pipeline vertex attribute or per-draw value to a pack-side GLSL input name.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client shader-pack GLSL input binding descriptor.
pub struct PackGlslInput {
    /// Pipeline-side attribute or per-draw input name.
    pub source: String,
    /// Target GLSL identifier seen by the shader pack program (e.g. `gl_Vertex` or a named
    /// attribute or uniform).
    pub glsl_name: String,
    /// GLSL value type of the bound input.
    pub ty: PackUniformType,
}

impl PackGlslInput {
    /// Map pipeline input `source` to pack-side GLSL name `glsl_name` of type `ty`.
    pub fn new(
        source: impl Into<String>,
        glsl_name: impl Into<String>,
        ty: PackUniformType,
    ) -> Self {
        Self {
            source: source.into(),
            glsl_name: glsl_name.into(),
            ty,
        }
    }
}

/// Whether `name` is a valid ASCII GLSL identifier (`[A-Za-z_][A-Za-z0-9_]*`).
pub(crate) fn is_valid_glsl_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Whether `value` is a safe, non-empty single-line GLSL `#define` value token sequence:
/// no ASCII control characters (including newlines), no leading/trailing whitespace, no
/// backslash continuations, no preprocessor `#`, and no `//`, `/*`, or `*/` comment tokens.
pub(crate) fn is_valid_glsl_define_value(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && !value.bytes().any(|b| b.is_ascii_control())
        && !value.contains('\\')
        && !value.contains('#')
        && !value.contains("//")
        && !value.contains("/*")
        && !value.contains("*/")
}

/// Whether `name` is a valid pack program stem (`[A-Za-z0-9_]+`, no path separators or dots).
fn is_valid_pack_program_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A texture a pipeline's shader samples ([`RenderPipeline::textured`]): always a rectangle of
/// the atlas the host binds, never a texture of its own. Including `<sunrise:pipeline_sprite.glsl>`
/// exposes `vec4 sampleSprite(vec2 uv)`, which maps `fract(uv)` into the sprite's rectangle and
/// loads one texel by integer coordinates at mip level 0 (nearest, unfiltered, so no neighbouring
/// sprite's texel enters) in vanilla's gamma space.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline texture binding.
pub enum PipelineTexture {
    /// A sprite of the game's block or item atlas, by its texture id: `powah:block/energy_charge`
    /// is `assets/powah/textures/block/energy_charge.png` as the `minecraft:blocks` atlas holds
    /// it, `minecraft:item/diamond` an item sprite. An animated sprite animates. A sprite in
    /// neither atlas refuses the pipeline ([`PipelineTextureError::NoSuchSprite`]).
    Sprite(Identifier),
}

/// Why a pipeline's texture was refused: the named cause its refusal is logged with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client render-pipeline texture load.
pub enum PipelineTextureError {
    /// No sprite of that id in the block or item atlas.
    NoSuchSprite,
    /// No texture file of that id in the mod's assets or a resource pack.
    NoSuchTextureFile,
    /// The id names a file (a `textures/` path or a `.png` name), not a sprite key.
    NotASpriteKey,
    /// A texture file that isn't an image the game can decode.
    Undecodable,
}

impl std::fmt::Display for PipelineTextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoSuchSprite => "no such sprite in the block or item atlas",
            Self::NoSuchTextureFile => "no such texture file",
            Self::NotASpriteKey => "the id names a file, not a sprite key",
            Self::Undecodable => "the texture file cannot be decoded",
        })
    }
}

impl std::error::Error for PipelineTextureError {}

/// Required byte alignment for vertex strides (`VertexFormat.VERTEX_ALIGNMENT = 4`).
pub const VERTEX_ALIGNMENT: u32 = 4;

/// Maximum number of vertex attributes across all bindings of a [`RenderPipeline`]
/// (`VertexFormat.MAX_VERTEX_ELEMENTS = 16`).
pub const MAX_VERTEX_ELEMENTS: usize = 16;

/// Maximum number of vertex buffer binding slots on a [`RenderPipeline`]
///. At GPU build time, a pipeline using a binding index `>=`
/// `device.limits().max_vertex_buffers` is refused cleanly.
pub const MAX_VERTEX_BINDINGS: usize = 16;

/// Per-attribute vertex data format (the portable WebGPU/wgpu subset of vanilla's `GpuFormat`,
/// supported across Linux Vulkan, Windows DX12/Vulkan, and Apple-chip macOS Metal without
/// extensions). Each format maps 1:1 to a GLSL vertex `in` type (`float`/`vecN`/`int`/`ivecN`/
/// `uint`/`uvecN`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex attribute format.
pub enum VertexAttributeFormat {
    /// One 32-bit float (`float`).
    Float32,
    /// Two 32-bit floats (`vec2`).
    Float32x2,
    /// Three 32-bit floats (`vec3`).
    Float32x3,
    /// Four 32-bit floats (`vec4`).
    Float32x4,
    /// Two 16-bit floats (`vec2`).
    Float16x2,
    /// Four 16-bit floats (`vec4`).
    Float16x4,
    /// Two unsigned normalized 8-bit integers (`vec2`).
    Unorm8x2,
    /// Four unsigned normalized 8-bit integers (`vec4`).
    Unorm8x4,
    /// Two signed normalized 8-bit integers (`vec2`).
    Snorm8x2,
    /// Four signed normalized 8-bit integers (`vec4`).
    Snorm8x4,
    /// Two unsigned 8-bit integers (`uvec2`).
    Uint8x2,
    /// Four unsigned 8-bit integers (`uvec4`).
    Uint8x4,
    /// Two signed 8-bit integers (`ivec2`).
    Sint8x2,
    /// Four signed 8-bit integers (`ivec4`).
    Sint8x4,
    /// Two unsigned normalized 16-bit integers (`vec2`).
    Unorm16x2,
    /// Four unsigned normalized 16-bit integers (`vec4`).
    Unorm16x4,
    /// Two signed normalized 16-bit integers (`vec2`).
    Snorm16x2,
    /// Four signed normalized 16-bit integers (`vec4`).
    Snorm16x4,
    /// Two unsigned 16-bit integers (`uvec2`).
    Uint16x2,
    /// Four unsigned 16-bit integers (`uvec4`).
    Uint16x4,
    /// Two signed 16-bit integers (`ivec2`).
    Sint16x2,
    /// Four signed 16-bit integers (`ivec4`).
    Sint16x4,
    /// One unsigned 32-bit integer (`uint`).
    Uint32,
    /// Two unsigned 32-bit integers (`uvec2`).
    Uint32x2,
    /// Three unsigned 32-bit integers (`uvec3`).
    Uint32x3,
    /// Four unsigned 32-bit integers (`uvec4`).
    Uint32x4,
    /// One signed 32-bit integer (`int`).
    Sint32,
    /// Two signed 32-bit integers (`ivec2`).
    Sint32x2,
    /// Three signed 32-bit integers (`ivec3`).
    Sint32x3,
    /// Four signed 32-bit integers (`ivec4`).
    Sint32x4,
    /// Packed 10-10-10-2 unsigned normalized components (`vec4`).
    Unorm10_10_10_2,
}

impl VertexAttributeFormat {
    /// Size of one attribute value in bytes.
    pub const fn byte_size(self) -> u32 {
        match self {
            Self::Unorm8x2 | Self::Snorm8x2 | Self::Uint8x2 | Self::Sint8x2 => 2,
            Self::Float32
            | Self::Float16x2
            | Self::Unorm8x4
            | Self::Snorm8x4
            | Self::Uint8x4
            | Self::Sint8x4
            | Self::Unorm16x2
            | Self::Snorm16x2
            | Self::Uint16x2
            | Self::Sint16x2
            | Self::Uint32
            | Self::Sint32
            | Self::Unorm10_10_10_2 => 4,
            Self::Float32x2
            | Self::Float16x4
            | Self::Unorm16x4
            | Self::Snorm16x4
            | Self::Uint16x4
            | Self::Sint16x4
            | Self::Uint32x2
            | Self::Sint32x2 => 8,
            Self::Float32x3 | Self::Uint32x3 | Self::Sint32x3 => 12,
            Self::Float32x4 | Self::Uint32x4 | Self::Sint32x4 => 16,
        }
    }

    /// Required byte alignment of the attribute's offset inside its vertex buffer (`min(4,
    /// byte_size)`, satisfying both `GpuFormat.byteAlignment()` and WebGPU vertex attribute
    /// alignment).
    pub const fn alignment(self) -> u32 {
        let size = self.byte_size();
        if size < 4 { size } else { 4 }
    }

    /// The GLSL vertex `in` type for this attribute (`float`/`vecN` for float and normalized
    /// formats, `int`/`ivecN` for signed integer formats, `uint`/`uvecN` for unsigned integer
    /// formats).
    pub const fn glsl_type(self) -> &'static str {
        match self {
            Self::Float32 => "float",
            Self::Float32x2
            | Self::Float16x2
            | Self::Unorm8x2
            | Self::Snorm8x2
            | Self::Unorm16x2
            | Self::Snorm16x2 => "vec2",
            Self::Float32x3 => "vec3",
            Self::Float32x4
            | Self::Float16x4
            | Self::Unorm8x4
            | Self::Snorm8x4
            | Self::Unorm16x4
            | Self::Snorm16x4
            | Self::Unorm10_10_10_2 => "vec4",
            Self::Uint32 => "uint",
            Self::Uint8x2 | Self::Uint16x2 | Self::Uint32x2 => "uvec2",
            Self::Uint32x3 => "uvec3",
            Self::Uint8x4 | Self::Uint16x4 | Self::Uint32x4 => "uvec4",
            Self::Sint32 => "int",
            Self::Sint8x2 | Self::Sint16x2 | Self::Sint32x2 => "ivec2",
            Self::Sint32x3 => "ivec3",
            Self::Sint8x4 | Self::Sint16x4 | Self::Sint32x4 => "ivec4",
        }
    }

    /// Whether this attribute is read as an integer (`int`/`ivecN`/`uint`/`uvecN`) rather than
    /// floating-point (`float`/`vecN`).
    pub const fn is_integer(self) -> bool {
        matches!(
            self,
            Self::Uint8x2
                | Self::Uint8x4
                | Self::Sint8x2
                | Self::Sint8x4
                | Self::Uint16x2
                | Self::Uint16x4
                | Self::Sint16x2
                | Self::Sint16x4
                | Self::Uint32
                | Self::Uint32x2
                | Self::Uint32x3
                | Self::Uint32x4
                | Self::Sint32
                | Self::Sint32x2
                | Self::Sint32x3
                | Self::Sint32x4
        )
    }
}

/// How a vertex buffer binding advances across vertices or instances (`VertexFormat.stepRate`).
///
/// WebGPU/wgpu supports per-vertex (`0`) and per-instance (`1`) step rates only; arbitrary
/// instance divisors `> 1` are refused with
/// [`VertexFormatError::UnsupportedStepRate`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex step mode.
pub enum VertexStepMode {
    /// Advance once per vertex (`stepRate = 0`).
    #[default]
    Vertex,
    /// Advance once per instance (`stepRate = 1`).
    Instance,
}

impl VertexStepMode {
    /// Convert a vanilla numeric `stepRate` (`0` = per-vertex, `1` = per-instance), refusing
    /// divisors `> 1` with [`VertexFormatError::UnsupportedStepRate`].
    pub const fn from_step_rate(step_rate: u32) -> Result<Self, VertexFormatError> {
        match step_rate {
            0 => Ok(Self::Vertex),
            1 => Ok(Self::Instance),
            rate => Err(VertexFormatError::UnsupportedStepRate(rate)),
        }
    }
}

/// One named attribute inside a [`VertexFormat`] (`VertexFormatElement`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex attribute declaration.
pub struct VertexAttribute {
    /// GLSL vertex `in` identifier (must be a valid non-`gl_` ASCII GLSL identifier).
    pub name: String,
    /// Byte offset from the start of each vertex/instance record.
    pub offset: u32,
    /// Data encoding of this attribute.
    pub format: VertexAttributeFormat,
}

impl VertexAttribute {
    /// Describe one vertex attribute.
    pub fn new(name: impl Into<String>, offset: u32, format: VertexAttributeFormat) -> Self {
        Self {
            name: name.into(),
            offset,
            format,
        }
    }
}

/// Why a [`VertexFormat`] declaration was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex format validation error.
pub enum VertexFormatError {
    /// Attribute name is not a valid non-`gl_` ASCII GLSL identifier.
    InvalidAttributeName(String),
    /// Two attributes in the same format share a name.
    DuplicateAttributeName(String),
    /// Attribute byte offset is not aligned to `format.alignment()`.
    UnalignedOffset {
        /// Attribute name.
        name: String,
        /// Misaligned byte offset.
        offset: u32,
        /// Required byte alignment.
        alignment: u32,
    },
    /// Attribute bytes extend past the format's vertex stride.
    AttributeOutOfBounds {
        /// Attribute name.
        name: String,
        /// Byte end offset (`offset + byte_size`).
        end: u32,
        /// Vertex stride in bytes.
        stride: u32,
    },
    /// Vertex stride is zero or not a multiple of [`VERTEX_ALIGNMENT`] (4)
    ///.
    InvalidStride(u32),
    /// Format declares more than [`MAX_VERTEX_ELEMENTS`] (16) attributes
    ///.
    TooManyAttributes(usize),
    /// Step-rate divisor `> 1` is not supported on WebGPU/wgpu.
    UnsupportedStepRate(u32),
}

impl std::fmt::Display for VertexFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidAttributeName(name) => write!(
                f,
                "invalid vertex attribute name {name:?}: expected a valid non-gl_ GLSL identifier"
            ),
            Self::DuplicateAttributeName(name) => {
                write!(f, "duplicate vertex attribute name {name:?}")
            }
            Self::UnalignedOffset {
                name,
                offset,
                alignment,
            } => write!(
                f,
                "vertex attribute {name:?} offset {offset} is not aligned to {alignment} bytes"
            ),
            Self::AttributeOutOfBounds { name, end, stride } => write!(
                f,
                "vertex attribute {name:?} ends at byte {end}, exceeding stride {stride}"
            ),
            Self::InvalidStride(stride) => write!(
                f,
                "vertex stride {stride} must be non-zero and a multiple of {VERTEX_ALIGNMENT}"
            ),
            Self::TooManyAttributes(count) => write!(
                f,
                "vertex format has {count} attributes, exceeding MAX_VERTEX_ELEMENTS ({MAX_VERTEX_ELEMENTS})"
            ),
            Self::UnsupportedStepRate(rate) => write!(
                f,
                "vertex step rate {rate} is unsupported: only 0 (Vertex) and 1 (Instance) are supported"
            ),
        }
    }
}

impl std::error::Error for VertexFormatError {}

/// Memory layout of one vertex or instance buffer binding (`com.mojang.renderpearl.api.vertex.VertexFormat`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex buffer format.
pub struct VertexFormat {
    /// Whether this binding advances per vertex or per instance.
    pub step_mode: VertexStepMode,
    /// Size of one record in bytes (`> 0`, multiple of [`VERTEX_ALIGNMENT`] = 4).
    pub stride: u32,
    /// Attributes read from this binding (`<= MAX_VERTEX_ELEMENTS`).
    pub attributes: Vec<VertexAttribute>,
}

impl VertexFormat {
    /// Start building a [`VertexFormat`] with the given [`VertexStepMode`].
    pub fn builder(step_mode: VertexStepMode) -> VertexFormatBuilder {
        VertexFormatBuilder {
            step_mode,
            unsupported_step_rate: None,
            attributes: Vec::new(),
            auto_offset: 0,
            explicit_stride: None,
        }
    }

    /// Start building a [`VertexFormat`] from a numeric vanilla `stepRate` (`0` = per-vertex,
    /// `1` = per-instance). Divisors `> 1` fail [`VertexFormatBuilder::build`] with
    /// [`VertexFormatError::UnsupportedStepRate`].
    pub fn builder_with_step_rate(step_rate: u32) -> VertexFormatBuilder {
        match VertexStepMode::from_step_rate(step_rate) {
            Ok(step_mode) => Self::builder(step_mode),
            Err(_) => VertexFormatBuilder {
                step_mode: VertexStepMode::Vertex,
                unsupported_step_rate: Some(step_rate),
                attributes: Vec::new(),
                auto_offset: 0,
                explicit_stride: None,
            },
        }
    }

    /// The default binding 0 of a pipeline, and what [`PipelineQuad`] gives: a position, a texture
    /// coordinate and a colour (`Position: Float32x3`
    /// at 0, `UV0: Float32x2` at 12, `Color: Unorm8x4` at 20, stride 24).
    pub fn position_tex_color() -> Self {
        Self {
            step_mode: VertexStepMode::Vertex,
            stride: 24,
            attributes: vec![
                VertexAttribute::new("Position", 0, VertexAttributeFormat::Float32x3),
                VertexAttribute::new("UV0", 12, VertexAttributeFormat::Float32x2),
                VertexAttribute::new("Color", 20, VertexAttributeFormat::Unorm8x4),
            ],
        }
    }

    /// Validate all vertex-format invariants (GLSL identifiers, uniqueness, alignment, stride,
    /// attribute bounds, and attribute count).
    pub fn validate(&self) -> Result<(), VertexFormatError> {
        if self.attributes.len() > MAX_VERTEX_ELEMENTS {
            return Err(VertexFormatError::TooManyAttributes(self.attributes.len()));
        }
        if self.stride == 0 || !self.stride.is_multiple_of(VERTEX_ALIGNMENT) {
            return Err(VertexFormatError::InvalidStride(self.stride));
        }
        let mut seen = std::collections::HashSet::with_capacity(self.attributes.len());
        for attr in &self.attributes {
            if !is_valid_glsl_identifier(&attr.name) || attr.name.starts_with("gl_") {
                return Err(VertexFormatError::InvalidAttributeName(attr.name.clone()));
            }
            if !seen.insert(attr.name.as_str()) {
                return Err(VertexFormatError::DuplicateAttributeName(attr.name.clone()));
            }
            let alignment = attr.format.alignment();
            if attr.offset % alignment != 0 {
                return Err(VertexFormatError::UnalignedOffset {
                    name: attr.name.clone(),
                    offset: attr.offset,
                    alignment,
                });
            }
            let end = attr.offset.saturating_add(attr.format.byte_size());
            if end > self.stride {
                return Err(VertexFormatError::AttributeOutOfBounds {
                    name: attr.name.clone(),
                    end,
                    stride: self.stride,
                });
            }
        }
        Ok(())
    }
}

/// Builder for [`VertexFormat`], created via [`VertexFormat::builder`].
#[derive(Clone, Debug)]
// no wire: local client vertex format builder.
// exhaustive: opaque builder with private fields.
pub struct VertexFormatBuilder {
    step_mode: VertexStepMode,
    unsupported_step_rate: Option<u32>,
    attributes: Vec<VertexAttribute>,
    auto_offset: u32,
    explicit_stride: Option<u32>,
}

impl VertexFormatBuilder {
    /// Append an attribute at the current byte cursor and advance the cursor by
    /// `format.byte_size()`.
    pub fn attribute(mut self, name: impl Into<String>, format: VertexAttributeFormat) -> Self {
        let offset = self.auto_offset;
        self.attributes.push(VertexAttribute {
            name: name.into(),
            offset,
            format,
        });
        self.auto_offset = offset.saturating_add(format.byte_size());
        self
    }

    /// Append an attribute at an explicit byte `offset`, advancing the automatic cursor to at
    /// least `offset + format.byte_size()`.
    pub fn attribute_at(
        mut self,
        name: impl Into<String>,
        offset: u32,
        format: VertexAttributeFormat,
    ) -> Self {
        self.attributes.push(VertexAttribute {
            name: name.into(),
            offset,
            format,
        });
        self.auto_offset = self
            .auto_offset
            .max(offset.saturating_add(format.byte_size()));
        self
    }

    /// Set an explicit vertex stride in bytes (must be non-zero and a multiple of
    /// [`VERTEX_ALIGNMENT`] = 4). When omitted, the byte cursor after the last attribute is used.
    pub fn stride(mut self, bytes: u32) -> Self {
        self.explicit_stride = Some(bytes);
        self
    }

    /// Validate and finish the [`VertexFormat`].
    pub fn build(self) -> Result<VertexFormat, VertexFormatError> {
        if let Some(rate) = self.unsupported_step_rate {
            return Err(VertexFormatError::UnsupportedStepRate(rate));
        }
        let format = VertexFormat {
            step_mode: self.step_mode,
            stride: self.explicit_stride.unwrap_or(self.auto_offset),
            attributes: self.attributes,
        };
        format.validate()?;
        Ok(format)
    }
}

/// Index element type for indexed mesh draws (`com.mojang.renderpearl.api.pipeline.IndexType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client mesh index format.
pub enum IndexFormat {
    /// 16-bit unsigned indices (`IndexType.SHORT`, 2 bytes per index).
    Uint16,
    /// 32-bit unsigned indices (`IndexType.INT`, 4 bytes per index).
    Uint32,
}

impl IndexFormat {
    /// Size of one index in bytes (`2` for `Uint16`, `4` for `Uint32`).
    pub const fn byte_size(self) -> u32 {
        match self {
            Self::Uint16 => 2,
            Self::Uint32 => 4,
        }
    }
}

/// Initial descriptor for one vertex or instance buffer in a [`MeshDescriptor`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client retained mesh vertex buffer descriptor.
pub struct MeshBufferInit<'a> {
    /// Record stride in bytes (`> 0`, multiple of 4).
    pub stride: u32,
    /// Total GPU buffer size in bytes (non-zero, multiple of `stride`, `<= device.limits().max_buffer_size`).
    pub byte_len: u64,
    /// Optional initial bytes written at offset 0 (`len <= byte_len`, multiple of 4).
    pub initial: Option<&'a [u8]>,
}

impl<'a> MeshBufferInit<'a> {
    /// Create a buffer sized exactly to `bytes` and initialized with `bytes`.
    pub const fn from_bytes(stride: u32, bytes: &'a [u8]) -> Self {
        Self {
            stride,
            byte_len: bytes.len() as u64,
            initial: Some(bytes),
        }
    }

    /// Create an uninitialized GPU buffer of `byte_len` bytes to be filled via
    /// [`MeshRef::write_vertices`].
    pub const fn empty(stride: u32, byte_len: u64) -> Self {
        Self {
            stride,
            byte_len,
            initial: None,
        }
    }
}

/// Initial descriptor for an optional index buffer in a [`MeshDescriptor`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client retained mesh index buffer descriptor.
pub struct MeshIndexInit<'a> {
    /// Index element encoding (`Uint16` or `Uint32`).
    pub format: IndexFormat,
    /// Total GPU buffer size in bytes (non-zero, multiple of 4 and `format.byte_size()`,
    /// `<= device.limits().max_buffer_size`).
    pub byte_len: u64,
    /// Optional initial index bytes written at offset 0 (`len <= byte_len`, multiple of 4).
    pub initial: Option<&'a [u8]>,
}

impl<'a> MeshIndexInit<'a> {
    /// Create an index buffer sized exactly to `bytes` and initialized with `bytes`.
    pub const fn from_bytes(format: IndexFormat, bytes: &'a [u8]) -> Self {
        Self {
            format,
            byte_len: bytes.len() as u64,
            initial: Some(bytes),
        }
    }

    /// Create an uninitialized GPU index buffer of `byte_len` bytes to be filled via
    /// [`MeshRef::write_indices`].
    pub const fn empty(format: IndexFormat, byte_len: u64) -> Self {
        Self {
            format,
            byte_len,
            initial: None,
        }
    }
}

/// Descriptor passed to [`crate::prelude::ClientRenderClientHost::create_mesh`] and
/// [`MeshRef::replace`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client retained mesh creation descriptor.
pub struct MeshDescriptor<'a> {
    /// One or more vertex/instance buffer bindings (`1..=MAX_VERTEX_BINDINGS`).
    pub vertex_buffers: &'a [MeshBufferInit<'a>],
    /// Optional index buffer.
    pub indices: Option<MeshIndexInit<'a>>,
}

impl<'a> MeshDescriptor<'a> {
    /// Describe a non-indexed mesh with `vertex_buffers`.
    pub const fn new(vertex_buffers: &'a [MeshBufferInit<'a>]) -> Self {
        Self {
            vertex_buffers,
            indices: None,
        }
    }

    /// Attach an index buffer to this descriptor.
    pub const fn with_indices(mut self, indices: MeshIndexInit<'a>) -> Self {
        self.indices = Some(indices);
        self
    }
}

/// Why a retained GPU [`MeshRef`] operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client retained mesh operation.
pub enum MeshError {
    /// No GPU renderer is available in this context.
    Unavailable,
    /// The handle was released, its owner mod was disabled, the world closed, or resources
    /// reloaded. Retired handles never become valid again.
    Stale,
    /// The calling mod does not own this mesh.
    NotOwned,
    /// Empty vertex buffer list, `> MAX_VERTEX_BINDINGS`, or binding index out of range.
    InvalidBinding,
    /// `write_indices` was called on a mesh that has no index buffer.
    NoIndexBuffer,
    /// A vertex buffer stride is zero or not a multiple of 4.
    InvalidStride,
    /// Byte offset, buffer size, or write length violates 4-byte, stride, or index alignment.
    InvalidAlignment,
    /// Buffer length is zero, or `byte_offset + data.len()` exceeds the buffer capacity.
    OutOfRange,
    /// Requested buffer size exceeds `device.limits().max_buffer_size`.
    ExceedsDeviceLimit,
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "GPU renderer is unavailable",
            Self::Stale => "mesh handle is stale or released",
            Self::NotOwned => "mesh handle is owned by another mod",
            Self::InvalidBinding => "invalid vertex buffer binding",
            Self::NoIndexBuffer => "mesh has no index buffer",
            Self::InvalidStride => "vertex buffer stride must be non-zero and a multiple of 4",
            Self::InvalidAlignment => "mesh buffer size, offset, or data length is misaligned",
            Self::OutOfRange => "mesh buffer write or size is out of range",
            Self::ExceedsDeviceLimit => "mesh buffer size exceeds device max_buffer_size",
        })
    }
}

impl std::error::Error for MeshError {}

impl crate::NoBackend for MeshError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// A live client context that can operate on retained GPU [`MeshRef`] handles.
pub trait MeshContext {
    /// The scoped engine access used by [`MeshRef`] methods.
    // hidden: internal trait method opening the retained-mesh backend for MeshRef.
    #[doc(hidden)]
    fn mesh_backend(&self) -> Option<MeshBackendRef<'_>> {
        None
    }
}

/// Opaque access to retained-mesh operations, not to the world or other client services.
// hidden: only the handle can open the mesh backend.
// exhaustive: private field; only constructed inside this crate.
// no wire: scoped backend borrow.
#[doc(hidden)]
pub struct MeshBackendRef<'a>(&'a dyn crate::prelude::ClientRenderClientBackend);

impl<H: crate::ClientHost + ?Sized> MeshContext for H {
    fn mesh_backend(&self) -> Option<MeshBackendRef<'_>> {
        self.backend().map(|backend| MeshBackendRef(backend.open()))
    }
}

/// Generational handle to retained GPU vertex and optional index buffers, created via
/// [`crate::prelude::ClientRenderClientHost::create_mesh`].
///
/// Drawing a [`MeshRef`] via [`crate::WorldDrawContext::draw_mesh`] records a small POD draw
/// command with zero CPU vertex copying per frame. Handles are owner-checked and retired
/// automatically on mod disable, world close, resource reload, or [`Self::release`].
// no wire: process-local generational handle to retained GPU buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the engine creates handles.
pub struct MeshRef(pub(crate) u64, pub(crate) u32);

impl MeshRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished mesh generation");
        }
        Ok(Self(generation, slot))
    }

    /// Partially or fully update vertex buffer `binding` starting at `byte_offset` (both
    /// `byte_offset` and `data.len()` must be 4-byte aligned and in bounds).
    pub fn write_vertices(
        self,
        host: &(impl MeshContext + ?Sized),
        binding: u8,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), MeshError> {
        host.mesh_backend()
            .ok_or(MeshError::Unavailable)?
            .0
            .mesh_write_vertices(self, binding, byte_offset, data)
    }

    /// Partially or fully update this mesh's index buffer starting at `byte_offset` (both
    /// `byte_offset` and `data.len()` must be 4-byte aligned and in bounds).
    pub fn write_indices(
        self,
        host: &(impl MeshContext + ?Sized),
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), MeshError> {
        host.mesh_backend()
            .ok_or(MeshError::Unavailable)?
            .0
            .mesh_write_indices(self, byte_offset, data)
    }

    /// Reallocate backing GPU buffer(s) in place without churning this handle. If `desc` is
    /// refused, the previous buffers remain intact.
    pub fn replace(
        self,
        host: &(impl MeshContext + ?Sized),
        desc: &MeshDescriptor<'_>,
    ) -> Result<(), MeshError> {
        host.mesh_backend()
            .ok_or(MeshError::Unavailable)?
            .0
            .mesh_replace(self, desc)
    }

    /// Immediately free this mesh's GPU buffers and retire the handle ([`MeshError::Stale`]
    /// thereafter).
    pub fn release(self, host: &(impl MeshContext + ?Sized)) -> Result<(), MeshError> {
        host.mesh_backend()
            .ok_or(MeshError::Unavailable)?
            .0
            .mesh_release(self)
    }
}

impl crate::__private::game::Fields for MeshRef {
    type Fields<'a> = crate::__private::game::ModelRefFields;
    fn fields(&self) -> Self::Fields<'_> {
        crate::__private::game::ModelRefFields {
            generation: self.0,
            slot: self.1,
        }
    }
}

/// Vertex or index range for [`MeshDraw`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client mesh draw range.
pub enum DrawRange {
    /// Non-indexed draw over vertex indices `start..end`. For [`PrimitiveTopology::Quads`] and
    /// [`PrimitiveTopology::Lines`], `end - start` must be a multiple of 4 and is expanded via
    /// the engine's shared quad index buffer; for [`PrimitiveTopology::TriangleFan`], it is
    /// expanded via the engine's shared fan index buffer.
    Vertices(std::ops::Range<u32>),
    /// Indexed draw over `indices` in the bound index buffer, adding `base_vertex` to each index.
    Indexed {
        /// Range of indices in the bound index buffer.
        indices: std::ops::Range<u32>,
        /// Signed value added to each index before fetching vertices.
        base_vertex: i32,
    },
}

/// Draw parameters for [`crate::WorldDrawContext::draw_mesh`] and
/// [`crate::WorldDrawContext::draw_vertices`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client mesh draw parameters.
pub struct MeshDraw {
    /// Vertex or index range to draw.
    pub range: DrawRange,
    /// Instance range to draw (`0..1` by default).
    pub instances: std::ops::Range<u32>,
}

impl MeshDraw {
    /// Non-indexed draw of `range` vertices with a single instance (`0..1`).
    pub const fn vertices(range: std::ops::Range<u32>) -> Self {
        Self {
            range: DrawRange::Vertices(range),
            instances: 0..1,
        }
    }

    /// Indexed draw of `indices` with `base_vertex` and a single instance (`0..1`).
    pub const fn indexed(indices: std::ops::Range<u32>, base_vertex: i32) -> Self {
        Self {
            range: DrawRange::Indexed {
                indices,
                base_vertex,
            },
            instances: 0..1,
        }
    }

    /// Set the instance range (`0..count`).
    pub const fn with_instances(mut self, instances: std::ops::Range<u32>) -> Self {
        self.instances = instances;
        self
    }
}

/// Maximum push-constant block size in bytes (`RenderPipeline.java:406`: `"Maximum push constant
/// size is 128 bytes"`). Backed portably by an inline std140 uniform slot in set 1.
pub const MAX_PUSH_CONSTANT_SIZE: u32 = 128;

/// Built-in vanilla set-0 uniform block, texel buffer, and sampler names declared in vanilla
/// 26.3's `net.minecraft.client.renderer.BindGroupLayouts` (including `Coeff0`..`Coeff1` for
/// `LevelRenderer.OIT_TRANSMITTANCE_TARGET_COUNT = OIT_COEFFICIENT_COUNT / 4 = 2`).
/// Mod-declared set-1 bindings may not reuse any of these names.
pub const VANILLA_BIND_GROUP_LAYOUT_NAMES: &[&str] = &[
    "DynamicTransforms",
    "Projection",
    "ChunkSection",
    "TerrainUniform",
    "Fog",
    "Globals",
    "Lighting",
    "Sampler0",
    "Sampler1",
    "Sampler2",
    "CloudInfo",
    "CloudFaces",
    "DissolveMaskSampler",
    "InSampler",
    "LightmapInfo",
    "SpriteAnimationInfo",
    "Sprite",
    "CurrentSprite",
    "NextSprite",
    "GlintSampler",
    "DepthBoundsSampler",
    "Coeff0",
    "Coeff1",
];

/// Texture filtering mode for a mod-declared [`BindingKind::CombinedImageSampler`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client sampler filter mode.
pub enum SamplerFilter {
    /// Nearest-neighbour texel sampling.
    #[default]
    Nearest,
    /// Bilinear texel filtering.
    Linear,
}

/// Texture coordinate addressing mode for a mod-declared [`BindingKind::CombinedImageSampler`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client sampler addressing mode.
pub enum SamplerAddress {
    /// Clamp texture coordinates to `[0, 1]`.
    #[default]
    ClampToEdge,
    /// Wrap texture coordinates modulo 1.
    Repeat,
    /// Mirror texture coordinates on alternating tiles.
    MirroredRepeat,
}

/// How a mod-declared [`BindingKind::CombinedImageSampler`] reads its texture: vanilla's
/// `GpuDevice.createSampler(addressU, addressV, minFilter, magFilter, maxAnisotropy, maxLod)`
/// without the anisotropy (it stays out until a mod needs it).
///
/// Like vanilla's, a sampler has no separate mipmap filter and no minimum level of detail. Vanilla's
/// backends blend between mip levels linearly whenever `maxLod` allows more than the base level
/// (`GlSampler`: `NEAREST` becomes `NEAREST_MIPMAP_LINEAR`, `LINEAR` becomes
/// `LINEAR_MIPMAP_LINEAR`; Vulkan: the mipmap mode is linear when `maxLod > 0.25`), and so does
/// this one. [`Sampler::new`] has no `max_lod`, so it blends between levels.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client sampler descriptor.
pub struct Sampler {
    /// Addressing of the `u` texture coordinate.
    pub address_u: SamplerAddress,
    /// Addressing of the `v` texture coordinate.
    pub address_v: SamplerAddress,
    /// Filter when the texture is minified.
    pub min: SamplerFilter,
    /// Filter when the texture is magnified.
    pub mag: SamplerFilter,
    /// The largest mip level of detail the sampler reads (`maxLod`): `Some(0.0)` reads only the
    /// base level. A value above `0.25` blends linearly between the two nearest levels, as
    /// vanilla does. `None` is [`Sampler::new`]'s: no limit, vanilla's absent `maxLod` (1000),
    /// so it blends between levels too. A sampler that blends is a filtering one: to sample a
    /// depth target, or an `R32Float` target where
    /// [`RenderCapabilities::float32_filterable`] is false, set `Some(0.0)` with
    /// [`SamplerFilter::Nearest`]. A value that is negative or not finite is refused with
    /// [`BindGroupLayoutError::BadMaxLod`] when the layout is built.
    pub max_lod: Option<f32>,
}

impl Sampler {
    /// One `filter` for minification and magnification, one `address` mode for both axes, and
    /// no `max_lod`, which blends between mip levels as vanilla's sampler does.
    pub fn new(filter: SamplerFilter, address: SamplerAddress) -> Self {
        Self {
            address_u: address,
            address_v: address,
            min: filter,
            mag: filter,
            max_lod: None,
        }
    }
}

impl PartialEq for Sampler {
    fn eq(&self, other: &Self) -> bool {
        self.address_u == other.address_u
            && self.address_v == other.address_v
            && self.min == other.min
            && self.mag == other.mag
            && self.max_lod.map(f32::to_bits) == other.max_lod.map(f32::to_bits)
    }
}

impl Eq for Sampler {}

impl std::hash::Hash for Sampler {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.address_u.hash(state);
        self.address_v.hash(state);
        self.min.hash(state);
        self.mag.hash(state);
        self.max_lod.map(f32::to_bits).hash(state);
    }
}

/// Integer texel format for a mod-declared [`BindingKind::TexelBuffer`]
/// (`UniformType.TEXEL_BUFFER`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client texel buffer format.
pub enum TexelBufferFormat {
    /// Signed 8-bit integer per texel (`GpuFormat.R8_SINT`), read via `isamplerBuffer`.
    R8Sint,
    /// Unsigned 8-bit integer per texel (`GpuFormat.R8_UINT`), read via `usamplerBuffer`.
    R8Uint,
    /// Signed 32-bit integer per texel (`GpuFormat.R32_SINT`), read via `isamplerBuffer`.
    R32Sint,
    /// Unsigned 32-bit integer per texel (`GpuFormat.R32_UINT`), read via `usamplerBuffer`.
    R32Uint,
}

impl TexelBufferFormat {
    /// Whether this format is signed (`isamplerBuffer`) vs unsigned (`usamplerBuffer`).
    pub const fn is_signed(self) -> bool {
        matches!(self, Self::R8Sint | Self::R32Sint)
    }

    /// Input byte width of one texel element.
    pub const fn element_bytes(self) -> usize {
        match self {
            Self::R8Sint | Self::R8Uint => 1,
            Self::R32Sint | Self::R32Uint => 4,
        }
    }

    /// GLSL sampler type name matching this texel buffer format.
    pub const fn glsl_type(self) -> &'static str {
        if self.is_signed() {
            "isamplerBuffer"
        } else {
            "usamplerBuffer"
        }
    }
}

/// Resource kind for one entry in a mod-declared set-1 [`BindGroupLayout`]
/// (`com.mojang.renderpearl.api.pipeline.UniformType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client bind-group entry kind.
pub enum BindingKind {
    /// A std140 uniform buffer block (`UniformType.UNIFORM_BUFFER`), bound via either a retained
    /// [`UniformBufferRef`] or inline per-draw bytes ([`UniformValue::Inline`]).
    UniformBuffer {
        /// Minimum byte size required for this uniform block.
        min_size: u32,
    },
    /// A combined 2D image + sampler (`UniformType.COMBINED_IMAGE_SAMPLER`), bound via
    /// [`TextureSource`].
    CombinedImageSampler {
        /// Addressing, filtering and mip range.
        sampler: Sampler,
    },
    /// An integer texel buffer (`UniformType.TEXEL_BUFFER`), backed by a read-only storage
    /// buffer and rewritten `texelFetch` calls.
    TexelBuffer {
        /// Integer texel format.
        format: TexelBufferFormat,
    },
    /// A std430 storage buffer block (`layout(std430) [readonly] buffer Name { ... };`), bound
    /// to a range of a retained [`StorageBufferRef`] ([`BindingValue::StorageBuffer`]). The
    /// GLSL block's `readonly` qualifier must agree with `read_only`. Writes are gated as
    /// `wgpu` gates them: a vertex-stage write needs `VERTEX_WRITABLE_STORAGE`, and a pipeline
    /// that uses a writable block in the vertex stage on a device without it is refused at
    /// registration. Like [`BindingKind::UniformBuffer`], the block declares its minimum byte
    /// size: a bound range shorter than `min_size` is refused with
    /// [`BufferError::BelowMinSize`] instead of failing inside the render pass.
    StorageBuffer {
        /// Whether shaders only read the block.
        read_only: bool,
        /// Minimum byte size of a range bound here; at least the shader's required minimum
        /// (for example 16 for a `vec4[]` block). Must be non-zero and at most the device's
        /// `max_storage_buffer_binding_size`.
        min_size: u32,
    },
    /// A single mip level of a mod-owned offscreen [`RenderTargetRef`] colour attachment bound
    /// as a storage image (`layout(<format>) [readonly|writeonly] uniform image2D Name;`), bound
    /// with [`BindingValue::StorageImage`]. The shader's `layout(<format>)` names the texel
    /// format, which must be the target's own, and its `readonly`/`writeonly` qualifiers must
    /// agree with `access`. The target must have been created with
    /// [`RenderTargetDescriptor::storage`] covering `access`, which is where an unsupported
    /// format or access is refused: not here, and not when a draw binds it.
    StorageImage {
        /// How shaders use the image.
        access: StorageImageAccess,
    },
}

/// How a shader uses a storage image ([`BindingKind::StorageImage`],
/// [`RenderTargetDescriptor::storage`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client storage image access mode.
pub enum StorageImageAccess {
    /// The shader only writes (`writeonly`); every supported format allows it.
    WriteOnly,
    /// The shader only reads (`readonly`).
    ReadOnly,
    /// The shader reads and writes; only formats the adapter reports read-write storage for
    /// ([`RenderCapabilities::read_write_storage`]) allow it.
    ReadWrite,
}

/// One named binding in a mod-declared set-1 [`BindGroupLayout`]
/// (`BindGroupLayout.UniformDescription`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client bind-group entry declaration.
pub struct BindEntry {
    /// GLSL block or sampler identifier.
    pub name: String,
    /// Resource kind and fixed sampler/format parameters.
    pub kind: BindingKind,
}

impl BindEntry {
    /// Construct a new bind-group layout entry.
    pub fn new(name: impl Into<String>, kind: BindingKind) -> Self {
        Self {
            name: name.into(),
            kind,
        }
    }
}

/// Why a [`BindGroupLayout`] or push-constant declaration was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client bind-group layout declaration.
pub enum BindGroupLayoutError {
    /// Binding name is not an ASCII GLSL identifier (a letter or `_`, then letters, digits or `_`).
    InvalidName(String),
    /// Two bindings in the same pipeline share a name (`BindGroupLayout.ensureCompatible`).
    DuplicateName(String),
    /// Binding name collides with a vanilla set-0 name from `BindGroupLayouts.java` (or
    /// `SpriteRect` on a `.textured(...)` pipeline).
    ReservedName(String),
    /// A `UniformBuffer` entry declared `min_size == 0`.
    ZeroUniformSize(String),
    /// A `StorageBuffer` entry declared `min_size == 0`.
    ZeroStorageSize(String),
    /// A `CombinedImageSampler` entry's [`Sampler::max_lod`] is negative or not finite.
    BadMaxLod(String),
    /// `push_constant_size` exceeds [`MAX_PUSH_CONSTANT_SIZE`] (`RenderPipeline.java:406`).
    PushConstantTooLarge(u32),
    /// `push_constant_size` is not a multiple of 4 bytes.
    PushConstantUnaligned(u32),
}

impl std::fmt::Display for BindGroupLayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName(name) => write!(f, "invalid GLSL binding name `{name}`"),
            Self::DuplicateName(name) => write!(f, "duplicate binding name `{name}`"),
            Self::ReservedName(name) => write!(
                f,
                "binding name `{name}` is reserved by vanilla set-0 BindGroupLayouts"
            ),
            Self::ZeroUniformSize(name) => {
                write!(f, "uniform buffer `{name}` must have min_size > 0")
            }
            Self::ZeroStorageSize(name) => {
                write!(f, "storage buffer `{name}` must have min_size > 0")
            }
            Self::BadMaxLod(name) => {
                write!(
                    f,
                    "sampler `{name}` must have a finite max_lod of at least 0"
                )
            }
            Self::PushConstantTooLarge(size) => write!(
                f,
                "push_constant_size {size} exceeds maximum {MAX_PUSH_CONSTANT_SIZE} bytes"
            ),
            Self::PushConstantUnaligned(size) => {
                write!(f, "push_constant_size {size} must be a multiple of 4 bytes")
            }
        }
    }
}

impl std::error::Error for BindGroupLayoutError {}

/// Mod-declared set-1 bind-group layout on a [`RenderPipeline`]
/// (`com.mojang.renderpearl.api.pipeline.BindGroupLayout`).
///
/// Set 0 holds vanilla's shared uniform blocks and samplers (`BindGroupLayouts.java`); set 1
/// holds every entry declared here in deterministic declaration order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client set-1 bind-group layout descriptor.
pub struct BindGroupLayout {
    /// Declared set-1 binding entries in deterministic binding order.
    pub entries: Vec<BindEntry>,
}

impl BindGroupLayout {
    /// Start building a set-1 [`BindGroupLayout`].
    pub fn builder() -> BindGroupLayoutBuilder {
        BindGroupLayoutBuilder::default()
    }

    /// Whether this layout declares no custom set-1 bindings.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The declared set-1 entries in binding order.
    pub fn entries(&self) -> &[BindEntry] {
        &self.entries
    }

    /// Validate every entry name, uniqueness (`BindGroupLayout.ensureCompatible`), vanilla
    /// `BindGroupLayouts.java` reservations (plus `SpriteRect` when `textured` is true), and
    /// positive uniform and storage `min_size`, and a finite, non-negative sampler `max_lod`.
    pub fn validate(&self, textured: bool) -> Result<(), BindGroupLayoutError> {
        let mut seen = std::collections::BTreeSet::new();
        for entry in &self.entries {
            if !is_valid_glsl_identifier(&entry.name) {
                return Err(BindGroupLayoutError::InvalidName(entry.name.clone()));
            }
            if VANILLA_BIND_GROUP_LAYOUT_NAMES.contains(&entry.name.as_str())
                || (textured && entry.name == "SpriteRect")
            {
                return Err(BindGroupLayoutError::ReservedName(entry.name.clone()));
            }
            if !seen.insert(entry.name.as_str()) {
                return Err(BindGroupLayoutError::DuplicateName(entry.name.clone()));
            }
            if let BindingKind::UniformBuffer { min_size: 0 } = entry.kind {
                return Err(BindGroupLayoutError::ZeroUniformSize(entry.name.clone()));
            }
            if let BindingKind::StorageBuffer { min_size: 0, .. } = entry.kind {
                return Err(BindGroupLayoutError::ZeroStorageSize(entry.name.clone()));
            }
            if let BindingKind::CombinedImageSampler { sampler } = entry.kind
                && sampler
                    .max_lod
                    .is_some_and(|lod| !(lod >= 0.0 && lod.is_finite()))
            {
                return Err(BindGroupLayoutError::BadMaxLod(entry.name.clone()));
            }
        }
        Ok(())
    }
}

/// Builder for a set-1 [`BindGroupLayout`] (`BindGroupLayout.Builder`).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
// no wire: local client bind-group layout builder.
pub struct BindGroupLayoutBuilder {
    entries: Vec<BindEntry>,
}

impl BindGroupLayoutBuilder {
    /// Add a std140 uniform buffer entry (`UniformType.UNIFORM_BUFFER`).
    pub fn uniform_buffer(mut self, name: impl Into<String>, min_size: u32) -> Self {
        self.entries.push(BindEntry {
            name: name.into(),
            kind: BindingKind::UniformBuffer { min_size },
        });
        self
    }

    /// Add a combined 2D image sampler entry (`UniformType.COMBINED_IMAGE_SAMPLER`), sampled as
    /// `sampler` describes (vanilla's `createSampler(addressU, addressV, minFilter, magFilter,
    /// maxAnisotropy, maxLod)` without the anisotropy).
    pub fn sampler(mut self, name: impl Into<String>, sampler: Sampler) -> Self {
        self.entries.push(BindEntry {
            name: name.into(),
            kind: BindingKind::CombinedImageSampler { sampler },
        });
        self
    }

    /// Add an integer texel buffer entry (`UniformType.TEXEL_BUFFER`).
    pub fn texel_buffer(mut self, name: impl Into<String>, format: TexelBufferFormat) -> Self {
        self.entries.push(BindEntry {
            name: name.into(),
            kind: BindingKind::TexelBuffer { format },
        });
        self
    }

    /// Append a pre-constructed [`BindEntry`].
    pub fn entry(mut self, entry: BindEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// Validate and build the [`BindGroupLayout`].
    pub fn build(self) -> Result<BindGroupLayout, BindGroupLayoutError> {
        let layout = BindGroupLayout {
            entries: self.entries,
        };
        layout.validate(false)?;
        Ok(layout)
    }
}

/// Why a retained GPU uniform/storage buffer operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client GPU buffer operation.
pub enum BufferError {
    /// No GPU renderer is available in this context.
    Unavailable,
    /// The handle was released, its owner mod was disabled, the world closed, or resources
    /// reloaded. Retired handles never become valid again.
    Stale,
    /// The calling mod does not own this buffer.
    NotOwned,
    /// Buffer byte length was zero.
    Empty,
    /// Byte offset or byte length is not 4-byte aligned (`wgpu::COPY_BUFFER_ALIGNMENT`), or a
    /// storage buffer range's offset is not a multiple of the device's
    /// `min_storage_buffer_offset_alignment`.
    InvalidAlignment,
    /// `byte_offset + bytes.len()` exceeds the buffer's capacity.
    OutOfRange,
    /// A readback of a storage buffer created without
    /// [`StorageBufferDescriptor::copy_src`](crate::StorageBufferDescriptor::copy_src).
    NotReadable,
    /// A readback was requested outside a frame-graph compute node's `record` callback, the
    /// only place a copy is recorded.
    NotInFrame,
    /// Requested buffer size exceeds the active device's uniform/buffer binding limit.
    ExceedsDeviceLimit,
    /// A storage buffer range is longer than the device's `max_storage_buffer_binding_size`.
    ExceedsBindingLimit,
    /// A storage buffer range is shorter than the `min_size` its [`BindingKind::StorageBuffer`]
    /// entry declares.
    BelowMinSize,
}

impl std::fmt::Display for BufferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "GPU renderer is unavailable",
            Self::Stale => "buffer handle is stale or released",
            Self::NotOwned => "buffer handle is owned by another mod",
            Self::Empty => "buffer size must be non-zero",
            Self::InvalidAlignment => "buffer offset and byte length must be 4-byte aligned",
            Self::OutOfRange => "buffer write exceeds buffer capacity",
            Self::NotReadable => "storage buffer was not created with copy_src",
            Self::NotInFrame => {
                "a readback is recorded only inside a compute node's record callback"
            }
            Self::ExceedsDeviceLimit => "buffer size exceeds active device limit",
            Self::ExceedsBindingLimit => "buffer range exceeds the device's storage binding limit",
            Self::BelowMinSize => "buffer range is shorter than the binding's min_size",
        })
    }
}

impl std::error::Error for BufferError {}

impl crate::NoBackend for BufferError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// A live client context that can operate on retained GPU [`UniformBufferRef`] handles.
pub trait UniformBufferContext {
    /// The scoped engine access used by [`UniformBufferRef`] methods.
    // hidden: internal trait method opening the retained-buffer backend for UniformBufferRef.
    #[doc(hidden)]
    fn uniform_buffer_backend(&self) -> Option<UniformBufferBackendRef<'_>> {
        None
    }
}

/// Opaque access to retained uniform-buffer operations, not to the world or other client services.
// hidden: only the handle can open the uniform-buffer backend.
// exhaustive: private field; only constructed inside this crate.
// no wire: scoped backend borrow.
#[doc(hidden)]
pub struct UniformBufferBackendRef<'a>(&'a dyn crate::prelude::ClientRenderClientBackend);

impl<H: crate::ClientHost + ?Sized> UniformBufferContext for H {
    fn uniform_buffer_backend(&self) -> Option<UniformBufferBackendRef<'_>> {
        self.backend()
            .map(|backend| UniformBufferBackendRef(backend.open()))
    }
}

/// Generational handle to a retained GPU std140 uniform buffer, created via
/// [`crate::prelude::ClientRenderClientHost::create_uniform_buffer`].
///
/// Handles are owner-checked and retired automatically on mod disable, world close, resource
/// reload, or [`Self::release`].
// no wire: process-local generational handle to a retained GPU uniform buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the engine creates handles.
pub struct UniformBufferRef(pub(crate) u64, pub(crate) u32);

impl UniformBufferRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished uniform buffer generation");
        }
        Ok(Self(generation, slot))
    }

    /// Partially or fully update this uniform buffer starting at `byte_offset` (both
    /// `byte_offset` and `bytes.len()` must be 4-byte aligned and in bounds).
    pub fn write(
        self,
        host: &(impl UniformBufferContext + ?Sized),
        byte_offset: u32,
        bytes: &[u8],
    ) -> Result<(), BufferError> {
        host.uniform_buffer_backend()
            .ok_or(BufferError::Unavailable)?
            .0
            .uniform_buffer_write(self, byte_offset, bytes)
    }

    /// Immediately free this uniform buffer and retire the handle ([`BufferError::Stale`]
    /// thereafter).
    pub fn release(self, host: &(impl UniformBufferContext + ?Sized)) -> Result<(), BufferError> {
        host.uniform_buffer_backend()
            .ok_or(BufferError::Unavailable)?
            .0
            .uniform_buffer_release(self)
    }
}

impl crate::__private::game::Fields for UniformBufferRef {
    type Fields<'a> = crate::__private::game::ModelRefFields;
    fn fields(&self) -> Self::Fields<'_> {
        crate::__private::game::ModelRefFields {
            generation: self.0,
            slot: self.1,
        }
    }
}

/// Value bound to a [`BindingKind::UniformBuffer`] entry in [`DrawBindings`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client per-draw uniform buffer binding.
pub enum UniformValue<'a> {
    /// A retained GPU uniform buffer created via
    /// [`crate::prelude::ClientRenderClientHost::create_uniform_buffer`].
    Buffer(UniformBufferRef),
    /// Inline per-draw std140 uniform bytes, sub-allocated from a per-frame ring buffer with
    /// dynamic offsets, so a draw creates no GPU buffer of its own.
    Inline(&'a [u8]),
}

/// Texture bound to a [`BindingKind::CombinedImageSampler`] entry in [`DrawBindings`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client per-draw texture binding source.
pub enum TextureSource<'a> {
    /// A sprite on the block/item atlas (`minecraft:block/stone`, `mymod:block/custom`).
    AtlasSprite(&'a Identifier),
    /// A live dynamic texture registered via
    /// [`crate::prelude::ClientRenderClientHost::create_dynamic_texture`].
    DynamicTexture(&'a Identifier),
    /// A standalone PNG texture asset (`assets/<ns>/textures/<path>.png` or exact `.png` asset
    /// path) loaded from the active resource packs.
    StandaloneTexture(&'a Identifier),
    /// The active world pass's depth texture (usable only when the pipeline does not write
    /// depth in the same pass).
    WorldDepth,
    /// Vanilla's active 16x16 lightmap texture.
    Lightmap,
    /// Colour attachment `attachment` (`0..color_targets.len()`) of a mod-owned offscreen
    /// [`RenderTargetRef`] created via
    /// [`crate::prelude::ClientRenderClientHost::create_render_target`].
    RenderTargetColor(RenderTargetRef, u8),
    /// Depth aspect of a mod-owned offscreen [`RenderTargetRef`] (must be bound to a
    /// non-filtering [`SamplerFilter::Nearest`] sampler).
    RenderTargetDepth(RenderTargetRef),
    /// A mip range of a colour attachment of a mod-owned offscreen [`RenderTargetRef`]
    /// (`GpuDevice.createTextureView(texture, baseMipLevel, mipLevels)`). A draw that samples a
    /// level is skipped when a draw in the same pass binds that level as a storage image it
    /// writes.
    RenderTargetColorMip(RenderTargetMipView),
    /// Colour attachment `attachment` (`0..color_targets.len()`) of a transient per-frame target
    /// allocated via [`crate::FrameGraphContext::create_transient`].
    TransientTargetColor(crate::TransientTargetId, u8),
    /// Depth aspect of a transient per-frame target allocated via
    /// [`crate::FrameGraphContext::create_transient`] (must be bound to a non-filtering
    /// [`SamplerFilter::Nearest`] sampler).
    TransientTargetDepth(crate::TransientTargetId),
}

/// A view of a mip range of a render target colour attachment (vanilla's `GpuTextureView`:
/// `baseMipLevel()` and `mipLevels()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render target mip view.
pub struct RenderTargetMipView {
    /// The render target.
    pub target: RenderTargetRef,
    /// The colour attachment (`0..color_targets.len()`).
    pub attachment: u8,
    /// First mip level of the view.
    pub base_mip: u32,
    /// Number of mip levels from `base_mip`: at least 1, and `base_mip + mip_levels` at most the
    /// target's [`RenderTargetDescriptor::mip_levels`].
    pub mip_levels: u32,
}

impl RenderTargetMipView {
    /// `mip_levels` levels of colour attachment `attachment` of `target`, from `base_mip`.
    pub const fn new(
        target: RenderTargetRef,
        attachment: u8,
        base_mip: u32,
        mip_levels: u32,
    ) -> Self {
        Self {
            target,
            attachment,
            base_mip,
            mip_levels,
        }
    }

    /// The single level `mip` of colour attachment `attachment` of `target`.
    pub const fn single(target: RenderTargetRef, attachment: u8, mip: u32) -> Self {
        Self::new(target, attachment, mip, 1)
    }
}

/// Colour attachment format for a mod-owned offscreen [`RenderTargetRef`].
///
/// All listed formats are renderable and sampleable across Linux (Vulkan), Windows (DX12/Vulkan),
/// and Apple-chip macOS (Metal) without optional device extensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client offscreen render target colour format.
pub enum RenderTargetFormat {
    /// 8-bit unsigned normalized RGBA (`Rgba8Unorm`, filterable).
    Rgba8Unorm,
    /// 16-bit floating-point RGBA (`Rgba16Float`, filterable).
    Rgba16Float,
    /// 32-bit single-channel floating-point (`R32Float`; bind with [`SamplerFilter::Nearest`]
    /// unless the active adapter supports float32 filtering).
    R32Float,
}

/// Depth/stencil attachment format for a mod-owned offscreen [`RenderTargetRef`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client offscreen render target depth format.
pub enum RenderTargetDepth {
    /// No depth or stencil attachment.
    #[default]
    None,
    /// 32-bit floating-point depth attachment (`Depth32Float`).
    Depth32Float,
    /// 24-bit+ depth with 8-bit stencil attachment (`Depth24PlusStencil8`).
    Depth24PlusStencil8,
}

/// The optional device features mod render code can ask about
/// ([`crate::prelude::ClientRenderClientHost::render_capabilities`]): one read, answered from
/// the adapter the game runs on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render capabilities.
pub struct RenderCapabilities {
    /// Whether `R32Float` textures can be sampled with a linear filter. Without it an
    /// `R32Float` render target is bound with [`SamplerFilter::Nearest`] only.
    pub float32_filterable: bool,
    /// The render target colour formats that support [`StorageImageAccess::ReadWrite`] storage
    /// images on this adapter. Whether a format supports [`StorageImageAccess::WriteOnly`] or
    /// [`StorageImageAccess::ReadOnly`] is also the adapter's answer: creating the target
    /// refuses with [`RenderTargetError::UnsupportedStorage`] when it does not.
    pub read_write_storage: Vec<RenderTargetFormat>,
    /// Whether the adapter runs compute shaders that use subgroup operations
    /// (`GL_KHR_shader_subgroup_*`). A [`crate::ComputePipeline`] that uses them is refused with
    /// [`crate::ComputePipelineError::UnsupportedFeature`] without it.
    pub subgroup: bool,
    /// Whether an indirect draw's `first_instance` may be non-zero on this adapter
    /// (`INDIRECT_FIRST_INSTANCE`).
    pub indirect_first_instance: bool,
    /// Whether an [`IndirectDraw`](crate::IndirectDraw) may read its command count from a GPU
    /// buffer ([`IndirectDraw::with_count_buffer`](crate::IndirectDraw::with_count_buffer);
    /// `multi_draw_indirect_count`, Vulkan's `vkCmdDrawIndexedIndirectCount`). False where the
    /// adapter lacks `MULTI_DRAW_INDIRECT_COUNT`, and on DirectX 12, whose count draws leave an
    /// indexed command's `base_vertex` out of the vertex fetch.
    pub multi_draw_indirect_count: bool,
}

/// Why a mod-owned offscreen [`RenderTargetRef`] operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client GPU render target operation.
pub enum RenderTargetError {
    /// No GPU renderer is available in this context.
    Unavailable,
    /// The handle was released or its owner mod was unloaded. Retired handles never become
    /// valid again.
    Stale,
    /// The calling mod does not own this render target.
    NotOwned,
    /// Width or height was zero or exceeded `device.limits().max_texture_dimension_2d`.
    InvalidSize,
    /// `color_targets.len()` exceeded [`MAX_COLOR_TARGETS`] (`8`).
    TooManyColorTargets,
    /// Both `color_targets` was empty and `depth` was [`RenderTargetDepth::None`].
    NoAttachments,
    /// `mip_levels` was 0 or more than [`RenderTargetDescriptor::max_mip_levels`] of the size.
    InvalidMipLevels,
    /// [`RenderTargetDescriptor::storage`] was set on a target with no colour attachment (depth
    /// attachments cannot be storage textures).
    StorageNeedsColorTarget,
    /// A colour format of the target does not support the requested storage access on this
    /// adapter ([`RenderCapabilities`]).
    UnsupportedStorage,
    /// The GPU device refused the allocation (out of memory or a validation failure).
    Device,
}

impl std::fmt::Display for RenderTargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "GPU renderer is unavailable",
            Self::Stale => "render target handle is stale or released",
            Self::NotOwned => "render target handle is owned by another mod",
            Self::InvalidSize => {
                "render target size must be >= 1 and <= device max texture dimension"
            }
            Self::TooManyColorTargets => "render target exceeds MAX_COLOR_TARGETS (8)",
            Self::NoAttachments => {
                "render target must declare at least one colour or depth attachment"
            }
            Self::InvalidMipLevels => "mip levels must be 1..=max_mip_levels of the target size",
            Self::StorageNeedsColorTarget => "storage images need a colour attachment",
            Self::UnsupportedStorage => {
                "this adapter does not support that storage access for the colour format"
            }
            Self::Device => "the GPU device refused the render target allocation",
        })
    }
}

impl std::error::Error for RenderTargetError {}

impl crate::NoBackend for RenderTargetError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Descriptor for allocating a mod-owned offscreen [`RenderTargetRef`] via
/// [`crate::prelude::ClientRenderClientHost::create_render_target`] (vanilla 26.3
/// `com.mojang.blaze3d.resource.RenderTargetDescriptor` / `TextureTarget`).
///
/// Mod-owned targets retain their contents across frames until reallocated by
/// [`RenderTargetRef::resize`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client offscreen render target descriptor.
pub struct RenderTargetDescriptor {
    /// Target dimensions `[width, height]` in pixels (`1..=device.limits().max_texture_dimension_2d`).
    pub size: [u32; 2],
    /// Colour attachments (`0..=MAX_COLOR_TARGETS`). Depth-only targets leave this empty and set
    /// [`Self::depth`] to a non-[`RenderTargetDepth::None`] format.
    pub color_targets: Vec<RenderTargetFormat>,
    /// Optional depth/stencil attachment.
    pub depth: RenderTargetDepth,
    /// Mip levels of every colour attachment (vanilla's `mipLevels` of `createTexture`): at
    /// least 1 and at most [`Self::max_mip_levels`] of `size`. Default 1. A depth attachment
    /// always has a single level. A resize keeps the count.
    pub mip_levels: u32,
    /// The storage access the colour attachments are created for (`None`, the default, makes no
    /// storage image possible). Refused with [`RenderTargetError::UnsupportedStorage`] unless
    /// every colour format's `TextureFormatFeatures` on this adapter allow `STORAGE_BINDING`
    /// and the matching `STORAGE_READ_ONLY`, `STORAGE_WRITE_ONLY` or `STORAGE_READ_WRITE` flag.
    pub storage: Option<StorageImageAccess>,
}

impl RenderTargetDescriptor {
    /// The most mip levels a `size` target can have: `32 - max(width, height).leading_zeros()`
    /// (0 for a zero side).
    pub const fn max_mip_levels(size: [u32; 2]) -> u32 {
        let longest = if size[0] > size[1] { size[0] } else { size[1] };
        32 - longest.leading_zeros()
    }

    /// Start an empty descriptor for `[width, height]` with no attachments yet.
    pub fn new(size: [u32; 2]) -> Self {
        Self {
            size,
            color_targets: Vec::new(),
            depth: RenderTargetDepth::None,
            mip_levels: 1,
            storage: None,
        }
    }

    /// Convenience descriptor for a single [`RenderTargetFormat::Rgba8Unorm`] colour attachment
    /// and the given `depth` format.
    pub fn rgba8(size: [u32; 2], depth: RenderTargetDepth) -> Self {
        Self {
            size,
            color_targets: vec![RenderTargetFormat::Rgba8Unorm],
            depth,
            mip_levels: 1,
            storage: None,
        }
    }

    /// Append one colour attachment format.
    pub fn color_target(mut self, format: RenderTargetFormat) -> Self {
        self.color_targets.push(format);
        self
    }

    /// Replace the colour attachments list.
    pub fn color_targets(mut self, formats: impl IntoIterator<Item = RenderTargetFormat>) -> Self {
        self.color_targets = formats.into_iter().collect();
        self
    }

    /// Set the depth/stencil attachment format.
    pub fn depth(mut self, depth: RenderTargetDepth) -> Self {
        self.depth = depth;
        self
    }

    /// Give every colour attachment `mip_levels` mip levels.
    pub fn mip_levels(mut self, mip_levels: u32) -> Self {
        self.mip_levels = mip_levels;
        self
    }

    /// Create the colour attachments for storage image `access` ([`BindingKind::StorageImage`]).
    pub fn storage(mut self, access: StorageImageAccess) -> Self {
        self.storage = Some(access);
        self
    }

    /// Validate size, colour attachment count, non-empty attachments, mip levels and the
    /// storage request's attachment against `max_texture_dimension_2d`.
    pub fn validate(&self, max_texture_dimension_2d: u32) -> Result<(), RenderTargetError> {
        let [w, h] = self.size;
        if w == 0 || h == 0 || w > max_texture_dimension_2d || h > max_texture_dimension_2d {
            return Err(RenderTargetError::InvalidSize);
        }
        if self.color_targets.len() > MAX_COLOR_TARGETS {
            return Err(RenderTargetError::TooManyColorTargets);
        }
        if self.color_targets.is_empty() && self.depth == RenderTargetDepth::None {
            return Err(RenderTargetError::NoAttachments);
        }
        if self.mip_levels == 0 || self.mip_levels > Self::max_mip_levels(self.size) {
            return Err(RenderTargetError::InvalidMipLevels);
        }
        if self.storage.is_some() && self.color_targets.is_empty() {
            return Err(RenderTargetError::StorageNeedsColorTarget);
        }
        Ok(())
    }
}

/// A live client context that can operate on mod-owned offscreen [`RenderTargetRef`] handles.
pub trait RenderTargetContext {
    /// The scoped engine access used by [`RenderTargetRef`] methods.
    // hidden: internal trait method opening the render-target backend for RenderTargetRef.
    #[doc(hidden)]
    fn render_target_backend(&self) -> Option<RenderTargetBackendRef<'_>> {
        None
    }
}

/// Opaque access to offscreen render-target operations, not to the world or other client services.
// hidden: only the handle can open the render-target backend.
// exhaustive: private field; only constructed inside this crate.
// no wire: scoped backend borrow.
#[doc(hidden)]
pub struct RenderTargetBackendRef<'a>(&'a dyn crate::prelude::ClientRenderClientBackend);

impl<H: crate::ClientHost + ?Sized> RenderTargetContext for H {
    fn render_target_backend(&self) -> Option<RenderTargetBackendRef<'_>> {
        self.backend()
            .map(|backend| RenderTargetBackendRef(backend.open()))
    }
}

/// Generational handle to a mod-owned offscreen render target, created via
/// [`crate::prelude::ClientRenderClientHost::create_render_target`].
///
/// Targets retain their contents across frames until resized. Handles are owner-checked. A target survives a resource reload and a world close, and is freed by
/// [`Self::release`] or when its mod unloads.
// no wire: process-local generational handle to a retained GPU render target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the engine creates handles.
pub struct RenderTargetRef(pub(crate) u64, pub(crate) u32);

impl RenderTargetRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished render target generation");
        }
        Ok(Self(generation, slot))
    }

    /// Resize this render target to `size` (`[width, height]`, each `>= 1` and
    /// `<= device.limits().max_texture_dimension_2d`).
    ///
    /// Calling `resize` with the target's current dimensions is a no-op that preserves existing
    /// attachment contents.
    pub fn resize(
        self,
        host: &(impl RenderTargetContext + ?Sized),
        size: [u32; 2],
    ) -> Result<(), RenderTargetError> {
        host.render_target_backend()
            .ok_or(RenderTargetError::Unavailable)?
            .0
            .render_target_resize(self, size)
    }

    /// Current `[width, height]` dimensions of this render target in pixels.
    pub fn size(
        self,
        host: &(impl RenderTargetContext + ?Sized),
    ) -> Result<[u32; 2], RenderTargetError> {
        host.render_target_backend()
            .ok_or(RenderTargetError::Unavailable)?
            .0
            .render_target_size(self)
    }

    /// Free this render target's GPU textures and retire the handle
    /// ([`RenderTargetError::Stale`] thereafter).
    pub fn release(
        self,
        host: &(impl RenderTargetContext + ?Sized),
    ) -> Result<(), RenderTargetError> {
        host.render_target_backend()
            .ok_or(RenderTargetError::Unavailable)?
            .0
            .render_target_release(self)
    }
}

impl crate::__private::game::Fields for RenderTargetRef {
    type Fields<'a> = crate::__private::game::ModelRefFields;
    fn fields(&self) -> Self::Fields<'_> {
        crate::__private::game::ModelRefFields {
            generation: self.0,
            slot: self.1,
        }
    }
}

/// Configuration for an offscreen render pass targeting a mod-owned [`RenderTargetRef`].
///
/// Used by `frame_graph_setup` for world offscreen passes and by picture-in-picture GUI
/// renderers. Unset clear ops (`None`) preserve the target's existing contents (`LoadOp::Load`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client offscreen render pass configuration.
pub struct OffscreenPass {
    /// Destination offscreen render target.
    pub target: RenderTargetRef,
    /// Per-colour-attachment clear values (`None` = preserve existing pixels, `Some(rgba)` =
    /// clear to `rgba`). Entries beyond the target's attachment count are ignored; omitted
    /// trailing entries preserve existing pixels.
    pub clear_color: Vec<Option<[f32; 4]>>,
    /// Depth clear value (`None` = preserve existing depth, `Some(d)` = clear to `d`, e.g.
    /// `0.0` for reversed-Z).
    pub clear_depth: Option<f32>,
    /// Stencil clear value when the target has a stencil aspect (`None` = preserve).
    pub clear_stencil: Option<u32>,
    /// Optional column-major 4x4 projection matrix overriding `Projection.ProjMat` for this pass.
    pub projection: Option<[f32; 16]>,
    /// Optional column-major 4x4 model-view matrix overriding `DynamicTransforms.ModelViewMat`
    /// for this pass.
    pub view: Option<[f32; 16]>,
    /// The mip level of the target's attachments the pass renders into (default 0), at that
    /// level's size (`GpuTextureView.getWidth(mipLevel)`). Must be below the target's
    /// [`RenderTargetDescriptor::mip_levels`].
    pub mip_level: u32,
}

impl OffscreenPass {
    /// Start an offscreen pass targeting `target` that preserves all existing attachment contents.
    pub fn new(target: RenderTargetRef) -> Self {
        Self {
            target,
            clear_color: Vec::new(),
            clear_depth: None,
            clear_stencil: None,
            projection: None,
            view: None,
            mip_level: 0,
        }
    }

    /// Render into mip level `mip_level` of the target (default 0).
    pub fn mip_level(mut self, mip_level: u32) -> Self {
        self.mip_level = mip_level;
        self
    }

    /// Set per-attachment colour clear operations.
    pub fn clear_color(mut self, clears: impl IntoIterator<Item = Option<[f32; 4]>>) -> Self {
        self.clear_color = clears.into_iter().collect();
        self
    }

    /// Clear colour attachment 0 to `rgba`.
    pub fn clear_color_0(mut self, rgba: [f32; 4]) -> Self {
        if self.clear_color.is_empty() {
            self.clear_color.push(Some(rgba));
        } else {
            self.clear_color[0] = Some(rgba);
        }
        self
    }

    /// Clear the depth attachment to `depth`.
    pub fn clear_depth(mut self, depth: f32) -> Self {
        self.clear_depth = Some(depth);
        self
    }

    /// Clear the stencil attachment to `stencil`.
    pub fn clear_stencil(mut self, stencil: u32) -> Self {
        self.clear_stencil = Some(stencil);
        self
    }

    /// Override `Projection.ProjMat` (column-major 4x4) for draws inside this pass.
    pub fn projection(mut self, matrix: [f32; 16]) -> Self {
        self.projection = Some(matrix);
        self
    }

    /// Override `DynamicTransforms.ModelViewMat` (column-major 4x4) for draws inside this pass.
    pub fn view(mut self, matrix: [f32; 16]) -> Self {
        self.view = Some(matrix);
        self
    }
}

/// Value bound to one set-1 entry in [`DrawBindings`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client per-draw set-1 binding value.
pub enum BindingValue<'a> {
    /// Uniform buffer binding (retained handle or inline bytes).
    Uniform(UniformValue<'a>),
    /// Combined 2D image/sampler binding.
    Texture(TextureSource<'a>),
    /// Raw texel elements matching the entry's [`TexelBufferFormat`] (1 byte/texel for `R8*`,
    /// 4 bytes/texel for `R32*`).
    TexelBuffer(&'a [u8]),
    /// A range of a retained [`StorageBufferRef`] for a [`BindingKind::StorageBuffer`] entry.
    StorageBuffer(StorageBufferSlice),
    /// One mip level of a render target colour attachment for a [`BindingKind::StorageImage`]
    /// entry.
    StorageImage(StorageImageBinding),
}

/// One mip level of a mod-owned offscreen render target's colour attachment, bound as a storage
/// image ([`DrawBindings::storage_image`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client storage image binding.
pub struct StorageImageBinding {
    /// The render target.
    pub target: RenderTargetRef,
    /// The colour attachment (`0..color_targets.len()`).
    pub attachment: u8,
    /// The mip level (`0..mip_levels`).
    pub mip_level: u32,
}

impl StorageImageBinding {
    /// Mip level `mip_level` of colour attachment `attachment` of `target`.
    pub const fn new(target: RenderTargetRef, attachment: u8, mip_level: u32) -> Self {
        Self {
            target,
            attachment,
            mip_level,
        }
    }
}

/// Per-draw set-1 resource bindings passed to [`crate::WorldDrawContext::draw_mesh`] and
/// [`crate::WorldDrawContext::draw_vertices`]. Pass `&DrawBindings::EMPTY` for pipelines with no
/// custom bind-group entries or push constants.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
// no wire: local client per-draw resource bindings.
pub struct DrawBindings<'a> {
    /// Named set-1 entries matching the pipeline's [`BindGroupLayout`].
    pub entries: Vec<(&'a str, BindingValue<'a>)>,
    /// Inline push-constant bytes when the pipeline declared `push_constant_size > 0`.
    pub push_constants: Option<&'a [u8]>,
}

impl<'a> DrawBindings<'a> {
    /// Empty bind-group bindings for pipelines that use only vanilla set-0 bindings.
    pub const EMPTY: Self = Self {
        entries: Vec::new(),
        push_constants: None,
    };

    /// Create an empty set-1 bindings builder.
    pub const fn new() -> Self {
        Self::EMPTY
    }

    /// Bind a retained [`UniformBufferRef`] to uniform entry `name`.
    pub fn uniform_buffer(mut self, name: &'a str, buffer: UniformBufferRef) -> Self {
        self.entries
            .push((name, BindingValue::Uniform(UniformValue::Buffer(buffer))));
        self
    }

    /// Bind inline std140 uniform `bytes` to uniform entry `name`.
    pub fn inline_uniform(mut self, name: &'a str, bytes: &'a [u8]) -> Self {
        self.entries
            .push((name, BindingValue::Uniform(UniformValue::Inline(bytes))));
        self
    }

    /// Bind a [`TextureSource`] to combined image sampler entry `name`.
    pub fn texture(mut self, name: &'a str, source: TextureSource<'a>) -> Self {
        self.entries.push((name, BindingValue::Texture(source)));
        self
    }

    /// Bind `slice` of a retained storage buffer to storage buffer entry `name`.
    pub fn storage_buffer(mut self, name: &'a str, slice: StorageBufferSlice) -> Self {
        self.entries
            .push((name, BindingValue::StorageBuffer(slice)));
        self
    }

    /// Bind one mip level of a render target colour attachment to storage image entry `name`.
    pub fn storage_image(mut self, name: &'a str, image: StorageImageBinding) -> Self {
        self.entries.push((name, BindingValue::StorageImage(image)));
        self
    }

    /// Bind raw texel bytes to texel buffer entry `name`.
    pub fn texel_buffer(mut self, name: &'a str, bytes: &'a [u8]) -> Self {
        self.entries.push((name, BindingValue::TexelBuffer(bytes)));
        self
    }

    /// Supply inline push-constant bytes (must be `<= push_constant_size`, 4-byte aligned).
    pub fn push_constants(mut self, bytes: &'a [u8]) -> Self {
        self.push_constants = Some(bytes);
        self
    }
}

/// Maximum number of colour targets a [`RenderPipeline`] can declare
/// (`ColorTargetState.MAX_COLOR_TARGETS`).
pub const MAX_COLOR_TARGETS: usize = 8;

/// Depth comparison operator (`com.mojang.renderpearl.api.pipeline.CompareOp`).
///
/// Sunrise uses reversed-Z in world passes (`1.0` near, `0.0` far, matching vanilla 26.3's
/// `DepthStencilState.DEFAULT = GREATER_THAN_OR_EQUAL`). On standard-Z shader-pack targets
/// (`0.0` near, `1.0` far), the renderer flips `Less*` ↔ `Greater*` automatically via
/// [`Self::flip_for_standard_z`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline depth comparison operator.
pub enum DepthCompare {
    /// Always pass (`CompareOp.ALWAYS_PASS`).
    Always,
    /// Pass when incoming depth `<` stored depth (`CompareOp.LESS_THAN`).
    Less,
    /// Pass when incoming depth `<=` stored depth (`CompareOp.LESS_THAN_OR_EQUAL`).
    LessEqual,
    /// Pass when incoming depth `==` stored depth (`CompareOp.EQUAL`).
    Equal,
    /// Pass when incoming depth `!=` stored depth (`CompareOp.NOT_EQUAL`).
    NotEqual,
    /// Pass when incoming depth `>=` stored depth (`CompareOp.GREATER_THAN_OR_EQUAL`, default
    /// for reversed-Z world passes).
    GreaterEqual,
    /// Pass when incoming depth `>` stored depth (`CompareOp.GREATER_THAN`).
    Greater,
    /// Never pass (`CompareOp.NEVER_PASS`).
    Never,
}

impl DepthCompare {
    /// Map a coarse [`WorldDepth`] to Sunrise's reversed-Z [`DepthCompare`].
    pub const fn from_world_depth(depth: WorldDepth) -> Self {
        match depth {
            WorldDepth::ThroughWalls => Self::Always,
            WorldDepth::Tested => Self::GreaterEqual,
        }
    }

    /// Flip `Less*` ↔ `Greater*` when targeting a standard-Z (`0.0` near, `1.0` far) render
    /// target such as a translated shader pack's `depthtex0`.
    pub const fn flip_for_standard_z(self) -> Self {
        match self {
            Self::Less => Self::Greater,
            Self::LessEqual => Self::GreaterEqual,
            Self::GreaterEqual => Self::LessEqual,
            Self::Greater => Self::Less,
            Self::Always => Self::Always,
            Self::Equal => Self::Equal,
            Self::NotEqual => Self::NotEqual,
            Self::Never => Self::Never,
        }
    }
}

/// Canonical bit representation of `f32` normalizing `-0.0` to `+0.0` so `DepthState` and
/// `PipelineState` satisfy `Eq` and `Hash`.
const fn canonical_f32_bits(value: f32) -> u32 {
    if value == 0.0 { 0 } else { value.to_bits() }
}

/// Depth test, depth write, and depth bias state
/// (`com.mojang.renderpearl.api.pipeline.DepthStencilState`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client render-pipeline depth-stencil state.
pub struct DepthState {
    /// Depth comparison function in Sunrise's reversed-Z world space.
    pub compare: DepthCompare,
    /// Whether fragments write to the depth buffer (`DepthStencilState.writeDepth`).
    pub write: bool,
    /// Constant depth bias in depth-buffer units (`wgpu::DepthBiasState::constant`,
    /// `DepthStencilState.depthBiasConstant`). Default: `0`.
    pub depth_bias_constant: i32,
    /// Slope-scaled depth bias factor (`wgpu::DepthBiasState::slope_scale`,
    /// `DepthStencilState.depthBiasScaleFactor`). Must be finite (refused at registration
    /// otherwise). Default: `0.0`.
    pub depth_bias_slope_scale: f32,
}

impl DepthState {
    /// Vanilla's `DepthStencilState.DEFAULT`: `GreaterEqual`, depth write enabled, zero bias.
    pub const DEFAULT: Self = Self {
        compare: DepthCompare::GreaterEqual,
        write: true,
        depth_bias_constant: 0,
        depth_bias_slope_scale: 0.0,
    };

    /// Construct a depth state with zero depth bias.
    pub const fn new(compare: DepthCompare, write: bool) -> Self {
        Self {
            compare,
            write,
            depth_bias_constant: 0,
            depth_bias_slope_scale: 0.0,
        }
    }

    /// Set constant and slope-scaled depth bias.
    pub const fn depth_bias(mut self, constant: i32, slope_scale: f32) -> Self {
        self.depth_bias_constant = constant;
        self.depth_bias_slope_scale = slope_scale;
        self
    }
}

impl PartialEq for DepthState {
    fn eq(&self, other: &Self) -> bool {
        self.compare == other.compare
            && self.write == other.write
            && self.depth_bias_constant == other.depth_bias_constant
            && canonical_f32_bits(self.depth_bias_slope_scale)
                == canonical_f32_bits(other.depth_bias_slope_scale)
    }
}

impl Eq for DepthState {}

impl std::hash::Hash for DepthState {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.compare.hash(state);
        self.write.hash(state);
        self.depth_bias_constant.hash(state);
        canonical_f32_bits(self.depth_bias_slope_scale).hash(state);
    }
}

/// Blending factor (`com.mojang.renderpearl.api.pipeline.BlendFactor`).
///
/// Note: WebGPU/wgpu maps both `ConstantColor`/`ConstantAlpha` to its `Constant` factor (and
/// `OneMinusConstantColor`/`OneMinusConstantAlpha` to `OneMinusConstant`), selecting RGB for the
/// colour equation and A for the alpha equation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline blend factor.
pub enum BlendFactor {
    /// Blend constant alpha.
    ConstantAlpha,
    /// Blend constant RGB colour.
    ConstantColor,
    /// Destination alpha.
    DstAlpha,
    /// Destination RGB colour.
    DstColor,
    /// Constant `1.0`.
    One,
    /// `1.0 - constant_alpha`.
    OneMinusConstantAlpha,
    /// `1.0 - constant_color`.
    OneMinusConstantColor,
    /// `1.0 - dst_alpha`.
    OneMinusDstAlpha,
    /// `1.0 - dst_color`.
    OneMinusDstColor,
    /// `1.0 - src_alpha`.
    OneMinusSrcAlpha,
    /// `1.0 - src_color`.
    OneMinusSrcColor,
    /// Source alpha.
    SrcAlpha,
    /// `min(src_alpha, 1.0 - dst_alpha)` for RGB, `1.0` for alpha.
    SrcAlphaSaturate,
    /// Source RGB colour.
    SrcColor,
    /// Constant `0.0`.
    Zero,
}

/// Blending operation (`com.mojang.renderpearl.api.pipeline.BlendOp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline blend operation.
pub enum BlendOp {
    /// `src + dst`.
    Add,
    /// `src - dst`.
    Subtract,
    /// `dst - src`.
    ReverseSubtract,
    /// `min(src, dst)`.
    Min,
    /// `max(src, dst)`.
    Max,
}

/// One channel's blend equation (`com.mojang.renderpearl.api.pipeline.BlendEquation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline blend equation.
pub struct BlendEquation {
    /// Source blend factor.
    pub src: BlendFactor,
    /// Destination blend factor.
    pub dst: BlendFactor,
    /// Blend operation combining weighted source and destination.
    pub op: BlendOp,
}

impl BlendEquation {
    /// Construct a blend equation with explicit source factor, destination factor, and op.
    pub const fn new(src: BlendFactor, dst: BlendFactor, op: BlendOp) -> Self {
        Self { src, dst, op }
    }

    /// Construct an additive (`BlendOp::Add`) blend equation.
    pub const fn add(src: BlendFactor, dst: BlendFactor) -> Self {
        Self {
            src,
            dst,
            op: BlendOp::Add,
        }
    }
}

/// Separate colour and alpha blend equations (`com.mojang.renderpearl.api.pipeline.BlendFunction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline custom blend function.
pub struct BlendFunction {
    /// RGB colour channel blend equation.
    pub color: BlendEquation,
    /// Alpha channel blend equation.
    pub alpha: BlendEquation,
}

impl BlendFunction {
    /// The first `Constant*` factor this function uses, which Sunrise refuses: it sets no blend
    /// constant, and a shader pack's blend table has no such factor.
    pub(crate) fn constant_factor(&self) -> Option<BlendFactor> {
        [
            self.color.src,
            self.color.dst,
            self.alpha.src,
            self.alpha.dst,
        ]
        .into_iter()
        .find(|factor| {
            matches!(
                factor,
                BlendFactor::ConstantColor
                    | BlendFactor::ConstantAlpha
                    | BlendFactor::OneMinusConstantColor
                    | BlendFactor::OneMinusConstantAlpha
            )
        })
    }

    /// `BlendFunction.LIGHTNING`: `SrcAlpha`, `One` (both colour and alpha).
    pub const LIGHTNING: Self = Self::symmetric(BlendFactor::SrcAlpha, BlendFactor::One);
    /// `BlendFunction.GLINT`: colour `(SrcColor, One)`, alpha `(Zero, One)`.
    pub const GLINT: Self = Self::separate(
        BlendFactor::SrcColor,
        BlendFactor::One,
        BlendFactor::Zero,
        BlendFactor::One,
    );
    /// `BlendFunction.OVERLAY`: colour `(SrcAlpha, One)`, alpha `(One, Zero)`.
    pub const OVERLAY: Self = Self::separate(
        BlendFactor::SrcAlpha,
        BlendFactor::One,
        BlendFactor::One,
        BlendFactor::Zero,
    );
    /// `BlendFunction.TRANSLUCENT`: colour `(SrcAlpha, OneMinusSrcAlpha)`, alpha `(One, OneMinusSrcAlpha)`.
    pub const TRANSLUCENT: Self = Self::separate(
        BlendFactor::SrcAlpha,
        BlendFactor::OneMinusSrcAlpha,
        BlendFactor::One,
        BlendFactor::OneMinusSrcAlpha,
    );
    /// `BlendFunction.TRANSLUCENT_PREMULTIPLIED_ALPHA`: `(One, OneMinusSrcAlpha)` on colour and alpha.
    pub const TRANSLUCENT_PREMULTIPLIED_ALPHA: Self = Self::separate(
        BlendFactor::One,
        BlendFactor::OneMinusSrcAlpha,
        BlendFactor::One,
        BlendFactor::OneMinusSrcAlpha,
    );
    /// `BlendFunction.ADDITIVE`: `(One, One)` on colour and alpha.
    pub const ADDITIVE: Self = Self::symmetric(BlendFactor::One, BlendFactor::One);
    /// `BlendFunction.ENTITY_OUTLINE_BLIT`: colour `(SrcAlpha, OneMinusSrcAlpha)`, alpha `(Zero, One)`.
    pub const ENTITY_OUTLINE_BLIT: Self = Self::separate(
        BlendFactor::SrcAlpha,
        BlendFactor::OneMinusSrcAlpha,
        BlendFactor::Zero,
        BlendFactor::One,
    );
    /// `BlendFunction.INVERT`: colour `(OneMinusDstColor, OneMinusSrcColor)`, alpha `(One, Zero)`.
    pub const INVERT: Self = Self::separate(
        BlendFactor::OneMinusDstColor,
        BlendFactor::OneMinusSrcColor,
        BlendFactor::One,
        BlendFactor::Zero,
    );
    /// `BlendFunction.MAX`: `(One, One, BlendOp::Max)` on colour and alpha.
    pub const MAX: Self = Self::symmetric_with_op(BlendFactor::One, BlendFactor::One, BlendOp::Max);

    /// Construct from explicit colour and alpha equations.
    pub const fn new(color: BlendEquation, alpha: BlendEquation) -> Self {
        Self { color, alpha }
    }

    /// Construct with separate colour and alpha additive equations.
    pub const fn separate(
        src_color: BlendFactor,
        dst_color: BlendFactor,
        src_alpha: BlendFactor,
        dst_alpha: BlendFactor,
    ) -> Self {
        Self {
            color: BlendEquation::add(src_color, dst_color),
            alpha: BlendEquation::add(src_alpha, dst_alpha),
        }
    }

    /// Construct with separate colour and alpha equations and explicit operations.
    pub const fn separate_with_ops(
        src_color: BlendFactor,
        dst_color: BlendFactor,
        color_op: BlendOp,
        src_alpha: BlendFactor,
        dst_alpha: BlendFactor,
        alpha_op: BlendOp,
    ) -> Self {
        Self {
            color: BlendEquation::new(src_color, dst_color, color_op),
            alpha: BlendEquation::new(src_alpha, dst_alpha, alpha_op),
        }
    }

    /// Construct with identical additive colour and alpha equations.
    pub const fn symmetric(src: BlendFactor, dst: BlendFactor) -> Self {
        let eq = BlendEquation::add(src, dst);
        Self {
            color: eq,
            alpha: eq,
        }
    }

    /// Construct with identical colour and alpha equations and an explicit operation.
    pub const fn symmetric_with_op(src: BlendFactor, dst: BlendFactor, op: BlendOp) -> Self {
        let eq = BlendEquation::new(src, dst, op);
        Self {
            color: eq,
            alpha: eq,
        }
    }
}

/// How a pipeline's fragments join the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline blend mode.
pub enum PipelineBlend {
    /// Solid colour written directly without alpha blending (vanilla's no-blend target state).
    Opaque,
    /// Laid over the frame by the fragment's alpha, as [`crate::WorldQuad`] is
    /// ([`BlendFunction::TRANSLUCENT`]).
    Translucent,
    /// Added to the frame, scaled by the fragment's alpha (colour `SrcAlpha + One`, alpha
    /// `Zero + One`).
    Additive,
    /// Explicit colour and alpha blend equations (vanilla's `BlendFunction` contract).
    Custom(BlendFunction),
}

impl PipelineBlend {
    /// Convert this blend mode into its underlying [`BlendFunction`], or `None` for
    /// [`Self::Opaque`].
    pub const fn to_blend_function(self) -> Option<BlendFunction> {
        match self {
            Self::Opaque => None,
            Self::Translucent => Some(BlendFunction::TRANSLUCENT),
            Self::Additive => Some(BlendFunction::separate(
                BlendFactor::SrcAlpha,
                BlendFactor::One,
                BlendFactor::Zero,
                BlendFactor::One,
            )),
            Self::Custom(func) => Some(func),
        }
    }
}

/// Per-target blending and channel write mask (vanilla's `ColorTargetState` contract). The target texture
/// format comes from the active render pass rather than the pipeline descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline colour target state.
pub struct ColorTargetState {
    /// How fragments join this colour target.
    pub blend: PipelineBlend,
    /// Which colour channels fragments write to this target.
    pub write: ColorWrite,
}

impl ColorTargetState {
    /// `ColorTargetState.DEFAULT`: opaque (no blend), write all RGBA channels.
    pub const DEFAULT: Self = Self {
        blend: PipelineBlend::Opaque,
        write: ColorWrite::ALL,
    };

    /// Construct a colour target state with explicit blend mode and write mask.
    pub const fn new(blend: PipelineBlend, write: ColorWrite) -> Self {
        Self { blend, write }
    }

    /// Construct a colour target state with `blend` and [`ColorWrite::ALL`].
    pub const fn blended(blend: PipelineBlend) -> Self {
        Self {
            blend,
            write: ColorWrite::ALL,
        }
    }
}

impl Default for ColorTargetState {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How vertices are assembled into primitives (`com.mojang.renderpearl.api.pipeline.PrimitiveTopology`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline primitive topology.
pub enum PrimitiveTopology {
    /// Vanilla's quad-expanded wide lines (`PrimitiveTopology.LINES`: 4 vertices -> 6 indices
    /// per line quad via the engine's shared quad index buffer).
    Lines,
    /// 1-pixel hardware line list (`PrimitiveTopology.DEBUG_LINES`).
    DebugLines,
    /// 1-pixel hardware line strip (`PrimitiveTopology.DEBUG_LINE_STRIP`).
    DebugLineStrip,
    /// Individual points (`PrimitiveTopology.POINTS`).
    Points,
    /// Independent triangles (`PrimitiveTopology.TRIANGLES`).
    Triangles,
    /// Connected triangle strip (`PrimitiveTopology.TRIANGLE_STRIP`).
    TriangleStrip,
    /// Triangle fan (`PrimitiveTopology.TRIANGLE_FAN`, as vanilla's sky draws it),
    /// emulated on WebGPU/wgpu via an engine-generated fan index buffer (`0, i, i + 1` for
    /// `i` in `1..vertex_count - 1`).
    TriangleFan,
    /// Quads expanded 4 vertices -> 6 indices via the engine's shared quad index buffer
    /// (`PrimitiveTopology.QUADS`). Default for [`RenderPipeline::new`] / `pipeline_quads`.
    #[default]
    Quads,
}

impl PrimitiveTopology {
    /// Vertices per primitive (`PrimitiveTopology.primitiveLength`).
    pub const fn primitive_length(self) -> u32 {
        match self {
            Self::Lines | Self::DebugLines | Self::DebugLineStrip => 2,
            Self::Points => 1,
            Self::Triangles | Self::TriangleStrip | Self::TriangleFan => 3,
            Self::Quads => 4,
        }
    }

    /// Vertex advance per additional primitive (`PrimitiveTopology.primitiveStride`).
    pub const fn primitive_stride(self) -> u32 {
        match self {
            Self::Lines | Self::DebugLines => 2,
            Self::DebugLineStrip | Self::Points | Self::TriangleStrip | Self::TriangleFan => 1,
            Self::Triangles => 3,
            Self::Quads => 4,
        }
    }

    /// Whether consecutive primitives share vertices (`PrimitiveTopology.connectedPrimitives`).
    pub const fn connected_primitives(self) -> bool {
        matches!(
            self,
            Self::DebugLineStrip | Self::TriangleStrip | Self::TriangleFan
        )
    }

    /// Number of indices produced for `vertex_count` vertices when drawing with engine-generated
    /// index buffers (`Quads` and `Lines`: `(vertex_count / 4) * 6`; `TriangleFan`:
    /// `(vertex_count - 2) * 3`; all native topologies: `vertex_count`).
    pub const fn index_count(self, vertex_count: u32) -> u32 {
        match self {
            Self::Lines | Self::Quads => (vertex_count / 4).saturating_mul(6),
            Self::TriangleFan => vertex_count.saturating_sub(2).saturating_mul(3),
            Self::DebugLines
            | Self::DebugLineStrip
            | Self::Points
            | Self::Triangles
            | Self::TriangleStrip => vertex_count,
        }
    }
}

/// Which colour channels a pipeline writes (`ColorTargetState` write mask).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline colour write mask.
pub struct ColorWrite {
    /// Write red, green and blue channels.
    pub color: bool,
    /// Write the alpha channel.
    pub alpha: bool,
}

impl ColorWrite {
    /// Write RGB and alpha (the default).
    pub const ALL: Self = Self {
        color: true,
        alpha: true,
    };
    /// Write only RGB, leaving alpha unchanged.
    pub const COLOR: Self = Self {
        color: true,
        alpha: false,
    };
    /// Write only alpha, leaving RGB unchanged.
    pub const ALPHA: Self = Self {
        color: false,
        alpha: true,
    };
    /// Write no colour channels (depth-only pass).
    pub const NONE: Self = Self {
        color: false,
        alpha: false,
    };

    /// Construct a write mask with explicit `color` (RGB) and `alpha` toggles.
    pub const fn new(color: bool, alpha: bool) -> Self {
        Self { color, alpha }
    }
}

impl Default for ColorWrite {
    fn default() -> Self {
        Self::ALL
    }
}

/// How polygons are rasterized (vanilla's `PolygonMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline polygon mode.
pub enum PolygonMode {
    /// Fill triangles (the default).
    #[default]
    Fill,
    /// Wireframe edges; requires the device's wireframe feature, so pipelines or modifier
    /// variants using `Line` are refused when the GPU lacks it.
    Line,
}

/// Fixed-function rasterization, depth, blend, colour-write, and topology state of a
/// [`RenderPipeline`] (vanilla's `RenderPipeline` state).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline fixed state.
pub struct PipelineState {
    /// How fragments join colour target 0 (and default MRT targets).
    pub blend: PipelineBlend,
    /// Depth test, depth write, and depth bias state (vanilla's `DepthStencilState` contract). `None`
    /// indicates a depthless pass (`RenderPipeline.wantsDepthTexture() == false`).
    pub depth_stencil: Option<DepthState>,
    /// Whether back faces are culled (`withCull`). Default: `false` (two-sided).
    pub cull: bool,
    /// Which colour channels fragments write (`ColorTargetState` write mask). Default:
    /// [`ColorWrite::ALL`].
    pub color_write: ColorWrite,
    /// Triangle fill or wireframe (`PolygonMode`). Default: [`PolygonMode::Fill`].
    pub polygon: PolygonMode,
    /// How vertices are assembled into primitives (vanilla's `PrimitiveTopology` contract). Default:
    /// [`PrimitiveTopology::Quads`].
    pub topology: PrimitiveTopology,
}

impl PipelineState {
    /// Default state for `blend` and `depth` (`depth_stencil: Some(DepthState::new(compare,
    /// false))`, `cull: false`, `color_write: ColorWrite::ALL`, `polygon: PolygonMode::Fill`,
    /// `topology: PrimitiveTopology::Quads`).
    pub const fn new(blend: PipelineBlend, depth: WorldDepth) -> Self {
        Self {
            blend,
            depth_stencil: Some(DepthState::new(
                DepthCompare::from_world_depth(depth),
                false,
            )),
            cull: false,
            color_write: ColorWrite::ALL,
            polygon: PolygonMode::Fill,
            topology: PrimitiveTopology::Quads,
        }
    }
}

/// Data-only state overrides registered by `Hooks::register_pipeline_modifiers` (NeoForge's
/// `RegisterPipelineModifiersEvent` / `PipelineModifier`).
///
/// Every field is optional: `None` leaves the underlying pipeline's value unchanged. Because a
/// modifier is pure data, applying it is idempotent by construction and the renderer caches each
/// `(pipeline, modifier stack)` variant lazily.
#[derive(Clone, Debug)]
#[non_exhaustive]
// no wire: local client pipeline-modifier registration descriptor.
pub struct PipelineModifier {
    owner: String,
    /// The modifier's own name (`mymod:xray`), unique among registered modifiers.
    pub id: Identifier,
    /// Override [`PipelineState::blend`]. Ignored for OIT variants where OIT supplies blending.
    pub blend: Option<PipelineBlend>,
    /// Override [`DepthState::compare`] on depth-attached pipelines (and `WorldDepth` on plain
    /// world primitives).
    pub depth_compare: Option<DepthCompare>,
    /// Override [`DepthState::write`] on depth-attached pipelines. Ignored for OIT variants
    /// where depth is read-only.
    pub depth_write: Option<bool>,
    /// Override `(depth_bias_constant, depth_bias_slope_scale)` on depth-attached pipelines.
    /// Non-finite slope scale is refused at registration.
    pub depth_bias: Option<(i32, f32)>,
    /// Override [`PipelineState::cull`].
    pub cull: Option<bool>,
    /// Override [`PipelineState::color_write`].
    pub color_write: Option<ColorWrite>,
    /// Override [`PipelineState::polygon`].
    pub polygon: Option<PolygonMode>,
    /// Override [`PipelineState::topology`].
    pub topology: Option<PrimitiveTopology>,
}

impl PartialEq for PipelineModifier {
    fn eq(&self, other: &Self) -> bool {
        let bias_eq = match (self.depth_bias, other.depth_bias) {
            (None, None) => true,
            (Some((ac, as_)), Some((bc, bs))) => {
                ac == bc && canonical_f32_bits(as_) == canonical_f32_bits(bs)
            }
            _ => false,
        };
        self.owner == other.owner
            && self.id == other.id
            && self.blend == other.blend
            && self.depth_compare == other.depth_compare
            && self.depth_write == other.depth_write
            && bias_eq
            && self.cull == other.cull
            && self.color_write == other.color_write
            && self.polygon == other.polygon
            && self.topology == other.topology
    }
}

impl Eq for PipelineModifier {}

impl PipelineModifier {
    /// Describe a modifier with no overrides set.
    pub fn new(id: Identifier) -> Self {
        Self {
            owner: String::new(),
            id,
            blend: None,
            depth_compare: None,
            depth_write: None,
            depth_bias: None,
            cull: None,
            color_write: None,
            polygon: None,
            topology: None,
        }
    }

    /// The owning mod, assigned by the host after registration.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Override [`PipelineState::blend`].
    pub fn blend(mut self, blend: PipelineBlend) -> Self {
        self.blend = Some(blend);
        self
    }

    /// Override depth testing from a coarse [`WorldDepth`] (`Tested` ->
    /// [`DepthCompare::GreaterEqual`], `ThroughWalls` -> [`DepthCompare::Always`]).
    pub fn depth(mut self, depth: WorldDepth) -> Self {
        self.depth_compare = Some(DepthCompare::from_world_depth(depth));
        self
    }

    /// Override [`DepthState::compare`].
    pub fn depth_compare(mut self, compare: DepthCompare) -> Self {
        self.depth_compare = Some(compare);
        self
    }

    /// Override [`DepthState::write`].
    pub fn depth_write(mut self, write: bool) -> Self {
        self.depth_write = Some(write);
        self
    }

    /// Override [`DepthState::depth_bias_constant`] and [`DepthState::depth_bias_slope_scale`].
    /// Non-finite `slope_scale` is refused at registration.
    pub fn depth_bias(mut self, constant: i32, slope_scale: f32) -> Self {
        self.depth_bias = Some((constant, slope_scale));
        self
    }

    /// Override [`PipelineState::cull`].
    pub fn cull(mut self, cull: bool) -> Self {
        self.cull = Some(cull);
        self
    }

    /// Override [`PipelineState::color_write`].
    pub fn color_write(mut self, write: ColorWrite) -> Self {
        self.color_write = Some(write);
        self
    }

    /// Override [`PipelineState::polygon`].
    pub fn polygon(mut self, polygon: PolygonMode) -> Self {
        self.polygon = Some(polygon);
        self
    }

    /// Override [`PipelineState::topology`].
    pub fn topology(mut self, topology: PrimitiveTopology) -> Self {
        self.topology = Some(topology);
        self
    }

    /// Validate modifier fields at registration freeze (e.g. finite `depth_bias` slope scale).
    pub fn validate(&self) -> Result<(), String> {
        if let Some(factor) = self
            .blend
            .and_then(PipelineBlend::to_blend_function)
            .and_then(|blend| blend.constant_factor())
        {
            return Err(format!("blend factor {factor:?} is not supported"));
        }
        if let Some((_, slope_scale)) = self.depth_bias
            && !slope_scale.is_finite()
        {
            return Err(format!(
                "invalid depth_bias slope_scale {slope_scale:?}: must be finite"
            ));
        }
        Ok(())
    }

    /// Apply this modifier's non-`None` overrides to `state`. Depth overrides (`depth_compare`,
    /// `depth_write`, `depth_bias`) apply only when `state.depth_stencil` is `Some`.
    pub const fn apply(&self, mut state: PipelineState) -> PipelineState {
        if let Some(blend) = self.blend {
            state.blend = blend;
        }
        if let Some(mut ds) = state.depth_stencil {
            if let Some(compare) = self.depth_compare {
                ds.compare = compare;
            }
            if let Some(depth_write) = self.depth_write {
                ds.write = depth_write;
            }
            if let Some((constant, slope_scale)) = self.depth_bias {
                ds.depth_bias_constant = constant;
                ds.depth_bias_slope_scale = slope_scale;
            }
            state.depth_stencil = Some(ds);
        }
        if let Some(cull) = self.cull {
            state.cull = cull;
        }
        if let Some(color_write) = self.color_write {
            state.color_write = color_write;
        }
        if let Some(polygon) = self.polygon {
            state.polygon = polygon;
        }
        if let Some(topology) = self.topology {
            state.topology = topology;
        }
        state
    }

    /// Whether this modifier overrides no state fields.
    pub const fn is_noop(&self) -> bool {
        self.blend.is_none()
            && self.depth_compare.is_none()
            && self.depth_write.is_none()
            && self.depth_bias.is_none()
            && self.cull.is_none()
            && self.color_write.is_none()
            && self.polygon.is_none()
            && self.topology.is_none()
    }
}

/// A registered pipeline modifier, resolved through
/// [`crate::RenderAssets::pipeline_modifier`]. Retain it between frames; modifiers are static
/// across resource reloads, though re-resolving in `client_resources_reloaded` is valid.
// no wire: process-local handle to a registered client pipeline modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private field; only the engine resolves modifier handles
pub struct PipelineModifierRef(pub(crate) u32);

impl PipelineModifierRef {
    pub(crate) fn from_slot(slot: u32) -> Self {
        Self(slot)
    }
}

/// A registered pipeline as the current resources built it, resolved through
/// [`crate::RenderAssets::render_pipeline`]. Retain it between frames; resolve it again after a
/// successful resource reload.
// no wire: process-local handle to a client render pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private fields; only the engine resolves pipeline handles
pub struct RenderPipelineRef(pub(crate) u64, pub(crate) u32);

impl RenderPipelineRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished pipeline generation");
        }
        Ok(Self(generation, slot))
    }
}

/// Why a pipeline could not be resolved or drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client render-pipeline lookup or draw.
pub enum RenderPipelineError {
    /// No mod registered a pipeline of this id.
    Unknown,
    /// The pipeline is registered, but the current resources give it no usable shader. The log
    /// names the pipeline and the reason.
    Invalid,
    /// A successful resource reload superseded the handle's generation.
    Stale,
}

/// A two-sided quad drawn by a mod pipeline. Corners run top left, bottom left, bottom right,
/// top right as seen from the front.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client render-pipeline quad primitive.
pub struct PipelineQuad {
    /// Four world-space corners in blocks, before camera subtraction.
    pub corners: [[f64; 3]; 4],
    /// Each corner's texture coordinate, handed to the shader as given.
    pub uv: [[f32; 2]; 4],
    /// Packed 0xAARRGGBB colour in vanilla's gamma space, passed straight to the GLSL `Color`
    /// attribute.
    pub color: u32,
}
impl PipelineQuad {
    /// Describe a quad; the draw context validates it when submitting the batch.
    pub fn new(corners: [[f64; 3]; 4], uv: [[f32; 2]; 4], color: u32) -> Self {
        Self { corners, uv, color }
    }
    /// A quad whose texture coordinates span 0 to 1: (0, 0) at the top left corner and (1, 1)
    /// at the bottom right.
    pub fn spanning(corners: [[f64; 3]; 4], color: u32) -> Self {
        Self::new(
            corners,
            [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            color,
        )
    }
}

/// Why a pipeline draw call was refused; nothing from that call was appended.
///
/// Individual quads with non-finite coordinates or UVs are skipped without refusing valid quads
/// in the same slice, and quad storage grows dynamically into the GPU buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client render-pipeline draw submission.
pub enum PipelineDrawError {
    /// The handle is stale, or names no usable pipeline.
    Pipeline(RenderPipelineError),
    /// The pipeline's effective [`PrimitiveTopology`] does not match the draw call (for example,
    /// submitting a non-[`PrimitiveTopology::Quads`] pipeline to `pipeline_quads`, or a
    /// `Quads`, `Lines` or `TriangleFan` pipeline to a non-indexed [`IndirectDraw`](crate::IndirectDraw)).
    TopologyMismatch,
    /// The retained [`MeshRef`] handle was stale, not owned, or unavailable.
    Mesh(MeshError),
    /// The provided vertex buffer count or per-binding stride does not match the pipeline's
    /// declared [`VertexFormat`] bindings.
    VertexFormatMismatch,
    /// [`DrawRange::Indexed`] was requested without an index buffer.
    MissingIndexBuffer,
    /// The vertex, index, or instance range is inverted, out of bounds, or (for `Quads`/`Lines`
    /// non-indexed draws) not a multiple of 4 vertices.
    InvalidRange,
    /// Transient vertex or index bytes are not aligned to 4 bytes, the binding stride, or the
    /// index element size.
    InvalidAlignment,
    /// Set-1 [`DrawBindings`] is missing a declared entry, supplies an unknown/duplicate entry,
    /// mismatches kind or `min_size`, references an unknown/unavailable texture, or provides an
    /// invalid push-constant payload.
    InvalidBinding,
    /// A retained [`UniformBufferRef`] in [`DrawBindings`] was stale, not owned, or unavailable.
    /// For an [`IndirectDraw`](crate::IndirectDraw), also its command buffer.
    Buffer(BufferError),
    /// An [`IndirectDraw`](crate::IndirectDraw)'s command buffer was not created with
    /// [`StorageBufferDescriptor::indirect`](crate::StorageBufferDescriptor::indirect), its
    /// offset is not a multiple of 4, `indirect_offset + max_count * stride` lies beyond the
    /// buffer; or its count buffer was not created with
    /// [`StorageBufferDescriptor::indirect`](crate::StorageBufferDescriptor::indirect), its
    /// count offset is not a multiple of 4, or the count's 4 bytes lie beyond that buffer.
    InvalidIndirect,
    /// An [`IndirectDraw`](crate::IndirectDraw) has a count buffer, and this adapter cannot draw
    /// a GPU-written count ([`RenderCapabilities::multi_draw_indirect_count`] is false).
    UnsupportedIndirectCount,
    /// A [`TextureSource::RenderTargetColor`] or [`TextureSource::RenderTargetDepth`] in
    /// [`DrawBindings`] binds a mip level of a [`RenderTargetRef`] both as a storage image and
    /// as something else the draw reads or writes (a sampled view, or a storage image of another
    /// access), which one render pass cannot do.
    TargetReadWriteHazard,
}

/// Well-known built-in vanilla render type identifiers resolved through
/// [`RenderAssets::render_type`](crate::RenderAssets::render_type).
///
/// Indices into this slice are their fixed slot IDs; custom render types registered via
/// [`RegisterRenderPipelinesEvent::register_render_type`](crate::RegisterRenderPipelinesEvent::register_render_type)
/// follow immediately after.
pub const BUILTIN_RENDER_TYPE_IDS: &[&str] = &[
    "minecraft:solid",
    "minecraft:cutout",
    "minecraft:translucent",
    "minecraft:entity_solid",
    "minecraft:entity_cutout",
    "minecraft:entity_translucent",
    "minecraft:eyes",
    "minecraft:energy_swirl",
    "minecraft:lightning",
    "minecraft:item_cutout",
    "minecraft:item_translucent",
];

/// Model-view Z layering transform applied to a [`RenderSetup`] (vanilla 26.3
/// `LayeringTransform.java`). Depth bias belongs to [`DepthState`] on [`RenderPipeline`], not here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-type setup descriptor.
pub enum LayeringTransform {
    /// `LayeringTransform.NO_LAYERING`: no model-view Z offset.
    #[default]
    None,
    /// `LayeringTransform.VIEW_OFFSET_Z_LAYERING`: offsets toward the camera along view Z (`+1.0`).
    ViewOffsetZ,
    /// `LayeringTransform.VIEW_OFFSET_Z_LAYERING_FORWARD`: offsets away from the camera along view Z (`-1.0`).
    ViewOffsetZForward,
}

/// Texture-matrix transform applied to a [`RenderSetup`] (vanilla 26.3 `TextureTransform.java`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
// no wire: local client render-type setup descriptor.
pub enum TextureTransform {
    /// `TextureTransform.DEFAULT_TEXTURING`: identity texture matrix.
    #[default]
    Default,
    /// `TextureTransform.GLINT_TEXTURING`: animated item enchantment glint matrix (scale `8.0`).
    Glint,
    /// `TextureTransform.ENTITY_GLINT_TEXTURING`: animated entity glint matrix (scale `0.5`).
    EntityGlint,
    /// `TextureTransform.ARMOR_ENTITY_GLINT_TEXTURING`: animated armor glint matrix (scale `0.16`).
    ArmorEntityGlint,
    /// `TextureTransform.OffsetTextureTransform`: static UV translation `[u_offset, v_offset]`.
    Offset([f32; 2]),
}

/// How a [`RenderSetup`] participates in entity outline rendering (vanilla 26.3
/// `RenderSetup.OutlineProperty`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-type setup descriptor.
pub enum OutlineProperty {
    /// `OutlineProperty.NONE`: does not participate in outline rendering.
    #[default]
    None,
    /// `OutlineProperty.IS_OUTLINE`: this render type is itself an outline pass.
    IsOutline,
    /// `OutlineProperty.AFFECTS_OUTLINE`: draws to the outline pass using `texture` (or
    /// `"Sampler0"` when `None`).
    AffectsOutline {
        /// Optional sampler name for the outline mask texture (`None` uses `"Sampler0"`).
        texture: Option<String>,
    },
}

/// Owned texture source bound on a [`RenderSetup`] (the owned counterpart of [`TextureSource`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-type texture source descriptor.
pub enum RenderTypeTextureSource {
    /// A sprite on the block/item atlas (`minecraft:block/stone`, `mymod:block/custom`).
    AtlasSprite(Identifier),
    /// A live dynamic texture registered via
    /// [`crate::prelude::ClientRenderClientHost::create_dynamic_texture`].
    DynamicTexture(Identifier),
    /// A standalone PNG texture asset (`assets/<ns>/textures/<path>.png` or exact `.png` asset
    /// path) loaded from the active resource packs.
    StandaloneTexture(Identifier),
    /// The active world pass's depth texture (usable only when the pipeline does not write
    /// depth in the same pass).
    WorldDepth,
    /// Vanilla's active 16x16 lightmap texture.
    Lightmap,
}

impl RenderTypeTextureSource {
    /// Borrow as a per-draw [`TextureSource`].
    pub fn as_borrowed(&self) -> TextureSource<'_> {
        match self {
            Self::AtlasSprite(id) => TextureSource::AtlasSprite(id),
            Self::DynamicTexture(id) => TextureSource::DynamicTexture(id),
            Self::StandaloneTexture(id) => TextureSource::StandaloneTexture(id),
            Self::WorldDepth => TextureSource::WorldDepth,
            Self::Lightmap => TextureSource::Lightmap,
        }
    }
}

/// One named texture binding on a [`RenderSetup`] (vanilla 26.3 `RenderSetup.TextureBinding`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-type texture binding descriptor.
pub struct RenderTypeTexture {
    /// GLSL sampler name (for example `"Sampler0"`).
    pub name: String,
    /// Texture source bound to this sampler.
    pub source: RenderTypeTextureSource,
    /// Sampler filtering mode.
    pub filter: SamplerFilter,
    /// Sampler address/wrap mode.
    pub address: SamplerAddress,
}

impl RenderTypeTexture {
    /// Describe one named texture binding for a [`RenderSetup`].
    pub fn new(
        name: impl Into<String>,
        source: RenderTypeTextureSource,
        filter: SamplerFilter,
        address: SamplerAddress,
    ) -> Self {
        Self {
            name: name.into(),
            source,
            filter,
            address,
        }
    }
}

/// Declarative custom render type registered in `Hooks::register_render_pipelines` (vanilla 26.3
/// `RenderSetup` / `RenderType.create`).
///
/// Wraps a [`RenderPipeline`] identifier with texture bindings, lightmap/overlay toggles,
/// crumbling/sorting/solid-phase flags, [`LayeringTransform`], [`TextureTransform`], and
/// [`OutlineProperty`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client render-type registration descriptor.
pub struct RenderSetup {
    owner: String,
    /// Unique namespaced identifier for this render type (for example `mymod:glowing_cutout`).
    pub id: Identifier,
    /// Registered mod or built-in vanilla [`RenderPipeline`] identifier.
    pub pipeline: Identifier,
    /// Named texture bindings (`RenderSetup.textures`), e.g. `"Sampler0"`.
    pub textures: Vec<RenderTypeTexture>,
    /// Bind the engine lightmap texture to `Sampler2` (`RenderSetup.useLightmap`).
    pub use_lightmap: bool,
    /// Bind the hurt/white-flash overlay texture to `Sampler1` (`RenderSetup.useOverlay`).
    pub use_overlay: bool,
    /// Whether geometry drawn with this render type participates in mining-crack crumbling
    /// (`RenderSetup.affectsCrumbling`).
    pub affects_crumbling: bool,
    /// Whether translucent quads sort far-to-near on upload (`RenderSetup.sortOnUpload`).
    pub sort_on_upload: bool,
    /// Force the solid model phase even when the underlying pipeline blends
    /// (`RenderSetup.forceSolidModelPhase`).
    pub force_solid_model_phase: bool,
    /// Model-view Z layering offset (`RenderSetup.layeringTransform`).
    pub layering: LayeringTransform,
    /// Texture-coordinate matrix transform (`RenderSetup.textureTransform`).
    pub texture_transform: TextureTransform,
    /// Entity outline pass participation (`RenderSetup.outlineProperty` + `outlineTextureName`).
    pub outline: OutlineProperty,
}

impl RenderSetup {
    /// Start a render type setup with `id` backed by `pipeline`, matching
    /// `RenderSetup.builder(pipeline)` defaults (no lightmap, no overlay, no crumbling, no sort,
    /// `LayeringTransform::None`, `TextureTransform::Default`, `OutlineProperty::None`).
    pub fn new(id: Identifier, pipeline: Identifier) -> Self {
        Self {
            owner: String::new(),
            id,
            pipeline,
            textures: Vec::new(),
            use_lightmap: false,
            use_overlay: false,
            affects_crumbling: false,
            sort_on_upload: false,
            force_solid_model_phase: false,
            layering: LayeringTransform::None,
            texture_transform: TextureTransform::Default,
            outline: OutlineProperty::None,
        }
    }

    /// Bind a named texture sampler (`RenderSetupBuilder.withTexture`).
    pub fn with_texture(
        mut self,
        name: impl Into<String>,
        source: RenderTypeTextureSource,
        filter: SamplerFilter,
        address: SamplerAddress,
    ) -> Self {
        self.textures
            .push(RenderTypeTexture::new(name, source, filter, address));
        self
    }

    /// Enable or disable binding the engine lightmap to `Sampler2` (`RenderSetupBuilder.useLightmap`).
    pub fn use_lightmap(mut self, enabled: bool) -> Self {
        self.use_lightmap = enabled;
        self
    }

    /// Enable or disable binding the overlay texture to `Sampler1` (`RenderSetupBuilder.useOverlay`).
    pub fn use_overlay(mut self, enabled: bool) -> Self {
        self.use_overlay = enabled;
        self
    }

    /// Enable or disable mining-crack crumbling participation (`RenderSetupBuilder.affectsCrumbling`).
    pub fn affects_crumbling(mut self, enabled: bool) -> Self {
        self.affects_crumbling = enabled;
        self
    }

    /// Enable or disable far-to-near quad sorting on upload (`RenderSetupBuilder.sortOnUpload`).
    pub fn sort_on_upload(mut self, enabled: bool) -> Self {
        self.sort_on_upload = enabled;
        self
    }

    /// Force the solid model phase (`RenderSetupBuilder.withForcedSolidModelPhase`).
    pub fn force_solid_model_phase(mut self, enabled: bool) -> Self {
        self.force_solid_model_phase = enabled;
        self
    }

    /// Set the model-view layering transform (`RenderSetupBuilder.setLayeringTransform`).
    pub fn layering(mut self, layering: LayeringTransform) -> Self {
        self.layering = layering;
        self
    }

    /// Set the texture-coordinate transform (`RenderSetupBuilder.setTextureTransform`).
    pub fn texture_transform(mut self, transform: TextureTransform) -> Self {
        self.texture_transform = transform;
        self
    }

    /// Set the outline pass participation (`RenderSetupBuilder.setOutline`).
    pub fn outline(mut self, outline: OutlineProperty) -> Self {
        self.outline = outline;
        self
    }

    /// The mod that registered this render type, stamped by
    /// [`RegisterRenderPipelinesEvent::register_render_type`](crate::RegisterRenderPipelinesEvent::register_render_type).
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Validate local setup invariants before startup publication.
    pub fn validate(&self) -> Result<(), String> {
        if let TextureTransform::Offset([u, v]) = self.texture_transform
            && (!u.is_finite() || !v.is_finite())
        {
            return Err("texture_transform offset must be finite".into());
        }
        let mut seen = std::collections::HashSet::with_capacity(self.textures.len());
        for tex in &self.textures {
            if tex.name.is_empty() {
                return Err("texture binding sampler name must not be empty".into());
            }
            if !seen.insert(tex.name.as_str()) {
                return Err(format!("duplicate texture binding sampler {:?}", tex.name));
            }
            if self.use_overlay && tex.name == "Sampler1" {
                return Err(
                    "texture binding \"Sampler1\" collides with use_overlay (Sampler1)".into(),
                );
            }
            if self.use_lightmap && tex.name == "Sampler2" {
                return Err(
                    "texture binding \"Sampler2\" collides with use_lightmap (Sampler2)".into(),
                );
            }
        }
        if let OutlineProperty::AffectsOutline {
            texture: Some(name),
        } = &self.outline
            && name.is_empty()
        {
            return Err("outline texture sampler name must not be empty".into());
        }
        Ok(())
    }
}

/// Handle to a built-in or custom render type resolved for the current resource load via
/// [`RenderAssets::render_type`](crate::RenderAssets::render_type).
///
/// Generational like [`RenderPipelineRef`]: resolve once in `client_setup` and again in
/// `client_resources_reloaded`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// no wire: local client resource-load generational handle.
// exhaustive: only the engine creates handles.
pub struct RenderTypeRef(pub(crate) u64, pub(crate) u32);

impl RenderTypeRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Option<Self> {
        (generation != 0).then_some(Self(generation, slot))
    }
}

/// Why [`RenderAssets::render_type`](crate::RenderAssets::render_type) could not resolve an
/// identifier, or why a draw rejected a [`RenderTypeRef`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-type error.
pub enum RenderTypeError {
    /// No built-in or registered custom render type has that identifier.
    Unknown,
    /// The render type's underlying pipeline failed to build in the current resource load.
    Invalid,
    /// The handle came from an earlier resource load; resolve it again in
    /// `client_resources_reloaded`.
    Stale,
}

impl crate::RegisterRenderPipelinesEvent<'_> {
    /// Register one render pipeline without exposing earlier registrations.
    ///
    /// The entry is attributed to this listener immediately. Any valid identifier is accepted
    /// here; duplicate IDs and vanilla collisions are refused when the engine freezes the
    /// registrations at startup.
    pub fn register(&mut self, mut entry: RenderPipeline) {
        entry.owner = self.mod_id.to_owned();
        self.pipelines.push(entry);
    }

    /// Register one custom render type ([`RenderSetup`]) without exposing earlier registrations.
    ///
    /// The entry is attributed to this listener immediately. Duplicate IDs, collisions with
    /// [`BUILTIN_RENDER_TYPE_IDS`], invalid texture/outline/transform configurations, and
    /// references to unknown pipelines are refused when the engine freezes registrations at
    /// startup.
    pub fn register_render_type(&mut self, mut entry: RenderSetup) {
        entry.owner = self.mod_id.to_owned();
        self.render_types.push(entry);
    }
}

impl crate::RegisterPipelineModifiersEvent<'_> {
    /// Register one pipeline modifier without exposing earlier registrations.
    ///
    /// The entry is attributed to this listener immediately. Duplicate modifier IDs fail
    /// client startup naming both mods when the engine freezes registrations.
    pub fn register(&mut self, mut entry: PipelineModifier) {
        entry.owner = self.mod_id.to_owned();
        self.modifiers.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline(id: &str) -> RenderPipeline {
        let id = Identifier::parse(id).unwrap();
        RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
    }
    fn ids(pipelines: &[RenderPipeline]) -> Vec<String> {
        pipelines.iter().map(|p| p.id.to_string()).collect()
    }

    #[test]
    fn stamps_owner_on_new_pipelines_across_namespaces_and_preserves_duplicates_for_freeze() {
        let mut pipelines = Vec::new();
        let mut render_types = Vec::new();
        let mut compute_pipelines = Vec::new();
        let mut event = crate::RegisterRenderPipelinesEvent {
            mod_id: "first_mod",
            pipelines: &mut pipelines,
            render_types: &mut render_types,
            compute_pipelines: &mut compute_pipelines,
        };
        event.register(pipeline("other:glow"));
        event.mod_id = "owning_mod";
        for id in ["minecraft:glow", "other:beam", "other:glow", "mine:beam"] {
            event.register(pipeline(id));
        }
        assert_eq!(
            ids(&pipelines),
            [
                "other:glow".to_owned(),
                "minecraft:glow".to_owned(),
                "other:beam".to_owned(),
                "other:glow".to_owned(),
                "mine:beam".to_owned(),
            ]
        );
        assert_eq!(pipelines[0].owner(), "first_mod");
        assert_eq!(pipelines[1].owner(), "owning_mod");
        assert_eq!(pipelines[2].owner(), "owning_mod");
        assert_eq!(pipelines[3].owner(), "owning_mod");
        assert_eq!(pipelines[4].owner(), "owning_mod");
    }

    #[test]
    fn a_mod_can_register_more_than_vanillas_pipeline_count() {
        let mut pipelines = Vec::new();
        let mut render_types = Vec::new();
        let mut compute_pipelines = Vec::new();
        {
            let mut event = crate::RegisterRenderPipelinesEvent {
                mod_id: "mine",
                pipelines: &mut pipelines,
                render_types: &mut render_types,
                compute_pipelines: &mut compute_pipelines,
            };
            for i in 0..131 {
                event.register(pipeline(&format!("mine:p{i}")));
            }
        }
        assert_eq!(pipelines.len(), 131);
        assert_eq!(pipelines.last().unwrap().id.to_string(), "mine:p130");

        crate::RegisterRenderPipelinesEvent {
            mod_id: "next",
            pipelines: &mut pipelines,
            render_types: &mut render_types,
            compute_pipelines: &mut compute_pipelines,
        }
        .register(pipeline("next:glow"));
        assert_eq!(pipelines.len(), 132);
    }

    #[test]
    fn effective_pack_programs_prepends_custom_names_ahead_of_geometry_class_chain() {
        let plain = pipeline("example:plain");
        assert!(plain.effective_pack_programs().is_empty());
        assert_eq!(plain.effective_pack_slot(), None);

        let entity = pipeline("example:entity").geometry_class(PipelineGeometryClass::Entity);
        assert_eq!(
            entity.effective_pack_programs(),
            [
                "gbuffers_entities",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            PipelineGeometryClass::Particles.default_programs(),
            [
                "gbuffers_particles",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            entity.effective_pack_slot(),
            Some(PackPassSlot::AfterOpaqueGbuffers)
        );

        let custom = pipeline("example:custom_water")
            .geometry_class(PipelineGeometryClass::Translucent)
            .pack_binding(
                PackBinding::new()
                    .program("custom_water")
                    .program("gbuffers_terrain")
                    .slot(PackPassSlot::AfterTranslucentGbuffers),
            );
        assert_eq!(
            custom.effective_pack_programs(),
            [
                "custom_water",
                "gbuffers_terrain",
                "gbuffers_water",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            custom.effective_pack_slot(),
            Some(PackPassSlot::AfterTranslucentGbuffers)
        );
    }

    #[test]
    fn pack_pass_slots_follow_shader_pack_frame_order() {
        let slots = [
            PackPassSlot::Shadow,
            PackPassSlot::AfterSky,
            PackPassSlot::AfterOpaqueGbuffers,
            PackPassSlot::AfterTranslucentGbuffers,
            PackPassSlot::AfterWeather,
        ];
        assert!(slots.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            PipelineGeometryClass::Sky.default_slot(),
            PackPassSlot::AfterSky
        );
        assert_eq!(
            PipelineGeometryClass::Terrain.default_slot(),
            PackPassSlot::AfterOpaqueGbuffers
        );
        assert_eq!(
            PipelineGeometryClass::Translucent.default_slot(),
            PackPassSlot::AfterTranslucentGbuffers
        );
        assert_eq!(
            PipelineGeometryClass::Weather.default_slot(),
            PackPassSlot::AfterWeather
        );
    }

    #[test]
    fn pack_binding_syntax_rejects_bad_identifiers_and_multiline_defines() {
        let valid = PackBinding::new()
            .program("custom_terrain")
            .define("MOD_ACTIVE")
            .define_value("MOD_MATERIAL_LEAVES", "1")
            .depth_target(
                PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                    .depth_sampler("modDepthTex0")
                    .opaque_depth_sampler("modDepthTex1"),
            )
            .frame_uniform(
                PackUniformDecl::new("modProjection", PackUniformType::Mat4)
                    .with_previous("modPreviousProjection"),
            )
            .glsl_binding(PackGlslInput::new(
                "position",
                "gl_Vertex",
                PackUniformType::Vec4,
            ));
        assert_eq!(valid.validate_syntax(), Ok(()));

        for bad_define in ["", "1BAD", "BAD-NAME", "GL_RESERVED", "has space"] {
            assert!(
                PackBinding::new()
                    .define(bad_define)
                    .validate_syntax()
                    .is_err(),
                "expected invalid define name {bad_define:?} to fail"
            );
        }
        for bad_val in [
            "",
            "   ",
            "1\n#define INJECT 1",
            "1\r0",
            "1 // comment",
            "1 /* comment */",
            "1 \\",
            "#include",
            " leading",
        ] {
            assert!(
                PackBinding::new()
                    .define_value("GOOD_NAME", bad_val)
                    .validate_syntax()
                    .is_err(),
                "expected invalid define value {bad_val:?} to fail"
            );
        }
        for bad_program in ["", "../escape", "gbuffers_terrain.fsh", "has space"] {
            assert!(
                PackBinding::new()
                    .program(bad_program)
                    .validate_syntax()
                    .is_err(),
                "expected invalid program {bad_program:?} to fail"
            );
        }
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .depth_sampler("gl_FragDepth")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .depth_sampler("sharedDepth")
                        .opaque_depth_sampler("sharedDepth")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .depth_sampler("dupDepth")
                        .depth_sampler("dupDepth")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .opaque_depth_sampler("dupOpaque")
                        .opaque_depth_sampler("dupOpaque")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .frame_uniform(
                    PackUniformDecl::new("modProj", PackUniformType::Mat4).with_previous("modProj")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .frame_uniform(
                    PackUniformDecl::new("projA", PackUniformType::Mat4).with_previous("projB")
                )
                .frame_uniform(PackUniformDecl::new("projB", PackUniformType::Mat4))
                .validate_syntax()
                .is_err()
        );
    }

    #[test]
    fn program_mode_and_depth_target_builders() {
        assert_eq!(
            PackBinding::new().program_mode,
            PackProgramMode::PackVertexAndFragment
        );
        let own = PackBinding::new()
            .program_mode(PackProgramMode::ModVertexPackFragment)
            .depth_target(PackDepthTarget::Main);
        assert_eq!(own.program_mode, PackProgramMode::ModVertexPackFragment);
        assert_eq!(own.depth_target, Some(PackDepthTarget::Main));
        assert_eq!(own.validate_syntax(), Ok(()));
        // The main depth has no samplers of its own: the builders leave it as it is.
        assert_eq!(
            PackDepthTarget::Main.depth_sampler("modDepth"),
            PackDepthTarget::Main
        );
        let aux = PackDepthTarget::new(Identifier::parse("example:aux").unwrap())
            .depth_sampler("modDepth");
        assert!(matches!(
            &aux,
            PackDepthTarget::Aux { depth_samplers, .. } if depth_samplers == &["modDepth"]
        ));
    }

    #[test]
    fn shader_defines_and_vertex_shader_builder_validate_syntax() {
        let p = pipeline("example:glow")
            .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
            .define("PULSE")
            .define_value("STEPS", "4");
        assert_eq!(
            p.vertex_shader,
            Some(Identifier::parse("minecraft:core/position_tex_color").unwrap())
        );
        assert_eq!(
            p.defines,
            vec![
                ("PULSE".to_owned(), None),
                ("STEPS".to_owned(), Some("4".to_owned())),
            ]
        );
        assert_eq!(p.validate_defines(), Ok(()));

        for bad_name in ["", "1BAD", "BAD-NAME", "GL_RESERVED", "has space"] {
            assert!(
                pipeline("example:bad")
                    .define(bad_name)
                    .validate_defines()
                    .is_err(),
                "expected invalid define name {bad_name:?} to fail"
            );
        }
        for bad_val in [
            "",
            "   ",
            "1\n#define INJECT 1",
            "1 // comment",
            "1 /* comment */",
            "1 \\",
            "#include",
        ] {
            assert!(
                pipeline("example:bad")
                    .define_value("GOOD", bad_val)
                    .validate_defines()
                    .is_err(),
                "expected invalid define value {bad_val:?} to fail"
            );
        }
    }

    #[test]
    fn pipeline_state_builders_and_modifier_overrides_apply_cleanly() {
        let p = pipeline("example:solid")
            .blend(PipelineBlend::Opaque)
            .depth(WorldDepth::Tested)
            .depth_write(true)
            .depth_bias(10, 1.5)
            .cull(true)
            .color_write(ColorWrite::COLOR)
            .polygon(PolygonMode::Line)
            .topology(PrimitiveTopology::Triangles)
            .order_independent();
        assert_eq!(
            p.state,
            PipelineState {
                blend: PipelineBlend::Opaque,
                depth_stencil: Some(DepthState {
                    compare: DepthCompare::GreaterEqual,
                    write: true,
                    depth_bias_constant: 10,
                    depth_bias_slope_scale: 1.5,
                }),
                cull: true,
                color_write: ColorWrite::COLOR,
                polygon: PolygonMode::Line,
                topology: PrimitiveTopology::Triangles,
            }
        );
        assert!(p.order_independent);
        assert_eq!(p.validate_state(), Ok(()));

        let noop = PipelineModifier::new(Identifier::parse("example:noop").unwrap());
        assert!(noop.is_noop());
        assert_eq!(noop.apply(p.state), p.state);

        let xray = PipelineModifier::new(Identifier::parse("example:xray").unwrap())
            .depth_compare(DepthCompare::LessEqual)
            .depth_write(false)
            .depth_bias(4, 0.5)
            .polygon(PolygonMode::Fill)
            .topology(PrimitiveTopology::DebugLines);
        assert!(!xray.is_noop());
        let modified = xray.apply(p.state);
        assert_eq!(
            modified.depth_stencil,
            Some(DepthState {
                compare: DepthCompare::LessEqual,
                write: false,
                depth_bias_constant: 4,
                depth_bias_slope_scale: 0.5,
            })
        );
        assert_eq!(modified.polygon, PolygonMode::Fill);
        assert_eq!(modified.topology, PrimitiveTopology::DebugLines);
        assert_eq!(modified.blend, PipelineBlend::Opaque);
        assert!(modified.cull);
        assert_eq!(modified.color_write, ColorWrite::COLOR);

        // Depthless pipeline stays depthless even under depth-overriding modifiers.
        let depthless = pipeline("example:post").depth_stencil(None);
        assert_eq!(depthless.state.depth_stencil, None);
        assert_eq!(xray.apply(depthless.state).depth_stencil, None);

        let mut modifiers = Vec::new();
        let mut event = crate::RegisterPipelineModifiersEvent {
            mod_id: "mod_b",
            modifiers: &mut modifiers,
        };
        event.register(xray);
        assert_eq!(modifiers.len(), 1);
        assert_eq!(modifiers[0].owner(), "mod_b");
    }

    #[test]
    fn constant_blend_factors_are_refused_by_name_in_state_target_and_modifier() {
        let constant =
            BlendFunction::symmetric(BlendFactor::OneMinusConstantAlpha, BlendFactor::One);
        let state = pipeline("example:constant").blend(PipelineBlend::Custom(constant));
        assert_eq!(
            state.validate_state().unwrap_err(),
            "blend factor OneMinusConstantAlpha is not supported"
        );
        let target = pipeline("example:constant_target").color_target(
            0,
            Some(ColorTargetState::blended(PipelineBlend::Custom(
                BlendFunction::symmetric(BlendFactor::One, BlendFactor::ConstantColor),
            ))),
        );
        assert_eq!(
            target.validate_state().unwrap_err(),
            "blend factor ConstantColor is not supported"
        );
        let modifier = PipelineModifier::new(Identifier::parse("example:constant_mod").unwrap())
            .blend(PipelineBlend::Custom(constant));
        assert_eq!(
            modifier.validate().unwrap_err(),
            "blend factor OneMinusConstantAlpha is not supported"
        );
        let fine = BlendFunction::symmetric(BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha);
        assert_eq!(
            pipeline("example:fine")
                .blend(PipelineBlend::Custom(fine))
                .validate_state(),
            Ok(())
        );
    }

    #[test]
    fn validate_state_refuses_bad_depth_bias_out_of_range_target_and_mixed_mrt_blends() {
        // Non-finite depth bias slope scale refused.
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let err = pipeline("example:bad_bias")
                .depth_bias(0, bad)
                .validate_state()
                .unwrap_err();
            assert!(err.contains("depth_bias_slope_scale"), "{err}");
            let mod_err = PipelineModifier::new(Identifier::parse("example:bad_mod").unwrap())
                .depth_bias(0, bad)
                .validate()
                .unwrap_err();
            assert!(mod_err.contains("depth_bias"), "{mod_err}");
        }

        // Color target index >= MAX_COLOR_TARGETS (8) refused.
        let out_of_range = pipeline("example:bad_slot")
            .color_target(MAX_COLOR_TARGETS, Some(ColorTargetState::DEFAULT));
        assert!(out_of_range.validate_state().is_err());

        // Differing blend functions across active MRT targets refused (a pipeline's blend state is one function across its active color targets).
        let mixed = pipeline("example:mixed_mrt")
            .color_target(
                0,
                Some(ColorTargetState::blended(PipelineBlend::Translucent)),
            )
            .color_target(
                1,
                Some(ColorTargetState::blended(PipelineBlend::Custom(
                    BlendFunction::INVERT,
                ))),
            );
        assert!(mixed.validate_state().is_err());

        // Same blend function (including Opaque + unused slots) accepted across 8 targets.
        let valid_mrt = pipeline("example:valid_mrt")
            .color_target(
                0,
                Some(ColorTargetState::blended(PipelineBlend::Translucent)),
            )
            .color_target(1, None)
            .color_target(2, Some(ColorTargetState::DEFAULT))
            .color_target(
                7,
                Some(ColorTargetState::blended(PipelineBlend::Custom(
                    BlendFunction::TRANSLUCENT,
                ))),
            );
        assert_eq!(valid_mrt.validate_state(), Ok(()));
        assert_eq!(valid_mrt.effective_color_targets().len(), 8);
    }

    #[test]
    fn depth_compare_flips_for_standard_z_and_topology_index_counts_are_the_emulated_counts() {
        assert_eq!(
            DepthCompare::GreaterEqual.flip_for_standard_z(),
            DepthCompare::LessEqual
        );
        assert_eq!(
            DepthCompare::Greater.flip_for_standard_z(),
            DepthCompare::Less
        );
        assert_eq!(
            DepthCompare::LessEqual.flip_for_standard_z(),
            DepthCompare::GreaterEqual
        );
        assert_eq!(
            DepthCompare::Less.flip_for_standard_z(),
            DepthCompare::Greater
        );
        assert_eq!(
            DepthCompare::Always.flip_for_standard_z(),
            DepthCompare::Always
        );
        assert_eq!(
            DepthCompare::Equal.flip_for_standard_z(),
            DepthCompare::Equal
        );
        assert_eq!(
            DepthCompare::NotEqual.flip_for_standard_z(),
            DepthCompare::NotEqual
        );
        assert_eq!(
            DepthCompare::Never.flip_for_standard_z(),
            DepthCompare::Never
        );

        assert_eq!(PrimitiveTopology::Quads.index_count(8), 12);
        assert_eq!(PrimitiveTopology::Lines.index_count(8), 12);
        // The emulated fan: `(n - 2) * 3` list indices, not vanilla's native fan draw.
        assert_eq!(PrimitiveTopology::TriangleFan.index_count(5), 9);
        assert_eq!(PrimitiveTopology::TriangleFan.index_count(2), 0);
        assert_eq!(PrimitiveTopology::Triangles.index_count(6), 6);
        assert_eq!(PrimitiveTopology::DebugLines.index_count(4), 4);
        assert_eq!(PrimitiveTopology::DebugLineStrip.index_count(4), 4);
        assert_eq!(PrimitiveTopology::Points.index_count(3), 3);
    }

    #[test]
    fn vertex_format_builder_validates_alignment_stride_names_count_and_step_rate() {
        // Valid compact 8-byte vertex format (4B uvec4 + 4B unorm8x4).
        let compact = VertexFormat::builder(VertexStepMode::Vertex)
            .attribute("PackedPos", VertexAttributeFormat::Uint8x4)
            .attribute("PackedColor", VertexAttributeFormat::Unorm8x4)
            .build()
            .unwrap();
        assert_eq!(compact.stride, 8);
        assert_eq!(compact.attributes.len(), 2);
        assert_eq!(compact.attributes[0].offset, 0);
        assert_eq!(compact.attributes[1].offset, 4);
        assert_eq!(compact.attributes[0].format.glsl_type(), "uvec4");
        assert_eq!(compact.attributes[1].format.glsl_type(), "vec4");

        // Unaligned offset refused.
        assert!(matches!(
            VertexFormat::builder(VertexStepMode::Vertex)
                .attribute_at("Pos", 2, VertexAttributeFormat::Float32x3)
                .stride(16)
                .build(),
            Err(VertexFormatError::UnalignedOffset {
                offset: 2,
                alignment: 4,
                ..
            })
        ));

        // Non-multiple-of-4 stride refused.
        assert_eq!(
            VertexFormat::builder(VertexStepMode::Vertex)
                .attribute("Uv", VertexAttributeFormat::Unorm8x2)
                .build(),
            Err(VertexFormatError::InvalidStride(2))
        );
        // Zero stride refused.
        assert_eq!(
            VertexFormat::builder(VertexStepMode::Vertex).build(),
            Err(VertexFormatError::InvalidStride(0))
        );

        // Duplicate attribute name refused.
        assert_eq!(
            VertexFormat::builder(VertexStepMode::Vertex)
                .attribute("Pos", VertexAttributeFormat::Float32x3)
                .attribute("Pos", VertexAttributeFormat::Float32)
                .build(),
            Err(VertexFormatError::DuplicateAttributeName("Pos".to_owned()))
        );

        // Invalid and gl_-prefixed names refused.
        for bad in ["", "1bad", "bad-name", "gl_Position"] {
            assert!(matches!(
                VertexFormat::builder(VertexStepMode::Vertex)
                    .attribute(bad, VertexAttributeFormat::Float32)
                    .build(),
                Err(VertexFormatError::InvalidAttributeName(_))
            ));
        }

        // More than 16 attributes refused.
        let mut too_many = VertexFormat::builder(VertexStepMode::Vertex);
        for i in 0..=MAX_VERTEX_ELEMENTS {
            too_many = too_many.attribute(format!("a{i}"), VertexAttributeFormat::Float32);
        }
        assert_eq!(
            too_many.build(),
            Err(VertexFormatError::TooManyAttributes(
                MAX_VERTEX_ELEMENTS + 1
            ))
        );

        // Step rate divisor > 1 refused.
        assert_eq!(
            VertexFormat::builder_with_step_rate(2)
                .attribute("InstOffset", VertexAttributeFormat::Float32x3)
                .build(),
            Err(VertexFormatError::UnsupportedStepRate(2))
        );
        assert_eq!(
            VertexFormat::builder_with_step_rate(1)
                .attribute("InstOffset", VertexAttributeFormat::Float32x3)
                .build()
                .unwrap()
                .step_mode,
            VertexStepMode::Instance
        );

        // Pipeline-level validation: binding >= 16, duplicate binding, cross-binding duplicate name,
        // and > 16 total attributes across bindings all refused.
        assert!(
            pipeline("example:bad_binding")
                .vertex_binding(MAX_VERTEX_BINDINGS as u8, compact.clone())
                .validate_vertex_bindings()
                .is_err()
        );
        assert!(
            pipeline("example:dup_binding")
                .vertex_binding(0, compact.clone())
                .vertex_binding(0, compact.clone())
                .validate_vertex_bindings()
                .is_err()
        );
        assert!(
            pipeline("example:dup_attr")
                .vertex_binding(0, compact.clone())
                .vertex_binding(1, compact.clone())
                .validate_vertex_bindings()
                .is_err()
        );
        let mut nine_a = VertexFormat::builder(VertexStepMode::Vertex);
        let mut eight_b = VertexFormat::builder(VertexStepMode::Instance);
        for i in 0..9 {
            nine_a = nine_a.attribute(format!("v{i}"), VertexAttributeFormat::Float32);
        }
        for i in 0..8 {
            eight_b = eight_b.attribute(format!("i{i}"), VertexAttributeFormat::Float32);
        }
        assert!(
            pipeline("example:too_many_total")
                .vertex_binding(0, nine_a.build().unwrap())
                .vertex_binding(1, eight_b.build().unwrap())
                .validate_vertex_bindings()
                .is_err()
        );
    }

    #[test]
    fn mesh_ref_routes_through_backend_and_refuses_without_one() {
        struct Bare;
        impl crate::Host for Bare {}
        crate::impl_host_areas!(Bare);
        impl crate::ClientHost for Bare {}
        crate::impl_client_host_areas!(Bare);

        let mesh = MeshRef::from_parts(1, 0).unwrap();
        let init = [MeshBufferInit::empty(8, 32)];
        let desc = MeshDescriptor::new(&init);
        assert_eq!(
            mesh.write_vertices(&Bare, 0, 0, &[0; 8]),
            Err(MeshError::Unavailable)
        );
        assert_eq!(
            mesh.write_indices(&Bare, 0, &[0; 4]),
            Err(MeshError::Unavailable)
        );
        assert_eq!(mesh.replace(&Bare, &desc), Err(MeshError::Unavailable));
        assert_eq!(mesh.release(&Bare), Err(MeshError::Unavailable));

        struct Recording {
            calls: std::cell::RefCell<Vec<&'static str>>,
        }
        impl crate::Host for Recording {
            fn backend(&self) -> Option<crate::BackendRef<'_>> {
                Some(crate::BackendRef::of(self))
            }
        }
        crate::impl_host_areas!(Recording);
        impl crate::ClientHost for Recording {}
        crate::impl_client_host_areas!(Recording);
        impl crate::Backend for Recording {}
        crate::impl_backend_areas!(Recording, except(client_render));
        impl crate::prelude::ClientRenderClientBackend for Recording {
            fn mesh_write_vertices(
                &self,
                _: MeshRef,
                _: u8,
                _: u64,
                _: &[u8],
            ) -> Result<(), MeshError> {
                self.calls.borrow_mut().push("write_vertices");
                Ok(())
            }
            fn mesh_write_indices(&self, _: MeshRef, _: u64, _: &[u8]) -> Result<(), MeshError> {
                self.calls.borrow_mut().push("write_indices");
                Ok(())
            }
            fn mesh_replace(&self, _: MeshRef, _: &MeshDescriptor<'_>) -> Result<(), MeshError> {
                self.calls.borrow_mut().push("replace");
                Ok(())
            }
            fn mesh_release(&self, _: MeshRef) -> Result<(), MeshError> {
                self.calls.borrow_mut().push("destroy");
                Ok(())
            }
        }

        let host = Recording {
            calls: std::cell::RefCell::new(Vec::new()),
        };
        mesh.write_vertices(&host, 0, 0, &[0; 8]).unwrap();
        mesh.write_indices(&host, 0, &[0; 4]).unwrap();
        mesh.replace(&host, &desc).unwrap();
        mesh.release(&host).unwrap();
        assert_eq!(
            *host.calls.borrow(),
            vec!["write_vertices", "write_indices", "replace", "destroy"]
        );
    }

    #[test]
    fn a_sampler_max_lod_must_be_finite_and_not_negative() {
        let layout = |max_lod| {
            let mut sampler = Sampler::new(SamplerFilter::Nearest, SamplerAddress::ClampToEdge);
            sampler.max_lod = max_lod;
            BindGroupLayout::builder()
                .sampler("ModTex", sampler)
                .build()
        };
        for ok in [None, Some(0.0), Some(3.0), Some(1000.0)] {
            assert!(layout(ok).is_ok(), "{ok:?}");
        }
        for bad in [-0.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                layout(Some(bad)),
                Err(BindGroupLayoutError::BadMaxLod("ModTex".to_owned())),
                "{bad}"
            );
        }
    }

    #[test]
    fn bind_group_layout_validation_refuses_duplicates_reserved_zero_size_and_oversize_push_constants()
     {
        // Valid layout with uniform buffer, sampler, and texel buffer.
        let layout = BindGroupLayout::builder()
            .uniform_buffer("ModParams", 16)
            .sampler(
                "ModTex",
                Sampler::new(SamplerFilter::Linear, SamplerAddress::Repeat),
            )
            .texel_buffer("ModLut", TexelBufferFormat::R32Sint)
            .build()
            .expect("valid set-1 layout");
        assert_eq!(layout.entries().len(), 3);

        // Duplicate name refused (BindGroupLayout.ensureCompatible).
        assert_eq!(
            BindGroupLayout::builder()
                .uniform_buffer("ModParams", 16)
                .sampler(
                    "ModParams",
                    Sampler::new(SamplerFilter::Nearest, SamplerAddress::ClampToEdge)
                )
                .build(),
            Err(BindGroupLayoutError::DuplicateName("ModParams".to_owned()))
        );

        // Vanilla BindGroupLayouts.java reserved names refused (including Coeff0/Coeff1).
        for reserved in [
            "Globals",
            "Projection",
            "DynamicTransforms",
            "Fog",
            "Sampler0",
            "DepthBoundsSampler",
            "Coeff0",
            "Coeff1",
            "CloudFaces",
        ] {
            assert_eq!(
                BindGroupLayout::builder()
                    .uniform_buffer(reserved, 16)
                    .build(),
                Err(BindGroupLayoutError::ReservedName(reserved.to_owned())),
                "reserved name {reserved} must be refused"
            );
        }

        // SpriteRect is reserved only when .textured(...) is set on the pipeline.
        let untextured = pipeline("test:untextured").uniform_buffer("SpriteRect", 16);
        assert!(untextured.validate_bind_group_layout().is_ok());
        let textured = pipeline("test:textured")
            .textured(PipelineTexture::Sprite(
                Identifier::parse("minecraft:block/stone").unwrap(),
            ))
            .uniform_buffer("SpriteRect", 16);
        assert_eq!(
            textured.validate_bind_group_layout(),
            Err(BindGroupLayoutError::ReservedName("SpriteRect".to_owned()))
        );

        // Zero min_size refused.
        assert_eq!(
            BindGroupLayout::builder()
                .uniform_buffer("EmptyBlock", 0)
                .build(),
            Err(BindGroupLayoutError::ZeroUniformSize(
                "EmptyBlock".to_owned()
            ))
        );
        let zero_storage = BindGroupLayout {
            entries: vec![BindEntry::new(
                "Tints",
                BindingKind::StorageBuffer {
                    read_only: true,
                    min_size: 0,
                },
            )],
        };
        assert_eq!(
            zero_storage.validate(false),
            Err(BindGroupLayoutError::ZeroStorageSize("Tints".to_owned()))
        );

        // Push constants: <= 128 B and 4-byte aligned accepted; > 128 B or unaligned refused.
        assert!(
            pipeline("test:pc_ok")
                .push_constant_size(MAX_PUSH_CONSTANT_SIZE)
                .validate_bind_group_layout()
                .is_ok()
        );
        assert_eq!(
            pipeline("test:pc_big")
                .push_constant_size(MAX_PUSH_CONSTANT_SIZE + 4)
                .validate_bind_group_layout(),
            Err(BindGroupLayoutError::PushConstantTooLarge(132))
        );
        assert_eq!(
            pipeline("test:pc_unaligned")
                .push_constant_size(14)
                .validate_bind_group_layout(),
            Err(BindGroupLayoutError::PushConstantUnaligned(14))
        );
    }

    #[test]
    fn uniform_buffer_ref_routes_through_backend_and_refuses_without_one() {
        struct Bare;
        impl crate::Host for Bare {}
        crate::impl_host_areas!(Bare);
        impl crate::ClientHost for Bare {}
        crate::impl_client_host_areas!(Bare);

        let buf = UniformBufferRef::from_parts(1, 0).unwrap();
        assert_eq!(buf.write(&Bare, 0, &[0; 16]), Err(BufferError::Unavailable));
        assert_eq!(buf.release(&Bare), Err(BufferError::Unavailable));

        struct Recording {
            calls: std::cell::RefCell<Vec<&'static str>>,
            written: std::cell::RefCell<Option<(u32, Vec<u8>)>>,
        }
        impl crate::Host for Recording {
            fn backend(&self) -> Option<crate::BackendRef<'_>> {
                Some(crate::BackendRef::of(self))
            }
        }
        crate::impl_host_areas!(Recording);
        impl crate::ClientHost for Recording {}
        crate::impl_client_host_areas!(Recording);
        impl crate::Backend for Recording {}
        crate::impl_backend_areas!(Recording, except(client_render));
        impl crate::prelude::ClientRenderClientBackend for Recording {
            fn uniform_buffer_write(
                &self,
                _: UniformBufferRef,
                offset: u32,
                data: &[u8],
            ) -> Result<(), BufferError> {
                self.calls.borrow_mut().push("write");
                *self.written.borrow_mut() = Some((offset, data.to_vec()));
                Ok(())
            }
            fn uniform_buffer_release(&self, _: UniformBufferRef) -> Result<(), BufferError> {
                self.calls.borrow_mut().push("destroy");
                Err(BufferError::Stale)
            }
        }

        let host = Recording {
            calls: std::cell::RefCell::new(Vec::new()),
            written: std::cell::RefCell::new(None),
        };
        buf.write(&host, 8, &[1, 2, 3, 4]).unwrap();
        // The backend's own answer reaches the caller unchanged.
        assert_eq!(buf.release(&host), Err(BufferError::Stale));
        assert_eq!(*host.calls.borrow(), vec!["write", "destroy"]);
        assert_eq!(*host.written.borrow(), Some((8, vec![1, 2, 3, 4])));
    }

    #[test]
    fn render_setup_builder_validation_and_registration() {
        let mut pipelines = Vec::new();
        let mut render_types = Vec::new();
        let mut compute_pipelines = Vec::new();
        let mut event = crate::RegisterRenderPipelinesEvent {
            mod_id: "example",
            pipelines: &mut pipelines,
            render_types: &mut render_types,
            compute_pipelines: &mut compute_pipelines,
        };
        let setup = RenderSetup::new(
            Identifier::parse("example:glowing").unwrap(),
            Identifier::parse("example:core/glow").unwrap(),
        )
        .with_texture(
            "Sampler0",
            RenderTypeTextureSource::StandaloneTexture(
                Identifier::parse("example:textures/glow.png").unwrap(),
            ),
            SamplerFilter::Linear,
            SamplerAddress::ClampToEdge,
        )
        .use_lightmap(true)
        .use_overlay(true)
        .affects_crumbling(true)
        .sort_on_upload(true)
        .force_solid_model_phase(false)
        .layering(LayeringTransform::ViewOffsetZForward)
        .texture_transform(TextureTransform::ArmorEntityGlint)
        .outline(OutlineProperty::AffectsOutline {
            texture: Some("Sampler0".into()),
        });
        assert!(setup.validate().is_ok());
        event.register_render_type(setup);
        assert_eq!(render_types.len(), 1);
        assert_eq!(render_types[0].owner(), "example");
        assert_eq!(
            render_types[0].layering,
            LayeringTransform::ViewOffsetZForward
        );
        assert_eq!(
            render_types[0].texture_transform,
            TextureTransform::ArmorEntityGlint
        );

        let bad_offset = RenderSetup::new(
            Identifier::parse("example:bad_offset").unwrap(),
            Identifier::parse("example:core/glow").unwrap(),
        )
        .texture_transform(TextureTransform::Offset([f32::NAN, 0.0]));
        assert!(bad_offset.validate().is_err());

        let sampler2_clash = RenderSetup::new(
            Identifier::parse("example:clash").unwrap(),
            Identifier::parse("example:core/glow").unwrap(),
        )
        .use_lightmap(true)
        .with_texture(
            "Sampler2",
            RenderTypeTextureSource::StandaloneTexture(
                Identifier::parse("example:textures/a.png").unwrap(),
            ),
            SamplerFilter::Nearest,
            SamplerAddress::Repeat,
        );
        assert!(sampler2_clash.validate().is_err());

        let dup_sampler = RenderSetup::new(
            Identifier::parse("example:dup").unwrap(),
            Identifier::parse("example:core/glow").unwrap(),
        )
        .with_texture(
            "Sampler0",
            RenderTypeTextureSource::StandaloneTexture(
                Identifier::parse("example:textures/a.png").unwrap(),
            ),
            SamplerFilter::Nearest,
            SamplerAddress::Repeat,
        )
        .with_texture(
            "Sampler0",
            RenderTypeTextureSource::StandaloneTexture(
                Identifier::parse("example:textures/b.png").unwrap(),
            ),
            SamplerFilter::Nearest,
            SamplerAddress::Repeat,
        );
        assert!(dup_sampler.validate().is_err());
    }

    #[test]
    fn render_target_descriptor_validates_size_attachments_and_bounds() {
        assert_eq!(
            RenderTargetDescriptor::rgba8([0, 64], RenderTargetDepth::None).validate(8192),
            Err(RenderTargetError::InvalidSize)
        );
        assert_eq!(
            RenderTargetDescriptor::rgba8([64, 0], RenderTargetDepth::None).validate(8192),
            Err(RenderTargetError::InvalidSize)
        );
        assert_eq!(
            RenderTargetDescriptor::rgba8([8193, 64], RenderTargetDepth::None).validate(8192),
            Err(RenderTargetError::InvalidSize)
        );
        assert_eq!(
            RenderTargetDescriptor::new([64, 64]).validate(8192),
            Err(RenderTargetError::NoAttachments)
        );
        assert_eq!(
            RenderTargetDescriptor::new([64, 64])
                .color_targets([RenderTargetFormat::Rgba8Unorm; MAX_COLOR_TARGETS + 1])
                .validate(8192),
            Err(RenderTargetError::TooManyColorTargets)
        );
        // Depth-only target is allowed.
        assert_eq!(
            RenderTargetDescriptor::new([128, 128])
                .depth(RenderTargetDepth::Depth32Float)
                .validate(8192),
            Ok(())
        );
        // Max colour targets + depth+stencil is allowed.
        assert_eq!(
            RenderTargetDescriptor::new([8192, 8192])
                .color_targets([RenderTargetFormat::Rgba16Float; MAX_COLOR_TARGETS])
                .depth(RenderTargetDepth::Depth24PlusStencil8)
                .validate(8192),
            Ok(())
        );
    }

    #[test]
    fn render_target_ref_routes_through_backend_and_refuses_without_one() {
        struct Bare;
        impl crate::Host for Bare {}
        crate::impl_host_areas!(Bare);
        impl crate::ClientHost for Bare {}
        crate::impl_client_host_areas!(Bare);

        let rt = RenderTargetRef::from_parts(1, 0).unwrap();
        assert_eq!(
            rt.resize(&Bare, [64, 64]),
            Err(RenderTargetError::Unavailable)
        );
        assert_eq!(rt.size(&Bare), Err(RenderTargetError::Unavailable));
        assert_eq!(rt.release(&Bare), Err(RenderTargetError::Unavailable));

        struct Recording {
            calls: std::cell::RefCell<Vec<&'static str>>,
            size: std::cell::Cell<[u32; 2]>,
        }
        impl crate::Host for Recording {
            fn backend(&self) -> Option<crate::BackendRef<'_>> {
                Some(crate::BackendRef::of(self))
            }
        }
        crate::impl_host_areas!(Recording);
        impl crate::ClientHost for Recording {}
        crate::impl_client_host_areas!(Recording);
        impl crate::Backend for Recording {}
        crate::impl_backend_areas!(Recording, except(client_render));
        impl crate::prelude::ClientRenderClientBackend for Recording {
            fn render_target_resize(
                &self,
                _: RenderTargetRef,
                size: [u32; 2],
            ) -> Result<(), RenderTargetError> {
                self.calls.borrow_mut().push("resize");
                self.size.set(size);
                Ok(())
            }
            fn render_target_size(
                &self,
                _: RenderTargetRef,
            ) -> Result<[u32; 2], RenderTargetError> {
                self.calls.borrow_mut().push("size");
                Ok(self.size.get())
            }
            fn render_target_release(&self, _: RenderTargetRef) -> Result<(), RenderTargetError> {
                self.calls.borrow_mut().push("release");
                Ok(())
            }
        }

        let host = Recording {
            calls: std::cell::RefCell::new(Vec::new()),
            size: std::cell::Cell::new([32, 32]),
        };
        assert_eq!(rt.size(&host), Ok([32, 32]));
        rt.resize(&host, [128, 64]).unwrap();
        assert_eq!(rt.size(&host), Ok([128, 64]));
        rt.release(&host).unwrap();
        assert_eq!(
            *host.calls.borrow(),
            vec!["size", "resize", "size", "release"]
        );

        let pass = OffscreenPass::new(rt)
            .clear_color_0([0.1, 0.2, 0.3, 1.0])
            .clear_depth(0.0)
            .clear_stencil(0)
            .projection([1.0; 16])
            .view([2.0; 16]);
        assert_eq!(pass.target, rt);
        assert_eq!(pass.clear_color, vec![Some([0.1, 0.2, 0.3, 1.0])]);
        assert_eq!(pass.clear_depth, Some(0.0));
        assert_eq!(pass.clear_stencil, Some(0));
        assert_eq!(pass.projection, Some([1.0; 16]));
        assert_eq!(pass.view, Some([2.0; 16]));

        let bindings = DrawBindings::new()
            .texture("uColor", TextureSource::RenderTargetColor(rt, 0))
            .texture("uDepth", TextureSource::RenderTargetDepth(rt));
        assert_eq!(bindings.entries.len(), 2);
    }

    #[test]
    fn mip_levels_are_bounded_by_the_size_and_storage_needs_a_colour_target() {
        let rgba =
            |size| RenderTargetDescriptor::new(size).color_target(RenderTargetFormat::R32Float);
        // `32 - leading_zeros(max side)`: 64 -> 7, 65 -> 7, 1 -> 1, a 128 x 8 strip -> 8.
        for (size, levels) in [([64, 64], 7), ([65, 3], 7), ([1, 1], 1), ([128, 8], 8)] {
            assert_eq!(
                RenderTargetDescriptor::max_mip_levels(size),
                levels,
                "{size:?}"
            );
            assert_eq!(rgba(size).mip_levels(levels).validate(8192), Ok(()));
            assert_eq!(
                rgba(size).mip_levels(levels + 1).validate(8192),
                Err(RenderTargetError::InvalidMipLevels)
            );
        }
        assert_eq!(rgba([64, 64]).validate(8192), Ok(()));
        assert_eq!(rgba([64, 64]).mip_levels, 1);
        assert_eq!(
            rgba([64, 64]).mip_levels(0).validate(8192),
            Err(RenderTargetError::InvalidMipLevels)
        );
        // The colour chain does not stop a depth attachment, which keeps a single level.
        assert_eq!(
            rgba([64, 64])
                .depth(RenderTargetDepth::Depth32Float)
                .mip_levels(2)
                .validate(8192),
            Ok(())
        );
        // Depth cannot be a storage texture.
        assert_eq!(
            RenderTargetDescriptor::new([64, 64])
                .depth(RenderTargetDepth::Depth32Float)
                .storage(StorageImageAccess::WriteOnly)
                .validate(8192),
            Err(RenderTargetError::StorageNeedsColorTarget)
        );
        assert_eq!(
            rgba([64, 64])
                .depth(RenderTargetDepth::Depth32Float)
                .storage(StorageImageAccess::ReadWrite)
                .validate(8192),
            Ok(())
        );
    }

    #[test]
    fn mip_views_and_storage_images_are_bound_by_name_and_level() {
        let rt = RenderTargetRef::from_parts(1, 0).unwrap();
        let view = RenderTargetMipView::new(rt, 1, 2, 3);
        assert_eq!((view.attachment, view.base_mip, view.mip_levels), (1, 2, 3));
        assert_eq!(RenderTargetMipView::single(rt, 0, 4).mip_levels, 1);
        let image = StorageImageBinding::new(rt, 0, 5);
        let bindings = DrawBindings::new()
            .texture("uPyramid", TextureSource::RenderTargetColorMip(view))
            .storage_image("uNext", image);
        assert!(matches!(
            bindings.entries[0],
            ("uPyramid", BindingValue::Texture(TextureSource::RenderTargetColorMip(v))) if v == view
        ));
        assert!(matches!(
            bindings.entries[1],
            ("uNext", BindingValue::StorageImage(i)) if i == image
        ));
        assert_eq!(OffscreenPass::new(rt).mip_level, 0);
        assert_eq!(OffscreenPass::new(rt).mip_level(3).mip_level, 3);
    }
}
