//! A pack core shader pipeline's blocks: vanilla's four bound only to the stages that read them,
//! and its Polytone blocks after the pipeline's own: `PolyGlobals` (wiki "Extra Shader
//! Uniforms", `sunrise_polytone::globals`), the frame's, and its expression uniforms
//! (`shader_modifiers`, wiki "Expression Uniforms on Any Shader"), resolved when the shader
//! loaded (`polytone_post::ShaderModifiers::resolve`), each a block of one float holding its
//! program's value this frame. The lightmap, sky, stars and block outline pipelines lay theirs out
//! with this, and the text pipelines too, through
//! [`bindings`] and [`sampler_bindings`]; any other pack core shader pipeline can.

use crate::polytone_post::ExpressionUniform;
use sunrise_glsl::Translated;

/// The block outline's shader, as a pack's `shader_modifiers` names it.
pub const LINES: [&str; 1] = ["minecraft:core/rendertype_lines"];
/// The sky disc's and the stars' shaders.
pub const SKY: [&str; 1] = ["minecraft:core/sky"];
pub const STARS: [&str; 1] = ["minecraft:core/stars"];
/// The text's shader, one for the GUI's, the world's, the see-through text and Improved
/// Transparency's stages (`polytone/shader_modifiers/core/text.json`). Each of those pipelines
/// binds its blocks with [`bindings`] and its samplers with [`sampler_bindings`].
pub(crate) const TEXT: [&str; 1] = ["minecraft:core/text"];

/// An expression uniform's block: one float, as std140 rounds a block.
const BLOCK: u64 = 16;

/// Each vanilla uniform block's offset in a pipeline's buffer (`core_shader::BLOCKS` order).
pub(crate) const VANILLA_OFFSETS: [u64; 4] = [0, 256, 512, 768];
pub(crate) const VANILLA_SIZES: [u64; 4] = [48, 64, 160, 48];

/// A pipeline's uniform blocks in `buffer`: `blocks` (binding, offset, size and the stages
/// reading it: vanilla's, [`vanilla_blocks`], and any of the pipeline's own), then `extra`'s
/// expression uniforms. Their layout entries, each visible to the stages that read it, and their
/// bind group entries.
pub(crate) fn bindings<'a>(
    blocks: &[(u32, u64, u64, wgpu::ShaderStages)],
    extra: &'a ExpressionBlocks,
    buffer: &'a wgpu::Buffer,
) -> (
    Vec<wgpu::BindGroupLayoutEntry>,
    Vec<wgpu::BindGroupEntry<'a>>,
) {
    let layout = (blocks.iter())
        .map(|&(binding, _, size, stages)| block_entry(binding, size, stages))
        .chain(extra.layout())
        .collect();
    let group = (blocks.iter())
        .map(|&(binding, offset, size, _)| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer,
                offset,
                size: wgpu::BufferSize::new(size),
            }),
        })
        .chain(extra.entries(buffer))
        .collect();
    (layout, group)
}

/// The texture and sampler pairs (a texture at `binding`, its sampler at `binding + 1`) of
/// `textures` that a stage of `program` reads, bound only to the stages that read either: their
/// layout entries and their bind group entries.
pub(crate) fn sampler_bindings<'a>(
    program: &Translated,
    textures: &[(u32, &'a wgpu::TextureView, &'a wgpu::Sampler)],
) -> (
    Vec<wgpu::BindGroupLayoutEntry>,
    Vec<wgpu::BindGroupEntry<'a>>,
) {
    let (mut layout, mut group) = (Vec::new(), Vec::new());
    for &(binding, view, sampler) in textures {
        let stages = program.stages_reading(binding) | program.stages_reading(binding + 1);
        if stages.is_empty() {
            continue;
        }
        layout.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: stages,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });
        layout.push(wgpu::BindGroupLayoutEntry {
            binding: binding + 1,
            visibility: stages,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        group.push(wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        });
        group.push(wgpu::BindGroupEntry {
            binding: binding + 1,
            resource: wgpu::BindingResource::Sampler(sampler),
        });
    }
    (layout, group)
}

/// The bytes each vanilla block has in a pipeline's buffer: from its [`VANILLA_OFFSETS`] to the
/// next.
pub(crate) const VANILLA_SLOT: u64 = 256;

/// Vanilla's block `index` (`core_shader::BLOCKS`' order) as `program` binds it: vanilla's size,
/// or the size its stages declare when larger (an older vanilla's block with more members, which
/// vanilla's builder takes: it checks a descriptor's name, type and dimension, not a block's
/// size). The bytes past vanilla's members read zero; [`oversized_block`] tells one that doesn't
/// fit its slot.
pub(crate) fn vanilla_block_size(program: &Translated, index: usize) -> u64 {
    block_size(program, index as u32, VANILLA_SIZES[index])
}

/// The block at `binding` as `program` binds it: `size` (what the pass writes), or the size its
/// stages declare when larger, the bytes past the pass's reading zero ([`vanilla_block_size`]).
pub(crate) fn block_size(program: &Translated, binding: u32, size: u64) -> u64 {
    size.max(program.declared_size(binding))
}

/// A vanilla block a program declares larger than its slot ([`VANILLA_SLOT`]): binding it would
/// reach the next block, so the program draws natively and the pack stays, as before such a
/// block failed the load (a whole buffer laid out from the program's sizes would fit it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OversizedBlock {
    pub block: &'static str,
    pub size: u64,
}

impl std::fmt::Display for OversizedBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "its {} block is {} bytes, past the {VANILLA_SLOT} Sunrise binds it in",
            self.block, self.size
        )
    }
}

/// The first of vanilla's blocks, then of the pass's `own` (`LightmapInfo`, `CloudInfo`, each in
/// a slot as wide), that `program` declares larger than its slot.
pub(crate) fn oversized_block(
    program: &Translated,
    own: &[&'static str],
) -> Option<OversizedBlock> {
    let own = own.iter().filter_map(|&block| {
        let i = program.other.iter().position(|n| n == block)?;
        Some((block, program.other_binding(i)))
    });
    (crate::core_shader::BLOCKS.iter().copied())
        .chain(own)
        .find_map(|(block, binding)| {
            let size = program.declared_size(binding);
            (size > VANILLA_SLOT).then_some(OversizedBlock { block, size })
        })
}

/// Vanilla's blocks a stage of `program` reads: binding, offset, size ([`vanilla_block_size`])
/// and the stages reading it.
pub(crate) fn vanilla_blocks(program: &Translated) -> Vec<(u32, u64, u64, wgpu::ShaderStages)> {
    (0..4u32)
        .map(|b| {
            let i = b as usize;
            (
                b,
                VANILLA_OFFSETS[i],
                vanilla_block_size(program, i),
                program.stages_reading(b),
            )
        })
        .filter(|(.., stages)| !stages.is_empty())
        .collect()
}

/// A layout entry for a uniform block at `binding`, visible to `stages`, at least `size` bytes.
pub(crate) fn block_entry(
    binding: u32,
    size: u64,
    stages: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stages,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    }
}

/// Why a shader's `PolyGlobals` declaration is refused ([`check_poly_globals`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlobalsRefusal {
    /// It isn't a uniform block.
    NotBlock,
    /// It declares this many members, past the block's.
    PastLayout(usize),
    /// Its member `declared` stands where the `index`th of `sunrise_polytone::globals::LAYOUT`
    /// should, and isn't of that member's type at its offset.
    Member { declared: String, index: usize },
}

impl std::fmt::Display for GlobalsRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use sunrise_polytone::globals::{BLOCK, LAYOUT};
        match self {
            Self::NotBlock => write!(f, "{BLOCK} is not a uniform block"),
            Self::PastLayout(n) => {
                write!(f, "{BLOCK} declares {n} members, past its {}", LAYOUT.len())
            }
            Self::Member { declared, index } => {
                let (name, kind, at) = LAYOUT[*index];
                write!(
                    f,
                    "{BLOCK}'s member {} ({declared}) must be {name}, a {kind:?} at byte {at}",
                    index + 1
                )
            }
        }
    }
}

/// The one rule for `PolyGlobals` on the core and post paths alike (the game owns its bytes):
/// each declaration of the block at `binding` in either stage is a struct whose members are
/// `sunrise_polytone::globals::LAYOUT` or a leading part of it, each of its type at its std140
/// offset. The refusal names the first member that isn't.
pub(crate) fn check_poly_globals(
    modules: [&naga::Module; 2],
    binding: u32,
) -> Result<(), GlobalsRefusal> {
    use naga::{Scalar, TypeInner, VectorSize};
    use sunrise_polytone::globals::{LAYOUT, Member};
    let kind = |inner: &TypeInner| match *inner {
        TypeInner::Matrix {
            columns: VectorSize::Quad,
            rows: VectorSize::Quad,
            scalar: Scalar::F32,
        } => Some(Member::Mat4),
        TypeInner::Scalar(Scalar::F32) => Some(Member::Float),
        TypeInner::Vector {
            size: VectorSize::Tri,
            scalar: Scalar::I32,
        } => Some(Member::IVec3),
        TypeInner::Vector {
            size: VectorSize::Tri,
            scalar: Scalar::F32,
        } => Some(Member::Vec3),
        _ => None,
    };
    for module in modules {
        let declared = (module.global_variables.iter()).filter(|(_, g)| {
            (g.binding.as_ref()).is_some_and(|b| b.group == 0 && b.binding == binding)
        });
        for (_, global) in declared {
            let TypeInner::Struct { members, .. } = &module.types[global.ty].inner else {
                return Err(GlobalsRefusal::NotBlock);
            };
            if members.len() > LAYOUT.len() {
                return Err(GlobalsRefusal::PastLayout(members.len()));
            }
            for (index, (member, (_, want, at))) in members.iter().zip(LAYOUT).enumerate() {
                let found = kind(&module.types[member.ty].inner);
                if found != Some(want) || member.offset as usize != at {
                    return Err(GlobalsRefusal::Member {
                        declared: member.name.clone().unwrap_or_default(),
                        index,
                    });
                }
            }
        }
    }
    Ok(())
}

/// `PolyGlobals`' std140 size.
const GLOBALS: u64 = sunrise_polytone::globals::BLOCK_SIZE as u64;

/// The frame's `PolyGlobals` bytes (`polytone_post::PolyGlobals::bytes`).
pub type PolyGlobalsBytes = [u8; sunrise_polytone::globals::BLOCK_SIZE];

/// A pipeline's Polytone blocks, laid out in its uniform buffer from `base`, 256 bytes apart (the
/// uniform offset alignment): `PolyGlobals` first when a stage reads it, then the expression
/// uniforms with this frame's values.
#[derive(Debug)]
pub(crate) struct ExpressionBlocks {
    /// `PolyGlobals`' binding, the stages that read it and the frame's bytes (zeros until the
    /// game gives them).
    globals: Option<(u32, wgpu::ShaderStages, PolyGlobalsBytes)>,
    uniforms: Vec<(ExpressionUniform, f32)>,
    base: u64,
}

impl ExpressionBlocks {
    /// The Polytone blocks `program`'s stages read: `PolyGlobals`, when a stage reads it, and
    /// `uniforms` (resolved when it loaded), from `base` in the pipeline's buffer.
    pub(crate) fn new(program: &Translated, uniforms: &[ExpressionUniform], base: u64) -> Self {
        let globals = (program.other.iter())
            .position(|name| name == sunrise_polytone::globals::BLOCK)
            .map(|i| program.other_binding(i))
            .map(|binding| {
                (
                    binding,
                    program.stages_reading(binding),
                    [0; GLOBALS as usize],
                )
            })
            .filter(|(_, stages, _)| !stages.is_empty());
        Self {
            globals,
            uniforms: uniforms.iter().map(|u| (u.clone(), 0.0)).collect(),
            base,
        }
    }

    /// The offset of the `i`th expression uniform's block.
    fn offset(&self, i: usize) -> u64 {
        self.base + 256 * (i as u64 + u64::from(self.globals.is_some()))
    }

    /// The uniform buffer's size with these blocks from `base`: `base` without any.
    pub(crate) fn size(&self) -> u64 {
        self.offset(self.uniforms.len())
    }

    /// Each value and `PolyGlobals` 0, as a frame where no world runs gives them (a menu, a
    /// loading screen).
    pub(crate) fn zero(&mut self) {
        self.set_globals(None);
        for (_, value) in &mut self.uniforms {
            *value = 0.0;
        }
    }

    /// Their layout entries, each visible to the stages that read it.
    pub(crate) fn layout(&self) -> impl Iterator<Item = wgpu::BindGroupLayoutEntry> + '_ {
        let globals = (self.globals.iter())
            .map(|&(binding, stages, _)| block_entry(binding, GLOBALS, stages));
        globals.chain((self.uniforms.iter()).map(|(u, _)| block_entry(u.binding, BLOCK, u.stages)))
    }

    pub(crate) fn entries<'a>(
        &'a self,
        buffer: &'a wgpu::Buffer,
    ) -> impl Iterator<Item = wgpu::BindGroupEntry<'a>> + 'a {
        let entry = move |binding, offset, size| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer,
                offset,
                size: wgpu::BufferSize::new(size),
            }),
        };
        let globals =
            (self.globals.iter()).map(move |&(binding, ..)| entry(binding, self.base, GLOBALS));
        globals.chain(
            (self.uniforms.iter().enumerate())
                .map(move |(i, (u, _))| entry(u.binding, self.offset(i), BLOCK)),
        )
    }

    /// The frame's blocks into `buffer`: `PolyGlobals` when a stage reads them, then each
    /// expression uniform's value, 16 bytes each.
    pub(crate) fn upload(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
    ) {
        if let Some((.., bytes)) = &self.globals {
            uploads.buffer(encoder, buffer, self.base, bytes);
        }
        for (i, (_, value)) in self.uniforms.iter().enumerate() {
            let mut block = [0u8; BLOCK as usize];
            block[..4].copy_from_slice(&value.to_le_bytes());
            uploads.buffer(encoder, buffer, self.offset(i), &block);
        }
    }

    /// Whether a stage reads `PolyGlobals`.
    pub(crate) fn reads_globals(&self) -> bool {
        self.globals.is_some()
    }

    /// The frame's `PolyGlobals` (`polytone_post::PolyGlobals::bytes`) for the stages that read
    /// them; none gives zeros.
    pub(crate) fn set_globals(&mut self, globals: Option<&PolyGlobalsBytes>) {
        if let Some((.., bytes)) = &mut self.globals {
            *bytes = globals.copied().unwrap_or([0; GLOBALS as usize]);
        }
    }

    /// Each value from its program in `frame` (the packs' expression context).
    pub(crate) fn evaluate(&mut self, frame: &crate::environment::Frame) {
        for (uniform, value) in &mut self.uniforms {
            *value = uniform.evaluate(frame);
        }
    }

    /// Whether a stage reads an expression uniform.
    pub(crate) fn has_uniforms(&self) -> bool {
        !self.uniforms.is_empty()
    }

    /// The uniforms and their values, for tests to set and read.
    #[cfg(test)]
    pub(crate) fn values(&mut self) -> &mut [(ExpressionUniform, f32)] {
        &mut self.uniforms
    }
}

/// The packs' core shader pipelines' Polytone blocks, for the game to give their values.
impl super::Renderer {
    /// The installed pack pipelines with their Polytone blocks: the block outline's, the sky's and
    /// the stars', the lightmap's except under fullbright (which keeps the texture white), and
    /// the text's (the world's, the see-through and the GUI's) only where a world runs (`world`;
    /// a frame where none runs writes zeros, [`Self::zero_text_blocks`]). Evaluation does not
    /// depend on which pipelines draw this frame.
    fn polytone_blocks(&mut self, world: bool) -> impl Iterator<Item = &mut ExpressionBlocks> {
        let fullbright = self.fullbright;
        let lightmap = (self.core_lightmap.as_mut())
            .filter(|_| !fullbright)
            .map(|core| &mut core.extra);
        let lines = self.core_lines.as_mut().map(|core| &mut core.extra);
        let sky = (self.core_sky.iter_mut()).flat_map(|core| core.expression_blocks());
        let text = (self.core_text.as_mut().filter(|_| world).into_iter())
            .flat_map(|core| core.expression_blocks())
            .chain(self.hud.text_expression_blocks().filter(|_| world));
        (lightmap.into_iter().chain(lines).chain(sky)).chain(text)
    }

    /// Whether the game needs an expression context for the installed pack pipelines this frame
    /// (`world`: whether a world runs).
    pub fn reads_pack_expressions(&mut self, world: bool) -> bool {
        self.polytone_blocks(world)
            .any(|blocks| blocks.has_uniforms())
    }

    /// Evaluates the installed pack pipelines' expression uniforms from `frame` (`world`: whether
    /// a world runs). The values are uploaded separately as each pipeline prepares its draw.
    pub fn evaluate_pack_expressions(&mut self, frame: &crate::environment::Frame, world: bool) {
        for blocks in self.polytone_blocks(world) {
            blocks.evaluate(frame);
        }
    }

    /// Whether an installed pack core shader reads `PolyGlobals` (`world`: whether a world runs):
    /// only then does the game make them for it ([`Self::set_poly_globals`]).
    pub fn reads_poly_globals(&mut self, world: bool) -> bool {
        self.polytone_blocks(world)
            .any(|blocks| blocks.reads_globals())
    }

    /// Whether the last frame's post effects were given `PolyGlobals`, which they ask for only
    /// when one of their passes reads the block.
    pub fn post_effects_read_poly_globals(&self) -> bool {
        (self.post_effects.as_ref()).is_some_and(|effects| effects.gave_poly_globals())
    }

    /// The frame's `PolyGlobals` (`polytone_post::PolyGlobals::bytes`) for the installed pack core
    /// shaders that read them (`world`: whether a world runs), which their next uploads write;
    /// none gives zeros.
    pub fn set_poly_globals(&mut self, globals: Option<&PolyGlobalsBytes>, world: bool) {
        for blocks in self.polytone_blocks(world) {
            blocks.set_globals(globals);
        }
    }

    /// The text's Polytone blocks are 0 where no world runs (a menu or a loading screen): a frame
    /// of [`super::Frame::title`], whatever the game last gave them.
    pub(super) fn zero_text_blocks(&mut self) {
        let text = (self.core_text.iter_mut()).flat_map(|core| core.expression_blocks());
        for blocks in text.chain(self.hud.text_expression_blocks()) {
            blocks.zero();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resolved uniform `name` at `binding` that the stages `stages` read, giving `value`.
    fn uniform(
        name: &str,
        binding: u32,
        stages: wgpu::ShaderStages,
        value: &str,
    ) -> ExpressionUniform {
        let file = serde_json::json!({ name: value }).to_string();
        let pack = sunrise_assets::resource_source::ResourceSource::indexed(
            [(
                "assets/minecraft/polytone/shader_modifiers/core/sky.json".to_owned(),
                sunrise_assets::resource_source::Resource::bytes(file.into_bytes()),
            )]
            .into_iter()
            .collect(),
        );
        let modifiers = crate::polytone_post::ShaderModifiers::read(&pack, Default::default());
        modifiers.uniform_for_test(name, binding, stages)
    }

    /// A program whose fragment stage reads `Tint`, and `PolyGlobals` when `globals` (which it
    /// otherwise imports without reading).
    fn program(globals: bool) -> Translated {
        let include = |id: &str| {
            sunrise_polytone::globals::is_include(id)
                .then(|| sunrise_polytone::globals::GLSL.to_owned())
        };
        let import = "#moj_import <polytone_globals.glsl>\n";
        let read = if globals {
            " + float(PolyPlayerBlockPos.y)"
        } else {
            ""
        };
        let fsh = format!(
            "#version 330\n{import}layout(std140) uniform Tint {{\n    float value;\n}};\n\
             layout(location = 0) out vec4 fragColor;\n\
             void main() {{\n    fragColor = vec4(value{read});\n}}\n"
        );
        crate::core_shader::translate_core_with(
            "#version 330\nlayout(location = 0) in vec3 Position;\n\
             void main() {\n    gl_Position = vec4(Position, 1.0);\n}\n",
            &fsh,
            &[],
            &include,
            &super::super::core_sky::GLUE,
            &|_| true,
        )
        .expect("the shaders translate")
    }

    /// The blocks bind after the pipeline's own, 256 bytes apart from `base`, each visible to
    /// the stages that read it: `PolyGlobals` first when a stage reads it, then the expression
    /// uniforms, which hold their programs' values.
    #[test]
    fn expression_blocks_lay_out_and_hold_their_values() {
        let (v, f) = (wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT);
        let uniforms = [
            uniform("Tint", 18, f, "0.25"),
            uniform("Fade", 20, v | f, "0.5 * 3"),
        ];
        let mut blocks = ExpressionBlocks::new(&program(false), &uniforms, 1024);
        assert!(!blocks.reads_globals() && blocks.has_uniforms());
        let layout: Vec<(u32, wgpu::ShaderStages, u64)> = blocks
            .layout()
            .map(|e| match e.ty {
                wgpu::BindingType::Buffer {
                    min_binding_size, ..
                } => (e.binding, e.visibility, min_binding_size.unwrap().get()),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(layout, [(18, f, 16), (20, v | f, 16)]);
        assert_eq!(blocks.size(), 1024 + 512);
        blocks.evaluate(&Default::default());
        let values: Vec<f32> = blocks.values().iter().map(|(_, v)| *v).collect();
        assert_eq!(values, [0.25, 1.5]);
        // `PolyGlobals` read: its 176 bytes at `base`, the uniforms after it.
        let reading = program(true);
        let globals = reading.other_binding(
            reading
                .other
                .iter()
                .position(|n| n == "PolyGlobals")
                .unwrap(),
        );
        let with = ExpressionBlocks::new(&reading, &uniforms[..1], 1024);
        assert!(with.reads_globals());
        let layout: Vec<(u32, wgpu::ShaderStages)> =
            with.layout().map(|e| (e.binding, e.visibility)).collect();
        assert_eq!(layout, [(globals, f), (18, f)]);
        assert_eq!(with.size(), 1024 + 512);
        let none = ExpressionBlocks::new(&program(false), &[], 1024);
        assert!(!none.has_uniforms() && !none.reads_globals());
        assert_eq!(none.size(), 1024);
    }

    /// A program whose fragment stage declares `PolyGlobals` with `members` (GLSL lines) and
    /// reads `read`, with `include` answering its imports.
    fn declaring(members: &str, read: &str) -> Translated {
        let fsh = format!(
            "#version 330\nlayout(std140) uniform PolyGlobals {{\n{members}}};\n\
             layout(location = 0) out vec4 fragColor;\n\
             void main() {{\n    fragColor = vec4({read});\n}}\n"
        );
        crate::core_shader::translate_core_with(
            "#version 330\nlayout(location = 0) in vec3 Position;\n\
             void main() {\n    gl_Position = vec4(Position, 1.0);\n}\n",
            &fsh,
            &[],
            &|_| None,
            &super::super::core_sky::GLUE,
            &|_| true,
        )
        .expect("the shaders translate")
    }

    /// `PolyGlobals`' one rule ([`check_poly_globals`]): the built-in declaration and a leading
    /// part of it pass; a member of another type or at another offset, or one past the block's,
    /// is refused by name.
    #[test]
    fn poly_globals_must_be_declared_as_the_game_lays_it_out() {
        let check = |program: &Translated| {
            let i = program
                .other
                .iter()
                .position(|n| n == "PolyGlobals")
                .unwrap();
            check_poly_globals(
                [&program.vertex, &program.fragment],
                program.other_binding(i),
            )
        };
        assert_eq!(check(&program(true)), Ok(()));
        let matrices = "    mat4 PolyProjMat;\n    mat4 PolyModelViewMat;\n";
        let leading = format!("{matrices}    float PolySunAngle;\n");
        assert_eq!(check(&declaring(&leading, "PolySunAngle")), Ok(()));
        assert_eq!(
            check(&declaring(
                &format!("{matrices}    vec4 PolySunAngle;\n"),
                "PolySunAngle.x"
            )),
            Err(GlobalsRefusal::Member {
                declared: "PolySunAngle".into(),
                index: 2
            })
        );
        assert_eq!(
            check(&declaring("    float PolySunAngle;\n", "PolySunAngle")),
            Err(GlobalsRefusal::Member {
                declared: "PolySunAngle".into(),
                index: 0
            })
        );
        let full = "    mat4 PolyProjMat;\n    mat4 PolyModelViewMat;\n    float PolySunAngle;\n\
            float PolyDayTime;\n    float PolyDeltaTime;\n    ivec3 PolyPlayerBlockPos;\n\
            vec3 PolyPlayerOffset;\n";
        assert_eq!(check(&declaring(full, "PolySunAngle")), Ok(()));
        assert_eq!(
            check(&declaring(&format!("{full}    float Extra;\n"), "Extra")),
            Err(GlobalsRefusal::PastLayout(8))
        );
        // A member of the right type at another offset (GLSL can't place one; a module can).
        let moved = {
            let program = declaring(full, "PolySunAngle");
            let i = program
                .other
                .iter()
                .position(|n| n == "PolyGlobals")
                .unwrap();
            let binding = program.other_binding(i);
            let mut fragment = program.fragment;
            let (handle, ty) = (fragment.global_variables.iter())
                .find(|(_, g)| g.binding.as_ref().is_some_and(|b| b.binding == binding))
                .map(|(handle, g)| (handle, g.ty))
                .unwrap();
            let mut block = fragment.types[ty].clone();
            if let naga::TypeInner::Struct { members, .. } = &mut block.inner {
                members[2].offset = 132;
            }
            block.name = Some("Moved".into());
            let block = fragment.types.insert(block, naga::Span::UNDEFINED);
            fragment.global_variables.get_mut(handle).ty = block;
            check_poly_globals([&program.vertex, &fragment], binding)
        };
        assert_eq!(
            moved,
            Err(GlobalsRefusal::Member {
                declared: "PolySunAngle".into(),
                index: 2
            })
        );
        let ivec = full.replace("ivec3 PolyPlayerBlockPos", "vec3 PolyPlayerBlockPos");
        assert_eq!(
            check(&declaring(&ivec, "PolySunAngle")),
            Err(GlobalsRefusal::Member {
                declared: "PolyPlayerBlockPos".into(),
                index: 5
            })
        );
    }

    /// The core shaders' loader refuses, named, a `shader_modifiers` file giving a block the
    /// pipeline fills (`PolyGlobals`, the lightmap's `LightmapInfo`) and a declaration the rule
    /// refuses.
    #[test]
    fn a_core_shader_cannot_take_over_poly_globals() {
        use crate::polytone_post::{ExpressionRefusal, ShaderModifiers};
        let modifiers = |file: &str| {
            let pack = sunrise_assets::resource_source::ResourceSource::indexed(
                [(
                    "assets/minecraft/polytone/shader_modifiers/core/sky.json".to_owned(),
                    sunrise_assets::resource_source::Resource::bytes(file.as_bytes().to_vec()),
                )]
                .into_iter()
                .collect(),
            );
            ShaderModifiers::read(&pack, Default::default())
        };
        let own = [sunrise_polytone::globals::BLOCK];
        let resolve = |file: &str, program: &Translated| {
            modifiers(file)
                .resolve((SKY[0], SKY[0]), program, &own)
                .map(|u| u.len())
        };
        assert_eq!(resolve(r#"{"Tint": "1"}"#, &program(true)), Ok(1));
        assert!(matches!(
            resolve(r#"{"Tint": "1", "PolyGlobals": "1"}"#, &program(true)),
            Err(ExpressionRefusal::Reserved(name)) if name == "PolyGlobals"
        ));
        // Any block the pipeline fills itself: the lightmap's `LightmapInfo` too.
        let lightmap = |file: &str| {
            let pack = sunrise_assets::resource_source::ResourceSource::indexed(
                [(
                    "assets/minecraft/polytone/shader_modifiers/core/lightmap.json".to_owned(),
                    sunrise_assets::resource_source::Resource::bytes(file.as_bytes().to_vec()),
                )]
                .into_iter()
                .collect(),
            );
            let shaders = super::super::core_lightmap::SHADERS;
            let own = [
                super::super::core_lightmap::INFO_BLOCK,
                sunrise_polytone::globals::BLOCK,
            ];
            ShaderModifiers::read(&pack, Default::default())
                .resolve((shaders[0], shaders[1]), &program(true), &own)
                .map(|u| u.len())
        };
        assert!(matches!(
            lightmap(r#"{"LightmapInfo": "1"}"#),
            Err(ExpressionRefusal::Reserved(name)) if name == "LightmapInfo"
        ));
        let wrong = declaring("    float PolySunAngle;\n", "PolySunAngle");
        assert!(matches!(
            resolve("{}", &wrong),
            Err(ExpressionRefusal::Globals(at, GlobalsRefusal::Member {
                index: 0,
                ..
            })) if at == sunrise_glsl::Location {
                file: "minecraft:shaders/core/core.fsh".into(),
                line: 2,
            }
        ));
    }
}
