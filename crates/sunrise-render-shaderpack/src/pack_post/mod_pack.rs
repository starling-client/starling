//! General mod-pipeline integration types for `PackPost`: declarative auxiliary depth targets,
//! typed per-frame uniforms with previous-frame auto-tracking, and pass-slot routing. Contains
//! zero mod-specific names; every identifier comes from the mod's registered `PackBinding`.

use std::collections::HashMap;

use super::types::{Pass, Source};

/// GLSL type of a mod-declared per-frame shader-pack uniform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModUniformType {
    Float,
    Int,
    Vec2,
    Vec3,
    Vec4,
    Mat3,
    Mat4,
}

impl ModUniformType {
    /// Whether a shader pack's GLSL uniform type `glsl_ty` is compatible with this declaration.
    pub fn matches_glsl(self, glsl_ty: &str) -> bool {
        match self {
            Self::Float => glsl_ty == "float",
            Self::Int => matches!(glsl_ty, "int" | "bool"),
            Self::Vec2 => glsl_ty == "vec2",
            Self::Vec3 => glsl_ty == "vec3",
            Self::Vec4 => glsl_ty == "vec4",
            Self::Mat3 => glsl_ty == "mat3",
            Self::Mat4 => glsl_ty == "mat4",
        }
    }

    /// The canonical GLSL type keyword for diagnostics.
    pub const fn glsl_name(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Int => "int",
            Self::Vec2 => "vec2",
            Self::Vec3 => "vec3",
            Self::Vec4 => "vec4",
            Self::Mat3 => "mat3",
            Self::Mat4 => "mat4",
        }
    }
}

/// Per-frame value for a mod-declared shader-pack uniform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModUniformValue {
    Float(f32),
    Int(i32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Mat3([[f32; 3]; 3]),
    Mat4([[f32; 4]; 4]),
}

impl ModUniformValue {
    /// Zero value for `ty`.
    pub const fn zero(ty: ModUniformType) -> Self {
        match ty {
            ModUniformType::Float => Self::Float(0.0),
            ModUniformType::Int => Self::Int(0),
            ModUniformType::Vec2 => Self::Vec2([0.0; 2]),
            ModUniformType::Vec3 => Self::Vec3([0.0; 3]),
            ModUniformType::Vec4 => Self::Vec4([0.0; 4]),
            ModUniformType::Mat3 => Self::Mat3([[0.0; 3]; 3]),
            ModUniformType::Mat4 => Self::Mat4([[0.0; 4]; 4]),
        }
    }

    /// The [`ModUniformType`] matching this value.
    pub const fn ty(self) -> ModUniformType {
        match self {
            Self::Float(_) => ModUniformType::Float,
            Self::Int(_) => ModUniformType::Int,
            Self::Vec2(_) => ModUniformType::Vec2,
            Self::Vec3(_) => ModUniformType::Vec3,
            Self::Vec4(_) => ModUniformType::Vec4,
            Self::Mat3(_) => ModUniformType::Mat3,
            Self::Mat4(_) => ModUniformType::Mat4,
        }
    }

    /// Write this value at `offset` in a std140 uniform buffer block. Out-of-bounds slices are
    /// ignored without panicking.
    pub fn write_std140(self, out: &mut [u8], offset: usize) {
        let put = |out: &mut [u8], at: usize, bytes: &[u8]| {
            if let Some(dst) = out.get_mut(at..at + bytes.len()) {
                dst.copy_from_slice(bytes);
            }
        };
        match self {
            Self::Float(v) => put(out, offset, bytemuck::bytes_of(&v)),
            Self::Int(v) => put(out, offset, bytemuck::bytes_of(&v)),
            Self::Vec2(v) => put(out, offset, bytemuck::cast_slice(&v)),
            Self::Vec3(v) => put(out, offset, bytemuck::cast_slice(&v)),
            Self::Vec4(v) => put(out, offset, bytemuck::cast_slice(&v)),
            Self::Mat3(cols) => {
                // std140 mat3: three vec3 columns, each aligned to a 16-byte stride.
                for (c, col) in cols.iter().enumerate() {
                    put(out, offset + c * 16, bytemuck::cast_slice(col));
                }
            }
            Self::Mat4(cols) => {
                for (c, col) in cols.iter().enumerate() {
                    put(out, offset + c * 16, bytemuck::cast_slice(col));
                }
            }
        }
    }
}

/// A mod-declared auxiliary depth target shared by pipelines referencing its index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModDepthTargetDecl {
    /// Debug label (the mod-namespaced target key).
    pub label: String,
    /// Sampler names exposed to pack passes for full depth.
    pub depth_samplers: Vec<String>,
    /// Sampler names exposed for the post-opaque depth snapshot.
    pub opaque_depth_samplers: Vec<String>,
}

/// A mod-declared per-frame uniform exposed to shader pack programs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModUniformDecl {
    pub name: String,
    pub ty: ModUniformType,
    pub previous_name: Option<String>,
}

/// Pack pass slot where a bound mod pipeline executes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModPassSlot {
    Shadow,
    AfterSky,
    AfterOpaqueGbuffers,
    AfterTranslucentGbuffers,
    AfterWeather,
}

/// First bind group index of a mod's own sets in a pack pass, after the pack's own sets
/// `0..=5` (the frame's, the geometry's, the draws', the block ids', the storage buffers' and
/// the geometry stage's indices). The mod's set goes after the pack's and never into one of
/// them, so the pack's layouts stay as they are.
pub const MOD_SET_BASE: u32 = 6;

/// Which stages of a mod pipeline's pack pass come from the pack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModProgramMode {
    /// The pack's program replaces both stages, its vertex stage reading the mod's vertices.
    #[default]
    PackVertexAndFragment,
    /// The mod's own vertex stage, linked to the pack's fragment stage of the same program.
    /// Needs the pack's fragment inputs to match the mod's vertex outputs, else the slot is
    /// unbound and the mod's own shaders draw.
    ModVertexPackFragment,
}

/// One vertex buffer binding of a mod pipeline's own vertex format.
#[derive(Clone, Debug)]
pub struct ModVertexBuffer {
    pub binding: u32,
    pub stride: u64,
    pub step_mode: wgpu::VertexStepMode,
    pub attributes: Vec<wgpu::VertexAttribute>,
}

/// A mod pipeline's own translated vertex stage for [`ModProgramMode::ModVertexPackFragment`],
/// with the vertex layout it reads and the layouts of the bind groups it declares: its frame
/// uniforms (group 0) and, when declared, its own set 1.
#[derive(Clone, Debug)]
pub struct ModVertexStage {
    pub module: naga::Module,
    pub buffers: Vec<ModVertexBuffer>,
    pub globals_layout: wgpu::BindGroupLayout,
    pub set1_layout: Option<wgpu::BindGroupLayout>,
}

/// Per-pipeline pack-binding metadata supplied to `PackPost::new_with_mods`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModPipelinePackSpec {
    /// Pipeline slot index in the mod pipeline registry.
    pub slot: usize,
    /// Pack pass slot where this pipeline draws when bound.
    pub pass_slot: ModPassSlot,
    /// Optional index into `ModPackRuntime::depth_targets`.
    pub depth_target: Option<usize>,
    /// Declared depth comparison lowered to the pack's standard-Z depth plane.
    pub depth_compare: wgpu::CompareFunction,
    /// Whether the pipeline requested depth writes and its target and slot allow them.
    pub depth_write: bool,
    /// Whether back faces are culled (`PipelineState::cull`).
    pub cull: bool,
    /// Which stages come from the pack.
    pub program_mode: ModProgramMode,
}

/// Mod-pipeline shader-pack declarations handed to `PackPost::new_with_mods`.
#[derive(Clone, Default)]
pub struct ModPackRuntime<'a> {
    pub depth_targets: &'a [ModDepthTargetDecl],
    pub uniforms: &'a [ModUniformDecl],
    pub pipelines: &'a [ModPipelinePackSpec],
    /// Vertex buffer layout for mod pipeline quads when `pipelines` is non-empty.
    pub vertex_buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    /// Optional per-pipeline-slot set-1 `wgpu::BindGroupLayout` for mod pipelines declaring
    /// custom uniforms, samplers, texel buffers, or push constants (`#5428`).
    pub bind_group_layouts: &'a [Option<&'a wgpu::BindGroupLayout>],
    /// Per-pipeline-slot own vertex stage, for pipelines in
    /// [`ModProgramMode::ModVertexPackFragment`].
    pub vertex_stages: &'a [Option<ModVertexStage>],
}

impl<'a> ModPackRuntime<'a> {
    /// This runtime with each slot's own vertex stage, for pipelines in
    /// [`ModProgramMode::ModVertexPackFragment`].
    pub fn with_vertex_stages(mut self, stages: &'a [Option<ModVertexStage>]) -> Self {
        self.vertex_stages = stages;
        self
    }
}

/// Owned, `Send` form of [`ModPackRuntime`] for moving into background shader-pack build threads.
#[derive(Clone, Debug, Default)]
pub struct OwnedModPackRuntime {
    pub depth_targets: Vec<ModDepthTargetDecl>,
    pub uniforms: Vec<ModUniformDecl>,
    pub pipelines: Vec<ModPipelinePackSpec>,
}

impl OwnedModPackRuntime {
    /// Borrow as a [`ModPackRuntime`] paired with `vertex_buffers`.
    pub fn as_runtime<'a>(
        &'a self,
        vertex_buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    ) -> ModPackRuntime<'a> {
        self.as_runtime_with_layouts(vertex_buffers, &[])
    }

    /// Borrow as a [`ModPackRuntime`] paired with `vertex_buffers` and per-slot set-1
    /// `bind_group_layouts`.
    pub fn as_runtime_with_layouts<'a>(
        &'a self,
        vertex_buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
        bind_group_layouts: &'a [Option<&'a wgpu::BindGroupLayout>],
    ) -> ModPackRuntime<'a> {
        ModPackRuntime {
            depth_targets: &self.depth_targets,
            uniforms: &self.uniforms,
            pipelines: &self.pipelines,
            vertex_buffers,
            bind_group_layouts,
            vertex_stages: &[],
        }
    }
}

/// One compiled shader-pack pass bound to a mod pipeline slot.
pub struct ModPackPass {
    pub slot: usize,
    pub pass_slot: ModPassSlot,
    /// Index into `Targets::mod_depth` when the active pack actually samples this target;
    /// `None` uses the pack's main `depthtex0` (or shadow depth in `Shadow`).
    pub depth_target: Option<usize>,
    /// Whether this pass's pipeline layout includes a mod-declared set-1 bind group (`#5428`).
    pub has_mod_bind_group: bool,
    /// Whether the pass runs the mod's own vertex stage ([`ModProgramMode::ModVertexPackFragment`]):
    /// its frame uniforms are then bound at [`MOD_SET_BASE`] and its set 1 at the one after,
    /// instead of its set 1 at index 1.
    pub mod_vertex: bool,
    pub pass: Pass,
}

/// How a [`ModDrawChunk`] issues its GPU draw call inside `PackPost::draw_mod_slot`.
#[derive(Clone, Debug)]
pub enum ModDrawKind {
    /// Non-indexed draw over `vertices`.
    Vertices { vertices: std::ops::Range<u32> },
    /// Indexed draw using `buffer` (`Uint16` or `Uint32`), `indices`, and `base_vertex`.
    Indexed {
        buffer: wgpu::Buffer,
        format: wgpu::IndexFormat,
        indices: std::ops::Range<u32>,
        base_vertex: i32,
    },
    /// `count` GPU-written commands from `offset` of `buffer`: indexed (20 bytes each) with the
    /// index buffer and format in `index`, else non-indexed (16 bytes each). With `count_buffer`
    /// (a buffer and the byte offset of a GPU-written `u32`), `count` is the most commands drawn
    /// and the `u32` is how many are.
    Indirect {
        buffer: wgpu::Buffer,
        offset: u64,
        count: u32,
        count_buffer: Option<(wgpu::Buffer, u64)>,
        index: Option<(wgpu::Buffer, wgpu::IndexFormat)>,
    },
}

/// One vertex-buffer draw chunk submitted for a mod pipeline slot (covers both transient
/// quads/vertices and retained `MeshRef` multi-binding / instanced draws).
#[derive(Clone)]
pub struct ModDrawChunk {
    pub slot: usize,
    /// Bound vertex/instance buffers: `(binding_index, buffer, byte_range)`.
    pub vertex_buffers: Vec<(u32, wgpu::Buffer, std::ops::Range<u64>)>,
    pub kind: ModDrawKind,
    pub instances: std::ops::Range<u32>,
    /// Optional mod-declared set-1 bind group and dynamic offsets (`#5428`).
    pub mod_bind_group: Option<(wgpu::BindGroup, Vec<u32>)>,
    /// The pipeline's frame-uniform group, bound at [`MOD_SET_BASE`] when its pass runs the mod's
    /// own vertex stage.
    pub mod_globals: Option<wgpu::BindGroup>,
}

impl ModDrawChunk {
    /// Convenience constructor for a single-binding quad draw backed by a shared `u32` quad
    /// index buffer (6 indices per quad, 1 instance).
    pub fn quads(
        slot: usize,
        vertices: wgpu::Buffer,
        indices: wgpu::Buffer,
        quads: std::ops::Range<u32>,
    ) -> Self {
        let byte_len = vertices.size();
        Self {
            slot,
            vertex_buffers: vec![(0, vertices, 0..byte_len)],
            kind: ModDrawKind::Indexed {
                buffer: indices,
                format: wgpu::IndexFormat::Uint32,
                indices: quads.start * 6..quads.end * 6,
                base_vertex: 0,
            },
            instances: 0..1,
            mod_bind_group: None,
            mod_globals: None,
        }
    }

    /// Whether this chunk draws zero vertices/indices or zero instances.
    pub fn is_empty(&self) -> bool {
        if self.instances.is_empty() {
            return true;
        }
        match &self.kind {
            ModDrawKind::Vertices { vertices } => vertices.is_empty(),
            ModDrawKind::Indexed { indices, .. } => indices.is_empty(),
            ModDrawKind::Indirect { count, .. } => *count == 0,
        }
    }
}

/// Result of matching a pack program's declared GLSL uniform against mod-declared uniforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModUniformMatch {
    /// Not a mod-declared uniform.
    None,
    /// Name and GLSL type match `decls[index]` (`previous == true` for `previous_name`).
    Bound { index: usize, previous: bool },
    /// Name matches a mod-declared uniform, but the pack's GLSL type is incompatible: leave
    /// unbound and log once, never panic.
    TypeMismatch { expected: ModUniformType },
}

/// Match a pack sampler name against mod-declared auxiliary depth targets.
pub fn mod_depth_source(targets: &[ModDepthTargetDecl], name: &str) -> Option<Source> {
    for (idx, target) in targets.iter().enumerate() {
        if target.depth_samplers.iter().any(|s| s == name) {
            return Some(Source::ModDepth(idx));
        }
        if target.opaque_depth_samplers.iter().any(|s| s == name) {
            return Some(Source::ModOpaqueDepth(idx));
        }
    }
    None
}

/// Match a pack program's declared uniform `(name, glsl_ty)` against `decls`.
pub fn match_mod_uniform(decls: &[ModUniformDecl], name: &str, glsl_ty: &str) -> ModUniformMatch {
    for (index, decl) in decls.iter().enumerate() {
        let matched = if decl.name == name {
            Some(false)
        } else if decl.previous_name.as_deref() == Some(name) {
            Some(true)
        } else {
            None
        };
        if let Some(previous) = matched {
            return if decl.ty.matches_glsl(glsl_ty) {
                ModUniformMatch::Bound { index, previous }
            } else {
                ModUniformMatch::TypeMismatch { expected: decl.ty }
            };
        }
    }
    ModUniformMatch::None
}

struct ModUniformEntry {
    ty: ModUniformType,
    current: ModUniformValue,
    previous: ModUniformValue,
    staged: Option<ModUniformValue>,
    has_history: bool,
}

/// Runtime storage for mod-declared per-frame uniforms, with automatic previous-frame tracking.
#[derive(Default)]
pub struct ModUniformTable {
    entries: Vec<ModUniformEntry>,
    by_name: HashMap<String, usize>,
}

impl ModUniformTable {
    pub fn new(decls: &[ModUniformDecl]) -> Self {
        let mut entries = Vec::with_capacity(decls.len());
        let mut by_name = HashMap::with_capacity(decls.len());
        for (i, decl) in decls.iter().enumerate() {
            let zero = ModUniformValue::zero(decl.ty);
            entries.push(ModUniformEntry {
                ty: decl.ty,
                current: zero,
                previous: zero,
                staged: None,
                has_history: false,
            });
            by_name.insert(decl.name.clone(), i);
        }
        Self { entries, by_name }
    }

    /// Stage this frame's value for `name`. Returns `false` if `name` is unknown or its
    /// [`ModUniformType`] mismatches the declaration; never panics.
    pub fn set(&mut self, name: &str, value: ModUniformValue) -> bool {
        let Some(&idx) = self.by_name.get(name) else {
            return false;
        };
        let entry = &mut self.entries[idx];
        if entry.ty != value.ty() {
            return false;
        }
        entry.staged = Some(value);
        true
    }

    /// Commit staged values for the frame about to render and advance `previous` aliases.
    /// On the first frame (or after [`Self::reset_history`]), `previous` equals `current`.
    pub fn advance_frame(&mut self) {
        for entry in &mut self.entries {
            if entry.has_history {
                entry.previous = entry.current;
            }
            if let Some(next) = entry.staged.take() {
                entry.current = next;
            }
            if !entry.has_history {
                entry.previous = entry.current;
                entry.has_history = true;
            }
        }
    }

    /// Forget prior-frame history so the next frame behaves like a freshly loaded pack's first.
    pub fn reset_history(&mut self) {
        for entry in &mut self.entries {
            let zero = ModUniformValue::zero(entry.ty);
            entry.current = zero;
            entry.previous = zero;
            entry.staged = None;
            entry.has_history = false;
        }
    }

    /// Read the active value for `index` (`previous == true` selects the previous-frame alias).
    pub fn value(&self, index: usize, previous: bool) -> Option<ModUniformValue> {
        let entry = self.entries.get(index)?;
        Some(if previous {
            entry.previous
        } else {
            entry.current
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_uniform_table_tracks_previous_frame_and_resets_history() {
        let decls = vec![
            ModUniformDecl {
                name: "modNear".into(),
                ty: ModUniformType::Float,
                previous_name: Some("modPrevNear".into()),
            },
            ModUniformDecl {
                name: "modProj".into(),
                ty: ModUniformType::Mat4,
                previous_name: Some("modPrevProj".into()),
            },
        ];
        let mut table = ModUniformTable::new(&decls);

        // Wrong type or unknown name is refused cleanly without panicking.
        assert!(!table.set("modNear", ModUniformValue::Int(4)));
        assert!(!table.set("unknownUniform", ModUniformValue::Float(1.0)));

        // Frame 1: previous equals current on first frame.
        assert!(table.set("modNear", ModUniformValue::Float(16.0)));
        table.advance_frame();
        assert_eq!(table.value(0, false), Some(ModUniformValue::Float(16.0)));
        assert_eq!(table.value(0, true), Some(ModUniformValue::Float(16.0)));

        // Frame 2: previous holds Frame 1's value while current holds Frame 2's.
        assert!(table.set("modNear", ModUniformValue::Float(32.0)));
        table.advance_frame();
        assert_eq!(table.value(0, false), Some(ModUniformValue::Float(32.0)));
        assert_eq!(table.value(0, true), Some(ModUniformValue::Float(16.0)));

        // Reset history drops Frame 1/2 state so the next frame starts fresh.
        table.reset_history();
        assert!(table.set("modNear", ModUniformValue::Float(64.0)));
        table.advance_frame();
        assert_eq!(table.value(0, false), Some(ModUniformValue::Float(64.0)));
        assert_eq!(table.value(0, true), Some(ModUniformValue::Float(64.0)));
    }

    #[test]
    fn match_mod_uniform_flags_incompatible_glsl_types_without_panicking() {
        let decls = vec![ModUniformDecl {
            name: "modMatrix".into(),
            ty: ModUniformType::Mat4,
            previous_name: Some("modPrevMatrix".into()),
        }];
        assert_eq!(
            match_mod_uniform(&decls, "modMatrix", "mat4"),
            ModUniformMatch::Bound {
                index: 0,
                previous: false,
            }
        );
        assert_eq!(
            match_mod_uniform(&decls, "modPrevMatrix", "mat4"),
            ModUniformMatch::Bound {
                index: 0,
                previous: true,
            }
        );
        assert_eq!(
            match_mod_uniform(&decls, "modMatrix", "int"),
            ModUniformMatch::TypeMismatch {
                expected: ModUniformType::Mat4,
            }
        );
        assert_eq!(
            match_mod_uniform(&decls, "modPrevMatrix", "vec4"),
            ModUniformMatch::TypeMismatch {
                expected: ModUniformType::Mat4,
            }
        );
        assert_eq!(
            match_mod_uniform(&decls, "unrelated", "float"),
            ModUniformMatch::None
        );
    }

    #[test]
    fn std140_writes_mat3_and_mat4_with_16_byte_column_stride() {
        let mut buf = [0u8; 64];
        ModUniformValue::Mat3([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]])
            .write_std140(&mut buf, 0);
        let words: &[f32] = bytemuck::cast_slice(&buf);
        assert_eq!(&words[0..3], &[1.0, 2.0, 3.0]);
        assert_eq!(words[3], 0.0);
        assert_eq!(&words[4..7], &[4.0, 5.0, 6.0]);
        assert_eq!(words[7], 0.0);
        assert_eq!(&words[8..11], &[7.0, 8.0, 9.0]);
        assert_eq!(words[11], 0.0);

        // Out-of-bounds offset never panics.
        ModUniformValue::Mat4([[1.0; 4]; 4]).write_std140(&mut buf, 48);
    }

    #[test]
    fn mod_depth_source_resolves_full_and_opaque_depth_samplers() {
        let targets = vec![ModDepthTargetDecl {
            label: "example:aux_depth".into(),
            depth_samplers: vec!["modDepth0".into(), "modDepth".into()],
            opaque_depth_samplers: vec!["modDepth1".into()],
        }];
        assert_eq!(
            mod_depth_source(&targets, "modDepth0"),
            Some(Source::ModDepth(0))
        );
        assert_eq!(
            mod_depth_source(&targets, "modDepth"),
            Some(Source::ModDepth(0))
        );
        assert_eq!(
            mod_depth_source(&targets, "modDepth1"),
            Some(Source::ModOpaqueDepth(0))
        );
        assert_eq!(mod_depth_source(&targets, "depthtex0"), None);
    }
}
