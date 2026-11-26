//! A resource pack's `core/text` shader on the GUI's text (`RenderPipelines.GUI_TEXT`, the shader
//! with `IS_GUI`): servers' HUDs that their pack's vertex shader moves to the screen's corners,
//! and text effects in its fragment shader. Without one the text draws natively.

use crate::resource_source::ResourceSource;
use std::sync::Arc;

use sunrise_glsl::{Glue, Translated};

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
    fragment_finish: None,
    interface: sunrise_shaders::DEVICELESS_INTERFACE_LIMITS,
};

/// Why a pack's core shader is refused (`translate_selected`), with the file it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A stage or include over the translator's budget for a whole shader
    /// (`sunrise_glsl::MAX_EXPANDED`).
    OverBudget { path: String },
    /// A stage or include that isn't UTF-8 text.
    NotText { path: String },
    /// A stage or include that couldn't be read otherwise.
    Unreadable { path: String, error: String },
    /// A stage the selected packs hid (`root` doesn't have it).
    StageHidden { path: String },
    /// An include the selected packs don't have (`ns:path`, as the shader asks for it).
    IncludeMissing { id: String },
    /// An include refused (`refusal`: missing, unreadable, over the budget, not text) at the
    /// directive asking for it, with the authored file and line and the stage that asked.
    Include {
        file: String,
        line: u32,
        stage: Option<naga::ShaderStage>,
        refusal: Box<Refusal>,
    },
    /// The translator refused the GLSL, with the authored positions and responsible stages.
    Translation(Vec<sunrise_glsl::Error>),
    /// The translator panicked on it.
    Panicked(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverBudget { path } => write!(
                f,
                "{path} is over the {} bytes a shader may have",
                sunrise_glsl::MAX_EXPANDED
            ),
            Self::NotText { path } => write!(f, "{path} is not UTF-8 text"),
            Self::Unreadable { path, error } => write!(f, "{path}: {error}"),
            Self::StageHidden { path } => write!(f, "{path} is not in the selected packs"),
            Self::IncludeMissing { id } => write!(f, "{id} is not in the selected packs"),
            Self::Include {
                file,
                line,
                stage,
                refusal,
            } => {
                let message = sunrise_glsl::shader_message(*stage, refusal);
                write!(f, "{file}:{line}: {message}")
            }
            Self::Translation(errors) => f.write_str(&sunrise_glsl::describe(errors)),
            Self::Panicked(e) => write!(f, "the shader translator failed: {e}"),
        }
    }
}

/// The id of the core shader stage `name` (`minecraft:shaders/core/<name>.<extension>`), which
/// errors in it name.
fn stage_id(name: &str, extension: &str) -> String {
    format!("minecraft:shaders/core/{name}.{extension}")
}

/// Both stages as the selected packs have them, when the packs change the shader: none when they
/// don't, or why a stage can't be read. A stage the packs don't replace, and every include, is
/// the stack's (`root`: what the selected packs left of the game's resources), never `defaults`
/// beside it, so a file a pack's filter hid stays hidden and refuses the shader. `defaults` only
/// tells a pack's shader from the game's: the packs change it when either stage, with its
/// includes resolved under any of `variants`' defines, differs from the game's (a pack replacing
/// only an include one of them keeps changes it). Each stage and include is read only up to the
/// translator's budget ([`read_shader_file`]); a stage over it refuses the shader by its path, an
/// include only where a program's defines keep it (`core_shader::expand_core_imports`). A panic
/// refuses it too.
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
                // The text and the refusal: where each line came from changes nothing.
                let resolved = |base| {
                    crate::core_shader::expand_core_imports(
                        text,
                        &stage_id(name, extension),
                        "minecraft:shaders/core/",
                        defines,
                        &|id| include_of(base, id).1,
                    )
                    .map(|expanded| expanded.text().to_owned())
                    .map_err(|e| e.refusal)
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

/// The program `stages` (as [`selected_stages`] gives them) make with `defines` and `glue`, the
/// pipeline giving the blocks and samplers `provided` says yes to; or why it is refused, a file
/// the translation couldn't read named by its path and the stage that needed it, a panic in
/// the translator a refusal too
/// ([`contained`]).
fn translate_selected(
    root: &ResourceSource,
    (vertex, fragment): &(String, String),
    (vertex_name, fragment_name): (&str, &str),
    defines: &[&str],
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Result<Translated, Refusal> {
    let unavailable = std::cell::RefCell::new(None);
    let include = |id: &str| {
        let (refusal, text) = include_of(root, id);
        *unavailable.borrow_mut() = refusal;
        text
    };
    let (vertex_file, fragment_file) =
        (stage_id(vertex_name, "vsh"), stage_id(fragment_name, "fsh"));
    let stages = sunrise_glsl::Stages {
        vertex_dir: "minecraft:shaders/core/",
        fragment_dir: "minecraft:shaders/core/",
        vertex,
        fragment,
        vertex_file: &vertex_file,
        fragment_file: &fragment_file,
    };
    let translated = contained(|| {
        crate::core_shader::translate_core_at(&stages, defines, &include, glue, provided)
    });
    // An include that couldn't be given stops the translation at its directive, the error's
    // file, line and stage.
    translated.map_err(|e| match (e, unavailable.into_inner()) {
        (Refusal::Translation(errors), Some(refusal)) => match errors.first() {
            Some(at) => Refusal::Include {
                file: at.file.clone(),
                line: at.line,
                stage: at.stage,
                refusal: Box::new(refusal),
            },
            None => refusal,
        },
        (e, _) => e,
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
    translate: impl FnOnce() -> Result<Translated, Vec<sunrise_glsl::Error>>,
) -> Result<Translated, Refusal> {
    crate::crash::contain(translate)
        .map_err(Refusal::Panicked)
        .and_then(|translated| translated.map_err(Refusal::Translation))
}

/// A shader file (a stage or an include) of `base`, read only up to the translator's budget
/// for a whole shader (`sunrise_glsl::MAX_EXPANDED`, 1 MiB): none when `base` hasn't it, the
/// refusal naming its path when it is over the budget or isn't text.
pub(crate) fn read_shader_file(
    base: &ResourceSource,
    path: &str,
) -> Result<Option<String>, Refusal> {
    if !base.contains(path) {
        return Ok(None);
    }
    let max = sunrise_glsl::MAX_EXPANDED as u64;
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

/// One pipeline vanilla 26.3 requires (`RenderPipelines.register`: every pipeline but the
/// wireframe ones, which `ShaderManager.apply` compiles whatever the frame draws) made from a
/// core shader Sunrise runs: its location, its defines, the Improved Transparency stage its
/// `OitPipelineSet` adds (`core_clouds::OIT_STAGES`' index, whose defines follow its own), and
/// the glue of the vertex format Sunrise gives the shader.
pub(crate) struct Required {
    pub pipeline: &'static str,
    pub defines: &'static [&'static str],
    pub oit: Option<usize>,
    pub glue: &'static Glue<'static>,
}

impl Required {
    /// Its defines, the Improved Transparency stage's after its own.
    fn all_defines(&self) -> Vec<&'static str> {
        let stage = self
            .oit
            .map(|i| crate::gfx::core_clouds::OIT_STAGES[i].defines);
        (self.defines.iter().chain(stage.into_iter().flatten()))
            .copied()
            .collect()
    }
}

/// `core/text`'s required pipelines, in `RenderPipelines`' order, each program once: `TEXT_POLYGON_OFFSET`
/// is `TEXT`'s program and `OIT_TEXT_POLYGON_OFFSET` `OIT_TEXT`'s (with `IS_GRAYSCALE` alike).
/// Sunrise draws [`TEXT_WORLD`], [`TEXT_GUI`], [`TEXT_SEE_THROUGH`] and [`TEXT_OIT`] with the
/// pack's shader.
pub(crate) const TEXT: [Required; 12] = {
    let world = &crate::gfx::core_text::WORLD_GLUE;
    let see_through = &crate::gfx::core_text::SEE_THROUGH_GLUE;
    let gray: &[&str] = &["IS_GRAYSCALE"];
    [
        Required {
            pipeline: "minecraft:pipeline/text",
            defines: &[],
            oit: None,
            glue: world,
        },
        Required {
            pipeline: "minecraft:pipeline/oit_depth_bounds_text",
            defines: &[],
            oit: Some(0),
            glue: world,
        },
        Required {
            pipeline: "minecraft:pipeline/oit_transmittance_text",
            defines: &[],
            oit: Some(1),
            glue: world,
        },
        Required {
            pipeline: "minecraft:pipeline/oit_accumulate_text",
            defines: &[],
            oit: Some(2),
            glue: world,
        },
        Required {
            pipeline: "minecraft:pipeline/gui_text",
            defines: &["IS_GUI"],
            oit: None,
            glue: &GLUE,
        },
        Required {
            pipeline: "minecraft:pipeline/text_grayscale",
            defines: gray,
            oit: None,
            glue: world,
        },
        Required {
            pipeline: "minecraft:pipeline/oit_depth_bounds_text_grayscale",
            defines: gray,
            oit: Some(0),
            glue: world,
        },
        Required {
            pipeline: "minecraft:pipeline/oit_transmittance_text_grayscale",
            defines: gray,
            oit: Some(1),
            glue: world,
        },
        Required {
            pipeline: "minecraft:pipeline/oit_accumulate_text_grayscale",
            defines: gray,
            oit: Some(2),
            glue: world,
        },
        Required {
            pipeline: "minecraft:pipeline/gui_text_grayscale",
            defines: &["IS_GUI", "IS_GRAYSCALE"],
            oit: None,
            glue: &GLUE,
        },
        Required {
            pipeline: "minecraft:pipeline/text_see_through",
            defines: &["IS_SEE_THROUGH"],
            oit: None,
            glue: see_through,
        },
        Required {
            pipeline: "minecraft:pipeline/text_grayscale_see_through",
            defines: &["IS_GRAYSCALE", "IS_SEE_THROUGH"],
            oit: None,
            glue: see_through,
        },
    ]
};
/// [`TEXT`]'s pipelines Sunrise draws with the pack's shader.
pub(crate) const TEXT_WORLD: usize = 0;
pub(crate) const TEXT_GUI: usize = 4;
pub(crate) const TEXT_SEE_THROUGH: usize = 10;
/// `OIT_TEXT`'s stages, in `OitStage`'s order.
pub(crate) const TEXT_OIT: [usize; 3] = [1, 2, 3];
const _: () = assert!(
    matches!(TEXT[TEXT_OIT[0]].oit, Some(0))
        && matches!(TEXT[TEXT_OIT[1]].oit, Some(1))
        && matches!(TEXT[TEXT_OIT[2]].oit, Some(2))
        && TEXT[TEXT_OIT[0]].defines.is_empty()
        && TEXT[TEXT_OIT[1]].defines.is_empty()
        && TEXT[TEXT_OIT[2]].defines.is_empty()
);

/// `core/rendertype_lines`' required pipelines, each program once: `LINES`' (also
/// `LINES_TRANSLUCENT`'s, `LINES_DEPTH_BIAS`' and `SECONDARY_BLOCK_OUTLINE`'s, which Sunrise
/// draws with the pack's shader), then `OIT_LINES_TRANSLUCENT`'s stages.
pub(crate) const LINES: [Required; 4] = {
    let glue = &crate::gfx::core_lines::GLUE;
    [
        Required {
            pipeline: "minecraft:pipeline/lines",
            defines: &[],
            oit: None,
            glue,
        },
        Required {
            pipeline: "minecraft:pipeline/oit_depth_bounds_lines_translucent",
            defines: &[],
            oit: Some(0),
            glue,
        },
        Required {
            pipeline: "minecraft:pipeline/oit_transmittance_lines_translucent",
            defines: &[],
            oit: Some(1),
            glue,
        },
        Required {
            pipeline: "minecraft:pipeline/oit_accumulate_lines_translucent",
            defines: &[],
            oit: Some(2),
            glue,
        },
    ]
};

pub(crate) const SKY: [Required; 1] = [Required {
    pipeline: "minecraft:pipeline/sky",
    defines: &[],
    oit: None,
    glue: &crate::gfx::core_sky::GLUE,
}];

pub(crate) const STARS: [Required; 1] = [Required {
    pipeline: "minecraft:pipeline/stars",
    defines: &[],
    oit: None,
    glue: &crate::gfx::core_sky::GLUE,
}];

/// `core/screenquad` with `core/lightmap`.
pub(crate) const LIGHTMAP: [Required; 1] = [Required {
    pipeline: "minecraft:pipeline/lightmap",
    defines: &[],
    oit: None,
    glue: &crate::gfx::core_lightmap::GLUE,
}];

/// What every program of a shader is checked against beside its translation: the packs'
/// expression uniforms (`shader_modifiers`), the shaders' ids in those files (vertex, fragment),
/// and the blocks the pass gives of its own (an Improved Transparency stage adds its samplers).
type Blocks<'a> = (
    &'a crate::polytone_post::ShaderModifiers,
    (&'a str, &'a str),
    &'a [&'a str],
);

/// A program with the expression uniforms of the blocks its stages read.
type Checked = (Translated, Vec<crate::polytone_post::ExpressionUniform>);

/// The packs' core shader stages `names` (`shaders/core/<name>.vsh` and `.fsh`, which may
/// differ, as `core/screenquad` and `core/lightmap` do) over `defaults` (the game's own
/// resources), as vanilla compiles them for each of `required`'s pipelines: none when no pack
/// changes either stage or an include any of them keeps, else each pipeline's program in
/// `required`'s order, with its expression uniforms. A program any of them can't make refuses
/// the whole load, named by that pipeline, whether Sunrise draws it or not: a stage or include
/// that can't be read, the translator refusing it, its stages not meeting
/// (`sunrise_glsl::LinkRefusal`), or a block its stages read that the pipeline can't give, as
/// vanilla's builder refuses a descriptor missing from the pipeline's layout (a block that isn't
/// one float, a `PolyGlobals` declared otherwise, an expression for one of the pass's own
/// blocks; `ShaderModifiers::resolve`).
fn translate_required(
    root: &ResourceSource,
    defaults: &ResourceSource,
    names: (&str, &str),
    required: &[Required],
    (modifiers, ids, own): Blocks<'_>,
) -> Result<Option<Vec<Checked>>, RequiredShaderRefused> {
    let defines: Vec<Vec<&str>> = required.iter().map(Required::all_defines).collect();
    let variants: Vec<&[&str]> = defines.iter().map(Vec::as_slice).collect();
    let Some(stages) = selected_stages(root, defaults, names, &variants) else {
        return Ok(None);
    };
    let stages = stages.map_err(|e| RequiredShaderRefused {
        pipeline: required[0].pipeline,
        why: e.to_string(),
    })?;
    (required.iter().zip(&defines))
        .map(|(r, defines)| {
            let refused = |why: String| RequiredShaderRefused {
                pipeline: r.pipeline,
                why,
            };
            let program = translate_selected(root, &stages, names, defines, r.glue, &|_| true)
                .map_err(|e| refused(e.to_string()))?;
            let stage = r
                .oit
                .map(|i| crate::gfx::core_clouds::OIT_STAGES[i].samplers);
            let own: Vec<&str> = (own.iter().chain(stage.into_iter().flatten()))
                .copied()
                .collect();
            let expressions =
                (modifiers.resolve(ids, &program, &own)).map_err(|e| refused(e.to_string()))?;
            Ok((program, expressions))
        })
        .collect::<Result<_, _>>()
        .map(Some)
}

/// A loaded pack shader program with its expression uniforms, as the atlas keeps them.
type Loaded = (
    Arc<Translated>,
    Vec<crate::polytone_post::ExpressionUniform>,
);

/// A drawn program as the atlas keeps it, said in the log; none, said too, when a vanilla block
/// it declares doesn't fit the slot Sunrise binds it in
/// (`expression_blocks::OversizedBlock`): that pipeline draws natively and the pack stays.
fn loaded((program, expressions): Checked, pipeline: &str) -> Option<Loaded> {
    let own = [crate::gfx::core_lightmap::INFO_BLOCK];
    if let Some(oversized) = crate::gfx::expression_blocks::oversized_block(&program, &own) {
        log::warn!("the resource pack's shader for {pipeline} is drawn natively: {oversized}");
        return None;
    }
    log::info!("the resource pack's shader for {pipeline} translated");
    Some((Arc::new(program), expressions))
}

/// A loaded program and its expression uniforms apart, none of either when it isn't used.
fn unzip_or_default(
    loaded: Option<Loaded>,
) -> (
    Option<Arc<Translated>>,
    Vec<crate::polytone_post::ExpressionUniform>,
) {
    match loaded {
        Some((program, expressions)) => (Some(program), expressions),
        None => (None, Vec::new()),
    }
}

/// The packs' `core/<name>` (one file for both stages: the block outline, the sky, the stars),
/// translated for each of its `required` pipelines; the first is the one Sunrise draws with it,
/// with `PolyGlobals` and the expression uniforms of its file.
fn load_one(
    (root, defaults, modifiers): (
        &ResourceSource,
        &ResourceSource,
        &crate::polytone_post::ShaderModifiers,
    ),
    id: &str,
    required: &[Required],
) -> Result<Option<Loaded>, RequiredShaderRefused> {
    let name = id.strip_prefix("minecraft:core/").unwrap_or(id);
    let own = [sunrise_polytone::globals::BLOCK];
    let blocks = (modifiers, (id, id), &own[..]);
    let Some(programs) = translate_required(root, defaults, (name, name), required, blocks)? else {
        return Ok(None);
    };
    Ok((programs.into_iter().next()).and_then(|p| loaded(p, required[0].pipeline)))
}

/// A pack's core shader that a pipeline vanilla requires can't be made from: in 26.3
/// `ShaderManager.apply` compiles every required pipeline (each pipeline the text, block outline,
/// sky, stars, lightmap and clouds shaders make, every stage of their `OitPipelineSet`s and every
/// grayscale variant included, whether Improved Transparency is on or not and whether Sunrise
/// draws it or not) and fails the reload when one doesn't ("Failed to load required shader
/// programs"), which drops the selected packs. The atlas's load fails with it, and so does making
/// the renderer when the device refuses a pipeline it draws, through the same reload failure.
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
/// load, named by its pipeline; one declaring a vanilla block past the slot Sunrise binds it in
/// leaves the native clouds (`expression_blocks::OversizedBlock`).
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
    let translate = |defines: &[&str], samplers: &[&str], pipeline| {
        translate_selected(
            root,
            &stages,
            ("clouds", "clouds"),
            defines,
            &clouds::GLUE,
            &|name| matches!(name, clouds::INFO_BLOCK | clouds::FACES) || samplers.contains(&name),
        )
        .map_err(|e| e.to_string())
        .and_then(|program| {
            clouds::check(&program)
                .map(|()| program)
                .map_err(|e| e.to_string())
        })
        .map_err(|why| RequiredShaderRefused { pipeline, why })
    };
    let classic = translate(&CLASSIC_DEFINES, &[], clouds::CLASSIC_PIPELINE)?;
    let stage = |i: usize| {
        let s = &OIT_STAGES[i];
        translate(s.defines, s.samplers, s.pipeline)
    };
    let oit = Box::new([stage(0)?, stage(1)?, stage(2)?]);
    let oversized = std::iter::once(&classic)
        .chain(oit.iter())
        .find_map(|p| crate::gfx::expression_blocks::oversized_block(p, &[clouds::INFO_BLOCK]));
    if let Some(oversized) = oversized {
        log::warn!("the resource pack's clouds shader is drawn natively: {oversized}");
        return Ok(None);
    }
    log::info!("the resource pack's clouds shader translated, with Improved Transparency's");
    Ok(Some(Arc::new(crate::clouds::pack::Programs {
        classic,
        oit,
    })))
}

/// The packs' core shaders the atlas carries (text, clouds, block outline, sky, stars,
/// lightmap), each translated for every pipeline vanilla requires of it, in `RenderPipelines`'
/// order: the first program any of them can't make, its blocks included, fails the load
/// ([`RequiredShaderRefused`], [`translate_required`]).
pub(super) fn load_shaders(
    atlas: &mut super::HudAtlas,
    root: &ResourceSource,
    defaults: &ResourceSource,
) -> Result<(), RequiredShaderRefused> {
    // The packs' expression uniforms (`shader_modifiers`), read once, with their config entries.
    let configs = crate::packs::polytone_configs(&crate::packs::Source::Opened(root.clone()));
    let modifiers = crate::polytone_post::ShaderModifiers::read(root, configs);
    let packs = (root, defaults, &modifiers);
    // The GUI's, the world's, the see-through text and Improved Transparency's three stages:
    // one shader, six drawn programs.
    let text_id = crate::gfx::expression_blocks::TEXT[0];
    let mut text: [Option<Loaded>; 6] = Default::default();
    let own = [sunrise_polytone::globals::BLOCK];
    let blocks = (&modifiers, (text_id, text_id), &own[..]);
    if let Some(programs) = translate_required(root, defaults, ("text", "text"), &TEXT, blocks)? {
        let mut programs: Vec<Option<Checked>> = programs.into_iter().map(Some).collect();
        // Each program Sunrise draws, by its place in `TEXT`.
        for (slot, index) in text.iter_mut().zip([
            TEXT_WORLD,
            TEXT_GUI,
            TEXT_SEE_THROUGH,
            TEXT_OIT[0],
            TEXT_OIT[1],
            TEXT_OIT[2],
        ]) {
            let program = programs[index].take().expect("each drawn program once");
            *slot = loaded(program, TEXT[index].pipeline);
        }
    }
    let [world, gui, see_through, oit @ ..] = text;
    let shaders = &mut atlas.text_shaders;
    let expressions = &mut shaders.expressions;
    (shaders.gui, expressions.gui) = unzip_or_default(gui);
    (shaders.world, expressions.world) = unzip_or_default(world);
    (shaders.see_through, expressions.see_through) = unzip_or_default(see_through);
    // The stages draw together or not at all.
    shaders.oit = match oit {
        [Some(a), Some(b), Some(c)] => Some(Box::new(crate::gfx::core_text::OitPrograms {
            programs: [a.0, b.0, c.0],
            expressions: [a.1, b.1, c.1],
        })),
        _ => None,
    };
    atlas.clouds_shader = load_clouds(root, defaults)?;
    let lines = crate::gfx::expression_blocks::LINES[0];
    (atlas.lines_shader, atlas.lines_expressions) =
        unzip_or_default(load_one(packs, lines, &LINES)?);
    let sky = crate::gfx::expression_blocks::SKY[0];
    (atlas.sky_shader, atlas.sky_expressions) = unzip_or_default(load_one(packs, sky, &SKY)?);
    let stars = crate::gfx::expression_blocks::STARS[0];
    (atlas.stars_shader, atlas.stars_expressions) =
        unzip_or_default(load_one(packs, stars, &STARS)?);
    (atlas.lightmap_shader, atlas.lightmap_expressions) =
        unzip_or_default(load_lightmap(root, defaults, &modifiers)?);
    Ok(())
}

/// The packs' `core/lightmap` with `core/screenquad`, when a pack replaces either stage or an
/// include they keep: the pass gives it `LightmapInfo` besides vanilla's blocks, and the packs'
/// expression uniforms (`modifiers`) of either stage's file. Unspecified float blocks read zero.
fn load_lightmap(
    root: &ResourceSource,
    defaults: &ResourceSource,
    modifiers: &crate::polytone_post::ShaderModifiers,
) -> Result<Option<Loaded>, RequiredShaderRefused> {
    let shaders = crate::gfx::core_lightmap::SHADERS;
    let names = ("screenquad", "lightmap");
    let own = [
        crate::gfx::core_lightmap::INFO_BLOCK,
        sunrise_polytone::globals::BLOCK,
    ];
    let blocks = (modifiers, (shaders[0], shaders[1]), &own[..]);
    let Some(programs) = translate_required(root, defaults, names, &LIGHTMAP, blocks)? else {
        return Ok(None);
    };
    Ok((programs.into_iter().next()).and_then(|p| loaded(p, LIGHTMAP[0].pipeline)))
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
        shader: (&Translated, &[crate::polytone_post::ExpressionUniform]),
        coverage: bool,
    ) -> Result<Option<Self>, String> {
        let pipeline = TEXT[TEXT_GUI].pipeline;
        // A program past the device's limits draws natively (`sunrise_glsl::PastDeviceLimit`).
        if !crate::core_shader::within_device_limits(device, shader.0, pipeline, &[format]) {
            return Ok(None);
        }
        let made = crate::core_shader::scoped(device, pipeline, || {
            Self::made(device, format, atlas, shader, coverage)
        })?;
        if made.is_none() {
            log::warn!(
                "the resource pack's GUI text shader is not used: it writes no colour at location 0"
            );
        }
        Ok(made)
    }

    /// The pipeline, made whatever the coverage mode so the device checks it, or none when the
    /// fragment stage writes no colour at location 0 (which the GUI's coverage pass wraps): the
    /// GUI text then draws natively, in every mode.
    fn made(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas: &wgpu::TextureView,
        (program, expressions): (&Translated, &[crate::polytone_post::ExpressionUniform]),
        coverage: bool,
    ) -> Option<Self> {
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
        let pipeline = Pipeline::new(device, &desc, source, coverage)?;
        if !program.fragment_writes().contains(&0) {
            return None;
        }
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

    /// One program of the packs' stages `names` with `defines`, as [`translate_required`] makes
    /// each: none when no pack changes the shader, else the translation, or why it is refused.
    fn translate_pair(
        root: &ResourceSource,
        defaults: &ResourceSource,
        names: (&str, &str),
        defines: &[&str],
        glue: &Glue,
        provided: &dyn Fn(&str) -> bool,
    ) -> Option<Result<Translated, Refusal>> {
        let stages = match selected_stages(root, defaults, names, &[defines])? {
            Ok(stages) => stages,
            Err(e) => return Some(Err(e)),
        };
        Some(translate_selected(
            root, &stages, names, defines, glue, provided,
        ))
    }

    /// `refusal` of an include asked for on `line` of the pack's `lightmap.fsh`.
    fn in_lightmap(line: u32, refusal: Refusal) -> Refusal {
        Refusal::Include {
            file: "minecraft:shaders/core/lightmap.fsh".into(),
            line,
            stage: Some(naga::ShaderStage::Fragment),
            refusal: Box::new(refusal),
        }
    }

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

    /// The pack's lightmap program as the atlas loads it: none when no pack changes it, or why
    /// `LIGHTMAP` can't be made from it.
    fn lightmap(own: &[(&str, &str)]) -> Result<Option<Arc<Translated>>, RequiredShaderRefused> {
        lightmap_with_expressions(own).map(|loaded| loaded.map(|(program, _)| program))
    }

    /// [`lightmap`] with its expression uniforms, as the atlas loads them.
    fn lightmap_with_expressions(
        own: &[(&str, &str)],
    ) -> Result<Option<Loaded>, RequiredShaderRefused> {
        let (root, defaults) = lightmap_packs(own);
        let modifiers = crate::polytone_post::ShaderModifiers::read(&root, Default::default());
        load_lightmap(&root, &defaults, &modifiers)
    }

    /// A pack's lightmap reading an expression uniform (`Tint`) is given it when a pack's
    /// `shader_modifiers` gives either stage one of that name, or zero otherwise.
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
                    .unwrap()
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
                .unwrap()
                .expect("the shader, without the block");
        assert!(expressions.is_empty());
        let wide = tinted
            .replace("float value;", "vec4 value;")
            .replace("vec4(value, ", "vec4(value.x, ");
        // The pipeline gives no `vec4` block: the load fails.
        assert_eq!(
            lightmap(&[(fsh, &wide)]).unwrap_err().pipeline,
            "minecraft:pipeline/lightmap",
            "a vec4 block"
        );
        let other = on("sky");
        for files in [
            vec![(fsh, tinted)],
            vec![(fsh, tinted), (&other, r#"{"Tint": "1"}"#)],
        ] {
            let (_, expressions) = lightmap_with_expressions(&files)
                .unwrap()
                .expect("shader uses zero");
            assert_eq!(expressions.len(), 1);
            assert_eq!(expressions[0].name, "Tint");
            assert_eq!(expressions[0].evaluate(&Default::default()), 0.0);
        }
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
        let (program, expressions) = lightmap_with_expressions(&[(fsh, reading)])
            .unwrap()
            .expect("the built-in include");
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
        let (program, _) = lightmap_with_expressions(&[(fsh, &calling), (include, &own)])
            .unwrap()
            .expect("the pack's own");
        assert_eq!(program.other, ["PolyGlobals", "LightmapInfo"]);
        // The built-in has no `shipped()`: the shader doesn't translate, which fails the load.
        assert_eq!(
            lightmap(&[(fsh, &calling)]).unwrap_err().pipeline,
            "minecraft:pipeline/lightmap"
        );
        // Its copy declaring the block otherwise fails the load too.
        let short = "layout(std140) uniform PolyGlobals {\n    float PolySunAngle;\n};\n";
        assert_eq!(
            lightmap(&[(fsh, reading), (include, short)])
                .unwrap_err()
                .pipeline,
            "minecraft:pipeline/lightmap"
        );
        let missing = lightmap(&[(fsh, &reading.replace("polytone_globals", "other"))]);
        assert_eq!(
            missing.unwrap_err(),
            RequiredShaderRefused {
                pipeline: "minecraft:pipeline/lightmap",
                why: "minecraft:shaders/core/lightmap.fsh:2: fragment shader: \
                      minecraft:shaders/include/other.glsl is not in the selected packs"
                    .to_owned(),
            },
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
            Err(in_lightmap(
                2,
                Refusal::NotText {
                    path: include.to_owned()
                }
            ))
        );
    }

    /// [`lightmap`]'s translation, with why it is refused.
    fn lightmap_translation(own: &[(&str, &str)]) -> Option<Result<Translated, Refusal>> {
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
            in_lightmap(
                2,
                Refusal::IncludeMissing {
                    id: "minecraft:shaders/core/tint.glsl".into()
                }
            )
        );
    }

    /// An error in a file a pack's include includes names that file and its line, through the
    /// loader's own reading of the packs.
    #[test]
    fn an_error_in_a_nested_include_names_its_file_and_line() {
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let tint = "assets/minecraft/shaders/core/tint.glsl";
        let deep = "assets/minecraft/shaders/core/inner/deep.glsl";
        let tint_glsl = "#moj_import \"inner/deep.glsl\"\n\
            vec4 tint(float f) { return vec4(f, 0.5 * f, 0.0, 1.0) * deep(); }\n";
        let deep_glsl = "// deep\nfloat deep() {\n    return not_declared;\n}\n";
        let refused =
            lightmap_translation(&[(fsh, IMPORTING_FSH), (tint, tint_glsl), (deep, deep_glsl)])
                .expect("the pack replaces a stage")
                .expect_err("the include has an error");
        let Refusal::Translation(errors) = refused else {
            panic!("not the translator's: {refused:?}");
        };
        let located: Vec<(&str, u32)> = errors.iter().map(|e| (e.file.as_str(), e.line)).collect();
        assert_eq!(located, [("minecraft:shaders/core/inner/deep.glsl", 3)]);
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
            in_lightmap(
                2,
                Refusal::IncludeMissing {
                    id: "minecraft:shaders/core/tint.glsl".into()
                }
            )
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
            in_lightmap(2, Refusal::OverBudget { path: tint.into() })
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
        assert_eq!(
            lightmap(&[(fsh, &big_stage), (tint, TINT_GLSL)]).unwrap_err(),
            RequiredShaderRefused {
                pipeline: "minecraft:pipeline/lightmap",
                why: Refusal::OverBudget { path: fsh.into() }.to_string()
            }
        );
    }

    /// A pack's `core/lightmap` is loaded with vanilla's `core/screenquad` (a stage of another
    /// name), and the reverse; none when no pack replaces either; the load fails when it reads
    /// an extra block whose layout the lightmap pass cannot bind.
    #[test]
    fn a_pack_replacing_either_lightmap_stage_is_loaded() {
        let red = "#version 330\nlayout(std140) uniform LightmapInfo {\n    float SkyFactor;\n} info;\n\
                   layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
                   void main() {\n    fragColor = vec4(info.SkyFactor, 0.0, 0.0, 1.0);\n}\n";
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let vsh = "assets/minecraft/shaders/core/screenquad.vsh";
        let program = lightmap(&[(fsh, red)])
            .unwrap()
            .expect("the pack's lightmap");
        assert_eq!(program.other, ["LightmapInfo"]);
        let quad = "#version 330\nlayout(location = 0) out vec2 texCoord;\n\
                    void main() {\n    texCoord = vec2(0.5);\n    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n}\n";
        assert!(lightmap(&[(vsh, quad)]).unwrap().is_some());
        assert!(
            matches!(lightmap(&[]), Ok(None)),
            "no pack replaces either stage"
        );
        let other = red
            .replace("LightmapInfo", "Elsewhere")
            .replace("float SkyFactor", "vec4 SkyFactor")
            .replace("info.SkyFactor", "info.SkyFactor.x");
        assert_eq!(
            lightmap(&[(fsh, &other)]).unwrap_err().pipeline,
            "minecraft:pipeline/lightmap"
        );
    }

    #[test]
    fn text_splits_from_the_rest() {
        let vertex = |text| HudVertex {
            pos: [0.0; 2],
            uv: [0.0; 2],
            color: 0,
            depth: 0.0,
            glint_uv: [0.0; 2],
            light: 0,
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

#[cfg(test)]
#[path = "pack_text_depth_tests.rs"]
mod depth_tests;

#[cfg(test)]
#[path = "pack_text_required_tests.rs"]
mod required_tests;
