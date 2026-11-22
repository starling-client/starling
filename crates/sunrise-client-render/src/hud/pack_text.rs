//! A resource pack's `core/text` shader on the GUI's text (`RenderPipelines.GUI_TEXT`, the shader
//! with `IS_GUI`): servers' HUDs that their pack's vertex shader moves to the screen's corners,
//! and text effects in its fragment shader. Without one the text draws natively.

use crate::resource_source::ResourceSource;
use std::sync::Arc;

use crate::core_shader::{CoreProgram, Glue};

use super::{HudBatch, HudVertex};
use crate::gfx::hud_coverage::Pipeline;

/// GUI text's vertex format (`POSITION_TEX_COLOR`) from the HUD's vertices. A glyph is two
/// triangles here, its corners top left, top right, bottom right, top left, bottom right, bottom
/// left; vanilla's quads go top left, bottom left, bottom right, top right, which is what
/// `gl_VertexID % 4` tells a pack's shader. The draw passes its first vertex as its instance.
const GLUE: Glue = Glue {
    inputs: &[("Position", "vec3"), ("Color", "vec4"), ("UV0", "vec2")],
    prelude: "layout(location = 0) in vec2 hz_pos;\n\
              layout(location = 1) in vec2 hz_uv;\n\
              layout(location = 2) in vec4 hz_color;\n\
              const int hz_corner[6] = int[6](0, 3, 2, 0, 2, 1);\n\
              int hz_vertex_id() {\n\
                  int i = int(gl_VertexIndex) - int(gl_InstanceIndex);\n\
                  return i / 6 * 4 + hz_corner[i % 6];\n\
              }",
    setup: "Position = vec3(hz_pos, 0.0); Color = hz_color; UV0 = hz_uv;",
    // GL's clip depth runs from -w to w, wgpu's from 0 to w; GUI text has no depth test.
    finish: "gl_Position.z = 0.5 * gl_Position.w;",
    vertex_id: "hz_vertex_id()",
};

/// The packs' core shader stages `names` (`shaders/core/<name>.vsh` and `.fsh`, which may differ,
/// as `core/screenquad` and `core/lightmap` do) with `defines`, over `defaults` (the game's own
/// resources, which have vanilla's and its includes), when a pack replaces either stage, giving
/// the blocks `provided` says yes to.
pub(super) fn load_pair(
    root: &ResourceSource,
    defaults: &ResourceSource,
    names: (&str, &str),
    defines: &[&str],
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Option<Arc<CoreProgram>> {
    let name = names.1;
    match translate_pair(root, defaults, names, defines, glue, provided)? {
        Ok(program) => {
            log::info!("the resource pack's {name} shader translated");
            Some(Arc::new(program))
        }
        Err(e) => {
            log::warn!("the resource pack's {name} shader is not used: {e}");
            None
        }
    }
}

/// Why a pack's core shader is refused (`load_pair`), with the file it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A stage or include over the translator's budget for a whole shader
    /// (`core_shader::MAX_EXPANDED`).
    OverBudget { path: String },
    /// A stage or include that isn't UTF-8 text.
    NotText { path: String },
    /// A stage or include that couldn't be read otherwise.
    Unreadable { path: String, error: String },
    /// A stage the selected packs hid (`root` doesn't have it).
    StageHidden { path: String },
    /// An include the selected packs don't have (`ns:path`, as the shader asks for it).
    IncludeMissing { id: String },
    /// The translator refused the GLSL.
    Translation(String),
    /// The translator panicked on it.
    Panicked(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverBudget { path } => write!(
                f,
                "{path} is over the {} bytes a shader may have",
                crate::core_shader::MAX_EXPANDED
            ),
            Self::NotText { path } => write!(f, "{path} is not UTF-8 text"),
            Self::Unreadable { path, error } => write!(f, "{path}: {error}"),
            Self::StageHidden { path } => write!(f, "{path} is not in the selected packs"),
            Self::IncludeMissing { id } => write!(f, "{id} is not in the selected packs"),
            Self::Translation(e) => f.write_str(e),
            Self::Panicked(e) => write!(f, "the shader translator failed: {e}"),
        }
    }
}

/// [`load_pair`]'s program: none when no pack changes the shader, else the translation, or why
/// the shader is refused. A stage the packs don't replace, and every include, is the stack's
/// (`root`: what the selected packs left of the game's resources), never `defaults` beside it,
/// so a file a pack's filter hid stays hidden and refuses the shader. `defaults` only tells a
/// pack's shader from the game's: the packs change it when either stage, with its includes
/// resolved, differs from the game's (a pack replacing only an include the shader uses changes
/// it). Each stage and include is read only up to the translator's budget
/// ([`read_shader_file`]); a stage over it refuses the shader by its path, an include only where
/// the stage's `defines` keep it (`core_shader::expand_core_imports`). A panic in the
/// translator refuses it too ([`contained`]).
fn translate_pair(
    root: &ResourceSource,
    defaults: &ResourceSource,
    (vertex_name, name): (&str, &str),
    defines: &[&str],
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Option<Result<CoreProgram, Refusal>> {
    let stages = match selected_stages(root, defaults, (vertex_name, name), &[defines])? {
        Ok(stages) => stages,
        Err(e) => return Some(Err(e)),
    };
    Some(translate_selected(root, &stages, defines, glue, provided))
}

/// Return both selected stage files if any required variant uses a pack override.
fn selected_stages(
    root: &ResourceSource,
    defaults: &ResourceSource,
    (vertex_name, fragment_name): (&str, &str),
    variants: &[&[&str]],
) -> Option<Result<(String, String), Refusal>> {
    let stage = |name, extension| {
        let path = format!("assets/minecraft/shaders/core/{name}.{extension}");
        let selected = read_shader_file(root, &path)?;
        let game = read_shader_file(defaults, &path).ok().flatten();
        let changed = match (&selected, &game) {
            (None, _) => defaults.contains(&path),
            (Some(text), Some(game)) if text == game => variants.iter().any(|defines| {
                let resolved = |base| {
                    crate::core_shader::expand_core_imports(
                        text,
                        "minecraft:shaders/core/",
                        defines,
                        &|id| include_of(base, id).1,
                    )
                };
                resolved(root) != resolved(defaults)
            }),
            _ => selected != game,
        };
        Ok((selected.ok_or(Refusal::StageHidden { path }), changed))
    };
    let result = || {
        let (vertex, v) = stage(vertex_name, "vsh")?;
        let (fragment, f) = stage(fragment_name, "fsh")?;
        Ok(if v || f {
            Some((vertex?, fragment?))
        } else {
            None
        })
    };
    crate::crash::contain(result)
        .map_err(Refusal::Panicked)
        .and_then(|r| r)
        .transpose()
}

fn translate_selected(
    root: &ResourceSource,
    (vertex, fragment): &(String, String),
    defines: &[&str],
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Result<CoreProgram, Refusal> {
    let unavailable = std::cell::RefCell::new(None);
    let include = |id: &str| {
        let (refusal, text) = include_of(root, id);
        *unavailable.borrow_mut() = refusal;
        text
    };
    let stages = crate::core_shader::Stages {
        dir: "minecraft:shaders/core/",
        vertex,
        fragment,
    };
    let translated = contained(|| {
        crate::core_shader::translate_core_at(&stages, defines, &include, glue, provided)
    });
    translated.map_err(|e| match e {
        Refusal::Translation(_) => unavailable.into_inner().unwrap_or(e),
        _ => e,
    })
}

/// The include `id` (`ns:path`) as `base` has it, for the translator: its text, none when `base`
/// hasn't it, or why it can't be read; with the refusal it is, where it can't be given.
fn include_of(
    base: &ResourceSource,
    id: &str,
) -> (Option<Refusal>, Result<Option<String>, String>) {
    let Some((namespace, path)) = id.split_once(':') else {
        return (None, Ok(None));
    };
    match read_shader_file(base, &format!("assets/{namespace}/{path}")) {
        Ok(Some(text)) => (None, Ok(Some(text))),
        Ok(None) if sunrise_polytone::globals::is_include(id) => (None, Ok(None)),
        Ok(None) => (
            Some(Refusal::IncludeMissing { id: id.to_owned() }),
            Ok(None),
        ),
        Err(e) => {
            let why = e.to_string();
            (Some(e), Err(why))
        }
    }
}

/// `translate`'s result, or its panic as a refusal: a pack's GLSL is untrusted, and a fault the
/// translator or naga meets in it must not end the game.
fn contained(
    translate: impl FnOnce() -> Result<CoreProgram, String>,
) -> Result<CoreProgram, Refusal> {
    crate::crash::contain(translate)
        .map_err(Refusal::Panicked)
        .and_then(|translated| translated.map_err(Refusal::Translation))
}

/// A shader file (a stage or an include) of `base`, read only up to the translator's budget
/// for a whole shader (`core_shader::MAX_EXPANDED`, 1 MiB): none when `base` hasn't it, the
/// refusal naming its path when it is over the budget or isn't text.
pub(crate) fn read_shader_file(
    base: &ResourceSource,
    path: &str,
) -> Result<Option<String>, Refusal> {
    if !base.contains(path) {
        return Ok(None);
    }
    let max = crate::core_shader::MAX_EXPANDED as u64;
    let path = path.to_owned();
    // `read_limited` refuses a resource over `max` as invalid data.
    let bytes = base.read_limited(&path, max).map_err(|e| match e.kind() {
        std::io::ErrorKind::InvalidData => Refusal::OverBudget { path: path.clone() },
        _ => Refusal::Unreadable {
            path: path.clone(),
            error: e.to_string(),
        },
    })?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| Refusal::NotText { path })
}

/// The packs' `core/lightmap` with `core/screenquad`, when a pack replaces either stage: the
/// pass gives it `LightmapInfo` besides vanilla's blocks, and the packs' expression uniforms
/// (`modifiers`) of either stage's file, resolved here for the blocks its stages read. A block
/// no file of its stages names, or an expression block that is not one float, refuses it.
fn load_lightmap(
    root: &ResourceSource,
    defaults: &ResourceSource,
    modifiers: &crate::polytone_post::ShaderModifiers,
) -> Option<(
    Arc<CoreProgram>,
    Vec<crate::polytone_post::ExpressionUniform>,
)> {
    let shaders = crate::gfx::core_lightmap::SHADERS;
    let names = modifiers.names(&shaders);
    let info = crate::gfx::core_lightmap::INFO_BLOCK;
    let globals = sunrise_polytone::globals::BLOCK;
    let program = load_pair(
        root,
        defaults,
        ("screenquad", "lightmap"),
        &[],
        &crate::gfx::core_lightmap::GLUE,
        &|block| block == info || block == globals || names.iter().any(|n| n == block),
    )?;
    match modifiers.resolve((shaders[0], shaders[1]), &program, &[info, globals]) {
        Ok(expressions) => Some((program, expressions)),
        Err(e) => {
            log::warn!("the resource pack's lightmap shader is not used: {e}");
            None
        }
    }
}

/// A loaded pack shader and its expression uniforms, as the atlas keeps them: none of either
/// when no pack's shader is used.
trait UnzipOrDefault {
    fn unzip_or_default(
        self,
    ) -> (
        Option<Arc<CoreProgram>>,
        Vec<crate::polytone_post::ExpressionUniform>,
    );
}

impl UnzipOrDefault
    for Option<(
        Arc<CoreProgram>,
        Vec<crate::polytone_post::ExpressionUniform>,
    )>
{
    fn unzip_or_default(
        self,
    ) -> (
        Option<Arc<CoreProgram>>,
        Vec<crate::polytone_post::ExpressionUniform>,
    ) {
        match self {
            Some((program, expressions)) => (Some(program), expressions),
            None => (None, Vec::new()),
        }
    }
}

/// The packs' core shader `id` (`minecraft:core/<name>`, one shader for both stages: the sky,
/// the stars, the block outline, and the text for the GUI's, the world's and the see-through
/// text) for the program `label` names in the log, with its `defines` (the text's `IS_GUI`,
/// none, `IS_SEE_THROUGH`), `PolyGlobals` and the blocks its file of `modifiers` names, and its
/// expression uniforms resolved for the blocks its stages read; none when no pack replaces it,
/// it doesn't translate, or a block refuses it (named in the log with the program).
fn load_with_expressions(
    root: &ResourceSource,
    defaults: &ResourceSource,
    modifiers: &crate::polytone_post::ShaderModifiers,
    id: &str,
    (label, defines): (&str, &[&str]),
    glue: &Glue,
) -> Option<(
    Arc<CoreProgram>,
    Vec<crate::polytone_post::ExpressionUniform>,
)> {
    let name = id.strip_prefix("minecraft:core/")?;
    let names = modifiers.names(&[id]);
    let globals = sunrise_polytone::globals::BLOCK;
    let provided = |block: &str| block == globals || names.iter().any(|n| n == block);
    let translated = translate_pair(root, defaults, (name, name), defines, glue, &provided)?;
    let resolved = translated.map_err(|e| e.to_string()).and_then(|program| {
        let expressions = modifiers.resolve((id, id), &program, &[globals]);
        expressions
            .map(|e| (Arc::new(program), e))
            .map_err(|e| e.to_string())
    });
    match resolved {
        Ok(loaded) => {
            log::info!("the resource pack's {label} shader translated");
            Some(loaded)
        }
        Err(e) => {
            log::warn!("the resource pack's {label} shader is not used: {e}");
            None
        }
    }
}

/// A pack's `core/clouds` that a pipeline vanilla requires can't be made from: in 26.3
/// `ShaderManager.apply` compiles every required pipeline (`CLOUDS`, `FLAT_CLOUDS` and every
/// stage of `OIT_CLOUDS` and `OIT_FLAT_CLOUDS`, whether Improved Transparency is on or not) and
/// fails the reload when one doesn't ("Failed to load required shader programs"), which drops
/// the selected packs. The atlas's load fails with it, through the same reload failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiredShaderRefused {
    /// The pipeline's location, `minecraft:pipeline/oit_transmittance_clouds`.
    pub pipeline: &'static str,
    pub why: String,
}

impl std::fmt::Display for RequiredShaderRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "failed to load required shader program {}: {}",
            self.pipeline, self.why
        )
    }
}

impl std::error::Error for RequiredShaderRefused {}

/// The packs' `core/clouds`, when the packs change it (a stage or an include it uses),
/// translated as vanilla compiles it for each of its pipelines: once for `CLOUDS`, and once for
/// each stage of `OIT_CLOUDS` with that stage's defines (`core_clouds::OIT_STAGES`). Every pass
/// gives it `CloudInfo` and the `CloudFaces` texel buffer besides vanilla's blocks, and an
/// Improved Transparency stage its samplers. A program any of them can't use refuses the whole
/// load, named by its pipeline.
fn load_clouds(
    root: &ResourceSource,
    defaults: &ResourceSource,
) -> Result<Option<Arc<crate::clouds::pack::Programs>>, RequiredShaderRefused> {
    use crate::gfx::core_clouds::{self as clouds, CLASSIC_DEFINES, OIT_STAGES};
    let variants = [
        &CLASSIC_DEFINES[..],
        OIT_STAGES[0].defines,
        OIT_STAGES[1].defines,
        OIT_STAGES[2].defines,
    ];
    let Some(stages) = selected_stages(root, defaults, ("clouds", "clouds"), &variants) else {
        return Ok(None);
    };
    let stages = stages.map_err(|e| RequiredShaderRefused {
        pipeline: clouds::CLASSIC_PIPELINE,
        why: e.to_string(),
    })?;
    let translate = |defines: &[&str], samplers: &[&str], targets: usize, pipeline| {
        translate_selected(root, &stages, defines, &clouds::GLUE, &|name| {
            matches!(name, clouds::INFO_BLOCK | clouds::FACES) || samplers.contains(&name)
        })
        .map_err(|e| e.to_string())
        .and_then(|program| clouds::check(&program, targets).map(|()| program))
        .map_err(|why| RequiredShaderRefused { pipeline, why })
    };
    let classic = translate(&CLASSIC_DEFINES, &[], 1, clouds::CLASSIC_PIPELINE)?;
    let stage = |i: usize| {
        let s = &OIT_STAGES[i];
        translate(s.defines, s.samplers, clouds::oit_targets(i), s.pipeline)
    };
    let oit = Box::new([stage(0)?, stage(1)?, stage(2)?]);
    log::info!("the resource pack's clouds shader translated, with Improved Transparency's");
    Ok(Some(Arc::new(crate::clouds::pack::Programs {
        classic,
        oit,
    })))
}

/// The packs' core shaders the atlas carries (text, block outline, sky, stars, lightmap,
/// clouds).
pub(super) fn load_shaders(
    atlas: &mut super::HudAtlas,
    root: &ResourceSource,
    defaults: &ResourceSource,
) -> Result<(), RequiredShaderRefused> {
    // The packs' expression uniforms (`shader_modifiers`), read once, with their config entries.
    let configs = crate::packs::polytone_configs(&crate::packs::Source::Opened(root.clone()));
    let modifiers = crate::polytone_post::ShaderModifiers::read(root, configs);
    let lines = load_with_expressions(
        root,
        defaults,
        &modifiers,
        crate::gfx::expression_blocks::LINES[0],
        ("rendertype_lines", &[]),
        &crate::gfx::core_lines::GLUE,
    );
    (atlas.lines_shader, atlas.lines_expressions) = lines.unzip_or_default();
    let sky = |id, label| {
        let glue = &crate::gfx::core_sky::GLUE;
        load_with_expressions(root, defaults, &modifiers, id, (label, &[]), glue)
    };
    (atlas.sky_shader, atlas.sky_expressions) =
        sky(crate::gfx::expression_blocks::SKY[0], "sky").unzip_or_default();
    (atlas.stars_shader, atlas.stars_expressions) =
        sky(crate::gfx::expression_blocks::STARS[0], "stars").unzip_or_default();
    let lightmap = load_lightmap(root, defaults, &modifiers);
    atlas.lightmap_shader = lightmap.as_ref().map(|(program, _)| program.clone());
    atlas.lightmap_expressions = lightmap.map(|(_, e)| e).unwrap_or_default();
    atlas.clouds_shader = load_clouds(root, defaults)?;
    // The GUI's, the world's and the see-through text: one shader, three programs.
    let text = |program: (&str, &[&str]), glue| {
        let id = crate::gfx::expression_blocks::TEXT[0];
        load_with_expressions(root, defaults, &modifiers, id, program, glue)
    };
    let gui = text(("GUI text", &["IS_GUI"]), &GLUE);
    let world = text(("world text", &[]), &crate::gfx::core_text::WORLD_GLUE);
    let see_through = text(
        ("see-through text", &["IS_SEE_THROUGH"]),
        &crate::gfx::core_text::SEE_THROUGH_GLUE,
    );
    let shaders = &mut atlas.text_shaders;
    let expressions = &mut shaders.expressions;
    (shaders.gui, expressions.gui) = gui.unzip_or_default();
    (shaders.world, expressions.world) = world.unzip_or_default();
    (shaders.see_through, expressions.see_through) = see_through.unzip_or_default();
    Ok(())
}

/// The pipeline and its resources.
pub(super) struct PackText {
    pipeline: Pipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    /// The Polytone blocks its stages read: `PolyGlobals` and the packs' expression uniforms
    /// (`shader_modifiers`).
    pub(super) extra: crate::gfx::expression_blocks::ExpressionBlocks,
}

impl PackText {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas: &wgpu::TextureView,
        (program, expressions): (&CoreProgram, &[crate::polytone_post::ExpressionUniform]),
        coverage: bool,
    ) -> Option<Self> {
        // A pack's shader is untrusted: the backend failing to compile it is an internal error.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let module = |label, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex = module("Resource pack text (vertex)", &program.vertex);
        let fragment = module("Resource pack text (fragment)", &program.fragment);
        // Each block and sampler bound only to the stages that read it: vanilla's four blocks,
        // then `PolyGlobals` and the expression uniforms; every sampler a pack names reads the atlas.
        let blocks = crate::gfx::expression_blocks::vanilla_blocks(program);
        let extra =
            crate::gfx::expression_blocks::ExpressionBlocks::new(program, expressions, 1024);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack text uniforms"),
            size: extra.size(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The font textures' nearest filtering.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Resource pack text"),
            ..Default::default()
        });
        let (mut entries, mut resources) =
            crate::gfx::expression_blocks::bindings(&blocks, &extra, &uniform);
        let textures = crate::core_shader::SAMPLERS.map(|(_, binding)| (binding, atlas, &sampler));
        let (layout, group) = crate::gfx::expression_blocks::sampler_bindings(program, &textures);
        entries.extend(layout);
        resources.extend(group);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Resource pack text"),
            entries: &entries,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Resource pack text"),
            layout: &layout,
            entries: &resources,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Resource pack text"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Unorm8x4];
        let desc = wgpu::RenderPipelineDescriptor {
            label: Some("Resource pack text"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<HudVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                // BlendFunction.TRANSLUCENT.
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        };
        let source = wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(program.fragment.clone()));
        let pipeline = Pipeline::new(device, &desc, source, coverage);
        let internal = pollster::block_on(internal.pop());
        if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
            log::warn!("the resource pack's GUI text shader is not used: {e}");
            return None;
        }
        let Some(pipeline) = pipeline else {
            log::warn!("the resource pack's GUI text shader is not used: it has no colour output");
            return None;
        };
        Some(Self {
            pipeline,
            group,
            uniform,
            extra,
        })
    }

    /// The frame's blocks, as `GuiRenderer` sets them for the GUI, then the expression
    /// uniforms' values.
    pub(super) fn prepare(
        &self,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        batch: &HudBatch,
        size: [u32; 2],
        scale: f32,
    ) {
        uploads.buffer(encoder, &self.uniform, 0, &uniforms(batch, size, scale));
        self.extra.upload(uploads, encoder, &self.uniform);
    }

    /// Draw `range` (whole glyph quads) with the pack's shader; the caller sets its own
    /// pipeline and bind group again after.
    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        range: std::ops::Range<u32>,
        coverage: bool,
    ) {
        pass.set_pipeline(self.pipeline.get(coverage));
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(range.clone(), range.start..range.start + 1);
    }
}

/// The std140 blocks: `Globals` (`GlobalSettingsUniform.update`: the camera at the origin,
/// `GlintAlpha`, `GameTime`, `ScreenSize`), `Projection` (`GuiRenderer`'s orthographic GUI
/// projection, 1000 to 11000 deep, in GL's depth range), `DynamicTransforms` (the GUI's model view
/// moving 11000 back, colour modulator white) and an empty `Fog`.
/// `scale`: target pixels per GUI pixel (`batch.scale`, times `HudRenderer::zoom`).
fn uniforms(batch: &HudBatch, size: [u32; 2], scale: f32) -> Vec<u8> {
    let mut data = vec![0f32; 256];
    let globals = 0;
    data[globals + 3] = batch.glint_alpha;
    data[globals + 7] = batch.game_time;
    data[globals + 8] = size[0] as f32;
    data[globals + 9] = size[1] as f32;
    // Matrix4f.setOrtho(0, width, height, 0, near, far) with the GUI's scale, columns.
    let (w, h) = (size[0] as f32 / scale, size[1] as f32 / scale);
    let (near, far) = (1000.0f32, 11000.0f32);
    let projection = 64;
    data[projection] = 2.0 / w;
    data[projection + 5] = -2.0 / h;
    data[projection + 10] = 2.0 / (near - far);
    data[projection + 12] = -1.0;
    data[projection + 13] = 1.0;
    data[projection + 14] = (far + near) / (near - far);
    data[projection + 15] = 1.0;
    let transforms = 128;
    for (matrix, z) in [(transforms, -11000.0), (transforms + 16, 0.0)] {
        data[matrix] = 1.0;
        data[matrix + 5] = 1.0;
        data[matrix + 10] = 1.0;
        data[matrix + 14] = z;
        data[matrix + 15] = 1.0;
    }
    data[transforms + 32..transforms + 36].fill(1.0);
    bytemuck::cast_slice(&data).to_vec()
}

/// `range` split into runs of GUI text and of the rest, in order.
pub(super) fn text_runs(
    vertices: &[HudVertex],
    range: std::ops::Range<u32>,
) -> Vec<(std::ops::Range<u32>, bool)> {
    let mut runs: Vec<(std::ops::Range<u32>, bool)> = Vec::new();
    for i in range {
        let text = vertices.get(i as usize).is_some_and(|v| v.text != 0);
        match runs.last_mut() {
            Some((run, t)) if *t == text => run.end = i + 1,
            _ => runs.push((i..i + 1, text)),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gui_projection_maps_gui_pixels_to_the_screen() {
        let mut batch = HudBatch::scratch([1920, 1080]);
        batch.scale = 4.0;
        let data: Vec<f32> =
            bytemuck::cast_slice(&uniforms(&batch, [1920, 1080], batch.scale)).to_vec();
        let column = |matrix: usize, c: usize| &data[matrix + 4 * c..matrix + 4 * c + 4];
        let apply = |matrix: usize, v: [f32; 4]| -> [f32; 4] {
            std::array::from_fn(|r| (0..4).map(|c| column(matrix, c)[r] * v[c]).sum())
        };
        // ProjMat * ModelViewMat * (x, y, 0, 1) for the GUI's corners: 480 x 270 GUI pixels.
        for ([x, y], [cx, cy]) in [([0.0, 0.0], [-1.0, 1.0]), ([480.0, 270.0], [1.0, -1.0])] {
            let clip = apply(64, apply(128, [x, y, 0.0, 1.0]));
            assert!(
                (clip[0] - cx).abs() < 1e-5 && (clip[1] - cy).abs() < 1e-5,
                "{clip:?}"
            );
            // At GL's far plane, as vanilla's GUI is.
            assert!((clip[2] - 1.0).abs() < 1e-4, "{clip:?}");
        }
        // A pack finds the GUI scale as ScreenSize.x * ProjMat[0][0] / 2.
        assert!((data[8] * data[64] / 2.0 - 4.0).abs() < 1e-4);
    }

    #[test]
    fn filtering_a_required_cloud_stage_fails_the_load() {
        use sunrise_assets::resource_source::Resource;
        let vertex = "assets/minecraft/shaders/core/clouds.vsh";
        let fragment = "assets/minecraft/shaders/core/clouds.fsh";
        let stack = |paths: &[&str]| {
            ResourceSource::indexed(
                paths
                    .iter()
                    .map(|&path| (path.to_owned(), Resource::bytes(b"void main() {}".to_vec())))
                    .collect(),
            )
        };
        let game = stack(&[vertex, fragment]);
        assert!(matches!(load_clouds(&game, &game), Ok(None)));
        for (visible, hidden) in [
            (&[vertex][..], fragment),
            (&[fragment][..], vertex),
            (&[][..], vertex),
        ] {
            let Err(error) = load_clouds(&stack(visible), &game) else {
                panic!("load accepted a stack missing {hidden}");
            };
            assert_eq!(error.pipeline, crate::gfx::core_clouds::CLASSIC_PIPELINE);
            assert_eq!(
                error.why,
                Refusal::StageHidden {
                    path: hidden.into()
                }
                .to_string()
            );
        }
    }

    /// Packs in memory: vanilla's lightmap stages (in shape) as the defaults, and `own` files
    /// of a pack over them.
    fn lightmap_packs(own: &[(&str, &str)]) -> (ResourceSource, ResourceSource) {
        use sunrise_assets::resource_source::Resource;
        let vanilla = [
            (
                "assets/minecraft/shaders/core/screenquad.vsh",
                "#version 330\nlayout(location = 0) out vec2 texCoord;\n\
                 void main() {\n    vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n\
                     gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);\n    texCoord = uv;\n}\n",
            ),
            (
                "assets/minecraft/shaders/core/lightmap.fsh",
                "#version 330\nlayout(std140) uniform LightmapInfo {\n    float SkyFactor;\n} info;\n\
                 layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
                 void main() {\n    fragColor = vec4(info.SkyFactor);\n}\n",
            ),
        ];
        let source = |files: Vec<(&str, &str)>| {
            ResourceSource::indexed(
                files
                    .into_iter()
                    .map(|(path, text)| {
                        (path.to_owned(), Resource::bytes(text.as_bytes().to_vec()))
                    })
                    .collect(),
            )
        };
        let mut pack = vanilla.to_vec();
        for (path, text) in own {
            pack.retain(|(p, _)| p != path);
            pack.push((path, text));
        }
        (source(pack), source(vanilla.to_vec()))
    }

    fn lightmap(own: &[(&str, &str)]) -> Option<Arc<CoreProgram>> {
        lightmap_with_expressions(own).map(|(program, _)| program)
    }

    /// [`lightmap`] with its expression uniforms, as the atlas loads them.
    fn lightmap_with_expressions(
        own: &[(&str, &str)],
    ) -> Option<(
        Arc<CoreProgram>,
        Vec<crate::polytone_post::ExpressionUniform>,
    )> {
        let (root, defaults) = lightmap_packs(own);
        let modifiers = crate::polytone_post::ShaderModifiers::read(&root, Default::default());
        load_lightmap(&root, &defaults, &modifiers)
    }

    /// A pack's lightmap reading an expression uniform (`Tint`) is given it when a pack's
    /// `shader_modifiers` gives either stage one of that name, and refused for an unknown block
    /// otherwise.
    #[test]
    fn a_lightmap_shader_is_given_the_packs_expression_uniforms() {
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let tinted = "#version 330\nlayout(std140) uniform LightmapInfo {\n    float SkyFactor;\n} info;\n\
                      layout(std140) uniform Tint {\n    float value;\n};\n\
                      layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
                      void main() {\n    fragColor = vec4(value, info.SkyFactor, 0.0, 1.0);\n}\n";
        let on =
            |shader: &str| format!("assets/minecraft/polytone/shader_modifiers/core/{shader}.json");
        for stage in ["lightmap", "screenquad"] {
            let (program, expressions) =
                lightmap_with_expressions(&[(fsh, tinted), (&on(stage), r#"{"Tint": "1"}"#)])
                    .unwrap_or_else(|| panic!("given by {stage}"));
            assert_eq!(program.other, ["LightmapInfo", "Tint"]);
            let resolved: Vec<_> = expressions.iter().map(|e| (&*e.name, e.stages)).collect();
            assert_eq!(resolved, [("Tint", wgpu::ShaderStages::FRAGMENT)]);
        }
        // A block the fragment stage declares and doesn't read is not bound; a `vec4` block of
        // the same 16 bytes refuses the shader.
        let unread = tinted.replace("vec4(value, ", "vec4(0.0, ");
        let (_, expressions) =
            lightmap_with_expressions(&[(fsh, &unread), (&on("lightmap"), r#"{"Tint": "1"}"#)])
                .expect("the shader, without the block");
        assert!(expressions.is_empty());
        let wide = tinted
            .replace("float value;", "vec4 value;")
            .replace("vec4(value, ", "vec4(value.x, ");
        assert!(
            lightmap(&[(fsh, &wide), (&on("lightmap"), r#"{"Tint": "1"}"#)]).is_none(),
            "a vec4 block"
        );
        assert!(
            lightmap(&[(fsh, tinted)]).is_none(),
            "no modifier gives Tint"
        );
        assert!(
            lightmap(&[(fsh, tinted), (&on("sky"), r#"{"Tint": "1"}"#)]).is_none(),
            "another shader's"
        );
    }

    /// A pack's lightmap importing Polytone's `polytone_globals.glsl` is given `PolyGlobals`
    /// without shipping the file (or with its own copy), beside the expression uniforms; the
    /// block is the game's, not an expression a modifier gives.
    #[test]
    fn a_lightmap_shader_is_given_poly_globals() {
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let reading = "#version 330\n#moj_import <polytone_globals.glsl>\n\
                       layout(std140) uniform LightmapInfo {\n    float SkyFactor;\n} info;\n\
                       layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
                       void main() {\n    fragColor = vec4(PolySunAngle, info.SkyFactor, 0.0, 1.0);\n}\n";
        let (program, expressions) =
            lightmap_with_expressions(&[(fsh, reading)]).expect("the built-in include");
        assert_eq!(program.other, ["PolyGlobals", "LightmapInfo"]);
        assert!(expressions.is_empty());
        // The pack's own copy is read instead: only it has `shipped()`, which the shader calls,
        // so the built-in standing in for it would refuse the shader.
        let include = "assets/minecraft/shaders/include/polytone_globals.glsl";
        let own = format!(
            "{}float shipped() {{ return 0.5; }}\n",
            sunrise_polytone::globals::GLSL
        );
        let calling = reading.replace("vec4(PolySunAngle,", "vec4(PolySunAngle * shipped(),");
        let (program, _) =
            lightmap_with_expressions(&[(fsh, &calling), (include, &own)]).expect("the pack's own");
        assert_eq!(program.other, ["PolyGlobals", "LightmapInfo"]);
        assert!(
            lightmap(&[(fsh, &calling)]).is_none(),
            "the built-in has no shipped()"
        );
        // Its copy declaring the block otherwise refuses the shader.
        let short = "layout(std140) uniform PolyGlobals {\n    float PolySunAngle;\n};\n";
        assert!(lightmap(&[(fsh, reading), (include, short)]).is_none());
        assert!(
            lightmap(&[(fsh, &reading.replace("polytone_globals", "other"))]).is_none(),
            "another missing import"
        );
    }

    /// A pack's `polytone_globals.glsl` that can't be read refuses the shader by its path; the
    /// built-in stands in only for a missing one.
    #[test]
    fn an_unreadable_poly_globals_refuses_the_shader() {
        use sunrise_assets::resource_source::Resource;
        let (_, defaults) = lightmap_packs(&[]);
        let fsh = "#version 330\n#moj_import <polytone_globals.glsl>\n\
                   layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
                   void main() {\n    fragColor = vec4(PolySunAngle);\n}\n";
        let include = "assets/minecraft/shaders/include/polytone_globals.glsl";
        let mut files: std::collections::BTreeMap<String, Resource> = (defaults.list("assets/"))
            .into_iter()
            .map(|path| {
                let bytes = defaults.read(&path).unwrap();
                (path, Resource::bytes(bytes))
            })
            .collect();
        files.insert(
            "assets/minecraft/shaders/core/lightmap.fsh".to_owned(),
            Resource::bytes(fsh.as_bytes().to_vec()),
        );
        let translation = |files: std::collections::BTreeMap<String, Resource>| {
            translate_pair(
                &ResourceSource::indexed(files),
                &defaults,
                ("screenquad", "lightmap"),
                &[],
                &crate::gfx::core_lightmap::GLUE,
                &|block| block == sunrise_polytone::globals::BLOCK,
            )
            .unwrap()
            .map(|_| ())
        };
        assert_eq!(translation(files.clone()), Ok(()));
        // The built-in standing in isn't a missing include: a shader failing for another reason
        // is refused for that reason.
        let mut broken = files.clone();
        broken.insert(
            "assets/minecraft/shaders/core/lightmap.fsh".to_owned(),
            Resource::bytes(
                fsh.replace("vec4(PolySunAngle)", "vec4(Undeclared)")
                    .into_bytes(),
            ),
        );
        assert!(matches!(translation(broken), Err(Refusal::Translation(_))));
        files.insert(include.to_owned(), Resource::bytes(vec![0xff, 0xfe]));
        assert_eq!(
            translation(files),
            Err(Refusal::NotText {
                path: include.to_owned()
            })
        );
    }

    /// [`lightmap`]'s translation, with why it is refused.
    fn lightmap_translation(own: &[(&str, &str)]) -> Option<Result<CoreProgram, Refusal>> {
        let (root, defaults) = lightmap_packs(own);
        translate_pair(
            &root,
            &defaults,
            ("screenquad", "lightmap"),
            &[],
            &crate::gfx::core_lightmap::GLUE,
            &|block| block == crate::gfx::core_lightmap::INFO_BLOCK,
        )
    }

    /// A pack's `lightmap.fsh` importing `"tint.glsl"`.
    const IMPORTING_FSH: &str = "#version 330\n#moj_import \"tint.glsl\"\n\
        layout(std140) uniform LightmapInfo {\n    float SkyFactor;\n} info;\n\
        layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
        void main() {\n    fragColor = tint(info.SkyFactor);\n}\n";
    const TINT_GLSL: &str = "vec4 tint(float f) { return vec4(f, 0.5 * f, 0.0, 1.0); }\n";

    /// The packs change a core shader when a stage, with its includes resolved, differs from the
    /// game's: a pack replacing only an include the lightmap's stage imports is translated, with
    /// the pack's include (its 0.25), as is every core shader that `translate_pair` loads; the
    /// game's own include changes nothing.
    #[test]
    fn a_pack_replacing_only_an_include_changes_a_core_shader() {
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let tint = "assets/minecraft/shaders/core/tint.glsl";
        let packs = |include: &str| {
            let (root, _) = lightmap_packs(&[(fsh, IMPORTING_FSH), (tint, include)]);
            root
        };
        let defaults = packs(TINT_GLSL);
        let translate = |root: &ResourceSource| {
            translate_pair(
                root,
                &defaults,
                ("screenquad", "lightmap"),
                &[],
                &crate::gfx::core_lightmap::GLUE,
                &|block| block == crate::gfx::core_lightmap::INFO_BLOCK,
            )
        };
        assert!(translate(&defaults).is_none(), "the game's own");
        let program = translate(&packs(&TINT_GLSL.replace("0.5 * f", "0.25 * f")))
            .expect("the pack's include changes the shader")
            .expect("it translates");
        let literal = |module: &naga::Module, value: f32| {
            module.functions.iter().any(|(_, f)| {
                f.expressions.iter().any(
                    |(_, e)| matches!(e, naga::Expression::Literal(naga::Literal::F32(v)) if *v == value),
                )
            })
        };
        assert!(literal(&program.fragment, 0.25) && !literal(&program.fragment, 0.5));
    }

    /// A stage the pack doesn't replace, and an include, come from the stack the selected packs
    /// left, never from the game's resources beside it: one a pack's filter hid refuses the
    /// shader, named, instead of coming back.
    #[test]
    fn a_file_the_selected_packs_hid_does_not_come_back() {
        use sunrise_assets::resource_source::Resource;
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let vsh = "assets/minecraft/shaders/core/screenquad.vsh";
        let tint = "assets/minecraft/shaders/core/tint.glsl";
        let (full, defaults) = lightmap_packs(&[(fsh, IMPORTING_FSH), (tint, TINT_GLSL)]);
        // The stack without one file: a filter hid it, though the game's resources have it.
        let hiding = |hidden: &str| {
            let files = full
                .list("assets/")
                .into_iter()
                .filter(|p| p != hidden)
                .map(|p| {
                    let text = full.read(&p).unwrap();
                    (p, Resource::bytes(text))
                })
                .collect();
            ResourceSource::indexed(files)
        };
        let defaults_with_tint = {
            let mut files: std::collections::BTreeMap<String, Resource> = defaults
                .list("assets/")
                .into_iter()
                .map(|p| {
                    let text = defaults.read(&p).unwrap();
                    (p, Resource::bytes(text))
                })
                .collect();
            files.insert(
                tint.to_owned(),
                Resource::bytes(TINT_GLSL.as_bytes().to_vec()),
            );
            ResourceSource::indexed(files)
        };
        let translate = |root: &ResourceSource| {
            translate_pair(
                root,
                &defaults_with_tint,
                ("screenquad", "lightmap"),
                &[],
                &crate::gfx::core_lightmap::GLUE,
                &|block| block == crate::gfx::core_lightmap::INFO_BLOCK,
            )
            .expect("the pack replaces a stage")
        };
        assert!(translate(&full).is_ok(), "nothing hidden");
        assert_eq!(
            translate(&hiding(vsh)).expect_err("the screen quad hidden"),
            Refusal::StageHidden { path: vsh.into() }
        );
        assert_eq!(
            translate(&hiding(tint)).expect_err("the include hidden"),
            Refusal::IncludeMissing {
                id: "minecraft:shaders/core/tint.glsl".into()
            }
        );
    }

    /// A stage that isn't UTF-8 text refuses the shader, named.
    #[test]
    fn a_stage_that_is_not_text_is_refused_by_its_path() {
        use sunrise_assets::resource_source::Resource;
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let (full, defaults) = lightmap_packs(&[]);
        let files = full
            .list("assets/")
            .into_iter()
            .map(|p| {
                let bytes = if p == fsh {
                    vec![0xff, 0xfe, 0x00]
                } else {
                    full.read(&p).unwrap()
                };
                (p, Resource::bytes(bytes))
            })
            .collect();
        let root = ResourceSource::indexed(files);
        let refused = translate_pair(
            &root,
            &defaults,
            ("screenquad", "lightmap"),
            &[],
            &crate::gfx::core_lightmap::GLUE,
            &|block| block == crate::gfx::core_lightmap::INFO_BLOCK,
        );
        assert_eq!(
            refused.expect("the pack replaces a stage").err(),
            Some(Refusal::NotText { path: fsh.into() })
        );
    }

    /// A panic in the translator is a refusal, not the game's end.
    #[test]
    fn a_translator_panic_refuses_the_shader() {
        assert!(matches!(
            contained(|| panic!("a fault in the translator")),
            Err(Refusal::Panicked(_))
        ));
    }

    /// A quoted import in a pack's `core/lightmap` is read from the shader's own folder,
    /// `minecraft:shaders/core/`, not from `shaders/include/`.
    #[test]
    fn a_lightmap_shaders_quoted_import_is_read_from_its_own_folder() {
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let beside = "assets/minecraft/shaders/core/tint.glsl";
        let program = lightmap_translation(&[(fsh, IMPORTING_FSH), (beside, TINT_GLSL)])
            .expect("the pack replaces a stage")
            .expect("the import beside the shader is found");
        assert_eq!(program.other, ["LightmapInfo"]);
        let include = "assets/minecraft/shaders/include/tint.glsl";
        let why = lightmap_translation(&[(fsh, IMPORTING_FSH), (include, TINT_GLSL)])
            .expect("the pack replaces a stage")
            .expect_err("an import only under shaders/include is not beside the shader");
        assert_eq!(
            why,
            Refusal::IncludeMissing {
                id: "minecraft:shaders/core/tint.glsl".into()
            }
        );
    }

    /// A shader file is read only up to the translator's 1 MiB budget: an import or a stage
    /// over it refuses the pack's lightmap, naming the file, and one at the budget is read.
    #[test]
    fn a_lightmap_shader_file_over_the_budget_is_refused_by_its_path() {
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let tint = "assets/minecraft/shaders/core/tint.glsl";
        // A comment pads the include to exactly `len` bytes.
        let padded = |len: usize| {
            let pad = len - TINT_GLSL.len() - 4;
            format!("// {}\n{TINT_GLSL}", "x".repeat(pad))
        };
        let at_budget = padded(1 << 20);
        assert_eq!(at_budget.len(), 1_048_576);
        let over = padded((1 << 20) + 1);
        let refused = |files: &[(&str, &str)]| {
            lightmap_translation(files)
                .expect("the pack replaces a stage")
                .expect_err("refused")
        };
        assert_eq!(
            refused(&[(fsh, IMPORTING_FSH), (tint, &over)]),
            Refusal::OverBudget { path: tint.into() }
        );
        // The same file at the budget gets past the read (and the translator then finds the
        // expanded shader too long, a refusal of its own).
        assert!(matches!(
            refused(&[(fsh, IMPORTING_FSH), (tint, &at_budget)]),
            Refusal::Translation(_)
        ));
        let big_stage = format!("// {}\n{IMPORTING_FSH}", "x".repeat(1 << 20));
        assert_eq!(
            refused(&[(fsh, &big_stage), (tint, TINT_GLSL)]),
            Refusal::OverBudget { path: fsh.into() }
        );
        assert!(lightmap(&[(fsh, &big_stage), (tint, TINT_GLSL)]).is_none());
    }

    /// A pack's `core/lightmap` is loaded with vanilla's `core/screenquad` (a stage of another
    /// name), and the reverse; none when no pack replaces either; refused when it reads a block
    /// the lightmap pass doesn't give.
    #[test]
    fn a_pack_replacing_either_lightmap_stage_is_loaded() {
        let red = "#version 330\nlayout(std140) uniform LightmapInfo {\n    float SkyFactor;\n} info;\n\
                   layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
                   void main() {\n    fragColor = vec4(info.SkyFactor, 0.0, 0.0, 1.0);\n}\n";
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let vsh = "assets/minecraft/shaders/core/screenquad.vsh";
        let program = lightmap(&[(fsh, red)]).expect("the pack's lightmap");
        assert_eq!(program.other, ["LightmapInfo"]);
        let quad = "#version 330\nlayout(location = 0) out vec2 texCoord;\n\
                    void main() {\n    texCoord = vec2(0.5);\n    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n}\n";
        assert!(lightmap(&[(vsh, quad)]).is_some());
        assert!(lightmap(&[]).is_none(), "no pack replaces either stage");
        let other = red.replace("LightmapInfo", "Elsewhere");
        assert!(lightmap(&[(fsh, &other)]).is_none());
    }

    #[test]
    fn text_splits_from_the_rest() {
        let vertex = |text| HudVertex {
            pos: [0.0; 2],
            uv: [0.0; 2],
            color: 0,
            depth: 0.0,
            glint_uv: [0.0; 2],
            text,
        };
        let vertices: Vec<HudVertex> = [0, 0, 1, 1, 1, 0].into_iter().map(vertex).collect();
        assert_eq!(
            text_runs(&vertices, 1..6),
            [(1..2, false), (2..5, true), (5..6, false)]
        );
    }
}

#[cfg(test)]
#[path = "pack_text_text_tests.rs"]
pub(crate) mod text_tests;

#[cfg(test)]
#[path = "pack_text_gpu_tests.rs"]
mod gpu_tests;

#[cfg(test)]
#[path = "pack_text_clouds_tests.rs"]
mod clouds_tests;
