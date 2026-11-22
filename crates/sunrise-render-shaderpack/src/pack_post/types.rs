//! The pack's entries and exits, and the puzzle its passes are built from: what the game hands
//! it (`World`, `Lines`, `Clouds`, `Weather`, `Geometry`, `TerrainPage`), what it makes of them
//! (`Pass`, `Pipeline`, `Mips`, `Targets`), the device's limits (`Gpu`), the pipeline jobs
//! `compile` makes together, and why it may refuse to run (`Refused`). `PackPost` itself is in
//! the parent module; these names are shared by its construction and its execution.

use std::collections::HashMap;

use glam::DVec3;
use sunrise_shaders::{BUFFERS, Group, SHADOW_COLORS};

use crate::pack_entities::{PROGRAMS, PackEntities};
use crate::pack_sky::PackSky;

pub use crate::pack_post::construction::{
    ComputeJob, Geometry, Job, Refused, RenderJob, Resources, Slot, TerrainDraw, TerrainList,
    TerrainPage, attach, attachment_bytes, bindings, blend_state, build_compute, build_pass,
    checked, compile, custom_index, depth_linear, entry, fits, helper, holds_floats, image_copies,
    noise, owned_buffers, sampler, snorm_format, source, target, target_format, texture,
    texture_entry, texture_entry_of, texture_format, viewport,
};
pub use crate::pack_weather::Weather;

/// The texture behind a pack sampler name.
#[derive(Clone, Copy, PartialEq)]
pub enum Source {
    Color(usize),
    Depth,
    /// `depthtex1`: depth without translucent terrain (a copy taken before it).
    Depth1,
    /// `depthtex2`: depth without translucent terrain and without the first-person hand.
    Depth2,
    Noise,
    /// A 1×1 stand-in for names this slice does not provide yet.
    Dummy,
    /// A 1×1 flat normal (128, 128, 255) stand-in when no normal map is present.
    Normals,
    /// A 1×1 black (0, 0, 0, 0) stand-in when no specular map is present.
    Specular,
    /// `texture.<stage>.<name>`: the pack's own texture, filtered and wrapped as its
    /// `.mcmeta` says.
    Custom {
        index: usize,
        blur: bool,
        clamp: bool,
    },
    /// `image.<name>` through its sampler name.
    Image(usize),
    /// A 1×1 depth of 1.0 behind a comparison sampler, until shadow maps exist: no shadow.
    Shadow,
    /// The block atlas (pack_atlas.rs), in gbuffers programs.
    Atlas,
    Lightmap,
    /// `shadowtex0`/`shadowtex1` (6.3), with depth comparison (`sampler2DShadow`) or not.
    ShadowDepth(usize, bool),
    /// `shadowcolor0/1` read outside the shadow programs (in them they are `Color`).
    ShadowColor(usize),
    /// The boats' water patches' depth (`sunrise_shaders::WATER_MASK`), for translucent terrain.
    WaterMask,
}

#[derive(Clone)]
pub struct Binding {
    pub source: Source,
    pub binding: u32,
    /// A filtering sampler (the texture is filterable).
    pub filtering: bool,
    /// The view's dimension: 2D, or 1D and 3D for custom textures and their stand-ins.
    pub dimension: wgpu::TextureViewDimension,
    /// Where this texture's sampler object is bound: the first texture of each kind binds it
    /// (`binding + 1`), the others share it (devices have as few as 16 sampler slots).
    pub sampler: u32,
}

/// The sampler objects a pass binds, one slot per kind it uses.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum SamplerKind {
    Linear,
    Nearest,
    Repeat,
    NearestRepeat,
    /// The block atlas: nearest texels, blended between the two nearest levels.
    Atlas,
    /// colortex buffers: linear, blended between the two nearest levels.
    Buffer,
    Comparison,
}

impl SamplerKind {
    pub fn of(source: Source, filtering: bool) -> Self {
        match source {
            Source::Color(_) | Source::ShadowColor(_) | Source::Dummy if !filtering => {
                Self::Nearest
            }
            Source::Depth
            | Source::Depth1
            | Source::Depth2
            | Source::ShadowDepth(_, false)
            | Source::WaterMask => Self::Nearest,
            Source::Noise => Self::Repeat,
            Source::Shadow | Source::ShadowDepth(_, true) => Self::Comparison,
            // Blocks look as the game draws them: nearest texels. Between levels, Iris 1.11.7
            // blends `gtexture`'s two nearest (our probe packs, #9307: a read at level 1.25 is
            // levels 1 and 2 mixed 3:1).
            Source::Atlas if filtering => Self::Atlas,
            Source::Atlas | Source::Normals | Source::Specular => Self::NearestRepeat,
            Source::Image(_) if filtering => Self::Linear,
            Source::Image(_) => Self::Nearest,
            Source::Custom { blur, clamp, .. } => match (blur && filtering, clamp) {
                (true, true) => Self::Linear,
                (false, true) => Self::Nearest,
                (true, false) => Self::Repeat,
                (false, false) => Self::NearestRepeat,
            },
            // Iris 1.11.7 blends a buffer's two nearest mipmap levels (our probe packs, #9307:
            // every colortex read at levels 1.25 and 2.5 in composite and final).
            Source::Color(_) => Self::Buffer,
            Source::ShadowColor(_) | Source::Dummy | Source::Lightmap => Self::Linear,
        }
    }

    pub fn binding_type(self) -> wgpu::SamplerBindingType {
        match self {
            Self::Linear | Self::Repeat | Self::Atlas | Self::Buffer => {
                wgpu::SamplerBindingType::Filtering
            }
            Self::Nearest | Self::NearestRepeat => wgpu::SamplerBindingType::NonFiltering,
            Self::Comparison => wgpu::SamplerBindingType::Comparison,
        }
    }
}

/// Points every sampler object of `module` at the slot `slot` gives its binding, so that
/// samplers sharing a slot become one variable (two variables may not share a binding).
pub fn share_samplers(module: &mut naga::Module, slot: &HashMap<u32, u32>) {
    let mut first = HashMap::new();
    let mut same = HashMap::new();
    for (h, g) in module.global_variables.iter_mut() {
        let Some(b) = g.binding.as_mut() else {
            continue;
        };
        if !matches!(module.types[g.ty].inner, naga::TypeInner::Sampler { .. }) {
            continue;
        }
        let Some(&to) = slot.get(&b.binding) else {
            continue;
        };
        b.binding = to;
        match first.entry(to) {
            std::collections::hash_map::Entry::Occupied(e) => _ = same.insert(h, *e.get()),
            std::collections::hash_map::Entry::Vacant(e) => _ = e.insert(h),
        }
    }
    if same.is_empty() {
        return;
    }
    let fix = |expressions: &mut naga::Arena<naga::Expression>| {
        for (_, e) in expressions.iter_mut() {
            if let naga::Expression::GlobalVariable(g) = e
                && let Some(&to) = same.get(g)
            {
                *g = to;
            }
        }
    };
    for (_, f) in module.functions.iter_mut() {
        fix(&mut f.expressions);
    }
    for ep in &mut module.entry_points {
        fix(&mut ep.function.expressions);
    }
}

/// The sky's geometry and the programs that draw it: `gbuffers_skybasic` (disc, sunset, stars,
/// void) and `gbuffers_skytextured` (sun, moon), each with its fallbacks. A part without a
/// program is Sunrise's own.
pub struct Sky {
    pub geometry: PackSky,
    pub basic: Option<Pass>,
    pub textured: Option<Pass>,
}

/// Entities, block entities, translucent entity parts and particles a pack draws
/// (`pack_entities::ENTITY`... order), after terrain or, `late`, after deferred; Sunrise draws
/// the others.
pub struct Entities {
    pub geometry: PackEntities,
    pub passes: [Option<Pass>; PROGRAMS],
    pub late: [bool; PROGRAMS],
    /// What draws entities and block entities into the shadow map (2.4).
    pub shadow: [Option<Pass>; 2],
    /// Only the player, only block entities that emit light (3.2).
    pub only: (bool, bool),
}

impl Entities {
    /// Whether a gbuffers program draws some of them (else only the shadow pass does).
    pub fn drawn(&self) -> bool {
        self.passes.iter().any(Option::is_some)
    }
}

/// What an image binds: a buffer (`colorimgN`, or `shadowcolorimgN` for shadowcolor N: the side
/// its sampler reads) or one of the pack's.
#[derive(Clone, Copy, PartialEq)]
pub enum Image {
    Color(usize),
    ShadowColor(usize),
    Custom(usize),
    /// The copy of a buffer or a pack image taken before a dispatch, which its loads read.
    ColorBefore(usize),
    ShadowColorBefore(usize),
    CustomBefore(usize),
    /// An image no `image.*` gives: its stand-in of the declared format and dimension
    /// (`pack_images::StandIn`).
    StandIn(wgpu::TextureFormat, wgpu::TextureViewDimension),
}

impl Image {
    fn buffer(self) -> Option<usize> {
        match self {
            Image::Color(b) => Some(b),
            _ => None,
        }
    }
}

/// The buffers and the pack images (masks) `images` read as copies taken before a dispatch.
pub fn befores(images: &[(u32, Image)]) -> (u16, u64) {
    images
        .iter()
        .fold((0, 0), |(colors, customs), i| match i.1 {
            Image::ColorBefore(b) => (colors | 1 << b, customs),
            Image::CustomBefore(k) => (colors, customs | 1 << k),
            _ => (colors, customs),
        })
}

/// The color buffers of `images`, as a mask.
pub fn image_buffers(images: &[(u32, Image)]) -> u16 {
    images
        .iter()
        .filter_map(|i| i.1.buffer())
        .fold(0, |m, b| m | 1 << b)
}

/// The shadowcolor buffers of `images`, as masks: those bound as themselves, and those read as
/// copies taken before a dispatch.
pub fn shadow_images(images: &[(u32, Image)]) -> (u16, u16) {
    images.iter().fold((0, 0), |(own, before), i| match i.1 {
        Image::ShadowColor(n) => (own | 1 << n, before),
        Image::ShadowColorBefore(n) => (own, before | 1 << n),
        _ => (own, before),
    })
}

/// The shadowcolor buffers a pass of `group` copies first: those it samples (`reads` in the
/// shadow programs, `shadow_reads` in any) and binds as images, and those whose copy its loads
/// read.
pub fn shadow_copies(group: Group, reads: u16, shadow_reads: u16, images: &[(u32, Image)]) -> u16 {
    let sampled = shadow_reads | if group.shadow() { reads } else { 0 };
    let (own, before) = shadow_images(images);
    own & sampled | before
}

/// The shadowcolor buffer image `name` binds and its format, while the shadow pass runs (its
/// buffers have the formats `shadow`). Only the two names spec 5.2 gives: `shadowcolorimg01` or
/// `shadowcolorimg2` is no buffer.
pub fn shadow_image(
    name: &str,
    shadow: Option<&[wgpu::TextureFormat; SHADOW_COLORS]>,
) -> Option<(usize, wgpu::TextureFormat)> {
    let n = match name {
        "shadowcolorimg0" => 0,
        "shadowcolorimg1" => 1,
        _ => return None,
    };
    shadow.map(|f| (n, f[n]))
}

/// The format of the sink of passes that write only images (a shadowcomp program's is
/// `ShadowTargets::sink`).
pub const SINK: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// What packs use of the device beyond drawing. OpenGL ES and older drivers lack some of it: a
/// program that needs what is missing is skipped with a note, and the game runs on without it.
#[derive(Clone, Copy)]
pub struct Gpu {
    pub compute: bool,
    /// Images and storage buffers written by fragment shaders.
    pub fragment_storage: bool,
    /// Storage read and written by vertex shaders (a geometry stage).
    pub vertex_storage: bool,
    /// `clear_texture`; without it images are cleared by copying zeros.
    pub clear: bool,
    pub adapter_formats: bool,
}

impl Gpu {
    pub fn of(adapter: &wgpu::Adapter, features: wgpu::Features) -> Self {
        use wgpu::DownlevelFlags as D;
        let flags = adapter.get_downlevel_capabilities().flags;
        let mut gpu = Gpu {
            compute: flags.contains(D::COMPUTE_SHADERS),
            fragment_storage: flags.contains(D::FRAGMENT_WRITABLE_STORAGE),
            vertex_storage: flags.contains(D::VERTEX_STORAGE)
                && features.contains(wgpu::Features::VERTEX_WRITABLE_STORAGE),
            clear: features.contains(wgpu::Features::CLEAR_TEXTURE),
            adapter_formats: features
                .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES),
        };
        // `SUNRISE_SHADER_PACK_WITHOUT=compute,fragment-storage,vertex-storage,clear` tries the
        // fallbacks on a device that has it all.
        let without = std::env::var("SUNRISE_SHADER_PACK_WITHOUT").unwrap_or_default();
        for w in without.split(',') {
            match w.trim() {
                "compute" => gpu.compute = false,
                "fragment-storage" => gpu.fragment_storage = false,
                "vertex-storage" => gpu.vertex_storage = false,
                "clear" => gpu.clear = false,
                _ => {}
            }
        }
        gpu
    }

    /// Whether textures of `format` can be bound as images.
    pub fn storable(
        self,
        adapter: &wgpu::Adapter,
        format: wgpu::TextureFormat,
        features: wgpu::Features,
    ) -> bool {
        let usages = if self.adapter_formats {
            adapter.get_texture_format_features(format).allowed_usages
        } else {
            format.guaranteed_format_features(features).allowed_usages
        };
        (self.compute || self.fragment_storage)
            && usages.contains(wgpu::TextureUsages::STORAGE_BINDING)
    }

    /// The stages that may bind images and storage buffers.
    pub fn storage_stages(self) -> wgpu::ShaderStages {
        let mut stages = wgpu::ShaderStages::NONE;
        if self.compute {
            stages |= wgpu::ShaderStages::COMPUTE;
        }
        if self.fragment_storage {
            stages |= wgpu::ShaderStages::FRAGMENT;
        }
        if self.vertex_storage {
            stages |= wgpu::ShaderStages::VERTEX;
        }
        stages
    }
}

/// A pass's pipeline: a draw, or a compute dispatch. Its slot is filled when the pack's
/// pipelines are compiled (`compile`), all of them at once once the pack is laid out.
#[derive(Clone)]
pub enum Pipeline {
    Render(Slot<wgpu::RenderPipeline>),
    Compute {
        pipeline: Slot<wgpu::ComputePipeline>,
        /// `workGroups`, else `workGroupsRender` and the local size.
        groups: Option<[u32; 3]>,
        render: [f32; 2],
        local: [u32; 3],
    },
}

#[derive(Clone)]
pub struct Pass {
    pub name: String,
    pub group: Group,
    pub pipeline: Pipeline,
    /// The same program as vanilla blends the sky's parts (`PostProgram::vanilla_sky_blends`):
    /// without blending for the disc and the void, then with `STARS_BLEND` for the stars.
    pub sky_parts: Option<[Slot<wgpu::RenderPipeline>; 2]>,
    /// The entity program without colour writes, for the boats' water patches
    /// (`RenderTypes.waterMask`: depth only, so no water shows inside a hull).
    pub water_mask: Option<Slot<wgpu::RenderPipeline>>,
    pub layout: wgpu::BindGroupLayout,
    pub ubo: wgpu::Buffer,
    pub block: Vec<u8>,
    /// Shader-declared defaults must survive a reset of the mutable upload block.
    pub initial_block: Vec<u8>,
    /// (index into `pack_uniforms::NAMES`, declared type, offset).
    pub uniforms: Vec<(Option<usize>, String, u32)>,
    pub samplers: Vec<Binding>,
    /// (location, buffer) of each output that reaches a buffer.
    pub writes: Vec<(u32, usize)>,
    /// Buffers flipped after the pass.
    pub flip: u16,
    /// `hz_depth_linear`: which of the depth samplers a plain sampler reads filter.
    pub depth_linear: [f32; 4],
    /// Buffers sampled, for the bind group cache key.
    pub reads: u16,
    /// shadowcolor buffers sampled by a program of the other buffers, or bound as images.
    pub shadow_reads: u16,
    /// Buffers written.
    pub written: u16,
    /// Buffers whose mip chain is regenerated right before it (6.1.7).
    pub mipmaps: u16,
    /// Reads the per-draw block (`DrawUniforms`).
    pub draw: bool,
    /// What `texture` reads instead of the block atlas.
    pub atlas: Option<wgpu::TextureView>,
    pub material_maps: bool,
    /// Draws the shared atlas's geometry: `normals` and `specular` are its paged maps
    /// (`PackPost::set_paged_maps`), as its programs were translated to read them.
    pub paged: bool,
    /// Reads `mc_Entity`'s table (`PackPost::block_ids`).
    pub ids: bool,
    /// The depth-only pipeline drawn first (`Geometry::prepass`) and the alpha it keeps above.
    pub prepass: Option<(Slot<wgpu::RenderPipeline>, f32)>,
    /// Bind groups by the sides read (bits 0–15), the buffers whose mip chain is current (bits
    /// 16–31) and the shadowcolor sides read or bound as images (bits 32 on).
    pub groups: HashMap<u64, wgpu::BindGroup>,
    /// The same for End portal draws, with `end_portal.png` bound as the atlas.
    pub portal_groups: HashMap<u64, wgpu::BindGroup>,
    /// (binding, target) of each image: for a buffer, the side its sampler reads.
    pub images: Vec<(u32, Image)>,
    /// Binds the pack's storage buffers in set 4.
    pub buffers: bool,
    /// Buffers both sampled and bound as images: sampled from a copy taken first, as wgpu binds
    /// a texture one way per pass.
    pub copies: u16,
    /// The same for shadowcolor buffers, and those whose copy a dispatch's loads read.
    pub shadow_copies: u16,
    /// Compute programs dispatched before this pass.
    pub compute: Vec<Pass>,
    /// Writes no buffer, only images: draws into `Targets::sink`.
    pub sink: bool,
    /// Instances to draw for a geometry stage folded into the vertex stage; 0 without one.
    pub geometry: u32,
    /// The pack's images (bit n: `image.*` n) this pass samples and stores: sampled from a copy
    /// taken just before it.
    pub image_copies: u64,
    /// The pack's images a compute pass's loads read as they were before it: copied first.
    pub snapshots: u64,
    /// `scale.<program>` (scale, offset x, offset y): the pass draws only that part of its
    /// buffers and keeps the rest.
    pub viewport: Option<[f32; 3]>,
}

impl Pass {
    /// Back to the uniform values the pass was made with, and no bind groups.
    pub fn reset_history(&mut self) {
        self.block.clone_from(&self.initial_block);
        self.groups.clear();
        self.portal_groups.clear();
        for compute in &mut self.compute {
            compute.reset_history();
        }
    }

    pub fn render(&self) -> &wgpu::RenderPipeline {
        match &self.pipeline {
            Pipeline::Render(p) => p.get().expect("compiled with its pack"),
            Pipeline::Compute { .. } => unreachable!("a compute pass draws nothing"),
        }
    }
}

/// One side of a buffer with mipmaps.
pub struct Mips {
    /// Every level, for sampling.
    pub all: wgpu::TextureView,
    /// From the second level down: its attachment, and a bind group over the level above.
    pub levels: Vec<(wgpu::TextureView, wgpu::BindGroup)>,
}

/// The picture a pack's `final` draws at the world's size while the window is another: it is
/// copied onto the window once, after the pack's passes, as the game's own post pass does
/// without a pack.
pub struct FinalTarget {
    pub size: [u32; 2],
    pub view: wgpu::TextureView,
}

pub struct Targets {
    pub size: [u32; 2],
    /// A and B of each buffer used.
    pub color: [Option<[wgpu::TextureView; 2]>; BUFFERS],
    pub depth: wgpu::TextureView,
    pub depth_texture: wgpu::Texture,
    /// Pre-translucent depth for `depthtex1`, also serving `depthtex2` without a shader hand.
    pub depth1: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// `depthtex2`, when a program after the hand reads it without the hand.
    pub depth2: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The boats' water patches' depth, when translucent terrain reads it (`Source::WaterMask`).
    pub water_mask: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// Both sides' mip chains of the buffers with mipmaps (`color` is their first level).
    pub mips: [Option<[Mips; 2]>; BUFFERS],
    /// Where a pass that both samples and stores a buffer samples it from.
    pub scratch: [Option<wgpu::TextureView>; BUFFERS],
    /// Each buffer's size: the render size, or its `size.buffer`.
    pub sizes: [[u32; 2]; BUFFERS],
    /// The attachment of passes that write only images.
    pub sink: wgpu::TextureView,
}

/// What a pack's gbuffers programs read and draw from the game's world.
#[derive(Clone)]
pub struct World<'a> {
    /// The block atlas and its size.
    pub atlas: Option<(wgpu::TextureView, [u32; 2])>,
    /// Of a stitched block atlas, each sprite layer's place (`PackAtlas::rects_table`); empty
    /// for a grid.
    pub sprite_rects: &'a [u32],
    pub material_maps: Option<[wgpu::TextureView; 2]>,
    pub lightmap: wgpu::TextureView,
    /// Terrain's geometry, when the pack draws it.
    pub terrain: Option<Geometry<'a>>,
    /// Per source block state: its model material id, then its fluid material id.
    pub block_ids: &'a [i32],
    /// The atlas of the sun and moon's sprites.
    pub celestial: Option<wgpu::TextureView>,
    pub gui_size: [u32; 2],
    /// Set 1 of the clouds' draw: their offset and palette.
    pub clouds: Option<&'a wgpu::BindGroupLayout>,
    /// The game can hand its rain and snow to the pack.
    pub weather: bool,
    /// The game can hand its first-person hand to the pack.
    pub hand: bool,
}

/// This frame's block outline, when the pack draws it: the outline buffer (one instance per
/// edge, pack_lines.rs), the edges of a stroke and the strokes in it, each drawn over the one
/// before (a high contrast outline's black stroke, then its bright one).
pub struct Lines {
    pub edges: wgpu::Buffer,
    pub count: u32,
    pub strokes: u32,
}

/// This frame's clouds, when the pack draws them: their vertices, uniform, quad indices and
/// quad count.
pub struct Clouds {
    pub vertices: wgpu::Buffer,
    pub group: wgpu::BindGroup,
    pub indices: wgpu::Buffer,
    pub quads: u32,
}

/// Shadow cache: the key of the shadow map the shadow targets hold. Only the
/// shadow passes write those targets, so while the key matches they still hold
/// it and the pass is skipped.
#[derive(Default)]
pub struct ShadowCache {
    pub key: Option<ShadowCacheKey>,
}

/// Key for validating the shadow cache.
/// Must match the cache state exactly to be considered valid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowCacheKey {
    pub sun_angle: f32,
    pub moon_angle: f32,
    pub snapped_camera: Option<DVec3>,
    pub shadow_settings_hash: u64,
    pub shadow_scene_revision: (u64, u64),
    pub ticks: u64,
    pub day_time: i64,
    pub shadow_uniforms_hash: u64,
}
