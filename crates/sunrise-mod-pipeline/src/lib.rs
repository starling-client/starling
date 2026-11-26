//! Mod render pipelines' GLSL core shaders, translated and checked without a GPU on top of
//! [`sunrise_glsl`].
//!
//! A mod registers a `RenderPipeline` naming a core shader program (`example:core/glow` ->
//! `assets/example/shaders/core/glow.vsh` and `glow.fsh`), with an optional separate vertex
//! shader id (such as `minecraft:core/position_tex_color`), optional preprocessor defines, and
//! an optional sprite texture.
//!
//! Both the game's resource load (`sunrise-client-render`) and `cargo sunrise-mod check` run
//! through [`translate`], using the same vanilla-shaped [`BLOCKS`], [`TEXTURED_SAMPLERS`],
//! [`SPRITE_INCLUDE`], vertex [`Glue`](sunrise_glsl::Glue), and crash-containment bounds
//! ([`guard`]).
//!
//! Colour follows vanilla's gamma space: a mod program writes what vanilla core shaders write
//! (`texture * vertexColor * ColorModulator`). On frames storing vanilla's encoding, outputs are
//! stored as written; on frames holding linear light, [`linear_fragment_finish`] decodes the
//! fragment output after `main()` (sanitized against negatives/NaN, alpha clamped as coverage).

pub mod check;
pub mod guard;
pub mod stubs;

use crate::check::{Problem, problem, translate_backends};
use naga::ShaderStage;
use naga::front::glsl::InterfaceLimits;
use std::sync::LazyLock;
use sunrise_glsl::{Expanded, Glue, Includes, Inputs, ResourceKind, Stages, Translated};

/// The folder under `assets/<namespace>/` holding core shader programs (`.vsh` and `.fsh`).
pub const CORE_FOLDER: &str = "shaders/core";

/// The folder under `assets/<namespace>/` holding `<ns:path>` shader includes.
pub const INCLUDE_FOLDER: &str = "shaders/include/";

/// Entry point name of translated GLSL stages.
pub const ENTRY: &str = "main";

/// Backend define always provided to core shaders on a 0..1 depth backend (matching vanilla's
/// `GlslCompiler.createBaseShaderOptions` and `core_shader.rs`).
pub const DEPTH_ZERO_TO_ONE: &str = "RENDERPEARL_DEPTH_IS_ZERO_TO_ONE";

/// Resource id of the built-in sprite helper include (`#include <sunrise:pipeline_sprite.glsl>`).
pub const SPRITE_INCLUDE: &str = "sunrise:shaders/include/pipeline_sprite.glsl";

/// Set-0 binding of `SpriteRect` when a pipeline declares `PipelineTexture::Sprite`.
/// Placed above vanilla's `Sampler0..=2` slots (`4..=9`) so future samplers never collide.
pub const SPRITE_RECT_BINDING: u32 = 10;

/// Uniform blocks bound for an untextured mod pipeline in set 0.
pub const BLOCKS: [(&str, u32); 4] = [
    ("Globals", 0),
    ("Projection", 1),
    ("DynamicTransforms", 2),
    ("Fog", 3),
];

/// Uniform blocks bound for a sprite-textured mod pipeline in set 0.
pub const BLOCKS_WITH_SPRITE: [(&str, u32); 5] = [
    ("Globals", 0),
    ("Projection", 1),
    ("DynamicTransforms", 2),
    ("Fog", 3),
    ("SpriteRect", SPRITE_RECT_BINDING),
];

/// Samplers bound for a textured mod pipeline in set 0 (`Sampler0` at binding 4, sampler at 5).
pub const TEXTURED_SAMPLERS: [(&str, u32); 1] = [("Sampler0", 4)];

/// Samplers the OIT stages bind (`DepthBoundsSampler`, `Coeff0`, `Coeff1`), in the order
/// `Oit::cloud_samples` supplies their texture views.
pub const OIT_STAGE_SAMPLERS: [&str; 3] = ["DepthBoundsSampler", "Coeff0", "Coeff1"];

/// Preprocessor defines and extra samplers for one OIT sub-pass (`DepthBounds`, `Transmittance`,
/// `Accumulate`), matching vanilla 26.3's `RenderPipelines.OIT_*_SNIPPET`.
#[derive(Clone, Copy, Debug)]
pub struct OitStageSpec {
    /// Human-readable stage name for diagnostics.
    pub name: &'static str,
    /// Preprocessor defines appended after the pipeline's own defines.
    pub defines: &'static [&'static str],
    /// Extra sampler names allowed via `sunrise_glsl`'s `provided` callback for this stage.
    pub samplers: &'static [&'static str],
}

/// The three OIT stages in execution order (`DepthBounds`, `Transmittance`, `Accumulate`).
pub const OIT_STAGES: [OitStageSpec; 3] = [
    OitStageSpec {
        name: "depth_bounds",
        defines: &[
            "OIT",
            "OIT_WAVELET_RANK 2",
            "OIT_COEFF_COUNT 8",
            "OIT_COEFF_ATTACHMENT_COUNT 2",
            "OIT_ALPHA_ONLY",
            "OIT_DEPTH_BOUNDS",
        ],
        samplers: &[],
    },
    OitStageSpec {
        name: "transmittance",
        defines: &[
            "OIT",
            "OIT_WAVELET_RANK 2",
            "OIT_COEFF_COUNT 8",
            "OIT_COEFF_ATTACHMENT_COUNT 2",
            "OIT_ALPHA_ONLY",
            "OIT_TRANSMITTANCE",
        ],
        samplers: &["DepthBoundsSampler"],
    },
    OitStageSpec {
        name: "accumulate",
        defines: &[
            "OIT",
            "OIT_WAVELET_RANK 2",
            "OIT_COEFF_COUNT 8",
            "OIT_COEFF_ATTACHMENT_COUNT 2",
            "OIT_ACCUMULATE",
        ],
        samplers: &OIT_STAGE_SAMPLERS,
    },
];

/// Conservative stage interface limits used when checking shaders without a GPU device.
pub const DEVICELESS_INTERFACE_LIMITS: InterfaceLimits = InterfaceLimits {
    inter_stage_variables: 32,
    vertex_attributes: 32,
};

/// Built-in GLSL include `<sunrise:pipeline_sprite.glsl>` for `PipelineTexture::Sprite` pipelines:
/// provides `SpriteRect`, `Sampler0`, and `vec4 sampleSprite(vec2 uv)` matching [`repeat_texel`]
/// in vanilla's gamma space (nearest integer texel inside the sprite's atlas rect at mip 0,
/// repeating past 0..1 without bleeding neighbouring sprites).
pub const PIPELINE_SPRITE_GLSL: &str = "\
#ifndef SUNRISE_PIPELINE_SPRITE_GLSL
#define SUNRISE_PIPELINE_SPRITE_GLSL
layout(std140) uniform SpriteRect {
    vec4 SunriseSpriteRect;
};
uniform sampler2D Sampler0;
vec4 sampleSprite(vec2 uv) {
    vec2 size = vec2(textureSize(Sampler0, 0));
    vec2 origin = round(SunriseSpriteRect.xy * size);
    vec2 extent = max(round(SunriseSpriteRect.zw * size) - origin, vec2(1.0));
    vec2 local = clamp(floor((uv - floor(uv)) * extent), vec2(0.0), extent - vec2(1.0));
    return texelFetch(Sampler0, ivec2(origin + local), 0);
}
#endif
";

/// Post-`main()` fragment finish for frames holding linear light: sanitizes negative and NaN
/// RGB channels to 0 before applying vanilla's 2.2 gamma-to-linear power, and clamps alpha
/// coverage to `[0, 1]` (NaN covers nothing).
pub fn linear_fragment_finish(out_name: &str) -> String {
    format!(
        "vec3 hz_rgb = mix(vec3(0.0), {out_name}.rgb, greaterThan({out_name}.rgb, vec3(0.0)));\n\
         float hz_a = clamp({out_name}.a > 0.0 ? {out_name}.a : 0.0, 0.0, 1.0);\n\
         {out_name} = vec4(pow(hz_rgb, vec3(2.2)), hz_a);"
    )
}

/// Returns the GLSL output variable name bound to fragment `location = 0` in `module`, if any.
fn fragment_loc0_output_name(module: &naga::Module) -> Option<&str> {
    let ep = module
        .entry_points
        .iter()
        .find(|e| e.stage == naga::ShaderStage::Fragment)?;
    let res = ep.function.result.as_ref()?;
    if let Some(naga::Binding::Location { location: 0, .. }) = res.binding {
        // Trace the entry point's returned expression back to the GLSL output GlobalVariable.
        let ret_expr = ep.function.body.iter().rev().find_map(|stmt| match stmt {
            naga::Statement::Return { value: Some(expr) } => Some(*expr),
            _ => None,
        })?;
        let ptr = match ep.function.expressions[ret_expr] {
            naga::Expression::Load { pointer } => pointer,
            _ => ret_expr,
        };
        if let naga::Expression::GlobalVariable(gv) = ep.function.expressions[ptr] {
            return module.global_variables[gv].name.as_deref();
        }
        return None;
    }
    if let naga::TypeInner::Struct { ref members, .. } = module.types[res.ty].inner {
        for m in members {
            if let Some(naga::Binding::Location { location: 0, .. }) = m.binding {
                return m.name.as_deref();
            }
        }
    }
    None
}

/// `sampleSprite`'s texel arithmetic, for testing without a GPU: the integer texel of an atlas
/// `size` texels wide and high that `uv` picks inside the sprite at `rect` (`[u0, v0, u1, v1]`).
/// The fraction of `uv` (negative values wrap) spans the rect once; the texel is always one of
/// the rect's own, its far edge excluded.
pub fn repeat_texel(uv: [f32; 2], rect: [f32; 4], size: [u32; 2]) -> [u32; 2] {
    std::array::from_fn(|axis| {
        let size = size[axis] as f32;
        let origin = (rect[axis] * size).round();
        let extent = ((rect[axis + 2] * size).round() - origin).max(1.0);
        let f = uv[axis] - uv[axis].floor();
        let local = (f * extent).floor().clamp(0.0, extent - 1.0);
        (origin + local) as u32
    })
}

/// The vertex and fragment GLSL source texts and paths of one mod render pipeline program.
pub struct ProgramSources<'a> {
    /// Asset path of the vertex shader for diagnostics (e.g. `assets/mod/shaders/core/a.vsh`).
    pub vertex_file: &'a str,
    /// Include base folder for quoted relative includes in the vertex shader
    /// (e.g. `mod:shaders/core/`).
    pub vertex_dir: &'a str,
    pub vertex: &'a str,
    /// Asset path of the fragment shader for diagnostics (e.g. `assets/mod/shaders/core/a.fsh`).
    pub fragment_file: &'a str,
    /// Include base folder for quoted relative includes in the fragment shader.
    pub fragment_dir: &'a str,
    pub fragment: &'a str,
}

/// A pipeline's default vertex format, kept for the program's lifetime so that
/// [`DEFAULT_VERTEX_INPUTS`] can borrow its attribute names.
static DEFAULT_VERTEX_FORMAT: LazyLock<mod_vocab::VertexFormat> =
    LazyLock::new(mod_vocab::VertexFormat::position_tex_color);

/// Default vertex inputs `(name, glsl_type)`: the attributes of
/// [`VertexFormat::position_tex_color`](mod_vocab::VertexFormat::position_tex_color) in
/// deterministic sequential location order (`0..3`), borrowed from that format.
pub static DEFAULT_VERTEX_INPUTS: LazyLock<Vec<(&'static str, &'static str)>> =
    LazyLock::new(|| {
        let format: &'static mod_vocab::VertexFormat = &DEFAULT_VERTEX_FORMAT;
        format
            .attributes
            .iter()
            .map(|attribute| (attribute.name.as_str(), attribute.format.glsl_type()))
            .collect()
    });

/// Bind-group set index for mod-declared `BindGroupLayout` entries (set 0 stays vanilla's shared
/// bindings from `BindGroupLayouts.java`).
pub const MOD_BIND_GROUP_SET: u32 = 1;

pub use mod_vocab::{MAX_PUSH_CONSTANT_SIZE, VANILLA_BIND_GROUP_LAYOUT_NAMES};

/// Resource kind of one mod-declared set-1 binding (`BindGroupLayout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModBindingKind {
    /// std140 uniform block (`binding = k`).
    UniformBuffer,
    /// Combined 2D image + sampler (`texture` at `k`, `sampler` at `k + 1`).
    CombinedImageSampler,
    /// Integer texel buffer (`isamplerBuffer`/`usamplerBuffer`, read-only storage at `k`).
    TexelBuffer,
    /// std430 storage buffer block (`buffer Name { ... }`, `binding = k`).
    StorageBuffer {
        /// Whether the block is `readonly`.
        read_only: bool,
    },
    /// `layout(<format>) uniform image2D` storage image (`binding = k`).
    StorageImage {
        /// What the declaration's `readonly`/`writeonly` qualifiers must say.
        access: sunrise_glsl::ImageAccess,
    },
}

/// One mod-declared set-1 binding passed into [`Options::mod_bindings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModBinding<'a> {
    pub name: &'a str,
    pub kind: ModBindingKind,
}

/// Deterministic set-1 binding indices assigned in declaration order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Set1Bindings<'a> {
    pub blocks: Vec<(&'a str, u32)>,
    pub samplers: Vec<(&'a str, u32)>,
    pub texel_buffers: Vec<(&'a str, u32)>,
    pub storage_buffers: Vec<(&'a str, u32, bool)>,
    pub storage_images: Vec<(&'a str, u32, sunrise_glsl::ImageAccess)>,
    pub push_constant_binding: Option<u32>,
    pub next_binding: u32,
}

/// Assign deterministic set-1 binding indices for `bindings` (in declaration order) followed by
/// an optional inline uniform slot for `push_constant_size > 0`.
pub fn assign_set1_bindings<'a>(
    bindings: &'a [ModBinding<'a>],
    push_constant_size: u32,
) -> Set1Bindings<'a> {
    let mut out = Set1Bindings::default();
    let mut next = 0u32;
    for b in bindings {
        match b.kind {
            ModBindingKind::UniformBuffer => {
                out.blocks.push((b.name, next));
                next += 1;
            }
            ModBindingKind::CombinedImageSampler => {
                out.samplers.push((b.name, next));
                next += 2;
            }
            ModBindingKind::TexelBuffer => {
                out.texel_buffers.push((b.name, next));
                next += 1;
            }
            ModBindingKind::StorageBuffer { read_only } => {
                out.storage_buffers.push((b.name, next, read_only));
                next += 1;
            }
            ModBindingKind::StorageImage { access } => {
                out.storage_images.push((b.name, next, access));
                next += 1;
            }
        }
    }
    if push_constant_size > 0 {
        out.push_constant_binding = Some(next);
        next += 1;
    }
    out.next_binding = next;
    out
}

/// The non-vanilla uniform blocks, `sampler2D`s, integer texel buffers, storage buffers and
/// storage images a mod shader pair declares, and the push-constant size its
/// `layout(push_constant)` block asks for ([`MAX_PUSH_CONSTANT_SIZE`], `0` without one), for
/// deviceless static checking (`cargo sunrise-mod check`), which has no `BindGroupLayout` to ask.
/// They are the declarations the translator binds, found by the same code
/// ([`sunrise_glsl::resources`]), each name once in the order the vertex stage then the fragment
/// stage first declares it. Reserved vanilla `BindGroupLayouts.java` names (plus `SpriteRect` when
/// `textured`) are never mod bindings, so undeclared vanilla names are still refused. The stages
/// are read as the translator reads them: with comments gone and includes inlined
/// ([`extract_program_bindings`]).
pub fn extract_mod_bindings(
    vertex: &str,
    fragment: &str,
    textured: bool,
) -> (Vec<(String, ModBindingKind)>, u32) {
    let mut out: Vec<(String, ModBindingKind)> = Vec::new();
    let mut push_constant_size = 0u32;
    let is_reserved = |name: &str| {
        VANILLA_BIND_GROUP_LAYOUT_NAMES.contains(&name) || (textured && name == "SpriteRect")
    };
    for resource in [vertex, fragment]
        .into_iter()
        .flat_map(sunrise_glsl::resources)
    {
        let kind = match resource.kind {
            ResourceKind::PushConstantBlock => {
                push_constant_size = MAX_PUSH_CONSTANT_SIZE;
                continue;
            }
            ResourceKind::UniformBlock => ModBindingKind::UniformBuffer,
            ResourceKind::Sampler2D => ModBindingKind::CombinedImageSampler,
            ResourceKind::TexelBuffer => ModBindingKind::TexelBuffer,
            ResourceKind::StorageBuffer { read_only } => {
                ModBindingKind::StorageBuffer { read_only }
            }
            ResourceKind::StorageImage { access } => ModBindingKind::StorageImage { access },
        };
        if !is_reserved(resource.name) && !out.iter().any(|(name, _)| name == resource.name) {
            out.push((resource.name.to_owned(), kind));
        }
    }
    (out, push_constant_size)
}

/// Compilation options for [`translate`].
#[derive(Clone, Copy)]
pub struct Options<'a> {
    /// Whether the pipeline declared `.textured(PipelineTexture::Sprite(...))`, exposing
    /// `Sampler0`, `SpriteRect`, and `#include <sunrise:pipeline_sprite.glsl>`.
    pub textured: bool,
    /// Pipeline defines (`NAME` or `NAME value`) in order; [`DEPTH_ZERO_TO_ONE`] is always
    /// prepended automatically.
    pub defines: &'a [&'a str],
    /// Declared vertex inputs `(name, glsl_type)` in deterministic sequential location order
    /// (`0..N`) across `(binding_index, attribute_index)`. Defaults to [`DEFAULT_VERTEX_INPUTS`].
    pub vertex_inputs: &'a [(&'a str, &'a str)],
    /// Mod-declared set-1 bindings (`BindGroupLayout`) in declaration order.
    pub mod_bindings: &'a [ModBinding<'a>],
    /// Declared push-constant block size in bytes (`0` = none, at most
    /// [`MAX_PUSH_CONSTANT_SIZE`] = 128 B).
    pub push_constant_size: u32,
    /// Whether the target frame stores linear light (attaching [`linear_fragment_finish`]).
    pub linear_light: bool,
    /// Also run SPIR-V, Metal and DirectX 12 backend translation checks (used by
    /// `cargo sunrise-mod check` and tests).
    pub check_backends: bool,
    /// Stage interface limits of the target device (or [`DEVICELESS_INTERFACE_LIMITS`]).
    pub interface: InterfaceLimits,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            textured: false,
            defines: &[],
            vertex_inputs: DEFAULT_VERTEX_INPUTS.as_slice(),
            mod_bindings: &[],
            push_constant_size: 0,
            linear_light: false,
            check_backends: false,
            interface: DEVICELESS_INTERFACE_LIMITS,
        }
    }
}

/// Result of [`translate`]: the validated naga program plus both stages' expanded GLSL text
/// (allowing a linear-light renderer to re-translate the fragment stage with
/// [`translate_linear_fragment`] without re-reading disk resources).
#[derive(Debug)]
pub struct BuiltProgram {
    pub translated: Translated,
    pub vertex_expanded: String,
    pub fragment_expanded: String,
}

/// Vertex & fragment `Glue` for [`VertexFormat::position_tex_color`](mod_vocab::VertexFormat::position_tex_color).
pub fn pipeline_glue<'a>(interface: InterfaceLimits, fragment_finish: Option<&'a str>) -> Glue<'a> {
    Glue {
        inputs: DEFAULT_VERTEX_INPUTS.as_slice(),
        prelude: "layout(location = 0) in vec3 hz_pos;\n\
                  layout(location = 1) in vec2 hz_uv;\n\
                  layout(location = 2) in vec4 hz_color;",
        setup: "Position = hz_pos; UV0 = hz_uv; Color = hz_color;",
        finish: "",
        vertex_id: "int(gl_VertexIndex)",
        fragment_finish,
        interface,
    }
}

/// What [`translate_inner`] and [`translate_linear_fragment`] both hand the translator for
/// `options`: the mod's set-1 bindings, and the glue snippets binding `options.vertex_inputs` to
/// deterministic sequential locations `0..N`.
struct Setup<'a> {
    set1: Set1Bindings<'a>,
    prelude: String,
    setup: String,
}

impl<'a> Setup<'a> {
    fn new(options: &Options<'a>) -> Self {
        use std::fmt::Write as _;
        let mut prelude = String::new();
        let mut setup = String::new();
        for (loc, (name, ty)) in options.vertex_inputs.iter().enumerate() {
            writeln!(prelude, "layout(location = {loc}) in {ty} hz_attr_{loc};").unwrap();
            write!(setup, "{name} = hz_attr_{loc}; ").unwrap();
        }
        Self {
            set1: assign_set1_bindings(options.mod_bindings, options.push_constant_size),
            prelude,
            setup,
        }
    }

    fn inputs<'b>(
        &'b self,
        options: &Options<'_>,
        defines: &'b [&'b str],
        includes: &'b Includes<'b>,
    ) -> Inputs<'b> {
        Inputs {
            blocks: if options.textured {
                &BLOCKS_WITH_SPRITE
            } else {
                &BLOCKS
            },
            samplers: if options.textured {
                &TEXTURED_SAMPLERS
            } else {
                &[]
            },
            extra_set: MOD_BIND_GROUP_SET,
            extra_blocks: &self.set1.blocks,
            extra_samplers: &self.set1.samplers,
            extra_texel_buffers: &self.set1.texel_buffers,
            extra_storage_buffers: &self.set1.storage_buffers,
            extra_storage_images: &self.set1.storage_images,
            push_constant_binding: self.set1.push_constant_binding,
            loose: &[],
            defines,
            includes,
        }
    }

    fn glue<'b>(&'b self, options: &Options<'b>, fragment_finish: Option<&'b str>) -> Glue<'b> {
        Glue {
            inputs: options.vertex_inputs,
            prelude: &self.prelude,
            setup: &self.setup,
            finish: "",
            vertex_id: "int(gl_VertexIndex)",
            fragment_finish,
            interface: options.interface,
        }
    }
}

/// Whether `ty` is a GLSL scalar/vector type producible by `VertexAttributeFormat::glsl_type`.
fn is_supported_vertex_glsl_type(ty: &str) -> bool {
    matches!(
        ty,
        "float"
            | "vec2"
            | "vec3"
            | "vec4"
            | "int"
            | "ivec2"
            | "ivec3"
            | "ivec4"
            | "uint"
            | "uvec2"
            | "uvec3"
            | "uvec4"
    )
}

/// The asset path of a file a located error names: an `#include`d file's id `ns:path` becomes
/// `assets/ns/path`, the spelling of a stage file, which is already an asset path. A [`Problem`]
/// names every file by its asset path, the path the mod's pack holds it at.
fn asset_path(file: &str) -> String {
    match file.split_once(':') {
        Some((namespace, path)) => format!("assets/{namespace}/{path}"),
        None => file.to_owned(),
    }
}

/// An include expansion refusal in `stage`, at the directive that caused it. The preprocessor
/// names the asset file and line but not the column.
fn import_problem(stage: ShaderStage, error: sunrise_glsl::ImportError) -> Problem {
    Problem {
        file: asset_path(&error.file),
        line: error.line as usize,
        column: 0,
        stage: Some(stage),
        message: error.refusal.to_string(),
    }
}

/// A refusal in `stage` at byte `at` of `code`, mapped to the authored file, line and
/// column by [`Expanded::error_at`]. An included declaration keeps its own file; `None`
/// names the stage file without a line or column.
fn refusal_at(stage: ShaderStage, code: &Expanded, at: Option<usize>, message: String) -> Problem {
    let error = code.error_at(at, message);
    Problem {
        file: asset_path(&error.file),
        line: error.line as usize,
        column: error.column as usize,
        stage: Some(stage),
        message: error.message,
    }
}

/// The vertex inputs `(name, glsl_type)` a vertex stage declares, from its expanded text
/// ([`sunrise_glsl::stage_inputs`]: includes inlined, comments gone). Refuses an unsupported
/// type, a name declared twice and more than 16 inputs, locating the refusal in the file and at
/// the line the declaration came from ([`refusal_at`]).
fn declared_vertex_inputs(vertex: &Expanded) -> Result<Vec<(String, String)>, Problem> {
    let decls = sunrise_glsl::stage_inputs(vertex.text());
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(decls.len());
    for (offset, name, ty) in decls {
        let refuse =
            |message: String| refusal_at(ShaderStage::Vertex, vertex, Some(offset), message);
        if !is_supported_vertex_glsl_type(ty) {
            return Err(refuse(format!(
                "unsupported vertex input type `{ty}` for `{name}`: expected float/vecN/int/ivecN/uint/uvecN"
            )));
        }
        if !seen.insert(name) {
            return Err(refuse(format!("duplicate vertex input `{name}`")));
        }
        out.push((name.to_owned(), ty.to_owned()));
        if out.len() > 16 {
            return Err(refuse(
                "vertex shader declares more than 16 vertex inputs".to_owned(),
            ));
        }
    }
    Ok(out)
}

/// Reads an include for a program: the sprite helpers ([`SPRITE_INCLUDE`]) are pasted when
/// `sprite` (the pipeline is textured, or the reader is finding out whether it is) and refused
/// otherwise, every other file comes from `include`.
fn read_include(
    id: &str,
    sprite: bool,
    include: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    if id != SPRITE_INCLUDE {
        return include(id);
    }
    if sprite {
        return Ok(Some(PIPELINE_SPRITE_GLSL.to_owned()));
    }
    Err(format!(
        "{SPRITE_INCLUDE} requires a sprite texture (RenderPipeline::textured)"
    ))
}

/// The [`Includes`] of a program's stages, reading files through `read` ([`read_include`]).
fn program_includes<'a>(read: &'a dyn Fn(&str) -> Result<Option<String>, String>) -> Includes<'a> {
    Includes {
        read,
        namespace: "minecraft",
        folder: INCLUDE_FOLDER,
        once: &["moj_import"],
    }
}

/// Refuse a top-level vertex `in` ([`sunrise_glsl::stage_inputs`]) whose name or GLSL type
/// doesn't match the supplied `vertex_inputs`. Both registration and the CLI check use this
/// check; the refusal names the declaration's file and line ([`refusal_at`]).
fn validate_vertex_inputs(
    vertex: &Expanded,
    vertex_inputs: &[(&str, &str)],
) -> Result<(), Problem> {
    for (offset, name, ty) in sunrise_glsl::stage_inputs(vertex.text()) {
        match vertex_inputs.iter().find(|(n, _)| *n == name) {
            None => {
                return Err(refusal_at(
                    ShaderStage::Vertex,
                    vertex,
                    Some(offset),
                    format!("vertex input `{name}` has no matching vertex attribute"),
                ));
            }
            Some((_, expected)) if *expected != ty => {
                return Err(refusal_at(
                    ShaderStage::Vertex,
                    vertex,
                    Some(offset),
                    format!(
                        "vertex input `{name}` declared as `{ty}` in shader, expected `{expected}` from VertexFormat"
                    ),
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// What a program's text says of its pipeline, for deviceless static checking (`cargo sunrise-mod
/// check`), which has no `RenderPipeline` to ask: [`extract_program_bindings`].
#[derive(Debug, PartialEq, Eq)]
pub struct ProgramBindings {
    /// Whether the program samples the pipeline's sprite texture: it names `Sampler0`,
    /// `SpriteRect` or `sampleSprite`, or includes [`SPRITE_INCLUDE`].
    pub textured: bool,
    /// The mod's own bindings ([`extract_mod_bindings`]).
    pub bindings: Vec<(String, ModBindingKind)>,
    /// The vertex stage's `in`s `(name, glsl_type)` in declaration order, the ones an included
    /// file declares too, checked as the translator checks them: a supported type, each name
    /// once, at most 16.
    pub vertex_inputs: Vec<(String, String)>,
    /// What its `layout(push_constant)` block asks for, `0` without one.
    pub push_constant_size: u32,
    /// The id (`ns:path`) of every file either stage's expansion pasted, each once, the vertex
    /// stage's first.
    pub includes: Vec<String>,
}

/// What a program's two stages declare, read as [`translate`] reads them: `#include`s inlined
/// under `defines` and comments removed. A uniform block, sampler or buffer an included file
/// declares is the program's too (#5905), and one named only in a comment is not, nor does a
/// sprite name in a comment make the program textured. A stage whose includes cannot be expanded
/// refuses the program at the directive that failed, as [`translate`] does. An unsupported,
/// duplicate or 17th vertex input is refused where it is declared. Both refusals name the
/// file (including an included file), line and responsible stage.
/// [`SPRITE_INCLUDE`] is always pasted: including it is itself asking for the sprite texture.
pub fn extract_program_bindings(
    sources: &ProgramSources<'_>,
    defines: &[&str],
    include: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<ProgramBindings, Problem> {
    let read = |id: &str| read_include(id, true, include);
    let includes = program_includes(&read);
    let defines = effective_defines(defines);
    let expand = |stage, text: &str, file: &str, dir: &str| {
        sunrise_glsl::expand_includes(text, file, dir, &includes, Some(&defines))
            .map_err(|e| import_problem(stage, e))
    };
    let vertex = expand(
        ShaderStage::Vertex,
        sources.vertex,
        sources.vertex_file,
        sources.vertex_dir,
    )?;
    let fragment = expand(
        ShaderStage::Fragment,
        sources.fragment,
        sources.fragment_file,
        sources.fragment_dir,
    )?;
    let textured = [&vertex, &fragment].iter().any(|stage| {
        stage
            .text()
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|word| matches!(word, "Sampler0" | "SpriteRect" | "sampleSprite"))
    });
    let (bindings, push_constant_size) =
        extract_mod_bindings(vertex.text(), fragment.text(), textured);
    let mut includes = vertex.includes().to_vec();
    for id in fragment.includes() {
        if !includes.contains(id) {
            includes.push(id.clone());
        }
    }
    Ok(ProgramBindings {
        textured,
        bindings,
        vertex_inputs: declared_vertex_inputs(&vertex)?,
        push_constant_size,
        includes,
    })
}

/// Prepend [`DEPTH_ZERO_TO_ONE`] to caller-supplied defines.
fn effective_defines<'a>(defines: &[&'a str]) -> Vec<&'a str> {
    std::iter::once(DEPTH_ZERO_TO_ONE)
        .chain(defines.iter().copied())
        .collect()
}

/// Translate and validate one mod render pipeline GLSL program (`.vsh` + `.fsh`) on a guarded
/// 64 MiB stack thread with crash/memory containment bounds ([`guard`]). Refusals name the
/// responsible stage when known.
pub fn translate(
    sources: &ProgramSources<'_>,
    options: &Options<'_>,
    include: &(dyn Fn(&str) -> Result<Option<String>, String> + Sync),
) -> Result<BuiltProgram, Vec<Problem>> {
    let defines = effective_defines(options.defines);
    guard::guarded(|| translate_inner(sources, options, &defines, &[], include))
        .unwrap_or_else(|reason| Err(vec![problem(sources.fragment_file, None, reason)]))
}

/// The GLSL source text and path of one mod compute shader (`.csh`).
pub struct ComputeSources<'a> {
    /// Asset path for diagnostics (e.g. `assets/mod/shaders/compute/a.csh`).
    pub file: &'a str,
    /// Include base folder for quoted relative includes (e.g. `mod:shaders/compute/`).
    pub dir: &'a str,
    pub source: &'a str,
}

/// Compilation options for [`translate_compute`].
#[derive(Clone, Copy)]
pub struct ComputeOptions<'a> {
    /// Preprocessor defines (`NAME` or `NAME value`) in order.
    pub defines: &'a [&'a str],
    /// Mod-declared bindings (`BindGroupLayout`) in declaration order, bound in group
    /// [`COMPUTE_BIND_GROUP`].
    pub mod_bindings: &'a [ModBinding<'a>],
    /// Also run SPIR-V, Metal and DirectX 12 backend translation checks.
    pub check_backends: bool,
    /// Stage interface limits of the target device (or [`DEVICELESS_INTERFACE_LIMITS`]).
    pub interface: InterfaceLimits,
}

/// The one bind group of a compute pipeline: the mod's own `BindGroupLayout`. A compute shader
/// reads no vanilla bindings, so there is no shared set 0 to sit beside.
pub const COMPUTE_BIND_GROUP: u32 = 0;

/// The folder under `assets/<namespace>/` holding compute shaders (`.csh`).
pub const COMPUTE_FOLDER: &str = "shaders/compute";

/// Translate and validate one mod compute shader (`.csh`) on a guarded 64 MiB stack thread with
/// the same crash/memory containment bounds as [`translate`]. Refusals name the compute stage;
/// an include expansion refusal names the file and line of the failing directive.
pub fn translate_compute(
    sources: &ComputeSources<'_>,
    options: &ComputeOptions<'_>,
    include: &(dyn Fn(&str) -> Result<Option<String>, String> + Sync),
) -> Result<sunrise_glsl::ComputeTranslated, Vec<Problem>> {
    guard::guarded(|| translate_compute_inner(sources, options, include)).unwrap_or_else(|reason| {
        Err(vec![problem(
            sources.file,
            Some(ShaderStage::Compute),
            reason,
        )])
    })
}

/// Check expanded source before translating the compute shader, keeping each refusal at its
/// source position when known.
fn translate_compute_inner(
    sources: &ComputeSources<'_>,
    options: &ComputeOptions<'_>,
    include: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<sunrise_glsl::ComputeTranslated, Vec<Problem>> {
    let stage = ShaderStage::Compute;
    let includes = program_includes(include);
    let expanded = sunrise_glsl::expand_includes(
        sources.source,
        sources.file,
        sources.dir,
        &includes,
        Some(options.defines),
    )
    .map_err(|e| vec![import_problem(stage, e)])?;
    if let Some((at, msg)) = guard::refusal(expanded.text()) {
        return Err(vec![refusal_at(stage, &expanded, at, msg)]);
    }
    let set = assign_set1_bindings(options.mod_bindings, 0);
    let inputs = Inputs {
        blocks: &[],
        samplers: &[],
        extra_set: COMPUTE_BIND_GROUP,
        extra_blocks: &set.blocks,
        extra_samplers: &set.samplers,
        extra_texel_buffers: &set.texel_buffers,
        extra_storage_buffers: &set.storage_buffers,
        extra_storage_images: &set.storage_images,
        push_constant_binding: None,
        loose: &[],
        defines: options.defines,
        includes: &includes,
    };
    let translated = sunrise_glsl::translate_compute(
        sources.source,
        sources.file,
        sources.dir,
        &inputs,
        options.interface,
    )
    .map_err(|errors| map_glsl_errors(errors, false))?;
    if options.check_backends {
        let errors: Vec<Problem> = translate_backends(&translated.module)
            .into_iter()
            .map(|err| problem(sources.file, Some(stage), err))
            .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
    }
    Ok(translated)
}

/// Translate and validate all three OIT variants (`DepthBounds`, `Transmittance`, `Accumulate`)
/// of an `.order_independent()` mod render pipeline program on a guarded 64 MiB stack thread.
/// When `additive` is true (`PipelineBlend::Additive`), `OIT_ADDITIVE` is also defined so the
/// shader adds light with no coverage.
pub fn translate_oit(
    sources: &ProgramSources<'_>,
    options: &Options<'_>,
    additive: bool,
    include: &(dyn Fn(&str) -> Result<Option<String>, String> + Sync),
) -> Result<[BuiltProgram; 3], Vec<Problem>> {
    let oit_options = Options {
        linear_light: false,
        ..*options
    };
    let mut out = Vec::with_capacity(3);
    for spec in &OIT_STAGES {
        let mut defines = effective_defines(options.defines);
        defines.extend_from_slice(spec.defines);
        if additive {
            defines.push("OIT_ADDITIVE");
        }
        let built = guard::guarded(|| {
            translate_inner(sources, &oit_options, &defines, spec.samplers, include)
        })
        .unwrap_or_else(|reason| Err(vec![problem(sources.fragment_file, None, reason)]))
        .map_err(|mut problems| {
            for p in &mut problems {
                p.message = format!("OIT {}: {}", spec.name, p.message);
            }
            problems
        })?;
        out.push(built);
    }
    Ok(out.try_into().expect("three OIT stages"))
}

/// Expand both stages ([`Expanded`]), refuse what the guard and the vertex inputs refuse, and
/// translate the program, once more with the linear-light finish when `options` ask for it.
/// Include expansion refusals name the stage, file and line of the failing directive. The
/// translator takes the authored sources so each stage keeps its own file, lines and include
/// folder; the built program also carries the expanded text.
fn translate_inner(
    sources: &ProgramSources<'_>,
    options: &Options<'_>,
    defines: &[&str],
    extra_samplers: &[&str],
    include: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<BuiltProgram, Vec<Problem>> {
    let read = |id: &str| read_include(id, options.textured, include);
    let includes = program_includes(&read);

    let vertex = sunrise_glsl::expand_includes(
        sources.vertex,
        sources.vertex_file,
        sources.vertex_dir,
        &includes,
        Some(defines),
    )
    .map_err(|e| vec![import_problem(ShaderStage::Vertex, e)])?;
    if let Some((at, msg)) = guard::refusal(vertex.text()) {
        return Err(vec![refusal_at(ShaderStage::Vertex, &vertex, at, msg)]);
    }
    validate_vertex_inputs(&vertex, options.vertex_inputs).map_err(|prob| vec![prob])?;

    let fragment = sunrise_glsl::expand_includes(
        sources.fragment,
        sources.fragment_file,
        sources.fragment_dir,
        &includes,
        Some(defines),
    )
    .map_err(|e| vec![import_problem(ShaderStage::Fragment, e)])?;
    if let Some((at, msg)) = guard::refusal(fragment.text()) {
        return Err(vec![refusal_at(ShaderStage::Fragment, &fragment, at, msg)]);
    }

    let stages = Stages {
        vertex_dir: sources.vertex_dir,
        fragment_dir: sources.fragment_dir,
        vertex: sources.vertex,
        fragment: sources.fragment,
        vertex_file: sources.vertex_file,
        fragment_file: sources.fragment_file,
    };

    if options.push_constant_size > MAX_PUSH_CONSTANT_SIZE {
        return Err(vec![problem(
            sources.fragment_file,
            None,
            format!(
                "push_constant_size {} exceeds maximum {MAX_PUSH_CONSTANT_SIZE} bytes",
                options.push_constant_size
            ),
        )]);
    }
    let setup = Setup::new(options);
    let set1 = &setup.set1;
    let inputs = setup.inputs(options, defines, &includes);
    let glue = setup.glue(options, None);

    let provided = |name: &str| extra_samplers.contains(&name);
    let to_problems = |errs| map_glsl_errors(errs, !options.textured);
    let mut translated =
        sunrise_glsl::translate(&stages, &inputs, &glue, &provided).map_err(to_problems)?;
    if let Some(pc_binding) = set1.push_constant_binding
        && let Some(actual_size) = translated.uniform_block_size(MOD_BIND_GROUP_SET, pc_binding)
        && actual_size > options.push_constant_size
    {
        return Err(vec![problem(
            sources.fragment_file,
            None,
            format!(
                "push_constant block size {actual_size} bytes exceeds declared push_constant_size {} bytes",
                options.push_constant_size
            ),
        )]);
    }

    if options.linear_light
        && let Some(out_name) = fragment_loc0_output_name(&translated.fragment)
    {
        let finish = linear_fragment_finish(out_name);
        let linear_glue = setup.glue(options, Some(&finish));
        translated = sunrise_glsl::translate(&stages, &inputs, &linear_glue, &provided)
            .map_err(to_problems)?;
    }

    if options.check_backends {
        let mut backend_errors = Vec::new();
        for (stage, file, module) in [
            (ShaderStage::Vertex, sources.vertex_file, &translated.vertex),
            (
                ShaderStage::Fragment,
                sources.fragment_file,
                &translated.fragment,
            ),
        ] {
            backend_errors.extend(
                translate_backends(module)
                    .into_iter()
                    .map(|err| problem(file, Some(stage), err)),
            );
        }
        if !backend_errors.is_empty() {
            return Err(backend_errors);
        }
    }

    Ok(BuiltProgram {
        translated,
        vertex_expanded: vertex.text().to_owned(),
        fragment_expanded: fragment.text().to_owned(),
    })
}

/// Re-translate an already-expanded program with [`linear_fragment_finish`] enabled, returning
/// the linear-light fragment [`naga::Module`]. No include folders are needed for these expanded
/// stages. Used by the GPU renderer only on linear-light frames so standard gamma loads avoid
/// translating twice.
pub fn translate_linear_fragment(
    vertex_expanded: &str,
    fragment_expanded: &str,
    options: &Options<'_>,
) -> Result<naga::Module, String> {
    guard::guarded(|| {
        let defines = effective_defines(options.defines);
        // The text is expanded already: nothing is left to include.
        let noop = |_: &str| Ok(None);
        let read = |id: &str| read_include(id, options.textured, &noop);
        let includes = program_includes(&read);
        let setup = Setup::new(options);
        let inputs = setup.inputs(options, &defines, &includes);
        // The program is one `translate` built, which provides no extra samplers: only
        // `translate_oit` does, and the renderer never re-translates an OIT program.
        let provided = |_: &str| false;
        let stages = Stages {
            vertex_dir: "",
            fragment_dir: "",
            vertex: vertex_expanded,
            fragment: fragment_expanded,
            vertex_file: "",
            fragment_file: "",
        };
        let glue = setup.glue(options, None);
        let base = sunrise_glsl::translate(&stages, &inputs, &glue, &provided)
            .map_err(|errs| sunrise_glsl::describe(&errs))?;
        let Some(out_name) = fragment_loc0_output_name(&base.fragment) else {
            return Ok(base.fragment);
        };
        let finish = linear_fragment_finish(out_name);
        let linear_glue = setup.glue(options, Some(&finish));
        sunrise_glsl::translate(&stages, &inputs, &linear_glue, &provided)
            .map_err(|errs| sunrise_glsl::describe(&errs))
            .map(|t| t.fragment)
    })
    .unwrap_or_else(Err)
}

/// The translator's `errors` as problems with their authored file (the stage's file or an
/// `#include`d one, by asset path), line, column and stage. On an untextured render pipeline
/// (`hint_textured`), an error refusing `Sampler0` or `SpriteRect` as unprovided adds the hint
/// to declare the pipeline textured.
fn map_glsl_errors(errors: Vec<sunrise_glsl::Error>, hint_textured: bool) -> Vec<Problem> {
    errors
        .into_iter()
        .map(|mut err| {
            if hint_textured && matches!(err.unprovided.as_deref(), Some("Sampler0" | "SpriteRect"))
            {
                err.message += " (requires RenderPipeline::textured)";
            }
            Problem {
                file: asset_path(&err.file),
                line: err.line as usize,
                column: err.column as usize,
                stage: err.stage,
                message: err.message,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VSH: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
";

    const FSH: &str = "\
#version 330
#include <minecraft:globals.glsl>
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:fog.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    vec4 color = vertexColor * ColorModulator;
#ifdef PULSE
    color.rgb *= 1.0 + sin(GameTime * 24000.0 * 0.2);
#endif
    fragColor = vec4(color.rgb * (1.0 - FogEnvironmentalStart * 0.0), color.a);
}
";

    fn stub_include(id: &str) -> Result<Option<String>, String> {
        Ok(stubs::read(id).map(str::to_owned))
    }

    fn sources<'a>(v: &'a str, f: &'a str) -> ProgramSources<'a> {
        ProgramSources {
            vertex_file: "assets/example/shaders/core/glow.vsh",
            vertex_dir: "example:shaders/core/",
            vertex: v,
            fragment_file: "assets/example/shaders/core/glow.fsh",
            fragment_dir: "example:shaders/core/",
            fragment: f,
        }
    }

    /// Translation reports the authored column, not column 0 or a position in rewritten GLSL.
    #[test]
    fn a_translator_problem_keeps_the_authored_column() {
        let fragment =
            "#version 330\nout vec4 fragColor;\nvoid main() {\n    fragColor = missing_color;\n}\n";
        let problems = translate(&sources(VSH, fragment), &Options::default(), &stub_include)
            .expect_err("the fragment reads an undeclared name");
        assert_eq!(problems.len(), 1, "{problems:?}");
        let problem = &problems[0];
        assert_eq!(
            (problem.file.as_str(), problem.line, problem.column),
            ("assets/example/shaders/core/glow.fsh", 4, 17)
        );
    }

    #[test]
    fn vanilla_style_program_translates_and_passes_all_backends() {
        let opts = Options {
            defines: &["PULSE"],
            check_backends: true,
            ..Default::default()
        };
        let built = translate(&sources(VSH, FSH), &opts, &stub_include).unwrap();
        assert_eq!(built.translated.vertex.entry_points.len(), 1);
        assert_eq!(built.translated.fragment.entry_points.len(), 1);
        assert!(!built.translated.stages_reading(0).is_empty()); // Globals
        assert!(!built.translated.stages_reading(1).is_empty()); // Projection
        assert!(!built.translated.stages_reading(2).is_empty()); // DynamicTransforms
        assert!(!built.translated.stages_reading(3).is_empty()); // Fog
    }

    #[test]
    fn linear_light_fragment_finish_validates_on_all_backends_and_skips_alpha_only() {
        let opts = Options {
            linear_light: true,
            check_backends: true,
            ..Default::default()
        };
        let built = translate(&sources(VSH, FSH), &opts, &stub_include).unwrap();
        assert!(
            built
                .translated
                .fragment
                .functions
                .iter()
                .any(|(_, f)| f.name.as_deref() == Some("hz_main"))
        );

        // Also verify lazy linear re-translation from expanded sources.
        let gamma = translate(
            &sources(VSH, FSH),
            &Options {
                linear_light: false,
                ..Default::default()
            },
            &stub_include,
        )
        .unwrap();
        let linear_frag = translate_linear_fragment(
            &gamma.vertex_expanded,
            &gamma.fragment_expanded,
            &Options::default(),
        )
        .unwrap();
        assert!(translate_backends(&linear_frag).is_empty());

        // Under OIT_ALPHA_ONLY with no location-0 output, fragment_finish is skipped cleanly.
        let oit_fsh = "\
#version 330
#include <minecraft:oit.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
#ifndef OIT_ALPHA_ONLY
out vec4 fragColor;
#endif
void main() {
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, vertexColor.a);
#else
    fragColor = sampleColorForAccumulation(vertexColor);
#endif
}
";
        let alpha_only = translate(
            &sources(VSH, oit_fsh),
            &Options {
                defines: &["OIT_ALPHA_ONLY"],
                linear_light: true,
                check_backends: true,
                ..Default::default()
            },
            &stub_include,
        )
        .unwrap();
        assert!(alpha_only.translated.fragment_writes().is_empty());
    }

    #[test]
    fn linear_light_fragment_finish_resolves_loc0_output_past_earlier_globals() {
        let fsh = "\
#version 330
in vec2 texCoord0;
in vec4 vertexColor;
vec4 scratchGlobal = vec4(0.25);
out vec4 customOut;
void main() {
    scratchGlobal = vertexColor * 0.5;
    customOut = scratchGlobal;
}
";
        let built = translate(
            &sources(VSH, fsh),
            &Options {
                linear_light: true,
                check_backends: true,
                ..Default::default()
            },
            &stub_include,
        )
        .unwrap();
        assert_eq!(built.translated.fragment_writes(), [0]);
        assert!(
            built
                .translated
                .fragment
                .functions
                .iter()
                .any(|(_, f)| f.name.as_deref() == Some("hz_main")),
            "linear_light must wrap main as hz_main using customOut, not scratchGlobal"
        );
    }

    #[test]
    fn sprite_include_binds_sampler0_and_sprite_rect_and_is_refused_when_untextured() {
        let fsh = "\
#version 330
#include <sunrise:pipeline_sprite.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = sampleSprite(texCoord0) * vertexColor;
}
";
        let built = translate(
            &sources(VSH, fsh),
            &Options {
                textured: true,
                check_backends: true,
                ..Default::default()
            },
            &stub_include,
        )
        .unwrap();
        assert!(!built.translated.stages_reading(4).is_empty()); // Sampler0 texture
        assert!(
            !built
                .translated
                .stages_reading(SPRITE_RECT_BINDING)
                .is_empty()
        ); // SpriteRect

        let err = translate(
            &sources(VSH, fsh),
            &Options {
                textured: false,
                ..Default::default()
            },
            &stub_include,
        )
        .unwrap_err();
        assert!(
            err[0].to_string().contains("RenderPipeline::textured"),
            "{err:?}"
        );

        // Raw `uniform sampler2D Sampler0;` without `.textured(...)` is also refused with hint.
        let raw_sampler = "\
#version 330
uniform sampler2D Sampler0;
in vec2 texCoord0;
out vec4 fragColor;
void main() { fragColor = texture(Sampler0, texCoord0); }
";
        let err = translate(
            &sources(VSH, raw_sampler),
            &Options::default(),
            &stub_include,
        )
        .unwrap_err();
        assert!(
            err[0].to_string().contains("RenderPipeline::textured"),
            "{err:?}"
        );
    }

    /// Stages from different folders resolve quoted includes independently and still translate
    /// the vertex stage with its authored line numbers and GLSL version.
    #[test]
    fn separate_vertex_and_fragment_directories_resolve_own_relative_includes() {
        let vsh = "#version 330\n#include \"vhelper.glsl\"\n\
            const uint authoredLine = __LINE__;\nconst int authoredVersion = __VERSION__;\n\
            in vec3 Position;\nout vec4 vertexColor;\n\
            void main() {\n\
                gl_Position = vec4(Position, float(authoredLine + uint(authoredVersion)));\n\
                vertexColor = vcolor();\n\
            }\n";
        let fsh = "#version 330\n#include \"fhelper.glsl\"\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = vertexColor * fscale(); }\n";
        let srcs = ProgramSources {
            vertex_file: "assets/libmod/shaders/core/v/shared.vsh",
            vertex_dir: "libmod:shaders/core/v/",
            vertex: vsh,
            fragment_file: "assets/appmod/shaders/core/f/glow.fsh",
            fragment_dir: "appmod:shaders/core/f/",
            fragment: fsh,
        };
        let include = |id: &str| match id {
            "libmod:shaders/core/v/vhelper.glsl" => {
                Ok(Some("vec4 vcolor() { return vec4(1.0); }\n".to_owned()))
            }
            "appmod:shaders/core/f/fhelper.glsl" => {
                Ok(Some("float fscale() { return 0.5; }\n".to_owned()))
            }
            other => stub_include(other),
        };
        let built = translate(&srcs, &Options::default(), &include).unwrap();
        assert_eq!(built.translated.fragment_writes(), [0]);
        let vertex = &built.translated.vertex;
        for (name, expected) in [
            ("authoredLine", naga::Literal::U32(3)),
            ("authoredVersion", naga::Literal::I32(330)),
        ] {
            let (_, constant) = vertex
                .constants
                .iter()
                .find(|(_, c)| c.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("missing constant {name}"));
            let expression = &vertex.global_expressions[constant.init];
            assert!(
                matches!(expression, naga::Expression::Literal(actual) if *actual == expected),
                "{name}: {expression:?}"
            );
        }
    }

    /// A split-folder vertex error after an include names the line in the stage file, not the
    /// shifted line in the expanded text.
    #[test]
    fn a_split_folder_vertex_error_names_its_authored_line() {
        let vsh = "#version 330\n#include \"height.glsl\"\nin vec3 Position;\n\
            layout(std140) uniform Unbound { vec4 Offset; };\n\
            void main() { gl_Position = vec4(Position, height()) + Offset; }\n";
        let fsh = "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }\n";
        let program = ProgramSources {
            vertex_file: "assets/libmod/shaders/core/v/shared.vsh",
            vertex_dir: "libmod:shaders/core/v/",
            ..sources(vsh, fsh)
        };
        let include = |id: &str| match id {
            "libmod:shaders/core/v/height.glsl" => {
                Ok(Some("float height() {\n    return 1.0;\n}\n".to_owned()))
            }
            other => stub_include(other),
        };
        let refused = translate(&program, &Options::default(), &include).unwrap_err();
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(refused[0].file, "assets/libmod/shaders/core/v/shared.vsh");
        assert_eq!(refused[0].line, 4, "{refused:?}");
        assert!(refused[0].message.contains("Unbound"), "{refused:?}");
    }

    #[test]
    fn guard_bounds_refuse_pathological_glsl_with_location() {
        let huge_chain = format!(
            "#version 330\nin vec2 texCoord0;\nout vec4 fragColor;\n\
             void main() {{\n    float a = texCoord0.x;\n    float x = a{};\n    fragColor = vec4(x);\n}}\n",
            "+a".repeat(250)
        );
        let err = translate(
            &sources(VSH, &huge_chain),
            &Options::default(),
            &stub_include,
        )
        .unwrap_err();
        assert_eq!(err[0].file, "assets/example/shaders/core/glow.fsh");
        assert!(err[0].line > 0, "{err:?}");
        assert!(err[0].message.contains("chains more than 200"), "{err:?}");
    }

    /// The guard refuses an expression chained too long in an included file, and the refusal
    /// names that file and the line in it, not the stage file and a line of the expanded text.
    #[test]
    fn a_guard_refusal_in_an_include_is_located_in_the_include() {
        let chain = format!("float a = 1.0;\nfloat x = a{};\n", "+a".repeat(250));
        let include = |id: &str| match id {
            "example:shaders/include/chain.glsl" => Ok(Some(chain.clone())),
            other => stub_include(other),
        };
        let fsh = "#version 330\nin vec2 texCoord0;\nout vec4 fragColor;\n\
            #include <example:chain.glsl>\nvoid main() { fragColor = vec4(1.0); }\n";
        let err = translate(&sources(VSH, fsh), &Options::default(), &include).unwrap_err();
        assert_eq!(err.len(), 1, "{err:?}");
        assert_eq!(err[0].file, "assets/example/shaders/include/chain.glsl");
        assert_eq!(err[0].line, 2);
        assert!(err[0].message.contains("chains more than 200"), "{err:?}");

        let csh = "#version 450\nlayout(local_size_x = 1) in;\n\
            #include <example:chain.glsl>\nvoid main() {}\n";
        let compute = ComputeSources {
            file: "assets/example/shaders/compute/a.csh",
            dir: "example:shaders/compute/",
            source: csh,
        };
        let options = ComputeOptions {
            defines: &[],
            mod_bindings: &[],
            check_backends: false,
            interface: DEVICELESS_INTERFACE_LIMITS,
        };
        let err = translate_compute(&compute, &options, &include).unwrap_err();
        assert_eq!(err.len(), 1, "{err:?}");
        assert_eq!(err[0].file, "assets/example/shaders/include/chain.glsl");
        assert_eq!(err[0].line, 2);
    }

    /// `sampleSprite`'s texel arithmetic: `uv` 0 to 1 spans the sprite's rect once, the far edge
    /// and every repeat (negative too) land back inside it, never on a neighbour's texel.
    #[test]
    fn the_repeat_picks_integer_texels_inside_the_rect_for_any_uv() {
        let rect = [0.5, 0.25, 0.75, 0.5];
        let at = |u: f32, v: f32| repeat_texel([u, v], rect, [64, 64]);
        assert_eq!(at(0.0, 0.0), [32, 16], "the near corner");
        assert_eq!(at(0.5, 0.5), [40, 24]);
        assert_eq!(at(0.999_999, 0.999_999), [47, 31], "just inside far edge");
        assert_eq!(at(1.0, 1.0), [32, 16], "far edge wraps to start");
        assert_eq!(at(2.5, 3.0625), [40, 17], "repeated");
        assert_eq!(at(-0.0625, -0.25), [47, 28], "negative wraps from far side");
        assert_eq!(at(-1.0, -2.0), [32, 16]);
        assert_eq!(at(-1.0e-9, -1.0e-9), [47, 31]);
        assert_eq!(at(-1000.5, 1000.75), [40, 28]);
        for u in [0.0, 0.25, 0.999, 1.0, -0.5, 7.75] {
            assert_eq!(
                repeat_texel([u, u], [0.5, 0.0, 1.0, 1.0], [2, 1]),
                [1, 0],
                "{u}"
            );
        }
        for u in [0.0, 0.999, -0.001, 3.5] {
            assert_eq!(
                repeat_texel([u, 0.0], [0.0, 0.0, 0.5, 1.0], [2, 1]),
                [0, 0],
                "{u}"
            );
        }
    }

    #[test]
    fn order_independent_program_translates_all_three_oit_variants_and_refuses_alpha_only_color_output()
     {
        let oit_fsh = "\
#version 330
#include <minecraft:globals.glsl>
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
";
        let opts = Options {
            check_backends: true,
            ..Default::default()
        };
        let translucent = translate_oit(&sources(VSH, oit_fsh), &opts, false, &stub_include)
            .expect("translucent OIT variants must translate");
        assert_eq!(translucent[0].translated.fragment_writes(), [0]);
        assert_eq!(translucent[1].translated.fragment_writes(), [0, 1]);
        assert_eq!(translucent[2].translated.fragment_writes(), [0]);
        assert!(translucent[0].translated.other.is_empty());
        assert_eq!(translucent[1].translated.other, ["DepthBoundsSampler"]);
        assert_eq!(
            translucent[2].translated.other,
            ["DepthBoundsSampler", "Coeff0", "Coeff1"]
        );

        let additive = translate_oit(&sources(VSH, oit_fsh), &opts, true, &stub_include)
            .expect("additive OIT variants must translate");
        assert_eq!(additive[2].translated.fragment_writes(), [0]);

        // Textured OIT program also translates cleanly.
        let tex_oit_fsh = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:oit.glsl>
#include <sunrise:pipeline_sprite.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
#ifndef OIT_ALPHA_ONLY
out vec4 fragColor;
#endif
void main() {
    vec4 color = sampleSprite(texCoord0) * vertexColor * ColorModulator;
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, color.a);
#else
#ifdef OIT_ACCUMULATE
    color = sampleColorForAccumulation(color);
#endif
    fragColor = color;
#endif
}
";
        let tex_opts = Options {
            textured: true,
            check_backends: true,
            ..Default::default()
        };
        translate_oit(&sources(VSH, tex_oit_fsh), &tex_opts, false, &stub_include)
            .expect("textured OIT variants must translate");

        // Writing `fragColor` under `OIT_ALPHA_ONLY` must be refused at the depth_bounds variant.
        let broken_fsh = "\
#version 330
#include <minecraft:oit.glsl>
in vec4 vertexColor;
out vec4 fragColor;
void main() {
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, vertexColor.a);
#else
    fragColor = sampleColorForAccumulation(vertexColor);
#endif
}
";
        let err = translate_oit(&sources(VSH, broken_fsh), &opts, false, &stub_include)
            .expect_err("fragColor under OIT_ALPHA_ONLY must be refused");
        assert!(
            err[0].message.contains("OIT depth_bounds"),
            "expected depth_bounds refusal, got: {err:?}"
        );
    }

    #[test]
    fn custom_integer_and_unorm_vertex_inputs_translate_and_refuse_undeclared_in() {
        let custom_vsh = "\
#version 330
#include <minecraft:projection.glsl>
#include <minecraft:dynamictransforms.glsl>
in uvec4 PackedPos;
in vec4 PackedColor;
in ivec2 CellOffset;
out vec4 vertexColor;
void main() {
    vec3 pos = vec3(PackedPos.xyz) + vec3(float(CellOffset.x), 0.0, float(CellOffset.y));
    gl_Position = ProjMat * ModelViewMat * vec4(pos, 1.0);
    vertexColor = PackedColor;
}
";
        let custom_fsh = "\
#version 330
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor;
}
";
        let custom_inputs = [
            ("PackedPos", "uvec4"),
            ("PackedColor", "vec4"),
            ("CellOffset", "ivec2"),
        ];
        let opts = Options {
            vertex_inputs: &custom_inputs,
            check_backends: true,
            ..Default::default()
        };
        let built = translate(&sources(custom_vsh, custom_fsh), &opts, &stub_include)
            .expect("custom integer+unorm vertex shader must translate on all backends");
        assert_eq!(built.translated.vertex.entry_points.len(), 1);

        // Undeclared vertex `in` is refused naming the attribute.
        let undeclared_vsh = "\
#version 330
#include <minecraft:projection.glsl>
in uvec4 PackedPos;
in vec3 UndeclaredAttr;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * vec4(vec3(PackedPos.xyz) + UndeclaredAttr, 1.0);
    vertexColor = vec4(1.0);
}
";
        let err = translate(&sources(undeclared_vsh, custom_fsh), &opts, &stub_include)
            .expect_err("undeclared vertex input must be refused");
        assert!(
            err[0].message.contains("UndeclaredAttr"),
            "expected UndeclaredAttr in error, got: {err:?}"
        );

        // Type mismatch between VertexFormat and shader `in` is refused.
        let wrong_type_vsh = "\
#version 330
#include <minecraft:projection.glsl>
in vec4 PackedPos;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * PackedPos;
    vertexColor = vec4(1.0);
}
";
        let err = translate(&sources(wrong_type_vsh, custom_fsh), &opts, &stub_include)
            .expect_err("mismatched vertex input type must be refused");
        assert!(
            err[0].message.contains("PackedPos") && err[0].message.contains("uvec4"),
            "expected type mismatch in error, got: {err:?}"
        );
    }

    #[test]
    fn set1_bind_group_layout_and_push_constants_translate_and_refuse_undeclared_or_oversize() {
        let vsh = "\
#version 330
#include <minecraft:projection.glsl>
#include <minecraft:dynamictransforms.glsl>
layout(push_constant) uniform Push { vec4 Offset; };
uniform isamplerBuffer Lut;
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    int lut = texelFetch(Lut, 0).r;
    gl_Position = ProjMat * ModelViewMat * vec4(Position + Offset.xyz, 1.0);
    texCoord0 = UV0 + vec2(float(lut) * 0.0, 0.0);
    vertexColor = Color;
}
";
        let fsh = "\
#version 330
layout(std140) uniform TintBlock { vec4 TintColor; };
uniform sampler2D CustomTex;
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = texture(CustomTex, texCoord0) * vertexColor * TintColor;
}
";
        let mod_bindings = [
            ModBinding {
                name: "TintBlock",
                kind: ModBindingKind::UniformBuffer,
            },
            ModBinding {
                name: "CustomTex",
                kind: ModBindingKind::CombinedImageSampler,
            },
            ModBinding {
                name: "Lut",
                kind: ModBindingKind::TexelBuffer,
            },
        ];
        let opts = Options {
            mod_bindings: &mod_bindings,
            push_constant_size: 16,
            check_backends: true,
            ..Default::default()
        };
        let built = translate(&sources(vsh, fsh), &opts, &stub_include)
            .expect("set-1 uniform + sampler + texel buffer + push_constant must translate");
        assert!(built.translated.other.is_empty());

        // Undeclared block is refused naming the missing block.
        let missing_opts = Options {
            mod_bindings: &mod_bindings[1..],
            push_constant_size: 16,
            ..Default::default()
        };
        let err = translate(&sources(vsh, fsh), &missing_opts, &stub_include)
            .expect_err("undeclared TintBlock must fail");
        assert!(err[0].message.contains("TintBlock"), "got: {err:?}");

        // Push constant block exceeding declared push_constant_size is refused.
        let tiny_pc_opts = Options {
            mod_bindings: &mod_bindings,
            push_constant_size: 8,
            ..Default::default()
        };
        let err = translate(&sources(vsh, fsh), &tiny_pc_opts, &stub_include)
            .expect_err("push_constant larger than declared size must fail");
        assert!(err[0].message.contains("push_constant"), "got: {err:?}");

        // push_constant_size > 128 B is refused.
        let huge_pc_opts = Options {
            mod_bindings: &mod_bindings,
            push_constant_size: MAX_PUSH_CONSTANT_SIZE + 4,
            ..Default::default()
        };
        let err = translate(&sources(vsh, fsh), &huge_pc_opts, &stub_include)
            .expect_err("push_constant_size > 128 must fail");
        assert!(err[0].message.contains("128"), "got: {err:?}");

        // Static extraction helper finds the custom set-1 bindings and push constant.
        let (extracted, pc) = extract_mod_bindings(vsh, fsh, false);
        assert_eq!(pc, MAX_PUSH_CONSTANT_SIZE);
        assert_eq!(
            extracted,
            vec![
                ("Lut".to_owned(), ModBindingKind::TexelBuffer),
                ("TintBlock".to_owned(), ModBindingKind::UniformBuffer),
                ("CustomTex".to_owned(), ModBindingKind::CombinedImageSampler),
            ]
        );
    }
    #[test]
    fn storage_blocks_translate_into_set_one_and_refuse_undeclared_or_mismatched_access() {
        let vsh = "\
#version 330
in vec3 Position;
in vec2 UV0;
in vec4 Color;
layout(std430) readonly buffer Quads {
    uint words[];
};
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    float w = float(words[gl_VertexID]);
    gl_Position = vec4(Position + vec3(w * 0.0), 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
";
        let fsh = "\
#version 330
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor;
}
";
        let declared = |read_only| {
            [ModBinding {
                name: "Quads",
                kind: ModBindingKind::StorageBuffer { read_only },
            }]
        };
        let bindings = declared(true);
        let opts = Options {
            mod_bindings: &bindings,
            check_backends: true,
            ..Default::default()
        };
        translate(&sources(vsh, fsh), &opts, &stub_include)
            .expect("a read-only storage block translates");

        let err = translate(&sources(vsh, fsh), &Options::default(), &stub_include)
            .expect_err("an undeclared storage block is refused");
        assert!(err[0].message.contains("Quads"), "got: {err:?}");

        let bindings = declared(false);
        let opts = Options {
            mod_bindings: &bindings,
            ..Default::default()
        };
        let err = translate(&sources(vsh, fsh), &opts, &stub_include)
            .expect_err("a `readonly` block declared read-write is refused");
        assert!(err[0].message.contains("read-write"), "got: {err:?}");

        let (extracted, _) = extract_mod_bindings(vsh, fsh, false);
        assert_eq!(
            extracted,
            vec![(
                "Quads".to_owned(),
                ModBindingKind::StorageBuffer { read_only: true }
            )]
        );
    }

    #[test]
    fn storage_images_translate_into_set_one_with_their_declared_format_and_access() {
        let vsh = "\
#version 330
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    gl_Position = vec4(Position, 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
";
        let fsh = |image: &str| {
            format!(
                "\
#version 330
in vec2 texCoord0;
in vec4 vertexColor;
{image}
out vec4 fragColor;
void main() {{
    imageStore(Pyramid, ivec2(0), vertexColor);
    fragColor = vertexColor;
}}
"
            )
        };
        let declared = |access| {
            [ModBinding {
                name: "Pyramid",
                kind: ModBindingKind::StorageImage { access },
            }]
        };
        let write_only = fsh("layout(rgba8) writeonly uniform image2D Pyramid;");
        let bindings = declared(sunrise_glsl::ImageAccess::WriteOnly);
        let opts = Options {
            mod_bindings: &bindings,
            check_backends: true,
            ..Default::default()
        };
        let built = translate(&sources(vsh, &write_only), &opts, &stub_include)
            .expect("a write-only rgba8 image translates");
        let module = &built.translated.fragment;
        let (_, image) = module
            .global_variables
            .iter()
            .find(|(_, var)| var.name.as_deref() == Some("Pyramid"))
            .expect("the image is a global");
        assert_eq!(
            image.binding,
            Some(naga::ResourceBinding {
                group: MOD_BIND_GROUP_SET,
                binding: 0
            })
        );
        assert!(matches!(
            module.types[image.ty].inner,
            naga::TypeInner::Image {
                class: naga::ImageClass::Storage {
                    format: naga::StorageFormat::Rgba8Unorm,
                    ..
                },
                ..
            }
        ));

        // The qualifiers must agree with the declared access, and the image must be declared.
        let read_write = declared(sunrise_glsl::ImageAccess::ReadWrite);
        let opts = Options {
            mod_bindings: &read_write,
            ..Default::default()
        };
        assert!(
            translate(&sources(vsh, &write_only), &opts, &stub_include).is_err(),
            "`writeonly` on a read-write image is refused"
        );
        assert!(
            translate(
                &sources(vsh, &write_only),
                &Options::default(),
                &stub_include,
            )
            .is_err(),
            "an undeclared image is refused"
        );

        // The texel format comes from `layout(<format>)`: without one the image is refused.
        let no_format = fsh("writeonly uniform image2D Pyramid;");
        let opts = Options {
            mod_bindings: &bindings,
            ..Default::default()
        };
        assert!(
            translate(&sources(vsh, &no_format), &opts, &stub_include).is_err(),
            "an image with no format is refused"
        );

        // `uniform readonly image2D` orders its qualifiers after the storage word.
        let (extracted, _) = extract_mod_bindings(
            vsh,
            &fsh("layout(r32f) uniform readonly image2D Pyramid;"),
            false,
        );
        assert_eq!(
            extracted,
            vec![(
                "Pyramid".to_owned(),
                ModBindingKind::StorageImage {
                    access: sunrise_glsl::ImageAccess::ReadOnly
                }
            )]
        );
    }

    /// The bindings come from the text the translator reads, where comments are gone: a
    /// declaration named only in a line or a block comment is not the mod's.
    #[test]
    fn a_name_only_in_a_comment_is_not_a_mod_binding() {
        let vsh = "#version 330\n// uniform sampler2D LineOnly;\nin vec3 Position;\n\
            void main() { gl_Position = vec4(Position, 1.0); }\n";
        let fsh = "#version 330\n/* uniform sampler2D Spanning;\n\
            layout(std140) uniform Hidden { vec4 h; };\n*/\n\
            uniform sampler2D Real; /* uniform sampler2D Trailing; */\n\
            out vec4 fragColor;\nvoid main() { fragColor = texture(Real, vec2(0.0)); }\n";
        let ProgramBindings {
            bindings,
            push_constant_size,
            ..
        } = extract_program_bindings(&sources(vsh, fsh), &[], &stub_include).unwrap();
        assert_eq!(
            bindings,
            vec![("Real".to_owned(), ModBindingKind::CombinedImageSampler)]
        );
        assert_eq!(push_constant_size, 0);
    }

    /// A vertex input only in a line or block comment is no input: the comments go before the
    /// declarations are read.
    #[test]
    fn a_commented_vertex_input_is_no_input() {
        let vsh = "#version 330\n// in vec3 Commented;\nin vec3 Position; // in vec2 Trailing;\n\
            /* in vec4 Block;\nin vec4 Spanning; */\nin vec2 UV0;\n\
            void main() { gl_Position = vec4(Position, UV0.x); }\n";
        let fsh = "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }\n";
        let program = extract_program_bindings(&sources(vsh, fsh), &[], &stub_include).unwrap();
        assert_eq!(
            program.vertex_inputs,
            [
                ("Position".to_owned(), "vec3".to_owned()),
                ("UV0".to_owned(), "vec2".to_owned())
            ]
        );
    }

    /// A vertex stage's `in`s are read from the text the translator reads, so those an included
    /// file declares are the program's too, and a refused one is located in the file and at the line
    /// it came from.
    #[test]
    fn vertex_inputs_come_from_the_expanded_vertex_stage() {
        let vsh = "#version 330\n#include <example:attributes.glsl>\nin vec2 UV0;\n\
            void main() { gl_Position = vec4(Position, 1.0); }\n";
        let fsh = "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }\n";
        let include = |id: &str| match id {
            "example:shaders/include/attributes.glsl" => Ok(Some(
                "in vec3 Position;\nlayout(location = 3) in uvec4 Packed;\n".to_owned(),
            )),
            other => stub_include(other),
        };
        let program = extract_program_bindings(&sources(vsh, fsh), &[], &include).unwrap();
        let expected = [("Position", "vec3"), ("Packed", "uvec4"), ("UV0", "vec2")]
            .map(|(name, ty)| (name.to_owned(), ty.to_owned()));
        assert_eq!(program.vertex_inputs, expected);

        let include = |id: &str| match id {
            "example:shaders/include/attributes.glsl" => Ok(Some("in mat4 Matrix;\n".to_owned())),
            other => stub_include(other),
        };
        let refused = extract_program_bindings(&sources(vsh, fsh), &[], &include)
            .expect_err("an unsupported vertex input type is refused");
        assert_eq!(
            refused.file,
            "assets/example/shaders/include/attributes.glsl"
        );
        assert_eq!(refused.line, 1);
        assert!(
            refused.message.contains("`mat4` for `Matrix`"),
            "{refused:?}"
        );

        // The translator refuses an undeclared `in` of an include the same way, on a line
        // that is no line of the expanded text.
        let include = |id: &str| match id {
            "example:shaders/include/attributes.glsl" => {
                Ok(Some("in vec3 Position;\nin vec4 Stray;\n".to_owned()))
            }
            other => stub_include(other),
        };
        let refused = translate(&sources(vsh, fsh), &Options::default(), &include)
            .expect_err("an undeclared vertex input is refused");
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(
            refused[0].file,
            "assets/example/shaders/include/attributes.glsl"
        );
        assert_eq!(refused[0].line, 2);
        assert_eq!(
            refused[0].message,
            "vertex input `Stray` has no matching vertex attribute"
        );
    }

    /// A refused input on a continued line keeps the vertex stage and the column of its name
    /// on the physical line, not where it lands after splicing.
    #[test]
    fn a_refused_vertex_input_keeps_its_column_after_line_splicing() {
        let vsh = "#version 330\nin vec4 \\\n    Stray;\nvoid main() { gl_Position = Stray; }\n";
        let problems = translate(&sources(vsh, FSH), &Options::default(), &stub_include)
            .expect_err("an undeclared vertex input is refused");
        assert_eq!(
            problems,
            [Problem {
                file: "assets/example/shaders/core/glow.vsh".to_owned(),
                line: 3,
                column: 5,
                stage: Some(ShaderStage::Vertex),
                message: "vertex input `Stray` has no matching vertex attribute".to_owned(),
            }]
        );
    }

    /// A program whose include cannot be expanded is refused with the include's error, not read
    /// from its unexpanded text (where a binding would still be found).
    #[test]
    fn a_program_including_a_missing_file_is_refused_with_no_bindings() {
        let vsh = "#version 330\n#include <minecraft:nothing_here.glsl>\nin vec3 Position;\n\
            void main() { gl_Position = vec4(Position, 1.0); }\n";
        let fsh = "#version 330\nuniform sampler2D Real;\nout vec4 fragColor;\n\
            void main() { fragColor = texture(Real, vec2(0.0)); }\n";
        let refused = extract_program_bindings(&sources(vsh, fsh), &[], &stub_include)
            .expect_err("a missing include refuses the program");
        assert_eq!(
            refused,
            Problem {
                file: "assets/example/shaders/core/glow.vsh".to_owned(),
                line: 2,
                column: 0,
                stage: Some(ShaderStage::Vertex),
                message: "minecraft:shaders/include/nothing_here.glsl not found".to_owned(),
            }
        );
    }

    /// A missing nested include names the file and line that asked for it, whether reading
    /// bindings for `check` or preparing a render or compute stage for translation.
    #[test]
    fn a_missing_nested_include_names_the_including_file_and_line() {
        let include = |id: &str| match id {
            "example:shaders/include/outer.glsl" => {
                Ok(Some("// outer\n\n#include \"missing.glsl\"\n".to_owned()))
            }
            other => stub_include(other),
        };
        let expected = Problem {
            file: "assets/example/shaders/include/outer.glsl".to_owned(),
            line: 3,
            column: 0,
            stage: Some(ShaderStage::Compute),
            message: "example:shaders/include/missing.glsl not found".to_owned(),
        };
        let bad = "#version 450\n#include <example:outer.glsl>\n";
        for (stage, render) in [
            (ShaderStage::Vertex, sources(bad, FSH)),
            (ShaderStage::Fragment, sources(VSH, bad)),
        ] {
            let expected = Problem {
                stage: Some(stage),
                ..expected.clone()
            };
            assert_eq!(
                extract_program_bindings(&render, &[], &include).unwrap_err(),
                expected
            );
            assert_eq!(
                translate(&render, &Options::default(), &include).unwrap_err(),
                std::slice::from_ref(&expected)
            );
        }
        let compute = ComputeSources {
            file: "assets/example/shaders/compute/a.csh",
            dir: "example:shaders/compute/",
            source: bad,
        };
        let options = ComputeOptions {
            defines: &[],
            mod_bindings: &[],
            check_backends: false,
            interface: DEVICELESS_INTERFACE_LIMITS,
        };
        let Err(problems) = translate_compute(&compute, &options, &include) else {
            panic!("a missing include must refuse the compute shader");
        };
        assert_eq!(problems, [expected]);
    }

    /// A compute include error keeps the authored position and the stage separately.
    #[test]
    fn a_compute_translator_problem_keeps_its_position_and_stage() {
        let include = |id: &str| match id {
            "example:shaders/include/pick.glsl" => Ok(Some(
                "float pick() {\n    return missing_value;\n}\n".to_owned(),
            )),
            other => stub_include(other),
        };
        let sources = ComputeSources {
            file: "assets/example/shaders/compute/a.csh",
            dir: "example:shaders/compute/",
            source: "#version 450\n#include <example:pick.glsl>\nlayout(local_size_x = 1) in;\n\
                     void main() { float x = pick(); }\n",
        };
        let options = ComputeOptions {
            defines: &[],
            mod_bindings: &[],
            check_backends: false,
            interface: DEVICELESS_INTERFACE_LIMITS,
        };
        let Err(problems) = translate_compute(&sources, &options, &include) else {
            panic!("an undeclared name in the include must refuse the compute shader");
        };
        assert_eq!(problems.len(), 1, "{problems:?}");
        let displayed = problems[0].to_string();
        assert!(
            displayed
                .starts_with("assets/example/shaders/include/pick.glsl:2:12: compute shader: "),
            "{displayed}"
        );
    }

    /// A translator error in an included file names it by its asset path, as a stage file is.
    #[test]
    fn a_translator_error_in_an_include_names_its_asset_path() {
        let include = |id: &str| {
            match id {
            "example:shaders/include/pick.glsl" => Ok(Some(
                "uniform sampler2D MySampler0;\nvec4 pick() { return texture(MySampler0, vec2(0.0)); }\n"
                    .to_owned(),
            )),
            other => stub_include(other),
        }
        };
        let fsh = "#version 330\n#include <example:pick.glsl>\nout vec4 fragColor;\n\
            void main() { fragColor = pick(); }\n";
        let problems = translate(&sources(VSH, fsh), &Options::default(), &include)
            .expect_err("a sampler the pipeline doesn't provide is refused");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].file, "assets/example/shaders/include/pick.glsl");
    }

    /// The hint to declare `.textured` follows the name the pipeline refused: `Sampler0`
    /// and `SpriteRect` get it on an untextured pipeline, a name merely ending in one does not.
    #[test]
    fn the_textured_hint_follows_the_unbound_name() {
        let hint = "(requires RenderPipeline::textured)";
        let refused = |declaration: &str, used: &str| {
            let fsh = format!(
                "#version 330\n{declaration}\nout vec4 fragColor;\n\
                 void main() {{ fragColor = {used}; }}\n"
            );
            let problems = translate(&sources(VSH, &fsh), &Options::default(), &stub_include)
                .expect_err("an undeclared name is refused");
            assert_eq!(problems.len(), 1, "{problems:?}");
            problems[0].message.clone()
        };
        let sampler = refused(
            "uniform sampler2D Sampler0;",
            "texture(Sampler0, vec2(0.0))",
        );
        assert!(sampler.ends_with(hint), "{sampler}");
        let rect = refused(
            "layout(std140) uniform SpriteRect { vec4 r; };",
            "vec4(r.x)",
        );
        assert!(rect.ends_with(hint), "{rect}");
        let other = refused(
            "uniform sampler2D MySampler0;",
            "texture(MySampler0, vec2(0.0))",
        );
        assert!(other.contains("MySampler0 is not"), "{other}");
        assert!(!other.contains(hint), "{other}");
    }
}
