//! Resource packs' core shaders (`shaders/core/*.vsh` and `.fsh`: the GLSL vanilla compiles for its
//! pipelines) through [`sunrise_glsl`], so a pack that reshapes text from `core/text` (a HUD its
//! vertex shader moves to the screen's corners, effects in its fragment shader) draws as in
//! vanilla. This is the vanilla half: its uniform blocks and samplers, where its includes live
//! (26.3's `#include` and the older `#moj_import`, which pastes a file once, as
//! `GlslPreprocessor` imported them), and the define its compiler gives every program.

use naga::front::glsl::InterfaceLimits;
use sunrise_glsl::{Error, Expanded, Glue, ImportError, Includes, Inputs, Stages, Translated};

/// Vanilla's uniform blocks (`shaders/include/*.glsl`) and their bindings in set 0.
pub const BLOCKS: [(&str, u32); 4] = [
    ("Globals", 0),
    ("Projection", 1),
    ("DynamicTransforms", 2),
    ("Fog", 3),
];

/// Vanilla's samplers: the texture's binding in set 0, the sampler object's the next one.
pub const SAMPLERS: [(&str, u32); 3] = [("Sampler0", 4), ("Sampler1", 6), ("Sampler2", 8)];

/// The members of [`BLOCKS`] that packs still declare as loose uniforms, beside the blocks'
/// imports in another branch (the layout before 26.x moved them into blocks).
const LOOSE: [&str; 18] = [
    "CameraBlockPos",
    "GlintAlpha",
    "CameraOffset",
    "GameTime",
    "ScreenSize",
    "MenuBlurRadius",
    "UseRgss",
    "ProjMat",
    "TextureMat",
    "ModelViewMat",
    "ColorModulator",
    "FogColor",
    "FogEnvironmentalStart",
    "FogRenderDistanceStart",
    "FogRenderDistanceEnd",
    "FogEnd",
    "FogStart",
    "FogShape",
];

/// What `GlslCompiler.createBaseShaderOptions` defines for every program it compiles, core and
/// post alike, on a backend whose depth runs from 0 to 1, as wgpu's does.
pub(crate) const DEPTH_ZERO_TO_ONE: &str = "RENDERPEARL_DEPTH_IS_ZERO_TO_ONE";

/// A device's stage interface limits, as naga's GLSL front end takes them: its
/// `max_inter_stage_shader_variables` and `max_vertex_attributes`.
pub fn interface_limits(limits: &wgpu::Limits) -> InterfaceLimits {
    InterfaceLimits {
        inter_stage_variables: limits.max_inter_stage_shader_variables,
        vertex_attributes: limits.max_vertex_attributes,
    }
}

/// The shaders of one pipeline (`text.vsh`, `text.fsh`) with vanilla's `defines` for it.
/// `include` answers a file id (`minecraft:shaders/include/fog.glsl`) with its text. For tests,
/// which give a stage's text without a file: errors name the stages
/// `minecraft:shaders/core/core.vsh` and `.fsh`. The game names them ([`translate_core_at`]).
#[cfg(test)]
pub(crate) fn translate_core(
    vertex: &str,
    fragment: &str,
    defines: &[&str],
    include: &dyn Fn(&str) -> Option<String>,
    glue: &Glue,
) -> Result<Translated, Vec<Error>> {
    translate_core_with(vertex, fragment, defines, include, glue, &|_| false)
}

/// [`translate_core`] for a pipeline that also gives the blocks and samplers `provided` says
/// yes to, at their [`Translated::other`] bindings; the others are still an error.
#[cfg(test)]
pub(crate) fn translate_core_with(
    vertex: &str,
    fragment: &str,
    defines: &[&str],
    include: &dyn Fn(&str) -> Option<String>,
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Result<Translated, Vec<Error>> {
    let sources = Stages {
        vertex_dir: "minecraft:shaders/core/",
        fragment_dir: "minecraft:shaders/core/",
        vertex,
        fragment,
        vertex_file: "minecraft:shaders/core/core.vsh",
        fragment_file: "minecraft:shaders/core/core.fsh",
    };
    translate_core_at(&sources, defines, &|id| Ok(include(id)), glue, provided)
}

/// [`translate_core_with`] for stages that aren't in `minecraft:shaders/core/`, with `read`
/// answering a file id with its text, none when the packs haven't it, or why it can't be read
/// ([`expand_imports`]). Errors name the file and line the author wrote.
pub fn translate_core_at(
    sources: &Stages,
    defines: &[&str],
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Result<Translated, Vec<Error>> {
    let read = with_polytone_globals(read);
    let defines = program_defines(defines);
    let inputs = Inputs {
        blocks: &BLOCKS,
        samplers: &SAMPLERS,
        extra_set: 0,
        extra_blocks: &[],
        extra_samplers: &[],
        extra_texel_buffers: &[],
        extra_storage_buffers: &[],
        extra_storage_images: &[],
        push_constant_binding: None,
        loose: &LOOSE,
        defines: &defines,
        includes: &includes(&read),
    };
    sunrise_glsl::translate(sources, &inputs, glue, provided)
}

/// Whether `device` can make `program`'s pipeline `pipeline`, drawing into `targets` (their
/// formats), within its limits; otherwise said in the log ([`sunrise_glsl::PastDeviceLimit`]),
/// and the pipeline draws natively.
pub(crate) fn within_device_limits(
    device: &wgpu::Device,
    program: &Translated,
    pipeline: &str,
    targets: &[wgpu::TextureFormat],
) -> bool {
    match program.past_device_limit(&device.limits(), targets) {
        Some(past) => {
            log::warn!("the resource pack's shader for {pipeline} is drawn natively: {past}");
            false
        }
        None => true,
    }
}

/// What `f` makes on `device`, or the device's refusal named by `pipeline` (its location,
/// `minecraft:pipeline/sky`): a pack's shader is untrusted, and the backend failing to compile it
/// is an internal error. A required pipeline it refuses fails the pack reload
/// (`hud::pack_text::RequiredShaderRefused`).
pub(crate) fn scoped<T>(
    device: &wgpu::Device,
    pipeline: &str,
    f: impl FnOnce() -> T,
) -> Result<T, String> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let made = f();
    let internal = pollster::block_on(internal.pop());
    if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
        return Err(format!("{pipeline}: {e}"));
    }
    Ok(made)
}

/// The packs' file `file` (its id) with its includes inlined, every branch's: the one resolver
/// of post shaders, which keep their own preprocessing ([`sunrise_glsl::expand_includes`]),
/// each line keeping the file and line it came from. Only a missing `polytone_globals.glsl` is
/// the built-in one (`sunrise_polytone::globals::GLSL`).
pub(crate) fn expand_imports(
    text: &str,
    file: &str,
    dir: &str,
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<Expanded, ImportError> {
    let read = with_polytone_globals(read);
    sunrise_glsl::expand_includes(text, file, dir, &includes(&read), None)
}

/// [`expand_imports`] resolving only the includes a core pipeline reaches under its `defines`
/// and the backend's ([`DEPTH_ZERO_TO_ONE`]), as the compiler sees them.
pub(crate) fn expand_core_imports(
    text: &str,
    file: &str,
    dir: &str,
    defines: &[&str],
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<Expanded, ImportError> {
    let read = with_polytone_globals(read);
    let defines = program_defines(defines);
    sunrise_glsl::expand_includes(text, file, dir, &includes(&read), Some(&defines))
}

/// `code`'s directives run with the pipeline's `defines`, the backend's first
/// ([`DEPTH_ZERO_TO_ONE`]) (`sunrise_glsl::run_directives`). The core shaders' and the post
/// shaders' one way to them.
pub(crate) fn run_directives(code: &Expanded, defines: &[&str]) -> Result<Expanded, Error> {
    sunrise_glsl::run_directives(code, &program_defines(defines))
}

/// A program's defines as vanilla's compiler gives them: the backend's ([`DEPTH_ZERO_TO_ONE`]),
/// then the pipeline's `defines` (`NAME` or `NAME value`).
fn program_defines<'a>(defines: &[&'a str]) -> Vec<&'a str> {
    std::iter::once(DEPTH_ZERO_TO_ONE)
        .chain(defines.iter().copied())
        .collect()
}

/// Vanilla's include layout: `<path>` is `minecraft:shaders/include/path`, `#moj_import` pastes
/// a file once.
fn includes<'a>(read: &'a dyn Fn(&str) -> Result<Option<String>, String>) -> Includes<'a> {
    Includes {
        read,
        namespace: "minecraft",
        folder: "shaders/include/",
        once: &["moj_import"],
    }
}

/// `read`, with Polytone's built-in `polytone_globals.glsl` where the packs have none.
fn with_polytone_globals(
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> impl Fn(&str) -> Result<Option<String>, String> {
    move |id| {
        Ok(read(id)?.or_else(|| {
            sunrise_polytone::globals::is_include(id)
                .then(|| sunrise_polytone::globals::GLSL.to_owned())
        }))
    }
}

/// A program of stages not yet validated (tests that build their own).
#[cfg(any(test, feature = "test-support"))]
pub fn validated(vertex: naga::Module, fragment: naga::Module, other: Vec<String>) -> Translated {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    let validate = |module: naga::Module| {
        let info = Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .expect("the module validates");
        (module, info)
    };
    let other_base = BLOCKS
        .iter()
        .map(|&(_, b)| b + 1)
        .chain(SAMPLERS.iter().map(|&(_, b)| b + 2))
        .max()
        .unwrap_or(0);
    Translated::new(validate(vertex), validate(fragment), other, other_base)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a missing `polytone_globals.glsl` is Polytone's built-in one: the packs' own file is
    /// read instead, one that can't be read refuses the shader with the reader's reason, and any
    /// other missing file is named.
    #[test]
    fn only_a_missing_poly_globals_is_built_in() {
        let import = "#moj_import <polytone_globals.glsl>\n";
        let file = "minecraft:shaders/core/text.vsh";
        let dir = "minecraft:shaders/core/";
        let globals = "minecraft:shaders/include/polytone_globals.glsl";
        let expand = |text: &str, read: &dyn Fn(&str) -> Result<Option<String>, String>| {
            expand_imports(text, file, dir, read)
                .map(|e| e.text().to_owned())
                .map_err(|e| e.refusal)
        };
        let built_in = expand(import, &|_| Ok(None)).unwrap();
        let pasted: String = (sunrise_polytone::globals::GLSL.lines())
            .map(|line| format!("{line}\n"))
            .collect();
        assert_eq!(built_in, format!("{pasted}\n"));
        let shipped = |id: &str| Ok((id == globals).then(|| "float shipped;\n".to_owned()));
        assert_eq!(expand(import, &shipped).unwrap().trim(), "float shipped;");
        let unreadable = |id: &str| Err(format!("{id}: not text"));
        assert_eq!(
            expand(import, &unreadable),
            Err(sunrise_glsl::ImportRefusal::Unreadable(format!(
                "{globals}: not text"
            )))
        );
        let other = expand("#moj_import <fog.glsl>\n", &|_| Ok(None));
        assert_eq!(
            other,
            Err(sunrise_glsl::ImportRefusal::NotFound(
                "minecraft:shaders/include/fog.glsl".into()
            ))
        );
    }
}
