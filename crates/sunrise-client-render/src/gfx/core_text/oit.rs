//! A resource pack's `core/text` under Improved Transparency: `OIT_TEXT` and
//! `OIT_TEXT_POLYGON_OFFSET`, which vanilla 26.3 builds alike (`OitPipelineSet.builder` over
//! `WORLD_TEXT_SNIPPET` alone, so neither has a depth bias), drawing translucent world text (a
//! text display's background and translucent text, a discrete name tag) in each `OitStage`, as
//! `Features::text` draws it natively.

use crate::gfx::expression_blocks::{self, ExpressionBlocks};
use crate::gfx::oit::{CloudSamples, Stage, stage_targets};
use crate::polytone_post::ExpressionUniform;
use sunrise_glsl::Translated;

/// One stage's pipeline and what its group binds.
pub(super) struct OitStage {
    pub(super) pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Vanilla's blocks the stage reads (`expression_blocks::vanilla_blocks`).
    blocks: Vec<(u32, u64, u64, wgpu::ShaderStages)>,
    /// The stage samplers it reads and its stage gives: the texture's binding (the sampler
    /// object's the next) and its place in [`CloudSamples::views`].
    samplers: Vec<(u32, usize)>,
    /// Which of the glyphs' (4) and the lightmap's (8) bindings its stages read.
    textures: Vec<u32>,
    pub(super) extra: ExpressionBlocks,
}

/// The three stages over a buffer of their own: vanilla's four blocks, then each stage's Polytone
/// blocks.
pub(crate) struct OitPart {
    pub(super) stages: [OitStage; 3],
    uniform: wgpu::Buffer,
    /// The glyphs (`Sampler0`) and the lightmap (`Sampler2`), as the world text binds them.
    glyphs: (wgpu::TextureView, wgpu::Sampler),
    lightmap: (wgpu::TextureView, wgpu::Sampler),
    /// `getClampToEdge(FilterMode.NEAREST)`, which the stage samplers are.
    stage_sampler: wgpu::Sampler,
    /// The groups, for the targets of the generation they were made for
    /// ([`CloudSamples::generation`]); none until the first frame under Improved Transparency.
    pub(super) groups: Option<(u64, [wgpu::BindGroup; 3])>,
}

/// Where vanilla's four blocks end in the buffer (the last one's offset and its slot,
/// `expression_blocks::VANILLA_OFFSETS`), and the first stage's Polytone blocks begin.
const VANILLA_BYTES: u64 = expression_blocks::VANILLA_OFFSETS[3] + expression_blocks::VANILLA_SLOT;

impl OitPart {
    /// `programs` (each stage's, with its expression uniforms) made into their pipelines: none
    /// when a program is past what the device takes in its stage's targets
    /// (`sunrise_glsl::PastDeviceLimit`), which leaves the translucent world text to the native
    /// pipelines in every stage; the device refusing a pipeline fails the load, named by it.
    pub(super) fn new(
        device: &wgpu::Device,
        programs: [(&Translated, &[ExpressionUniform]); 3],
        atlas: &wgpu::TextureView,
        lightmap: (&wgpu::TextureView, &wgpu::Sampler),
    ) -> Result<Option<Box<Self>>, String> {
        let text = &crate::hud::pack_text::TEXT;
        let pipeline_of = |i: usize| text[crate::hud::pack_text::TEXT_OIT[i]].pipeline;
        let formats = |i: usize| -> Vec<wgpu::TextureFormat> {
            (stage_targets(Stage::ALL[i]).iter())
                .filter_map(|t| t.as_ref().map(|t| t.format))
                .collect()
        };
        let fits = (0..3).all(|i| {
            crate::core_shader::within_device_limits(
                device,
                programs[i].0,
                pipeline_of(i),
                &formats(i),
            )
        });
        if !fits {
            return Ok(None);
        }
        let mut base = VANILLA_BYTES;
        let extras: [ExpressionBlocks; 3] = std::array::from_fn(|i| {
            let (program, expressions) = programs[i];
            let extra = ExpressionBlocks::new(program, expressions, base);
            base = extra.size();
            extra
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack Improved Transparency text uniforms"),
            size: base,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyphs = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Resource pack Improved Transparency text glyphs"),
            ..Default::default()
        });
        let stage = |i: usize, extra| {
            crate::core_shader::scoped(device, pipeline_of(i), || {
                OitStage::new(
                    device,
                    programs[i].0,
                    Stage::ALL[i],
                    extra,
                    [(4, atlas, &glyphs), (8, lightmap.0, lightmap.1)],
                )
            })
        };
        let [e0, e1, e2] = extras;
        let stages = [stage(0, e0)?, stage(1, e1)?, stage(2, e2)?];
        Ok(Some(Box::new(Self {
            stages,
            uniform,
            glyphs: (atlas.clone(), glyphs),
            lightmap: (lightmap.0.clone(), lightmap.1.clone()),
            stage_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Resource pack Improved Transparency text stage samples"),
                ..Default::default()
            }),
            groups: None,
        })))
    }

    /// The frame's vanilla blocks (`data`) and each stage's Polytone blocks.
    pub(super) fn prepare(
        &self,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        data: &[u8; VANILLA_BYTES as usize],
    ) {
        uploads.buffer(encoder, &self.uniform, 0, data);
        for stage in &self.stages {
            stage.extra.upload(uploads, encoder, &self.uniform);
        }
    }

    /// The stages' groups for `samples`' targets, made again only when those targets were.
    pub(super) fn bind(&mut self, device: &wgpu::Device, samples: &CloudSamples<'_>) {
        if (self.groups.as_ref()).is_some_and(|(made, _)| *made == samples.generation) {
            return;
        }
        let textures = [
            (4, &self.glyphs.0, &self.glyphs.1),
            (8, &self.lightmap.0, &self.lightmap.1),
        ];
        let group = |stage: &OitStage| {
            let (_, mut entries) =
                expression_blocks::bindings(&stage.blocks, &stage.extra, &self.uniform);
            entries.extend(stage_textures(&textures, &stage.textures));
            for &(binding, i) in &stage.samplers {
                entries.push(wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(samples.views[i]),
                });
                entries.push(wgpu::BindGroupEntry {
                    binding: binding + 1,
                    resource: wgpu::BindingResource::Sampler(&self.stage_sampler),
                });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Resource pack Improved Transparency text"),
                layout: &stage.layout,
                entries: &entries,
            })
        };
        let groups = [
            group(&self.stages[0]),
            group(&self.stages[1]),
            group(&self.stages[2]),
        ];
        self.groups = Some((samples.generation, groups));
    }
}

/// The entries of `textures` whose bindings `bound` lists, as their pair of texture and sampler.
fn stage_textures<'a>(
    textures: &[(u32, &'a wgpu::TextureView, &'a wgpu::Sampler); 2],
    bound: &[u32],
) -> Vec<wgpu::BindGroupEntry<'a>> {
    let mut entries = Vec::new();
    for &(binding, view, sampler) in textures {
        if bound.contains(&binding) {
            entries.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: binding + 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
    }
    entries
}

impl OitStage {
    /// `program`'s pipeline for `stage`: the entity vertices, back faces culled, the world's
    /// depth tested and not written, no bias, into the stage's targets.
    fn new(
        device: &wgpu::Device,
        program: &Translated,
        stage: Stage,
        extra: ExpressionBlocks,
        textures: [(u32, &wgpu::TextureView, &wgpu::Sampler); 2],
    ) -> Self {
        let label = "Resource pack Improved Transparency text";
        let blocks = expression_blocks::vanilla_blocks(program);
        let mut entries: Vec<wgpu::BindGroupLayoutEntry> = (blocks.iter())
            .map(|&(binding, _, size, stages)| {
                expression_blocks::block_entry(binding, size, stages)
            })
            .chain(extra.layout())
            .collect();
        let (texture_entries, _) = expression_blocks::sampler_bindings(program, &textures);
        let textures: Vec<u32> = (texture_entries.iter())
            .filter(|e| matches!(e.ty, wgpu::BindingType::Texture { .. }))
            .map(|e| e.binding)
            .collect();
        entries.extend(texture_entries);
        let names = crate::gfx::core_clouds::OIT_STAGES[stage as usize].samplers;
        let samplers: Vec<(u32, usize)> = (crate::gfx::core_clouds::STAGE_SAMPLERS.iter())
            .enumerate()
            .filter(|(_, name)| names.contains(name))
            .filter_map(|(i, name)| {
                let at = program.other.iter().position(|n| n == name)?;
                Some((program.other_binding(at), i))
            })
            .filter(|&(binding, _)| {
                !(program.stages_reading(binding) | program.stages_reading(binding + 1)).is_empty()
            })
            .collect();
        for &(binding, _) in &samplers {
            let visibility = program.stages_reading(binding) | program.stages_reading(binding + 1);
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: binding + 1,
                visibility,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::mesher::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let module = |module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let (vertex, fragment) = (module(&program.vertex), module(&program.fragment));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: crate::gfx::oit_features::OIT_TEXT_BIAS,
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &stage_targets(stage),
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            layout,
            blocks,
            samplers,
            textures,
            extra,
        }
    }
}

#[cfg(test)]
#[path = "oit_tests.rs"]
mod tests;
