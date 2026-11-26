//! Data-driven client frame graph (`#963`, NeoForge 26.3's `FrameGraphSetupEvent`, vanilla
//! `FrameGraphBuilder` / `LevelTargetBundle` / `CrossFrameResourcePool`).
//!
//! Mods record offscreen passes during `Hooks::frame_graph_setup` reading and writing named
//! `LevelTargetBundle` built-in targets, imported mod `RenderTargetRef` targets (always external
//! and never culled), and transient per-frame targets pooled across frames. Passes are ordered
//! topologically relative to `RenderLevelStage` anchors and explicit/resource dependencies with
//! deterministic registration-order tie-breaking; cycles are refused cleanly and logged once.

use super::mod_pipelines::{OffscreenPassData, PipelineDraw, ResolvedSet1Entry};
use super::mod_render_targets::{
    ModRenderTargets, ResolvedOffscreenTarget, wgpu_color_format, wgpu_depth_format,
};
use super::world_primitives::WorldPrimitives;
use mod_api::{
    FrameGraphError, FramePassDescriptor, FramePassId, FramePassRef, FrameTarget, OffscreenPass,
    OitStage, RenderLevelStage, RenderTargetDepth, RenderTargetDescriptor, RenderTargetFormat,
    TransientTargetId,
};
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, HashMap, VecDeque};

/// Starting view ID for transient frame-graph target views, disjoint from `ModUniforms` and
/// `ModRenderTargets` (`1 << 32`) so `ModPipelines` set-1 cache keys never collide.
const FIRST_TRANSIENT_VIEW_ID: u64 = 1 << 33;

/// Number of idle frames a pooled transient target is retained before freeing
/// (`CrossFrameResourcePool.java`: `framesToKeep = 3` in vanilla 26.3 `GameRenderer`).
const TRANSIENT_FRAMES_TO_KEEP: u32 = 3;

/// Execution slot in the world frame where a compiled mod offscreen pass runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameGraphSlot {
    /// Before the main world passes open (passes with no `after(stage)` constraint, or
    /// `before(AfterSky)`).
    BeforeWorld,
    /// Immediately after the named [`RenderLevelStage`] boundary.
    AfterStage(RenderLevelStage),
}

/// One recorded mod offscreen pass with its draw commands and byte pools.
#[derive(Clone, Debug)]
pub struct RecordedFramePass {
    pub label: String,
    /// The render target the pass draws into; `None` for a compute node.
    pub target: Option<FrameTarget>,
    pub clear_color: Vec<Option<[f32; 4]>>,
    pub clear_depth: Option<f32>,
    pub clear_stencil: Option<u32>,
    pub projection: Option<[f32; 16]>,
    pub view: Option<[f32; 16]>,
    pub reads: Vec<FrameTarget>,
    pub writes: Vec<FrameTarget>,
    pub after: Vec<FramePassRef>,
    pub before: Vec<FramePassRef>,
    pub disable_culling: bool,
    /// The mip level of a mod target the pass renders into ([`FramePassDescriptor::mip_level`]).
    pub mip_level: u32,
    pub draws: Vec<PipelineDraw>,
    pub transient_vertices: Vec<u8>,
    pub transient_indices: Vec<u8>,
    pub inline_uniforms: Vec<u8>,
    pub texel_data: Vec<u8>,
    pub owner: Option<&'static str>,
    /// The dispatches of a compute node (`None` for a render pass).
    pub dispatches: Option<Vec<super::mod_compute::RecordedDispatch>>,
    /// The storage buffer copies a compute node requested, each encoded between the dispatches
    /// recorded before and after its request.
    pub readbacks: Vec<super::mod_readback::RecordedReadback>,
}

impl RecordedFramePass {
    /// Construct an empty recorded pass from a validated [`FramePassDescriptor`].
    pub fn from_descriptor(desc: FramePassDescriptor, owner: Option<&'static str>) -> Self {
        Self {
            label: desc.label,
            target: Some(desc.target),
            clear_color: desc.clear_color,
            clear_depth: desc.clear_depth,
            clear_stencil: desc.clear_stencil,
            projection: desc.projection,
            view: desc.view,
            reads: desc.reads,
            writes: desc.writes,
            after: desc.after,
            before: desc.before,
            disable_culling: desc.disable_culling,
            mip_level: desc.mip_level,
            draws: Vec::new(),
            transient_vertices: Vec::new(),
            transient_indices: Vec::new(),
            inline_uniforms: Vec::new(),
            texel_data: Vec::new(),
            owner,
            dispatches: None,
            readbacks: Vec::new(),
        }
    }

    /// An empty compute node from a validated [`ComputePassDescriptor`]; it is never culled.
    pub fn from_compute_descriptor(
        desc: mod_api::ComputePassDescriptor,
        owner: Option<&'static str>,
    ) -> Self {
        Self {
            label: desc.label,
            target: None,
            clear_color: Vec::new(),
            clear_depth: None,
            clear_stencil: None,
            projection: None,
            view: None,
            reads: desc.reads,
            writes: desc.writes,
            after: desc.after,
            before: desc.before,
            disable_culling: true,
            mip_level: 0,
            draws: Vec::new(),
            transient_vertices: Vec::new(),
            transient_indices: Vec::new(),
            inline_uniforms: Vec::new(),
            texel_data: Vec::new(),
            owner,
            dispatches: Some(Vec::new()),
            readbacks: Vec::new(),
        }
    }
}

/// Per-frame recorded frame graph collected during `Hooks::frame_graph_setup`.
#[derive(Clone, Debug, Default)]
pub struct RecordedFrameGraph {
    pub transients: Vec<RenderTargetDescriptor>,
    pub passes: Vec<RecordedFramePass>,
}

impl RecordedFrameGraph {
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    pub fn clear(&mut self) {
        self.transients.clear();
        self.passes.clear();
    }

    /// Register a validated transient target descriptor and return its per-frame id.
    pub fn add_transient(&mut self, desc: RenderTargetDescriptor) -> TransientTargetId {
        let id = TransientTargetId::from_raw(self.transients.len() as u32);
        self.transients.push(desc);
        id
    }

    /// Look up a transient target's descriptor in this frame.
    pub fn transient_desc(&self, id: TransientTargetId) -> Option<&RenderTargetDescriptor> {
        self.transients.get(id.raw() as usize)
    }

    /// Validate a candidate [`FramePassDescriptor`] against existing passes, targets, and
    /// acyclicity before recording its draws or adding it to the graph.
    pub fn validate_new_pass(
        &self,
        desc: &FramePassDescriptor,
        oit_on: bool,
    ) -> Result<(), FrameGraphError> {
        self.validate_spec(PassGraphSpec::from_descriptor(desc), oit_on)
    }

    /// [`Self::validate_new_pass`] for a compute node.
    pub fn validate_new_compute_pass(
        &self,
        desc: &mod_api::ComputePassDescriptor,
        oit_on: bool,
    ) -> Result<(), FrameGraphError> {
        self.validate_spec(PassGraphSpec::from_compute_descriptor(desc), oit_on)
    }

    fn validate_spec(&self, desc: PassGraphSpec<'_>, oit_on: bool) -> Result<(), FrameGraphError> {
        if desc.label.trim().is_empty() {
            return Err(FrameGraphError::EmptyLabel);
        }
        match desc.target {
            None | Some(FrameTarget::Mod(_)) => {}
            Some(FrameTarget::Transient(id)) => {
                if self.transient_desc(id).is_none() {
                    return Err(FrameGraphError::UnknownTransientTarget(id));
                }
            }
            Some(other) => return Err(FrameGraphError::UnsupportedTarget(other)),
        }
        for &t in desc.reads {
            if Some(t) == desc.target || desc.writes.contains(&t) {
                return Err(FrameGraphError::SelfDependency);
            }
            if let FrameTarget::Transient(id) = t
                && self.transient_desc(id).is_none()
            {
                return Err(FrameGraphError::UnknownTransientTarget(id));
            }
        }
        for &t in desc.writes {
            if let FrameTarget::Transient(id) = t
                && self.transient_desc(id).is_none()
            {
                return Err(FrameGraphError::UnknownTransientTarget(id));
            }
        }
        let pass_count = self.passes.len();
        for &dep in desc.after.iter().chain(desc.before.iter()) {
            if let FramePassRef::Pass(id) = dep
                && id.raw() as usize >= pass_count
            {
                return Err(FrameGraphError::UnknownPass(id));
            }
        }
        // Check acyclicity with the candidate appended (without culling so cycles in not-yet-
        // consumed transient chains are caught immediately on `add_pass`).
        let mut specs: Vec<PassGraphSpec<'_>> = self
            .passes
            .iter()
            .map(PassGraphSpec::from_recorded)
            .collect();
        specs.push(desc);
        compile_specs(&specs, self.transients.len(), oit_on, false).map(|_| ())
    }

    /// Append a previously validated [`RecordedFramePass`] and return its [`FramePassId`].
    pub fn push_pass(&mut self, pass: RecordedFramePass) -> FramePassId {
        let id = FramePassId::from_raw(self.passes.len() as u32);
        self.passes.push(pass);
        id
    }

    /// Compile this recorded graph into a culled, topologically sorted execution plan with
    /// transient resource lifetimes (`FrameGraphBuilder.execute`).
    pub fn compile(&self, oit_on: bool) -> Result<CompiledFrameGraph, FrameGraphError> {
        let specs: Vec<PassGraphSpec<'_>> = self
            .passes
            .iter()
            .map(PassGraphSpec::from_recorded)
            .collect();
        compile_specs(&specs, self.transients.len(), oit_on, true)
    }
}

/// Borrowed view of a pass's graph metadata so `validate_new_pass` and `compile` share one
/// implementation without cloning draw buffers.
struct PassGraphSpec<'a> {
    label: &'a str,
    target: Option<FrameTarget>,
    reads: &'a [FrameTarget],
    writes: &'a [FrameTarget],
    after: &'a [FramePassRef],
    before: &'a [FramePassRef],
    disable_culling: bool,
}

impl<'a> PassGraphSpec<'a> {
    fn from_recorded(p: &'a RecordedFramePass) -> Self {
        Self {
            label: &p.label,
            target: p.target,
            reads: &p.reads,
            writes: &p.writes,
            after: &p.after,
            before: &p.before,
            disable_culling: p.disable_culling,
        }
    }

    fn from_compute_descriptor(d: &'a mod_api::ComputePassDescriptor) -> Self {
        Self {
            label: &d.label,
            target: None,
            reads: &d.reads,
            writes: &d.writes,
            after: &d.after,
            before: &d.before,
            disable_culling: true,
        }
    }

    fn from_descriptor(d: &'a FramePassDescriptor) -> Self {
        Self {
            label: &d.label,
            target: Some(d.target),
            reads: &d.reads,
            writes: &d.writes,
            after: &d.after,
            before: &d.before,
            disable_culling: d.disable_culling,
        }
    }

    fn writes_external(&self) -> bool {
        self.target.is_some_and(|t| t.is_external()) || self.writes.iter().any(|t| t.is_external())
    }

    fn all_writes(&self) -> impl Iterator<Item = FrameTarget> + '_ {
        self.target.into_iter().chain(self.writes.iter().copied())
    }
}

fn normalize_anchor(stage: RenderLevelStage, oit_on: bool) -> RenderLevelStage {
    let active = RenderLevelStage::frame_order(oit_on);
    if active.contains(&stage) {
        return stage;
    }
    if oit_on {
        match stage {
            RenderLevelStage::AfterTranslucentFeatures
            | RenderLevelStage::AfterTranslucentBlocks
            | RenderLevelStage::AfterTranslucentParticles
            | RenderLevelStage::AfterWeather => {
                RenderLevelStage::OitTranslucent(OitStage::Accumulate)
            }
            _ => RenderLevelStage::AfterLevel,
        }
    } else {
        match stage {
            RenderLevelStage::OitTranslucent(_) => RenderLevelStage::AfterTranslucentBlocks,
            _ => RenderLevelStage::AfterLevel,
        }
    }
}

/// Culled, topologically sorted frame-graph execution plan with transient target lifetimes.
#[derive(Clone, Debug, Default)]
pub struct CompiledFrameGraph {
    /// Indices into `RecordedFrameGraph::passes` in execution order (culled passes omitted).
    pub order: Vec<usize>,
    /// Execution positions (`0..order.len()`) grouped by [`FrameGraphSlot`].
    pub by_slot: HashMap<FrameGraphSlot, Vec<usize>>,
    /// For each `TransientTargetId`, `Some((first_order_pos, last_order_pos))` if kept.
    pub transient_lifetimes: Vec<Option<(usize, usize)>>,
    /// Indexed by `order_pos`: transient targets to acquire immediately before running `order[pos]`.
    pub acquire_before: Vec<Vec<TransientTargetId>>,
    /// Indexed by `order_pos`: transient targets to release immediately after running `order[pos]`.
    pub release_after: Vec<Vec<TransientTargetId>>,
}

fn compile_specs(
    passes: &[PassGraphSpec<'_>],
    transient_count: usize,
    oit_on: bool,
    cull: bool,
) -> Result<CompiledFrameGraph, FrameGraphError> {
    let anchors = RenderLevelStage::frame_order(oit_on);
    let num_anchors = anchors.len();
    let num_passes = passes.len();
    let total_nodes = num_anchors + num_passes;

    let anchor_node = |stage: RenderLevelStage| -> usize {
        let norm = normalize_anchor(stage, oit_on);
        anchors
            .iter()
            .position(|&s| s == norm)
            .unwrap_or(num_anchors - 1)
    };
    let pass_node = |p: usize| -> usize { num_anchors + p };

    let mut adj: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); total_nodes];
    let mut rev_mod: Vec<Vec<usize>> = vec![Vec::new(); num_passes];

    let add_edge = |u: usize, v: usize, adj: &mut [BTreeSet<usize>], rev_mod: &mut [Vec<usize>]| {
        if adj[u].insert(v) && u >= num_anchors && v >= num_anchors {
            rev_mod[v - num_anchors].push(u - num_anchors);
        }
    };

    // Fixed anchor chain in engine frame order.
    for i in 0..num_anchors.saturating_sub(1) {
        add_edge(i, i + 1, &mut adj, &mut rev_mod);
    }

    // Explicit `after` and `before` dependencies.
    for (p, spec) in passes.iter().enumerate() {
        let node = pass_node(p);
        for &dep in spec.after {
            let from = match dep {
                FramePassRef::Anchor(stage) => anchor_node(stage),
                FramePassRef::Pass(id) => {
                    let idx = id.raw() as usize;
                    if idx >= num_passes {
                        return Err(FrameGraphError::UnknownPass(id));
                    }
                    if idx == p {
                        return Err(FrameGraphError::SelfDependency);
                    }
                    pass_node(idx)
                }
                _ => continue,
            };
            add_edge(from, node, &mut adj, &mut rev_mod);
        }
        for &dep in spec.before {
            let to = match dep {
                FramePassRef::Anchor(stage) => anchor_node(stage),
                FramePassRef::Pass(id) => {
                    let idx = id.raw() as usize;
                    if idx >= num_passes {
                        return Err(FrameGraphError::UnknownPass(id));
                    }
                    if idx == p {
                        return Err(FrameGraphError::SelfDependency);
                    }
                    pass_node(idx)
                }
                _ => continue,
            };
            add_edge(node, to, &mut adj, &mut rev_mod);
        }
    }

    // Implicit resource RAW, WAR, and WAW edges across mod passes in registration order
    // (matching `FrameGraphBuilder.java` handle aliasing + `readBy` ordering).
    let mut last_writer: HashMap<FrameTarget, usize> = HashMap::new();
    let mut readers_since_write: HashMap<FrameTarget, Vec<usize>> = HashMap::new();
    for (p, spec) in passes.iter().enumerate() {
        let p_node = pass_node(p);
        for &target in spec.reads {
            if let Some(&writer) = last_writer.get(&target)
                && writer != p
            {
                add_edge(pass_node(writer), p_node, &mut adj, &mut rev_mod);
            }
            readers_since_write.entry(target).or_default().push(p);
        }
        for target in spec.all_writes() {
            if let Some(readers) = readers_since_write.get(&target) {
                for &reader in readers {
                    if reader != p {
                        add_edge(pass_node(reader), p_node, &mut adj, &mut rev_mod);
                    }
                }
            }
            if let Some(&writer) = last_writer.get(&target)
                && writer != p
            {
                add_edge(pass_node(writer), p_node, &mut adj, &mut rev_mod);
            }
            last_writer.insert(target, p);
            readers_since_write.remove(&target);
        }
    }

    // Dead-pass culling (`FrameGraphBuilder.identifyPassesToKeep`):
    // External outputs (built-in LevelTargetBundle targets + imported mod RenderTargetRef) and
    // `disable_culling` passes are roots; transitively keep all prerequisite mod passes.
    let mut kept_pass = vec![!cull; num_passes];
    if cull {
        let mut queue = VecDeque::new();
        for (p, spec) in passes.iter().enumerate() {
            if spec.disable_culling || spec.writes_external() {
                kept_pass[p] = true;
                queue.push_back(p);
            }
        }
        while let Some(p) = queue.pop_front() {
            for &prev in &rev_mod[p] {
                if !kept_pass[prev] {
                    kept_pass[prev] = true;
                    queue.push_back(prev);
                }
            }
        }
    }

    let is_active = |node: usize| -> bool {
        if node < num_anchors {
            true
        } else {
            kept_pass[node - num_anchors]
        }
    };

    // Kahn's topological sort with deterministic registration-order tie-breaking:
    // priority `(0, pass_idx, node)` for mod passes, `(1, anchor_idx, node)` for anchors so
    // ready mod passes execute in registration order at their earliest valid slot.
    let mut in_degree = vec![0usize; total_nodes];
    let mut active_count = 0usize;
    for u in 0..total_nodes {
        if !is_active(u) {
            continue;
        }
        active_count += 1;
        for &v in &adj[u] {
            if is_active(v) {
                in_degree[v] += 1;
            }
        }
    }

    let node_key = |node: usize| -> (u8, usize, usize) {
        if node < num_anchors {
            (1, node, node)
        } else {
            (0, node - num_anchors, node)
        }
    };

    let mut ready: BinaryHeap<Reverse<(u8, usize, usize)>> = BinaryHeap::new();
    for node in 0..total_nodes {
        if is_active(node) && in_degree[node] == 0 {
            ready.push(Reverse(node_key(node)));
        }
    }

    let mut visited = 0usize;
    let mut current_slot = FrameGraphSlot::BeforeWorld;
    let mut order = Vec::with_capacity(num_passes);
    let mut by_slot: HashMap<FrameGraphSlot, Vec<usize>> = HashMap::new();

    while let Some(Reverse((_, _, node))) = ready.pop() {
        visited += 1;
        if node < num_anchors {
            current_slot = FrameGraphSlot::AfterStage(anchors[node]);
        } else {
            let p = node - num_anchors;
            let order_pos = order.len();
            order.push(p);
            by_slot.entry(current_slot).or_default().push(order_pos);
        }
        for &v in &adj[node] {
            if !is_active(v) {
                continue;
            }
            in_degree[v] -= 1;
            if in_degree[v] == 0 {
                ready.push(Reverse(node_key(v)));
            }
        }
    }

    if visited < active_count {
        let mut involved = Vec::new();
        for (p, spec) in passes.iter().enumerate() {
            let node = pass_node(p);
            if is_active(node) && in_degree[node] > 0 {
                involved.push(spec.label.to_owned());
            }
        }
        if involved.is_empty() {
            for (a, stage) in anchors.iter().enumerate() {
                if in_degree[a] > 0 {
                    involved.push(format!("{stage:?}"));
                }
            }
        }
        return Err(FrameGraphError::Cycle { passes: involved });
    }

    // Assign transient target lifetimes (`FrameGraphBuilder.assignResourceLifetimes`).
    let mut transient_lifetimes: Vec<Option<(usize, usize)>> = vec![None; transient_count];
    for (order_pos, &p) in order.iter().enumerate() {
        let spec = &passes[p];
        for target in spec.reads.iter().copied().chain(spec.all_writes()) {
            if let FrameTarget::Transient(id) = target {
                let idx = id.raw() as usize;
                if let Some(slot) = transient_lifetimes.get_mut(idx) {
                    *slot = Some(match *slot {
                        None => (order_pos, order_pos),
                        Some((first, _)) => (first, order_pos),
                    });
                }
            }
        }
    }

    let mut acquire_before = vec![Vec::new(); order.len()];
    let mut release_after = vec![Vec::new(); order.len()];
    for (idx, lifetime) in transient_lifetimes.iter().enumerate() {
        if let Some((first, last)) = *lifetime {
            let id = TransientTargetId::from_raw(idx as u32);
            acquire_before[first].push(id);
            release_after[last].push(id);
        }
    }

    Ok(CompiledFrameGraph {
        order,
        by_slot,
        transient_lifetimes,
        acquire_before,
        release_after,
    })
}

struct PooledColorAttachment {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    view_id: u64,
    format: wgpu::TextureFormat,
}

struct PooledDepthAttachment {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    attachment_view: wgpu::TextureView,
    sample_view: wgpu::TextureView,
    view_id: u64,
    format: wgpu::TextureFormat,
    has_stencil: bool,
}

/// One GPU-backed transient target managed by [`TransientTargetPool`].
pub struct PooledTransientTarget {
    desc: RenderTargetDescriptor,
    colors: Vec<PooledColorAttachment>,
    depth: Option<PooledDepthAttachment>,
    idle_frames: u32,
}

impl PooledTransientTarget {
    fn as_resolved(&self) -> ResolvedOffscreenTarget {
        ResolvedOffscreenTarget {
            handle: mod_api::__private::built::render_target_ref(1, 0),
            size: self.desc.size,
            color_views: self.colors.iter().map(|c| c.view.clone()).collect(),
            color_formats: self.colors.iter().map(|c| c.format).collect(),
            depth_view: self.depth.as_ref().map(|d| d.attachment_view.clone()),
            depth_format: self.depth.as_ref().map(|d| d.format),
            has_stencil: self.depth.as_ref().is_some_and(|d| d.has_stencil),
        }
    }

    fn color_view(&self, attachment: u8) -> Option<(u64, wgpu::TextureView)> {
        self.colors
            .get(attachment as usize)
            .map(|c| (c.view_id, c.view.clone()))
    }

    fn depth_view(&self) -> Option<(u64, wgpu::TextureView)> {
        self.depth
            .as_ref()
            .map(|d| (d.view_id, d.sample_view.clone()))
    }
}

/// Cross-frame pool of transient GPU render targets (`CrossFrameResourcePool`), matching
/// vanilla's 3-frame retention before freeing unused targets.
pub struct TransientTargetPool {
    idle: Vec<PooledTransientTarget>,
    next_view_id: u64,
}

impl Default for TransientTargetPool {
    fn default() -> Self {
        Self {
            idle: Vec::new(),
            next_view_id: FIRST_TRANSIENT_VIEW_ID,
        }
    }
}

impl TransientTargetPool {
    fn alloc_view_id(&mut self) -> u64 {
        let id = self.next_view_id;
        self.next_view_id = self
            .next_view_id
            .checked_add(1)
            .expect("transient target view id exhausted");
        id
    }

    /// Acquire an idle pooled target matching `desc`, or allocate a fresh one on `device`.
    pub fn acquire(
        &mut self,
        device: &wgpu::Device,
        desc: &RenderTargetDescriptor,
    ) -> PooledTransientTarget {
        if let Some(pos) = self.idle.iter().position(|t| &t.desc == desc) {
            let mut target = self.idle.swap_remove(pos);
            target.idle_frames = 0;
            return target;
        }
        let extent = wgpu::Extent3d {
            width: desc.size[0],
            height: desc.size[1],
            depth_or_array_layers: 1,
        };
        let mut colors = Vec::with_capacity(desc.color_targets.len());
        for (idx, &fmt) in desc.color_targets.iter().enumerate() {
            let format = wgpu_color_format(fmt);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("Mod transient target color {idx}")),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let view_id = self.alloc_view_id();
            colors.push(PooledColorAttachment {
                texture,
                view,
                view_id,
                format,
            });
        }
        let depth = wgpu_depth_format(desc.depth).map(|format| {
            let has_stencil = matches!(format, wgpu::TextureFormat::Depth24PlusStencil8);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Mod transient target depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let attachment_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let sample_view = texture.create_view(&wgpu::TextureViewDescriptor {
                aspect: wgpu::TextureAspect::DepthOnly,
                ..Default::default()
            });
            let view_id = self.alloc_view_id();
            PooledDepthAttachment {
                texture,
                attachment_view,
                sample_view,
                view_id,
                format,
                has_stencil,
            }
        });
        PooledTransientTarget {
            desc: desc.clone(),
            colors,
            depth,
            idle_frames: 0,
        }
    }

    /// Return a transient target to the pool after its last reader pass finishes.
    pub fn release(&mut self, mut target: PooledTransientTarget) {
        target.idle_frames = 0;
        self.idle.push(target);
    }

    /// Age idle targets at the end of a frame and drop entries unused for
    /// [`TRANSIENT_FRAMES_TO_KEEP`] frames.
    pub fn end_frame(&mut self) {
        self.idle.retain_mut(|target| {
            target.idle_frames += 1;
            target.idle_frames < TRANSIENT_FRAMES_TO_KEEP
        });
    }

    /// Drop all pooled transient targets (world close, resource reload).
    pub fn clear(&mut self) {
        self.idle.clear();
    }
}

/// Per-frame executor stepping through a [`CompiledFrameGraph`] across world render slots.
pub struct FrameGraphRunner {
    recorded: RecordedFrameGraph,
    compiled: CompiledFrameGraph,
    active_transients: HashMap<TransientTargetId, PooledTransientTarget>,
    /// Scratch for a compute node's dispatches that bind a transient target, reused across
    /// passes and frames.
    dispatch_scratch: Vec<super::mod_compute::RecordedDispatch>,
    /// Each planned dispatch with its index among the node's recorded dispatches.
    dispatch_plan: Vec<(usize, DispatchPick)>,
}

/// Where a compute node's dispatch to record comes from.
#[derive(Clone, Copy)]
enum DispatchPick {
    /// The node's own dispatch, which binds no transient target.
    Recorded(usize),
    /// The resolved copy in the runner's scratch.
    Scratch(usize),
}

impl FrameGraphRunner {
    /// Compile `recorded` for `oit_on` and prepare a runner, returning `None` when empty or
    /// when a cycle is detected (logging each distinct cycle once without panicking).
    pub fn prepare(
        recorded: RecordedFrameGraph,
        oit_on: bool,
        warned_cycles: &mut std::collections::HashSet<Vec<String>>,
    ) -> Option<Self> {
        if recorded.is_empty() {
            return None;
        }
        match recorded.compile(oit_on) {
            Ok(compiled) => {
                if compiled.order.is_empty() {
                    return None;
                }
                Some(Self {
                    recorded,
                    compiled,
                    active_transients: HashMap::new(),
                    dispatch_scratch: Vec::new(),
                    dispatch_plan: Vec::new(),
                })
            }
            Err(FrameGraphError::Cycle { passes }) => {
                if warned_cycles.insert(passes.clone()) {
                    log::warn!(
                        "Sunrise: frame graph cycle detected between {}; skipping mod frame graph",
                        passes.join(", ")
                    );
                }
                None
            }
            Err(err) => {
                log::warn!("Sunrise: frame graph compilation failed: {err}");
                None
            }
        }
    }

    /// Whether any kept pass is scheduled to run at `slot`.
    pub fn has_slot(&self, slot: FrameGraphSlot) -> bool {
        self.compiled
            .by_slot
            .get(&slot)
            .is_some_and(|v| !v.is_empty())
    }

    /// Execute all kept passes scheduled for `slot` in topological order.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_slot(
        &mut self,
        slot: FrameGraphSlot,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world_primitives: &mut WorldPrimitives,
        mod_render_targets: &ModRenderTargets,
        transient_pool: &mut TransientTargetPool,
    ) {
        let Some(positions) = self.compiled.by_slot.get(&slot) else {
            return;
        };
        for &order_pos in positions {
            for &id in &self.compiled.acquire_before[order_pos] {
                if let Some(desc) = self.recorded.transient_desc(id) {
                    let target = transient_pool.acquire(device, desc);
                    self.active_transients.insert(id, target);
                }
            }

            let pass_idx = self.compiled.order[order_pos];
            let pass = &self.recorded.passes[pass_idx];
            if let Some(dispatches) = &pass.dispatches {
                resolve_dispatch_transients(
                    dispatches,
                    &self.active_transients,
                    &mut self.dispatch_scratch,
                    &mut self.dispatch_plan,
                );
                // Each readback is encoded after the dispatches recorded before its request.
                let mut start = 0;
                for readback in pass.readbacks.iter().map(Some).chain([None]) {
                    let end = match readback {
                        Some(readback) => {
                            start
                                + self.dispatch_plan[start..]
                                    .iter()
                                    .take_while(|(index, _)| *index < readback.after)
                                    .count()
                        }
                        None => self.dispatch_plan.len(),
                    };
                    if start < end {
                        let scratch = &self.dispatch_scratch;
                        world_primitives.pipelines.compute.record(
                            device,
                            encoder,
                            self.dispatch_plan[start..end]
                                .iter()
                                .map(|(_, pick)| match *pick {
                                    DispatchPick::Recorded(i) => &dispatches[i],
                                    DispatchPick::Scratch(i) => &scratch[i],
                                }),
                        );
                    }
                    start = end;
                    if let Some(readback) = readback {
                        readback.encode(encoder);
                    }
                }
            }
            let resolved_target = match pass.target {
                Some(FrameTarget::Mod(rt)) => mod_render_targets
                    .resolve_target(pass.owner, rt, pass.mip_level)
                    .ok(),
                Some(FrameTarget::Transient(id)) => {
                    self.active_transients.get(&id).map(|t| t.as_resolved())
                }
                _ => None,
            };

            if let Some(target) = resolved_target {
                let resolved_draws =
                    resolve_pass_transient_bindings(&pass.draws, &self.active_transients);
                let mut offscreen = OffscreenPass::new(target.handle);
                offscreen.clear_color = pass.clear_color.clone();
                offscreen.clear_depth = pass.clear_depth;
                offscreen.clear_stencil = pass.clear_stencil;
                offscreen.projection = pass.projection;
                offscreen.view = pass.view;
                offscreen.mip_level = pass.mip_level;
                world_primitives.execute_offscreen_pass(
                    device,
                    queue,
                    encoder,
                    &OffscreenPassData {
                        pass: &offscreen,
                        target: &target,
                        draws: &resolved_draws,
                        transient_vertices: &pass.transient_vertices,
                        transient_indices: &pass.transient_indices,
                        inline_uniforms: &pass.inline_uniforms,
                        texel_data: &pass.texel_data,
                    },
                );
            }

            for &id in &self.compiled.release_after[order_pos] {
                if let Some(target) = self.active_transients.remove(&id) {
                    transient_pool.release(target);
                }
            }
        }
    }

    /// Release any remaining active transient targets (e.g. if a shader pack skipped a stage)
    /// and advance the transient pool's frame counter.
    pub fn finish(mut self, transient_pool: &mut TransientTargetPool) {
        for (_, target) in self.active_transients.drain() {
            transient_pool.release(target);
        }
        transient_pool.end_frame();
    }
}

fn resolve_pass_transient_bindings(
    draws: &[PipelineDraw],
    active_transients: &HashMap<TransientTargetId, PooledTransientTarget>,
) -> Vec<PipelineDraw> {
    let mut out = Vec::with_capacity(draws.len());
    for draw in draws {
        let mut cloned = draw.clone();
        let mut ok = true;
        if let Some(bindings) = &mut cloned.bindings {
            for entry in &mut bindings.entries {
                match *entry {
                    ResolvedSet1Entry::TransientColor { target, attachment } => {
                        if let Some((view_id, view)) = active_transients
                            .get(&target)
                            .and_then(|t| t.color_view(attachment))
                        {
                            *entry = ResolvedSet1Entry::Texture {
                                view_id,
                                view,
                                mips: None,
                            };
                        } else {
                            ok = false;
                            break;
                        }
                    }
                    ResolvedSet1Entry::TransientDepth { target } => {
                        if let Some((view_id, view)) =
                            active_transients.get(&target).and_then(|t| t.depth_view())
                        {
                            *entry = ResolvedSet1Entry::Texture {
                                view_id,
                                view,
                                mips: None,
                            };
                        } else {
                            ok = false;
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        if ok {
            out.push(cloned);
        }
    }
    out
}

/// Plan which of `dispatches` to record: those that bind no transient target as they are, the
/// others as copies in `scratch` with each transient target binding swapped for its pooled
/// texture view. A dispatch whose transient is not live is dropped, as a draw's is.
fn resolve_dispatch_transients(
    dispatches: &[super::mod_compute::RecordedDispatch],
    active_transients: &HashMap<TransientTargetId, PooledTransientTarget>,
    scratch: &mut Vec<super::mod_compute::RecordedDispatch>,
    plan: &mut Vec<(usize, DispatchPick)>,
) {
    scratch.clear();
    plan.clear();
    for (index, dispatch) in dispatches.iter().enumerate() {
        let binds_transient = dispatch.entries.iter().any(|entry| {
            matches!(
                entry,
                ResolvedSet1Entry::TransientColor { .. } | ResolvedSet1Entry::TransientDepth { .. }
            )
        });
        if !binds_transient {
            plan.push((index, DispatchPick::Recorded(index)));
            continue;
        }
        let mut cloned = dispatch.clone();
        let mut ok = true;
        for entry in &mut cloned.entries {
            let view = match *entry {
                ResolvedSet1Entry::TransientColor { target, attachment } => active_transients
                    .get(&target)
                    .and_then(|t| t.color_view(attachment)),
                ResolvedSet1Entry::TransientDepth { target } => {
                    active_transients.get(&target).and_then(|t| t.depth_view())
                }
                _ => continue,
            };
            match view {
                Some((view_id, view)) => {
                    *entry = ResolvedSet1Entry::Texture {
                        view_id,
                        view,
                        mips: None,
                    }
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            plan.push((index, DispatchPick::Scratch(scratch.len())));
            scratch.push(cloned);
        }
    }
}

/// Validate a transient target sampler binding against its descriptor during draw recording.
pub fn validate_transient_color_binding(
    desc: &RenderTargetDescriptor,
    attachment: u8,
    filterable: bool,
    float32_filterable: bool,
) -> bool {
    let Some(&fmt) = desc.color_targets.get(attachment as usize) else {
        return false;
    };
    if filterable && fmt == RenderTargetFormat::R32Float && !float32_filterable {
        return false;
    }
    true
}

/// Validate a transient target depth sampler binding against its descriptor during draw recording.
pub fn validate_transient_depth_binding(desc: &RenderTargetDescriptor, filterable: bool) -> bool {
    !filterable && desc.depth != RenderTargetDepth::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::RenderTargetRef;

    fn rt(generation: u64, slot: u32) -> RenderTargetRef {
        mod_api::__private::built::render_target_ref(generation, slot)
    }

    fn rgba8_desc(size: [u32; 2]) -> RenderTargetDescriptor {
        RenderTargetDescriptor::rgba8(size, RenderTargetDepth::None)
    }

    fn push(graph: &mut RecordedFrameGraph, desc: FramePassDescriptor) -> FramePassId {
        graph.validate_new_pass(&desc, false).expect("valid pass");
        graph.push_pass(RecordedFramePass::from_descriptor(desc, None))
    }

    #[test]
    fn topological_sort_orders_transient_chain_and_breaks_ties_by_registration_order() {
        let mut graph = RecordedFrameGraph::default();
        let tmp = graph.add_transient(rgba8_desc([64, 64]));
        let out = rt(1, 0);

        // Pass A writes transient `tmp`; Pass B reads `tmp` and writes external `out`.
        // Pass C and D are independent external passes after `AfterOpaqueBlocks` to test
        // deterministic registration-order tie-breaking.
        let p_a = push(
            &mut graph,
            FramePassDescriptor::new("pass_a", FrameTarget::Transient(tmp)),
        );
        let p_b = push(
            &mut graph,
            FramePassDescriptor::new("pass_b", FrameTarget::Mod(out))
                .reads(FrameTarget::Transient(tmp)),
        );
        let p_c = push(
            &mut graph,
            FramePassDescriptor::new("pass_c", FrameTarget::Mod(rt(2, 1)))
                .after_stage(RenderLevelStage::AfterOpaqueBlocks),
        );
        let p_d = push(
            &mut graph,
            FramePassDescriptor::new("pass_d", FrameTarget::Mod(rt(3, 2)))
                .after_stage(RenderLevelStage::AfterOpaqueBlocks),
        );

        let compiled = graph.compile(false).expect("acyclic graph");
        assert_eq!(
            compiled.order,
            vec![
                p_a.raw() as usize,
                p_b.raw() as usize,
                p_c.raw() as usize,
                p_d.raw() as usize
            ]
        );
        assert_eq!(
            compiled.by_slot.get(&FrameGraphSlot::BeforeWorld),
            Some(&vec![0, 1])
        );
        assert_eq!(
            compiled.by_slot.get(&FrameGraphSlot::AfterStage(
                RenderLevelStage::AfterOpaqueBlocks
            )),
            Some(&vec![2, 3])
        );
        // Transient `tmp` is acquired before pass A (pos 0) and released after pass B (pos 1).
        assert_eq!(compiled.transient_lifetimes, vec![Some((0, 1))]);
        assert_eq!(compiled.acquire_before[0], vec![tmp]);
        assert_eq!(compiled.release_after[1], vec![tmp]);
    }

    #[test]
    fn dead_pass_culling_drops_unread_transient_keeps_disable_culling_and_never_culls_mod_targets()
    {
        let mut graph = RecordedFrameGraph::default();
        let dead_tmp = graph.add_transient(rgba8_desc([32, 32]));
        let kept_tmp = graph.add_transient(rgba8_desc([32, 32]));
        let imported = rt(1, 0);

        let _culled = push(
            &mut graph,
            FramePassDescriptor::new("unused_transient", FrameTarget::Transient(dead_tmp)),
        );
        let pinned = push(
            &mut graph,
            FramePassDescriptor::new("pinned_transient", FrameTarget::Transient(kept_tmp))
                .disable_culling(),
        );
        let ext = push(
            &mut graph,
            FramePassDescriptor::new("imported_mod_target", FrameTarget::Mod(imported)),
        );

        let compiled = graph.compile(false).expect("acyclic");
        assert_eq!(
            compiled.order,
            vec![pinned.raw() as usize, ext.raw() as usize]
        );
        assert_eq!(compiled.transient_lifetimes[dead_tmp.raw() as usize], None);
        assert_eq!(
            compiled.transient_lifetimes[kept_tmp.raw() as usize],
            Some((0, 0))
        );
    }

    #[test]
    fn cycle_rejection_reports_involved_labels_without_panicking() {
        let mut graph = RecordedFrameGraph::default();
        let tmp = graph.add_transient(rgba8_desc([32, 32]));
        let out = rt(1, 0);

        let p_a = push(
            &mut graph,
            FramePassDescriptor::new("pass_a", FrameTarget::Transient(tmp)),
        );
        // Pass B reads `tmp` (creating A -> B) AND declares `before(pass_a)` (creating B -> A).
        let cyclic = FramePassDescriptor::new("pass_b", FrameTarget::Mod(out))
            .reads(FrameTarget::Transient(tmp))
            .before_pass(p_a);
        let err = graph
            .validate_new_pass(&cyclic, false)
            .expect_err("cycle must be refused");
        match err {
            FrameGraphError::Cycle { passes } => {
                assert_eq!(passes, vec!["pass_a".to_owned(), "pass_b".to_owned()]);
            }
            other => panic!("expected Cycle, got {other:?}"),
        }

        // Anchor-order cycle: after AfterWeather AND before AfterOpaqueBlocks.
        let anchor_cycle = FramePassDescriptor::new("bad_anchor_pass", FrameTarget::Mod(out))
            .after_stage(RenderLevelStage::AfterWeather)
            .before_stage(RenderLevelStage::AfterOpaqueBlocks);
        assert!(matches!(
            graph.validate_new_pass(&anchor_cycle, false),
            Err(FrameGraphError::Cycle { .. })
        ));
    }

    #[test]
    fn self_dependency_and_unsupported_builtin_write_target_are_refused() {
        let graph = RecordedFrameGraph::default();
        let out = rt(1, 0);
        let self_rw =
            FramePassDescriptor::new("self_rw", FrameTarget::Mod(out)).reads(FrameTarget::Mod(out));
        assert_eq!(
            graph.validate_new_pass(&self_rw, false),
            Err(FrameGraphError::SelfDependency)
        );

        let builtin_write = FramePassDescriptor::new("write_main", FrameTarget::MainColor);
        assert_eq!(
            graph.validate_new_pass(&builtin_write, false),
            Err(FrameGraphError::UnsupportedTarget(FrameTarget::MainColor))
        );
    }

    /// The first texel of mip `level` of `texture`, read back after `enc`'s commands.
    fn read_texel(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut enc: wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        level: u32,
    ) -> [u8; 4] {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fg mip readback"),
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range().unwrap();
        data[..4].try_into().unwrap()
    }

    /// A frame-graph pass built from an `OffscreenPass` renders into the mip level it names
    /// (#5990): level 1 of a two-level target takes the pass's clear and level 0 is untouched.
    /// Level 2 is out of range, and the target refuses it as `add_pass` does.
    #[test]
    #[ignore = "daily"]
    fn gpu_frame_graph_pass_renders_into_its_mip_level() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        let mut targets = ModRenderTargets::default();
        let rt = targets
            .create(&device, Some("test_mod"), &rgba8_desc([8, 8]).mip_levels(2))
            .expect("two-level render target");
        assert!(matches!(
            targets.resolve_target(Some("test_mod"), rt, 2),
            Err(mod_api::RenderTargetError::InvalidMipLevels)
        ));
        let mut offscreen = OffscreenPass::new(rt);
        offscreen.clear_color = vec![Some([0.0, 1.0, 0.0, 1.0])];
        offscreen.mip_level = 1;
        let desc = FramePassDescriptor::offscreen("mip_1", offscreen).disable_culling();
        assert_eq!(desc.mip_level, 1, "the descriptor keeps the pass's level");
        let mut recorded = RecordedFrameGraph::default();
        recorded
            .validate_new_pass(&desc, false)
            .expect("valid pass");
        recorded.push_pass(RecordedFramePass::from_descriptor(desc, Some("test_mod")));

        let mut transient_pool = TransientTargetPool::default();
        let mut primitives =
            WorldPrimitives::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        let mut warned = std::collections::HashSet::new();
        let mut runner =
            FrameGraphRunner::prepare(recorded, false, &mut warned).expect("compiled graph");
        let mut enc = device.create_command_encoder(&Default::default());
        runner.execute_slot(
            FrameGraphSlot::BeforeWorld,
            &device,
            &queue,
            &mut enc,
            &mut primitives,
            &targets,
            &mut transient_pool,
        );
        runner.finish(&mut transient_pool);
        let texture = targets.color_texture(rt, 0).expect("colour texture");
        let level_1 = read_texel(&device, &queue, enc, texture, 1);
        assert_eq!(level_1, [0, 255, 0, 255], "level 1 takes the pass");
        let enc = device.create_command_encoder(&Default::default());
        let level_0 = read_texel(&device, &queue, enc, texture, 0);
        assert_ne!(level_0, [0, 255, 0, 255], "level 0 is untouched");
    }

    #[test]
    #[ignore = "daily"]
    fn gpu_frame_graph_runs_two_pass_transient_chain_culls_unused_and_rejects_cycle() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };

        use crate::mod_pipelines::Pipelines;
        use mod_api::{
            Identifier, MeshDraw, PipelineBlend, RenderPipeline, Sampler, SamplerAddress,
            SamplerFilter, VertexAttributeFormat, VertexFormat, VertexStepMode, WorldDepth,
        };

        // Pass A pipeline: writes solid red into transient target.
        let write_id = Identifier::parse("test:fg_write_red").unwrap();
        let write_vsh = "\
            #version 330\n\
            in vec3 Pos;\n\
            void main() { gl_Position = vec4(Pos, 1.0); }\n";
        let write_fsh = "\
            #version 330\n\
            out vec4 fragColor;\n\
            void main() { fragColor = vec4(1.0, 0.0, 0.0, 1.0); }\n";

        // Pass B pipeline: samples transient target and writes into imported RenderTargetRef.
        let sample_id = Identifier::parse("test:fg_sample_tmp").unwrap();
        let sample_vsh = "\
            #version 330\n\
            in vec3 Pos;\n\
            in vec2 Uv;\n\
            out vec2 vUv;\n\
            void main() { vUv = Uv; gl_Position = vec4(Pos, 1.0); }\n";
        let sample_fsh = "\
            #version 330\n\
            uniform sampler2D SrcTex;\n\
            in vec2 vUv;\n\
            out vec4 fragColor;\n\
            void main() { fragColor = texture(SrcTex, vUv); }\n";

        let write_fmt = VertexFormat::builder(VertexStepMode::Vertex)
            .attribute("Pos", VertexAttributeFormat::Float32x3)
            .build()
            .unwrap();
        let sample_fmt = VertexFormat::builder(VertexStepMode::Vertex)
            .attribute("Pos", VertexAttributeFormat::Float32x3)
            .attribute("Uv", VertexAttributeFormat::Float32x2)
            .build()
            .unwrap();

        let p_write = RenderPipeline::new(
            write_id.clone(),
            write_id,
            PipelineBlend::Opaque,
            WorldDepth::ThroughWalls,
        )
        .vertex_binding(0, write_fmt);
        let p_sample = RenderPipeline::new(
            sample_id.clone(),
            sample_id,
            PipelineBlend::Opaque,
            WorldDepth::ThroughWalls,
        )
        .vertex_binding(0, sample_fmt)
        .sampler(
            "SrcTex",
            Sampler::new(SamplerFilter::Nearest, SamplerAddress::ClampToEdge),
        );

        let set = Pipelines::build(&[p_write, p_sample], 1, |path| {
            if path.contains("fg_write_red") && path.ends_with(".vsh") {
                Ok(Some(write_vsh.to_owned()))
            } else if path.contains("fg_write_red") && path.ends_with(".fsh") {
                Ok(Some(write_fsh.to_owned()))
            } else if path.contains("fg_sample_tmp") && path.ends_with(".vsh") {
                Ok(Some(sample_vsh.to_owned()))
            } else if path.contains("fg_sample_tmp") && path.ends_with(".fsh") {
                Ok(Some(sample_fsh.to_owned()))
            } else {
                Ok(None)
            }
        })
        .expect("translate frame graph test pipelines");

        let mut targets = ModRenderTargets::default();
        let out_rt = targets
            .create(&device, Some("test_mod"), &rgba8_desc([8, 8]))
            .expect("create output render target");
        let mut transient_pool = TransientTargetPool::default();

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        primitives
            .pipelines
            .build(&device, &set, None)
            .expect("build frame graph test pipelines");

        // Fullscreen quad vertices for Pass A (Pos only) and Pass B (Pos + Uv).
        let quad_pos: [[f32; 3]; 4] = [
            [-1.0, -1.0, 0.5],
            [1.0, -1.0, 0.5],
            [1.0, 1.0, 0.5],
            [-1.0, 1.0, 0.5],
        ];
        let quad_pos_bytes: &[u8] = bytemuck::cast_slice(&quad_pos);
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct PosUv([f32; 3], [f32; 2]);
        let quad_pos_uv = [
            PosUv([-1.0, -1.0, 0.5], [0.0, 1.0]),
            PosUv([1.0, -1.0, 0.5], [1.0, 1.0]),
            PosUv([1.0, 1.0, 0.5], [1.0, 0.0]),
            PosUv([-1.0, 1.0, 0.5], [0.0, 0.0]),
        ];
        let quad_pos_uv_bytes: &[u8] = bytemuck::cast_slice(&quad_pos_uv);

        let mut recorded = RecordedFrameGraph::default();
        let tmp = recorded.add_transient(rgba8_desc([8, 8]));
        let unused_tmp = recorded.add_transient(rgba8_desc([8, 8]));

        // Pass A: writes red into transient `tmp`.
        let mut pass_a = RecordedFramePass::from_descriptor(
            FramePassDescriptor::new("pass_a", FrameTarget::Transient(tmp))
                .clear_color_0([0.0, 0.0, 0.0, 1.0]),
            Some("test_mod"),
        );
        pass_a.transient_vertices.extend_from_slice(quad_pos_bytes);
        pass_a.draws.push(PipelineDraw {
            slot: 0,
            stage: 0,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: super::super::mod_pipelines::PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..quad_pos_bytes.len() as u64)],
                index_range: None,
            },
            bindings: None,
        });
        let id_a = recorded.push_pass(pass_a);

        // Pass C (unread transient, culled).
        let pass_c = RecordedFramePass::from_descriptor(
            FramePassDescriptor::new("pass_c_culled", FrameTarget::Transient(unused_tmp)),
            Some("test_mod"),
        );
        recorded.push_pass(pass_c);

        // Pass B: samples `tmp` and writes into imported `out_rt`.
        let mut pass_b = RecordedFramePass::from_descriptor(
            FramePassDescriptor::new("pass_b", FrameTarget::Mod(out_rt))
                .clear_color_0([0.0, 0.0, 1.0, 1.0])
                .reads(FrameTarget::Transient(tmp)),
            Some("test_mod"),
        );
        pass_b
            .transient_vertices
            .extend_from_slice(quad_pos_uv_bytes);
        pass_b.draws.push(PipelineDraw {
            slot: 1,
            stage: 0,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: super::super::mod_pipelines::PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..quad_pos_uv_bytes.len() as u64)],
                index_range: None,
            },
            bindings: Some(super::super::mod_pipelines::ResolvedDrawBindings {
                entries: vec![ResolvedSet1Entry::TransientColor {
                    target: tmp,
                    attachment: 0,
                }],
                push_constants: None,
            }),
        });
        let _id_b = recorded.push_pass(pass_b);

        // Verify a cyclic pass D is rejected cleanly without panicking.
        let cyclic_d = FramePassDescriptor::new("pass_d_cycle", FrameTarget::Mod(out_rt))
            .reads(FrameTarget::Transient(tmp))
            .before_pass(id_a);
        assert!(matches!(
            recorded.validate_new_pass(&cyclic_d, false),
            Err(FrameGraphError::Cycle { .. })
        ));

        let mut warned = std::collections::HashSet::new();
        let mut runner =
            FrameGraphRunner::prepare(recorded, false, &mut warned).expect("compiled graph");
        assert_eq!(
            runner.compiled.order.len(),
            2,
            "unused pass C must be culled"
        );

        let mut enc = device.create_command_encoder(&Default::default());
        runner.execute_slot(
            FrameGraphSlot::BeforeWorld,
            &device,
            &queue,
            &mut enc,
            &mut primitives,
            &targets,
            &mut transient_pool,
        );
        runner.finish(&mut transient_pool);

        // Read back centre pixel of `out_rt` and verify Pass A's red propagated through `tmp`!
        let out_tex = targets.color_texture(out_rt, 0).expect("output texture");
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fg test readback"),
            size: 256 * 8,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: out_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(8),
                },
            },
            wgpu::Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();
        let data = data.unwrap();
        let px = &data[4 * 256 + 4 * 4..4 * 256 + 4 * 4 + 4];
        assert!(
            px[0] > 240 && px[1] < 15 && px[2] < 15,
            "expected red pixel from Pass A -> transient -> Pass B, got {px:?}"
        );
    }
}
