//! Compute pipelines a client mod registers as plain data, and the dispatches it records with
//! them (`RegisterRenderPipelinesEvent::compute_pipeline`, [`ComputeDispatchContext`]).
//!
//! Vanilla 26.3's `renderpearl` API has no compute pass, so the shape follows what it does have:
//! a pipeline is data built from a shader and a [`BindGroupLayout`] (`BindGroupLayout` /
//! `RenderPipeline.Builder.withShaderDefine`), bound through [`DrawBindings`] like a draw's, and
//! registered on the same registrar NeoForge's `RegisterRenderPipelinesEvent` gives render
//! pipelines.

use crate::client::render_pipeline::{is_valid_glsl_define_value, is_valid_glsl_identifier};
use crate::{
    BindGroupLayout, BindGroupLayoutError, BufferError, DrawBindings, FramePassId, FramePassRef,
    FrameTarget, Identifier, RenderLevelStage, StorageBufferRef,
};

/// A compute pipeline registered with `event.compute_pipeline(..)` in
/// `Hooks::register_render_pipelines`: a GLSL 450 compute shader
/// (`assets/<ns>/shaders/compute/<path>.csh`) and the [`BindGroupLayout`] it binds, translated by
/// `sunrise-glsl` when resources load, again on every resource reload.
///
/// The layout is the pipeline's only bind group: uniform buffers, sampled textures, texel
/// buffers, std430 storage buffers and `image2D` storage images, bound per dispatch with
/// [`DrawBindings`]. A shader writing a storage buffer declares it without `readonly` and the
/// layout says `read_only: false`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client compute-pipeline registration descriptor.
pub struct ComputePipeline {
    owner: String,
    /// The pipeline's own name (`mymod:hiz_reduce`), unique among mod compute pipelines.
    pub id: Identifier,
    /// The compute shader: `mymod:hiz_reduce` resolves to
    /// `assets/mymod/shaders/compute/hiz_reduce.csh`, as the mod's assets or a resource pack
    /// supply it. `.csh` is the one extension (Iris/OptiFine's compute suffix, beside
    /// `core/<path>.vsh` and `.fsh`).
    pub shader: Identifier,
    /// Preprocessor defines `(name, value)`: each `name` a valid non-`GL_` GLSL identifier, each
    /// `value` (when present) a non-empty single-line token without comments, backslashes or
    /// `#`.
    pub defines: Vec<(String, Option<String>)>,
    /// The entries the shader binds, in one bind group.
    pub bind_group_layout: BindGroupLayout,
}

impl ComputePipeline {
    /// Start describing the compute pipeline `id` that runs the shader `shader`.
    pub fn builder(id: Identifier, shader: Identifier) -> ComputePipelineBuilder {
        ComputePipelineBuilder {
            pipeline: Self {
                owner: String::new(),
                id,
                shader,
                defines: Vec::new(),
                bind_group_layout: BindGroupLayout::default(),
            },
        }
    }

    /// The owning mod, assigned by the host when the pipeline is registered.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub(crate) fn set_owner(&mut self, owner: &str) {
        owner.clone_into(&mut self.owner);
    }

    /// Check the declared defines and layout, as the engine does when it freezes registrations.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in &self.defines {
            if !is_valid_glsl_identifier(name) || name.starts_with("GL_") {
                return Err(format!(
                    "invalid shader define name {name:?}: expected a valid non-GL_ GLSL identifier"
                ));
            }
            if let Some(value) = value
                && !is_valid_glsl_define_value(value)
            {
                return Err(format!(
                    "invalid shader define value for {name}: {value:?} must be a non-empty single-line token without comments, backslashes, or '#'"
                ));
            }
        }
        self.bind_group_layout
            .validate(false)
            .map_err(|e: BindGroupLayoutError| format!("bind group layout: {e:?}"))
    }
}

/// Builds a [`ComputePipeline`] ([`ComputePipeline::builder`]).
#[derive(Clone, Debug)]
#[non_exhaustive]
// no wire: local client compute-pipeline builder.
pub struct ComputePipelineBuilder {
    pipeline: ComputePipeline,
}

impl ComputePipelineBuilder {
    /// The entries the shader binds ([`ComputePipeline::bind_group_layout`]).
    pub fn bind_group_layout(mut self, layout: BindGroupLayout) -> Self {
        self.pipeline.bind_group_layout = layout;
        self
    }

    /// Define a preprocessor flag `#define NAME` (vanilla's `withShaderDefine(key)`).
    pub fn define(mut self, name: impl Into<String>) -> Self {
        self.pipeline.defines.push((name.into(), None));
        self
    }

    /// Define a preprocessor macro `#define NAME VALUE` (vanilla's
    /// `withShaderDefine(key, value)`).
    pub fn define_value(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.pipeline
            .defines
            .push((name.into(), Some(value.into())));
        self
    }

    /// The pipeline description; the engine validates it when it freezes registrations.
    pub fn build(self) -> ComputePipeline {
        self.pipeline
    }
}

/// A compute node of the frame graph, added during [`crate::Hooks::frame_graph_setup`] with
/// [`crate::FrameGraphContext::add_compute_pass`]. It has no render target: the graph orders it
/// by the targets it `reads` and `writes` (a mod render target it binds as a storage image or a
/// sampled texture, or a transient it samples) and by `after` and `before`, and it is never
/// culled, since a dispatch may write a storage buffer no target reflects.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client frame-graph compute pass descriptor.
pub struct ComputePassDescriptor {
    /// Human-readable name for GPU debug labels and cycle diagnostics.
    pub label: String,
    /// Targets the dispatches read.
    pub reads: Vec<FrameTarget>,
    /// Targets the dispatches write.
    pub writes: Vec<FrameTarget>,
    /// Passes or engine anchors that must execute before this node.
    pub after: Vec<FramePassRef>,
    /// Passes or engine anchors that must execute after this node.
    pub before: Vec<FramePassRef>,
}

impl ComputePassDescriptor {
    /// Start describing a compute node named `label`.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            reads: Vec::new(),
            writes: Vec::new(),
            after: Vec::new(),
            before: Vec::new(),
        }
    }

    /// Declare that the dispatches read `target`.
    pub fn reads(mut self, target: FrameTarget) -> Self {
        self.reads.push(target);
        self
    }

    /// Declare that the dispatches write `target`.
    pub fn writes(mut self, target: FrameTarget) -> Self {
        self.writes.push(target);
        self
    }

    /// Require engine anchor `stage` to execute before this node.
    pub fn after_stage(mut self, stage: RenderLevelStage) -> Self {
        self.after.push(FramePassRef::Anchor(stage));
        self
    }

    /// Require this node to execute before engine anchor `stage`.
    pub fn before_stage(mut self, stage: RenderLevelStage) -> Self {
        self.before.push(FramePassRef::Anchor(stage));
        self
    }

    /// Require mod pass `pass` to execute before this node.
    pub fn after_pass(mut self, pass: FramePassId) -> Self {
        self.after.push(FramePassRef::Pass(pass));
        self
    }

    /// Require this node to execute before mod pass `pass`.
    pub fn before_pass(mut self, pass: FramePassId) -> Self {
        self.before.push(FramePassRef::Pass(pass));
        self
    }
}

/// A registered compute pipeline as the current resources built it, resolved through
/// [`crate::RenderAssets::compute_pipeline`]. Retain it between frames; resolve it again after a
/// successful resource reload.
// no wire: process-local handle to a client compute pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private fields; only the engine resolves pipeline handles
pub struct ComputePipelineRef(pub(crate) u64, pub(crate) u32);

impl ComputePipelineRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished compute pipeline generation");
        }
        Ok(Self(generation, slot))
    }
}

/// Why a compute pipeline could not be resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client compute-pipeline lookup.
pub enum ComputePipelineError {
    /// No mod registered a compute pipeline of this id.
    Unknown,
    /// The pipeline is registered, but the current resources give it no usable shader, or the
    /// device refuses it (workgroup size, workgroup storage or binding counts over the device's
    /// limits). The log names the pipeline and the reason.
    Invalid,
    /// The shader uses subgroup operations and the adapter lacks them
    /// ([`crate::RenderCapabilities::subgroup`]): never a silent wrong result.
    UnsupportedFeature,
    /// A successful resource reload superseded the handle's generation.
    Stale,
}

/// Why a compute dispatch was refused; nothing was recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client compute dispatch.
pub enum ComputeDispatchError {
    /// No dispatch can be recorded in this context.
    Unavailable,
    /// The pipeline handle is from before a resource reload, or its pipeline did not build.
    Pipeline(ComputePipelineError),
    /// A binding is missing, of the wrong kind, stale, or not allowed for this dispatch.
    InvalidBinding,
    /// A bound buffer range or an indirect argument buffer was refused.
    Buffer(BufferError),
    /// A workgroup count exceeds `max_compute_workgroups_per_dimension`.
    TooManyWorkgroups,
    /// The indirect buffer was not created with `indirect: true`, `offset` is not a multiple of
    /// 4, or `offset + 12` lies beyond the buffer.
    InvalidIndirect,
}

/// Records compute dispatches, executed outside any render pass at the point of the frame the
/// context belongs to: a frame-graph compute pass
/// ([`crate::FrameGraphContext::add_compute_pass`]) runs them at its place in the graph.
/// wgpu orders a dispatch's writes before later passes that read them.
pub trait ComputeDispatchContext {
    /// Dispatch `workgroups` of `pipeline` with `bindings`. A zero on any axis is a legal no-op.
    /// Each axis must be at most `max_compute_workgroups_per_dimension`.
    fn dispatch(
        &mut self,
        pipeline: ComputePipelineRef,
        bindings: &DrawBindings<'_>,
        workgroups: [u32; 3],
    ) -> Result<(), ComputeDispatchError> {
        let _ = (pipeline, bindings, workgroups);
        Err(ComputeDispatchError::Unavailable)
    }

    /// Dispatch with the three `u32` workgroup counts (x, y, z) stored at `offset` of
    /// `indirect`, read when the dispatch runs. `indirect` must have been created with
    /// [`crate::StorageBufferDescriptor::indirect`], `offset` is a multiple of 4 and
    /// `offset + 12` at most the buffer's size.
    fn dispatch_indirect(
        &mut self,
        pipeline: ComputePipelineRef,
        bindings: &DrawBindings<'_>,
        indirect: StorageBufferRef,
        offset: u64,
    ) -> Result<(), ComputeDispatchError> {
        let _ = (pipeline, bindings, indirect, offset);
        Err(ComputeDispatchError::Unavailable)
    }
}

impl crate::RegisterRenderPipelinesEvent<'_> {
    /// Register one compute pipeline without exposing earlier registrations.
    ///
    /// The entry is attributed to this listener immediately. Duplicate IDs, invalid defines or
    /// layouts, and a layout the device cannot bind are refused when the engine freezes
    /// registrations at startup.
    pub fn compute_pipeline(&mut self, mut entry: ComputePipeline) {
        entry.set_owner(self.mod_id);
        self.compute_pipelines.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BindEntry, BindingKind, RenderPipeline, Sampler, SamplerAddress, SamplerFilter};

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    #[test]
    fn the_builder_keeps_what_it_is_given_and_the_event_stamps_the_owner() {
        let layout = BindGroupLayout::builder()
            .entry(BindEntry::new(
                "Counts",
                BindingKind::StorageBuffer {
                    read_only: false,
                    min_size: 4,
                },
            ))
            .build()
            .unwrap();
        let pipeline = ComputePipeline::builder(id("mine:reduce"), id("mine:reduce"))
            .bind_group_layout(layout.clone())
            .define("TILE")
            .define_value("LEVELS", "4")
            .build();
        assert_eq!(pipeline.bind_group_layout, layout);
        assert_eq!(
            pipeline.defines,
            [
                ("TILE".to_owned(), None),
                ("LEVELS".to_owned(), Some("4".to_owned()))
            ]
        );
        assert_eq!(pipeline.validate(), Ok(()));

        let mut compute = Vec::new();
        let mut pipelines: Vec<RenderPipeline> = Vec::new();
        let mut render_types = Vec::new();
        let mut event = crate::RegisterRenderPipelinesEvent {
            mod_id: "owner_mod",
            pipelines: &mut pipelines,
            render_types: &mut render_types,
            compute_pipelines: &mut compute,
        };
        event.compute_pipeline(pipeline);
        assert_eq!(compute.len(), 1);
        assert_eq!(compute[0].owner(), "owner_mod");
    }

    #[test]
    fn validation_refuses_bad_defines_and_layouts() {
        let bad_name = ComputePipeline::builder(id("mine:a"), id("mine:a"))
            .define("GL_VENDOR")
            .build();
        assert!(bad_name.validate().is_err());
        let bad_value = ComputePipeline::builder(id("mine:a"), id("mine:a"))
            .define_value("N", "1 // two")
            .build();
        assert!(bad_value.validate().is_err());
        // The layout builder refuses a name vanilla reserves.
        assert!(
            BindGroupLayout::builder()
                .sampler(
                    "Globals",
                    Sampler::new(SamplerFilter::Nearest, SamplerAddress::ClampToEdge)
                )
                .build()
                .is_err()
        );
    }

    #[test]
    fn a_context_with_no_backend_refuses_dispatches() {
        struct Nothing;
        impl ComputeDispatchContext for Nothing {}
        let handle = ComputePipelineRef::from_parts(1, 0).unwrap();
        assert!(ComputePipelineRef::from_parts(0, 0).is_err());
        assert_eq!(
            Nothing.dispatch(handle, &DrawBindings::EMPTY, [1, 1, 1]),
            Err(ComputeDispatchError::Unavailable)
        );
    }
}
