//! Vanilla's post-effect chains (`PostChainConfig`, `PostChain`, `PostPass`,
//! `GameRenderer.preparePostEffects` / `applyPostEffects`): a `post_effect/*.json`
//! names internal targets and passes (a vertex and a fragment shader, inputs read
//! from targets, the target written, std140 uniform blocks) that run over the
//! frame drawn so far, before the HUD.
//!
//! The JSON is read at run time from the active resource root, so a resource pack
//! can replace or add chains. So are the shaders the passes name: their GLSL
//! (`shaders/<path>.vsh` / `.fsh`, vanilla's or a pack's own) is translated to naga
//! by `post_glsl` and bound as vanilla binds it (`SamplerInfo`, `Globals`, the pass's
//! own uniform blocks, a sampler per input: a target's colour or depth, or a
//! `textures/effect/` image). Sunrise's own WGSL stand-ins (`post_chain.wgsl`) for the
//! vanilla `core/screenquad`, `post/rotscale`, `post/invert`, `post/blit`, `post/bits`,
//! `post/color_convolve`, `post/box_blur` and `post/spiderclip` are only the fallback
//! for a pass whose GLSL is missing or does not translate. A chain that cannot load
//! (an unknown shader without a stand-in, a target the frame has no equivalent of)
//! fails like a chain that does not compile (`GameRenderer.failedPostEffects`), once,
//! and is skipped.
//!
//! Every target keeps GL's rows (row 0 is the bottom edge): the frame is flipped into
//! the chain's `main` and back, and each pass draws with clip y negated.
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc;
use sunrise_assets::resource_source::ResourceSource;

/// `PostChain.MAIN_TARGET_ID`.
const MAIN: &str = "minecraft:main";
/// `PostPass`'s uniform buffers are 128 bytes.
const BLOCK_BYTES: usize = 128;
/// Our uniform buffers are bigger than the blocks they feed, so a shader that declares
/// more than vanilla writes still binds.
const UNIFORM_BYTES: u64 = 256;

/// `ResourceLocation.withDefaultNamespace` on an id read from a chain.
fn normalise(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// One uniform of a block (`UniformValue`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Uniform {
    Int(i32),
    IVec3([i32; 3]),
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Matrix([f32; 16]),
}

impl Uniform {
    /// std140 (alignment, size) in bytes: `Std140SizeCalculator`.
    fn layout(&self) -> (usize, usize) {
        match self {
            Uniform::Int(_) | Uniform::Float(_) => (4, 4),
            Uniform::Vec2(_) => (8, 8),
            Uniform::Vec3(_) | Uniform::IVec3(_) => (16, 12),
            Uniform::Vec4(_) => (16, 16),
            Uniform::Matrix(_) => (16, 64),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Uniform::Int(v) => out.extend_from_slice(&v.to_le_bytes()),
            Uniform::IVec3(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Float(v) => out.extend_from_slice(&v.to_le_bytes()),
            Uniform::Vec2(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Vec3(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Vec4(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Matrix(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
        }
    }
}

/// A uniform block in std140 order, as `PostPass` builds it with `Std140Builder`.
pub fn pack_block(values: &[Uniform]) -> Result<[u8; BLOCK_BYTES], String> {
    let mut bytes = Vec::new();
    for value in values {
        let (align, _) = value.layout();
        while bytes.len() % align != 0 {
            bytes.push(0);
        }
        value.write(&mut bytes);
    }
    if bytes.len() > BLOCK_BYTES {
        return Err(format!("a uniform block of {} bytes", bytes.len()));
    }
    let mut block = [0u8; BLOCK_BYTES];
    block[..bytes.len()].copy_from_slice(&bytes);
    Ok(block)
}

/// `PostChainConfig.InternalTarget`.
#[derive(Clone, Debug, PartialEq)]
pub struct InternalTarget {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub persistent: bool,
    /// `clear_color` as straight RGBA.
    pub clear: [f32; 4],
}

/// What an input reads: `PostChainConfig.TargetInput` or `TextureInput`.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A target's colour, or its depth buffer (`use_depth_buffer`).
    Target { id: String, depth: bool },
    /// `textures/effect/<location>.png`.
    Texture { location: String },
}

/// `PostChainConfig.Input`.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub sampler: String,
    pub source: Source,
    pub bilinear: bool,
}

/// `PostChainConfig.Pass`.
#[derive(Clone, Debug, PartialEq)]
pub struct PassConfig {
    pub vertex: String,
    pub fragment: String,
    pub inputs: Vec<Input>,
    pub output: String,
    pub uniforms: BTreeMap<String, Vec<Uniform>>,
}

/// `PostChainConfig`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub targets: BTreeMap<String, InternalTarget>,
    pub passes: Vec<PassConfig>,
}

fn floats<const N: usize>(v: &Value) -> Result<[f32; N], String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == N)
        .ok_or("a vector of the wrong length")?;
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64().ok_or("a vector entry that is not a number")? as f32;
    }
    Ok(out)
}

fn uniform(v: &Value) -> Result<Uniform, String> {
    let kind = v["type"].as_str().ok_or("a uniform without a type")?;
    let value = &v["value"];
    Ok(match kind {
        "int" => Uniform::Int(value.as_i64().ok_or("a bad int")? as i32),
        "float" => Uniform::Float(value.as_f64().ok_or("a bad float")? as f32),
        "vec2" => Uniform::Vec2(floats(value)?),
        "vec3" => Uniform::Vec3(floats(value)?),
        "vec4" => Uniform::Vec4(floats(value)?),
        "ivec3" => {
            let a = value
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or("a bad ivec3")?;
            let mut out = [0; 3];
            for (o, x) in out.iter_mut().zip(a) {
                *o = x.as_i64().ok_or("a bad ivec3")? as i32;
            }
            Uniform::IVec3(out)
        }
        "matrix4x4" => {
            let flat: Vec<f32> = match value.as_array() {
                Some(rows) if rows.first().is_some_and(Value::is_array) => rows
                    .iter()
                    .flat_map(|r| r.as_array().into_iter().flatten())
                    .filter_map(|x| x.as_f64().map(|x| x as f32))
                    .collect(),
                Some(flat) => flat
                    .iter()
                    .filter_map(|x| x.as_f64().map(|x| x as f32))
                    .collect(),
                None => Vec::new(),
            };
            Uniform::Matrix(flat.try_into().map_err(|_| "a bad matrix4x4")?)
        }
        other => return Err(format!("unknown uniform type {other}")),
    })
}

impl Config {
    /// `PostChainConfig.CODEC` over the file's text.
    pub fn parse(text: &str) -> Result<Self, String> {
        let json: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut targets = BTreeMap::new();
        if let Some(map) = json.get("targets").and_then(Value::as_object) {
            for (id, t) in map {
                let size = |key: &str| -> Result<Option<u32>, String> {
                    match t.get(key) {
                        None => Ok(None),
                        Some(v) => match v.as_u64() {
                            Some(n) if n > 0 => Ok(Some(n as u32)),
                            _ => Err(format!("target {id}: a bad {key}")),
                        },
                    }
                };
                let argb = t.get("clear_color").and_then(Value::as_u64).unwrap_or(0) as u32;
                let [a, r, g, b] = [24, 16, 8, 0].map(|s| ((argb >> s) & 255) as f32 / 255.0);
                targets.insert(
                    normalise(id),
                    InternalTarget {
                        width: size("width")?,
                        height: size("height")?,
                        persistent: t
                            .get("persistent")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        clear: [r, g, b, a],
                    },
                );
            }
        }
        let mut passes = Vec::new();
        for (n, p) in json
            .get("passes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let text = |key: &str| -> Result<String, String> {
                p.get(key)
                    .and_then(Value::as_str)
                    .map(normalise)
                    .ok_or(format!("pass {n}: no {key}"))
            };
            let mut inputs: Vec<Input> = Vec::new();
            for i in p
                .get("inputs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let sampler = i["sampler_name"]
                    .as_str()
                    .ok_or(format!("pass {n}: an input without sampler_name"))?;
                if inputs.iter().any(|x| x.sampler == sampler) {
                    return Err(format!("Encountered repeated sampler name: {sampler}"));
                }
                // Codec.xor(TextureInput, TargetInput): exactly one of them parses.
                let source = match (i.get("location"), i.get("target")) {
                    (Some(location), None) => {
                        for key in ["width", "height"] {
                            if !i.get(key).and_then(Value::as_u64).is_some_and(|v| v > 0) {
                                return Err(format!("pass {n}: a texture input needs a {key}"));
                            }
                        }
                        Source::Texture {
                            location: normalise(
                                location
                                    .as_str()
                                    .ok_or(format!("pass {n}: a bad location"))?,
                            ),
                        }
                    }
                    (None, Some(target)) => Source::Target {
                        id: normalise(target.as_str().ok_or(format!("pass {n}: a bad target"))?),
                        depth: i
                            .get("use_depth_buffer")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    },
                    _ => return Err(format!("pass {n}: an input needs a location or a target")),
                };
                inputs.push(Input {
                    sampler: sampler.to_owned(),
                    source,
                    bilinear: i.get("bilinear").and_then(Value::as_bool).unwrap_or(false),
                });
            }
            let mut uniforms = BTreeMap::new();
            if let Some(map) = p.get("uniforms").and_then(Value::as_object) {
                for (block, list) in map {
                    let list = list
                        .as_array()
                        .ok_or(format!("pass {n}: a bad uniform block"))?;
                    uniforms.insert(
                        block.clone(),
                        list.iter().map(uniform).collect::<Result<Vec<_>, _>>()?,
                    );
                }
            }
            passes.push(PassConfig {
                vertex: text("vertex_shader")?,
                fragment: text("fragment_shader")?,
                inputs,
                output: text("output")?,
                uniforms,
            });
        }
        Ok(Config { targets, passes })
    }

    /// `PostChain.getReferencedExternalTargets`.
    pub fn external_targets(&self) -> HashSet<&str> {
        let mut out = HashSet::new();
        for p in &self.passes {
            let targets = p.inputs.iter().filter_map(|i| match &i.source {
                Source::Target { id, .. } => Some(id),
                Source::Texture { .. } => None,
            });
            for id in targets.chain([&p.output]) {
                if !self.targets.contains_key(id) {
                    out.insert(id.as_str());
                }
            }
        }
        out
    }
}

/// A stand-in for a vanilla vertex shader: its entry point and uniform block.
const VERTEX: [(&str, &str, &str); 2] = [
    ("minecraft:core/screenquad", "vs_quad_gl", ""),
    ("minecraft:post/rotscale", "vs_rotscale", "RotScaleConfig"),
];
// The stand-ins are the fallback for a pass whose GLSL is missing or does not translate.
/// A stand-in for a vanilla fragment shader: entry point, uniform block and the
/// samplers it reads (`In`, then `Blur`).
const FRAGMENT: [(&str, &str, &str, usize); 6] = [
    ("minecraft:post/invert", "fs_invert", "InvertConfig", 1),
    ("minecraft:post/blit", "fs_blit", "BlitConfig", 1),
    ("minecraft:post/bits", "fs_bits", "BitsConfig", 1),
    (
        "minecraft:post/color_convolve",
        "fs_color_convolve",
        "ColorConfig",
        1,
    ),
    ("minecraft:post/box_blur", "fs_box_blur", "BlurConfig", 1),
    (
        "minecraft:post/spiderclip",
        "fs_spiderclip",
        "SpiderConfig",
        2,
    ),
];
const SAMPLERS: [&str; 2] = ["In", "Blur"];

/// A pass input resolved against the chain's targets.
#[derive(Clone, Copy)]
enum Src {
    Target {
        index: usize,
        depth: bool,
    },
    /// `Chain::images`.
    Image(usize),
}

#[derive(Clone, Copy)]
struct InputRef {
    src: Src,
    bilinear: bool,
}

/// What a translated shader's uniform block is bound to.
enum Block {
    /// `SamplerInfo`: `OutSize`, then each input's size.
    Info,
    /// `Globals` (`GlobalSettingsUniform`).
    Globals,
    /// One of the pass's `uniforms` groups.
    Custom(wgpu::Buffer),
    /// A block the request sets every frame (`Extras::blocks`), by name.
    Dynamic(String, wgpu::Buffer),
    /// `PolyGlobals`, the game's ([`Request::poly_globals`]), written every draw.
    PolyGlobals(wgpu::Buffer),
}

/// A pass running the resources' own GLSL, translated by `post_glsl`.
struct Translated {
    layout: wgpu::BindGroupLayout,
    vertex: wgpu::ShaderModule,
    fragment: wgpu::ShaderModule,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    /// Binding and what it holds.
    blocks: Vec<(u32, Block)>,
    /// A sampler's texture binding (the sampler object follows) and the input it reads.
    samplers: Vec<(u32, usize)>,
}

/// Why a translated pass is refused: one rule, whether the translator, the pass's configuration
/// or the device (its modules, layout or pipeline) refuses it. A pass naming vanilla's shaders
/// then runs on the built-in stand-ins; any other refuses its effect, named in the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PassRefusal {
    /// More `what` than the device binds: declared by both stages (`stage` none, checked before
    /// any analysis) or read by one stage.
    OverLimit {
        what: &'static str,
        stage: Option<wgpu::ShaderStages>,
        count: usize,
        limit: usize,
    },
    /// A `PolyGlobals` declaration the one rule refuses (`expression_blocks::check_poly_globals`).
    Globals(crate::gfx::expression_blocks::GlobalsRefusal),
    /// Its `uniforms` give `PolyGlobals`, the game's block.
    GlobalsGiven,
    /// The shader translator's refusal or fault.
    Translator(String),
    /// What the device refuses in the shader modules, the layout or the pipeline.
    Pipeline(String),
    /// The pass's own configuration (a missing target, input or uniform block).
    Config(String),
}

impl From<crate::post_glsl::Refused> for PassRefusal {
    fn from(refused: crate::post_glsl::Refused) -> Self {
        match refused {
            crate::post_glsl::Refused::Declares {
                what,
                declared,
                limit,
            } => Self::OverLimit {
                what,
                stage: None,
                count: declared,
                limit,
            },
            other => Self::Translator(other.to_string()),
        }
    }
}

impl std::fmt::Display for PassRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverLimit {
                what,
                stage: None,
                count,
                limit,
            } => write!(
                f,
                "it declares {count} {what}, past the {limit} the device binds"
            ),
            Self::OverLimit {
                what,
                stage: Some(stage),
                count,
                limit,
            } => write!(
                f,
                "its {stage:?} stage reads {count} {what}, past the device's {limit}"
            ),
            Self::Globals(refusal) => refusal.fmt(f),
            Self::GlobalsGiven => write!(
                f,
                "its uniforms give {}, which the game fills",
                sunrise_polytone::globals::BLOCK
            ),
            Self::Translator(e) | Self::Pipeline(e) | Self::Config(e) => f.write_str(e),
        }
    }
}

/// Why a chain doesn't load: one of its passes, or the chain itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LoadError {
    Pass(usize, PassRefusal),
    Chain(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pass(n, refusal) => write!(f, "pass {n}: {refusal}"),
            Self::Chain(e) => f.write_str(e),
        }
    }
}

impl From<String> for LoadError {
    fn from(e: String) -> Self {
        Self::Chain(e)
    }
}

impl From<&str> for LoadError {
    fn from(e: &str) -> Self {
        Self::Chain(e.to_owned())
    }
}

/// Whether a pass's bindings fit the device's limits for each stage (uniform blocks, sampled
/// textures, samplers), before the layout is made: the refusal names the stage.
fn within_limits(
    entries: &[wgpu::BindGroupLayoutEntry],
    limits: &wgpu::Limits,
) -> Result<(), PassRefusal> {
    for stage in [wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT] {
        let count = |kind: fn(&wgpu::BindingType) -> bool| {
            (entries.iter())
                .filter(|e| e.visibility.contains(stage) && kind(&e.ty))
                .count()
        };
        for (what, count, limit) in [
            (
                "uniform blocks",
                count(|t| matches!(t, wgpu::BindingType::Buffer { .. })),
                limits.max_uniform_buffers_per_shader_stage,
            ),
            (
                "textures",
                count(|t| matches!(t, wgpu::BindingType::Texture { .. })),
                limits.max_sampled_textures_per_shader_stage,
            ),
            (
                "samplers",
                count(|t| matches!(t, wgpu::BindingType::Sampler(_))),
                limits.max_samplers_per_shader_stage,
            ),
        ] {
            if count > limit as usize {
                return Err(PassRefusal::OverLimit {
                    what,
                    stage: Some(stage),
                    count,
                    limit: limit as usize,
                });
            }
        }
    }
    Ok(())
}

/// The error scopes a pack pass's GPU objects are made in: validation and internal errors, as the
/// packs' core shader pipelines have them.
fn gpu_scopes(device: &wgpu::Device) -> [wgpu::ErrorScopeGuard; 2] {
    [
        device.push_error_scope(wgpu::ErrorFilter::Validation),
        device.push_error_scope(wgpu::ErrorFilter::Internal),
    ]
}

/// What [`gpu_scopes`] caught, innermost first.
fn gpu_refusal([validation, internal]: [wgpu::ErrorScopeGuard; 2]) -> Option<String> {
    let internal = pollster::block_on(internal.pop());
    let validation = pollster::block_on(validation.pop());
    internal.or(validation).map(|e| e.to_string())
}

impl Translated {
    /// The pipeline for `format`, made once; a pipeline the device refuses is the refusal.
    fn ensure_pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> Result<(), String> {
        if self.pipelines.contains_key(&format) {
            return Ok(());
        }
        let scopes = gpu_scopes(device);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Post effect"),
            bind_group_layouts: &[Some(&self.layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Post effect pass"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.vertex,
                entry_point: Some("main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.fragment,
                entry_point: Some("main"),
                // ColorTargetState.DEFAULT: no blending.
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        if let Some(e) = gpu_refusal(scopes) {
            return Err(e);
        }
        self.pipelines.insert(format, pipeline);
        Ok(())
    }
}

struct Pass {
    /// Every input in the chain's order (translated passes).
    refs: Vec<InputRef>,
    translated: Option<Translated>,
    vs: &'static str,
    fs: &'static str,
    /// Per sampler slot: the target read and its filter.
    inputs: [(usize, bool); 2],
    output: usize,
    info: wgpu::Buffer,
    vs_block: wgpu::Buffer,
    fs_block: wgpu::Buffer,
    /// For a translated pass naming vanilla's shaders: the built-in stand-in it falls back to
    /// when the device refuses its pipeline, as a pass the translator refuses does.
    fallback: Option<Box<Pass>>,
}

impl Chain {
    /// Each translated pass's pipeline for `format`. A pass whose pipeline the device refuses
    /// is replaced by its built-in stand-in when it has one; else the refusal, named.
    fn ensure_pipelines(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> Result<(), LoadError> {
        for (n, pass) in self.passes.iter_mut().enumerate() {
            let Some(translated) = &mut pass.translated else {
                continue;
            };
            let Err(e) = translated.ensure_pipeline(device, format) else {
                continue;
            };
            match pass.fallback.take() {
                Some(standin) => {
                    log::debug!("post effect pass {n} uses the built-in shaders: {e}");
                    *pass = *standin;
                }
                None => return Err(LoadError::Pass(n, PassRefusal::Pipeline(e))),
            }
        }
        Ok(())
    }
}

/// A loaded chain (`PostChain`): its targets are 0 = the frame, then the internal
/// ones in id order.
struct Chain {
    targets: Vec<(String, InternalTarget)>,
    passes: Vec<Pass>,
    /// Textures for `targets[1..]`, with the size they were made at.
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// A depth buffer per target a pass reads the depth of (`DEFAULT_DEPTH`, cleared to 1;
    /// the frame's own when it has one).
    depths: Vec<Option<(wgpu::Texture, wgpu::TextureView)>>,
    /// The `textures/effect/` images the passes read, with their size.
    images: Vec<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
    size: [u32; 2],
}

/// What a frame asks for.
pub struct Request<'a> {
    /// `GameRenderer.requestedPostEffects`, in order.
    pub effects: &'a [String],
    /// The active resource root (`assets/<namespace>/post_effect/<path>.json`).
    pub root: &'a ResourceSource,
    /// `menuBackgroundBlurriness`, the radius of a `box_blur` pass with `Radius` 0.
    pub menu_blur_radius: f32,
    /// What the `Globals` block holds besides the screen size and the blur radius.
    pub globals: Globals,
    /// What resource packs' Polytone post chains add, when a pack has some.
    pub extras: Option<&'a Extras>,
    /// Builds the game's `PolyGlobals` for this frame: called at most once, and only when an
    /// applied effect has a pass reading the block (whichever effect it is in). Without it such
    /// a pass reads zeros.
    pub poly_globals: Option<&'a dyn Fn() -> crate::gfx::expression_blocks::PolyGlobalsBytes>,
}

/// What resource packs' Polytone post chains add to every chain (#11486, wiki "Shaders"): their
/// declared targets (`post_targets`), kept between frames, and uniform blocks set every frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    pub targets: BTreeMap<String, InternalTarget>,
    /// Each block's std140 bytes, by its name (an expression uniform's). `PolyGlobals` isn't one:
    /// every pass reading it is given the game's ([`Request::poly_globals`]).
    pub blocks: BTreeMap<String, Vec<u8>>,
}

/// `GlobalSettingsUniform.update`'s other values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Globals {
    pub camera_block_pos: [i32; 3],
    pub glint_alpha: f32,
    /// The camera's offset inside its block.
    pub camera_offset: [f32; 3],
    /// `(gameTime % 24000 + partialTick) / 24000`.
    pub game_time: f32,
    pub use_rgss: bool,
}

impl Globals {
    /// The std140 block: `ivec3, float, vec3, float, vec2, int, int`, written in place (no
    /// allocation: the pack pipelines ask for it every frame).
    pub(crate) fn bytes(&self, size: [u32; 2], menu_blur_radius: i32) -> [u8; 48] {
        let [x, y, z] = self.camera_block_pos.map(i32::to_le_bytes);
        let [ox, oy, oz] = self.camera_offset.map(f32::to_le_bytes);
        let words: [[u8; 4]; 12] = [
            x,
            y,
            z,
            self.glint_alpha.to_le_bytes(),
            ox,
            oy,
            oz,
            self.game_time.to_le_bytes(),
            (size[0] as f32).to_le_bytes(),
            (size[1] as f32).to_le_bytes(),
            menu_blur_radius.to_le_bytes(),
            i32::from(self.use_rgss).to_le_bytes(),
        ];
        let mut out = [0; 48];
        for (slot, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(words) {
            *slot = word;
        }
        out
    }
}

impl PostEffects {
    /// Whether the last frame's effects were given the game's `PolyGlobals`: only when one of
    /// them has a pass reading the block.
    pub fn gave_poly_globals(&self) -> bool {
        self.poly_globals.is_some()
    }
}

impl<'a> Request<'a> {
    /// No chains.
    pub fn none() -> Self {
        Request {
            effects: &[],
            root: empty_source(),
            menu_blur_radius: 0.0,
            globals: Globals::default(),
            extras: None,
            poly_globals: None,
        }
    }
}

/// Every chain, the pipelines and the frame copy they work on.
pub struct PostEffects {
    layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    pipelines: HashMap<(&'static str, &'static str, wgpu::TextureFormat), wgpu::RenderPipeline>,
    nearest: wgpu::Sampler,
    bilinear: wgpu::Sampler,
    /// `ShaderManager`'s chains by id; `None` when the file is missing.
    chains: HashMap<String, Option<Chain>>,
    /// `GameRenderer.failedPostEffects`.
    failed: HashSet<String>,
    root: ResourceSource,
    /// The frame as the chains see `minecraft:main`.
    work: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The frame as copied, before it is flipped into `work`.
    staging: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The flip between the frame and GL's rows.
    blit: Option<Pass>,
    /// The `Globals` block.
    globals: wgpu::Buffer,
    /// Chains loading on a thread of their own ([`PostEffects::preload`]), for the
    /// root they load from; `pending` until they arrive.
    preloading: Option<(ResourceSource, mpsc::Receiver<Preloaded>)>,
    #[cfg(test)]
    test_workers: Vec<std::thread::JoinHandle<()>>,
    pending: HashSet<String>,
    /// This frame's `Extras::blocks`.
    extra_blocks: BTreeMap<String, Vec<u8>>,
    /// This frame's `PolyGlobals` ([`Request::poly_globals`]), built only when an applied
    /// effect's pass reads them.
    poly_globals: Option<crate::gfx::expression_blocks::PolyGlobalsBytes>,
    /// The frame's depth with GL's rows, for the passes that read `minecraft:main`'s: the
    /// pipeline, and the texture at the frame's size.
    gl_depth: Option<(wgpu::BindGroupLayout, wgpu::RenderPipeline)>,
    gl_depth_target: Option<(wgpu::Texture, wgpu::TextureView)>,
}

#[cfg(test)]
impl Drop for PostEffects {
    fn drop(&mut self) {
        drop(self.preloading.take());
        let mut panic = None;
        for worker in self.test_workers.drain(..) {
            if let Err(error) = worker.join() {
                panic.get_or_insert(error);
            }
        }
        if !std::thread::panicking()
            && let Some(error) = panic
        {
            std::panic::resume_unwind(error);
        }
    }
}

/// The frame's depth with its rows flipped as the chains' colour is (row 0 at the bottom). Its
/// values stay: 26.3 keeps reverse-Z depth as Sunrise does (the sky is 0), and packs' post
/// shaders read it so.
const GL_DEPTH_WGSL: &str = r#"
@group(0) @binding(0) var depth: texture_depth_2d;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let size = textureDimensions(depth);
    let x = min(u32(at.x), size.x - 1u);
    let y = size.y - 1u - min(u32(at.y), size.y - 1u);
    return vec4<f32>(textureLoad(depth, vec2<u32>(x, y), 0), 0.0, 0.0, 1.0);
}
"#;

/// A chain [`PostEffects::preload`] loaded, by id.
type Preloaded = (String, Result<Option<Chain>, LoadError>);

fn empty_source() -> &'static ResourceSource {
    static EMPTY: std::sync::OnceLock<ResourceSource> = std::sync::OnceLock::new();
    EMPTY.get_or_init(|| ResourceSource::indexed(Default::default()))
}

fn resource_path(id: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("assets/{ns}/post_effect/{path}.json")
}

fn read_chain(
    root: &ResourceSource,
    id: &str,
    shaders: &crate::spectator_shaders::SpectatorShaders,
) -> Result<Option<String>, String> {
    let resource = resource_path(id);
    match root.read_to_string(&resource) {
        Ok(text) => Ok(Some(text)),
        Err(error) => {
            let entities = shaders.entities_for_chain(id);
            if entities.is_empty() {
                return Ok(None);
            }
            Err(format!(
                "Spectator entities {entities:?}, shader {id}, resource {resource}: {error}"
            ))
        }
    }
}

impl PostEffects {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Post effects"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post_chain.wgsl").into()),
        });
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Post effect bindings"),
            entries: &[
                uniform(0),
                uniform(1),
                uniform(2),
                texture(3),
                sampler(4),
                texture(5),
                sampler(6),
            ],
        });
        // SamplerCache.getClampToEdge.
        let make = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Post effect"),
                mag_filter: filter,
                min_filter: filter,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            })
        };
        Self {
            layout,
            shader,
            pipelines: HashMap::new(),
            nearest: make(wgpu::FilterMode::Nearest),
            bilinear: make(wgpu::FilterMode::Linear),
            chains: HashMap::new(),
            failed: HashSet::new(),
            preloading: None,
            #[cfg(test)]
            test_workers: Vec::new(),
            pending: HashSet::new(),
            root: empty_source().clone(),
            extra_blocks: BTreeMap::new(),
            poly_globals: None,
            gl_depth: None,
            gl_depth_target: None,
            work: None,
            staging: None,
            blit: None,
            globals: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Post effect globals"),
                size: UNIFORM_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    /// `GameRenderer.reloadPostEffects`-alike: forget chains loaded from another
    /// root, and the failures with them (`shouldResetFailedPostEffects`).
    fn use_root(&mut self, root: &ResourceSource) {
        if !self.root.same_generation(root) {
            self.root = root.to_owned();
            self.chains.clear();
            self.failed.clear();
            self.preloading = None;
            self.pending.clear();
        }
    }

    /// Loads the `ids` chains (namespaced) not loaded yet and compiles their pipelines for
    /// `format`, on a thread of their own: `apply` leaves a chain out until it has
    /// arrived instead of compiling it in the frame that first asks for it.
    pub fn preload<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        root: &ResourceSource,
        ids: impl IntoIterator<Item = &'a str>,
        format: wgpu::TextureFormat,
    ) {
        self.use_root(root);
        if self.preloading.is_some() {
            return;
        }
        let ids: Vec<String> = ids
            .into_iter()
            .filter(|id| !self.chains.contains_key(*id) && !self.failed.contains(*id))
            .map(normalise)
            .collect();
        if ids.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (device, queue, root) = (device.clone(), queue.clone(), root.to_owned());
        let format = format.remove_srgb_suffix();
        let spawned = std::thread::Builder::new()
            .name("Post effects".into())
            .spawn({
                let ids = ids.clone();
                move || {
                    for id in ids {
                        let mut chain = Self::load(&root, &device, &queue, &id, &Extras::default());
                        if let Ok(Some(loaded)) = &mut chain
                            && let Err(e) = loaded.ensure_pipelines(&device, format)
                        {
                            chain = Err(e);
                        }
                        if tx.send((id, chain)).is_err() {
                            return;
                        }
                    }
                }
            });
        if let Ok(worker) = spawned {
            #[cfg(test)]
            self.test_workers.push(worker);
            #[cfg(not(test))]
            drop(worker);
            self.pending.extend(ids);
            self.preloading = Some((self.root.clone(), rx));
        }
    }

    /// Takes the chains `preload` has finished.
    fn collect_preloaded(&mut self) {
        let Some((root, rx)) = &self.preloading else {
            return;
        };
        if !root.same_generation(&self.root) {
            self.preloading = None;
            self.pending.clear();
            return;
        }
        loop {
            match rx.try_recv() {
                Ok((id, loaded)) => {
                    self.pending.remove(&id);
                    let loaded = loaded.unwrap_or_else(|e| {
                        log::warn!("Failed to load post effect {id}: {e}");
                        self.failed.insert(id.clone());
                        None
                    });
                    self.chains.insert(id, loaded);
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.preloading = None;
                    self.pending.clear();
                    return;
                }
            }
        }
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        key: (&'static str, &'static str),
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let layout = &self.layout;
        let shader = &self.shader;
        self.pipelines
            .entry((key.0, key.1, format))
            .or_insert_with(|| {
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("Post effect"),
                        bind_group_layouts: &[Some(layout)],
                        ..Default::default()
                    });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Post effect pass"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: shader,
                        entry_point: Some(key.0),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: shader,
                        entry_point: Some(key.1),
                        // ColorTargetState.DEFAULT: no blending.
                        targets: &[Some(format.into())],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            })
    }

    /// `PostChain.load` for the file of `id`: `Ok(None)` when it does not exist,
    /// except registered spectator chains, whose missing resources are diagnosed.
    fn load(
        root: &ResourceSource,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: &str,
        extras: &Extras,
    ) -> Result<Option<Chain>, LoadError> {
        let Some(text) = read_chain(root, id, crate::spectator_shaders::table())? else {
            return Ok(None);
        };
        let mut config = Config::parse(&text)?;
        for (target, t) in &extras.targets {
            config
                .targets
                .entry(target.clone())
                .or_insert_with(|| t.clone());
        }
        // isPostEffectValid / LevelTargetBundle.MAIN_TARGETS: the frame is the one
        // external target Sunrise has.
        if let Some(bad) = config.external_targets().into_iter().find(|t| *t != MAIN) {
            return Err(format!(
                "Referenced external targets are not available in this context: [{bad}]"
            )
            .into());
        }
        let targets: Vec<(String, InternalTarget)> = std::iter::once((
            MAIN.to_owned(),
            InternalTarget {
                width: None,
                height: None,
                persistent: true,
                clear: [0.0; 4],
            },
        ))
        .chain(config.targets.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();
        let index = |name: &str| targets.iter().position(|(t, _)| t == name);
        let mut images = Vec::new();
        let mut passes = Vec::new();
        for (n, p) in config.passes.iter().enumerate() {
            let output = index(&p.output).ok_or("a target is missing")?;
            let pass = match Self::translated_pass(
                root,
                device,
                queue,
                p,
                output,
                &index,
                &mut images,
                extras,
            ) {
                Ok(mut pass) => {
                    // What a pipeline the device refuses falls back to, as below.
                    pass.fallback = (Self::standin_pass(device, queue, n, p, output, &index))
                        .ok()
                        .map(Box::new);
                    pass
                }
                Err(e) => match Self::standin_pass(device, queue, n, p, output, &index) {
                    Ok(pass) => {
                        log::debug!("post effect {id} pass {n} uses the built-in shaders: {e}");
                        pass
                    }
                    Err(_) => return Err(LoadError::Pass(n, e)),
                },
            };
            passes.push(pass);
        }
        Ok(Some(Chain {
            targets,
            passes,
            textures: Vec::new(),
            depths: Vec::new(),
            images,
            size: [0, 0],
        }))
    }

    /// A pass over the resources' own GLSL (`PostChain.createPass`).
    #[allow(clippy::too_many_arguments)]
    fn translated_pass(
        root: &ResourceSource,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        p: &PassConfig,
        output: usize,
        index: &dyn Fn(&str) -> Option<usize>,
        images: &mut Vec<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
        extras: &Extras,
    ) -> Result<Pass, PassRefusal> {
        let globals = sunrise_polytone::globals::BLOCK;
        if p.uniforms.contains_key(globals) {
            return Err(PassRefusal::GlobalsGiven);
        }
        let limits = device.limits();
        // A pack's GLSL is untrusted: a fault the translator meets refuses the pass. Declaring
        // more than the device binds refuses it before any analysis.
        let declarable = crate::post_glsl::Declarable::of(&limits);
        let program = crate::crash::contain(|| {
            crate::post_glsl::translate(root, &p.vertex, &p.fragment, declarable)
        })
        .map_err(|e| PassRefusal::Translator(format!("the shader translator failed: {e}")))?
        .map_err(PassRefusal::from)?;
        let modules = [&program.vertex, &program.fragment];
        // What each stage reads, from the translator's validation of each stage.
        let read = &program.read;
        let config = PassRefusal::Config;
        let mut refs = Vec::new();
        for input in &p.inputs {
            let src = match &input.source {
                Source::Target { id, depth } => {
                    let target = index(id).ok_or(config("a target is missing".into()))?;
                    if target == output && !depth {
                        return Err(config("it reads the target it writes".into()));
                    }
                    Src::Target {
                        index: target,
                        depth: *depth,
                    }
                }
                Source::Texture { location } => {
                    images.push(Self::image(root, device, queue, location).map_err(config)?);
                    Src::Image(images.len() - 1)
                }
            };
            refs.push(InputRef {
                src,
                bilinear: input.bilinear,
            });
        }
        let mut pass = Pass::new(device, "", "", [(0, false); 2], output);
        let mut entries = Vec::new();
        let mut blocks = Vec::new();
        for (name, binding) in &program.blocks {
            if name == globals {
                crate::gfx::expression_blocks::check_poly_globals(modules, *binding)
                    .map_err(PassRefusal::Globals)?;
            }
            // Bound only to the stages that read it: one only declared (an imported block the
            // shader doesn't use) binds nothing.
            let stages = crate::core_shader::stages_reading(read, *binding);
            if stages.is_empty() {
                continue;
            }
            let block = match name.as_str() {
                "SamplerInfo" => Block::Info,
                "Globals" => Block::Globals,
                // The game's, written every draw ([`Request::poly_globals`]), whichever effect
                // the pass is in: never a pass's own values.
                other if other == globals => {
                    Block::PolyGlobals(device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Post effect PolyGlobals"),
                        size: UNIFORM_BYTES,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }))
                }
                other if !p.uniforms.contains_key(other) && extras.blocks.contains_key(other) => {
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Post effect pack uniforms"),
                        size: UNIFORM_BYTES,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    Block::Dynamic(other.to_owned(), buffer)
                }
                other => {
                    let values = (p.uniforms.get(other))
                        .ok_or_else(|| config(format!("the uniform block {other} is missing")))?;
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Post effect uniforms"),
                        size: UNIFORM_BYTES,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    queue.write_buffer(&buffer, 0, &pack_block(values).map_err(config)?);
                    Block::Custom(buffer)
                }
            };
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: *binding,
                visibility: stages,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
            blocks.push((*binding, block));
        }
        let mut samplers = Vec::new();
        for (name, texture) in &program.samplers {
            let input = name.strip_suffix("Sampler").unwrap_or(name);
            let slot = (p.inputs.iter())
                .position(|i| i.sampler == input)
                .ok_or_else(|| config(format!("no input for the sampler {name}")))?;
            let stages = crate::core_shader::stages_reading(read, *texture);
            if stages.is_empty() {
                continue;
            }
            // A depth buffer is read unfiltered.
            let depth = matches!(refs[slot].src, Src::Target { depth: true, .. });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: *texture,
                visibility: stages,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: !depth },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: texture + 1,
                visibility: stages,
                ty: wgpu::BindingType::Sampler(if depth {
                    wgpu::SamplerBindingType::NonFiltering
                } else {
                    wgpu::SamplerBindingType::Filtering
                }),
                count: None,
            });
            samplers.push((*texture, slot));
        }
        let module = |module: naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Post effect shader"),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
            })
        };
        within_limits(&entries, &limits)?;
        // Anything else the device refuses in the modules or the layout refuses the pass too.
        let scopes = gpu_scopes(device);
        let translated = Translated {
            layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Post effect bindings"),
                entries: &entries,
            }),
            vertex: module(program.vertex),
            fragment: module(program.fragment),
            pipelines: HashMap::new(),
            blocks,
            samplers,
        };
        gpu_refusal(scopes).map_or(Ok(()), |e| Err(PassRefusal::Pipeline(e)))?;
        pass.refs = refs;
        pass.translated = Some(translated);
        Ok(pass)
    }

    /// A pass over Sunrise's WGSL stand-ins for the vanilla post shaders.
    fn standin_pass(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        n: usize,
        p: &PassConfig,
        output: usize,
        index: &dyn Fn(&str) -> Option<usize>,
    ) -> Result<Pass, String> {
        let &(_, vs, vs_block) = VERTEX
            .iter()
            .find(|v| v.0 == p.vertex)
            .ok_or(format!("pass {n}: no vertex shader {}", p.vertex))?;
        let &(_, fs, fs_block, samplers) = FRAGMENT
            .iter()
            .find(|f| f.0 == p.fragment)
            .ok_or(format!("pass {n}: no fragment shader {}", p.fragment))?;
        let mut inputs = [(0, false); 2];
        for (slot, name) in SAMPLERS.iter().enumerate().take(samplers) {
            let input = p
                .inputs
                .iter()
                .find(|i| i.sampler == *name)
                .ok_or(format!("pass {n}: {} needs the sampler {name}", p.fragment))?;
            let Source::Target { id, depth: false } = &input.source else {
                return Err(format!(
                    "pass {n}: the built-in shaders read target colours"
                ));
            };
            let target = index(id).ok_or("a target is missing")?;
            if target == output {
                return Err(format!("pass {n}: reads the target it writes"));
            }
            inputs[slot] = (target, input.bilinear);
        }
        // A shader reading one sampler binds the same texture to the other,
        // never the target it writes.
        if samplers == 1 {
            inputs[1] = inputs[0];
        }
        let block = |name: &str| match p.uniforms.get(name) {
            Some(values) => pack_block(values),
            None if name.is_empty() => Ok([0; BLOCK_BYTES]),
            None => Err(format!("pass {n}: the uniform block {name} is missing")),
        };
        let pass = Pass::new(device, vs, fs, inputs, output);
        queue.write_buffer(&pass.vs_block, 0, &block(vs_block)?);
        queue.write_buffer(&pass.fs_block, 0, &block(fs_block)?);
        Ok(pass)
    }

    /// `textures/effect/<location>.png` as a texture (`PostChain.createPass`'s `TextureInput`).
    fn image(
        root: &ResourceSource,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        location: &str,
    ) -> Result<(wgpu::Texture, wgpu::TextureView, [u32; 2]), String> {
        let (ns, path) = location.split_once(':').unwrap_or(("minecraft", location));
        let file = format!("assets/{ns}/textures/effect/{path}.png");
        let bytes = root.read(&file).map_err(|e| format!("{file}: {e}"))?;
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        // As the other resource-pack images: stb_image never checks chunk CRCs.
        decoder.ignore_checksums(true);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let (width, height) = (reader.info().width, reader.info().height);
        crate::hud::check_pack_size(width, height).map_err(|e| e.to_string())?;
        // The decode ceiling is not this device's: an image no texture of it holds is refused
        // before its pixels are made, not handed to validation.
        let max = device.limits().max_texture_dimension_2d;
        if width.max(height) > max {
            return Err(format!(
                "a {width}x{height} image is larger than this device's {max}-pixel textures"
            ));
        }
        let buffer = |factors: &[usize]| {
            crate::hud::image_buffer::<u8>(factors, crate::hud::MAX_IMAGE_BYTES)
                .map_err(|e| e.to_string())
        };
        let size = reader.output_buffer_size().unwrap_or(usize::MAX);
        let mut data = buffer(&[size])?;
        data.resize(size, 0);
        let info = reader.next_frame(&mut data).map_err(|e| e.to_string())?;
        let pixels = &data[..info.buffer_size()];
        let mut rgba = buffer(&[info.width as usize, info.height as usize, 4])?;
        match info.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(pixels),
            png::ColorType::Rgb => rgba.extend(
                pixels
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .flat_map(|p| [p[0], p[1], p[2], 255]),
            ),
            png::ColorType::Grayscale => rgba.extend(pixels.iter().flat_map(|g| [*g, *g, *g, 255])),
            png::ColorType::GrayscaleAlpha => rgba.extend(
                pixels
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .flat_map(|p| [p[0], p[0], p[0], p[1]]),
            ),
            png::ColorType::Indexed => return Err("an indexed image".into()),
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Post effect image"),
            size: wgpu::Extent3d {
                width: info.width,
                height: info.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(info.width * 4),
                rows_per_image: Some(info.height),
            },
            wgpu::Extent3d {
                width: info.width,
                height: info.height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&Default::default());
        Ok((texture, view, [info.width, info.height]))
    }

    fn texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: [u32; 2],
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Post effect target"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        (texture, view)
    }

    /// Runs the chains `req` names over `frame` (the frame's texture, copied from)
    /// and puts the result on `target` (its view): `GameRenderer.preparePostEffects`
    /// then `applyPostEffects`. `frame` must allow `COPY_SRC`. `depth` is the frame's
    /// depth buffer, for a pass that reads the depth of `minecraft:main` (it must
    /// allow `TEXTURE_BINDING`; without it that depth reads as 1). Returns whether
    /// any chain ran.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
        target: &wgpu::TextureView,
        size: [u32; 2],
        depth: Option<&wgpu::TextureView>,
        req: &Request,
        mut timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) -> bool {
        self.use_root(req.root);
        self.collect_preloaded();
        self.extra_blocks = req.extras.map(|e| e.blocks.clone()).unwrap_or_default();
        self.poly_globals = None;
        let mut applied = Vec::new();
        for effect in req.effects {
            let id = normalise(effect);
            // A chain still compiling on `preload`'s thread waits for it.
            if self.failed.contains(&id) || applied.contains(&id) || self.pending.contains(&id) {
                continue;
            }
            if !self.chains.contains_key(&id) {
                let empty = Extras::default();
                let extras = req.extras.unwrap_or(&empty);
                let loaded = match Self::load(&self.root, device, queue, &id, extras) {
                    Ok(chain) => chain,
                    Err(e) => {
                        log::warn!("Failed to load post effect {id}: {e}");
                        self.failed.insert(id.clone());
                        None
                    }
                };
                self.chains.insert(id.clone(), loaded);
            }
            if matches!(self.chains.get(&id), Some(Some(_))) {
                applied.push(id);
            }
        }
        let format = frame.format().remove_srgb_suffix();
        // A pass whose pipeline the device refuses refuses its effect, named, which is skipped.
        applied.retain(|id| {
            let Some(Some(chain)) = self.chains.get_mut(id) else {
                return false;
            };
            let Err(e) = chain.ensure_pipelines(device, format) else {
                return true;
            };
            log::warn!("Failed to load post effect {id}: {e}");
            self.failed.insert(id.clone());
            self.chains.insert(id.clone(), None);
            false
        });
        if applied.is_empty() {
            return false;
        }
        let reads_globals = |chain: &Chain| {
            (chain.passes.iter().filter_map(|p| p.translated.as_ref())).any(|t| {
                t.blocks
                    .iter()
                    .any(|(_, b)| matches!(b, Block::PolyGlobals(_)))
            })
        };
        if applied
            .iter()
            .any(|id| matches!(self.chains.get(id), Some(Some(c)) if reads_globals(c)))
        {
            self.poly_globals = req.poly_globals.map(|build| build());
        }
        if self.work.as_ref().is_none_or(|(t, _)| {
            t.size().width != size[0] || t.size().height != size[1] || t.format() != format
        }) {
            self.work = Some(Self::texture(device, format, size));
        }
        if self.staging.as_ref().is_none_or(|(t, _)| {
            t.size().width != size[0] || t.size().height != size[1] || t.format() != format
        }) {
            self.staging = Some(Self::texture(device, format, size));
        }
        queue.write_buffer(
            &self.globals,
            0,
            &req.globals.bytes(size, req.menu_blur_radius as i32),
        );
        if self.blit.is_none() {
            let pass = Pass::new(device, "vs_quad", "fs_blit", [(0, false); 2], 0);
            queue.write_buffer(&pass.vs_block, 0, &[0; BLOCK_BYTES]);
            queue.write_buffer(
                &pass.fs_block,
                0,
                &[1.0f32; 32].map(f32::to_le_bytes).concat(),
            );
            self.blit = Some(pass);
        }
        let copy_size = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let work = self.work.take().expect("made above");
        let staging = self.staging.take().expect("made above");
        let blit = self.blit.take().expect("made above");
        // The chains see GL's rows: flip the frame in, and back out below.
        encoder.copy_texture_to_texture(
            frame.as_image_copy(),
            staging.0.as_image_copy(),
            copy_size,
        );
        self.run_pass(
            device,
            queue,
            encoder,
            &blit,
            [(&staging.1, size), (&staging.1, size)],
            (&work.1, size),
            0.0,
            format,
            timing.as_deref_mut(),
        );
        self.staging = Some(staging);
        if let Some(view) = depth {
            self.gl_depth(device, encoder, view, size);
        }
        let gl_depth = self.gl_depth_target.take();
        let depth = depth.and(gl_depth.as_ref().map(|(_, v)| v));
        for id in &applied {
            let mut chain = self.chains.remove(id).flatten().expect("loaded above");
            self.run_chain(
                device,
                queue,
                encoder,
                &work.1,
                &mut chain,
                size,
                depth,
                req.menu_blur_radius,
                format,
                timing.as_deref_mut(),
            );
            self.chains.insert(id.clone(), Some(chain));
        }
        // The frame, as the chains left it.
        self.run_pass(
            device,
            queue,
            encoder,
            &blit,
            [(&work.1, size), (&work.1, size)],
            (target, size),
            0.0,
            format,
            timing,
        );
        self.blit = Some(blit);
        self.work = Some(work);
        self.gl_depth_target = gl_depth;
        true
    }

    /// Copies `depth` (the frame's) into `gl_depth_target` with GL's rows, at `size`.
    fn gl_depth(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        size: [u32; 2],
    ) {
        let (layout, pipeline) = self.gl_depth.get_or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Post effect GL depth"),
                source: wgpu::ShaderSource::Wgsl(GL_DEPTH_WGSL.into()),
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Post effect GL depth"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Post effect GL depth"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Post effect GL depth"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::TextureFormat::R32Float.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
            (layout, pipeline)
        });
        if self
            .gl_depth_target
            .as_ref()
            .is_none_or(|(t, _)| t.size().width != size[0] || t.size().height != size[1])
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Post effect GL depth"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.gl_depth_target = Some((texture, view));
        }
        let (_, target) = self.gl_depth_target.as_ref().expect("made above");
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Post effect GL depth"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Post effect GL depth"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    #[allow(clippy::too_many_arguments)]
    fn run_chain(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        main: &wgpu::TextureView,
        chain: &mut Chain,
        size: [u32; 2],
        main_depth: Option<&wgpu::TextureView>,
        menu_radius: f32,
        format: wgpu::TextureFormat,
        mut timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) {
        let sizes: Vec<[u32; 2]> = chain
            .targets
            .iter()
            .map(|(_, t)| [t.width.unwrap_or(size[0]), t.height.unwrap_or(size[1])])
            .collect();
        // PostChain.addToFrame: internal targets at their size, cleared to their
        // colour; a persistent one only when it is made.
        let remade = chain.size != size || chain.textures.len() + 1 != chain.targets.len();
        if remade {
            chain.size = size;
            chain.textures = sizes[1..]
                .iter()
                .map(|s| Self::texture(device, format, *s))
                .collect();
            let mut wanted = vec![false; chain.targets.len()];
            for r in chain.passes.iter().flat_map(|p| &p.refs) {
                if let Src::Target { index, depth: true } = r.src {
                    wanted[index] = true;
                }
            }
            chain.depths = wanted
                .iter()
                .zip(&sizes)
                .map(|(wanted, s)| {
                    wanted.then(|| {
                        let texture = device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("Post effect depth"),
                            size: wgpu::Extent3d {
                                width: s[0],
                                height: s[1],
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Depth32Float,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::RENDER_ATTACHMENT,
                            view_formats: &[],
                        });
                        let view = texture.create_view(&Default::default());
                        (texture, view)
                    })
                })
                .collect();
        }
        // The depth of a target nothing draws depth into stays at the far plane.
        for (i, depth) in chain.depths.iter().enumerate() {
            if let Some((_, view)) = depth
                && !(i == 0 && main_depth.is_some())
            {
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Post effect depth clear"),
                    timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes("Post clear")),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
            }
        }
        for (i, (_, t)) in chain.targets.iter().enumerate().skip(1) {
            if remade || !t.persistent {
                let c = t.clear.map(f64::from);
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Post effect target clear"),
                    timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes("Post clear")),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &chain.textures[i - 1].1,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: c[0],
                                g: c[1],
                                b: c[2],
                                a: c[3],
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            }
        }
        let chain = &*chain;
        let view = |i: usize| {
            if i == 0 {
                main
            } else {
                &chain.textures[i - 1].1
            }
        };
        let depth_view = |i: usize| match (i, main_depth) {
            (0, Some(depth)) => depth,
            _ => &chain.depths[i].as_ref().expect("made for the pass").1,
        };
        for pass in &chain.passes {
            if let Some(translated) = &pass.translated {
                let inputs: Vec<(&wgpu::TextureView, [u32; 2])> = pass
                    .refs
                    .iter()
                    .map(|r| match r.src {
                        Src::Target {
                            index,
                            depth: false,
                        } => (view(index), sizes[index]),
                        Src::Target { index, depth: true } => (depth_view(index), sizes[index]),
                        Src::Image(i) => (&chain.images[i].1, chain.images[i].2),
                    })
                    .collect();
                self.draw_translated(
                    device,
                    queue,
                    encoder,
                    pass,
                    translated,
                    &inputs,
                    (view(pass.output), sizes[pass.output]),
                    format,
                    timing.as_deref_mut(),
                );
                continue;
            }
            let [a, b] = pass.inputs.map(|(t, _)| (view(t), sizes[t]));
            self.run_pass(
                device,
                queue,
                encoder,
                pass,
                [a, b],
                (view(pass.output), sizes[pass.output]),
                menu_radius,
                format,
                timing.as_deref_mut(),
            );
        }
    }

    /// `PostPass.addToFrame`'s execution for a translated pass.
    #[allow(clippy::too_many_arguments)]
    fn draw_translated(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &Pass,
        translated: &Translated,
        inputs: &[(&wgpu::TextureView, [u32; 2])],
        output: (&wgpu::TextureView, [u32; 2]),
        format: wgpu::TextureFormat,
        timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) {
        let Some(pipeline) = translated.pipelines.get(&format) else {
            return;
        };
        // SamplerInfo: OutSize, then each input's size.
        let sizes = std::iter::once(output.1).chain(inputs.iter().map(|i| i.1));
        let info: Vec<u8> = sizes
            .flat_map(|s| s.map(|v| (v as f32).to_le_bytes()))
            .flatten()
            .collect();
        queue.write_buffer(
            &pass.info,
            0,
            &info[..info.len().min(UNIFORM_BYTES as usize)],
        );
        let mut entries = Vec::new();
        for (binding, block) in &translated.blocks {
            let buffer = match block {
                Block::Info => &pass.info,
                Block::Globals => &self.globals,
                Block::Custom(buffer) => buffer,
                Block::PolyGlobals(buffer) => {
                    let zeros = [0; sunrise_polytone::globals::BLOCK_SIZE];
                    queue.write_buffer(buffer, 0, self.poly_globals.as_ref().unwrap_or(&zeros));
                    buffer
                }
                Block::Dynamic(name, buffer) => {
                    if let Some(bytes) = self.extra_blocks.get(name) {
                        let n = bytes.len().min(UNIFORM_BYTES as usize) & !3;
                        queue.write_buffer(buffer, 0, &bytes[..n]);
                    }
                    buffer
                }
            };
            entries.push(wgpu::BindGroupEntry {
                binding: *binding,
                resource: buffer.as_entire_binding(),
            });
        }
        for (binding, slot) in &translated.samplers {
            let ref_ = &pass.refs[*slot];
            let depth = matches!(ref_.src, Src::Target { depth: true, .. });
            entries.push(wgpu::BindGroupEntry {
                binding: *binding,
                resource: wgpu::BindingResource::TextureView(inputs[*slot].0),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: binding + 1,
                resource: wgpu::BindingResource::Sampler(if ref_.bilinear && !depth {
                    &self.bilinear
                } else {
                    &self.nearest
                }),
            });
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Post effect pass"),
            layout: &translated.layout,
            entries: &entries,
        });
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Post effect pass"),
            timestamp_writes: timing.and_then(|t| t.writes("Post effect")),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output.0,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, &group, &[]);
        rp.draw(0..3, 0..1);
    }

    /// `PostPass.addToFrame`'s execution: one fullscreen triangle.
    #[allow(clippy::too_many_arguments)]
    fn run_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &Pass,
        inputs: [(&wgpu::TextureView, [u32; 2]); 2],
        output: (&wgpu::TextureView, [u32; 2]),
        menu_radius: f32,
        format: wgpu::TextureFormat,
        timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) {
        // SamplerInfo: OutSize, then each input's size.
        let info = [
            output.1[0] as f32,
            output.1[1] as f32,
            inputs[0].1[0] as f32,
            inputs[0].1[1] as f32,
            inputs[1].1[0] as f32,
            inputs[1].1[1] as f32,
            menu_radius,
            0.0,
        ];
        queue.write_buffer(&pass.info, 0, &info.map(f32::to_le_bytes).concat());
        self.pipeline(device, (pass.vs, pass.fs), format);
        let Some(pipeline) = self.pipelines.get(&(pass.vs, pass.fs, format)) else {
            return;
        };
        let sampler = |bilinear| {
            if bilinear {
                &self.bilinear
            } else {
                &self.nearest
            }
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Post effect pass"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pass.info.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pass.vs_block.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: pass.fs_block.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(inputs[0].0),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(sampler(pass.inputs[0].1)),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(inputs[1].0),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(sampler(pass.inputs[1].1)),
                },
            ],
        });
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Post effect pass"),
            timestamp_writes: timing.and_then(|t| t.writes("Post effect")),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output.0,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, &group, &[]);
        rp.draw(0..3, 0..1);
    }
}

impl Pass {
    fn new(
        device: &wgpu::Device,
        vs: &'static str,
        fs: &'static str,
        inputs: [(usize, bool); 2],
        output: usize,
    ) -> Self {
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Pass {
            vs,
            fs,
            inputs,
            output,
            info: buffer("Post effect sampler info", UNIFORM_BYTES),
            vs_block: buffer("Post effect vertex uniforms", BLOCK_BYTES as u64),
            fs_block: buffer("Post effect fragment uniforms", BLOCK_BYTES as u64),
            refs: Vec::new(),
            translated: None,
            fallback: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const INVERT_THEN_BLIT: &str = r#"{
        "targets": { "swap": {} },
        "passes": [
            { "vertex_shader": "minecraft:core/screenquad",
              "fragment_shader": "minecraft:post/invert",
              "inputs": [ { "sampler_name": "In", "target": "minecraft:main" } ],
              "output": "swap",
              "uniforms": { "InvertConfig": [ { "name": "InverseAmount", "type": "float", "value": 1.0 } ] } },
            { "vertex_shader": "minecraft:core/screenquad",
              "fragment_shader": "minecraft:post/blit",
              "inputs": [ { "sampler_name": "In", "target": "swap", "bilinear": true } ],
              "output": "minecraft:main",
              "uniforms": { "BlitConfig": [ { "name": "ColorModulate", "type": "vec4", "value": [1.0, 0.0, 0.5, 1.0] } ] } }
        ]
    }"#;

    /// `Globals::bytes` lays the block out in std140 order, each value little-endian in its own
    /// four bytes.
    #[test]
    fn globals_lay_out_their_block() {
        let globals = Globals {
            camera_block_pos: [1, -2, 3],
            glint_alpha: 0.375,
            camera_offset: [0.25, 0.5, 0.75],
            game_time: 0.125,
            use_rgss: true,
        };
        let expected: [u8; 48] = [
            0x01, 0x00, 0x00, 0x00, 0xfe, 0xff, 0xff, 0xff, 0x03, 0x00, 0x00, 0x00, // ivec3
            0x00, 0x00, 0xc0, 0x3e, // glint alpha 0.375
            0x00, 0x00, 0x80, 0x3e, 0x00, 0x00, 0x00, 0x3f, 0x00, 0x00, 0x40, 0x3f, // vec3
            0x00, 0x00, 0x00, 0x3e, // game time 0.125
            0x00, 0x00, 0x20, 0x44, 0x00, 0x00, 0xf0, 0x43, // vec2 640, 480
            0x07, 0x00, 0x00, 0x00, // menu blur radius 7
            0x01, 0x00, 0x00, 0x00, // rgss on
        ];
        assert_eq!(globals.bytes([640, 480], 7), expected);
    }

    #[test]
    fn shaders_validate() {
        let module = naga::front::wgsl::parse_str(include_str!("post_chain.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn a_chain_is_read_like_postchainconfig() {
        let config = Config::parse(INVERT_THEN_BLIT).unwrap();
        // Ids without a namespace are minecraft's.
        assert_eq!(
            config.targets.keys().collect::<Vec<_>>(),
            ["minecraft:swap"]
        );
        assert_eq!(config.passes.len(), 2);
        assert_eq!(config.passes[0].output, "minecraft:swap");
        assert_eq!(
            config.passes[1].inputs,
            [Input {
                sampler: "In".into(),
                source: Source::Target {
                    id: "minecraft:swap".into(),
                    depth: false
                },
                bilinear: true
            }]
        );
        assert_eq!(
            config.passes[1].uniforms["BlitConfig"],
            [Uniform::Vec4([1.0, 0.0, 0.5, 1.0])]
        );
        // Only the frame is outside the chain.
        assert_eq!(config.external_targets(), HashSet::from(["minecraft:main"]));
    }

    #[test]
    fn target_options_and_clear_colour() {
        let config = Config::parse(
            r#"{"targets":{"a":{"width":16,"height":8,"persistent":true,"clear_color":2180972544}}}"#,
        )
        .unwrap();
        let t = &config.targets["minecraft:a"];
        assert_eq!((t.width, t.height, t.persistent), (Some(16), Some(8), true));
        // 0x81FF0000: alpha 129, red 255.
        assert_eq!(t.clear, [1.0, 0.0, 0.0, 129.0 / 255.0]);
        assert!(Config::parse(r#"{"targets":{"a":{"width":0}}}"#).is_err());
    }

    #[test]
    fn inputs_are_checked() {
        let pass = |inputs: &str| {
            format!(
                r#"{{"passes":[{{"vertex_shader":"a","fragment_shader":"b","output":"o","inputs":{inputs}}}]}}"#
            )
        };
        // Repeated sampler names are refused.
        assert!(
            Config::parse(&pass(
                r#"[{"sampler_name":"In","target":"x"},{"sampler_name":"In","target":"y"}]"#
            ))
            .is_err()
        );
        // A texture input needs its size; an input is a target or a texture, not both.
        assert!(Config::parse(&pass(r#"[{"sampler_name":"In","location":"noise"}]"#)).is_err());
        assert!(
            Config::parse(&pass(
                r#"[{"sampler_name":"In","location":"noise","target":"x","width":4,"height":4}]"#
            ))
            .is_err()
        );
        let config = Config::parse(&pass(
            r#"[{"sampler_name":"A","location":"noise","width":4,"height":4,"bilinear":true},
                {"sampler_name":"B","target":"x","use_depth_buffer":true}]"#,
        ))
        .unwrap();
        assert_eq!(
            config.passes[0].inputs[0].source,
            Source::Texture {
                location: "minecraft:noise".into()
            }
        );
        assert_eq!(
            config.passes[0].inputs[1].source,
            Source::Target {
                id: "minecraft:x".into(),
                depth: true
            }
        );
        // Only targets can be external.
        assert_eq!(
            config.external_targets(),
            HashSet::from(["minecraft:x", "minecraft:o"])
        );
    }

    #[test]
    fn blocks_are_packed_as_std140() {
        // BlurConfig { vec2 BlurDir; float Radius }.
        let block = pack_block(&[Uniform::Vec2([1.0, 0.0]), Uniform::Float(7.0)]).unwrap();
        let words: Vec<f32> = block
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(words[..3], [1.0, 0.0, 7.0]);
        // ColorConfig { vec3, vec3, vec3 }: each starts on 16 bytes.
        let block = pack_block(&[
            Uniform::Vec3([1.0, 2.0, 3.0]),
            Uniform::Vec3([4.0, 5.0, 6.0]),
        ])
        .unwrap();
        let words: Vec<f32> = block
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(words[..7], [1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 6.0]);
        // RotScaleConfig { vec2, vec2, float }.
        let block = pack_block(&[
            Uniform::Vec2([1.0, 2.0]),
            Uniform::Vec2([3.0, 4.0]),
            Uniform::Float(5.0),
        ])
        .unwrap();
        let words: Vec<f32> = block
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(words[..5], [1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!(pack_block(&[Uniform::Matrix([0.0; 16]); 3]).is_err());
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        device_where(|_| true)
    }

    /// A device for a chain that reads the frame's depth. On GL the depth is copied into an
    /// `R32Float` target first, which an adapter without `EXT_color_buffer_float` (Mesa's llvmpipe)
    /// cannot render to: skip there, as the other GPU tests skip a missing capability.
    fn depth_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        device_where(|adapter| {
            crate::gfx::test_gpu::supports_texture_usage(
                adapter,
                wgpu::TextureFormat::R32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            )
        })
    }

    fn device_where(
        supported: impl Fn(&wgpu::Adapter) -> bool,
    ) -> Option<(wgpu::Device, wgpu::Queue)> {
        let adapter = crate::gfx::test_gpu::adapter()?;
        if !supported(&adapter) {
            return None;
        }
        let pair = crate::gfx::test_gpu::optional_device_with(&adapter, &Default::default())?;
        eprintln!("post-effect GPU: {:?}", adapter.get_info());
        Some(pair)
    }

    fn scratch(name: &str, json: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("sunrise-post-chain-{}-{name}", std::process::id()));
        let dir = root.join("assets/minecraft/post_effect");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.json")), json).unwrap();
        root
    }

    fn run(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        effects: &mut PostEffects,
        root: &ResourceSource,
        names: &[String],
    ) -> (bool, [u8; 4]) {
        run_with(device, queue, effects, root, names, None, None)
    }

    /// [`run`] with what the packs' Polytone post chains add (`extras`).
    fn run_with(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        effects: &mut PostEffects,
        root: &ResourceSource,
        names: &[String],
        extras: Option<&Extras>,
        poly_globals: Option<crate::gfx::expression_blocks::PolyGlobalsBytes>,
    ) -> (bool, [u8; 4]) {
        let build = poly_globals.map(|bytes| move || bytes);
        let frame = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 8,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = frame.create_view(&Default::default());
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&Default::default());
        // The frame drawn so far: (0.2, 0.4, 0.6).
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.2,
                        g: 0.4,
                        b: 0.6,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let ran = effects.apply(
            device,
            queue,
            &mut encoder,
            &frame,
            &view,
            [8, 4],
            None,
            &Request {
                effects: names,
                root,
                menu_blur_radius: 5.0,
                globals: Globals::default(),
                extras,
                poly_globals: build.as_ref().map(|b| b as _),
            },
            None,
        );
        let buffer = crate::gfx::copy_frame(device, &mut encoder, &frame, [8, 4]);
        queue.submit([encoder.finish()]);
        if let Some(error) = pollster::block_on(scope.pop()) {
            panic!("{error}");
        }
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.get_mapped_range(..).unwrap();
        (ran, [data[0], data[1], data[2], data[3]])
    }

    /// The chain runs over the frame and the result lands on it: invert into
    /// `swap`, then a blit modulated by (1, 0, 0.5, 1) back onto the frame.
    #[test]
    #[ignore = "daily"]
    fn a_chain_draws_onto_the_frame() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let root = scratch("invert_then_blit", INVERT_THEN_BLIT);
        let source = ResourceSource::directory(root.clone());
        let mut effects = PostEffects::new(&device);
        let names = ["minecraft:invert_then_blit".to_owned()];
        let (ran, px) = run(&device, &queue, &mut effects, &source, &names);
        assert!(ran);
        // (0.8, 0.6, 0.4) * (1, 0, 0.5): 204, 0, 51.
        let near = |a: u8, b: u8| a.abs_diff(b) <= 2;
        assert!(
            near(px[0], 204) && px[1] == 0 && near(px[2], 51) && px[3] == 255,
            "{px:?}"
        );
        // A second frame is the same (the chain and its targets are reused).
        let (_, again) = run(&device, &queue, &mut effects, &source, &names);
        assert_eq!(again, px);
        // Missing chains and a chain that does not load change nothing.
        let (ran, px) = run(
            &device,
            &queue,
            &mut effects,
            &source,
            &["minecraft:nothing".to_owned()],
        );
        assert!(!ran);
        assert!(
            near(px[0], 51) && near(px[1], 102) && near(px[2], 153),
            "{px:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A scratch root with one single-pass chain per `(name, fragment shader)`, each drawing its
    /// fragment shader over the frame (`pass uniforms` given as JSON), and a full-screen vertex
    /// shader.
    fn one_pass_chains(name: &str, chains: &[(&str, String, &str)]) -> PathBuf {
        let root = scratch(name, "{}");
        let shaders = root.join("assets/minecraft/shaders");
        std::fs::create_dir_all(shaders.join("post")).unwrap();
        std::fs::write(
            shaders.join("post/full.vsh"),
            "#version 330\nvoid main() {\n  vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n\
               gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);\n}\n",
        )
        .unwrap();
        for (chain, fsh, uniforms) in chains {
            std::fs::write(shaders.join(format!("post/{chain}.fsh")), fsh).unwrap();
            std::fs::write(
                root.join(format!("assets/minecraft/post_effect/{chain}.json")),
                format!(
                    r#"{{"passes":[{{"vertex_shader":"minecraft:post/full",
                        "fragment_shader":"minecraft:post/{chain}","inputs":[],
                        "output":"minecraft:main","uniforms":{{{uniforms}}}}}]}}"#
                ),
            )
            .unwrap();
        }
        root
    }

    /// A pack post shader reads the `PolyGlobals` the frame gives (`Request::poly_globals`) in any
    /// effect, not only a Polytone chain's: the block's bytes as given, zeros on a frame without
    /// them, and the built-in include's two helpers. Expected values worked out by hand from the
    /// helpers' math: the player at (3.5, 64.25, -7.75) is block (3, 64, -8) less the offset
    /// (-0.5, -0.25, -0.25); a projection scaling x and y by 2 and a model-view moving by
    /// (1, 2, 3) take screen point (0.75, 0.25) at depth 0.5 to NDC (0.5, -0.5, 0.5), view
    /// (0.25, -0.25, 0.5) and world (-0.75, -2.25, -2.5); and with the game's reverse-Z
    /// projection, a point's depth back to the point (the geometric hit).
    #[test]
    #[ignore = "daily"]
    fn a_post_pass_reads_the_games_poly_globals() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let fsh = |color: &str| {
            format!(
                "#version 330\n#moj_import <polytone_globals.glsl>\n\
                 layout(location = 0) out vec4 fragColor;\n\
                 void main() {{\n  {color}\n}}\n"
            )
        };
        let root = one_pass_chains(
            "poly_globals",
            &[
                (
                    "raw",
                    fsh(
                        "fragColor = vec4(float(PolyPlayerBlockPos.y) / 255.0, PolySunAngle, \
                         PolyDayTime / 255.0, 1.0);",
                    ),
                    "",
                ),
                (
                    "player",
                    fsh(
                        "vec3 p = polyPlayerPos();\n  fragColor = vec4(p.x * 2.0 / 255.0, \
                         (p.y - 60.0) * 4.0 / 255.0, -p.z * 4.0 / 255.0, 1.0);",
                    ),
                    "",
                ),
                (
                    "hit",
                    fsh(
                        "vec3 w = polyPositionFromDepth(vec2(0.75, 0.25), 2.45 / 99.95);\n  \
                         fragColor = vec4((w + vec3(1.0, 4.0, 6.0)) * 100.0 / 255.0, 1.0);",
                    ),
                    "",
                ),
                (
                    "depth",
                    fsh("vec3 w = polyPositionFromDepth(vec2(0.75, 0.25), 0.5);\n  \
                         fragColor = vec4((w + vec3(1.0, 3.0, 3.0)) * 100.0 / 255.0, 1.0);"),
                    "",
                ),
            ],
        );
        let mut projection = [0.0; 16];
        (projection[0], projection[5], projection[10], projection[15]) = (2.0, 2.0, 1.0, 1.0);
        let mut model_view = [0.0; 16];
        (model_view[0], model_view[5], model_view[10], model_view[15]) = (1.0, 1.0, 1.0, 1.0);
        model_view[12..15].copy_from_slice(&[1.0, 2.0, 3.0]);
        let globals = sunrise_polytone::globals::Globals {
            projection,
            model_view,
            sun_angle: 0.2,
            day_time: 153.0,
            delta_time: 0.0,
            player: [3.5, 64.25, -7.75],
        };
        let bytes = globals.std140();
        let source = ResourceSource::directory(root.clone());
        let mut effects = PostEffects::new(&device);
        // Plain effects (no Polytone chain, no extras): the pass reads the game's block.
        let mut drawn = |chain: &str, given| {
            let names = [format!("minecraft:{chain}")];
            let (ran, px) = run_with(&device, &queue, &mut effects, &source, &names, None, given);
            assert!(ran, "{chain}");
            px
        };
        // Block height 64, sun angle 0.2 (51), day time 153.
        assert_eq!(drawn("raw", Some(bytes)), [64, 51, 153, 255]);
        // A frame without them: zeros, never the last frame's bytes.
        assert_eq!(drawn("raw", None), [0, 0, 0, 255]);
        // polyPlayerPos() (3.5, 64.25, -7.75): 3.5 * 2 = 7, 4.25 * 4 = 17, 7.75 * 4 = 31.
        assert_eq!(drawn("player", Some(bytes)), [7, 17, 31, 255]);
        // (-0.75, -2.25, -2.5) + (1, 3, 3) = (0.25, 0.75, 0.5), times 100.
        assert_eq!(drawn("depth", Some(bytes)), [25, 75, 50, 255]);
        // The game's own projection form: reverse-Z, `perspective_rh(90 degrees, 1, far 100,
        // near 0.05)`, so r = 0.05 / 99.95 and a view point (x, y, z) has device depth
        // (100 + z) r / -z. The point (1, -1, -2) (seen at uv (0.75, 0.25)) has depth 98 r / 2 =
        // 2.45 / 99.95, and the model-view's move by (1, 2, 3) puts it at (0, -3, -5); plus
        // (1, 4, 6), times 100. The depth taken as OpenGL's (depth * 2 - 1, as Polytone 6.11 on
        // 26.2 does) would put it about 0.05 blocks behind the camera instead.
        let r = 0.05f32 / 99.95;
        let mut reverse_z = [0.0; 16];
        (
            reverse_z[0],
            reverse_z[5],
            reverse_z[10],
            reverse_z[11],
            reverse_z[14],
        ) = (1.0, 1.0, r, -1.0, 100.0 * r);
        let hit = sunrise_polytone::globals::Globals {
            projection: reverse_z,
            ..globals
        }
        .std140();
        assert_eq!(drawn("hit", Some(hit)), [100, 100, 100, 255]);
        assert!(effects.failed.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A post pass binds only the blocks its stages read: twelve read beside an imported, unused
    /// `PolyGlobals`, or beside twelve only declared, fit the device's twelve per stage and run.
    /// Refused, each for its own reason, and skipped: 25 declared (before any analysis),
    /// thirteen read, a pass whose uniforms give `PolyGlobals` (the game's), one declaring it
    /// other than its layout, and one whose pipeline the device refuses (its fragment stage
    /// reads an input the vertex stage doesn't write), loaded in a frame or preloaded.
    #[test]
    #[ignore = "daily"]
    fn a_post_pass_binds_what_it_reads_and_is_refused_past_the_limits() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        assert_eq!(device.limits().max_uniform_buffers_per_shader_stage, 12);
        // `n` blocks read, then `unused` only declared.
        let blocks_with = |n: usize, unused: usize, import: bool| {
            let declared: String = (0..n + unused)
                .map(|i| format!("layout(std140) uniform B{i} {{ float b{i}; }};\n"))
                .collect();
            let read: String = (0..n).map(|i| format!(" + b{i}")).collect();
            let import = if import {
                "#moj_import <polytone_globals.glsl>\n"
            } else {
                ""
            };
            let fsh = format!(
                "#version 330\n{import}{declared}layout(location = 0) out vec4 fragColor;\n\
                 void main() {{\n  fragColor = vec4(0.0{read}, 0.0, 0.0, 1.0);\n}}\n"
            );
            let uniforms: Vec<String> = (0..n)
                .map(|i| format!(r#""B{i}":[{{"name":"b{i}","type":"float","value":0.0}}]"#))
                .collect();
            (fsh, uniforms.join(","))
        };
        let blocks = |n, import| blocks_with(n, 0, import);
        let (twelve, twelve_uniforms) = blocks(12, true);
        // Declared by both stages: up to twice the per-stage limit, read or not, is fine; one
        // more is refused before any analysis.
        let (wide, wide_uniforms) = blocks_with(12, 12, false);
        let (crowded, crowded_uniforms) = blocks_with(1, 24, false);
        let (thirteen, thirteen_uniforms) = blocks(13, false);
        let (one, _) = blocks(1, true);
        let taken = r#""B0":[{"name":"b0","type":"float","value":0.0}],
            "PolyGlobals":[{"name":"PolySunAngle","type":"float","value":1.0}]"#;
        let short = "#version 330\nlayout(std140) uniform PolyGlobals { float PolySunAngle; };\n\
            layout(location = 0) out vec4 fragColor;\n\
            void main() {\n  fragColor = vec4(PolySunAngle);\n}\n"
            .to_owned();
        let unmatched = "#version 330\nlayout(location = 0) in vec2 texCoord;\n\
            layout(location = 0) out vec4 fragColor;\n\
            void main() {\n  fragColor = vec4(texCoord, 0.0, 1.0);\n}\n"
            .to_owned();
        let root = one_pass_chains(
            "limits",
            &[
                ("twelve", twelve, &twelve_uniforms),
                ("wide", wide, &wide_uniforms),
                ("crowded", crowded, &crowded_uniforms),
                ("thirteen", thirteen, &thirteen_uniforms),
                ("taken", one, taken),
                ("short", short, ""),
                ("unmatched", unmatched, ""),
            ],
        );
        let source = ResourceSource::directory(root.clone());
        // Why each is refused.
        let load = |chain: &str| {
            let id = format!("minecraft:{chain}");
            let loaded = PostEffects::load(&source, &device, &queue, &id, &Extras::default());
            let mut chain = loaded?.expect("the chain");
            chain.ensure_pipelines(&device, wgpu::TextureFormat::Rgba8Unorm)
        };
        assert_eq!(load("twelve"), Ok(()));
        assert_eq!(load("wide"), Ok(()));
        let fragment = Some(wgpu::ShaderStages::FRAGMENT);
        let blocks_past = |stage, count, limit| {
            Err(LoadError::Pass(
                0,
                PassRefusal::OverLimit {
                    what: "uniform blocks",
                    stage,
                    count,
                    limit,
                },
            ))
        };
        assert_eq!(load("crowded"), blocks_past(None, 25, 24));
        assert_eq!(load("thirteen"), blocks_past(fragment, 13, 12));
        assert_eq!(
            load("taken"),
            Err(LoadError::Pass(0, PassRefusal::GlobalsGiven))
        );
        assert!(matches!(
            load("short"),
            Err(LoadError::Pass(
                0,
                PassRefusal::Globals(crate::gfx::expression_blocks::GlobalsRefusal::Member {
                    index: 0,
                    ..
                })
            ))
        ));
        assert!(matches!(
            load("unmatched"),
            Err(LoadError::Pass(0, PassRefusal::Pipeline(_)))
        ));
        // In a frame: the refused ones are named in the log and skipped.
        let mut effects = PostEffects::new(&device);
        let mut ran = |chain: &str| {
            let names = [format!("minecraft:{chain}")];
            run(&device, &queue, &mut effects, &source, &names).0
        };
        for chain in ["twelve", "wide"] {
            assert!(ran(chain), "{chain}");
        }
        for chain in ["crowded", "thirteen", "taken", "short", "unmatched"] {
            assert!(!ran(chain), "{chain}");
        }
        let mut failed: Vec<&String> = effects.failed.iter().collect();
        failed.sort();
        assert_eq!(
            failed,
            [
                "minecraft:crowded",
                "minecraft:short",
                "minecraft:taken",
                "minecraft:thirteen",
                "minecraft:unmatched"
            ]
        );
        // A preloaded chain whose pipeline the device refuses arrives refused, named, too.
        let mut effects = PostEffects::new(&device);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        effects.preload(&device, &queue, &source, ["minecraft:unmatched"], format);
        let started = std::time::Instant::now();
        while !effects.pending.is_empty() {
            assert!(started.elapsed().as_secs() < 60, "the chain never arrived");
            std::thread::sleep(std::time::Duration::from_millis(1));
            effects.collect_preloaded();
        }
        assert!(effects.failed.contains("minecraft:unmatched"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// One rule for a refused pass: a pass naming vanilla's shaders whose pipeline the device
    /// refuses (the pack's `blit.fsh` reads an input its `screenquad.vsh` doesn't write) runs on
    /// the built-in stand-ins, as one the translator refuses does, and the effect runs: the
    /// frame (0.2, 0.4, 0.6) times (1, 0, 1). The same pass naming no vanilla shader is refused
    /// with its effect (the limits test's `unmatched`).
    #[test]
    #[ignore = "daily"]
    fn a_pass_whose_pipeline_is_refused_falls_back_to_the_stand_in() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let blit = |from: &str, to: &str, modulate: &str| {
            format!(
                r#"{{"vertex_shader":"minecraft:core/screenquad","fragment_shader":"minecraft:post/blit",
                    "inputs":[{{"sampler_name":"In","target":"{from}"}}],"output":"{to}",
                    "uniforms":{{"BlitConfig":[{{"name":"ColorModulate","type":"vec4","value":{modulate}}}]}}}}"#
            )
        };
        let root = scratch(
            "refused_blit",
            &format!(
                r#"{{"targets":{{"swap":{{}}}},"passes":[{},{}]}}"#,
                blit("minecraft:main", "swap", "[1.0, 0.0, 1.0, 1.0]"),
                blit("swap", "minecraft:main", "[1.0, 1.0, 1.0, 1.0]")
            ),
        );
        let shaders = root.join("assets/minecraft/shaders");
        std::fs::create_dir_all(shaders.join("core")).unwrap();
        std::fs::create_dir_all(shaders.join("post")).unwrap();
        std::fs::write(
            shaders.join("core/screenquad.vsh"),
            "#version 330\nvoid main() {\n  vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n\
               gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);\n}\n",
        )
        .unwrap();
        std::fs::write(
            shaders.join("post/blit.fsh"),
            "#version 330\nuniform sampler2D InSampler;\n\
             layout(std140) uniform BlitConfig { vec4 ColorModulate; };\n\
             layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {\n  fragColor = texture(InSampler, texCoord) * ColorModulate;\n}\n",
        )
        .unwrap();
        let source = ResourceSource::directory(root.clone());
        // It loads translated, with its stand-in kept for the pipeline the device refuses.
        let mut chain = PostEffects::load(
            &source,
            &device,
            &queue,
            "minecraft:refused_blit",
            &Extras::default(),
        )
        .unwrap()
        .unwrap();
        assert!(
            chain
                .passes
                .iter()
                .all(|p| p.translated.is_some() && p.fallback.is_some())
        );
        assert_eq!(
            chain.ensure_pipelines(&device, wgpu::TextureFormat::Rgba8Unorm),
            Ok(())
        );
        assert!(chain.passes.iter().all(|p| p.translated.is_none()));
        let mut effects = PostEffects::new(&device);
        let names = ["minecraft:refused_blit".to_owned()];
        let (ran, px) = run(&device, &queue, &mut effects, &source, &names);
        assert!(ran);
        assert_eq!(px, [51, 0, 153, 255]);
        assert!(effects.failed.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_chain_naming_an_unknown_shader_fails_once() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let root = scratch(
            "unknown",
            r#"{"passes":[{"vertex_shader":"minecraft:core/screenquad","fragment_shader":"minecraft:post/nope",
                "inputs":[{"sampler_name":"In","target":"minecraft:main"}],"output":"minecraft:main"}]}"#,
        );
        let source = ResourceSource::directory(root.clone());
        let mut effects = PostEffects::new(&device);
        let names = ["minecraft:unknown".to_owned()];
        assert!(!run(&device, &queue, &mut effects, &source, &names).0);
        assert!(effects.failed.contains("minecraft:unknown"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The chains the prepared vanilla resources hold all load (skipped without
    /// them): none names a shader Sunrise lacks.
    /// Renders `names` over a frame whose top half is red and bottom half green,
    /// returning the pixel rows 0 and 3 of the 8x4 result.
    fn run_halves(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        effects: &mut PostEffects,
        root: &ResourceSource,
        names: &[String],
    ) -> [[u8; 4]; 2] {
        let frame = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 8,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut pixels = Vec::new();
        for y in 0..4 {
            for _ in 0..8 {
                pixels.extend_from_slice(if y < 2 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 255, 0, 255]
                });
            }
        }
        queue.write_texture(
            frame.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(32),
                rows_per_image: Some(4),
            },
            frame.size(),
        );
        let view = frame.create_view(&Default::default());
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(effects.apply(
            device,
            queue,
            &mut encoder,
            &frame,
            &view,
            [8, 4],
            None,
            &Request {
                effects: names,
                root,
                menu_blur_radius: 0.0,
                globals: Globals::default(),
                extras: None,
                poly_globals: None,
            },
            None,
        ));
        let buffer = crate::gfx::copy_frame(device, &mut encoder, &frame, [8, 4]);
        queue.submit([encoder.finish()]);
        if let Some(error) = pollster::block_on(scope.pop()) {
            panic!("{error}");
        }
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.get_mapped_range(..).unwrap();
        let row = |y: usize| {
            let at = y * 256;
            [data[at], data[at + 1], data[at + 2], data[at + 3]]
        };
        [row(0), row(3)]
    }

    /// Frame orientation survives a chain through an intermediate target.
    #[test]
    #[ignore = "daily"]
    fn a_chain_keeps_the_frame_upright() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            return;
        };
        let root = scratch(
            "upright",
            r#"{"targets":{"swap":{}},"passes":[
            {"vertex_shader":"minecraft:core/screenquad","fragment_shader":"minecraft:post/blit",
             "inputs":[{"sampler_name":"In","target":"minecraft:main"}],"output":"swap",
             "uniforms":{"BlitConfig":[{"name":"ColorModulate","type":"vec4","value":[1.0,1.0,1.0,1.0]}]}},
            {"vertex_shader":"minecraft:core/screenquad","fragment_shader":"minecraft:post/blit",
             "inputs":[{"sampler_name":"In","target":"swap"}],"output":"minecraft:main",
             "uniforms":{"BlitConfig":[{"name":"ColorModulate","type":"vec4","value":[1.0,1.0,1.0,1.0]}]}}]}"#,
        );
        let source = ResourceSource::directory(root.clone());
        let mut effects = PostEffects::new(&device);
        let [top, bottom] = run_halves(
            &device,
            &queue,
            &mut effects,
            &source,
            &["minecraft:upright".to_owned()],
        );
        assert_eq!(top, [255, 0, 0, 255]);
        assert_eq!(bottom, [0, 255, 0, 255]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A chain with its own GLSL, a target read twice, a depth input and a
    /// `textures/effect` image: nothing here is one of vanilla's shaders.
    #[test]
    #[ignore = "daily"]
    fn a_chain_runs_its_own_glsl() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = depth_device() else {
            return;
        };
        let root = scratch(
            "custom",
            r#"{"targets":{"half":{"width":4,"height":2}},"passes":[
            {"vertex_shader":"minecraft:core/quad","fragment_shader":"minecraft:post/mine",
             "inputs":[{"sampler_name":"In","target":"minecraft:main","bilinear":true},
                       {"sampler_name":"Depth","target":"minecraft:main","use_depth_buffer":true},
                       {"sampler_name":"Tint","location":"tint","width":1,"height":1}],
             "output":"half",
             "uniforms":{"MineConfig":[{"name":"Scale","type":"float","value":0.5}]}},
            {"vertex_shader":"minecraft:core/quad","fragment_shader":"minecraft:post/mine",
             "inputs":[{"sampler_name":"In","target":"half"},
                       {"sampler_name":"Depth","target":"half","use_depth_buffer":true},
                       {"sampler_name":"Tint","location":"tint","width":1,"height":1}],
             "output":"minecraft:main",
             "uniforms":{"MineConfig":[{"name":"Scale","type":"float","value":1.0}]}}]}"#,
        );
        let source = ResourceSource::directory(root.clone());
        own_glsl(&root);
        let mut effects = PostEffects::new(&device);
        let [top, bottom] = run_halves(
            &device,
            &queue,
            &mut effects,
            &source,
            &["minecraft:custom".to_owned()],
        );
        // Red (1, 0, 0) * the image (0.5, 1, 1) * 0.5 * depth 1, then again * 1: 0.125.
        let near = |a: u8, b: u8| a.abs_diff(b) <= 3;
        assert!(near(top[0], 32) && top[1] == 0 && top[3] == 255, "{top:?}");
        // Green stays below it: 0.5 * 1 * 1.
        assert!(bottom[0] == 0 && near(bottom[1], 128), "{bottom:?}");
        assert!(effects.failed.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The frame's depth reaches a pass as 26.3 keeps it, reverse-Z (the sky 0, a near pixel
    /// close to 1), not as GL's window depth.
    #[test]
    #[ignore = "daily"]
    fn a_pass_reads_the_frame_depth_as_it_is() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = depth_device() else {
            return;
        };
        let root = scratch(
            "depth",
            r#"{"passes":[{"vertex_shader":"minecraft:core/quad","fragment_shader":"minecraft:post/depth",
             "inputs":[{"sampler_name":"Depth","target":"minecraft:main","use_depth_buffer":true}],
             "output":"minecraft:main"}]}"#,
        );
        own_glsl(&root);
        std::fs::write(
            root.join("assets/minecraft/shaders/post/depth.fsh"),
            "#version 330\nuniform sampler2D DepthSampler;\nlayout(location = 0) in vec2 texCoord;\n\
             layout(location = 0) out vec4 fragColor;\n\
             void main() { fragColor = vec4(texture(DepthSampler, texCoord).r, 0.0, 0.0, 1.0); }\n",
        )
        .unwrap();
        let size = wgpu::Extent3d {
            width: 8,
            height: 4,
            depth_or_array_layers: 1,
        };
        let texture = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let frame = texture(
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (view, depth_view) = (
            frame.create_view(&Default::default()),
            depth.create_view(&Default::default()),
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.25),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        let mut effects = PostEffects::new(&device);
        let ran = effects.apply(
            &device,
            &queue,
            &mut encoder,
            &frame,
            &view,
            [8, 4],
            Some(&depth_view),
            &Request {
                effects: &["minecraft:depth".to_owned()],
                root: &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
                menu_blur_radius: 0.0,
                globals: Globals::default(),
                extras: None,
                poly_globals: None,
            },
            None,
        );
        let buffer = crate::gfx::copy_frame(&device, &mut encoder, &frame, [8, 4]);
        queue.submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let red = buffer.get_mapped_range(..).unwrap()[0];
        assert!(ran);
        // 0.25 of 255, not 0.75.
        assert!(red.abs_diff(64) <= 1, "{red}");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// `core/quad.vsh`, `post/mine.fsh` and the `tint` image of `a_chain_runs_its_own_glsl`.
    fn own_glsl(root: &Path) {
        let shaders = root.join("assets/minecraft/shaders");
        std::fs::create_dir_all(shaders.join("core")).unwrap();
        std::fs::create_dir_all(shaders.join("post")).unwrap();
        std::fs::write(
            shaders.join("core/quad.vsh"),
            "#version 330\nlayout(location = 0) out vec2 texCoord;\nvoid main() {\n  vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n  gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);\n  texCoord = uv;\n}\n",
        )
        .unwrap();
        std::fs::write(
            shaders.join("post/mine.fsh"),
            "#version 330\nuniform sampler2D InSampler;\nuniform sampler2D DepthSampler;\nuniform sampler2D TintSampler;\n\
             layout(std140) uniform SamplerInfo { vec2 OutSize; vec2 InSize; };\n\
             layout(std140) uniform MineConfig { float Scale; };\n\
             layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {\n  vec4 c = texture(InSampler, texCoord);\n  float d = texture(DepthSampler, texCoord).r;\n\
               vec4 t = texture(TintSampler, texCoord);\n  fragColor = vec4(c.rgb * t.rgb * Scale * d, 1.0);\n}\n",
        )
        .unwrap();
        // A 1x1 image, (0.5, 1, 1).
        let effect = root.join("assets/minecraft/textures/effect");
        std::fs::create_dir_all(&effect).unwrap();
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[128, 255, 255, 255]).unwrap();
        }
        std::fs::write(effect.join("tint.png"), png_bytes).unwrap();
    }

    /// A chain `preload` loaded arrives with its pipelines compiled on the loading
    /// thread, and runs as one loaded in its frame does.
    #[test]
    #[ignore = "daily"]
    fn a_preloaded_archive_chain_arrives_compiled() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = depth_device() else {
            return;
        };
        let root = scratch(
            "preloaded",
            r#"{"targets":{"half":{"width":4,"height":2}},"passes":[
            {"vertex_shader":"minecraft:core/quad","fragment_shader":"minecraft:post/mine",
             "inputs":[{"sampler_name":"In","target":"minecraft:main","bilinear":true},
                       {"sampler_name":"Depth","target":"minecraft:main","use_depth_buffer":true},
                       {"sampler_name":"Tint","location":"tint","width":1,"height":1}],
             "output":"half",
             "uniforms":{"MineConfig":[{"name":"Scale","type":"float","value":0.5}]}},
            {"vertex_shader":"minecraft:core/quad","fragment_shader":"minecraft:post/mine",
             "inputs":[{"sampler_name":"In","target":"half"},
                       {"sampler_name":"Depth","target":"half","use_depth_buffer":true},
                       {"sampler_name":"Tint","location":"tint","width":1,"height":1}],
             "output":"minecraft:main",
             "uniforms":{"MineConfig":[{"name":"Scale","type":"float","value":1.0}]}}]}"#,
        );
        let source = ResourceSource::directory(root.clone());
        own_glsl(&root);
        use std::io::Write;
        let jar = root.join("selected.jar");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
        for name in source.list("assets/") {
            writer
                .start_file(&name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(&source.read(&name).unwrap()).unwrap();
        }
        writer.finish().unwrap();
        let source = ResourceSource::archive(&jar).unwrap();
        std::fs::remove_dir_all(root.join("assets")).unwrap();
        let mut effects = PostEffects::new(&device);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        effects.preload(&device, &queue, &source, ["minecraft:preloaded"], format);
        assert!(effects.pending.contains("minecraft:preloaded"));
        let started = std::time::Instant::now();
        while !effects.pending.is_empty() {
            assert!(started.elapsed().as_secs() < 60, "the chain never arrived");
            std::thread::sleep(std::time::Duration::from_millis(1));
            effects.collect_preloaded();
        }
        let chain = effects.chains["minecraft:preloaded"]
            .as_ref()
            .expect("loaded");
        assert!(chain.passes.iter().all(|pass| {
            pass.translated
                .as_ref()
                .is_some_and(|t| t.pipelines.contains_key(&format))
        }));
        let [top, bottom] = run_halves(
            &device,
            &queue,
            &mut effects,
            &source,
            &["minecraft:preloaded".to_owned()],
        );
        let near = |a: u8, b: u8| a.abs_diff(b) <= 3;
        assert!(near(top[0], 32) && top[1] == 0 && top[3] == 255, "{top:?}");
        assert!(bottom[0] == 0 && near(bottom[1], 128), "{bottom:?}");
        effects.use_root(&source.clone());
        assert!(effects.chains.contains_key("minecraft:preloaded"));
        // Reopening the same pack creates a new generation, even at the same path.
        effects.use_root(&ResourceSource::archive(&jar).unwrap());
        assert!(effects.chains.is_empty());
        assert!(effects.pending.is_empty());
        assert!(!root.join("assets").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    const VANILLA_CHAINS: [&str; 4] = ["blur", "creeper", "invert", "spider"];

    fn vanilla_source(effects: impl IntoIterator<Item = &'static str>) -> ResourceSource {
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client/26.3")
            });
        for name in effects {
            let path = root.join(resource_path(name));
            assert!(
                path.is_file(),
                "missing prepared post-effect file: {}",
                path.display()
            );
        }
        ResourceSource::directory(root)
    }

    /// Vanilla's own prepared chains run on the GPU without a validation error.
    #[test]
    #[ignore = "daily"]
    fn vanilla_chains_run() {
        let _gpu = crate::gfx::test_gpu::lock();
        let source = vanilla_source(VANILLA_CHAINS);
        let Some((device, queue)) = device() else {
            return;
        };
        for name in VANILLA_CHAINS {
            let mut effects = PostEffects::new(&device);
            run_halves(
                &device,
                &queue,
                &mut effects,
                &source,
                &[format!("minecraft:{name}")],
            );
            assert!(effects.failed.is_empty(), "{name}");
            // The chain ran on the resources' own GLSL, not the stand-ins.
            let chain = effects.chains[&format!("minecraft:{name}")]
                .as_ref()
                .unwrap();
            assert!(
                chain.passes.iter().all(|p| p.translated.is_some()),
                "{name}"
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn vanilla_chains_load() {
        let _gpu = crate::gfx::test_gpu::lock();
        let source = vanilla_source(VANILLA_CHAINS.into_iter().chain(["entity_outline"]));
        let Some((device, queue)) = device() else {
            return;
        };
        for name in VANILLA_CHAINS {
            let id = format!("minecraft:{name}");
            assert!(
                PostEffects::load(&source, &device, &queue, &id, &Extras::default())
                    .unwrap()
                    .is_some(),
                "{id}"
            );
        }
        // entity_outline needs targets and shaders the world pass has of its own.
        assert!(
            PostEffects::load(
                &source,
                &device,
                &queue,
                "minecraft:entity_outline",
                &Extras::default()
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod missing_resource_test {
    #[test]
    fn an_unregistered_missing_chain_stays_optional() {
        let source = super::ResourceSource::indexed(Default::default());
        let shaders = crate::spectator_shaders::SpectatorShaders::default();
        assert_eq!(
            super::read_chain(&source, "example:optional", &shaders),
            Ok(None)
        );
        assert!(super::read_chain(&source, "minecraft:creeper", &shaders).is_err());
    }

    #[test]
    fn missing_registered_resource_is_diagnosed() {
        let source = super::ResourceSource::indexed(Default::default());
        let shaders =
            crate::spectator_shaders::SpectatorShaders::new(vec![mod_api::EntitySpectatorShader {
                entity: "minecraft:cow".into(),
                shader: "example:cow_vison".into(),
            }]);
        let error = super::read_chain(&source, shaders.chain("minecraft:cow").unwrap(), &shaders)
            .unwrap_err();
        assert!(error.contains("minecraft:cow"));
        assert!(error.contains("example:cow_vison"));
        assert!(error.contains("assets/example/post_effect/cow_vison.json"));
    }
}
