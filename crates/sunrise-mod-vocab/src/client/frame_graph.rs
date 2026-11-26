//! Client frame-graph pass descriptors, targets, and errors (`#963`, NeoForge 26.3's
//! `FrameGraphSetupEvent`, vanilla `FrameGraphBuilder` / `LevelTargetBundle`).

use crate::{OffscreenPass, RenderLevelStage, RenderTargetError, RenderTargetRef};

/// Opaque per-frame handle for a transient offscreen render target allocated during
/// [`crate::Hooks::frame_graph_setup`] (`FrameGraphBuilder.createInternal`).
///
/// Transient targets are pooled across frames (`CrossFrameResourcePool`) and released
/// automatically after the last pass in the compiled frame graph that reads them. Handles are
/// valid only for the frame in which they were created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// no wire: per-frame transient GPU target handle.
// exhaustive: private field; only the engine mints handles
pub struct TransientTargetId(u32);

impl TransientTargetId {
    /// Construct a transient target handle from a raw per-frame index.
    // hidden: the engine mints handles; a mod only receives them
    #[doc(hidden)]
    pub const fn from_raw(index: u32) -> Self {
        Self(index)
    }

    /// Raw per-frame index of this transient target.
    // hidden: the engine reads the index; a mod only passes handles back
    #[doc(hidden)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Opaque per-frame handle identifying a mod pass added during
/// [`crate::Hooks::frame_graph_setup`] (`FrameGraphBuilder.addPass`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// no wire: per-frame pass identifier.
// exhaustive: private field; only the engine mints handles
pub struct FramePassId(u32);

impl FramePassId {
    /// Construct a pass handle from a raw per-frame index.
    // hidden: the engine mints handles; a mod only receives them
    #[doc(hidden)]
    pub const fn from_raw(index: u32) -> Self {
        Self(index)
    }

    /// Raw per-frame index of this pass.
    // hidden: the engine reads the index; a mod only passes handles back
    #[doc(hidden)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Named render target referenced in [`FramePassDescriptor::reads`],
/// [`FramePassDescriptor::writes`], or [`FramePassDescriptor::target`].
///
/// Built-in variants match vanilla 26.3's `LevelTargetBundle` (`main`, `entity_outline`, and the
/// order-independent transparency targets). Mod-owned [`RenderTargetRef`] targets are imported
/// external resources (`FrameGraphBuilder.importExternal`) and are never culled; transient
/// targets (`FrameGraphBuilder.createInternal`) are pooled per frame and culled when unused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client frame-graph target identifier.
pub enum FrameTarget {
    /// `LevelTargetBundle.main` colour attachment (or an active shader pack's `colortex0` before
    /// `final` composition).
    MainColor,
    /// `LevelTargetBundle.main` depth attachment (or an active shader pack's `depthtex0` before
    /// `final` composition).
    MainDepth,
    /// `LevelTargetBundle.entityOutline` (`minecraft:entity_outline`).
    EntityOutline,
    /// `LevelTargetBundle.alwaysOnTopDepth` (OIT always-on-top depth target).
    OitAlwaysOnTopDepth,
    /// `LevelTargetBundle.depthBounds` (OIT depth-bounds target).
    OitDepthBounds,
    /// `LevelTargetBundle.depthBoundsCulled` (OIT culled depth-bounds target).
    OitDepthBoundsCulled,
    /// `LevelTargetBundle.transmittance[index]` (OIT transmittance target slice).
    OitTransmittance(u8),
    /// `LevelTargetBundle.accumulate` (OIT weighted colour accumulation target).
    OitAccumulate,
    /// `LevelTargetBundle.oitCloudDepth` (OIT cloud depth target).
    OitCloudDepth,
    /// `LevelTargetBundle.oitTerrainWithWaterPatchDepth` (OIT terrain + water-patch depth target).
    OitTerrainWithWaterPatchDepth,
    /// Imported mod-owned offscreen render target (`FrameGraphBuilder.importExternal`). Always
    /// treated as an external output and never culled.
    Mod(RenderTargetRef),
    /// Transient per-frame offscreen render target allocated via
    /// [`crate::FrameGraphContext::create_transient`] (`FrameGraphBuilder.createInternal`).
    Transient(TransientTargetId),
}

impl FrameTarget {
    /// Whether this target is an external frame resource (`FrameGraphBuilder.externalResources`):
    /// built-in `LevelTargetBundle` targets and imported mod [`RenderTargetRef`] targets are
    /// external and keep their producing passes alive; [`Self::Transient`] targets are internal
    /// and may be culled when no kept pass reads them.
    pub const fn is_external(self) -> bool {
        !matches!(self, Self::Transient(_))
    }
}

/// An ordering reference in [`FramePassDescriptor::after`] or [`FramePassDescriptor::before`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client frame-graph ordering reference.
pub enum FramePassRef {
    /// An engine world-render anchor, reusing the existing [`RenderLevelStage`] anchors
    /// (`AfterSky`, `AfterOpaqueBlocks`, `AfterOpaqueFeatures`, `AfterTranslucentFeatures`,
    /// `AfterTranslucentBlocks`, `AfterTranslucentParticles`, `AfterWeather`,
    /// `OitTranslucent`, `AfterLevel`).
    Anchor(RenderLevelStage),
    /// Another mod pass added in the same frame's [`crate::Hooks::frame_graph_setup`].
    Pass(FramePassId),
}

/// Data-driven description of one mod offscreen render pass added during
/// [`crate::Hooks::frame_graph_setup`] (`FrameGraphBuilder.addPass` / `FramePass`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client frame-graph pass descriptor.
pub struct FramePassDescriptor {
    /// Human-readable pass name used for GPU debug labels and cycle-rejection diagnostics
    /// (`FrameGraphBuilder.addPass(name)`).
    pub label: String,
    /// Render target this pass draws into ([`FrameTarget::Mod`] or [`FrameTarget::Transient`]).
    pub target: FrameTarget,
    /// Per-attachment colour clear values (`None` = `LoadOp::Load` preserving existing contents;
    /// `Some([r, g, b, a])` = `LoadOp::Clear`). Missing trailing entries default to `Load`.
    pub clear_color: Vec<Option<[f32; 4]>>,
    /// Optional depth clear value (`None` = `LoadOp::Load`; `Some(d)` = `LoadOp::Clear(d)`).
    pub clear_depth: Option<f32>,
    /// Optional stencil clear value (`None` = `LoadOp::Load`; `Some(s)` = `LoadOp::Clear(s)`).
    pub clear_stencil: Option<u32>,
    /// Optional column-major 4x4 projection matrix overriding `Projection.ProjMat` for this pass.
    pub projection: Option<[f32; 16]>,
    /// Optional column-major 4x4 model-view matrix overriding `DynamicTransforms.ModelViewMat`
    /// for this pass.
    pub view: Option<[f32; 16]>,
    /// Targets read by this pass (`FramePass.reads`).
    pub reads: Vec<FrameTarget>,
    /// Additional targets written by this pass (`FramePass.readsAndWrites`; `target` is
    /// implicitly written).
    pub writes: Vec<FrameTarget>,
    /// Passes or engine anchors that must execute before this pass (`FramePass.requires`).
    pub after: Vec<FramePassRef>,
    /// Passes or engine anchors that must execute after this pass.
    pub before: Vec<FramePassRef>,
    /// Disable dead-pass culling for this pass (`FramePass.disableCulling`).
    pub disable_culling: bool,
    /// The mip level of a [`FrameTarget::Mod`] target's attachments the pass renders into
    /// (default 0), at that level's size, as [`OffscreenPass::mip_level`]. Must be below the
    /// target's [`RenderTargetDescriptor::mip_levels`](crate::RenderTargetDescriptor::mip_levels);
    /// a transient target has one level. A level out of range refuses the pass with
    /// [`FrameGraphError::InvalidTarget`] of [`RenderTargetError::InvalidMipLevels`].
    pub mip_level: u32,
}

impl FramePassDescriptor {
    /// Start describing a pass named `label` that draws into `target` while preserving existing
    /// attachment contents.
    pub fn new(label: impl Into<String>, target: FrameTarget) -> Self {
        Self {
            label: label.into(),
            target,
            clear_color: Vec::new(),
            clear_depth: None,
            clear_stencil: None,
            projection: None,
            view: None,
            reads: Vec::new(),
            writes: Vec::new(),
            after: Vec::new(),
            before: Vec::new(),
            disable_culling: false,
            mip_level: 0,
        }
    }

    /// Build a [`FramePassDescriptor`] from an [`OffscreenPass`] targeting an imported
    /// [`RenderTargetRef`].
    pub fn offscreen(label: impl Into<String>, pass: OffscreenPass) -> Self {
        Self {
            label: label.into(),
            target: FrameTarget::Mod(pass.target),
            clear_color: pass.clear_color,
            clear_depth: pass.clear_depth,
            clear_stencil: pass.clear_stencil,
            projection: pass.projection,
            view: pass.view,
            reads: Vec::new(),
            writes: Vec::new(),
            after: Vec::new(),
            before: Vec::new(),
            disable_culling: false,
            mip_level: pass.mip_level,
        }
    }

    /// Set per-attachment colour clear operations.
    pub fn clear_color(mut self, clears: impl IntoIterator<Item = Option<[f32; 4]>>) -> Self {
        self.clear_color = clears.into_iter().collect();
        self
    }

    /// Clear colour attachment 0 to `rgba`.
    pub fn clear_color_0(mut self, rgba: [f32; 4]) -> Self {
        if self.clear_color.is_empty() {
            self.clear_color.push(Some(rgba));
        } else {
            self.clear_color[0] = Some(rgba);
        }
        self
    }

    /// Clear the depth attachment to `depth`.
    pub fn clear_depth(mut self, depth: f32) -> Self {
        self.clear_depth = Some(depth);
        self
    }

    /// Clear the stencil attachment to `stencil`.
    pub fn clear_stencil(mut self, stencil: u32) -> Self {
        self.clear_stencil = Some(stencil);
        self
    }

    /// Override `Projection.ProjMat` (column-major 4x4) for draws inside this pass.
    pub fn projection(mut self, matrix: [f32; 16]) -> Self {
        self.projection = Some(matrix);
        self
    }

    /// Override `DynamicTransforms.ModelViewMat` (column-major 4x4) for draws inside this pass.
    pub fn view(mut self, matrix: [f32; 16]) -> Self {
        self.view = Some(matrix);
        self
    }

    /// Declare that this pass reads `target` (`FramePass.reads`).
    pub fn reads(mut self, target: FrameTarget) -> Self {
        self.reads.push(target);
        self
    }

    /// Declare an additional target written by this pass (`FramePass.readsAndWrites`).
    pub fn writes(mut self, target: FrameTarget) -> Self {
        self.writes.push(target);
        self
    }

    /// Require `dep` to execute before this pass (`FramePass.requires`).
    pub fn after(mut self, dep: FramePassRef) -> Self {
        self.after.push(dep);
        self
    }

    /// Require this pass to execute before `dep`.
    pub fn before(mut self, dep: FramePassRef) -> Self {
        self.before.push(dep);
        self
    }

    /// Require engine anchor `stage` to execute before this pass.
    pub fn after_stage(self, stage: RenderLevelStage) -> Self {
        self.after(FramePassRef::Anchor(stage))
    }

    /// Require this pass to execute before engine anchor `stage`.
    pub fn before_stage(self, stage: RenderLevelStage) -> Self {
        self.before(FramePassRef::Anchor(stage))
    }

    /// Require mod pass `pass` to execute before this pass.
    pub fn after_pass(self, pass: FramePassId) -> Self {
        self.after(FramePassRef::Pass(pass))
    }

    /// Require this pass to execute before mod pass `pass`.
    pub fn before_pass(self, pass: FramePassId) -> Self {
        self.before(FramePassRef::Pass(pass))
    }

    /// Disable dead-pass culling for this pass (`FramePass.disableCulling`).
    pub fn disable_culling(mut self) -> Self {
        self.disable_culling = true;
        self
    }

    /// Render into mip level `level` of the target ([`Self::mip_level`]).
    pub fn mip_level(mut self, level: u32) -> Self {
        self.mip_level = level;
        self
    }
}

/// Why a frame-graph pass or graph compilation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client frame-graph error.
pub enum FrameGraphError {
    /// Pass label was empty.
    EmptyLabel,
    /// Imported [`RenderTargetRef`] handle was stale, not owned by this mod, or unavailable.
    InvalidTarget(RenderTargetError),
    /// Transient target handle was not allocated in this frame.
    UnknownTransientTarget(TransientTargetId),
    /// Ordering dependency referenced a [`FramePassId`] not added in this frame.
    UnknownPass(FramePassId),
    /// Pass declared a self-dependency or reads and writes the same render target in one pass.
    SelfDependency,
    /// Adding the pass or resolving the frame graph created a dependency cycle
    /// (`FrameGraphBuilder`: `"Frame graph cycle detected between ..."`).
    Cycle {
        /// Labels of the passes and/or anchors involved in the cycle, in deterministic order.
        passes: Vec<String>,
    },
    /// Target cannot be used as the write target of a mod offscreen pass (built-in
    /// `LevelTargetBundle` targets are read via `reads` and drawn into via `render_level_stage`).
    UnsupportedTarget(FrameTarget),
    /// Frame-graph context is unavailable.
    Unavailable,
}

impl std::fmt::Display for FrameGraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyLabel => write!(f, "frame graph pass label must not be empty"),
            Self::InvalidTarget(err) => write!(f, "invalid frame graph render target: {err}"),
            Self::UnknownTransientTarget(id) => {
                write!(f, "unknown transient frame target #{}", id.raw())
            }
            Self::UnknownPass(id) => write!(f, "unknown frame graph pass #{}", id.raw()),
            Self::SelfDependency => {
                write!(f, "frame graph pass cannot depend on or self-sample itself")
            }
            Self::Cycle { passes } => {
                write!(
                    f,
                    "frame graph cycle detected between {}",
                    passes.join(", ")
                )
            }
            Self::UnsupportedTarget(target) => {
                write!(
                    f,
                    "frame target {target:?} cannot be used as an offscreen mod pass target"
                )
            }
            Self::Unavailable => write!(f, "frame graph context is unavailable"),
        }
    }
}

impl std::error::Error for FrameGraphError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_graph_types_are_not_wire_values() {
        trait Local<Marker> {
            fn assert_local() {}
        }
        impl<T> Local<()> for T {}
        impl<T: crate::wire::Wire> Local<u8> for T {}

        let _ = <TransientTargetId as Local<_>>::assert_local;
        let _ = <FramePassId as Local<_>>::assert_local;
        let _ = <FrameTarget as Local<_>>::assert_local;
        let _ = <FramePassRef as Local<_>>::assert_local;
        let _ = <FramePassDescriptor as Local<_>>::assert_local;
        let _ = <FrameGraphError as Local<_>>::assert_local;
    }

    #[test]
    fn frame_target_external_classification_matches_level_target_bundle_and_ruling() {
        let rt = RenderTargetRef::from_parts(1, 0).unwrap();
        let tmp = TransientTargetId::from_raw(0);
        assert!(FrameTarget::MainColor.is_external());
        assert!(FrameTarget::MainDepth.is_external());
        assert!(FrameTarget::EntityOutline.is_external());
        assert!(FrameTarget::OitAlwaysOnTopDepth.is_external());
        assert!(FrameTarget::OitDepthBounds.is_external());
        assert!(FrameTarget::OitDepthBoundsCulled.is_external());
        assert!(FrameTarget::OitTransmittance(0).is_external());
        assert!(FrameTarget::OitAccumulate.is_external());
        assert!(FrameTarget::OitCloudDepth.is_external());
        assert!(FrameTarget::OitTerrainWithWaterPatchDepth.is_external());
        assert!(FrameTarget::Mod(rt).is_external());
        assert!(!FrameTarget::Transient(tmp).is_external());
    }

    #[test]
    fn frame_pass_descriptor_builder_records_ordering_and_clears() {
        let rt = RenderTargetRef::from_parts(2, 1).unwrap();
        let p0 = FramePassId::from_raw(0);
        let desc = FramePassDescriptor::offscreen(
            "far_composite",
            OffscreenPass::new(rt)
                .clear_color_0([0.0, 0.0, 0.0, 1.0])
                .clear_depth(0.0),
        )
        .reads(FrameTarget::MainDepth)
        .after_stage(RenderLevelStage::AfterOpaqueBlocks)
        .before_stage(RenderLevelStage::AfterTranslucentBlocks)
        .after_pass(p0)
        .disable_culling();

        assert_eq!(desc.label, "far_composite");
        assert_eq!(desc.target, FrameTarget::Mod(rt));
        assert_eq!(desc.clear_color, vec![Some([0.0, 0.0, 0.0, 1.0])]);
        assert_eq!(desc.clear_depth, Some(0.0));
        assert_eq!(desc.reads, vec![FrameTarget::MainDepth]);
        assert_eq!(
            desc.after,
            vec![
                FramePassRef::Anchor(RenderLevelStage::AfterOpaqueBlocks),
                FramePassRef::Pass(p0),
            ]
        );
        assert_eq!(
            desc.before,
            vec![FramePassRef::Anchor(
                RenderLevelStage::AfterTranslucentBlocks
            )]
        );
        assert!(desc.disable_culling);
    }

    #[test]
    fn cycle_error_formats_involved_pass_labels() {
        let err = FrameGraphError::Cycle {
            passes: vec!["pass_a".to_owned(), "pass_b".to_owned()],
        };
        assert_eq!(
            err.to_string(),
            "frame graph cycle detected between pass_a, pass_b"
        );
    }
}
