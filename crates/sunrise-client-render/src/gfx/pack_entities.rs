//! Entities, block entities, translucent entity parts and particles drawn by a shader pack's
//! `gbuffers_entities`, `gbuffers_block`, `gbuffers_entities_translucent` and
//! `gbuffers_particles` (spec 2.3, 5.3, 5.4). Re-exported from sunrise_render_shaderpack.

pub use sunrise_render_shaderpack::pack_entities::*;

pub trait PackEntitiesExt {
    fn prepare_client(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        entities: &[crate::gfx::Vertex],
        spans: &[sunrise_render_core::EntitySpan],
        drawn: [bool; PROGRAMS],
        crack: ([f32; 4], [u32; 2]),
        shadow: Casters,
        rest: &mut Vec<crate::gfx::Vertex>,
        skin: Option<&crate::gfx::entity_skin::SkinFrame>,
        motion: (&[(u32, u64)], &[u32]),
    );
}

impl PackEntitiesExt for PackEntities {
    fn prepare_client(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        entities: &[crate::gfx::Vertex],
        spans: &[sunrise_render_core::EntitySpan],
        drawn: [bool; PROGRAMS],
        crack: ([f32; 4], [u32; 2]),
        shadow: Casters,
        rest: &mut Vec<crate::gfx::Vertex>,
        skin: Option<&crate::gfx::entity_skin::SkinFrame>,
        motion: (&[(u32, u64)], &[u32]),
    ) {
        let (jobs, spans_slice, vertices) = match skin {
            Some(s) => {
                let j: Vec<sunrise_render_core::SkinRunJob> = s
                    .jobs
                    .iter()
                    .map(|job| sunrise_render_core::SkinRunJob {
                        dst: job.dst,
                        count: job.count,
                        overlay: job.overlay,
                    })
                    .collect();
                (Some(j), Some(s.spans.as_slice()), s.vertices)
            }
            None => (None, None, 0),
        };
        let posed = jobs.as_ref().map(|j| sunrise_render_core::PosedModelRuns {
            vertices,
            jobs: j,
            spans: spans_slice.unwrap_or_default(),
        });
        self.prepare(
            device,
            queue,
            layout,
            entities,
            spans,
            drawn,
            crack,
            shadow,
            rest,
            posed.as_ref(),
            motion,
        );
    }
}
