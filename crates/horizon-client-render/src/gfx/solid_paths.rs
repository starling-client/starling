// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
// SPDX-License-Identifier: MIT
// Adapted from Vasysik/horizon-android; see THIRD_PARTY_NOTICES.md and LICENSE.
//! Solid terrain through cheaper shaders on Adreno's Vulkan, in the frames whose instances prove
//! they draw the same pixels. Vasysik's Android fork measured them on an Adreno 710. The frames
//! of shader packs and of occlusion culling keep the usual pipelines, and so does a device that
//! refuses these.
use super::adreno::TerrainModule;

/// One solid pass's pipeline for both kinds of section draw.
pub(super) struct Pass {
    sections: wgpu::RenderPipeline,
    /// None without flat draws.
    flat: Option<wgpu::RenderPipeline>,
}

/// Where a pass reads its vertices, as `Renderer::new` builds the usual solid pipelines.
struct Source<'a, 'b> {
    /// The per-section draws', then the flat draws' (None without flat draws).
    layouts: (&'a wgpu::PipelineLayout, Option<&'a wgpu::PipelineLayout>),
    buffers: (
        &'a [Option<wgpu::VertexBufferLayout<'b>>],
        &'a [Option<wgpu::VertexBufferLayout<'b>>],
    ),
    /// The vertex entry points, `_opaque` or `_cutout` to come.
    vertex: (&'static str, &'static str),
}

impl Pass {
    fn new(
        terrain: &TerrainModule<'_>,
        source: &Source<'_, '_>,
        (label, flat_label): (&str, &str),
        part: &str,
        fragment: &str,
    ) -> Option<Self> {
        let back = Some(wgpu::Face::Back);
        let (vertex, flat_vertex) = source.vertex;
        let (buffers, flat_buffers) = source.buffers;
        let sections = terrain.pipeline(
            source.layouts.0,
            label,
            &format!("{vertex}_{part}"),
            fragment,
            buffers,
            back,
            true,
            None,
        )?;
        let flat = match source.layouts.1 {
            Some(layout) => Some(terrain.pipeline(
                layout,
                flat_label,
                &format!("{flat_vertex}_{part}"),
                fragment,
                flat_buffers,
                back,
                true,
                None,
            )?),
            None => None,
        };
        Some(Self { sections, flat })
    }
    fn get(&self, flat: bool) -> Option<&wgpu::RenderPipeline> {
        if flat {
            self.flat.as_ref()
        } else {
            Some(&self.sections)
        }
    }
}

/// What a frame's solid passes draw through in place of the usual pipelines.
#[derive(Clone, Copy, Default)]
pub(super) struct FramePasses<'a> {
    opaque: Option<&'a Pass>,
    cutout: Option<&'a Pass>,
}

impl FramePasses<'_> {
    pub fn opaque(&self, flat: bool) -> Option<&wgpu::RenderPipeline> {
        self.opaque.and_then(|pass| pass.get(flat))
    }
    pub fn cutout(&self, flat: bool) -> Option<&wgpu::RenderPipeline> {
        self.cutout.and_then(|pass| pass.get(flat))
    }
}

pub(super) struct SolidPaths {
    /// The cutout pass without `faded` (terrain.wgsl's `fs_cutout_static`): 21.770/22.101 ms of
    /// GPU time against 20.988/21.247 ms in the fork's measurement, in a tablet's 16-chunk scene.
    static_cutout: Pass,
    pub static_cutout_on: bool,
    /// Both passes through terrain.wgsl's `SETTLED_TERRAIN`, the opaque one then the cutout one:
    /// 21.576/21.567 ms of GPU time against 20.154/20.168 ms over that in the fork's measurement.
    settled: Option<[Pass; 2]>,
    pub settled_on: bool,
}

impl SolidPaths {
    /// `flat_bindings`: the flat draws' bind group 1, where they exist. `pulled`: the solid draws
    /// read their quads from the arena's pages (`vs_section*`, in either layout of
    /// `mesher::packed`) with only the origins a vertex buffer, at slot 1, and the flat draws
    /// none, as the usual pipelines do once flat draws exist (`Renderer::new`, `Flat::new`).
    /// Otherwise they take `buffers`, the vertices and the origins (`vs_terrain*`).
    pub fn new(
        terrain: &TerrainModule<'_>,
        bindings: &wgpu::BindGroupLayout,
        layout: &wgpu::PipelineLayout,
        flat_bindings: Option<&wgpu::BindGroupLayout>,
        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
        pulled: bool,
    ) -> Option<Self> {
        let flat_layout = flat_bindings.map(|flat| {
            terrain
                .kept
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Flat terrain layout"),
                    bind_group_layouts: &[Some(bindings), Some(flat)],
                    ..Default::default()
                })
        });
        let origins = [None, buffers.get(1).cloned().flatten()];
        let source = match &flat_layout {
            Some(pages) if pulled => Source {
                layouts: (pages, Some(pages)),
                buffers: (&origins, &[]),
                vertex: ("vs_section", "vs_section_flat"),
            },
            _ => Source {
                layouts: (layout, flat_layout.as_ref()),
                // The flat draws without the origin buffer.
                buffers: (buffers, &buffers[..1]),
                vertex: ("vs_terrain", "vs_terrain_flat"),
            },
        };
        let constants: Vec<_> = terrain
            .constants
            .iter()
            .copied()
            .chain([("SETTLED_TERRAIN", 1.0)])
            .collect();
        let settled_module = TerrainModule {
            constants: &constants,
            ..*terrain
        };
        let settled_opaque = Pass::new(
            &settled_module,
            &source,
            ("Settled terrain opaque", "Settled terrain opaque (flat)"),
            "opaque",
            "fs_opaque",
        );
        let settled_cutout = Pass::new(
            &settled_module,
            &source,
            ("Settled terrain cutout", "Settled terrain cutout (flat)"),
            "cutout",
            "fs_cutout_static",
        );
        Some(Self {
            settled: settled_opaque.zip(settled_cutout).map(<[Pass; 2]>::from),
            settled_on: true,
            static_cutout: Pass::new(
                terrain,
                &source,
                (
                    "Terrain cutout without fades",
                    "Terrain cutout without fades (flat)",
                ),
                "cutout",
                "fs_cutout_static",
            )?,
            static_cutout_on: true,
        })
    }

    /// The passes `origins`, the frame's instances, and `filtering`,
    /// `Options.textureFiltering`, allow.
    pub fn for_frame(&self, origins: &[[f32; 4]], filtering: u32) -> FramePasses<'_> {
        if !self.static_cutout_on || !origins.iter().all(|origin| unfaded(origin[3])) {
            return FramePasses::default();
        }
        let settled = self
            .settled
            .as_ref()
            .filter(|_| self.settled_on && can_use_settled(origins, filtering));
        match settled {
            Some([opaque, cutout]) => FramePasses {
                opaque: Some(opaque),
                cutout: Some(cutout),
            },
            None => FramePasses {
                opaque: None,
                cutout: Some(&self.static_cutout),
            },
        }
    }
}

/// `Options.textureFiltering`'s RGSS, which settled terrain doesn't take.
const RGSS: u32 = 1;

/// A fade's code is what a negative `w` holds (terrain.wgsl's `fade_code`).
fn unfaded(w: f32) -> bool {
    w >= 0.0
}

/// A whole section is the bare UV step: no fade-in in its mantissa, and not a distant tile's step
/// (`section_instance`, `lod::instance`).
fn settled(w: f32) -> bool {
    w == 1.0 / crate::mesher::UV_STEPS
}

pub(super) fn can_use_settled(origins: &[[f32; 4]], filtering: u32) -> bool {
    filtering != RGSS && origins.iter().all(|origin| settled(origin[3]))
}

#[cfg(test)]
mod tests;
