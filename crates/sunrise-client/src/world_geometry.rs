//! Per-view custom geometry; buffers pre-reserve initial space and grow dynamically.
use crate::gfx::mod_meshes::ResolvedMeshDraw;
use crate::gfx::mod_pipelines::{
    self, IndirectSource, PipelineDraw, PipelineDrawSource, PipelineRun, PipelineVertex,
    ResolvedDrawBindings, ResolvedSet1Entry,
};
use crate::gfx::world_primitives::{
    Batch, DEFAULT_CAPACITY as CAPACITY, DrawRanges, OIT_STAGES, OitBatch,
    STAGE_AFTER_OPAQUE_BLOCKS, STAGE_AFTER_OPAQUE_FEATURES, STAGE_AFTER_TRANSLUCENT_BLOCKS,
    STAGE_AFTER_TRANSLUCENT_FEATURES, STAGE_AFTER_TRANSLUCENT_PARTICLES, STAGE_AFTER_WEATHER,
    STAGE_CUSTOM_GEOMETRY, STAGES,
};
use crate::{gizmos, mesher::Vertex};
use glam::{DVec3, Mat4, Vec3, Vec4};
use mod_api::{
    DrawError, InstanceError, ModelInstance, OitStage, PipelineQuad, PreparedWorldDraw,
    PreparedWorldDrawError, RenderLevelStage, WorldDepth, WorldLine, WorldQuad,
};

/// Initial pre-reservation hint for model copies per view; storage grows dynamically.
pub(crate) const INSTANCE_CAPACITY: u32 = 16_384;

/// One accepted copy, already checked. `light` and `overlay` are the renderer's vanilla packing.
#[derive(Clone, Copy)]
pub struct ModelCopy {
    pub slot: u32,
    pub generation: u64,
    pub origin: [f64; 3],
    pub transform: [f32; 12],
    pub tint: u32,
    pub light: u32,
    pub overlay: u32,
    /// `ModelInstance::color` and `hidden_faces`, as given.
    pub color: u32,
    pub hidden_faces: u8,
    /// Whether an active pipeline modifier set `WorldDepth::ThroughWalls` on this copy.
    pub through_walls: bool,
}

/// The model a batch draws. Resolved once, then applied to every copy.
#[derive(Clone, Copy)]
pub struct ModelBatch {
    pub slot: u32,
    pub generation: u64,
    pub bounds: [[f32; 3]; 2],
}

/// One label's span of the name-tag vertices, and its squared distance from the camera.
#[derive(Clone, Copy, Debug)]
struct LabelRun {
    distance_sq: f64,
    start: u32,
    end: u32,
}

/// Deferred geometry collected during `submit_custom_geometry` and debug renderers, appended
/// onto the tail (`STAGE_CUSTOM_GEOMETRY`) when [`WorldGeometry::finalize`] runs.
#[derive(Default)]
struct StageBuffers {
    quads: Vec<Vertex>,
    quads_through: Vec<Vertex>,
    lines: Vec<[f32; 8]>,
    lines_through: Vec<[f32; 8]>,
    pipeline_vertices: Vec<PipelineVertex>,
    pipeline_runs: Vec<PipelineRun>,
    pipeline_draws: Vec<PipelineDraw>,
    transient_vertices: Vec<u8>,
    transient_indices: Vec<u8>,
    inline_uniforms: Vec<u8>,
    texel_data: Vec<u8>,
}
impl StageBuffers {
    fn clear(&mut self) {
        self.quads.clear();
        self.quads_through.clear();
        self.lines.clear();
        self.lines_through.clear();
        self.pipeline_vertices.clear();
        self.pipeline_runs.clear();
        self.pipeline_draws.clear();
        self.transient_vertices.clear();
        self.transient_indices.clear();
        self.inline_uniforms.clear();
        self.texel_data.clear();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActiveTarget {
    /// Standard world pass buffers (`STAGE_AFTER_OPAQUE_BLOCKS..=STAGE_AFTER_WEATHER`).
    Primary,
    /// Deferred `submit_custom_geometry` and debug renderer buffers (`STAGE_CUSTOM_GEOMETRY`).
    Custom,
    /// Order-independent transparency sub-pass buffers (`OitStage` `0..=2`).
    Oit,
}

/// Borrowed frame-graph setup surface passed into `native_frame_graph_setup`.
#[derive(Default)]
pub struct FrameGraphSurface {
    pub graph: crate::gfx::mod_frame_graph::RecordedFrameGraph,
    pub oit_on: bool,
    pub camera: [f64; 3],
}

/// Borrowed stage surface passed into `native_render_level_stage`.
pub struct StageSurface<'a> {
    geometry: &'a mut WorldGeometry,
    stage: RenderLevelStage,
}
impl<'a> StageSurface<'a> {
    /// The render stage this surface belongs to.
    pub fn stage(&self) -> RenderLevelStage {
        self.stage
    }
    /// Record a draw command only after resolving it in the current borrowed view and stage.
    pub fn draw(&mut self, batch: PreparedWorldDraw) -> Result<(), PreparedWorldDrawError> {
        let (_, ranges) = self
            .geometry
            .view_id
            .and_then(|view| batch.__slot(view))
            .and_then(|slot| self.geometry.prepared.get(slot))
            .filter(|(stage, _)| *stage == self.stage)
            .ok_or(PreparedWorldDrawError)?;
        self.geometry.stage_draws.push(ranges.clone());
        Ok(())
    }
}

/// Reused between views, including frames with no subscribers or output.
pub struct WorldGeometry {
    view_id: Option<u64>,
    prepared: Vec<(RenderLevelStage, DrawRanges)>,
    stage_draws: Vec<DrawRanges>,
    frame_graph: FrameGraphSurface,
    camera: [f64; 3],
    view: Mat4,
    white: [f32; 2],
    lines: [Vec<[f32; 8]>; 2],
    quads: [Vec<Vertex>; 2],
    /// Quads drawn by mod pipelines, 4 vertices each, and the runs that share a pipeline and
    /// a stage.
    pipeline_quads: Vec<PipelineVertex>,
    pipeline_runs: Vec<PipelineRun>,
    /// Unified mod-pipeline draw commands (`draw_mesh`, `draw_vertices`, `pipeline_quads`).
    pipeline_draws: Vec<PipelineDraw>,
    transient_vertices: Vec<u8>,
    transient_indices: Vec<u8>,
    inline_uniforms: Vec<u8>,
    texel_data: Vec<u8>,
    /// OIT mod-pipeline quads and runs (`RenderLevelStage::OitTranslucent`).
    oit_pipeline_quads: Vec<PipelineVertex>,
    oit_pipeline_runs: Vec<PipelineRun>,
    oit_pipeline_draws: Vec<PipelineDraw>,
    oit_transient_vertices: Vec<u8>,
    oit_transient_indices: Vec<u8>,
    oit_inline_uniforms: Vec<u8>,
    oit_texel_data: Vec<u8>,
    /// Currently pushed pipeline modifier slots (outermost first, innermost last).
    modifier_stack: Vec<u32>,
    /// Interned modifier stacks referenced by `PipelineRun::modifier_stack`.
    modifier_stacks: crate::mod_pipelines::ModifierStacks,
    /// Per-frame shader-pack uniform values submitted via `WorldDrawContext::set_pack_uniform`.
    pack_uniforms: Vec<(&'static str, mod_api::PackUniformValue)>,
    /// Geometry submitted in `submit_custom_geometry` and debug renderers before pass stages run.
    custom: StageBuffers,
    /// Order-independent transparency sub-pass primitives (`DepthBounds`, `Transmittance`,
    /// `Accumulate`).
    oit_quads: Vec<Vertex>,
    oit_quads_through: Vec<Vertex>,
    oit_lines: Vec<[f32; 8]>,
    oit_lines_through: Vec<[f32; 8]>,
    /// Labels (`name_tag`), 4 vertices a quad: drawn with the world's name tags, then their
    /// see-through copies.
    labels: [Vec<Vertex>; 2],
    /// Each label's span of `labels[0]` and its squared distance from the camera, so the
    /// translucent name tags draw far to near ([`Self::sort_labels`]).
    label_runs: Vec<LabelRun>,
    /// Reused while [`Self::sort_labels`] reorders `labels[0]`.
    label_scratch: Vec<Vertex>,
    /// `PipelineFrame.time` for this view.
    time: f32,
    copies: Vec<ModelCopy>,
    submitted_groups: Vec<mod_api::InstanceGroupRef>,
    model_cut: u32,
    /// The standard pass stage (`0..=6`) lines and quads go to now.
    stage: usize,
    /// Where each standard pass stage before `stage` ended (`Batch::stage_ends`).
    stage_ends: [[u32; 4]; STAGES - 1],
    /// The OIT sub-pass stage (`0..=2`) OIT lines and quads go to now.
    oit_stage: usize,
    /// Where each OIT sub-pass stage before `oit_stage` ended (`OitBatch::stage_ends`).
    oit_stage_ends: [[u32; 4]; OIT_STAGES - 1],
    /// Which storage bucket receives `lines`, `quads`, and `pipeline_quads` writes.
    active: ActiveTarget,
    /// Whether `finalize` has already closed pass stages and appended `custom`.
    finalized: bool,
}
impl Default for WorldGeometry {
    fn default() -> Self {
        Self {
            view_id: None,
            prepared: Vec::new(),
            stage_draws: Vec::new(),
            frame_graph: FrameGraphSurface::default(),
            camera: [0.0; 3],
            view: Mat4::IDENTITY,
            white: [0.0; 2],
            lines: std::array::from_fn(|_| Vec::with_capacity(CAPACITY as usize)),
            quads: std::array::from_fn(|_| Vec::with_capacity(CAPACITY as usize * 4)),
            pipeline_quads: Vec::with_capacity(CAPACITY as usize * 4),
            pipeline_runs: Vec::with_capacity(CAPACITY as usize),
            pipeline_draws: Vec::new(),
            transient_vertices: Vec::new(),
            transient_indices: Vec::new(),
            inline_uniforms: Vec::new(),
            texel_data: Vec::new(),
            oit_pipeline_quads: Vec::new(),
            oit_pipeline_runs: Vec::new(),
            oit_pipeline_draws: Vec::new(),
            oit_transient_vertices: Vec::new(),
            oit_transient_indices: Vec::new(),
            oit_inline_uniforms: Vec::new(),
            oit_texel_data: Vec::new(),
            modifier_stack: Vec::new(),
            modifier_stacks: crate::mod_pipelines::ModifierStacks::new(),
            pack_uniforms: Vec::new(),
            custom: StageBuffers::default(),
            oit_quads: Vec::new(),
            oit_quads_through: Vec::new(),
            oit_lines: Vec::new(),
            oit_lines_through: Vec::new(),
            labels: std::array::from_fn(|_| Vec::with_capacity(CAPACITY as usize * 4)),
            label_runs: Vec::with_capacity(CAPACITY as usize),
            label_scratch: Vec::with_capacity(CAPACITY as usize * 4),
            time: 0.0,
            copies: Vec::with_capacity(INSTANCE_CAPACITY as usize),
            submitted_groups: Vec::new(),
            model_cut: 0,
            stage: 0,
            stage_ends: [[0; 4]; STAGES - 1],
            oit_stage: 0,
            oit_stage_ends: [[0; 4]; OIT_STAGES - 1],
            active: ActiveTarget::Primary,
            finalized: false,
        }
    }
}
impl WorldGeometry {
    /// Start a view even if nothing will be submitted; previous output disappears.
    pub fn reset(&mut self, camera: [f64; 3], view: Mat4, white: [f32; 2]) {
        self.view_id = None;
        self.prepared.clear();
        self.stage_draws.clear();
        self.frame_graph.graph.clear();
        self.frame_graph.oit_on = false;
        self.camera = camera;
        self.view = view;
        self.white = white;
        self.lines.iter_mut().for_each(Vec::clear);
        self.quads.iter_mut().for_each(Vec::clear);
        self.pipeline_quads.clear();
        self.pipeline_runs.clear();
        self.pipeline_draws.clear();
        self.transient_vertices.clear();
        self.transient_indices.clear();
        self.inline_uniforms.clear();
        self.texel_data.clear();
        self.oit_pipeline_quads.clear();
        self.oit_pipeline_runs.clear();
        self.oit_pipeline_draws.clear();
        self.oit_transient_vertices.clear();
        self.oit_transient_indices.clear();
        self.oit_inline_uniforms.clear();
        self.oit_texel_data.clear();
        self.modifier_stack.clear();
        self.modifier_stacks.clear();
        self.pack_uniforms.clear();
        self.custom.clear();
        self.oit_quads.clear();
        self.oit_quads_through.clear();
        self.oit_lines.clear();
        self.oit_lines_through.clear();
        self.labels.iter_mut().for_each(Vec::clear);
        self.label_runs.clear();
        self.clear_copies();
        self.model_cut = 0;
        self.submitted_groups.clear();
        self.stage = 0;
        self.stage_ends = [[0; 4]; STAGES - 1];
        self.oit_stage = 0;
        self.oit_stage_ends = [[0; 4]; OIT_STAGES - 1];
        self.active = ActiveTarget::Primary;
        self.finalized = false;
    }
    /// Borrow the frame-graph setup surface for `native_frame_graph_setup`.
    pub fn frame_graph_surface(&mut self, oit_on: bool) -> &mut FrameGraphSurface {
        self.frame_graph.graph.clear();
        self.frame_graph.oit_on = oit_on;
        self.frame_graph.camera = self.camera;
        &mut self.frame_graph
    }
    /// Take any recorded frame graph passes for compilation and execution.
    pub fn take_frame_graph(&mut self) -> Option<crate::gfx::mod_frame_graph::RecordedFrameGraph> {
        if self.frame_graph.graph.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.frame_graph.graph))
        }
    }
    /// Borrow a stage-scoped surface for `native_render_level_stage`.
    pub fn stage_surface(&mut self, stage: RenderLevelStage) -> StageSurface<'_> {
        self.stage_draws.clear();
        StageSurface {
            geometry: self,
            stage,
        }
    }
    /// Route subsequent `lines`, `quads`, and `pipeline_quads` writes into the deferred custom
    /// geometry buffer (`STAGE_CUSTOM_GEOMETRY`), appended at the tail by [`Self::finalize`].
    pub fn begin_custom_geometry(&mut self) {
        self.active = ActiveTarget::Custom;
    }
    /// Route subsequent writes into a pass-bearing stage (`AfterOpaqueBlocks..=AfterWeather` or
    /// `OitTranslucent(oit)`), advancing stage end boundaries monotonically. Stages going
    /// backwards stay at the current stage; no-pass stages (`AfterSky`, `AfterLevel`) are no-ops.
    pub fn begin_pass_stage(&mut self, stage: RenderLevelStage) {
        match stage {
            RenderLevelStage::AfterOpaqueBlocks
            | RenderLevelStage::AfterOpaqueFeatures
            | RenderLevelStage::AfterTranslucentFeatures
            | RenderLevelStage::AfterTranslucentBlocks
            | RenderLevelStage::AfterTranslucentParticles
            | RenderLevelStage::AfterWeather => {
                self.active = ActiveTarget::Primary;
                let index = match stage {
                    RenderLevelStage::AfterOpaqueBlocks => STAGE_AFTER_OPAQUE_BLOCKS,
                    RenderLevelStage::AfterOpaqueFeatures => STAGE_AFTER_OPAQUE_FEATURES,
                    RenderLevelStage::AfterTranslucentFeatures => STAGE_AFTER_TRANSLUCENT_FEATURES,
                    RenderLevelStage::AfterTranslucentBlocks => STAGE_AFTER_TRANSLUCENT_BLOCKS,
                    RenderLevelStage::AfterTranslucentParticles => {
                        STAGE_AFTER_TRANSLUCENT_PARTICLES
                    }
                    RenderLevelStage::AfterWeather => STAGE_AFTER_WEATHER,
                    _ => unreachable!(),
                };
                let counts = self.counts();
                for ended in self.stage..index.min(STAGES - 1) {
                    self.stage_ends[ended] = counts;
                }
                self.stage = self.stage.max(index);
            }
            RenderLevelStage::OitTranslucent(oit) => {
                self.active = ActiveTarget::Oit;
                let oit_index = match oit {
                    OitStage::DepthBounds => 0,
                    OitStage::Transmittance => 1,
                    OitStage::Accumulate => 2,
                };
                let counts = self.oit_counts();
                for ended in self.oit_stage..oit_index.min(OIT_STAGES - 1) {
                    self.oit_stage_ends[ended] = counts;
                }
                self.oit_stage = self.oit_stage.max(oit_index);
            }
            RenderLevelStage::AfterSky | RenderLevelStage::AfterLevel => {}
            _ => {}
        }
    }
    /// Push a registered pipeline modifier slot onto the active stack. Returns `false` if `slot`
    /// is out of range of the registered modifiers (without pushing).
    pub fn push_pipeline_modifier(&mut self, slot: u32) -> bool {
        if (slot as usize) >= crate::mod_pipelines::registered_modifiers().len() {
            return false;
        }
        self.modifier_stack.push(slot);
        true
    }

    /// Pop the most recently pushed pipeline modifier slot.
    pub fn pop_pipeline_modifier(&mut self) {
        self.modifier_stack.pop();
    }

    /// Intern the active modifier stack (`0` when empty).
    fn active_modifier_stack(&mut self) -> u32 {
        self.modifier_stacks.intern(&self.modifier_stack)
    }

    /// The depth the active modifier stack forces, if any (outermost wins).
    fn depth_override(&self) -> Option<WorldDepth> {
        if self.modifier_stack.is_empty() {
            return None;
        }
        let defs = crate::mod_pipelines::registered_modifiers();
        self.modifier_stack
            .iter()
            .rev()
            .filter_map(|&slot| defs.get(slot as usize).and_then(|m| m.depth_compare))
            .next_back()
            .map(|cmp| {
                if cmp == mod_api::DepthCompare::Always {
                    WorldDepth::ThroughWalls
                } else {
                    WorldDepth::Tested
                }
            })
    }

    /// Effective `WorldDepth` after applying the active modifier stack.
    fn effective_depth(&self, base: WorldDepth) -> WorldDepth {
        self.depth_override().unwrap_or(base)
    }

    /// Effective [`mod_api::PrimitiveTopology`] after applying the active modifier stack
    /// (outermost wins).
    pub fn effective_topology(
        &self,
        base: mod_api::PrimitiveTopology,
    ) -> mod_api::PrimitiveTopology {
        if self.modifier_stack.is_empty() {
            return base;
        }
        let defs = crate::mod_pipelines::registered_modifiers();
        let mut topology = base;
        for &slot in self.modifier_stack.iter().rev() {
            if let Some(t) = defs.get(slot as usize).and_then(|m| m.topology) {
                topology = t;
            }
        }
        topology
    }

    /// Append geometry to a shared CPU stream and return a handle to just the appended ranges.
    /// Only the renderer's prepare hook can call this; stage callbacks can only queue handles.
    pub fn prepare_draw<E>(
        &mut self,
        stage: RenderLevelStage,
        append: impl FnOnce(&mut Self) -> Result<(), E>,
    ) -> Result<PreparedWorldDraw, E> {
        let prev_active = self.active;
        let prev_stage = self.stage;
        let prev_oit_stage = self.oit_stage;
        let oit = stage.oit_stage();
        self.active = if oit.is_some() {
            ActiveTarget::Oit
        } else {
            ActiveTarget::Primary
        };
        if let Some(oit_stage) = oit {
            self.oit_stage = match oit_stage {
                OitStage::DepthBounds => 0,
                OitStage::Transmittance => 1,
                OitStage::Accumulate => 2,
            };
        } else {
            self.stage = match stage {
                RenderLevelStage::AfterOpaqueBlocks => STAGE_AFTER_OPAQUE_BLOCKS,
                RenderLevelStage::AfterOpaqueFeatures => STAGE_AFTER_OPAQUE_FEATURES,
                RenderLevelStage::AfterTranslucentFeatures => STAGE_AFTER_TRANSLUCENT_FEATURES,
                RenderLevelStage::AfterTranslucentBlocks => STAGE_AFTER_TRANSLUCENT_BLOCKS,
                RenderLevelStage::AfterTranslucentParticles => STAGE_AFTER_TRANSLUCENT_PARTICLES,
                RenderLevelStage::AfterWeather => STAGE_AFTER_WEATHER,
                _ => STAGE_CUSTOM_GEOMETRY,
            };
        }
        let start = if oit.is_some() {
            self.oit_counts()
        } else {
            self.counts()
        };
        let pipe_start = if oit.is_some() {
            self.oit_pipeline_runs.len() as u32
        } else {
            self.pipeline_runs.len() as u32
        };
        let draw_start = if oit.is_some() {
            self.oit_pipeline_draws.len() as u32
        } else {
            self.pipeline_draws.len() as u32
        };
        let res = append(self);
        self.active = prev_active;
        self.stage = prev_stage;
        self.oit_stage = prev_oit_stage;
        res?;
        let end = if oit.is_some() {
            self.oit_counts()
        } else {
            self.counts()
        };
        let pipe_end = if oit.is_some() {
            self.oit_pipeline_runs.len() as u32
        } else {
            self.pipeline_runs.len() as u32
        };
        let draw_end = if oit.is_some() {
            self.oit_pipeline_draws.len() as u32
        } else {
            self.pipeline_draws.len() as u32
        };
        let slot = self.prepared.len();
        self.prepared.push((
            stage,
            DrawRanges {
                primitives: std::array::from_fn(|i| start[i]..end[i]),
                pipeline_runs: pipe_start..pipe_end,
                pipeline_draws: draw_start..draw_end,
            },
        ));
        // Issue one identity per view that actually prepares a batch. This also prevents
        // a handle from a closed world resolving in a different renderer's first view.
        static NEXT_VIEW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let view = *self
            .view_id
            .get_or_insert_with(|| NEXT_VIEW.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        Ok(PreparedWorldDraw::__new(view, slot))
    }

    pub fn stage_draws(&self) -> &[DrawRanges] {
        &self.stage_draws
    }

    /// Close standard pass stages through `STAGE_AFTER_WEATHER` and append deferred custom
    /// geometry onto the tail as `STAGE_CUSTOM_GEOMETRY`, keeping `stage_ends` monotonic across
    /// all 6 boundaries.
    pub fn finalize(&mut self) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        self.active = ActiveTarget::Primary;
        let counts = self.counts();
        for ended in self.stage..STAGE_CUSTOM_GEOMETRY {
            self.stage_ends[ended] = counts;
        }
        self.stage = STAGE_CUSTOM_GEOMETRY;
        let oit_counts = self.oit_counts();
        for ended in self.oit_stage..(OIT_STAGES - 1) {
            self.oit_stage_ends[ended] = oit_counts;
        }
        self.oit_stage = OIT_STAGES - 1;
        self.quads[0].extend_from_slice(&self.custom.quads);
        self.quads[1].extend_from_slice(&self.custom.quads_through);
        self.lines[0].extend_from_slice(&self.custom.lines);
        self.lines[1].extend_from_slice(&self.custom.lines_through);
        self.pipeline_quads
            .extend_from_slice(&self.custom.pipeline_vertices);
        self.pipeline_runs
            .extend_from_slice(&self.custom.pipeline_runs);
        let v_base = self.transient_vertices.len() as u64;
        let i_base = self.transient_indices.len() as u64;
        let u_base = self.inline_uniforms.len();
        let t_base = self.texel_data.len();
        self.transient_vertices
            .extend_from_slice(&self.custom.transient_vertices);
        self.transient_indices
            .extend_from_slice(&self.custom.transient_indices);
        self.inline_uniforms
            .extend_from_slice(&self.custom.inline_uniforms);
        self.texel_data.extend_from_slice(&self.custom.texel_data);
        for mut cmd in self.custom.pipeline_draws.drain(..) {
            if let PipelineDrawSource::Transient {
                ref mut vertex_ranges,
                ref mut index_range,
            } = cmd.source
            {
                for (_, range) in vertex_ranges.iter_mut() {
                    range.start += v_base;
                    range.end += v_base;
                }
                if let Some((_, range)) = index_range.as_mut() {
                    range.start += i_base;
                    range.end += i_base;
                }
            }
            if let Some(ref mut bindings) = cmd.bindings {
                for entry in &mut bindings.entries {
                    match entry {
                        ResolvedSet1Entry::InlineUniform { byte_range } => {
                            byte_range.start += u_base;
                            byte_range.end += u_base;
                        }
                        ResolvedSet1Entry::TexelBuffer { byte_range } => {
                            byte_range.start += t_base;
                            byte_range.end += t_base;
                        }
                        _ => {}
                    }
                }
                if let Some(ref mut pc) = bindings.push_constants {
                    pc.start += u_base;
                    pc.end += u_base;
                }
            }
            self.pipeline_draws.push(cmd);
        }
        self.custom.clear();
    }
    /// The game time this view's pipeline shaders read, in seconds (`PipelineFrame.time`).
    pub fn set_time(&mut self, seconds: f32) {
        self.time = seconds;
    }
    /// Entity-stream cut used by directly staged instances in this view.
    pub fn set_model_cut(&mut self, cut: u32) {
        self.model_cut = cut;
    }
    pub fn model_cut(&self) -> u32 {
        self.model_cut
    }
    pub fn camera(&self) -> [f64; 3] {
        self.camera
    }
    /// Validate every copy (including camera-relative shift and transformed-bounds finiteness)
    /// and cull against the frustum, skipping invalid copies without rolling back earlier valid
    /// copies. `stage` returns true for copies it consumes directly.
    pub fn submit_model(
        &mut self,
        batch: ModelBatch,
        instances: &[ModelInstance],
        cull: Option<&impl crate::gfx::Cull>,
        mut stage: impl FnMut(&ModelCopy) -> bool,
    ) {
        self.submit_model_inner(batch, instances, cull, |_, copy| stage(copy));
    }
    /// Submit copies from persistent group storage through the same per-copy finiteness and
    /// camera-relative validation as transient batches (catching composed-transform or
    /// camera-relative overflow).
    pub fn submit_validated_model(
        &mut self,
        batch: ModelBatch,
        instances: &[ModelInstance],
        cull: Option<&impl crate::gfx::Cull>,
        stage: impl FnMut(usize, &ModelCopy) -> bool,
    ) {
        self.submit_model_inner(batch, instances, cull, stage);
    }
    fn submit_model_inner(
        &mut self,
        batch: ModelBatch,
        instances: &[ModelInstance],
        cull: Option<&impl crate::gfx::Cull>,
        mut stage: impl FnMut(usize, &ModelCopy) -> bool,
    ) {
        let through_walls = self.effective_depth(WorldDepth::Tested) == WorldDepth::ThroughWalls;
        for (index, instance) in instances.iter().enumerate() {
            if instance.origin.iter().any(|v| !v.is_finite())
                || instance.transform.iter().any(|v| !v.is_finite())
            {
                continue;
            }
            if !copy_visible(
                batch.bounds,
                instance.transform,
                instance.origin,
                self.camera,
                cull,
            ) || instance.light.block > 15
                || instance.light.sky > 15
            {
                continue;
            }
            let copy = ModelCopy {
                slot: batch.slot,
                generation: batch.generation,
                origin: instance.origin,
                transform: instance.transform,
                tint: instance.tint,
                light: pack_light(instance.light),
                overlay: pack_overlay(instance.overlay),
                color: instance.color,
                hidden_faces: instance.hidden_faces,
                through_walls,
            };
            if !stage(index, &copy) {
                self.copies.push(copy);
            }
        }
    }
    pub fn queue_group(&mut self, group: mod_api::InstanceGroupRef) {
        self.submitted_groups.push(group);
    }
    pub fn drain_submitted_groups(
        &mut self,
    ) -> impl Iterator<Item = mod_api::InstanceGroupRef> + '_ {
        self.submitted_groups.drain(..)
    }
    pub fn take_submitted_groups(&mut self) -> Vec<mod_api::InstanceGroupRef> {
        std::mem::take(&mut self.submitted_groups)
    }
    pub fn recycle_submitted_groups(&mut self, submitted: Vec<mod_api::InstanceGroupRef>) {
        self.submitted_groups = submitted;
    }
    pub fn copy_count(&self) -> usize {
        self.copies.len()
    }
    pub fn copy(&self, index: usize) -> ModelCopy {
        self.copies[index]
    }
    pub fn clear_copies(&mut self) {
        self.copies.clear();
    }
    /// Validate once at the submission boundary, before any output is appended.
    fn validate_positions(&self, points: impl Iterator<Item = [f64; 3]>) -> Result<(), DrawError> {
        if points
            .flatten()
            .enumerate()
            .any(|(i, p)| !p.is_finite() || !((p - self.camera[i % 3]) as f32).is_finite())
        {
            return Err(DrawError::NonFinite);
        }
        Ok(())
    }
    /// Append an entire line batch or leave the output unchanged.
    pub fn lines(&mut self, lines: &[WorldLine]) -> Result<(), DrawError> {
        self.validate_positions(lines.iter().flat_map(|l| [l.from, l.to]))?;
        for l in lines {
            if !l.width.is_finite() {
                return Err(DrawError::NonFinite);
            }
            if l.width <= 0.0 {
                return Err(DrawError::InvalidWidth);
            }
        }
        let depth_override = self.depth_override();
        let (out_tested, out_through) = match self.active {
            ActiveTarget::Primary => {
                let [tested, through] = &mut self.lines;
                (tested, through)
            }
            ActiveTarget::Custom => (&mut self.custom.lines, &mut self.custom.lines_through),
            ActiveTarget::Oit => (&mut self.oit_lines, &mut self.oit_lines_through),
        };
        for l in lines {
            let line = gizmos::Line {
                start: l.from,
                end: l.to,
                color: l.color,
                width: l.width,
            };
            let out = if depth_override.unwrap_or(l.depth) == WorldDepth::ThroughWalls {
                &mut *out_through
            } else {
                &mut *out_tested
            };
            gizmos::camera_relative_to(&[line], self.camera, self.view, |line| out.push(line));
        }
        Ok(())
    }
    /// Append an entire quad batch or leave the output unchanged on non-finite coordinates.
    pub fn quads(&mut self, quads: &[WorldQuad]) -> Result<(), DrawError> {
        self.validate_positions(quads.iter().flat_map(|q| q.corners))?;
        let depth_override = self.depth_override();
        let (out_tested, out_through) = match self.active {
            ActiveTarget::Primary => {
                let [tested, through] = &mut self.quads;
                (tested, through)
            }
            ActiveTarget::Custom => (&mut self.custom.quads, &mut self.custom.quads_through),
            ActiveTarget::Oit => (&mut self.oit_quads, &mut self.oit_quads_through),
        };
        for q in quads {
            let color = crate::world_primitives::vertex_color(q.color, 255);
            let out = if depth_override.unwrap_or(q.depth) == WorldDepth::ThroughWalls {
                &mut *out_through
            } else {
                &mut *out_tested
            };
            out.extend(q.corners.map(|p| Vertex {
                pos: std::array::from_fn(|i| (p[i] - self.camera[i]) as f32),
                uv: self.white,
                color,
                layer: 0,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.0; 2],
            }));
        }
        Ok(())
    }
    /// Refuses a label's position before its text is read or laid out.
    pub fn check_label(&self, at: [f64; 3]) -> Result<(), DrawError> {
        self.validate_positions(std::iter::once(at))
    }
    /// Append one label's quads, laid out by the caller (`world` drawn with the name tags,
    /// `see_through` over everything). `distance_sq` is the label's squared distance from
    /// the camera, by which the name tags are sorted.
    pub fn label(&mut self, distance_sq: f64, world: &[Vertex], see_through: &[Vertex]) {
        if !world.is_empty() {
            let start = self.labels[0].len() as u32;
            self.label_runs.push(LabelRun {
                distance_sq,
                start,
                end: start + world.len() as u32,
            });
        }
        self.labels[0].extend_from_slice(world);
        self.labels[1].extend_from_slice(see_through);
    }
    /// Orders the labels drawn with the name tags far to near, as vanilla sorts its translucent
    /// name tags (`NameTagFeatureRenderer`): equally far ones keep their submission order.
    /// Their see-through copies are not depth tested and keep submission order.
    pub fn sort_labels(&mut self) {
        if self
            .label_runs
            .windows(2)
            .all(|pair| pair[0].distance_sq >= pair[1].distance_sq)
        {
            return;
        }
        self.label_runs
            .sort_by(|a, b| b.distance_sq.total_cmp(&a.distance_sq));
        let mut sorted = std::mem::take(&mut self.label_scratch);
        sorted.clear();
        for run in &mut self.label_runs {
            let start = sorted.len() as u32;
            sorted.extend_from_slice(&self.labels[0][run.start as usize..run.end as usize]);
            (run.start, run.end) = (start, sorted.len() as u32);
        }
        self.label_scratch = std::mem::replace(&mut self.labels[0], sorted);
    }
    /// This view's labels: those drawn with the name tags, then their see-through copies.
    pub fn labels(&self) -> [&[Vertex]; 2] {
        [&self.labels[0], &self.labels[1]]
    }
    /// Append valid quads drawn by the mod pipeline in `slot`. The caller has resolved the
    /// handle; quads with any non-finite corner, camera-relative `f32` coordinate, or UV are
    /// skipped individually without rejecting valid quads in the same slice. Accepted quads
    /// that follow the last ones with the same pipeline and stage join their run.
    pub fn pipeline_quads(&mut self, slot: u32, quads: &[PipelineQuad]) {
        let modifier_stack = self.active_modifier_stack();
        let (vertices, runs, stage) = match self.active {
            ActiveTarget::Primary => (
                &mut self.pipeline_quads,
                &mut self.pipeline_runs,
                self.stage as u32,
            ),
            ActiveTarget::Custom => (
                &mut self.custom.pipeline_vertices,
                &mut self.custom.pipeline_runs,
                STAGE_CUSTOM_GEOMETRY as u32,
            ),
            ActiveTarget::Oit => (
                &mut self.oit_pipeline_quads,
                &mut self.oit_pipeline_runs,
                self.oit_stage as u32,
            ),
        };
        let mut pushed = 0u32;
        for q in quads {
            let corners_finite = q.corners.iter().all(|corner| {
                corner
                    .iter()
                    .zip(self.camera)
                    .all(|(&p, cam)| p.is_finite() && ((p - cam) as f32).is_finite())
            });
            if !corners_finite || q.uv.iter().flatten().any(|c| !c.is_finite()) {
                continue;
            }
            let color = crate::world_primitives::vertex_color(q.color, 255);
            vertices.extend((0..4).map(|corner| PipelineVertex {
                pos: std::array::from_fn(|i| (q.corners[corner][i] - self.camera[i]) as f32),
                uv: q.uv[corner],
                color,
            }));
            pushed += 1;
        }
        if pushed == 0 {
            return;
        }
        match runs.last_mut() {
            Some(run)
                if run.slot == slot
                    && run.stage == stage
                    && run.modifier_stack == modifier_stack =>
            {
                run.quads += pushed;
            }
            _ => runs.push(PipelineRun {
                slot,
                stage,
                quads: pushed,
                modifier_stack,
            }),
        }
    }
    /// Append inline uniform or push-constant bytes into the active target's per-frame byte pool.
    pub fn push_inline_uniform(&mut self, bytes: &[u8]) -> std::ops::Range<usize> {
        let pool = match self.active {
            ActiveTarget::Primary => &mut self.inline_uniforms,
            ActiveTarget::Custom => &mut self.custom.inline_uniforms,
            ActiveTarget::Oit => &mut self.oit_inline_uniforms,
        };
        let start = pool.len();
        pool.extend_from_slice(bytes);
        start..pool.len()
    }

    /// Append expanded `i32`/`u32` texel-buffer bytes into the active target's per-frame byte pool.
    pub fn push_texel_data(&mut self, bytes: &[u8]) -> std::ops::Range<usize> {
        let pool = match self.active {
            ActiveTarget::Primary => &mut self.texel_data,
            ActiveTarget::Custom => &mut self.custom.texel_data,
            ActiveTarget::Oit => &mut self.oit_texel_data,
        };
        let start = pool.len();
        pool.extend_from_slice(bytes);
        start..pool.len()
    }

    /// Append a unified transient vertex/index draw command (used by `draw_vertices` and the
    /// `pipeline_quads` shorthand). Caller has already validated alignment, strides, and bounds.
    pub fn draw_vertices_slot(
        &mut self,
        slot: u32,
        vertices: &[(u32, &[u8])],
        indices: Option<(mod_api::IndexFormat, &[u8])>,
        draw: mod_api::MeshDraw,
        bindings: Option<ResolvedDrawBindings>,
    ) {
        let modifier_stack = self.active_modifier_stack();
        let (draws, v_pool, i_pool, stage) = match self.active {
            ActiveTarget::Primary => (
                &mut self.pipeline_draws,
                &mut self.transient_vertices,
                &mut self.transient_indices,
                self.stage as u32,
            ),
            ActiveTarget::Custom => (
                &mut self.custom.pipeline_draws,
                &mut self.custom.transient_vertices,
                &mut self.custom.transient_indices,
                STAGE_CUSTOM_GEOMETRY as u32,
            ),
            ActiveTarget::Oit => (
                &mut self.oit_pipeline_draws,
                &mut self.oit_transient_vertices,
                &mut self.oit_transient_indices,
                self.oit_stage as u32,
            ),
        };
        let mut vertex_ranges = Vec::with_capacity(vertices.len());
        for &(binding, bytes) in vertices {
            let start = v_pool.len() as u64;
            v_pool.extend_from_slice(bytes);
            let end = v_pool.len() as u64;
            vertex_ranges.push((binding, start..end));
        }
        let index_range = indices.map(|(fmt, bytes)| {
            let start = i_pool.len() as u64;
            i_pool.extend_from_slice(bytes);
            let end = i_pool.len() as u64;
            (fmt, start..end)
        });
        draws.push(PipelineDraw {
            slot,
            stage,
            modifier_stack,
            draw,
            source: PipelineDrawSource::Transient {
                vertex_ranges,
                index_range,
            },
            bindings,
        });
    }
    /// Record a zero-copy retained [`mod_api::MeshRef`] draw command. Caller has already
    /// validated the handle, ownership, strides, and bounds.
    pub fn draw_mesh_slot(
        &mut self,
        slot: u32,
        resolved: ResolvedMeshDraw,
        draw: mod_api::MeshDraw,
        bindings: Option<ResolvedDrawBindings>,
    ) {
        self.draw_source_slot(slot, draw, PipelineDrawSource::Retained(resolved), bindings);
    }
    /// Record a GPU-driven draw command. Caller has already validated the pipeline, the mesh,
    /// the command buffer and the bindings.
    pub fn draw_indirect_slot(
        &mut self,
        slot: u32,
        indirect: IndirectSource,
        bindings: Option<ResolvedDrawBindings>,
    ) {
        self.draw_source_slot(
            slot,
            mod_api::MeshDraw::vertices(0..0).with_instances(0..1),
            PipelineDrawSource::Indirect(indirect),
            bindings,
        );
    }
    fn draw_source_slot(
        &mut self,
        slot: u32,
        draw: mod_api::MeshDraw,
        source: PipelineDrawSource,
        bindings: Option<ResolvedDrawBindings>,
    ) {
        let modifier_stack = self.active_modifier_stack();
        let (draws, stage) = match self.active {
            ActiveTarget::Primary => (&mut self.pipeline_draws, self.stage as u32),
            ActiveTarget::Custom => (
                &mut self.custom.pipeline_draws,
                STAGE_CUSTOM_GEOMETRY as u32,
            ),
            ActiveTarget::Oit => (&mut self.oit_pipeline_draws, self.oit_stage as u32),
        };
        draws.push(PipelineDraw {
            slot,
            stage,
            modifier_stack,
            draw,
            source,
            bindings,
        });
    }
    /// Record a per-frame value for a registered shader-pack uniform without heap allocation.
    pub fn set_pack_uniform(&mut self, name: &'static str, value: mod_api::PackUniformValue) {
        if let Some((_, existing)) = self.pack_uniforms.iter_mut().find(|(k, _)| *k == name) {
            *existing = value;
        } else {
            self.pack_uniforms.push((name, value));
        }
    }
    /// Borrow the existing renderer's batch, with no copying or frame allocation. The frame's
    /// builder names the pipeline set the queued slots belong to.
    pub fn batch(&self) -> Batch<'_> {
        Batch {
            lines: &self.lines[0],
            lines_through: &self.lines[1],
            quads: &self.quads[0],
            quads_through: &self.quads[1],
            stage_ends: self.stage_ends(),
            oit: OitBatch {
                quads: &self.oit_quads,
                quads_through: &self.oit_quads_through,
                lines: &self.oit_lines,
                lines_through: &self.oit_lines_through,
                stage_ends: self.oit_stage_ends(),
            },
            pipelines: mod_pipelines::Batch {
                set: None,
                vertices: &self.pipeline_quads,
                runs: &self.pipeline_runs,
                draws: &self.pipeline_draws,
                transient_vertices: &self.transient_vertices,
                transient_indices: &self.transient_indices,
                inline_uniforms: &self.inline_uniforms,
                texel_data: &self.texel_data,
                modifiers: Some(&self.modifier_stacks),
                modifier_defs: None,
                oit_vertices: &self.oit_pipeline_quads,
                oit_runs: &self.oit_pipeline_runs,
                oit_draws: &self.oit_pipeline_draws,
                oit_transient_vertices: &self.oit_transient_vertices,
                oit_transient_indices: &self.oit_transient_indices,
                oit_inline_uniforms: &self.oit_inline_uniforms,
                oit_texel_data: &self.oit_texel_data,
                time: self.time,
            },
            pack_uniforms: &self.pack_uniforms,
        }
    }
    /// [`Self::batch`] for a frame drawn with the resources that built `pipelines`, the set
    /// the queued pipeline quads' slots were resolved in.
    pub fn batch_with<'a>(&'a self, pipelines: &'a crate::mod_pipelines::Pipelines) -> Batch<'a> {
        let mut batch = self.batch();
        batch.pipelines.set = Some(pipelines);
        batch
    }
    /// Quads, quads through walls, lines and lines through walls queued so far.
    fn counts(&self) -> [u32; 4] {
        [
            (self.quads[0].len() / 4) as u32,
            (self.quads[1].len() / 4) as u32,
            self.lines[0].len() as u32,
            self.lines[1].len() as u32,
        ]
    }
    /// OIT quads, OIT quads through walls, OIT lines and OIT lines through walls queued so far.
    fn oit_counts(&self) -> [u32; 4] {
        [
            (self.oit_quads.len() / 4) as u32,
            (self.oit_quads_through.len() / 4) as u32,
            self.oit_lines.len() as u32,
            self.oit_lines_through.len() as u32,
        ]
    }
    /// The ends of the stages before the current one; those not reached yet end at the counts
    /// so far, so what follows draws at the current stage.
    fn stage_ends(&self) -> [[u32; 4]; STAGES - 1] {
        let counts = self.counts();
        std::array::from_fn(|stage| {
            if stage < self.stage {
                self.stage_ends[stage]
            } else {
                counts
            }
        })
    }
    /// The ends of the OIT stages before the current one.
    fn oit_stage_ends(&self) -> [[u32; 4]; OIT_STAGES - 1] {
        let counts = self.oit_counts();
        std::array::from_fn(|stage| {
            if stage < self.oit_stage {
                self.oit_stage_ends[stage]
            } else {
                counts
            }
        })
    }
}

/// Row-major 3×4 to a column-major matrix. The last row is the affine 0,0,0,1.
#[inline]
pub fn row_major_3x4(m: [f32; 12]) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(m[0], m[4], m[8], 0.0),
        Vec4::new(m[1], m[5], m[9], 0.0),
        Vec4::new(m[2], m[6], m[10], 0.0),
        Vec4::new(m[3], m[7], m[11], 1.0),
    )
}

/// Block-entity vertex light: sky in the high nibble and block in the low one.
pub fn pack_light(light: mod_api::LightCoords) -> u32 {
    crate::block_entity_render::vertex_light((light.sky << 4) | light.block)
}

/// `LivingEntityRenderer.getOverlayCoords` as the entity shader's overlay code.
pub fn pack_overlay(overlay: mod_api::Overlay) -> u32 {
    if overlay.hurt {
        return 1;
    }
    match u32::from(overlay.white.min(15)) {
        0 => 0,
        column => crate::overlay::WHITE + column,
    }
}

fn copy_visible(
    bounds: [[f32; 3]; 2],
    transform: [f32; 12],
    origin: [f64; 3],
    camera: [f64; 3],
    cull: Option<&impl crate::gfx::Cull>,
) -> bool {
    let local = row_major_3x4(transform);
    let delta = DVec3::from(origin) - DVec3::from(camera);
    let shift = Vec3::new(delta.x as f32, delta.y as f32, delta.z as f32);
    if !shift.is_finite() {
        return false;
    }
    let translation = local.w_axis.truncate();
    if !(translation + shift).is_finite() {
        return false;
    }
    if !bounds.iter().flatten().all(|v| v.is_finite()) {
        return true;
    }
    // Affine extrema are selected independently for each output axis. Keep the
    // multiply/add order of transform_point3, without transforming eight corners.
    let x0 = local.x_axis.truncate() * bounds[0][0];
    let x1 = local.x_axis.truncate() * bounds[1][0];
    let y0 = local.y_axis.truncate() * bounds[0][1];
    let y1 = local.y_axis.truncate() * bounds[1][1];
    let z0 = local.z_axis.truncate() * bounds[0][2];
    let z1 = local.z_axis.truncate() * bounds[1][2];
    let mut min = translation + (z0.min(z1) + (y0.min(y1) + x0.min(x1))) + shift;
    let mut max = translation + (z0.max(z1) + (y0.max(y1) + x0.max(x1))) + shift;
    if !min.is_finite() || !max.is_finite() {
        // Overflow can produce NaNs whose min/max depends on corner order.
        // Preserve that order only for these exceptional finite inputs.
        min = Vec3::splat(f32::INFINITY);
        max = Vec3::splat(f32::NEG_INFINITY);
        for corner in 0..8 {
            let point = Vec3::new(
                bounds[usize::from(corner & 1 != 0)][0],
                bounds[usize::from(corner & 2 != 0)][1],
                bounds[usize::from(corner & 4 != 0)][2],
            );
            let world = local.transform_point3(point) + shift;
            min = min.min(world);
            max = max.max(world);
        }
    }
    if !min.is_finite() || !max.is_finite() {
        return false;
    }
    cull.is_none_or(|cull| cull.intersects(min, max))
}

pub fn validate_instances(instances: &[ModelInstance]) -> Result<(), InstanceError> {
    for instance in instances {
        if instance.origin.iter().any(|v| !v.is_finite())
            || instance.transform.iter().any(|v| !v.is_finite())
        {
            return Err(InstanceError::NonFinite);
        }
        if instance.light.block > 15 || instance.light.sky > 15 {
            return Err(InstanceError::InvalidLight);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_draw_uses_only_the_requested_batch_in_its_view_and_stage() {
        use RenderLevelStage::*;
        let line = WorldLine::new(
            [0.0, 0.0, -2.0],
            [1.0, 0.0, -2.0],
            u32::MAX,
            2.0,
            WorldDepth::Tested,
        );
        let mut draw = WorldGeometry::default();
        let first = draw
            .prepare_draw(AfterWeather, |g| g.lines(&[line]))
            .unwrap();
        let second = draw
            .prepare_draw(AfterOpaqueBlocks, |g| g.lines(&[line, line]))
            .unwrap();
        draw.finalize();

        let mut surface = draw.stage_surface(AfterOpaqueBlocks);
        assert_eq!(surface.draw(first), Err(PreparedWorldDrawError));
        surface.draw(second).unwrap();
        assert_eq!(draw.stage_draws().len(), 1);
        assert_eq!(draw.stage_draws()[0].primitives, [0..0, 0..0, 1..3, 0..0]);

        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        let new = draw
            .prepare_draw(AfterOpaqueBlocks, |g| g.lines(&[line]))
            .unwrap();
        let mut surface = draw.stage_surface(AfterOpaqueBlocks);
        assert_eq!(surface.draw(second), Err(PreparedWorldDrawError));
        surface.draw(new).unwrap();
        assert_eq!(draw.stage_draws().len(), 1);
        assert_eq!(draw.stage_draws()[0].primitives[2], 0..1);
    }

    #[test]
    fn stages_end_where_the_next_begins_and_never_go_back() {
        use RenderLevelStage::*;
        let mut draw = WorldGeometry::default();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.5, 0.5]);
        let line = |depth| WorldLine::new([0.0, 0.0, -1.0], [1.0, 0.0, -1.0], !0, 1.0, depth);
        let quad = WorldQuad::new([[0.0, 0.0, -1.0]; 4], !0, WorldDepth::ThroughWalls);
        // Custom geometry submitted before pass stages lands in STAGE_CUSTOM_GEOMETRY on finalize.
        draw.begin_custom_geometry();
        draw.lines(&[line(WorldDepth::Tested)]).unwrap();
        draw.begin_pass_stage(AfterOpaqueBlocks);
        draw.lines(&[line(WorldDepth::Tested)]).unwrap();
        // Nothing at AfterOpaqueFeatures or AfterTranslucentFeatures; a quad through walls at
        // AfterTranslucentBlocks.
        draw.begin_pass_stage(AfterTranslucentBlocks);
        draw.quads(&[quad]).unwrap();
        // Still in the current stage: what follows a backwards step is not reordered.
        draw.begin_pass_stage(AfterOpaqueBlocks);
        draw.lines(&[line(WorldDepth::ThroughWalls)]).unwrap();
        // OIT sub-passes route independently without disturbing standard stage_ends.
        draw.begin_pass_stage(OitTranslucent(OitStage::DepthBounds));
        draw.lines(&[line(WorldDepth::Tested)]).unwrap();
        draw.begin_pass_stage(OitTranslucent(OitStage::Accumulate));
        draw.quads(&[quad]).unwrap();
        draw.finalize();
        assert_eq!(
            draw.batch().stage_ends,
            [
                [0, 0, 1, 0],
                [0, 0, 1, 0],
                [0, 0, 1, 0],
                [0, 1, 1, 1],
                [0, 1, 1, 1],
                [0, 1, 1, 1],
            ],
            "standard stages end monotonically and custom geometry sits after stage 5"
        );
        assert_eq!(draw.batch().lines.len(), 2);
        assert_eq!(draw.batch().oit.lines.len(), 1);
        assert_eq!(draw.batch().oit.quads_through.len(), 4);
        assert_eq!(draw.batch().oit.stage_ends, [[0, 0, 1, 0], [0, 0, 1, 0]]);
        // A new view starts at the first stage again.
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.5, 0.5]);
        draw.lines(&[line(WorldDepth::Tested)]).unwrap();
        assert_eq!(draw.batch().stage_ends, [[0, 0, 1, 0]; STAGES - 1]);
        assert!(draw.batch().oit.is_empty());
    }

    #[test]
    fn custom_geometry_finalizes_into_tail_stage_without_mixing_into_pass_stages() {
        use RenderLevelStage::*;
        let mut draw = WorldGeometry::default();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.5, 0.5]);
        let line =
            |x: f64, depth| WorldLine::new([x, 0.0, -1.0], [x + 1.0, 0.0, -1.0], !0, 1.0, depth);
        let quad = |x: f64, depth| WorldQuad::new([[x, 0.0, -1.0]; 4], !0, depth);
        let pquad = |x: f64| PipelineQuad::spanning([[x, 0.0, -1.0]; 4], !0);

        // Submit deferred custom geometry before any pass stage runs.
        draw.begin_custom_geometry();
        draw.lines(&[line(90.0, WorldDepth::Tested)]).unwrap();
        draw.lines(&[line(91.0, WorldDepth::ThroughWalls)]).unwrap();
        draw.quads(&[quad(92.0, WorldDepth::Tested)]).unwrap();
        draw.quads(&[quad(93.0, WorldDepth::ThroughWalls)]).unwrap();
        draw.pipeline_quads(7, &[pquad(94.0)]);

        // Deferred custom geometry stays isolated until finalize.
        assert!(draw.batch().is_empty());

        // Standard pass stages 0..=5 populate primary buffers without seeing deferred geometry.
        draw.begin_pass_stage(AfterOpaqueBlocks);
        draw.lines(&[line(10.0, WorldDepth::Tested)]).unwrap();
        draw.quads(&[quad(11.0, WorldDepth::Tested)]).unwrap();
        draw.pipeline_quads(2, &[pquad(12.0)]);

        draw.begin_pass_stage(AfterWeather);
        draw.lines(&[line(20.0, WorldDepth::ThroughWalls)]).unwrap();
        draw.quads(&[quad(21.0, WorldDepth::ThroughWalls)]).unwrap();
        draw.pipeline_quads(3, &[pquad(22.0)]);

        draw.finalize();
        // Calling finalize again is a no-op.
        draw.finalize();

        let batch = draw.batch();
        assert_eq!(
            batch.stage_ends,
            [
                [1, 0, 1, 0],
                [1, 0, 1, 0],
                [1, 0, 1, 0],
                [1, 0, 1, 0],
                [1, 0, 1, 0],
                [1, 1, 1, 1],
            ],
            "pass stages 0..=5 hold only pass-stage writes; custom geometry starts after stage 5"
        );
        assert_eq!(
            batch.lines.iter().map(|l| l[0]).collect::<Vec<_>>(),
            [10.0, 90.0]
        );
        assert_eq!(
            batch.lines_through.iter().map(|l| l[0]).collect::<Vec<_>>(),
            [20.0, 91.0]
        );
        assert_eq!(batch.quads.len(), 8);
        assert_eq!(batch.quads[0].pos[0], 11.0);
        assert_eq!(batch.quads[4].pos[0], 92.0);
        assert_eq!(batch.quads_through.len(), 8);
        assert_eq!(batch.quads_through[0].pos[0], 21.0);
        assert_eq!(batch.quads_through[4].pos[0], 93.0);
        assert_eq!(
            batch.pipelines.runs,
            [
                PipelineRun {
                    slot: 2,
                    stage: STAGE_AFTER_OPAQUE_BLOCKS as u32,
                    quads: 1,
                    modifier_stack: 0,
                },
                PipelineRun {
                    slot: 3,
                    stage: STAGE_AFTER_WEATHER as u32,
                    quads: 1,
                    modifier_stack: 0,
                },
                PipelineRun {
                    slot: 7,
                    stage: STAGE_CUSTOM_GEOMETRY as u32,
                    quads: 1,
                    modifier_stack: 0,
                },
            ]
        );
        assert_eq!(batch.pipelines.vertices.len(), 12);
        assert!(batch.oit.is_empty());
    }

    #[test]
    fn oit_stages_populate_oit_batch_and_leave_standard_batch_untouched() {
        use RenderLevelStage::*;
        let mut draw = WorldGeometry::default();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.5, 0.5]);
        let line =
            |x: f64, depth| WorldLine::new([x, 0.0, -1.0], [x + 1.0, 0.0, -1.0], !0, 1.0, depth);
        let quad = |x: f64, depth| WorldQuad::new([[x, 0.0, -1.0]; 4], !0, depth);
        let pquad = |x: f64| PipelineQuad::spanning([[x, 0.0, -1.0]; 4], !0);

        draw.begin_pass_stage(AfterSky);
        draw.begin_pass_stage(AfterOpaqueBlocks);
        draw.lines(&[line(5.0, WorldDepth::Tested)]).unwrap();
        draw.quads(&[quad(6.0, WorldDepth::Tested)]).unwrap();
        draw.pipeline_quads(1, &[pquad(7.0)]);

        draw.begin_pass_stage(OitTranslucent(OitStage::DepthBounds));
        draw.quads(&[quad(1.0, WorldDepth::Tested)]).unwrap();
        draw.lines(&[line(11.0, WorldDepth::Tested)]).unwrap();

        draw.begin_pass_stage(OitTranslucent(OitStage::Transmittance));
        draw.quads(&[quad(2.0, WorldDepth::ThroughWalls)]).unwrap();
        draw.lines(&[
            line(12.0, WorldDepth::ThroughWalls),
            line(13.0, WorldDepth::Tested),
        ])
        .unwrap();

        draw.begin_pass_stage(OitTranslucent(OitStage::Accumulate));
        draw.quads(&[quad(3.0, WorldDepth::Tested), quad(4.0, WorldDepth::Tested)])
            .unwrap();
        draw.lines(&[line(14.0, WorldDepth::ThroughWalls)]).unwrap();
        // Pipeline quads in OIT sub-passes populate `oit_runs` and leave standard runs alone.
        draw.pipeline_quads(9, &[pquad(99.0)]);

        draw.begin_pass_stage(AfterLevel);
        draw.finalize();

        let batch = draw.batch();
        // Standard batch spans and pipeline runs are completely untouched by OIT writes.
        assert_eq!(batch.stage_ends, [[1, 0, 1, 0]; STAGES - 1]);
        assert_eq!(batch.quads.len(), 4);
        assert_eq!(batch.quads[0].pos[0], 6.0);
        assert!(batch.quads_through.is_empty());
        assert_eq!(batch.lines.len(), 1);
        assert_eq!(batch.lines[0][0], 5.0);
        assert!(batch.lines_through.is_empty());
        assert_eq!(
            batch.pipelines.runs,
            [PipelineRun {
                slot: 1,
                stage: STAGE_AFTER_OPAQUE_BLOCKS as u32,
                quads: 1,
                modifier_stack: 0,
            }]
        );
        assert_eq!(
            batch.pipelines.oit_runs,
            [PipelineRun {
                slot: 9,
                stage: 2,
                quads: 1,
                modifier_stack: 0,
            }]
        );
        assert_eq!(batch.pipelines.oit_vertices.len(), 4);

        // OIT batch has exact per-OIT-stage ends and primitive order.
        assert!(!batch.oit.is_empty());
        assert_eq!(batch.oit.stage_ends, [[1, 0, 1, 0], [1, 1, 2, 1]]);
        assert_eq!(batch.oit.quads.len(), 12);
        assert_eq!(
            [
                batch.oit.quads[0].pos[0],
                batch.oit.quads[4].pos[0],
                batch.oit.quads[8].pos[0],
            ],
            [1.0, 3.0, 4.0]
        );
        assert_eq!(batch.oit.quads_through.len(), 4);
        assert_eq!(batch.oit.quads_through[0].pos[0], 2.0);
        assert_eq!(
            batch.oit.lines.iter().map(|l| l[0]).collect::<Vec<_>>(),
            [11.0, 13.0]
        );
        assert_eq!(
            batch
                .oit
                .lines_through
                .iter()
                .map(|l| l[0])
                .collect::<Vec<_>>(),
            [12.0, 14.0]
        );
    }
    #[test]
    fn world_geometry_batches_are_atomic_and_reset_per_view() {
        let mut draw = WorldGeometry::default();
        draw.reset([30_000_000.0, 20.0, 0.0], Mat4::IDENTITY, [0.25, 0.75]);
        let line = WorldLine::new(
            [30_000_000.25, 20.0, -1.0],
            [30_000_001.0, 20.0, -1.0],
            0x80402010,
            2.0,
            WorldDepth::Tested,
        );
        draw.lines(&[line]).unwrap();
        assert_eq!(
            &draw.batch().lines[0][..6],
            &[0.25, 0.0, -1.0, 1.0, 0.0, -1.0]
        );
        assert_eq!(draw.batch().lines[0][6].to_bits(), line.color);
        let mut bad = line;
        bad.width = 0.0;
        assert_eq!(draw.lines(&[line, bad]), Err(DrawError::InvalidWidth));
        bad.width = f32::NAN;
        assert_eq!(draw.lines(&[line, bad]), Err(DrawError::NonFinite));
        bad = line;
        bad.to[0] = f64::MAX;
        assert_eq!(draw.lines(&[line, bad]), Err(DrawError::NonFinite));
        assert_eq!(draw.batch().lines.len(), 1);
        let quad = WorldQuad::new([line.from; 4], line.color, WorldDepth::ThroughWalls);
        draw.quads(&[quad]).unwrap();
        assert_eq!(draw.batch().quads_through.len(), 4);
        assert_eq!(draw.batch().quads_through[0].pos, [0.25, 0.0, -1.0]);
        assert_eq!(
            draw.batch().quads_through[0].color.to_le_bytes(),
            [0x40, 0x20, 0x10, 0x80]
        );
        let mut invalid_quad = quad;
        invalid_quad.corners[3][1] = f64::INFINITY;
        assert_eq!(draw.quads(&[quad, invalid_quad]), Err(DrawError::NonFinite));
        assert_eq!(draw.batch().quads_through.len(), 4);
        let mut through = line;
        through.depth = WorldDepth::ThroughWalls;
        draw.lines(&vec![through; CAPACITY as usize + 1]).unwrap();
        assert_eq!(draw.batch().lines_through.len(), CAPACITY as usize + 1);
        draw.quads(&vec![quad; CAPACITY as usize + 1]).unwrap();
        assert_eq!(
            draw.batch().quads_through.len(),
            (CAPACITY as usize + 2) * 4
        );
        let capacities = (
            draw.lines.each_ref().map(Vec::capacity),
            draw.quads.each_ref().map(Vec::capacity),
        );
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        assert!(draw.batch().is_empty());
        assert_eq!(draw.quads.iter().map(Vec::len).sum::<usize>(), 0);
        assert_eq!(
            capacities,
            (
                draw.lines.each_ref().map(Vec::capacity),
                draw.quads.each_ref().map(Vec::capacity)
            )
        );
    }

    #[test]
    fn pipeline_quads_skip_non_finite_grow_dynamically_and_run_by_pipeline_and_stage() {
        use RenderLevelStage::*;
        let mut draw = WorldGeometry::default();
        draw.reset([30_000_000.0, 20.0, 0.0], Mat4::IDENTITY, [0.5, 0.5]);
        draw.set_time(12.5);
        let quad = PipelineQuad::new(
            [
                [30_000_000.25, 21.0, -1.0],
                [30_000_000.25, 20.0, -1.0],
                [30_000_001.25, 20.0, -1.0],
                [30_000_001.25, 21.0, -1.0],
            ],
            [[0.0, 0.0], [0.0, 2.0], [3.0, 2.0], [3.0, 0.0]],
            0x80402010,
        );
        draw.begin_pass_stage(AfterOpaqueBlocks);
        draw.pipeline_quads(4, &[quad, quad]);
        // The same pipeline at the same stage joins the run; another pipeline starts one, and
        // so does the first pipeline again after it, to keep the order submitted.
        draw.pipeline_quads(4, &[quad]);
        draw.pipeline_quads(1, &[quad]);
        draw.pipeline_quads(4, &[quad]);
        draw.begin_pass_stage(AfterWeather);
        draw.pipeline_quads(4, &[quad]);
        draw.pipeline_quads(4, &[]);

        let pipelines = draw.batch().pipelines;
        let run = |slot, stage, quads| PipelineRun {
            slot,
            stage,
            quads,
            modifier_stack: 0,
        };
        assert_eq!(
            pipelines.runs,
            [run(4, 0, 3), run(1, 0, 1), run(4, 0, 1), run(4, 5, 1)]
        );
        assert_eq!(pipelines.vertices.len(), 24);
        assert_eq!(pipelines.time, 12.5);
        assert!(pipelines.set.is_none());
        // Camera-relative before narrowing to f32, texture coordinates as given, and the
        // colour's bytes red first.
        let corner = pipelines.vertices[2];
        assert_eq!(corner.pos, [1.25, 0.0, -1.0]);
        assert_eq!(corner.uv, [3.0, 2.0]);
        assert_eq!(corner.color.to_le_bytes(), [0x40, 0x20, 0x10, 0x80]);
        assert!(!draw.batch().is_empty());

        // Non-finite quads are skipped individually without rejecting valid quads in the slice.
        let mut bad_corner = quad;
        bad_corner.corners[3][1] = f64::NAN;
        let mut bad_uv = quad;
        bad_uv.uv[1][0] = f32::INFINITY;
        let mut overflow_corner = quad;
        overflow_corner.corners[0][0] = f64::MAX;
        draw.pipeline_quads(4, &[bad_corner, bad_uv, overflow_corner]);
        assert_eq!(draw.batch().pipelines.vertices.len(), 24);
        draw.pipeline_quads(4, &[quad, bad_corner, bad_uv, quad]);
        assert_eq!(draw.batch().pipelines.vertices.len(), 32);
        assert_eq!(
            draw.batch().pipelines.runs.last(),
            Some(&run(4, STAGE_AFTER_WEATHER as u32, 3))
        );

        // Plain and pipeline quads both grow past the initial hint.
        let plain = WorldQuad::new(quad.corners, !0, WorldDepth::Tested);
        draw.quads(&vec![plain; CAPACITY as usize]).unwrap();
        draw.pipeline_quads(4, &vec![quad; CAPACITY as usize]);
        assert_eq!(
            draw.batch().pipelines.vertices.len(),
            (8 + CAPACITY as usize) * 4
        );
        let capacity = (
            draw.pipeline_quads.capacity(),
            draw.pipeline_runs.capacity(),
        );
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        assert!(draw.batch().is_empty());
        assert_eq!(
            capacity,
            (
                draw.pipeline_quads.capacity(),
                draw.pipeline_runs.capacity()
            ),
            "reset keeps grown capacity for the next view"
        );
    }

    struct NearX;
    impl crate::gfx::Cull for NearX {
        fn intersects(&self, min: glam::Vec3, max: glam::Vec3) -> bool {
            max.x >= 0.0 && min.x <= 4.0
        }
        fn contains(&self, min: glam::Vec3, max: glam::Vec3) -> bool {
            min.x >= 0.0 && max.x <= 4.0
        }
    }

    #[test]
    fn model_batches_cull_first_skip_invalid_visible_and_grow_dynamically() {
        let mut draw = WorldGeometry::default();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        let batch = ModelBatch {
            slot: 3,
            generation: 9,
            bounds: [[0.0; 3], [1.0; 3]],
        };
        let near = ModelInstance::at([1.0, 2.0, 3.0], 0xFF80_C040);
        let far = ModelInstance::at([40.0, 0.0, 0.0], 0xFF80_C040);
        let mut bad_light = near;
        bad_light.light.block = 16;
        let mut bad_origin = near;
        bad_origin.origin[0] = f64::NAN;
        let mut bad_transform = near;
        bad_transform.transform[4] = f32::INFINITY;
        let mut culled_bad_light = far;
        culled_bad_light.light.block = 16;
        // Valid copies survive alongside culled and invalid visible copies; no rollback.
        struct CountedCull(std::cell::Cell<u32>);
        impl crate::gfx::Cull for CountedCull {
            fn intersects(&self, min: Vec3, max: Vec3) -> bool {
                self.0.set(self.0.get() + 1);
                crate::gfx::Cull::intersects(&NearX, min, max)
            }
            fn contains(&self, min: Vec3, max: Vec3) -> bool {
                crate::gfx::Cull::contains(&NearX, min, max)
            }
        }
        let cull = CountedCull(std::cell::Cell::new(0));
        draw.submit_model(
            batch,
            &[
                near,
                far,
                bad_light,
                bad_origin,
                bad_transform,
                culled_bad_light,
            ],
            Some(&cull),
            |_| false,
        );
        assert_eq!(cull.0.get(), 4, "only invalid bounds may skip culling");
        assert_eq!(draw.copy_count(), 1);
        assert_eq!(draw.copy(0).light, 0xF0F0);
        assert_eq!(draw.copy(0).overlay, 0);
        let mut hurt = near;
        hurt.overlay = mod_api::Overlay::new(8, true).unwrap();
        hurt.light = mod_api::LightCoords::new(15, 3).unwrap();
        draw.submit_model(batch, &[hurt], None::<&NearX>, |_| false);
        assert_eq!(draw.copy_count(), 2);
        assert_eq!(draw.copy(1).light, 0xF030);
        assert_eq!(draw.copy(1).overlay, 1);
        // Growth past the initial 16,384 hint succeeds without error.
        draw.submit_model(
            batch,
            &vec![near; INSTANCE_CAPACITY as usize],
            None::<&NearX>,
            |_| false,
        );
        assert_eq!(draw.copy_count(), INSTANCE_CAPACITY as usize + 2);
        let reserved = draw.copies.capacity();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        assert_eq!(draw.copy_count(), 0);
        assert!(draw.copies.capacity() >= reserved);
    }

    #[test]
    fn directly_staged_models_are_not_baked_and_grow_past_initial_hint() {
        let mut draw = WorldGeometry::default();
        let batch = ModelBatch {
            slot: 7,
            generation: 3,
            bounds: [[0.0; 3], [1.0; 3]],
        };
        let copy = ModelInstance::at([1.0, 2.0, 3.0], u32::MAX);
        let mut staged = 0usize;
        draw.submit_model(
            batch,
            &vec![copy; INSTANCE_CAPACITY as usize + 5],
            None::<&crate::gfx::Frustum>,
            |_| {
                staged += 1;
                true
            },
        );
        assert_eq!(staged, INSTANCE_CAPACITY as usize + 5);
        assert_eq!(draw.copy_count(), 0, "direct copies must not also be baked");
    }

    #[test]
    fn affine_culling_matches_the_eight_corner_box() {
        use std::cell::Cell;
        struct Bounds(Cell<[Vec3; 2]>);
        impl crate::gfx::Cull for Bounds {
            fn intersects(&self, min: Vec3, max: Vec3) -> bool {
                self.0.set([min, max]);
                true
            }
            fn contains(&self, _: Vec3, _: Vec3) -> bool {
                false
            }
        }
        let capture = Bounds(Cell::new([Vec3::ZERO; 2]));
        let bounds = [[-0.75, -2.0, 0.125], [1.5, 0.5, 3.0]];
        let origin = [30_000_000.25, -100.5, -30_000_000.75];
        let camera = [30_000_000.0, 5.25, -30_000_000.0];
        let shift = (DVec3::from(origin) - DVec3::from(camera)).as_vec3();
        let mut seed = 42u32;
        for case in 0..1024 {
            // Include shear, reflections, nonuniform scales, and finite inputs that overflow.
            let scale = if case < 512 { 1.0 } else { f32::MAX * 0.25 };
            let transform = std::array::from_fn(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                ((seed >> 8) as f32 / 2097152.0 - 4.0) * scale
            });
            let matrix = row_major_3x4(transform);
            let mut expected = [Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)];
            for corner in 0..8 {
                let point = Vec3::new(
                    bounds[corner & 1][0],
                    bounds[(corner >> 1) & 1][1],
                    bounds[(corner >> 2) & 1][2],
                );
                let point = matrix.transform_point3(point) + shift;
                expected[0] = expected[0].min(point);
                expected[1] = expected[1].max(point);
            }
            let expected_finite = expected.iter().all(|point| point.is_finite())
                && (matrix.w_axis.truncate() + shift).is_finite();
            assert_eq!(
                copy_visible(bounds, transform, origin, camera, Some(&capture)),
                expected_finite
            );
            if expected_finite {
                assert_eq!(capture.0.get(), expected);
            }
        }
    }

    #[test]
    fn extreme_origin_and_overflowing_composed_transform_are_rejected_in_both_paths() {
        let mut draw = WorldGeometry::default();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        let batch = ModelBatch {
            slot: 1,
            generation: 1,
            bounds: [[0.0; 3], [1.0; 3]],
        };
        let valid = ModelInstance::at([1.0, 2.0, 3.0], 0xFFFF_FFFF);
        let extreme_origin = ModelInstance::at([f64::MAX, 0.0, 0.0], 0xFFFF_FFFF);
        let mut overflowing_bounds = valid;
        overflowing_bounds.transform = [
            f32::MAX,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            f32::MAX,
        ];
        overflowing_bounds.origin = [0.0, 0.0, 1e38];
        let mut non_finite_composed = valid;
        non_finite_composed.transform[0] = f32::INFINITY;

        // Transient path rejects extreme origins and overflowing bounds with or without a frustum.
        draw.submit_model(
            batch,
            &[extreme_origin, overflowing_bounds, valid],
            Some(&NearX),
            |_| false,
        );
        assert_eq!(draw.copy_count(), 1);
        assert_eq!(draw.copy(0).origin, valid.origin);

        draw.clear_copies();
        draw.submit_model(
            batch,
            &[extreme_origin, overflowing_bounds, valid],
            None::<&NearX>,
            |_| false,
        );
        assert_eq!(draw.copy_count(), 1);
        assert_eq!(draw.copy(0).origin, valid.origin);

        // Retained path (`submit_validated_model`) rejects non-finite composed transforms and
        // extreme camera-relative origins just like the transient path.
        draw.clear_copies();
        draw.submit_validated_model(
            batch,
            &[
                extreme_origin,
                overflowing_bounds,
                non_finite_composed,
                valid,
            ],
            Some(&NearX),
            |_, _| false,
        );
        assert_eq!(draw.copy_count(), 1);
        assert_eq!(draw.copy(0).origin, valid.origin);

        draw.clear_copies();
        draw.submit_validated_model(
            batch,
            &[
                extreme_origin,
                overflowing_bounds,
                non_finite_composed,
                valid,
            ],
            None::<&NearX>,
            |_, _| false,
        );
        assert_eq!(draw.copy_count(), 1);
        assert_eq!(draw.copy(0).origin, valid.origin);
    }

    #[test]
    fn labels_grow_dynamically_and_reset_per_view() {
        let mut draw = WorldGeometry::default();
        draw.reset([30_000_000.0, 20.0, 0.0], Mat4::IDENTITY, [0.5, 0.5]);
        assert_eq!(draw.check_label([30_000_000.5, 70.0, 0.5]), Ok(()));
        for at in [
            [f64::NAN, 70.0, 0.5],
            [0.5, f64::INFINITY, 0.5],
            [f64::MAX, 70.0, 0.5],
        ] {
            assert_eq!(draw.check_label(at), Err(DrawError::NonFinite));
        }
        let quad = [<Vertex as bytemuck::Zeroable>::zeroed(); 4];
        // One glyph in the world, and its see-through copy on a backdrop: three quads.
        draw.label(1.0, &quad, &[quad, quad].concat());
        assert_eq!(draw.labels().map(<[Vertex]>::len), [4, 8]);
        let world = WorldQuad::new([[30_000_000.0, 20.0, -1.0]; 4], !0, WorldDepth::Tested);
        draw.quads(&vec![world; CAPACITY as usize]).unwrap();
        // Labels grow past the initial 4,096 quad hint alongside plain quads.
        draw.label(1.0, &quad, &quad);
        assert_eq!(draw.labels().map(<[Vertex]>::len), [8, 12]);
        let capacities = draw.labels.each_ref().map(Vec::capacity);
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        assert_eq!(draw.labels().map(<[Vertex]>::len), [0, 0]);
        assert_eq!(draw.quads.iter().map(Vec::len).sum::<usize>(), 0);
        assert_eq!(capacities, draw.labels.each_ref().map(Vec::capacity));
    }

    #[test]
    fn labels_draw_far_to_near_and_equal_distances_keep_their_order() {
        let mut draw = WorldGeometry::default();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.5, 0.5]);
        // Each label's quad is told apart by its colour; two quads for the second.
        let quad = |color| {
            [Vertex {
                color,
                ..<Vertex as bytemuck::Zeroable>::zeroed()
            }; 4]
        };
        let colors = |draw: &WorldGeometry| -> Vec<u32> {
            draw.labels()[0]
                .iter()
                .step_by(4)
                .map(|v| v.color)
                .collect()
        };
        draw.label(4.0, &quad(1), &[]);
        draw.label(16.0, &[quad(2), quad(2)].concat(), &quad(9));
        draw.label(4.0, &quad(3), &[]);
        draw.label(1.0, &quad(4), &[]);
        draw.sort_labels();
        assert_eq!(colors(&draw), [2, 2, 1, 3, 4]);
        assert_eq!(
            draw.labels()[1].len(),
            4,
            "see-through copies are not reordered"
        );
        // Sorted runs stay sorted when labels are added and sorted again.
        draw.label(9.0, &quad(5), &[]);
        draw.sort_labels();
        assert_eq!(colors(&draw), [2, 2, 5, 1, 3, 4]);
        assert_eq!(draw.labels()[0].len() + draw.labels()[1].len(), 28);
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.5, 0.5]);
        draw.sort_labels();
        assert!(draw.labels()[0].is_empty());
    }

    #[test]
    fn repeated_empty_or_culled_submissions_do_not_block_visible_draws() {
        let mut draw = WorldGeometry::default();
        draw.reset([0.0; 3], Mat4::IDENTITY, [0.0; 2]);
        let batch = ModelBatch {
            slot: 3,
            generation: 9,
            bounds: [[0.0; 3], [1.0; 3]],
        };
        let near = ModelInstance::at([1.0, 2.0, 3.0], 0xFF80_C040);
        let far = ModelInstance::at([40.0, 0.0, 0.0], 0xFF80_C040);
        for _ in 0..=INSTANCE_CAPACITY {
            draw.submit_validated_model(batch, &[], Some(&NearX), |_, _| false);
            draw.submit_validated_model(batch, &[far], Some(&NearX), |_, _| false);
        }
        assert_eq!(draw.copy_count(), 0);
        draw.submit_validated_model(batch, &[near], Some(&NearX), |_, _| false);
        assert_eq!(draw.copy_count(), 1);
    }
}
