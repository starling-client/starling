//! One GLSL program (a vertex and a fragment shader, as a GL program links them) and a
//! description of what its pipeline gives it ([`Inputs`]), as two validated naga modules. The
//! crate knows no game: which blocks and samplers exist, where includes are found and what each
//! define means are the caller's.
//!
//! The GLSL (`#include`, or another directive the caller names that pastes a file once, with
//! explicit locations or without) is written again as the Vulkan-flavoured GLSL 450 naga reads:
//! - GLSL a GL compiler takes and naga doesn't is rewritten first ([`glsl_fixups`]: a helper
//!   taking the `sampler2D` it reads, an array sized by its initializer only);
//! - includes are inlined ([`Includes`]), under the defines the program is compiled with;
//! - after the rewrites below, the directives run ([`run_directives`]): conditions and macros
//!   are Sunrise's own preprocessor's ([`condition`], [`macros`]), and naga reads code with no
//!   directive or define left but `#version`, `#extension` and `#pragma`;
//! - the uniform blocks get the bindings of [`Inputs::blocks`], and each sampler of
//!   [`Inputs::samplers`] is split into a texture and a sampler object, combined again where the
//!   sampler is named;
//! - the vertex inputs become globals the caller's [`Glue`] fills before the program's `main`
//!   (renamed `hz_main`) runs, and `gl_VertexID` and `gl_VertexIndex` read the glue's expression;
//!   the glue's [`Glue::fragment_finish`] runs after the fragment shader's `main`, renamed too;
//! - varyings get locations by name across the stages, the fragment output location 0;
//! - an integer texel buffer (`uniform isamplerBuffer Faces;`) becomes a read-only storage buffer
//!   of `int`s (`uint`s for `usamplerBuffer`), and `texelFetch` on it reads one into the first
//!   channel of an `ivec4`: wgpu has no texel buffers. Like any block the caller gives beyond
//!   its own, it is bound at [`Translated::other_binding`], and refused when not given.
//!
//! Rewrites keep where the code came from ([`Expanded`]), so an error names the file (the
//! stage's or an include's id), line and column the author wrote ([`Error`]).
//!
//! A caller that writes the stage as naga reads it itself (`sunrise-shaders`, for Iris/OptiFine
//! programs) compiles it through [`compile_stage`] instead, its own resources moved through a
//! [`BindingMap`].

use std::collections::HashSet;

use naga::ShaderStage;
use naga::front::glsl::{Frontend, InterfaceLimits, Options};
use naga::valid::{Capabilities, ValidationFlags, Validator};

pub mod condition;
mod directives;
mod glsl_fixups;
mod located;
pub mod macros;
mod stage;

pub use directives::run_directives;
pub use located::{Error, Expanded, Location, describe, shader_message};
use located::{Lines, Origin};
pub use macros::Macros;
pub use stage::{BindingMap, Source, compile_stage};

/// What a program's pipeline gives it, beyond its [`Glue`].
pub struct Inputs<'a> {
    /// Uniform blocks by name, with their binding in set 0.
    pub blocks: &'a [(&'a str, u32)],
    /// Samplers by name: the texture's binding in set 0, the sampler object's the next one.
    pub samplers: &'a [(&'a str, u32)],
    /// Bind group set index for caller-declared extra bindings ([`Self::extra_blocks`],
    /// [`Self::extra_samplers`], [`Self::extra_texel_buffers`], and
    /// [`Self::push_constant_binding`]). Defaults to `0` for vanilla callers; mod pipelines
    /// place their declared `BindGroupLayout` in set `1`.
    pub extra_set: u32,
    /// Extra uniform blocks by name with their binding in [`Self::extra_set`].
    pub extra_blocks: &'a [(&'a str, u32)],
    /// Extra combined samplers by name: texture at `b`, sampler object at `b + 1` in
    /// [`Self::extra_set`].
    pub extra_samplers: &'a [(&'a str, u32)],
    /// Extra integer texel buffers by name: read-only storage buffer at `b` in
    /// [`Self::extra_set`].
    pub extra_texel_buffers: &'a [(&'a str, u32)],
    /// Extra std430 storage buffer blocks by name: `(name, binding, read_only)` in
    /// [`Self::extra_set`]. The block's `readonly` qualifier must agree with `read_only`.
    pub extra_storage_buffers: &'a [(&'a str, u32, bool)],
    /// Extra `image2D` storage images by name: `(name, binding, access)` in
    /// [`Self::extra_set`]. The declaration's `readonly`/`writeonly` qualifiers must agree with
    /// `access`, and its `layout(<format>)` names the texel format.
    pub extra_storage_images: &'a [(&'a str, u32, ImageAccess)],
    /// Optional binding in [`Self::extra_set`] onto which `layout(push_constant) uniform ...`
    /// blocks are translated as a std140 uniform buffer. `None` refuses `layout(push_constant)`.
    pub push_constant_binding: Option<u32>,
    /// Loose `uniform T name;` declarations dropped where they stand, as a block already declares
    /// the name (an older version of the block kept in an `#else` branch); any other loose
    /// uniform is an error.
    pub loose: &'a [&'a str],
    /// The defines the program is compiled with, in order: `NAME` or `NAME value`.
    pub defines: &'a [&'a str],
    pub includes: &'a Includes<'a>,
}

/// Where a program's includes come from.
pub struct Includes<'a> {
    /// A file id's text (`ns:path`), none when nobody has it, or why it can't be read, which
    /// refuses the program.
    pub read: &'a dyn Fn(&str) -> Result<Option<String>, String>,
    /// The namespace of an `<path>` include that names none.
    pub namespace: &'a str,
    /// The folder `<ns:path>` is under, ending in `/`; `"relative"` is from the including file's.
    pub folder: &'a str,
    /// Directives besides `#include` that paste a file, each file once per stage (`#include`
    /// pastes it every time, the file's own guards keeping one copy).
    pub once: &'a [&'a str],
}

/// What the caller's pipeline gives the program's vertex shader, and what it runs after the
/// fragment shader's.
pub struct Glue<'a> {
    /// The inputs the program's vertex shader may declare: name and GLSL type.
    pub inputs: &'a [(&'a str, &'a str)],
    /// Declarations before the program's code: the real attributes, helpers.
    pub prelude: &'a str,
    /// Statements before the program's `main`: `inputs` filled from the attributes.
    pub setup: &'a str,
    /// Statements after it, on `gl_Position`.
    pub finish: &'a str,
    /// The expression the program's `gl_VertexID` and `gl_VertexIndex` read.
    pub vertex_id: &'a str,
    /// Statements after the fragment shader's `main` (renamed `hz_main`, as the vertex
    /// shader's is), on its outputs, once `main` returns; a `discard` ends the invocation
    /// before them. None leaves the fragment shader as the program wrote it. Only [`translate`] runs it: a caller of [`compile_stage`]
    /// writes its own `main`.
    pub fragment_finish: Option<&'a str>,
    /// The stage interface limits the stages are translated with (a device's, or the lowest
    /// any device has when checking without one).
    pub interface: InterfaceLimits,
}

/// A pipeline's two stages, validated.
#[derive(Debug)]
pub struct Translated {
    pub vertex: naga::Module,
    pub fragment: naga::Module,
    /// The blocks and samplers that are none of [`Inputs::blocks`] and [`Inputs::samplers`], in
    /// binding order: the `i`th is at [`Translated::other_binding`]`(i)` (a sampler's object one
    /// after its texture; a block leaves that binding unused).
    pub other: Vec<String>,
    /// The resources of set 0 the stages' entry points read, each with those stages
    /// ([`bindings_read`], from the validation that made the program).
    pub read: Vec<(u32, wgpu_types::ShaderStages)>,
    /// The resources of set 1 the stages' entry points read, each with those stages.
    pub extra_read: Vec<(u32, wgpu_types::ShaderStages)>,
    /// First set-0 binding assigned to [`Translated::other`], strictly above every binding in
    /// [`Inputs::blocks`] and [`Inputs::samplers`].
    other_base: u32,
    /// The code each stage was parsed from, with where its lines came from, and where the
    /// fragment stage declares each output location (a byte of its code): what
    /// [`Translated::declared_at`] and [`Translated::output_at`] read. None for a program made
    /// from modules alone ([`Translated::new`]).
    sources: Option<Box<Sources>>,
}

#[derive(Debug)]
struct Sources {
    code: [Expanded; 2],
    outputs: Vec<(u32, usize)>,
}

/// Where the first of `stages` (each a module and the code it was parsed from) that declares
/// the resource at `binding` of set 0 declares it, preferring one whose entry point reads it
/// (`reading`, in the stages' order); the first stage's file with no line when none declares
/// it.
pub fn declared_at(
    stages: [(&naga::Module, &Expanded); 2],
    reading: [bool; 2],
    binding: u32,
) -> Location {
    let location_of = |(module, code): (&naga::Module, &Expanded)| {
        (module.global_variables.iter())
            .find(|(_, g)| {
                g.binding
                    .as_ref()
                    .is_some_and(|b| b.group == 0 && b.binding == binding)
            })
            .map(|(handle, _)| code.location_in(module.global_variables.get_span(handle)))
    };
    let order = [0, 1].into_iter().filter(|&i| reading[i]).chain([0, 1]);
    order
        .filter_map(|i| location_of(stages[i]))
        .next()
        .unwrap_or_else(|| Location {
            file: stages[0].1.file().to_owned(),
            line: 0,
        })
}

/// The integer texel buffers a stage declares (`uniform isamplerBuffer` or `usamplerBuffer`).
fn texel_buffers(text: &str) -> HashSet<&str> {
    resources(text)
        .into_iter()
        .filter(|resource| resource.kind == ResourceKind::TexelBuffer)
        .map(|resource| resource.name)
        .collect()
}

/// `texelFetch(Buffer, i)` on each integer texel buffer as `hz_fetch_Buffer(i)`, which the
/// buffer's declaration defines (see `rewrite`); `texelFetch` on anything else is left alone.
/// One pass over the text, however many buffers it declares: each `texelFetch` is looked at
/// once, its first argument's name looked up among them.
fn texel_fetches(lines: &mut Lines) {
    let text = lines.text();
    let buffers = texel_buffers(text);
    if buffers.is_empty() {
        return;
    }
    let mut edits = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find("texelFetch") {
        let at = from + found;
        let after = at + "texelFetch".len();
        from = after;
        // A name only ending in `texelFetch` is not it; one only starting with it has no `(`
        // next, so it isn't rewritten either.
        let whole = !text[..at].chars().next_back().is_some_and(is_ident);
        let args = text[after..]
            .trim_start()
            .strip_prefix('(')
            .map(str::trim_start);
        let Some(args) = args.filter(|_| whole) else {
            continue;
        };
        let name = &args[..args.find(|c: char| !is_ident(c)).unwrap_or(args.len())];
        if !buffers.contains(name) {
            continue;
        }
        if let Some(tail) = args[name.len()..].trim_start().strip_prefix(',') {
            let end = text.len() - tail.len();
            edits.push((at..end, format!("hz_fetch_{name}(")));
            from = end;
        }
    }
    lines.edit(edits);
}

impl Translated {
    /// The program of two stages and their validation, with what they read
    /// ([`Translated::read`]).
    pub fn new(
        (vertex, vertex_info): (naga::Module, naga::valid::ModuleInfo),
        (fragment, fragment_info): (naga::Module, naga::valid::ModuleInfo),
        other: Vec<String>,
        other_base: u32,
    ) -> Self {
        let read =
            bindings_read_in_group([(&vertex, &vertex_info), (&fragment, &fragment_info)], 0);
        let extra_read =
            bindings_read_in_group([(&vertex, &vertex_info), (&fragment, &fragment_info)], 1);
        Self {
            vertex,
            fragment,
            other,
            read,
            extra_read,
            other_base,
            sources: None,
        }
    }

    /// Where a stage declares the resource at `binding` of set 0, a stage reading it first
    /// ([`declared_at`]): what a refusal of that declaration names. Nowhere known for a program
    /// made from modules alone.
    pub fn declared_at(&self, binding: u32) -> Location {
        let Some(sources) = &self.sources else {
            return Location::default();
        };
        let stages = self.stages_reading(binding);
        declared_at(
            [
                (&self.vertex, &sources.code[0]),
                (&self.fragment, &sources.code[1]),
            ],
            [
                stages.contains(wgpu_types::ShaderStages::VERTEX),
                stages.contains(wgpu_types::ShaderStages::FRAGMENT),
            ],
            binding,
        )
    }

    /// Where the fragment stage declares its output at `location`; its file with no line when
    /// it declares none there (nowhere known for a program made from modules alone).
    pub fn output_at(&self, location: u32) -> Location {
        let Some(sources) = &self.sources else {
            return Location::default();
        };
        let at = sources.outputs.iter().find(|(l, _)| *l == location);
        sources.code[1].location_at(at.map(|(_, at)| *at))
    }

    /// The binding of the `index`th block or sampler of [`Translated::other`] in set 0.
    pub fn other_binding(&self, index: usize) -> u32 {
        self.other_base + 2 * index as u32
    }

    /// The size the stages declare the uniform block at `binding` of set 0 with, the larger of
    /// the two; 0 when neither declares one there.
    pub fn declared_size(&self, binding: u32) -> u64 {
        [&self.vertex, &self.fragment]
            .into_iter()
            .flat_map(|module| {
                module
                    .global_variables
                    .iter()
                    .filter_map(move |(_, global)| {
                        let bound = global.binding.as_ref()?;
                        (bound.group == 0
                            && bound.binding == binding
                            && global.space == naga::AddressSpace::Uniform)
                            .then(|| u64::from(module.types[global.ty].inner.size(module.to_ctx())))
                    })
            })
            .max()
            .unwrap_or(0)
    }

    /// The stages whose entry points read the resource at `binding` of set 0, through any
    /// function they call: what a pipeline layout must make that binding visible to. Empty
    /// for one a stage only declares (or the preprocessor kept but nothing reads).
    pub fn stages_reading(&self, binding: u32) -> wgpu_types::ShaderStages {
        stages_reading(&self.read, binding)
    }

    /// The stages whose entry points read the resource at `(group, binding)`.
    pub fn stages_reading_in_group(&self, group: u32, binding: u32) -> wgpu_types::ShaderStages {
        match group {
            0 => stages_reading(&self.read, binding),
            1 => stages_reading(&self.extra_read, binding),
            _ => wgpu_types::ShaderStages::NONE,
        }
    }

    /// Maximum declared std140 byte span of the uniform block at `(group, binding)` across both
    /// stages, or `None` if neither stage declares a uniform struct at that binding.
    pub fn uniform_block_size(&self, group: u32, binding: u32) -> Option<u32> {
        let mut max_span: Option<u32> = None;
        for module in [&self.vertex, &self.fragment] {
            for (_, global) in module.global_variables.iter() {
                let matches = global
                    .binding
                    .as_ref()
                    .is_some_and(|b| b.group == group && b.binding == binding);
                if matches
                    && global.space == naga::AddressSpace::Uniform
                    && let naga::TypeInner::Struct { span, .. } = module.types[global.ty].inner
                {
                    max_span = Some(max_span.map_or(span, |prev| prev.max(span)));
                }
            }
        }
        max_span
    }

    /// Whether every stage that reads the block at `binding` of set 0 declares it as one float
    /// (`uniform Name { float value; };`), what an expression uniform holds. A stage that only
    /// declares it, whatever its shape, binds nothing of it.
    pub fn is_float_block(&self, binding: u32) -> bool {
        let stages = self.stages_reading(binding);
        [
            (&self.vertex, wgpu_types::ShaderStages::VERTEX),
            (&self.fragment, wgpu_types::ShaderStages::FRAGMENT),
        ]
        .into_iter()
        .filter(|(_, stage)| stages.contains(*stage))
        .map(|(module, _)| module)
        .all(|module| {
            module
                .global_variables
                .iter()
                .filter(|(_, g)| {
                    g.binding
                        .as_ref()
                        .is_some_and(|b| b.group == 0 && b.binding == binding)
                })
                .all(|(_, g)| match (g.space, &module.types[g.ty].inner) {
                    (naga::AddressSpace::Uniform, naga::TypeInner::Struct { members, .. }) => {
                        matches!(
                            members.as_slice(),
                            [member] if matches!(
                                module.types[member.ty].inner,
                                naga::TypeInner::Scalar(naga::Scalar::F32)
                            )
                        )
                    }
                    _ => false,
                })
        })
    }

    /// The locations the fragment stage writes.
    pub fn fragment_writes(&self) -> Vec<u32> {
        locations(&self.fragment, false)
    }

    pub fn reads_binding(&self, binding: u32) -> bool {
        [&self.vertex, &self.fragment].into_iter().any(|module| {
            let bound: HashSet<naga::Handle<naga::GlobalVariable>> = module
                .global_variables
                .iter()
                .filter(|(_, g)| {
                    g.binding
                        .as_ref()
                        .is_some_and(|b| b.group == 0 && b.binding == binding)
                })
                .map(|(h, _)| h)
                .collect();
            let reads = |f: &naga::Function| {
                f.expressions.iter().any(
                    |(_, e)| matches!(e, naga::Expression::GlobalVariable(h) if bound.contains(h)),
                )
            };
            module.functions.iter().any(|(_, f)| reads(f))
                || module.entry_points.iter().any(|e| reads(&e.function))
        })
    }
}

/// The resources of set 0 that the entry points of each stage read, through any function they
/// call (naga's per entry point use, from the stage's own validation), each with the stages
/// reading it.
pub fn bindings_read(
    stages: [(&naga::Module, &naga::valid::ModuleInfo); 2],
) -> Vec<(u32, wgpu_types::ShaderStages)> {
    bindings_read_in_group(stages, 0)
}

/// The resources of bind group `group` that the entry points of each stage read, through any
/// function they call, each with the stages reading it.
pub fn bindings_read_in_group(
    stages: [(&naga::Module, &naga::valid::ModuleInfo); 2],
    group: u32,
) -> Vec<(u32, wgpu_types::ShaderStages)> {
    let mut read: Vec<(u32, wgpu_types::ShaderStages)> = Vec::new();
    for ((module, info), stage) in stages.into_iter().zip([
        wgpu_types::ShaderStages::VERTEX,
        wgpu_types::ShaderStages::FRAGMENT,
    ]) {
        for (handle, global) in module.global_variables.iter() {
            let Some(binding) = global.binding.as_ref().filter(|b| b.group == group) else {
                continue;
            };
            let reads =
                (0..module.entry_points.len()).any(|i| !info.get_entry_point(i)[handle].is_empty());
            if !reads {
                continue;
            }
            match read.iter_mut().find(|(b, _)| *b == binding.binding) {
                Some((_, stages)) => *stages |= stage,
                None => read.push((binding.binding, stage)),
            }
        }
    }
    read
}

/// The stages in `read` ([`bindings_read`]) that read the resource at `binding`.
pub fn stages_reading(
    read: &[(u32, wgpu_types::ShaderStages)],
    binding: u32,
) -> wgpu_types::ShaderStages {
    (read.iter())
        .find(|(b, _)| *b == binding)
        .map_or(wgpu_types::ShaderStages::NONE, |(_, stages)| *stages)
}

/// A program's authored stage sources, their diagnostic file names, and each stage's own
/// folder (`ns:path/`) for quoted (relative) includes.
pub struct Stages<'a> {
    pub vertex_dir: &'a str,
    pub fragment_dir: &'a str,
    pub vertex: &'a str,
    pub fragment: &'a str,
    pub vertex_file: &'a str,
    pub fragment_file: &'a str,
}

/// The program of `sources` under `inputs`, its vertex stage fed by `glue`. Blocks and samplers
/// beyond [`Inputs::blocks`] and [`Inputs::samplers`] are bound at [`Translated::other`]'s
/// bindings when `provided` says yes to their name (a block's and a sampler's alike: the caller
/// knows which kind each one is), and are an error otherwise. Each stage's quoted includes
/// resolve from its own folder. Errors name the file, line and column the author wrote and the
/// responsible stage when known ([`Error`]).
pub fn translate(
    sources: &Stages,
    inputs: &Inputs,
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Result<Translated, Vec<Error>> {
    let Stages {
        vertex_dir,
        fragment_dir,
        vertex,
        fragment,
        vertex_file,
        fragment_file,
    } = *sources;
    let stage = |text: &str, file: &str, dir: &str, stage| -> Result<Expanded, Vec<Error>> {
        let mut expanded = expand_includes(text, file, dir, inputs.includes, Some(inputs.defines))
            .map_err(|e| vec![Error::from(e).with_stage(stage)])?;
        expanded.lines = glsl_fixups::fixups(expanded.lines);
        texel_fetches(&mut expanded.lines);
        Ok(expanded)
    };
    let vertex_text = stage(vertex, vertex_file, vertex_dir, ShaderStage::Vertex)?;
    let fragment_text = stage(fragment, fragment_file, fragment_dir, ShaderStage::Fragment)?;
    let base =
        other_binding_base(inputs).map_err(|message| vec![Error::new(vertex_file, 0, message)])?;
    let mut names = Names::new(base);
    let rewritten = |text: &Expanded, stage, names: &mut Names| {
        rewrite(text, stage, inputs, glue, names).map_err(|e| vec![e.with_stage(stage)])
    };
    let vertex_code = rewritten(&vertex_text, ShaderStage::Vertex, &mut names)?;
    let fragment_code = rewritten(&fragment_text, ShaderStage::Fragment, &mut names)?;
    // The directives run where naga's preprocessor ran them: on the rewritten stage, the
    // program's defines first.
    let directives = |code: &Expanded, stage| {
        run_directives(code, inputs.defines).map_err(|e| vec![e.with_stage(stage)])
    };
    let vertex_parsed = directives(&vertex_code, ShaderStage::Vertex)?;
    let fragment_parsed = directives(&fragment_code, ShaderStage::Fragment)?;
    let mut vertex = compile(
        ShaderStage::Vertex,
        &vertex_parsed,
        &names,
        provided,
        glue.interface,
    )?;
    let fragment = compile(
        ShaderStage::Fragment,
        &fragment_parsed,
        &names,
        provided,
        glue.interface,
    )?;
    if let Err(refusal) = link(&vertex.0, &fragment.0) {
        // At the fragment shader's input of that location.
        let at = (names.inputs.iter()).find(|(l, _)| *l == refusal.location());
        return Err(vec![
            fragment_code
                .error_at(at.map(|(_, at)| *at), refusal.to_string())
                .with_stage(ShaderStage::Fragment),
        ]);
    }
    follow_fragment_interpolation(&mut vertex.0, &fragment.0);
    let mut program = Translated::new(vertex, fragment, names.other, names.other_base);
    // Each output's declaration, at the start of its line in the code that was parsed: the
    // directives keep every line where it was.
    let outputs = (names.outputs.into_iter())
        .map(|(location, at)| {
            let line = fragment_code.lines.line_of(at);
            (location, fragment_parsed.lines.line_start(line))
        })
        .collect();
    program.sources = Some(Box::new(Sources {
        code: [vertex_parsed, fragment_parsed],
        outputs,
    }));
    Ok(program)
}

/// Why the two stages don't meet, by vanilla's check when it builds a pipeline
/// (`PipelineBuilder`), at the location the fragment stage reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkRefusal {
    /// The vertex stage doesn't write it.
    Missing(u32),
    /// Float, int or uint in one stage, another in the other.
    BaseType(u32),
    /// Another number of components.
    Components(u32),
    /// Flat in one stage only.
    Flat(u32),
}

impl LinkRefusal {
    /// The location the fragment stage reads, where the stages don't meet.
    pub fn location(self) -> u32 {
        match self {
            Self::Missing(l) | Self::BaseType(l) | Self::Components(l) | Self::Flat(l) => l,
        }
    }
}

impl std::fmt::Display for LinkRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(l) => write!(
                f,
                "the fragment shader reads location {l}, which the vertex shader doesn't write"
            ),
            Self::BaseType(l) => write!(f, "the stages' base types differ at location {l}"),
            Self::Components(l) => write!(f, "the stages' vector lengths differ at location {l}"),
            Self::Flat(l) => write!(
                f,
                "the stages' interpolation differs at location {l}: flat in one only"
            ),
        }
    }
}

/// Vanilla's check of the two stages' interface when it builds a pipeline (`PipelineBuilder`):
/// each location the fragment stage reads is written by the vertex stage, with the same base type
/// (float, int or uint), the same number of components, and flat in both or in neither. It
/// compares no other interpolation or sampling ([`follow_fragment_interpolation`]).
fn link(vertex: &naga::Module, fragment: &naga::Module) -> Result<(), LinkRefusal> {
    let given = varyings(vertex, false);
    for (location, read) in varyings(fragment, true) {
        let Some((_, written)) = given.iter().find(|(l, _)| *l == location) else {
            return Err(LinkRefusal::Missing(location));
        };
        if read.kind != written.kind {
            return Err(LinkRefusal::BaseType(location));
        }
        if read.components != written.components {
            return Err(LinkRefusal::Components(location));
        }
        if read.flat() != written.flat() {
            return Err(LinkRefusal::Flat(location));
        }
    }
    Ok(())
}

/// The vertex stage's outputs given the interpolation and sampling the fragment stage reads each
/// with (`noperspective`, `centroid`, `sample`): vanilla's check compares only flat-ness
/// ([`link`]) and the fragment stage's qualifiers decide, while the device refuses any other
/// difference. An output the fragment stage doesn't read keeps its own.
fn follow_fragment_interpolation(vertex: &mut naga::Module, fragment: &naga::Module) {
    let read = varyings(fragment, true);
    let follow = |binding: &mut naga::Binding| {
        if let naga::Binding::Location {
            location,
            interpolation,
            sampling,
            ..
        } = binding
            && let Some((_, wanted)) = read.iter().find(|(l, _)| l == location)
        {
            *interpolation = wanted.interpolation;
            *sampling = wanted.sampling;
        }
    };
    for entry in &mut vertex.entry_points {
        let Some(result) = &mut entry.function.result else {
            continue;
        };
        if let Some(binding) = &mut result.binding {
            follow(binding);
            continue;
        }
        let ty = &vertex.types[result.ty];
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            continue;
        };
        let mut members = members.clone();
        for member in &mut members {
            if let Some(binding) = &mut member.binding {
                follow(binding);
            }
        }
        let followed = naga::Type {
            name: ty.name.clone(),
            inner: naga::TypeInner::Struct {
                members,
                span: *span,
            },
        };
        // The arena keeps one of each type: the same struct twice (nothing changed, or one
        // already declared alike) stays as it is.
        if vertex.types.get(&followed).is_none() {
            vertex.types.replace(result.ty, followed);
        }
    }
}

/// What a stage passes at one location, as [`link`] compares it.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Varying {
    kind: Option<naga::ScalarKind>,
    components: u8,
    interpolation: Option<naga::Interpolation>,
    sampling: Option<naga::Sampling>,
}

impl Varying {
    fn flat(&self) -> bool {
        self.interpolation == Some(naga::Interpolation::Flat)
    }
}

/// The locations the entry point reads (`inputs`) or writes, each with what passes there.
fn varyings(module: &naga::Module, inputs: bool) -> Vec<(u32, Varying)> {
    let mut out = Vec::new();
    let mut add = |ty: naga::Handle<naga::Type>, binding: Option<&naga::Binding>| {
        let varying = |ty: naga::Handle<naga::Type>, binding: &naga::Binding| {
            let (kind, components) = match module.types[ty].inner {
                naga::TypeInner::Scalar(s) => (Some(s.kind), 1),
                naga::TypeInner::Vector { size, scalar } => (Some(scalar.kind), size as u8),
                _ => (None, 0),
            };
            let naga::Binding::Location {
                location,
                interpolation,
                sampling,
                ..
            } = *binding
            else {
                return None;
            };
            Some((
                location,
                Varying {
                    kind,
                    components,
                    interpolation,
                    sampling,
                },
            ))
        };
        if let Some(binding) = binding {
            out.extend(varying(ty, binding));
        } else if let naga::TypeInner::Struct { members, .. } = &module.types[ty].inner {
            for m in members {
                out.extend(m.binding.as_ref().and_then(|b| varying(m.ty, b)));
            }
        }
    };
    for entry in &module.entry_points {
        if inputs {
            for arg in &entry.function.arguments {
                add(arg.ty, arg.binding.as_ref());
            }
        } else if let Some(result) = &entry.function.result {
            add(result.ty, result.binding.as_ref());
        }
    }
    out
}

/// A limit of wgpu's default device a program is past, where vanilla's device takes more: the
/// program draws natively and the pack stays, as before such a program failed the load (asking
/// the adapter for its own limits would fit it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PastDeviceLimit {
    /// More uniform blocks read by `stage` than `max_uniform_buffers_per_shader_stage`.
    UniformBlocks {
        stage: wgpu_types::ShaderStages,
        blocks: u32,
        limit: u32,
    },
    /// More varyings, or a varying at a higher location, than
    /// `max_inter_stage_shader_variables` allows.
    Varyings {
        count: u32,
        highest: u32,
        limit: u32,
    },
    /// The fragment stage writes the colour target at `location` (of `target`'s format) with
    /// another base type, or fewer components than the format has, than the device takes:
    /// vanilla's builder doesn't check a stage's outputs.
    ColourOutput {
        location: u32,
        kind: Option<naga::ScalarKind>,
        components: u8,
        target: wgpu_types::TextureFormat,
    },
}

impl std::fmt::Display for PastDeviceLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UniformBlocks {
                stage,
                blocks,
                limit,
            } => write!(
                f,
                "its {stage:?} stage reads {blocks} uniform blocks, past the device's {limit}"
            ),
            Self::Varyings {
                count,
                highest,
                limit,
            } => write!(
                f,
                "its stages pass {count} varyings up to location {highest}, past the device's {limit}"
            ),
            Self::ColourOutput {
                location,
                kind,
                components,
                target,
            } => write!(
                f,
                "its fragment stage writes colour target {location} ({target:?}) with {components} \
                 {kind:?} components, which the device doesn't take there"
            ),
        }
    }
}

impl Translated {
    /// The first of `limits` (the device's) the program is past, as the device would refuse its
    /// pipeline: the uniform blocks each stage reads (each bound only to the stages reading it),
    /// the varyings either stage passes, and the colour it writes into each of `targets` (their
    /// formats): the base type of the format, with at least as many components.
    pub fn past_device_limit(
        &self,
        limits: &wgpu_types::Limits,
        targets: &[wgpu_types::TextureFormat],
    ) -> Option<PastDeviceLimit> {
        for (location, written) in varyings(&self.fragment, false) {
            let Some(&target) = targets.get(location as usize) else {
                continue;
            };
            let kind = match target.sample_type(None, None) {
                Some(wgpu_types::TextureSampleType::Sint) => naga::ScalarKind::Sint,
                Some(wgpu_types::TextureSampleType::Uint) => naga::ScalarKind::Uint,
                _ => naga::ScalarKind::Float,
            };
            if written.kind != Some(kind) || written.components < target.components() {
                return Some(PastDeviceLimit::ColourOutput {
                    location,
                    kind: written.kind,
                    components: written.components,
                    target,
                });
            }
        }
        let stages = [
            (&self.vertex, wgpu_types::ShaderStages::VERTEX),
            (&self.fragment, wgpu_types::ShaderStages::FRAGMENT),
        ];
        for (module, stage) in stages {
            let blocks = (module.global_variables.iter())
                .filter(|(_, g)| g.space == naga::AddressSpace::Uniform)
                .filter_map(|(_, g)| g.binding.as_ref())
                .filter(|b| b.group == 0 && self.stages_reading(b.binding).contains(stage))
                .count() as u32;
            let limit = limits.max_uniform_buffers_per_shader_stage;
            if blocks > limit {
                return Some(PastDeviceLimit::UniformBlocks {
                    stage,
                    blocks,
                    limit,
                });
            }
        }
        let limit = limits.max_inter_stage_shader_variables;
        [(&self.vertex, false), (&self.fragment, true)]
            .into_iter()
            .find_map(|(module, inputs)| {
                let locations = locations(module, inputs);
                let count = locations.len() as u32;
                let highest = locations.iter().copied().max()?;
                (count > limit || highest >= limit).then_some(PastDeviceLimit::Varyings {
                    count,
                    highest,
                    limit,
                })
            })
    }
}

/// A compute shader, expanded, translated and validated ([`translate_compute`]).
#[derive(Debug)]
pub struct ComputeTranslated {
    pub module: naga::Module,
    /// The `local_size_x`, `local_size_y` and `local_size_z` the entry point declares.
    pub workgroup_size: [u32; 3],
    /// Bytes of `shared` (workgroup address space) variables the entry point uses
    /// (`max_compute_workgroup_storage_size` bounds them).
    pub workgroup_storage_bytes: u32,
    /// Whether the shader uses subgroup operations (`GL_KHR_shader_subgroup_*`, `subgroupAdd` and
    /// the like): a device without `Features::SUBGROUP` must refuse it.
    pub uses_subgroup: bool,
    /// The bindings of group 0 the entry point reads or writes.
    pub read: Vec<u32>,
}

/// One GLSL 450 compute shader under `inputs`, whose extra bindings are in
/// [`Inputs::extra_set`] (a compute pipeline has one bind group, the mod's, so callers use set
/// `0`). `layout(local_size_x = ..., local_size_y = ..., local_size_z = ...) in;` declares the
/// workgroup size; the caller checks it, [`ComputeTranslated::workgroup_storage_bytes`] and
/// [`ComputeTranslated::uses_subgroup`] against the device's limits and features, as only it
/// knows them. `in` and `out` variables are refused: a compute shader has no stage interface.
/// A refusal carries the compute stage and the authored position when known ([`Error`]).
pub fn translate_compute(
    text: &str,
    file: &str,
    dir: &str,
    inputs: &Inputs,
    interface: InterfaceLimits,
) -> Result<ComputeTranslated, Vec<Error>> {
    let stage = ShaderStage::Compute;
    let refused = |e: Error| vec![e.with_stage(stage)];
    let mut expanded = expand_includes(text, file, dir, inputs.includes, Some(inputs.defines))
        .map_err(|e| refused(e.into()))?;
    expanded.lines = glsl_fixups::fixups(expanded.lines);
    texel_fetches(&mut expanded.lines);
    let glue = Glue {
        inputs: &[],
        prelude: "",
        setup: "",
        finish: "",
        vertex_id: "0",
        fragment_finish: None,
        interface,
    };
    let mut names = Names::new(
        other_binding_base(inputs).map_err(|message| refused(Error::new(file, 0, message)))?,
    );
    let code = rewrite(&expanded, stage, inputs, &glue, &mut names).map_err(refused)?;
    // The directives run on the rewritten stage, the program's defines first, as for the other
    // stages.
    let parsed = run_directives(&code, inputs.defines).map_err(refused)?;
    let (module, info) = compile(stage, &parsed, &names, &|_| false, interface)?;
    let Some(entry) = module.entry_points.first() else {
        return Err(refused(Error::new(
            file,
            0,
            "the compute shader has no `main`",
        )));
    };
    let workgroup_size = entry.workgroup_size;
    let uses_subgroup = module_uses_subgroup(&module);
    let used = info.get_entry_point(0);
    let mut workgroup_storage_bytes = 0u32;
    let mut read = Vec::new();
    for (handle, global) in module.global_variables.iter() {
        if used[handle].is_empty() {
            continue;
        }
        if global.space == naga::AddressSpace::WorkGroup {
            workgroup_storage_bytes = workgroup_storage_bytes
                .saturating_add(module.types[global.ty].inner.size(module.to_ctx()));
        }
        if let Some(binding) = global.binding.as_ref().filter(|b| b.group == 0) {
            read.push(binding.binding);
        }
    }
    Ok(ComputeTranslated {
        module,
        workgroup_size,
        workgroup_storage_bytes,
        uses_subgroup,
        read,
    })
}

/// Whether `module` has a subgroup operation (each of the ballot, gather and collective
/// statements yields a result expression, which is what is looked for, so no statement walk is
/// needed) or an entry point that reads a subgroup built-in.
fn module_uses_subgroup(module: &naga::Module) -> bool {
    let operation = |expression: &naga::Expression| {
        matches!(
            expression,
            naga::Expression::SubgroupBallotResult
                | naga::Expression::SubgroupOperationResult { .. }
        )
    };
    let built_in = |argument: &naga::FunctionArgument| {
        matches!(
            argument.binding,
            Some(naga::Binding::BuiltIn(
                naga::BuiltIn::SubgroupSize
                    | naga::BuiltIn::SubgroupInvocationId
                    | naga::BuiltIn::NumSubgroups
                    | naga::BuiltIn::SubgroupId
            ))
        )
    };
    module
        .functions
        .iter()
        .map(|(_, function)| function)
        .chain(module.entry_points.iter().map(|entry| &entry.function))
        .any(|function| {
            function.expressions.iter().any(|(_, e)| operation(e))
                || function.arguments.iter().any(built_in)
        })
}

/// A bound on a program's includes, which are pasted every time.
const MAX_PASTES: u32 = 1024;
pub const MAX_EXPANDED: usize = 1 << 20;
/// A bound on the bytes the expansion scans (each pasted file, every time), however little of
/// them it keeps: lines of imports already pasted add nothing to the shader. The directives'
/// macro expansion takes it as a count of tokens ([`run_directives`], [`Macros::checked`]): a
/// token is at least a byte, so that bound is no looser than the bytes'. Each token the expansion
/// holds is a small allocation of its own (its text and the macros hidden from it), so a stage
/// that reaches the bound holds a few GiB at its peak (3.3 GiB measured). A shader pack's files
/// spend as many units each, in what their expansion holds (`sunrise-shaders`'
/// `MAX_EXPANSION_WORK`, [`Macros::bound`]).
pub const MAX_SCANNED: usize = 8 * MAX_EXPANDED;

/// Why a shader's includes refuse it ([`expand_includes`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportRefusal {
    /// A target that is neither `<ns:path>` nor a quoted path from the including file's folder.
    Bad(String),
    Directive(String),
    /// A file nobody has (its id).
    NotFound(String),
    /// A file that can't be read: why, naming it.
    Unreadable(String),
    /// Includes nested more than 16 deep.
    TooDeep,
    /// More than [`MAX_PASTES`] files pasted.
    TooManyPastes,
    /// The shader grown past [`MAX_EXPANDED`] bytes.
    TooLarge,
    /// More than [`MAX_SCANNED`] bytes scanned.
    TooMuchScanned,
}

impl std::fmt::Display for ImportRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Directive(why) => f.write_str(why),
            Self::Bad(target) => write!(f, "bad include {target}"),
            Self::NotFound(id) => write!(f, "{id} not found"),
            Self::Unreadable(why) => f.write_str(why),
            Self::TooDeep => f.write_str("includes nest more than 16 deep"),
            Self::TooManyPastes => write!(f, "includes paste more than {MAX_PASTES} files"),
            Self::TooLarge => write!(f, "includes grow the shader past {MAX_EXPANDED} bytes"),
            Self::TooMuchScanned => {
                write!(
                    f,
                    "includes make the translator read past {MAX_SCANNED} bytes"
                )
            }
        }
    }
}

/// An [`ImportRefusal`] at the file and line of the directive it stops at (line 0: the file as
/// a whole, as for an `#if` it leaves open).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportError {
    pub file: String,
    pub line: u32,
    pub refusal: ImportRefusal,
}

impl From<ImportError> for Error {
    fn from(e: ImportError) -> Self {
        Error::new(e.file, e.line, e.refusal.to_string())
    }
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Error::from(self.clone()).fmt(f)
    }
}

/// The file `file` (its id, `ns:path`) with its includes inlined (`#include <ns:path>`,
/// `#include "relative"` to the including file's folder, and the same with each of
/// [`Includes::once`]), `#version` and `#extension` lines dropped. An `#include` is pasted every
/// time, as a GL compiler's includes are (the files' `#ifndef` guards keep one copy); a
/// once-directive pastes a file once per stage. With `defines`, only the includes the compiler
/// reaches under them are resolved, and macros an included file defines govern the rest of its
/// caller; without, every branch's are. The expansion is bounded (16 levels, [`MAX_PASTES`]
/// pastes, [`MAX_EXPANDED`] bytes kept, [`MAX_SCANNED`] bytes scanned). Each line of the result
/// keeps the file, line and columns it came from ([`Expanded`]), and a refusal names its
/// directive's file and line.
pub fn expand_includes(
    text: &str,
    file: &str,
    dir: &str,
    includes: &Includes,
    defines: Option<&[&str]>,
) -> Result<Expanded, ImportError> {
    expand_with(text, file, dir, includes, defines)
}

/// naga's options for one `stage` of code whose directives have run ([`run_directives`]),
/// under the device's stage interface `limits`: no defines, as no macro is left.
pub fn glsl_options(stage: ShaderStage, limits: InterfaceLimits) -> Options {
    Options {
        interface_limits: limits,
        ..Options::from(stage)
    }
}

mod import_conditions;

fn expand_with(
    text: &str,
    file: &str,
    dir: &str,
    includes: &Includes,
    defines: Option<&[&str]>,
) -> Result<Expanded, ImportError> {
    struct Walk<'a> {
        includes: &'a Includes<'a>,
        conditions: Option<import_conditions::Conditions>,
        seen: HashSet<String>,
        pastes: u32,
        scanned: usize,
        out: Lines,
        /// The ids of the files read, the stage's first, and each one's index.
        files: Vec<String>,
        index: std::collections::HashMap<String, u32>,
    }
    impl Walk<'_> {
        /// The index of the file `id`, added the first time.
        fn file(&mut self, id: &str) -> u32 {
            if let Some(&index) = self.index.get(id) {
                return index;
            }
            let index = u32::try_from(self.files.len()).unwrap_or(u32::MAX);
            self.files.push(id.to_owned());
            self.index.insert(id.to_owned(), index);
            index
        }

        fn refused(&self, file: u32, line: u32, refusal: ImportRefusal) -> ImportError {
            ImportError {
                file: self.files[file as usize].clone(),
                line,
                refusal,
            }
        }

        /// Counts `text` against [`MAX_SCANNED`].
        fn scan(&mut self, text: &str) -> Result<(), ImportRefusal> {
            self.scanned += text.len();
            if self.scanned > MAX_SCANNED {
                return Err(ImportRefusal::TooMuchScanned);
            }
            Ok(())
        }

        fn walk(
            &mut self,
            text: &str,
            file: u32,
            dir: &str,
            depth: u32,
        ) -> Result<(), ImportError> {
            let mut authored = Lines::default();
            let mut physical = vec![1u32];
            let splice = self.conditions.is_some();
            for (n, line) in text.split_inclusive('\n').enumerate() {
                let origin = Origin {
                    file,
                    line: n as u32 + 1,
                };
                // Splicing precedes comment removal. The directive's line is the first one
                // of the logical line; each piece still has its own physical position.
                if splice
                    && let Some(part) = line
                        .strip_suffix("\\\r\n")
                        .or_else(|| line.strip_suffix("\\\n"))
                {
                    authored.push_authored(part, origin);
                } else {
                    authored.push_authored(line, origin);
                    physical.push(n as u32 + 2);
                }
            }
            let clean;
            let text = if splice {
                clean = strip_comments(authored.text());
                &clean
            } else {
                authored.text()
            };
            let mut branches = import_conditions::Conditionals::default();
            let mut line_offset = 0i64;
            for (n, line) in text.lines().enumerate() {
                // The line the author wrote it on: the first of a continued line.
                let written = physical[n];
                let origin = Some(Origin {
                    file,
                    line: written,
                });
                let directive = line.trim_start().strip_prefix('#').map(str::trim_start);
                let (word, rest) = directive.map_or(("", ""), |d| {
                    let i = d.find(|c| !is_ident(c)).unwrap_or(d.len());
                    (&d[..i], d[i..].trim())
                });
                if let Some(state) = &mut self.conditions {
                    // Whether the line is the preprocessor's alone (a branch's or `#line`).
                    let consumed: Result<bool, ImportRefusal> = 'directive: {
                        let location = u32::try_from(i64::from(physical[n]) + line_offset);
                        let Ok(location) = location else {
                            break 'directive Err(ImportRefusal::Directive(
                                "line number outside GLSL range".into(),
                            ));
                        };
                        match branches.apply_with(word, |kind| state.test(kind, rest, location)) {
                            Ok(skipped) if skipped || !branches.active() => {
                                break 'directive Ok(true);
                            }
                            Ok(_) => {}
                            Err(why) => break 'directive Err(ImportRefusal::Directive(why)),
                        }
                        match word {
                            "define" => state.define(rest),
                            "undef" => state.undef(rest),
                            "line" => {
                                let next = match state.line(rest, location) {
                                    Ok(next) => next,
                                    Err(why) => {
                                        break 'directive Err(ImportRefusal::Directive(why));
                                    }
                                };
                                line_offset = i64::from(next)
                                    - i64::from(
                                        physical.get(n + 1).copied().unwrap_or(physical[n] + 1),
                                    );
                                break 'directive Ok(true);
                            }
                            "" if state.uses_line(line) => {
                                let directive = format!("#line {}\n", location.saturating_sub(1));
                                self.out.push(&directive, origin);
                            }
                            _ => {}
                        }
                        Ok(false)
                    };
                    match consumed {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(refusal) => return Err(self.refused(file, written, refusal)),
                    }
                }
                match word {
                    _ if word == "include" || self.includes.once.contains(&word) => {
                        match self.include(word, rest, dir, depth) {
                            Ok(()) => {}
                            Err(Included::Here(refusal)) => {
                                return Err(self.refused(file, written, refusal));
                            }
                            Err(Included::Inside(error)) => return Err(error),
                        }
                    }
                    "version" | "extension" => {}
                    _ => {
                        let at = line.as_ptr() as usize - text.as_ptr() as usize;
                        self.out
                            .push_spanned(line, origin, &authored, at..at + line.len());
                    }
                }
                self.out.push("\n", origin);
                if self.out.text().len() > MAX_EXPANDED {
                    return Err(self.refused(file, written, ImportRefusal::TooLarge));
                }
            }
            if branches.open() {
                return Err(self.refused(
                    file,
                    0,
                    ImportRefusal::Directive("#if without #endif".into()),
                ));
            }
            Ok(())
        }

        /// The file an include directive `word` names (`rest`) pasted, when it is to be.
        fn include(
            &mut self,
            word: &str,
            rest: &str,
            dir: &str,
            depth: u32,
        ) -> Result<(), Included> {
            let id = include_id(rest, dir, self.includes)
                .ok_or_else(|| Included::Here(ImportRefusal::Bad(rest.to_owned())))?;
            if !(self.seen.insert(id.clone()) || word == "include") {
                return Ok(());
            }
            self.pastes += 1;
            if self.pastes > MAX_PASTES {
                return Err(Included::Here(ImportRefusal::TooManyPastes));
            }
            let text = (self.includes.read)(&id)
                .map_err(|why| Included::Here(ImportRefusal::Unreadable(why)))?
                .ok_or_else(|| Included::Here(ImportRefusal::NotFound(id.clone())))?;
            if depth + 1 > 16 {
                return Err(Included::Here(ImportRefusal::TooDeep));
            }
            self.scan(&text).map_err(Included::Here)?;
            let parent = &id[..id.rfind('/').map_or(id.len(), |i| i + 1)];
            let file = self.file(&id);
            self.walk(&text, file, parent, depth + 1)
                .map_err(Included::Inside)
        }
    }
    /// Why an include isn't pasted: at its directive, or inside the file.
    enum Included {
        Here(ImportRefusal),
        Inside(ImportError),
    }
    let mut walk = Walk {
        includes,
        conditions: None,
        seen: HashSet::new(),
        pastes: 0,
        scanned: 0,
        out: Lines::default(),
        files: Vec::new(),
        index: std::collections::HashMap::new(),
    };
    let top = walk.file(file);
    walk.scan(text)
        .map_err(|refusal| walk.refused(top, 0, refusal))?;
    let version = stage_version(text);
    walk.conditions = defines.map(|defines| import_conditions::Conditions::new(defines, version));
    walk.walk(text, top, dir, 0)?;
    Ok(Expanded {
        lines: walk.out,
        files: walk.files,
        version,
    })
}

/// `<ns:path>` under [`Includes::folder`] ([`Includes::namespace`] when there's no namespace), or
/// `"path"` from the including file's folder.
fn include_id(target: &str, dir: &str, includes: &Includes) -> Option<String> {
    if let Some(inner) = target.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
        let (ns, path) = inner.split_once(':').unwrap_or((includes.namespace, inner));
        return Some(format!("{ns}:{}{path}", includes.folder));
    }
    let relative = target.strip_prefix('"')?.strip_suffix('"')?;
    let (ns, base) = dir.split_once(':')?;
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    Some(format!("{ns}:{}", parts.join("/")))
}

/// The GLSL version a stage file declares (GLSL 4.60 §3.3), what its `__VERSION__` reads in
/// every condition and in its code: the number of its first `#version` line, comments and
/// continued lines read as the preprocessor reads them. As the GLSL spec allows nothing but
/// comments and white space before `#version`, the first one counts whatever branch it sits in,
/// and one in an include is not the stage's. A stage without `#version` targets 1.10 (§3.3), and
/// a `#version` without a decimal number (or one past `u32`) reads as that too.
pub(crate) fn stage_version(text: &str) -> u32 {
    let mut joined = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let part = line
            .strip_suffix("\\\r\n")
            .or_else(|| line.strip_suffix("\\\n"));
        joined.push_str(part.unwrap_or(line));
    }
    let clean = strip_comments(&joined);
    (clean.lines())
        .filter_map(directives::directive)
        .find(|(word, _)| *word == "version")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .filter(|number| number.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|number| number.parse().ok())
        .unwrap_or(110)
}

/// Comments as spaces, line breaks kept.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // Keep comment bytes as spaces: a pasted file still counts toward the stage budget.
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('/', Some('/')) => {
                out.push(' ');
                while let Some(&c) = chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    out.extend(std::iter::repeat_n(' ', c.len_utf8()));
                    chars.next();
                }
            }
            ('/', Some('*')) => {
                chars.next();
                out.push_str("  ");
                let mut last = ' ';
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                    } else {
                        out.extend(std::iter::repeat_n(' ', c.len_utf8()));
                    }
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// How a shader uses a storage image, as its `readonly`/`writeonly` qualifiers say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageAccess {
    /// `readonly`.
    ReadOnly,
    /// `writeonly`.
    WriteOnly,
    /// Neither qualifier.
    ReadWrite,
}

impl ImageAccess {
    /// The access the memory `qualifiers` of an `image2D` declaration say.
    pub fn of_qualifiers(qualifiers: &[&str]) -> Self {
        match (
            qualifiers.contains(&"readonly"),
            qualifiers.contains(&"writeonly"),
        ) {
            (true, false) => Self::ReadOnly,
            (false, true) => Self::WriteOnly,
            _ => Self::ReadWrite,
        }
    }
}

/// What a resource a stage declares is, as [`resources`] lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    /// A `uniform Name { ... }` block.
    UniformBlock,
    /// A `layout(push_constant) uniform Name { ... }` block.
    PushConstantBlock,
    /// `uniform sampler2D Name;`.
    Sampler2D,
    /// `uniform isamplerBuffer Name;` or `uniform usamplerBuffer Name;`.
    TexelBuffer,
    /// A `buffer Name { ... }` block.
    StorageBuffer {
        /// Whether the block says `readonly`.
        read_only: bool,
    },
    /// `layout(<format>) uniform image2D Name;`.
    StorageImage {
        /// What the declaration's `readonly` and `writeonly` qualifiers say.
        access: ImageAccess,
    },
}

/// A resource a stage declares: what `rewrite` binds by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resource<'a> {
    pub name: &'a str,
    pub kind: ResourceKind,
}

/// The resources the text of a stage declares, in the order it declares them (a name declared
/// twice is listed twice). It finds them as `rewrite` does when it binds them, so a caller
/// that needs to know which names a program declares (a static check, which has no layout to ask)
/// never disagrees with the translator: a declaration is read whatever else is on its line, and
/// whatever real GLSL qualifier (interpolation, precision, `invariant`, memory) stands before
/// `uniform`; any other word before it makes the statement no declaration. The text is read as
/// `rewrite` reads it: with comments gone and includes inlined ([`expand_includes`]).
pub fn resources(text: &str) -> Vec<Resource<'_>> {
    text.lines()
        .flat_map(declaration_lines)
        .filter_map(|(_, line)| declared(line))
        .filter_map(|declared| {
            let (name, kind) = match declared {
                Declared::Storage(block) => (
                    block.name,
                    ResourceKind::StorageBuffer {
                        read_only: block.qualifiers.contains(&"readonly"),
                    },
                ),
                Declared::Block {
                    name,
                    push_constant: true,
                    ..
                } => (name, ResourceKind::PushConstantBlock),
                Declared::Block { name, .. } => (name, ResourceKind::UniformBlock),
                Declared::TexelBuffer(decl) => (decl.name, ResourceKind::TexelBuffer),
                Declared::Image(decl) => (
                    decl.name,
                    ResourceKind::StorageImage {
                        access: ImageAccess::of_qualifiers(&decl.qualifiers),
                    },
                ),
                Declared::Sampler(decl) => (decl.name, ResourceKind::Sampler2D),
                Declared::Other(_) => return None,
            };
            Some(Resource { name, kind })
        })
        .collect()
}

/// The `in` declarations of a stage's text as `(offset, name, type)`, in the order it declares
/// them (a name declared twice is listed twice); `offset` is the byte of `text` where the name
/// stands. They are found as [`resources`] finds its own: through the one declaration
/// classifier, whatever else is on the line, and the text is read as `rewrite` reads it, with
/// comments gone and includes inlined ([`expand_includes`]).
pub fn stage_inputs(text: &str) -> Vec<(usize, &str, &str)> {
    lines_at(text)
        .flat_map(|(line_at, line)| {
            declaration_lines(line)
                .into_iter()
                .map(move |(at, piece)| (line_at + at, piece))
        })
        .filter_map(|(piece_at, piece)| match declared(piece)? {
            Declared::Other(decl) if decl.storage == "in" => {
                Some((piece_at + decl.name_at, decl.name, decl.ty))
            }
            _ => None,
        })
        .collect()
}

/// The lines of `text` (as [`str::lines`] splits them), each with the byte where it starts.
fn lines_at(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut start = 0;
    text.lines().map(move |line| {
        let at = start;
        start += text[start..]
            .find('\n')
            .map_or(text.len() - start, |n| n + 1);
        (at, line)
    })
}

/// A declaration on one line, classified: the one reading of a line that [`rewrite`] binds from
/// and [`resources`] and [`stage_inputs`] list from.
enum Declared<'a> {
    /// A `buffer Name {` block.
    Storage(StorageBlock<'a>),
    /// A `uniform Name {` block, a push-constant one when its `layout(...)` says so, with the
    /// line from its `uniform` token on.
    Block {
        name: &'a str,
        tail: &'a str,
        push_constant: bool,
    },
    /// `uniform isamplerBuffer Name;` or `uniform usamplerBuffer Name;`.
    TexelBuffer(Decl<'a>),
    /// `uniform image2D Name;`.
    Image(Decl<'a>),
    /// `uniform sampler2D Name;`.
    Sampler(Decl<'a>),
    /// Any other declaration: a loose uniform, an `in` or an `out`.
    Other(Decl<'a>),
}

/// What `line` declares, if it declares anything: a storage block first, then a uniform block,
/// then any other declaration. Its `layout(...)` is split off once ([`layout_prefix`]) and each
/// reader gets the parts.
fn declared(line: &str) -> Option<Declared<'_>> {
    let (layout, rest, rest_at) = layout_prefix(line)?;
    if let Some(block) = storage_block(layout, rest) {
        return Some(Declared::Storage(block));
    }
    if let Some((name, tail)) = block_name(rest) {
        return Some(Declared::Block {
            name,
            tail,
            push_constant: layout.split(',').any(|item| item.trim() == "push_constant"),
        });
    }
    let decl = declaration(layout, rest, rest_at)?;
    Some(match (decl.storage, decl.ty) {
        ("uniform", "isamplerBuffer" | "usamplerBuffer") => Declared::TexelBuffer(decl),
        ("uniform", "image2D") => Declared::Image(decl),
        ("uniform", "sampler2D") => Declared::Sampler(decl),
        _ => Declared::Other(decl),
    })
}

/// A global declaration on one line: `layout(...) qualifiers storage type name;`.
struct Decl<'a> {
    /// What stood inside `layout(...)`, empty without one.
    layout: &'a str,
    /// `layout(location = N)`'s N.
    location: Option<u32>,
    qualifiers: Vec<&'a str>,
    storage: &'a str,
    ty: &'a str,
    name: &'a str,
    /// The byte of the line given to [`declaration`] where `name` stands.
    name_at: usize,
}

/// The precision qualifiers, which GLSL takes before or after the storage word (`in highp vec4 X;`).
const PRECISION_QUALIFIERS: [&str; 3] = ["highp", "mediump", "lowp"];

/// The memory qualifiers of a declaration: before its storage word, or after it
/// (`uniform readonly image2D img;`).
const MEMORY_QUALIFIERS: [&str; 5] = ["readonly", "writeonly", "coherent", "volatile", "restrict"];

/// The other qualifiers that may stand before a declaration's `in`, `out` or `uniform`:
/// interpolation, auxiliary storage and `invariant`/`precise`.
const LEADING_QUALIFIERS: [&str; 8] = [
    "flat",
    "smooth",
    "noperspective",
    "centroid",
    "sample",
    "patch",
    "invariant",
    "precise",
];

/// A line's leading `layout(...)` qualifier split off: what stood inside it (trimmed, empty
/// without one), the line after it, and the byte of `line` where that rest starts. `None` for a
/// `layout` with no closed parenthesis.
fn layout_prefix(line: &str) -> Option<(&str, &str, usize)> {
    let trimmed = line.trim();
    let lead = line.len() - line.trim_start().len();
    let Some(after) = trimmed.strip_prefix("layout") else {
        return Some(("", trimmed, lead));
    };
    let after = after.trim_start().strip_prefix('(')?;
    let close = after.find(')')?;
    let rest = after[close + 1..].trim_start();
    Some((
        after[..close].trim(),
        rest,
        lead + trimmed.len() - rest.len(),
    ))
}

/// The declaration `line` is, if it is one: a statement of `layout(...)`, then only real
/// qualifiers (`LEADING_QUALIFIERS`, `PRECISION_QUALIFIERS`, `MEMORY_QUALIFIERS`), then `in`,
/// `out` or `uniform`, then precision and memory qualifiers, the type and the name. Any other
/// word before the storage word makes it some other statement (`x = in vec2 y;`,
/// `return out vec4 c;`). `layout`, `rest` and `rest_at` are the parts of the line
/// [`layout_prefix`] split off.
fn declaration<'a>(layout: &'a str, rest: &'a str, rest_at: usize) -> Option<Decl<'a>> {
    let location = layout.split(',').find_map(|q| {
        let (key, value) = q.split_once('=')?;
        (key.trim() == "location").then(|| value.trim().parse().ok())?
    });
    let body = rest.strip_suffix(';')?.trim_end();
    let words: Vec<&str> = body.split_whitespace().collect();
    let storage_at = words
        .iter()
        .position(|w| matches!(*w, "in" | "out" | "uniform"))?;
    let mut qualifiers = words[..storage_at].to_vec();
    if !qualifiers.iter().all(|w| {
        [
            &LEADING_QUALIFIERS[..],
            &PRECISION_QUALIFIERS,
            &MEMORY_QUALIFIERS,
        ]
        .iter()
        .any(|set| set.contains(w))
    }) {
        return None;
    }
    let mut tail = &words[storage_at + 1..];
    while let [word, others @ ..] = tail
        && (PRECISION_QUALIFIERS.contains(word) || MEMORY_QUALIFIERS.contains(word))
    {
        qualifiers.push(*word);
        tail = others;
    }
    let [ty, name] = *tail else {
        return None;
    };
    if !name.chars().all(is_ident) || !ty.chars().all(is_ident) {
        return None;
    }
    Some(Decl {
        layout,
        location,
        qualifiers,
        storage: words[storage_at],
        ty,
        name,
        // The name is the body's last word.
        name_at: rest_at + body.len() - name.len(),
    })
}

/// GLSL pack shaders are often minified, with several global declarations on one line, some after
/// code (`void f() { } uniform sampler2D Late;`). Keep the line-oriented rewriter simple by
/// separating every declaration statement ([`declared`]) wherever it stands on the line; the
/// code around them stays as it is, in pieces, each with the byte of `line` where it starts. A
/// statement ends at a `;` outside braces, and a declaration may start where the line does or
/// after a `}` that closes a braced body.
fn declaration_lines(line: &str) -> Vec<(usize, &str)> {
    let mut parts = Vec::new();
    let mut taken = 0;
    let mut depth = 0usize;
    let mut starts = vec![0];
    for (at, c) in line.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    starts.push(at + 1);
                }
            }
            ';' if depth == 0 => {
                let end = at + 1;
                // The latest start wins: `void f() { int x; } uniform sampler2D T;` declares T.
                if let Some(&start) = starts
                    .iter()
                    .rev()
                    .find(|&&start| declared(&line[start..end]).is_some())
                {
                    if taken < start {
                        parts.push((taken, &line[taken..start]));
                    }
                    parts.push((start, &line[start..end]));
                    taken = end;
                }
                starts = vec![end];
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        return vec![(0, line)];
    }
    if taken < line.len() {
        parts.push((taken, &line[taken..]));
    }
    parts
}

/// The block a `[layout(...)] uniform Name {` line opens, with its name and the line from the
/// `uniform` token on (`tail`, what the line leaves after its `layout(...)`, which is rewritten).
/// `uniform` is a whole word, never the start of an identifier such as `uniformColor`.
fn block_name(tail: &str) -> Option<(&str, &str)> {
    let rest = tail
        .strip_prefix("uniform")
        .filter(|after| !after.starts_with(is_ident))?
        .trim();
    let name_end = rest.find(|c: char| !is_ident(c)).unwrap_or(rest.len());
    let (name, after) = rest.split_at(name_end);
    // `{` alone, or the whole block on the line (`uniform Day { float daylight; };`).
    (!name.is_empty() && (after.trim().is_empty() || after.trim_start().starts_with('{')))
        .then_some((name, tail))
}

/// A `[layout(..)] [memory qualifiers] buffer Name {` line: a std430 storage buffer block.
#[derive(Debug, PartialEq, Eq)]
struct StorageBlock<'a> {
    /// What the `layout(...)` qualifier lists, empty when the line has none.
    layout: &'a str,
    /// The block's name.
    name: &'a str,
    /// Its memory qualifiers (`readonly`, `writeonly`, `coherent`, `volatile`, `restrict`).
    qualifiers: Vec<&'a str>,
    /// The line from the block's `{` on, empty when the brace is on the next line.
    tail: &'a str,
}

/// The storage buffer block a line opens, if it opens one: `layout` is what stood inside its
/// `layout(...)` and `rest` the line after it.
fn storage_block<'a>(layout: &'a str, mut rest: &'a str) -> Option<StorageBlock<'a>> {
    let word_end = |s: &str| s.find(|c: char| !is_ident(c)).unwrap_or(s.len());
    let mut qualifiers = Vec::new();
    loop {
        let (word, after) = rest.split_at(word_end(rest));
        let after = after.trim_start();
        match word {
            memory if MEMORY_QUALIFIERS.contains(&memory) => {
                qualifiers.push(word);
                rest = after;
            }
            "buffer" => {
                let (name, tail) = after.split_at(word_end(after));
                let tail = tail.trim_start();
                return (!name.is_empty() && (tail.is_empty() || tail.starts_with('{'))).then_some(
                    StorageBlock {
                        layout,
                        name,
                        qualifiers,
                        tail,
                    },
                );
            }
            _ => return None,
        }
    }
}

/// First set-0 binding strictly above every caller-declared block (`b`) and sampler (`b`, `b + 1`).
fn other_binding_base(inputs: &Inputs) -> Result<u32, String> {
    let mut base = 0u32;
    for &(_, b) in inputs.blocks {
        let next = b
            .checked_add(1)
            .ok_or_else(|| "block binding index overflow".to_owned())?;
        base = base.max(next);
    }
    for &(_, b) in inputs.samplers {
        let next = b
            .checked_add(2)
            .ok_or_else(|| "sampler binding index overflow".to_owned())?;
        base = base.max(next);
    }
    if inputs.extra_set == 0 {
        for &(_, b) in inputs.extra_blocks {
            let next = b
                .checked_add(1)
                .ok_or_else(|| "block binding index overflow".to_owned())?;
            base = base.max(next);
        }
        for &(_, b) in inputs.extra_samplers {
            let next = b
                .checked_add(2)
                .ok_or_else(|| "sampler binding index overflow".to_owned())?;
            base = base.max(next);
        }
        for &(_, b) in inputs.extra_texel_buffers {
            let next = b
                .checked_add(1)
                .ok_or_else(|| "texel buffer binding index overflow".to_owned())?;
            base = base.max(next);
        }
        for &(_, b, _) in inputs.extra_storage_buffers {
            let next = b
                .checked_add(1)
                .ok_or_else(|| "storage buffer binding index overflow".to_owned())?;
            base = base.max(next);
        }
        for &(_, b, _) in inputs.extra_storage_images {
            let next = b
                .checked_add(1)
                .ok_or_else(|| "storage image binding index overflow".to_owned())?;
            base = base.max(next);
        }
        if let Some(b) = inputs.push_constant_binding {
            let next = b
                .checked_add(1)
                .ok_or_else(|| "push constant binding index overflow".to_owned())?;
            base = base.max(next);
        }
    }
    Ok(base)
}

/// What the two stages share: the vertex stage's outputs and their locations, and the names of
/// the blocks and samplers of other pipelines in binding order.
struct Names {
    varyings: Vec<(String, u32)>,
    other: Vec<String>,
    other_base: u32,
    /// The fragment stage's inputs: each one's location and where the rewritten stage declares
    /// it (a byte of its text), which an error about the location names.
    inputs: Vec<(u32, usize)>,
    /// The fragment stage's outputs, the same way.
    outputs: Vec<(u32, usize)>,
}

impl Names {
    fn new(other_base: u32) -> Self {
        Self {
            varyings: Vec::new(),
            other: Vec::new(),
            other_base,
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    fn location(&self, varying: &str) -> Option<u32> {
        self.varyings
            .iter()
            .find(|(v, _)| v == varying)
            .map(|(_, location)| *location)
    }

    /// A block or sampler of another pipeline: its binding (two for a sampler).
    fn other_binding(&mut self, name: &str) -> Result<u32, String> {
        let index = match self.other.iter().position(|n| n == name) {
            Some(index) => index,
            None => {
                self.other.push(name.to_owned());
                self.other.len() - 1
            }
        };
        let offset = u32::try_from(index)
            .ok()
            .and_then(|i| i.checked_mul(2))
            .ok_or_else(|| "other binding index overflow".to_owned())?;
        let binding = self
            .other_base
            .checked_add(offset)
            .ok_or_else(|| "other binding index overflow".to_owned())?;
        binding
            .checked_add(1)
            .ok_or_else(|| "other binding index overflow".to_owned())?;
        Ok(binding)
    }
}

/// One stage written again for naga (see the module's notes). Generated code points back to
/// the declarations it replaces; copied block tails keep the positions of their own bytes.
fn rewrite(
    source: &Expanded,
    stage: ShaderStage,
    inputs: &Inputs,
    glue: &Glue,
    names: &mut Names,
) -> Result<Expanded, Error> {
    let text = source.text();
    let vertex = stage == ShaderStage::Vertex;
    let mut out = Lines::uniform("#version 450 core\n", None);
    if vertex {
        out.push(glue.prelude, None);
        out.push("\n", None);
        for (name, ty) in glue.inputs {
            out.push(&format!("{ty} {name};\n"), None);
        }
    }
    // Where the program's own code starts: the glue above it is left as it is.
    let pack_start = out.text().len();
    let mut outputs: Vec<&str> = Vec::new();
    let explicit: Vec<u32> = if vertex {
        text.lines()
            .flat_map(declaration_lines)
            .filter_map(|(_, line)| {
                let (layout, rest, rest_at) = layout_prefix(line)?;
                declaration(layout, rest, rest_at)
            })
            .filter(|d| d.storage == "out")
            .filter_map(|d| d.location)
            .collect()
    } else {
        Vec::new()
    };
    for (n, (here, source_line)) in lines_at(text).enumerate() {
        let origin = source.lines.origin(n);
        for (offset, line) in declaration_lines(source_line) {
            let from = here + offset;
            let Some(found) = declared(line) else {
                out.push_spanned(line, origin, &source.lines, from..from + line.len());
                out.push("\n", origin);
                continue;
            };
            let declaration_at = from + line.len() - line.trim_start().len();
            let at = |message: String| source.error_at(Some(declaration_at), message);
            let write = |out: &mut Lines, code: &str| {
                out.push_replacement(code, origin, &source.lines, declaration_at);
            };
            match found {
                Declared::Storage(block) => {
                    let Some(&(_, binding, read_only)) = inputs
                        .extra_storage_buffers
                        .iter()
                        .find(|(buffer, _, _)| *buffer == block.name)
                    else {
                        return Err(at(format!(
                            "storage buffer {} is not declared on this pipeline",
                            block.name
                        )));
                    };
                    if block.qualifiers.contains(&"readonly") != read_only {
                        return Err(at(format!(
                            "storage buffer {} is declared {} in the layout but {} in the shader",
                            block.name,
                            if read_only { "read-only" } else { "read-write" },
                            if read_only {
                                "without `readonly`"
                            } else {
                                "`readonly`"
                            },
                        )));
                    }
                    if let Some(packing) = block
                        .layout
                        .split(',')
                        .map(str::trim)
                        .find(|item| matches!(*item, "std140" | "shared" | "packed"))
                    {
                        return Err(at(format!(
                            "storage buffer {} is declared `layout({packing})`; a storage block is std430",
                            block.name
                        )));
                    }
                    let set = inputs.extra_set;
                    let (name, qualifiers, tail) =
                        (block.name, block.qualifiers.join(" "), block.tail);
                    write(
                        &mut out,
                        &format!(
                            "layout(std430, set = {set}, binding = {binding}) {qualifiers} buffer {name} "
                        ),
                    );
                    // `tail` is a suffix of the trimmed declaration, even if it is empty.
                    let tail_at = from + line.trim_end().len() - tail.len();
                    out.push_spanned(tail, origin, &source.lines, tail_at..tail_at + tail.len());
                    write(&mut out, "\n");
                }
                Declared::Block {
                    name,
                    tail,
                    push_constant,
                } => {
                    let (set, binding) = if push_constant {
                        let Some(b) = inputs.push_constant_binding else {
                            return Err(at(format!(
                                "push_constant block {name} is not declared on this pipeline"
                            )));
                        };
                        (inputs.extra_set, b)
                    } else if let Some((_, b)) =
                        inputs.blocks.iter().find(|(block, _)| *block == name)
                    {
                        (0, *b)
                    } else if let Some((_, b)) =
                        inputs.extra_blocks.iter().find(|(block, _)| *block == name)
                    {
                        (inputs.extra_set, *b)
                    } else {
                        (0, names.other_binding(name).map_err(at)?)
                    };
                    write(
                        &mut out,
                        &format!("layout(std140, set = {set}, binding = {binding}) "),
                    );
                    let tail_at = tail.as_ptr() as usize - text.as_ptr() as usize;
                    out.push_spanned(tail, origin, &source.lines, tail_at..tail_at + tail.len());
                    write(&mut out, "\n");
                }
                Declared::TexelBuffer(decl) => {
                    let (set, binding) = match inputs
                        .extra_texel_buffers
                        .iter()
                        .find(|(buf, _)| *buf == decl.name)
                    {
                        Some((_, b)) => (inputs.extra_set, *b),
                        None => (0, names.other_binding(decl.name).map_err(at)?),
                    };
                    let name = decl.name;
                    let (scalar, vector) = if decl.ty == "isamplerBuffer" {
                        ("int", "ivec4")
                    } else {
                        ("uint", "uvec4")
                    };
                    write(
                        &mut out,
                        &format!(
                            "layout(std430, set = {set}, binding = {binding}) readonly buffer hz_buffer_{name} {{ {scalar} hz_data_{name}[]; }};\n\
                             {vector} hz_fetch_{name}(int i) {{ return {vector}(hz_data_{name}[i], 0, 0, 1); }}\n"
                        ),
                    );
                }
                Declared::Image(decl) => {
                    let Some(&(_, binding, access)) = inputs
                        .extra_storage_images
                        .iter()
                        .find(|(image, _, _)| *image == decl.name)
                    else {
                        return Err(at(format!(
                            "storage image {} is not declared on this pipeline",
                            decl.name
                        )));
                    };
                    if ImageAccess::of_qualifiers(&decl.qualifiers) != access {
                        return Err(at(format!(
                            "storage image {} is declared {access:?} in the layout but its qualifiers say {:?}",
                            decl.name,
                            ImageAccess::of_qualifiers(&decl.qualifiers),
                        )));
                    }
                    if decl.layout.is_empty() || decl.layout.contains(['=', ',']) {
                        return Err(at(format!(
                            "storage image {} needs exactly `layout(<texel format>)`, such as `layout(rgba8)`",
                            decl.name
                        )));
                    }
                    let qualifiers = decl.qualifiers.join(" ");
                    let (set, format, name) = (inputs.extra_set, decl.layout, decl.name);
                    write(
                        &mut out,
                        &format!(
                            "layout({format}, set = {set}, binding = {binding}) {qualifiers} uniform image2D {name};\n"
                        ),
                    );
                }
                Declared::Sampler(decl) => {
                    let (set, binding) = match inputs
                        .samplers
                        .iter()
                        .find(|(sampler, _)| *sampler == decl.name)
                    {
                        Some((_, b)) => (0, *b),
                        None => match inputs
                            .extra_samplers
                            .iter()
                            .find(|(sampler, _)| *sampler == decl.name)
                        {
                            Some((_, b)) => (inputs.extra_set, *b),
                            None => (0, names.other_binding(decl.name).map_err(at)?),
                        },
                    };
                    let name = decl.name;
                    write(
                        &mut out,
                        &format!(
                            "layout(set = {set}, binding = {binding}) uniform texture2D hz_tex_{name};\n\
                         layout(set = {set}, binding = {}) uniform sampler hz_smp_{name};\n\
                         #define {name} sampler2D(hz_tex_{name}, hz_smp_{name})\n",
                            binding + 1
                        ),
                    );
                }
                Declared::Other(decl) => {
                    if stage == ShaderStage::Compute && matches!(decl.storage, "in" | "out") {
                        return Err(at(format!(
                            "{} {} is not allowed in a compute shader, which has no stage interface",
                            decl.storage, decl.name
                        )));
                    }
                    let qualifiers = decl.qualifiers.join(" ");
                    match decl.storage {
                        "uniform" => {
                            // A loose declaration of a name a block declares (an older version
                            // kept in an `#else` branch): the preprocessor picks the branch
                            // later, but this pass sees both.
                            if !inputs.loose.contains(&decl.name) {
                                return Err(at(format!("uniform {} outside a block", decl.name)));
                            }
                            write(&mut out, "\n");
                        }
                        "in" if vertex => {
                            if glue.inputs.iter().any(|(name, _)| *name == decl.name) {
                                // Declared by the glue as a global.
                                write(&mut out, "\n");
                            } else {
                                // Another format's input (`UV2` behind `#if !defined(IS_GUI)`):
                                // the preprocessor drops it when it is for another pipeline,
                                // else it reads 0.
                                write(&mut out, &format!("{} {};\n", decl.ty, decl.name));
                            }
                        }
                        "out" if vertex => {
                            // One location each: naga takes scalars and vectors only.
                            let location = decl
                                .location
                                .or_else(|| names.location(decl.name))
                                .unwrap_or_else(|| {
                                    (0..)
                                        .find(|l| {
                                            !explicit.contains(l)
                                                && names.varyings.iter().all(|(_, v)| v != l)
                                        })
                                        .unwrap_or_default()
                                });
                            let varying = (decl.name.to_owned(), location);
                            if !names.varyings.contains(&varying) {
                                names.varyings.push(varying);
                            }
                            write(
                                &mut out,
                                &format!(
                                    "layout(location = {location}) {qualifiers} out {} {};\n",
                                    decl.ty, decl.name
                                ),
                            );
                        }
                        "in" => {
                            let location = decl
                                .location
                                .or_else(|| names.location(decl.name))
                                .ok_or_else(|| {
                                    at(format!(
                                        "input {} isn't written by the vertex shader",
                                        decl.name
                                    ))
                                })?;
                            names.inputs.push((location, out.text().len()));
                            write(
                                &mut out,
                                &format!(
                                    "layout(location = {location}) {qualifiers} in {} {};\n",
                                    decl.ty, decl.name
                                ),
                            );
                        }
                        _ => {
                            // Pipelines with more targets (OIT's, behind `#ifdef OIT`) name
                            // them; the colour this pipeline draws is the one at location 0.
                            let location = decl.location.unwrap_or_else(|| {
                                let at = outputs.iter().position(|o| *o == decl.name);
                                at.unwrap_or_else(|| {
                                    outputs.push(decl.name);
                                    outputs.len() - 1
                                }) as u32
                            });
                            names.outputs.push((location, out.text().len()));
                            write(
                                &mut out,
                                &format!(
                                    "layout(location = {location}) out {} {};\n",
                                    decl.ty, decl.name
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    if let (false, Some(finish)) = (vertex, glue.fragment_finish) {
        let code = &out.text()[pack_start..];
        let edits = main_at(code)
            .map(|at| (at + pack_start..at + pack_start + 4, "hz_main".to_owned()))
            .collect();
        out.edit(edits);
        out.push(&format!("void main() {{\nhz_main();\n{finish}\n}}\n"), None);
    }
    if vertex {
        // The program's `gl_VertexID` and `gl_VertexIndex` both read the glue's expression (a
        // pipeline drawing six vertices per quad turns its own index back into the program's
        // four per quad). Only the program's code is rewritten, so the glue's expression keeps
        // reading the real index.
        let vertex_id = format!("({})", glue.vertex_id);
        let code = &out.text()[pack_start..];
        let mut edits: Vec<(std::ops::Range<usize>, String)> = main_at(code)
            .map(|at| (at..at + 4, "hz_main".to_owned()))
            .chain(ident_at(code, "gl_VertexIndex").map(|r| (r, vertex_id.clone())))
            .chain(ident_at(code, "gl_VertexID").map(|r| (r, vertex_id.clone())))
            .map(|(r, with)| (r.start + pack_start..r.end + pack_start, with))
            .collect();
        edits.sort_by_key(|(r, _)| r.start);
        out.edit(edits);
        out.push(
            &format!(
                "void main() {{\n{}\nhz_main();\n{}\n}}\n",
                glue.setup, glue.finish
            ),
            None,
        );
    }
    Ok(Expanded {
        lines: out,
        files: source.files.clone(),
        version: source.version,
    })
}

/// Where `text` has `from` as a whole identifier.
fn ident_at<'a>(text: &'a str, from: &'a str) -> impl Iterator<Item = std::ops::Range<usize>> + 'a {
    text.match_indices(from)
        .map(|(at, _)| at..at + from.len())
        .filter(|r| {
            !text[..r.start].chars().next_back().is_some_and(is_ident)
                && !text[r.end..].chars().next().is_some_and(is_ident)
        })
}

/// Where the `main` of each `void main(` is, which becomes `hz_main`.
fn main_at(text: &str) -> impl Iterator<Item = usize> + '_ {
    ident_at(text, "main")
        .filter(|r| {
            text[..r.start].trim_end().ends_with("void")
                && text[r.end..].trim_start().starts_with('(')
        })
        .map(|r| r.start)
}

/// Parse and validate one stage, attaching it and the authored position to each error. Parse
/// refusals quote the rewritten line naga read. A block or sampler the pipeline does not
/// provide is refused with its name in [`Error::unprovided`].
fn compile(
    stage: ShaderStage,
    code: &Expanded,
    names: &Names,
    provided: &dyn Fn(&str) -> bool,
    limits: InterfaceLimits,
) -> Result<(naga::Module, naga::valid::ModuleInfo), Vec<Error>> {
    let options = glsl_options(stage, limits);
    let text = code.text();
    let module = Frontend::default().parse(&options, text).map_err(|e| {
        code.errors_in(stage, &e, |kind, line| format!("({}): {kind}", line.trim()))
    })?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map_err(|e| {
            // The innermost span naga gives (an expression inside the function the error names).
            let span = (e.spans().map(|(s, _)| *s))
                .filter(|s| s.to_range().is_some())
                .last();
            let span = span.unwrap_or_default();
            vec![
                code.error_in(span, stage::validation_message(e.into_inner()))
                    .with_stage(stage),
            ]
        })?;
    let entry = info.get_entry_point(0);
    for (handle, global) in module.global_variables.iter() {
        let Some(binding) = global
            .binding
            .as_ref()
            .filter(|b| b.group == 0)
            .map(|b| b.binding)
        else {
            continue;
        };
        if binding >= names.other_base && !entry[handle].is_empty() {
            // The name it was declared with (a block's instance may have none).
            let index = ((binding - names.other_base) / 2) as usize;
            let name = names.other.get(index).map_or("?", String::as_str);
            if provided(name) {
                continue;
            }
            return Err(vec![
                code.error_in(
                    module.global_variables.get_span(handle),
                    format!("{name} is not a block or sampler of this pipeline"),
                )
                .with_unprovided(name)
                .with_stage(stage),
            ]);
        }
    }
    Ok((module, info))
}

/// The locations the entry point reads (`inputs`) or writes.
fn locations(module: &naga::Module, inputs: bool) -> Vec<u32> {
    varyings(module, inputs)
        .into_iter()
        .map(|(l, _)| l)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Blocks, samplers and loose uniforms in a game's shape: the tests' own.
    pub(crate) const BLOCKS: [(&str, u32); 4] = [
        ("Globals", 0),
        ("Projection", 1),
        ("DynamicTransforms", 2),
        ("Fog", 3),
    ];
    pub(crate) const SAMPLERS: [(&str, u32); 3] =
        [("Sampler0", 4), ("Sampler1", 6), ("Sampler2", 8)];
    const LOOSE: [&str; 3] = ["ProjMat", "ModelViewMat", "GameTime"];
    /// The lowest stage interface limits a device has.
    pub(crate) const LIMITS: InterfaceLimits = InterfaceLimits {
        inter_stage_variables: 32,
        vertex_attributes: 32,
    };

    /// The stages' files in the tests.
    pub(crate) const VSH_FILE: &str = "minecraft:shaders/core/test.vsh";
    pub(crate) const FSH_FILE: &str = "minecraft:shaders/core/test.fsh";

    fn includes<'a>(read: &'a dyn Fn(&str) -> Result<Option<String>, String>) -> Includes<'a> {
        Includes {
            read,
            namespace: "minecraft",
            folder: "shaders/include/",
            once: &["moj_import"],
        }
    }

    pub(crate) fn translate_core_at(
        sources: &Stages,
        defines: &[&str],
        read: &dyn Fn(&str) -> Result<Option<String>, String>,
        glue: &Glue,
        provided: &dyn Fn(&str) -> bool,
    ) -> Result<Translated, Vec<Error>> {
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
            defines,
            includes: &includes(read),
        };
        translate(sources, &inputs, glue, provided)
    }

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
            vertex_file: VSH_FILE,
            fragment_file: FSH_FILE,
        };
        translate_core_at(&sources, defines, &|id| Ok(include(id)), glue, provided)
    }

    pub(crate) fn translate_core(
        vertex: &str,
        fragment: &str,
        defines: &[&str],
        include: &dyn Fn(&str) -> Option<String>,
        glue: &Glue,
    ) -> Result<Translated, Vec<Error>> {
        translate_core_with(vertex, fragment, defines, include, glue, &|_| false)
    }

    fn expand_imports(
        text: &str,
        dir: &str,
        read: &dyn Fn(&str) -> Result<Option<String>, String>,
    ) -> Result<String, ImportRefusal> {
        expand_includes(text, VSH_FILE, dir, &includes(read), None)
            .map(|e| e.text().to_owned())
            .map_err(|e| e.refusal)
    }

    fn expand(
        text: &str,
        dir: &str,
        include: &dyn Fn(&str) -> Option<String>,
    ) -> Result<String, ImportRefusal> {
        expand_imports(text, dir, &|id| Ok(include(id)))
    }

    fn expand_core_imports(
        text: &str,
        dir: &str,
        defines: &[&str],
        read: &dyn Fn(&str) -> Result<Option<String>, String>,
    ) -> Result<String, ImportRefusal> {
        expand_includes(text, VSH_FILE, dir, &includes(read), Some(defines))
            .map(|e| e.text().to_owned())
            .map_err(|e| e.refusal)
    }

    const INCLUDES: [(&str, &str); 4] = [
        (
            "minecraft:shaders/include/dynamictransforms.glsl",
            "#ifndef DT\n#define DT\nlayout(std140) uniform DynamicTransforms {\n    mat4 ModelViewMat;\n    mat4 TextureMat;\n    vec4 ColorModulator;\n    vec3 ModelOffset;\n};\n#endif\n",
        ),
        (
            "minecraft:shaders/include/projection.glsl",
            "layout(std140) uniform Projection {\n    mat4 ProjMat;\n};\n",
        ),
        (
            "minecraft:shaders/include/globals.glsl",
            "layout(std140) uniform Globals {\n    ivec3 CameraBlockPos;\n    float GlintAlpha;\n    vec3 CameraOffset;\n    float GameTime;\n    vec2 ScreenSize;\n    int MenuBlurRadius;\n    int UseRgss;\n};\n",
        ),
        (
            "minecraft:shaders/include/fog.glsl",
            "layout(std140) uniform Fog {\n    vec4 FogColor;\n    float FogEnvironmentalStart;\n};\nvec4 apply_fog(vec4 c) { return mix(c, FogColor, FogEnvironmentalStart); }\n",
        ),
    ];

    fn include(id: &str) -> Option<String> {
        INCLUDES
            .iter()
            .find(|(path, _)| *path == id)
            .map(|(_, text)| (*text).to_owned())
    }

    const GLUE: Glue = Glue {
        inputs: &[("Position", "vec3"), ("Color", "vec4"), ("UV0", "vec2")],
        prelude: "layout(location = 0) in vec2 hz_pos;\nlayout(location = 1) in vec2 hz_uv;\nlayout(location = 2) in vec4 hz_color;\n\
                  const int hz_corner[6] = int[6](0, 3, 2, 0, 2, 1);\n\
                  int hz_vertex_id() { int i = int(gl_VertexIndex) - int(gl_InstanceIndex); return i / 6 * 4 + hz_corner[i % 6]; }",
        setup: "Position = vec3(hz_pos, 0.0); Color = hz_color; UV0 = hz_uv;",
        finish: "gl_Position.z = 0.5 * gl_Position.w;",
        vertex_id: "hz_vertex_id()",
        fragment_finish: None,
        interface: LIMITS,
    };

    /// 26.3's `text.vsh` and `text.fsh` in shape: `#include`, explicit locations, an extension.
    const VANILLA_VSH: &str = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        #if !defined(IS_GUI)\n#include <minecraft:fog.glsl>\n#endif\n\
        #include <minecraft:dynamictransforms.glsl>\n#include <minecraft:projection.glsl>\n\
        layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\nlayout(location = 2) in vec2 UV0;\n\
        layout(location = 2) out vec4 vertexColor;\nlayout(location = 3) out vec2 texCoord0;\n\
        void main() {\n    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);\n    vertexColor = Color;\n    texCoord0 = UV0;\n}\n";
    const VANILLA_FSH: &str = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        #include <minecraft:dynamictransforms.glsl>\n\
        uniform sampler2D Sampler0;\n\
        layout(location = 2) in vec4 vertexColor;\nlayout(location = 3) in vec2 texCoord0;\n\
        layout(location = 0) out vec4 fragColor;\n\
        void main() {\n    vec4 color = texture(Sampler0, texCoord0) * vertexColor * ColorModulator;\n    if (color.a < 0.1) {\n        discard;\n    }\n    fragColor = color;\n}\n";

    #[test]
    fn vanilla_26_3_text_shaders_translate() {
        let program =
            translate_core(VANILLA_VSH, VANILLA_FSH, &["IS_GUI"], &include, &GLUE).unwrap();
        assert_eq!(program.vertex.entry_points.len(), 1);
        assert_eq!(program.fragment.entry_points.len(), 1);
    }

    /// Whether `function`'s body calls the function named `callee`.
    fn calls(module: &naga::Module, function: &naga::Function, callee: &str) -> bool {
        function.body.iter().any(|s| {
            matches!(s, naga::Statement::Call { function, .. }
                if module.functions[*function].name.as_deref() == Some(callee))
        })
    }

    /// The fragment stage's GLSL `main` (naga's entry point calls it).
    fn glsl_main(module: &naga::Module) -> &naga::Function {
        let (_, main) = (module.functions.iter())
            .find(|(_, f)| f.name.as_deref() == Some("main"))
            .expect("main");
        main
    }

    /// `fragment_finish` runs in the fragment stage's `main` after the program's `main`, renamed
    /// `hz_main` (whose `discard` stays its own).
    #[test]
    fn a_fragment_finish_runs_after_the_programs_main() {
        let glue = Glue {
            fragment_finish: Some("fragColor = fragColor.bgra;"),
            ..GLUE
        };
        let program = translate_core(VANILLA_VSH, VANILLA_FSH, &["IS_GUI"], &include, &glue)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let fragment = &program.fragment;
        let main = glsl_main(fragment);
        let body: Vec<&naga::Statement> = main.body.iter().collect();
        let call = (body.iter())
            .position(|s| {
                matches!(s, naga::Statement::Call { function, .. }
                if fragment.functions[*function].name.as_deref() == Some("hz_main"))
            })
            .unwrap_or_else(|| panic!("main calls hz_main: {:#?}", main.body));
        let bgra = [
            naga::SwizzleComponent::Z,
            naga::SwizzleComponent::Y,
            naga::SwizzleComponent::X,
            naga::SwizzleComponent::W,
        ];
        let swizzled = body[call..].iter().any(|s| {
            matches!(s, naga::Statement::Store { value, .. }
                if matches!(main.expressions[*value], naga::Expression::Swizzle { pattern, .. } if pattern == bgra))
        });
        assert!(swizzled, "{:#?}", main.body);
        let (_, program_main) = (fragment.functions.iter())
            .find(|(_, f)| f.name.as_deref() == Some("hz_main"))
            .expect("the program's main, renamed");
        assert!(
            (program_main.body.iter()).any(|s| matches!(s, naga::Statement::If { .. })),
            "the program's discard stays in its main"
        );
    }

    /// Without `fragment_finish` the fragment stage's `main` is the program's, as written.
    #[test]
    fn no_fragment_finish_leaves_the_fragment_main_as_written() {
        let program = translate_core(VANILLA_VSH, VANILLA_FSH, &["IS_GUI"], &include, &GLUE)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let fragment = &program.fragment;
        assert!(!calls(fragment, glsl_main(fragment), "hz_main"));
        assert!(
            (fragment.functions.iter()).all(|(_, f)| f.name.as_deref() != Some("hz_main")),
            "no hz_main"
        );
    }

    /// A 26.3 pack's `gl_VertexIndex` means vanilla's vertex number, four per quad. The text
    /// pipeline draws six per glyph, so the pack's code must read the glue's `hz_vertex_id()`
    /// (as `gl_VertexID` does), never the raw index.
    #[test]
    fn a_packs_gl_vertex_index_reads_the_glues_vertex_number() {
        let vsh = "#version 330\n#include <minecraft:dynamictransforms.glsl>\n#include <minecraft:projection.glsl>\n\
            layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\nlayout(location = 2) in vec2 UV0;\n\
            layout(location = 2) out vec4 vertexColor;\nlayout(location = 3) out vec2 texCoord0;\n\
            void main() {\n    int corner = gl_VertexIndex % 4;\n\
                gl_Position = ProjMat * ModelViewMat * vec4(Position + vec3(float(corner), 0.0, 0.0), 1.0);\n\
                vertexColor = Color;\n    texCoord0 = UV0;\n}\n";
        let program = translate_core(vsh, VANILLA_FSH, &["IS_GUI"], &include, &GLUE).unwrap();
        let module = &program.vertex;
        let named = |name: &str| {
            module
                .functions
                .iter()
                .find(|(_, f)| f.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let (vertex_id, _) = named("hz_vertex_id");
        let (_, pack_main) = named("hz_main");
        fn calls(block: &naga::Block, target: naga::Handle<naga::Function>) -> bool {
            block.iter().any(|statement| match statement {
                naga::Statement::Call { function, .. } => *function == target,
                naga::Statement::Block(inner) => calls(inner, target),
                naga::Statement::If { accept, reject, .. } => {
                    calls(accept, target) || calls(reject, target)
                }
                naga::Statement::Loop {
                    body, continuing, ..
                } => calls(body, target) || calls(continuing, target),
                _ => false,
            })
        }
        assert!(calls(&pack_main.body, vertex_id));
    }

    /// A 26.2-style pack shader as a HUD pack writes it: `#moj_import`, no locations, a flat
    /// varying, `gl_VertexID`, `texelFetch` and `textureSize` on the glyph atlas, `GameTime`.
    #[test]
    fn a_hud_pack_text_shader_translates() {
        let vsh = "#version 330\n\
            #moj_import <minecraft:dynamictransforms.glsl>\n#moj_import <minecraft:projection.glsl>\n\
            in vec3 Position;\nin vec4 Color;\nin vec2 UV0;\n\
            out vec4 vertexColor;\nout vec2 texCoord0;\n\
            #ifdef IS_GUI\n#moj_import <minecraft:globals.glsl>\nuniform sampler2D Sampler0; // the glyphs\nflat out vec4 effect;\n#endif\n\
            /* moved\n   glyphs */\n\
            void main() {\n    vec3 p = Position;\n#ifdef IS_GUI\n    int corner = gl_VertexID % 4;\n\
                ivec2 size = textureSize(Sampler0, 0);\n\
                if (ivec4(round(texelFetch(Sampler0, ivec2(UV0 * vec2(size)), 0) * 255.0)) == ivec4(177, 126, 91, 1)) {\n\
                    p.x += float(corner) + ScreenSize.x * GameTime;\n    }\n    effect = vec4(float((ivec3(Color.rgb * 255.0) >> 4) == ivec3(3, 14, 10)));\n#endif\n\
                gl_Position = ProjMat * ModelViewMat * vec4(p, 1.0);\n    vertexColor = Color;\n    texCoord0 = UV0;\n}\n";
        let fsh = "#version 330\n#moj_import <minecraft:dynamictransforms.glsl>\n#moj_import <minecraft:globals.glsl>\n\
            uniform sampler2D Sampler0;\nin vec4 vertexColor;\nin vec2 texCoord0;\nflat in vec4 effect;\nout vec4 fragColor;\n\
            void main() {\n    vec4 color = texture(Sampler0, texCoord0) * vertexColor;\n    if (effect.x > 0.5) {\n        fragColor = vertexColor * ColorModulator;\n        return;\n    }\n\
                if (color.a < 0.1) {\n        discard;\n    }\n    fragColor = color * sin(GameTime);\n}\n";
        translate_core(vsh, fsh, &["IS_GUI"], &include, &GLUE).unwrap();
    }

    #[test]
    fn varyings_meet_at_the_same_locations() {
        let vsh = "#version 330\nin vec3 Position;\nout float b;\nout vec2 m;\nout vec4 a;\n\
            void main() { gl_Position = vec4(Position, 1.0); b = 1.0; m = vec2(1.0); a = vec4(b); }\n";
        let fsh = "#version 330\nin vec4 a;\nin float b;\nout vec4 fragColor;\nvoid main() { fragColor = a * b; }\n";
        let program = translate_core(vsh, fsh, &[], &include, &GLUE).unwrap();
        let mut read = locations(&program.fragment, true);
        read.sort_unstable();
        assert_eq!(read, [0, 2]);
    }

    #[test]
    fn explicit_varying_locations_are_kept() {
        let vsh = "#version 330\nin vec3 Position;\n\
            #ifdef IS_GUI\nlayout(location = 4) out float gui;\n#else\nlayout(location = 4) out float world;\n#endif\n\
            layout(location = 13) out vec4 a;\nout float b;\n\
            void main() { gl_Position = vec4(Position, 1.0); a = vec4(1.0); b = 1.0;\n\
            #ifdef IS_GUI\ngui = 1.0;\n#else\nworld = 1.0;\n#endif\n}\n";
        let fsh = "#version 330\n#ifdef IS_GUI\nlayout(location = 4) in float gui;\n#endif\n\
            layout(location = 13) in vec4 a;\nin float b;\nout vec4 fragColor;\n\
            void main() { fragColor = a * b;\n#ifdef IS_GUI\nfragColor *= gui;\n#endif\n}\n";
        let program = translate_core(vsh, fsh, &["IS_GUI"], &include, &GLUE).unwrap();
        let mut read = locations(&program.fragment, true);
        read.sort_unstable();
        assert_eq!(read, [0, 4, 13]);
    }

    /// A vertex stage writing `written` (a qualified `out` declaration) at location 1, and a
    /// fragment stage reading `read` there through `value`, each translated with a stage that
    /// meets it: the two modules [`link`] is given.
    fn stages(written: &str, read: &str, value: &str) -> (naga::Module, naga::Module) {
        let ty = |declaration: &str| declaration.rsplit(' ').next().unwrap().to_owned();
        let vsh = |written: &str| {
            format!(
                "#version 330\nin vec3 Position;\nlayout(location = 1) {written} tag;\n\
                 layout(location = 2) out vec3 unread;\nvoid main() {{ gl_Position = vec4(Position, 1.0); \
                 tag = {}(1); unread = vec3(1.0); }}\n",
                ty(written)
            )
        };
        let fsh = |read: &str, value: &str| {
            format!(
                "#version 330\nlayout(location = 1) {read} tag;\nout vec4 fragColor;\n\
                 void main() {{ fragColor = vec4({value}); }}\n"
            )
        };
        let plain = "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }\n";
        let vertex = translate_core(&vsh(written), plain, &[], &include, &GLUE).unwrap();
        let meeting = read
            .replacen("in ", "out ", 1)
            .replace("noperspective ", "");
        let meeting = meeting.replace("centroid ", "").replace("sample ", "");
        let fragment =
            translate_core(&vsh(&meeting), &fsh(read, value), &[], &include, &GLUE).unwrap();
        (vertex.vertex, fragment.fragment)
    }

    /// Vanilla's link check (`PipelineBuilder`): a varying the fragment stage reads with another
    /// base type, another number of components, or flat in one stage only, than the vertex stage
    /// writes at its location is refused, named; nothing else about interpolation or sampling is
    /// compared, and a vertex output the fragment stage doesn't read is fine.
    #[test]
    fn the_stages_meet_as_vanillas_link_check_requires() {
        for (written, read, value, refused) in [
            ("out vec2", "in vec2", "tag.x", None),
            ("flat out ivec2", "flat in ivec2", "float(tag.x)", None),
            ("out vec2", "noperspective in vec2", "tag.x", None),
            ("centroid out vec2", "in vec2", "tag.x", None),
            ("out vec2", "sample in vec2", "tag.x", None),
            (
                "flat out ivec2",
                "flat in vec2",
                "tag.x",
                Some(LinkRefusal::BaseType(1)),
            ),
            (
                "flat out uvec2",
                "flat in ivec2",
                "float(tag.x)",
                Some(LinkRefusal::BaseType(1)),
            ),
            (
                "out vec3",
                "in vec2",
                "tag.x",
                Some(LinkRefusal::Components(1)),
            ),
            (
                "out vec2",
                "in float",
                "tag",
                Some(LinkRefusal::Components(1)),
            ),
            (
                "flat out vec2",
                "in vec2",
                "tag.x",
                Some(LinkRefusal::Flat(1)),
            ),
            (
                "out vec2",
                "flat in vec2",
                "tag.x",
                Some(LinkRefusal::Flat(1)),
            ),
        ] {
            let (vertex, fragment) = stages(written, read, value);
            assert_eq!(
                link(&vertex, &fragment).err(),
                refused,
                "{written} / {read}"
            );
        }
        let (vertex, _) = stages("out vec2", "in vec2", "tag.x");
        let (_, fragment) = stages("out vec2", "in vec4", "tag.x");
        assert_eq!(link(&vertex, &fragment), Err(LinkRefusal::Components(1)));
        let fsh = "#version 330\nlayout(location = 3) in vec2 tag;\nout vec4 fragColor;\n\
                   void main() { fragColor = vec4(tag.x); }\n";
        let vsh = "#version 330\nin vec3 Position;\nlayout(location = 3) out vec2 tag;\n\
                   void main() { gl_Position = vec4(Position, 1.0); tag = vec2(1.0); }\n";
        let missing = translate_core(vsh, fsh, &[], &include, &GLUE)
            .unwrap()
            .fragment;
        assert_eq!(link(&vertex, &missing), Err(LinkRefusal::Missing(3)));
    }

    /// The vertex stage's output takes the interpolation and sampling the fragment stage reads
    /// it with, as the device needs them equal: a program whose stages differ only there
    /// translates, its vertex output `noperspective`, `centroid` or `sample` as its fragment
    /// input is, and an output the fragment stage doesn't read keeps its own.
    #[test]
    fn the_vertex_output_takes_the_fragment_inputs_interpolation() {
        let qualifiers = |module: &naga::Module, inputs: bool, location: u32| {
            varyings(module, inputs)
                .into_iter()
                .find(|(l, _)| *l == location)
                .map(|(_, v)| (v.interpolation, v.sampling))
                .unwrap()
        };
        for (written, read) in [
            ("out vec2", "noperspective in vec2"),
            ("centroid out vec2", "in vec2"),
            ("out vec2", "sample in vec2"),
            ("noperspective out vec2", "centroid in vec2"),
        ] {
            let vsh = format!(
                "#version 330\nin vec3 Position;\nlayout(location = 1) {written} tag;\n\
                 layout(location = 2) flat out ivec3 unread;\nvoid main() {{ gl_Position = vec4(Position, 1.0); \
                 tag = vec2(1.0); unread = ivec3(1); }}\n"
            );
            let fsh = format!(
                "#version 330\nlayout(location = 1) {read} tag;\nout vec4 fragColor;\n\
                 void main() {{ fragColor = vec4(tag.x); }}\n"
            );
            let program = translate_core(&vsh, &fsh, &[], &include, &GLUE).unwrap();
            assert_eq!(
                qualifiers(&program.vertex, false, 1),
                qualifiers(&program.fragment, true, 1),
                "{written} / {read}"
            );
            assert_eq!(
                qualifiers(&program.vertex, false, 2).0,
                Some(naga::Interpolation::Flat),
                "{written} / {read}: the unread output"
            );
        }
    }

    /// wgpu's default device takes 16 varyings at locations 0 to 15 and 12 uniform blocks a
    /// stage, where vanilla's device takes more: a program past either is told by the typed note
    /// the pipelines draw natively with (`within_device_limits`).
    #[test]
    fn a_program_past_the_default_device_limits_is_told() {
        let limits = wgpu_types::Limits::default();
        let program = |location: u32, blocks: usize| {
            let declared: String = (0..blocks)
                .map(|i| format!("layout(std140) uniform B{i} {{\n    float v{i};\n}};\n"))
                .collect();
            let sum: String = (0..blocks).map(|i| format!(" + v{i}")).collect();
            let vsh = format!(
                "#version 330\nin vec3 Position;\nlayout(location = {location}) out float tag;\n\
                 void main() {{ gl_Position = vec4(Position, 1.0); tag = 1.0; }}\n"
            );
            let fsh = format!(
                "#version 330\n{declared}layout(location = {location}) in float tag;\nout vec4 fragColor;\n\
                 void main() {{ fragColor = vec4(tag{sum}); }}\n"
            );
            translate_core_with(&vsh, &fsh, &[], &include, &GLUE, &|_| true).unwrap()
        };
        assert_eq!(
            program(15, 12).past_device_limit(&limits, &[wgpu_types::TextureFormat::Rgba8Unorm]),
            None
        );
        assert_eq!(
            program(16, 0).past_device_limit(&limits, &[wgpu_types::TextureFormat::Rgba8Unorm]),
            Some(PastDeviceLimit::Varyings {
                count: 1,
                highest: 16,
                limit: 16
            })
        );
        assert_eq!(
            program(0, 13).past_device_limit(&limits, &[wgpu_types::TextureFormat::Rgba8Unorm]),
            Some(PastDeviceLimit::UniformBlocks {
                stage: wgpu_types::ShaderStages::FRAGMENT,
                blocks: 13,
                limit: 12
            })
        );
    }

    /// The device takes a colour target written with the format's base type and at least as many
    /// components; vanilla's builder checks no output, so a program writing a 4-channel float
    /// target with a `vec3` or a `uvec4` is told by the note it draws natively with, while a
    /// single-channel target takes a `float`.
    #[test]
    fn a_colour_output_the_device_doesnt_take_is_told() {
        let limits = wgpu_types::Limits::default();
        let program = |output: &str, value: &str| {
            let fsh = format!(
                "#version 330\nlayout(location = 0) out {output} fragColor;\nvoid main() {{ fragColor = {value}; }}\n"
            );
            let vsh = "#version 330\nin vec3 Position;\nvoid main() { gl_Position = vec4(Position, 1.0); }\n";
            translate_core(vsh, &fsh, &[], &include, &GLUE).unwrap()
        };
        let rgba = [wgpu_types::TextureFormat::Rgba8Unorm];
        assert_eq!(
            program("vec4", "vec4(1.0)").past_device_limit(&limits, &rgba),
            None
        );
        for (output, value, kind, components) in [
            ("vec3", "vec3(1.0)", naga::ScalarKind::Float, 3),
            ("uvec4", "uvec4(1u)", naga::ScalarKind::Uint, 4),
        ] {
            assert_eq!(
                program(output, value).past_device_limit(&limits, &rgba),
                Some(PastDeviceLimit::ColourOutput {
                    location: 0,
                    kind: Some(kind),
                    components,
                    target: wgpu_types::TextureFormat::Rgba8Unorm
                }),
                "{output}"
            );
        }
        let single = [wgpu_types::TextureFormat::R16Float];
        assert_eq!(
            program("float", "1.0").past_device_limit(&limits, &single),
            None
        );
    }

    #[test]
    fn an_include_is_pasted_every_time_and_a_moj_import_once() {
        let file = |id: &str| {
            (id == "minecraft:shaders/include/g.glsl")
                .then(|| "#ifndef G\n#define G\nconst float g = 1.0;\n#endif\n".to_owned())
        };
        let twice =
            "#ifdef A\n#include <minecraft:g.glsl>\n#else\n#include <minecraft:g.glsl>\n#endif\n";
        let expanded = expand(twice, "minecraft:shaders/core/", &file).unwrap();
        assert_eq!(expanded.matches("const float g").count(), 2);
        let imported = expand(
            &twice.replace("#include", "#moj_import"),
            "minecraft:shaders/core/",
            &file,
        )
        .unwrap();
        assert_eq!(imported.matches("const float g").count(), 1);
    }

    #[test]
    fn a_location_another_branch_gave_the_same_name_isnt_reused() {
        let vsh = "#version 330\nin vec3 Position;\n\
            #ifdef IS_GUI\nlayout(location = 0) out vec4 vertexColor;\n\
            #else\nlayout(location = 1) out vec4 vertexColor;\nlayout(location = 0) out vec4 worldOther;\n#endif\n\
            void main() { gl_Position = vec4(Position, 1.0); vertexColor = vec4(1.0);\n\
            #ifndef IS_GUI\nworldOther = vec4(0.5);\n#endif\n}\n";
        let fsh = "#version 330\n\
            #ifdef IS_GUI\nlayout(location = 0) in vec4 vertexColor;\n\
            #else\nlayout(location = 1) in vec4 vertexColor;\nlayout(location = 0) in vec4 worldOther;\n#endif\n\
            out vec4 fragColor;\nvoid main() { fragColor = vertexColor;\n\
            #ifndef IS_GUI\nfragColor *= worldOther;\n#endif\n}\n";
        let world = translate_core(vsh, fsh, &[], &include, &GLUE).unwrap();
        let mut written = locations(&world.vertex, false);
        written.sort_unstable();
        assert_eq!(written, [0, 1]);
        let mut read = locations(&world.fragment, true);
        read.sort_unstable();
        assert_eq!(read, [0, 1]);
        let gui = translate_core(vsh, fsh, &["IS_GUI"], &include, &GLUE).unwrap();
        assert_eq!(locations(&gui.vertex, false), [0]);
    }

    #[test]
    fn includes_that_multiply_are_refused_once_over_budget() {
        let file = |id: &str| {
            let level: u32 = id
                .strip_prefix("minecraft:shaders/include/l")?
                .strip_suffix(".glsl")?
                .parse()
                .ok()?;
            let next = format!("#include <minecraft:l{}.glsl>\n", level + 1).repeat(4);
            Some(format!(
                "#ifndef L{level}\n#define L{level}\n{}#endif\n",
                if level < 15 { next.as_str() } else { "" }
            ))
        };
        let error = expand(
            "#include <minecraft:l0.glsl>\n",
            "minecraft:shaders/core/",
            &file,
        )
        .unwrap_err();
        assert_eq!(error, ImportRefusal::TooManyPastes);
    }

    /// The bytes scanned are bounded too, not only those kept: a file of imports already pasted
    /// keeps a line break each, and is still read in full every time it is included. Seven
    /// pastes of 1.05 MB fit [`MAX_SCANNED`] (8 MiB), the eighth is refused.
    #[test]
    fn includes_that_keep_little_are_refused_once_scanned_past_the_budget() {
        let seen = "#moj_import <b.glsl>\n".repeat(50_000);
        let file = |id: &str| match id {
            "minecraft:shaders/include/a.glsl" => Some(seen.clone()),
            "minecraft:shaders/include/b.glsl" => Some(String::new()),
            _ => None,
        };
        let main = |n: usize| format!("#moj_import <b.glsl>\n{}", "#include <a.glsl>\n".repeat(n));
        let dir = "minecraft:shaders/core/";
        assert!(expand(&main(7), dir, &file).is_ok());
        assert_eq!(
            expand(&main(8), dir, &file),
            Err(ImportRefusal::TooMuchScanned)
        );
    }

    #[test]
    fn a_define_with_a_value_is_defined_to_it() {
        let vsh = "#version 330\nin vec3 Position;\n\
            void main() { gl_Position = vec4(Position, float(COUNT)); }\n";
        let fsh =
            "#version 330\nout vec4 fragColor;\nvoid main() { fragColor = vec4(COUNT / 4); }\n";
        assert!(translate_core(vsh, fsh, &["COUNT 8"], &include, &GLUE).is_ok());
    }

    #[test]
    fn minified_varying_declarations_are_rewritten() {
        let vsh = "#version 330\nout float sphericalVertexDistance;out float cylindricalVertexDistance;\
            void main() { sphericalVertexDistance = 0.0; cylindricalVertexDistance = 0.0; \
            gl_Position = vec4(0.0); }\n";
        let fsh = "#version 330\nin float sphericalVertexDistance;in float cylindricalVertexDistance;\
            out vec4 fragColor;void main() { fragColor = vec4(sphericalVertexDistance + cylindricalVertexDistance); }\n";
        translate_core(vsh, fsh, &[], &include, &GLUE).unwrap();
    }

    #[test]
    fn legacy_uniforms_next_to_modern_blocks_are_ignored() {
        let vsh = "#version 330\n#include <minecraft:projection.glsl>\n\
            uniform mat4 ProjMat;out vec4 vertexColor;\
            void main() { vertexColor = vec4(1.0); gl_Position = ProjMat[0]; }\n";
        let fsh = "#version 330\nin vec4 vertexColor;out vec4 fragColor;\
            void main() { fragColor = vertexColor; }\n";
        translate_core(vsh, fsh, &[], &include, &GLUE).unwrap();
    }

    /// A block declared on one line, as packs write a one-float uniform, is bound like one
    /// over several lines; the blocks and samplers the caller provides are accepted, in binding
    /// order, and one it doesn't provide is not.
    #[test]
    fn a_block_on_one_line_is_bound_and_provided_blocks_are_accepted() {
        let vsh = "#version 330\nlayout(std140) uniform Day { float daylight; };\nout vec4 vertexColor;\n\
            void main() { vertexColor = vec4(daylight); gl_Position = vec4(0.0); }\n";
        let fsh = "#version 330\nuniform sampler2D Bounds;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = vertexColor * texture(Bounds, vec2(0.5)); }\n";
        let both = |name: &str| name == "Day" || name == "Bounds";
        let program = translate_core_with(vsh, fsh, &[], &include, &GLUE, &both).unwrap();
        assert_eq!(program.other, ["Day", "Bounds"]);
        // The bindings the caller gives them at.
        let bindings = |module: &naga::Module| -> Vec<(Option<String>, u32)> {
            module
                .global_variables
                .iter()
                .filter_map(|(_, g)| Some((g.name.clone(), g.binding.as_ref()?.binding)))
                .collect()
        };
        assert!(
            bindings(&program.vertex)
                .iter()
                .any(|(_, b)| *b == program.other_binding(0)),
            "{:?}",
            bindings(&program.vertex)
        );
        let fragment = bindings(&program.fragment);
        assert!(
            fragment.contains(&(Some("hz_tex_Bounds".into()), program.other_binding(1))),
            "{fragment:?}"
        );
        assert!(
            fragment.contains(&(Some("hz_smp_Bounds".into()), program.other_binding(1) + 1)),
            "{fragment:?}"
        );
        // Each at the line that declares it.
        let error = translate_core(vsh, fsh, &[], &include, &GLUE).unwrap_err();
        assert_eq!(
            error,
            vec![Error {
                file: VSH_FILE.into(),
                line: 2,
                column: 24,
                stage: Some(ShaderStage::Vertex),
                unprovided: Some("Day".into()),
                message: "Day is not a block or sampler of this pipeline".into(),
            }]
        );
        let only_day = |name: &str| name == "Day";
        let error = translate_core_with(vsh, fsh, &[], &include, &GLUE, &only_day).unwrap_err();
        assert_eq!(
            error,
            vec![Error {
                file: FSH_FILE.into(),
                line: 2,
                column: 1,
                stage: Some(ShaderStage::Fragment),
                unprovided: Some("Bounds".into()),
                message: "Bounds is not a block or sampler of this pipeline".into(),
            }]
        );
    }

    fn block_named(line: &str) -> Option<(&str, &str)> {
        block_name(layout_prefix(line)?.1)
    }

    fn storage_of(line: &str) -> Option<StorageBlock<'_>> {
        let (layout, rest, _) = layout_prefix(line)?;
        storage_block(layout, rest)
    }

    /// Only a uniform block's declaration names a block, written over several lines or on one.
    #[test]
    fn block_names_are_read_from_block_declarations_only() {
        let name = |line| block_named(line).map(|(name, _)| name);
        assert_eq!(name("layout(std140) uniform Day {"), Some("Day"));
        assert_eq!(name("uniform Day { float daylight; };"), Some("Day"));
        assert_eq!(name("uniform Foo{"), Some("Foo"));
        assert_eq!(name("uniform Globals"), Some("Globals"));
        assert_eq!(
            block_named("layout(std140) uniform Day {").map(|(_, tail)| tail),
            Some("uniform Day {")
        );
        assert_eq!(block_named("uniform float x;"), None);
        // Only `layout(...)` may stand before `uniform`, and it must close.
        assert_eq!(block_named("layout(std140) flat uniform Day {"), None);
        assert_eq!(block_named("layout(std140 uniform Day {"), None);
        assert_eq!(block_named("uniform sampler2D Sampler0;"), None);
    }

    /// `uniform` is a keyword only as a whole token: `uniformColor` opens no block, and a block
    /// after code on its line is listed and bound.
    #[test]
    fn uniform_is_a_whole_token_and_a_declaration_after_code_is_read() {
        let kinds = |text| {
            resources(text)
                .iter()
                .map(|r| (r.name, r.kind))
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds("vec4 uniformColor = vec4(1.0);"), []);
        assert_eq!(
            kinds("layout(location = 0) vec4 uniformColor\n    = vec4(1.0);"),
            []
        );
        assert_eq!(
            kinds("void f() { } uniform sampler2D Late;"),
            [("Late", ResourceKind::Sampler2D)]
        );
        assert_eq!(
            kinds("void f() { int x; } uniform Late { vec4 a; };"),
            [("Late", ResourceKind::UniformBlock)]
        );
        // Bound: the sampler gets its texture and sampler globals and `f` stays.
        let fsh = "#version 330\nvoid f() { } uniform sampler2D Late;\nout vec4 fragColor;\n\
            void main() { fragColor = texture(Late, vec2(0.5)); f(); }\n";
        let vsh = "#version 330\nvoid main() { gl_Position = vec4(0.0); }\n";
        translate_core_with(vsh, fsh, &[], &include, &GLUE, &|name| name == "Late").unwrap();
    }

    /// The `in` declarations of a stage, name and type in order, whatever else is on the line.
    #[test]
    fn stage_inputs_lists_every_in_declaration() {
        let text = "in vec3 Position; in vec4 Color;\nuniform float u;\nvoid f() { } in vec2 UV0;\nout vec4 o;\n";
        assert_eq!(
            stage_inputs(text),
            [
                (text.find("Position").unwrap(), "Position", "vec3"),
                (text.find("Color").unwrap(), "Color", "vec4"),
                (text.find("UV0").unwrap(), "UV0", "vec2"),
            ]
        );
    }

    /// Only real GLSL qualifiers may stand before `in`, `out` or `uniform`: a statement that has
    /// another word there (an assignment, a `return`) is no declaration, and each real
    /// qualifier form still is one.
    #[test]
    fn a_statement_with_junk_before_the_storage_word_is_no_declaration() {
        let junk = "x = in vec2 y;\nreturn out vec4 c;\nreturn uniform sampler2D s;\n\
            foo(a) uniform image2D img;\nint k in vec2 z;\n";
        assert_eq!(resources(junk), []);
        assert_eq!(stage_inputs(junk), []);
        let real = "flat in uint a;\nsmooth in vec2 b;\nnoperspective in vec2 c;\n\
            centroid in vec2 d;\nsample in vec2 e;\ninvariant out vec4 f;\n\
            precise out vec4 g;\nlayout(location = 2) flat in int h;\n\
            highp uniform sampler2D S;\nlowp in vec2 i;\nmediump in vec2 j;\n\
            layout(rgba8) coherent volatile restrict writeonly uniform image2D Img;\n";
        assert_eq!(
            stage_inputs(real),
            [
                (real.find("a;").unwrap(), "a", "uint"),
                (real.find("b;").unwrap(), "b", "vec2"),
                (real.find("c;").unwrap(), "c", "vec2"),
                (real.find("d;").unwrap(), "d", "vec2"),
                (real.find("e;").unwrap(), "e", "vec2"),
                (real.find("h;").unwrap(), "h", "int"),
                (real.find("i;").unwrap(), "i", "vec2"),
                (real.find("j;").unwrap(), "j", "vec2"),
            ]
        );
        assert_eq!(
            resources(real),
            [
                Resource {
                    name: "S",
                    kind: ResourceKind::Sampler2D
                },
                Resource {
                    name: "Img",
                    kind: ResourceKind::StorageImage {
                        access: ImageAccess::WriteOnly
                    }
                },
            ]
        );
    }

    /// A preprocessor line is no declaration, whatever words it holds: the classifier lists
    /// nothing from it, and the translator leaves it as it is (a loose `uniform` declaration
    /// of an unknown name would be refused if it read as one).
    #[test]
    fn a_preprocessor_line_is_no_declaration() {
        let text = "#version 330\n#define IN in vec3 X;\n#define U uniform float Nope;\n\
            #define S uniform sampler2D T;\nvoid main() { }\n";
        assert_eq!(resources(text), []);
        assert_eq!(stage_inputs(text), []);
        let read = |_: &str| -> Result<Option<String>, String> { Ok(None) };
        let includes = includes(&read);
        let inputs = Inputs {
            blocks: &BLOCKS,
            samplers: &SAMPLERS,
            extra_set: 1,
            extra_blocks: &[],
            extra_samplers: &[],
            extra_texel_buffers: &[],
            extra_storage_buffers: &[],
            extra_storage_images: &[],
            push_constant_binding: None,
            loose: &LOOSE,
            defines: &[],
            includes: &includes,
        };
        let expanded = expand_includes(text, VSH_FILE, "minecraft:shaders/core/", &includes, None)
            .unwrap_or_else(|e| panic!("{e}"));
        let rewritten = rewrite(
            &expanded,
            ShaderStage::Vertex,
            &inputs,
            &GLUE,
            &mut Names::new(10),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let rewritten = rewritten.text();
        for line in text.lines().skip(1).take(3) {
            assert!(rewritten.contains(&format!("{line}\n")), "{rewritten}");
        }
    }

    /// GLSL takes a precision word after the storage word too (`in highp vec4 X;`): such a
    /// line is a declaration of its type and name, as with the word before it.
    #[test]
    fn a_precision_word_after_the_storage_word_is_a_declaration() {
        let text = "#version 330\nin highp vec4 X;\nuniform lowp sampler2D T;\nvoid main() { }\n";
        assert_eq!(
            stage_inputs(text),
            [(text.find("X;").unwrap(), "X", "vec4")]
        );
        assert_eq!(
            resources(text),
            [Resource {
                name: "T",
                kind: ResourceKind::Sampler2D
            }]
        );
    }

    /// Only a storage buffer's declaration names a storage block, with its memory qualifiers.
    #[test]
    fn storage_blocks_are_read_from_buffer_declarations_only() {
        let block = storage_of("layout(std430) readonly buffer Quads {").unwrap();
        assert_eq!(
            (block.name, block.qualifiers, block.tail),
            ("Quads", vec!["readonly"], "{")
        );
        let block = storage_of("buffer Free { uint slots[]; };").unwrap();
        assert_eq!((block.name, block.qualifiers.len()), ("Free", 0));
        assert_eq!(storage_of("buffer Next").unwrap().tail, "");
        assert_eq!(storage_of("uniform isamplerBuffer Faces;"), None);
        assert_eq!(storage_of("uniform Day {"), None);
        assert_eq!(storage_of("vec4 buffer;"), None);
        assert_eq!(storage_of("uint data[];"), None);
        assert_eq!(
            storage_of("layout(std430, binding = 3) buffer Q {")
                .unwrap()
                .layout,
            "std430, binding = 3"
        );
    }

    fn storage_stages<'a>(vertex: &'a str, fragment: &'a str) -> Stages<'a> {
        Stages {
            vertex_dir: "minecraft:shaders/core/",
            fragment_dir: "minecraft:shaders/core/",
            vertex,
            fragment,
            vertex_file: VSH_FILE,
            fragment_file: FSH_FILE,
        }
    }

    fn translate_storage(vertex: &str) -> Result<Translated, Vec<Error>> {
        let fragment = "#version 330\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            void main() { fragColor = vertexColor; }\n";
        let read = |_: &str| -> Result<Option<String>, String> { Ok(None) };
        let includes = includes(&read);
        let inputs = Inputs {
            blocks: &BLOCKS,
            samplers: &SAMPLERS,
            extra_set: 1,
            extra_blocks: &[],
            extra_samplers: &[],
            extra_texel_buffers: &[],
            extra_storage_buffers: &[("Data", 0, true)],
            extra_storage_images: &[],
            push_constant_binding: None,
            loose: &LOOSE,
            defines: &[],
            includes: &includes,
        };
        translate(&storage_stages(vertex, fragment), &inputs, &GLUE, &|_| {
            false
        })
    }

    /// A storage block is laid out std430 (GLSL 4.60 §7.6.2.2): `vec3` has base alignment 16 and
    /// size 12, so `{ vec3 a; float b; vec3 c[]; }` puts `b` in `a`'s padding at 12 and `c` at 16,
    /// and an array's stride is its element's alignment (16 for `vec3`), not rounded up as std140
    /// rounds a struct.
    #[test]
    fn a_storage_block_is_laid_out_std430() {
        let vsh = "#version 330\nlayout(std430) readonly buffer Data { vec3 a; float b; vec3 c[]; };\n\
            out vec4 vertexColor;\n\
            void main() { vertexColor = vec4(a + c[0], b); gl_Position = vec4(0.0); }\n";
        let program = translate_storage(vsh).unwrap_or_else(|e| panic!("{e:?}"));
        let module = &program.vertex;
        let (_, global) = module
            .global_variables
            .iter()
            .find(|(_, g)| matches!(g.space, naga::AddressSpace::Storage { .. }))
            .expect("the storage block");
        let naga::TypeInner::Struct { members, .. } = &module.types[global.ty].inner else {
            panic!("a storage block is a struct");
        };
        let offsets: Vec<u32> = members.iter().map(|m| m.offset).collect();
        assert_eq!(offsets, [0, 12, 16]);
        let naga::TypeInner::Array { stride, .. } = module.types[members[2].ty].inner else {
            panic!("`c` is an array");
        };
        assert_eq!(stride, 16);
    }

    /// An invalid member name points inside the storage block, not to its rewritten layout.
    #[test]
    fn an_error_in_a_rewritten_storage_block_names_the_member_column() {
        let vsh = "#version 330\nlayout(std430) readonly buffer Data { vec4 in; };\n\
            out vec4 vertexColor;\n\
            void main() { vertexColor = vec4(1.0); gl_Position = vec4(0.0); }\n";
        let errors = translate_storage(vsh).unwrap_err();
        let error = &errors[0];
        assert!(error.message.ends_with("found In"), "{errors:?}");
        assert_eq!(
            (error.file.as_str(), error.line, error.column),
            (VSH_FILE, 2, 44)
        );
    }

    /// A storage block that names another packing is refused, not rewritten to std430.
    #[test]
    fn a_storage_block_naming_another_packing_is_refused() {
        for packing in ["std140", "shared", "packed"] {
            let vsh = format!(
                "#version 330\nlayout({packing}) readonly buffer Data {{ vec4 a[]; }};\n\
                out vec4 vertexColor;\n\
                void main() {{ vertexColor = a[0]; gl_Position = vec4(0.0); }}\n"
            );
            let errors = translate_storage(&vsh).expect_err(packing);
            assert!(!errors.is_empty(), "got: {errors:?}");
        }
    }

    /// The resources of a stage are each declaration `rewrite` binds, whatever else is on its
    /// line and whatever qualifier stands before `uniform`, and nothing else.
    #[test]
    fn resources_lists_each_declaration_the_translator_binds() {
        let text = "\
layout(std140) uniform Day { float daylight; };
layout(push_constant) uniform Push { vec4 offset; };
highp uniform sampler2D Tex;
uniform sampler2D TexA; uniform sampler2D TexB;
uniform sampler2D TexC; uniform Scene { vec4 tint; };
uniform sampler2D TexD; layout(std430) readonly buffer Data { vec4 d[]; };
uniform isamplerBuffer Faces;
uniform usamplerBuffer Ids;
buffer Free { uint slots[]; };
layout(rgba16f) writeonly uniform image2D Out;
layout(r32f) uniform readonly image2D In;
uniform image2D Both;
uniform image3D Volume;
uniform float loose;
uniform sampler2D A, B;
in vec2 uv;
void main() { }
";
        let found: Vec<(&str, ResourceKind)> =
            resources(text).iter().map(|r| (r.name, r.kind)).collect();
        use ResourceKind::*;
        assert_eq!(
            found,
            [
                ("Day", UniformBlock),
                ("Push", PushConstantBlock),
                ("Tex", Sampler2D),
                ("TexA", Sampler2D),
                ("TexB", Sampler2D),
                ("TexC", Sampler2D),
                ("Scene", UniformBlock),
                ("TexD", Sampler2D),
                ("Data", StorageBuffer { read_only: true }),
                ("Faces", TexelBuffer),
                ("Ids", TexelBuffer),
                ("Free", StorageBuffer { read_only: false }),
                (
                    "Out",
                    StorageImage {
                        access: ImageAccess::WriteOnly
                    }
                ),
                (
                    "In",
                    StorageImage {
                        access: ImageAccess::ReadOnly
                    }
                ),
                (
                    "Both",
                    StorageImage {
                        access: ImageAccess::ReadWrite
                    }
                ),
            ]
        );
    }

    /// An array too large to lay out (#4134) is the shader's error at its line, not a panic.
    #[test]
    fn an_array_too_large_to_lay_out_is_an_error() {
        let vsh = "#version 330\nout vec4 vertexColor;\nvoid main() { vertexColor = vec4(1.0); }\n";
        let fsh = "#version 330\nin vec4 vertexColor;\nfloat big[0x7fffffff];\nout vec4 fragColor;\n\
            void main() { fragColor = vertexColor; }\n";
        let error = translate_core(vsh, fsh, &[], &include, &GLUE).unwrap_err();
        assert_eq!((error[0].file.as_str(), error[0].line), (FSH_FILE, 3));
        assert!(
            error[0].message.starts_with("(float big[0x7fffffff];)"),
            "{error:?}"
        );
    }

    /// An output array whose second element would take location 2^32 (#4912) is the shader's
    /// error at its line, not an overflow.
    #[test]
    fn an_output_whose_locations_pass_u32_is_an_error() {
        let vsh = "#version 330\nout vec4 vertexColor;\nvoid main() { vertexColor = vec4(1.0); }\n";
        let fsh = "#version 330\nin vec4 vertexColor;\nout vec4 fragColor;\n\
            layout(location = 4294967295u) out vec4 extra[2];\n\
            void main() { fragColor = vertexColor; extra[0] = vertexColor; extra[1] = vertexColor; }\n";
        let error = translate_core(vsh, fsh, &[], &include, &GLUE).unwrap_err();
        assert_eq!((error[0].file.as_str(), error[0].line), (FSH_FILE, 4));
        assert!(
            error[0]
                .message
                .starts_with("(layout(location = 4294967295u) out vec4 extra[2];)"),
            "{error:?}"
        );
    }

    #[test]
    fn what_sunrise_does_not_provide_is_an_error() {
        let fsh = "#version 330\nin vec4 vertexColor;\nout vec4 fragColor;\nvoid main() { fragColor = vertexColor; }\n";
        // Another format's input is a global the shader may read: not an error.
        let normal = "#version 330\nin vec3 Normal;\nout vec4 vertexColor;\nvoid main() { vertexColor = vec4(Normal, 1.0); }\n";
        translate_core(normal, fsh, &[], &include, &GLUE).unwrap();
        let lightmap = "#version 330\nuniform sampler2D Lightmap;\nout vec4 vertexColor;\nvoid main() { vertexColor = texelFetch(Lightmap, ivec2(0), 0); }\n";
        let error = translate_core(lightmap, fsh, &[], &include, &GLUE).unwrap_err();
        assert_eq!(
            error,
            vec![Error {
                file: VSH_FILE.into(),
                line: 2,
                column: 1,
                stage: Some(ShaderStage::Vertex),
                unprovided: Some("Lightmap".into()),
                message: "Lightmap is not a block or sampler of this pipeline".into(),
            }]
        );
        // Behind a define the pipeline doesn't have, it is no error.
        let oit = "#version 330\n#ifdef OIT\nuniform sampler2D Coeff0;\n#endif\nout vec4 vertexColor;\n\
            void main() {\n#ifdef OIT\n    vertexColor = texelFetch(Coeff0, ivec2(0), 0);\n#endif\n}\n";
        translate_core(oit, fsh, &[], &include, &GLUE).unwrap();
        let missing = "#version 330\n#moj_import <minecraft:nothing.glsl>\n\
            out vec4 vertexColor;\nvoid main() { vertexColor = vec4(1.0); }\n";
        // At the line that imports it.
        let error = translate_core(missing, fsh, &[], &include, &GLUE).unwrap_err();
        assert_eq!(
            error,
            vec![Error {
                file: VSH_FILE.into(),
                line: 2,
                column: 0,
                stage: Some(ShaderStage::Vertex),
                unprovided: None,
                message: "minecraft:shaders/include/nothing.glsl not found".into(),
            }]
        );
        // An include the defines drop is never read, as shaderc preprocesses: missing, it is no
        // error; kept by a define, it is.
        let branch = "#version 330\n#ifdef WANTS\n#moj_import <minecraft:nothing.glsl>\n#endif\n\
            out vec4 vertexColor;\nvoid main() { vertexColor = vec4(1.0); }\n";
        translate_core(branch, fsh, &[], &include, &GLUE).unwrap();
        let error = translate_core(branch, fsh, &["WANTS"], &include, &GLUE).unwrap_err();
        assert_eq!(
            error,
            vec![Error {
                file: VSH_FILE.into(),
                line: 3,
                column: 0,
                stage: Some(ShaderStage::Vertex),
                unprovided: None,
                message: "minecraft:shaders/include/nothing.glsl not found".into(),
            }]
        );
        let active_after_skipped =
            branch.replace("#endif", "#endif\n#moj_import <minecraft:nothing.glsl>");
        let error = translate_core(&active_after_skipped, fsh, &[], &include, &GLUE).unwrap_err();
        assert_eq!(
            error,
            vec![Error {
                file: VSH_FILE.into(),
                line: 5,
                column: 0,
                stage: Some(ShaderStage::Vertex),
                unprovided: None,
                message: "minecraft:shaders/include/nothing.glsl not found".into(),
            }]
        );
        let unreadable = |id: &str| -> Result<Option<String>, String> {
            if id.ends_with("nothing.glsl") {
                Err(format!("{id}: not text"))
            } else {
                Ok(include(id))
            }
        };
        let stages = Stages {
            vertex_dir: "minecraft:shaders/core/",
            fragment_dir: "minecraft:shaders/core/",
            vertex: branch,
            fragment: fsh,
            vertex_file: VSH_FILE,
            fragment_file: FSH_FILE,
        };
        translate_core_at(&stages, &[], &unreadable, &GLUE, &|_| false).unwrap();
        let error =
            translate_core_at(&stages, &["WANTS"], &unreadable, &GLUE, &|_| false).unwrap_err();
        assert_eq!(
            error,
            vec![Error {
                file: VSH_FILE.into(),
                line: 3,
                column: 0,
                stage: Some(ShaderStage::Vertex),
                unprovided: None,
                message: "minecraft:shaders/include/nothing.glsl: not text".into(),
            }]
        );
    }

    #[test]
    fn unconditional_imports_share_their_dependency() {
        let large = " ".repeat(MAX_EXPANDED / 2 + 1);
        let calls = std::cell::Cell::new(0);
        let read = |id: &str| {
            if id.ends_with("/shared") {
                calls.set(calls.get() + 1);
                Ok(Some(large.clone()))
            } else {
                Ok(Some("#moj_import <shared>".to_owned()))
            }
        };
        expand_core_imports("#moj_import <left>\n#moj_import <right>", "", &[], &read).unwrap();
        assert_eq!(calls.get(), 1);
        calls.set(0);
        expand_core_imports(
            "#ifdef ENABLED\n#moj_import <left>\n#moj_import <right>\n#endif",
            "",
            &["ENABLED"],
            &read,
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
        let repeated = expand_core_imports("#include <shared>\n#include <shared>", "", &[], &read);
        assert_eq!(repeated.unwrap_err(), ImportRefusal::TooLarge);
    }

    #[test]
    fn includes_follow_glsl_line_numbers_and_macros_from_the_caller_and_child() {
        let read = |id: &str| match id.rsplit('/').next() {
            Some("switch") => Ok(Some("#define ENABLE(x) ((x) + 2)".into())),
            Some("active") => Ok(Some("float kept;".into())),
            _ => panic!("read skipped include {id}"),
        };
        let source = "#version 330\n#if __LINE__ == 2\n#include <switch>\n#endif\n#if __LINE__ == 5 && ENABLE(3) == 5\n#include <active>\n#else\n#include <skipped>\n#endif";
        assert!(
            expand_core_imports(source, "", &[], &read)
                .unwrap()
                .contains("float kept;")
        );
    }

    #[test]
    fn line_directives_and_continuations_keep_the_callers_include_location() {
        let source = "#line 40\n#if defined(__LINE__) && \\\n__LINE__ == 41\n#include <inner>\n#endif\n#if __LINE__ == 46\n#include <wrong>\n#endif";
        let read = |id: &str| {
            assert!(id.ends_with("/inner"), "{id}");
            Ok(Some("#if __LINE__ == 1\nfloat included;\n#endif".into()))
        };
        assert!(
            expand_core_imports(source, "", &[], &read)
                .unwrap()
                .contains("float included;")
        );
    }

    #[test]
    fn independent_guards_do_not_replay_unrelated_definitions() {
        let source = format!(
            "#define UNUSED {}\n{}",
            "a".repeat(50_000),
            "#if 1\nfloat a;\n#endif\n".repeat(210)
        );
        assert_eq!(
            expand_core_imports(&source, "", &[], &|_| Ok(None))
                .unwrap()
                .matches("float a;")
                .count(),
            210
        );
    }

    #[test]
    fn only_an_active_include_consumes_a_legacy_import() {
        let fragment = "out vec4 fragColor;\nvoid main() { fragColor = vec4(1.0); }";
        let include = |_: &str| Some("float imported() { return 1.0; }".to_owned());
        let siblings = "#if 0\n#moj_import <a>\n#elif 0\n#moj_import <a>\n#else\n#moj_import <a>\n#endif\nvoid main() { gl_Position = vec4(imported()); }";
        translate_core(siblings, fragment, &[], &include, &GLUE).unwrap();
        for first in ["#include", "#moj_import"] {
            let vertex = format!(
                "#define hz_moj_import_0\n#if 0\n#moj_import <a>\n#endif\n{first} <a>\n#moj_import <a>\nvoid main() {{ gl_Position = vec4(imported()); }}"
            );
            translate_core(&vertex, fragment, &[], &include, &GLUE).unwrap();
        }
    }

    #[test]
    fn includes_resolve_by_id_or_from_the_including_folder() {
        assert_eq!(
            include_id(
                "<fog.glsl>",
                "minecraft:shaders/core/",
                &includes(&|_| Ok(None))
            )
            .as_deref(),
            Some("minecraft:shaders/include/fog.glsl")
        );
        assert_eq!(
            include_id(
                "<bitmade:hud/lanes.glsl>",
                "minecraft:shaders/core/",
                &includes(&|_| Ok(None))
            )
            .as_deref(),
            Some("bitmade:shaders/include/hud/lanes.glsl")
        );
        assert_eq!(
            include_id(
                "\"../include/fog.glsl\"",
                "minecraft:shaders/core/",
                &includes(&|_| Ok(None))
            )
            .as_deref(),
            Some("minecraft:shaders/include/fog.glsl")
        );
    }

    #[test]
    fn translate_supports_caller_high_bindings_and_custom_includes() {
        let read = |id: &str| -> Result<Option<String>, String> {
            match id {
                "demo:custom/inc/scale.glsl" => {
                    Ok(Some("float scale_it(float v) { return v * 0.5; }\n".into()))
                }
                _ => Ok(None),
            }
        };
        let custom_includes = Includes {
            read: &read,
            namespace: "demo",
            folder: "custom/inc/",
            once: &[],
        };
        let inputs = Inputs {
            blocks: &[("Camera", 17), ("Params", 24)],
            samplers: &[("Albedo", 15)],
            extra_set: 0,
            extra_blocks: &[],
            extra_samplers: &[],
            extra_texel_buffers: &[],
            extra_storage_buffers: &[],
            extra_storage_images: &[],
            push_constant_binding: None,
            loose: &[],
            defines: &[],
            includes: &custom_includes,
        };
        let vsh = "#version 330\n\
            #include <scale.glsl>\n\
            layout(std140) uniform Camera { mat4 ViewProj; };\n\
            layout(std140) uniform Extra { float extra_v; };\n\
            out vec2 uv;\n\
            void main() {\n\
                uv = vec2(scale_it(extra_v), 0.5);\n\
                gl_Position = ViewProj * vec4(0.0, 0.0, 0.0, 1.0);\n\
            }\n";
        let fsh = "#version 330\n\
            layout(std140) uniform Params { vec4 Tint; };\n\
            uniform sampler2D Albedo;\n\
            uniform sampler2D ExtraTex;\n\
            in vec2 uv;\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = Tint * texture(Albedo, uv) * texture(ExtraTex, uv);\n\
            }\n";
        let stages = Stages {
            vertex_dir: "demo:custom/core/",
            fragment_dir: "demo:custom/core/",
            vertex: vsh,
            fragment: fsh,
            vertex_file: "demo:custom/core/test.vsh",
            fragment_file: "demo:custom/core/test.fsh",
        };
        let provided = |name: &str| name == "Extra" || name == "ExtraTex";
        let program = translate(&stages, &inputs, &GLUE, &provided)
            .expect("caller bindings >= 15 and provided others must translate");
        assert_eq!(program.other, ["Extra", "ExtraTex"]);
        // Highest caller binding is Params at 24 (Albedo uses 15 and 16, Camera uses 17),
        // so `other` starts at 25 without colliding with any caller binding.
        assert_eq!(program.other_binding(0), 25);
        assert_eq!(program.other_binding(1), 27);
        assert_eq!(
            program.stages_reading(15),
            wgpu_types::ShaderStages::FRAGMENT
        );
        assert_eq!(
            program.stages_reading(16),
            wgpu_types::ShaderStages::FRAGMENT
        );
        assert_eq!(program.stages_reading(17), wgpu_types::ShaderStages::VERTEX);
        assert_eq!(
            program.stages_reading(24),
            wgpu_types::ShaderStages::FRAGMENT
        );
        assert_eq!(program.stages_reading(25), wgpu_types::ShaderStages::VERTEX);
        assert_eq!(
            program.stages_reading(27),
            wgpu_types::ShaderStages::FRAGMENT
        );

        // An unprovided extra block is still refused by its real name.
        let err = translate(&stages, &inputs, &GLUE, &|_| false).unwrap_err();
        assert_eq!(
            err,
            vec![Error {
                file: "demo:custom/core/test.vsh".into(),
                line: 4,
                column: 24,
                stage: Some(ShaderStage::Vertex),
                unprovided: Some("Extra".into()),
                message: "Extra is not a block or sampler of this pipeline".into(),
            }]
        );

        // With `once: &[]`, `#moj_import` is not treated as an include directive.
        let untouched = expand_includes(
            "#moj_import <scale.glsl>\nfloat x;\n",
            "demo:custom/core/test.vsh",
            "demo:custom/core/",
            &custom_includes,
            None,
        )
        .unwrap();
        assert!(
            untouched.text().contains("#moj_import <scale.glsl>"),
            "{}",
            untouched.text()
        );
    }

    #[test]
    fn set1_extra_bindings_and_push_constants_bind_into_group_1() {
        let read = |_: &str| -> Result<Option<String>, String> { Ok(None) };
        let inc = Includes {
            read: &read,
            namespace: "minecraft",
            folder: "shaders/include/",
            once: &[],
        };
        let inputs = Inputs {
            blocks: &BLOCKS,
            samplers: &[],
            extra_set: 1,
            extra_blocks: &[("ModParams", 0)],
            extra_samplers: &[("ModTex", 1)],
            extra_texel_buffers: &[("ModLut", 3)],
            extra_storage_buffers: &[],
            extra_storage_images: &[],
            push_constant_binding: Some(4),
            loose: &[],
            defines: &[],
            includes: &inc,
        };
        let vsh = "#version 330\n\
            layout(std140) uniform Projection { mat4 ProjMat; };\n\
            layout(push_constant) uniform PushData { vec4 Offset; };\n\
            uniform isamplerBuffer ModLut;\n\
            out vec2 uv;\n\
            void main() {\n\
                int idx = texelFetch(ModLut, 0).r;\n\
                uv = vec2(float(idx), 0.5);\n\
                gl_Position = ProjMat * Offset;\n\
            }\n";
        let fsh = "#version 330\n\
            layout(std140) uniform ModParams { vec4 Tint; };\n\
            uniform sampler2D ModTex;\n\
            in vec2 uv;\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = Tint * texture(ModTex, uv);\n\
            }\n";
        let stages = Stages {
            vertex_dir: "demo:shaders/core/",
            fragment_dir: "demo:shaders/core/",
            vertex: vsh,
            fragment: fsh,
            vertex_file: VSH_FILE,
            fragment_file: FSH_FILE,
        };
        let glue = Glue {
            inputs: &[],
            prelude: "",
            setup: "",
            finish: "",
            vertex_id: "int(gl_VertexIndex)",
            fragment_finish: None,
            interface: LIMITS,
        };
        let translated = translate(&stages, &inputs, &glue, &|_| false)
            .expect("set-1 bindings and push_constant must translate");
        assert!(translated.other.is_empty());
        assert_eq!(
            translated.stages_reading_in_group(1, 0),
            wgpu_types::ShaderStages::FRAGMENT
        );
        assert_eq!(
            translated.stages_reading_in_group(1, 1),
            wgpu_types::ShaderStages::FRAGMENT
        );
        assert_eq!(
            translated.stages_reading_in_group(1, 2),
            wgpu_types::ShaderStages::FRAGMENT
        );
        assert_eq!(
            translated.stages_reading_in_group(1, 3),
            wgpu_types::ShaderStages::VERTEX
        );
        assert_eq!(
            translated.stages_reading_in_group(1, 4),
            wgpu_types::ShaderStages::VERTEX
        );
        assert_eq!(translated.uniform_block_size(1, 4), Some(16));

        // Without push_constant_binding, layout(push_constant) is refused.
        let no_pc = Inputs {
            push_constant_binding: None,
            ..inputs
        };
        let err = translate(&stages, &no_pc, &glue, &|_| false)
            .expect_err("undeclared push_constant must fail");
        assert!(describe(&err).contains("push_constant"), "got: {err:?}");
    }

    fn compute_uses_subgroup(source: &str) -> bool {
        let read = |_: &str| -> Result<Option<String>, String> { Ok(None) };
        let includes = includes(&read);
        let inputs = Inputs {
            blocks: &[],
            samplers: &[],
            extra_set: 0,
            extra_blocks: &[],
            extra_samplers: &[],
            extra_texel_buffers: &[],
            extra_storage_buffers: &[("Out", 0, false)],
            extra_storage_images: &[],
            push_constant_binding: None,
            loose: &[],
            defines: &[],
            includes: &includes,
        };
        translate_compute(source, "t:test.csh", "", &inputs, LIMITS)
            .unwrap_or_else(|e| panic!("{e:?}"))
            .uses_subgroup
    }

    /// Subgroup use is read from the translated module, not the source text: an identifier that
    /// merely starts with `subgroup` is not an operation.
    #[test]
    fn only_a_real_subgroup_operation_sets_uses_subgroup() {
        let body = "layout(local_size_x = 8) in;\nlayout(std430) buffer Out { uint values[]; };\n";
        let named = format!(
            "#version 450\n{body}void main() {{ uint subgroupCount = 3u; values[gl_GlobalInvocationID.x] = subgroupCount; }}\n"
        );
        assert!(!compute_uses_subgroup(&named));
        let built_in = format!(
            "#version 450\n#extension GL_KHR_shader_subgroup_basic : enable\n{body}void main() {{ values[gl_GlobalInvocationID.x] = gl_SubgroupSize; }}\n"
        );
        assert!(compute_uses_subgroup(&built_in));
    }
}

#[cfg(test)]
#[path = "texel_tests.rs"]
mod texel_buffer_tests;

#[cfg(test)]
#[path = "located_tests.rs"]
mod located_program_tests;

#[cfg(test)]
mod directives_tests;
