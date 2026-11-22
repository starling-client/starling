use super::*;
use crate::gfx::hud_coverage::{self, Pipeline};

pub struct HudRenderer {
    pub(super) pictures: HashMap<u64, (Arc<pictures::Picture>, wgpu::BindGroup)>,
    pub sprite_animations: Vec<(Rect, crate::sprites::SpriteAnimation, (usize, usize, u32))>,
    /// The shared atlas texture, for run-time glyph uploads.
    pub atlas_texture: Option<wgpu::Texture>,
    pub(super) pipeline: Pipeline,
    /// `RenderPipelines.CROSSHAIR`.
    invert_pipeline: Pipeline,
    additive_pipeline: Pipeline,
    depth_pipeline: Pipeline,
    /// `RenderPipelines.GLINT` over a GUI model (`Blend::DepthGlint`).
    depth_glint_pipeline: Pipeline,
    vignette_pipeline: Pipeline,
    end_portal_pipeline: Pipeline,
    /// The layout of `group`, which the CIT glint pipelines share.
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) group: wgpu::BindGroup,
    /// The pack's CIT enchantment glints, when it has any.
    cit_glint: Option<cit_glint::CitGlintPass>,
    pub(super) uniform: wgpu::Buffer,
    pub(super) buffer: wgpu::Buffer,
    capacity: u64,
    format: wgpu::TextureFormat,
    coverage: bool,
    pub(super) atlas_view: wgpu::TextureView,
    /// The resource pack's text shader, when it has one that works.
    pub(super) pack_text: Option<pack_text::PackText>,
    /// Target pixels per window pixel: above 1 for a screenshot drawn larger than the
    /// window (`Renderer::screenshot_quality`).
    pub zoom: f32,
}
impl HudRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        view: &wgpu::TextureView,
        glint_view: &wgpu::TextureView,
        glint_armor_view: &wgpu::TextureView,
        glint_sampler: &wgpu::Sampler,
        coverage: bool,
    ) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sunrise HUD"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../hud.wgsl").into()),
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("HUD"),
            ..Default::default()
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("HUD linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HUD size"),
            contents: bytemuck::bytes_of(&HudUniform::new([1.0; 4], EndPortalFill::default(), 0)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("HUD bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("HUD resources"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(glint_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(glint_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(glint_armor_view),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HUD layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes =
            wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Unorm8x4,3=>Float32,4=>Float32x2];
        // `depth`: the model's depth test, and whether the run writes depth.
        let create = |label, blend, fragment_entry, depth: Option<bool>| {
            let desc = wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<HudVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                primitive: Default::default(),
                depth_stencil: depth.map(|write| wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment_entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            };
            let source = wgpu::ShaderSource::Wgsl(include_str!("../hud.wgsl").into());
            Pipeline::new(device, &desc, source, coverage).expect("the HUD's coverage")
        };
        let pipeline = create("HUD", wgpu::BlendState::ALPHA_BLENDING, "fs_main", None);
        let depth_pipeline = create(
            "HUD model",
            wgpu::BlendState::ALPHA_BLENDING,
            "fs_main",
            Some(true),
        );
        // BlendFunction.INVERT: ONE_MINUS_DST_COLOR, ONE_MINUS_SRC_COLOR,
        // ONE, ZERO.
        let invert_pipeline = create(
            "HUD crosshair",
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::OneMinusDst,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
            },
            "fs_crosshair",
            None,
        );
        // BlendFunction.ADDITIVE: ONE, ONE.
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let additive_pipeline = create(
            "HUD text highlight",
            wgpu::BlendState {
                color: add,
                alpha: add,
            },
            "fs_main",
            None,
        );
        // RenderPipelines.GLINT tests depth EQUAL and writes none; the faces are
        // the model's own, so LEQUAL passes the same pixels.
        let depth_glint_pipeline = create(
            "HUD model glint",
            wgpu::BlendState {
                color: add,
                alpha: add,
            },
            "fs_main",
            Some(false),
        );
        // RenderPipelines.VIGNETTE: ZERO, ONE_MINUS_SRC_COLOR, ZERO, ONE.
        let vignette_pipeline = create(
            "HUD vignette",
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            },
            "fs_vignette",
            None,
        );
        // RenderPipelines.END_PORTAL: opaque.
        let end_portal_pipeline = create(
            "HUD end portal",
            wgpu::BlendState::REPLACE,
            "fs_end_portal",
            None,
        );
        let capacity = 1 << 16;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("HUD vertices"),
            size: capacity * std::mem::size_of::<HudVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pictures: HashMap::new(),
            atlas_texture: None,
            sprite_animations: Vec::new(),
            pipeline,
            invert_pipeline,
            additive_pipeline,
            depth_pipeline,
            depth_glint_pipeline,
            vignette_pipeline,
            end_portal_pipeline,
            layout,
            group,
            cit_glint: None,
            uniform,
            buffer,
            capacity,
            format,
            coverage,
            atlas_view: view.clone(),
            pack_text: None,
            zoom: 1.0,
        })
    }
    /// Draw the CIT glint layers of `textures` (the pack's enchantment rules); `None` or no
    /// textures draws none.
    pub fn set_cit_glints(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: Option<&cit_glint::GlintTextures>,
    ) {
        self.cit_glint = textures.and_then(|t| {
            cit_glint::CitGlintPass::new(device, queue, self.format, &self.layout, t, self.coverage)
        });
    }
    /// Draw the GUI's text with `shader` (`HudAtlas::text_shaders`' `gui`, with the Polytone
    /// expression uniforms its stages read, `shader_modifiers`), or natively.
    pub fn set_text_shader(
        &mut self,
        device: &wgpu::Device,
        shader: Option<(
            &crate::core_shader::CoreProgram,
            &[crate::polytone_post::ExpressionUniform],
        )>,
    ) {
        self.pack_text = shader.and_then(|shader| {
            pack_text::PackText::new(device, self.format, &self.atlas_view, shader, self.coverage)
        });
    }
    /// The GUI text's Polytone blocks, when a pack's text shader draws it.
    pub(crate) fn text_expression_blocks(
        &mut self,
    ) -> Option<&mut crate::gfx::expression_blocks::ExpressionBlocks> {
        self.pack_text.as_mut().map(|text| &mut text.extra)
    }
    /// Stage the batch's `patches` and `image_patches` (client mods' dynamic textures) into
    /// the shared atlas texture.
    pub fn upload_patches(
        &self,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        batch: &HudBatch,
    ) {
        let Some(texture) = &self.atlas_texture else {
            return;
        };
        let glyphs = batch
            .patches
            .iter()
            .map(|p| ([p.x, p.y], [p.width, p.height], &p.rgba[..]));
        let images = batch
            .image_patches
            .iter()
            .map(|p| (p.origin, p.size, &p.rgba[..]));
        for ([x, y], [width, height], rgba) in glyphs.chain(images) {
            if x.saturating_add(width) <= texture.width()
                && y.saturating_add(height) <= texture.height()
            {
                uploads.texture(
                    encoder,
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x, y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    width,
                    height,
                    rgba,
                );
            }
        }
    }
    /// Stage the batch's atlas patches, vertices and uniform: before the
    /// frame's first pass, so the copies share one command buffer.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        batch: &HudBatch,
        size: [u32; 2],
    ) {
        self.prepare_pictures(device, uploads, encoder, batch);
        self.upload_patches(uploads, encoder, batch);
        if batch.vertices.is_empty() || size.contains(&0) {
            return;
        }
        let count = batch.vertices.len() as u64;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("HUD vertices"),
                size: self.capacity * std::mem::size_of::<HudVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        uploads.buffer(
            encoder,
            &self.buffer,
            0,
            bytemuck::cast_slice(&batch.vertices),
        );
        uploads.buffer(
            encoder,
            &self.uniform,
            0,
            bytemuck::bytes_of(&HudUniform::new(
                [
                    size[0] as f32,
                    size[1] as f32,
                    batch.scale * self.zoom,
                    batch.glint_alpha,
                ],
                batch.end_portal,
                batch.glint_colour,
            )),
        );
        if let Some(pack_text) = &self.pack_text {
            pack_text.prepare(uploads, encoder, batch, size, batch.scale * self.zoom);
        }
    }
    /// Draw the batch `prepare` staged.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        batch: &HudBatch,
        size: [u32; 2],
    ) {
        self.draw_range(
            encoder,
            target,
            None,
            depth,
            batch,
            size,
            0..batch.vertices.len() as u32,
            None,
        );
    }
    /// Draw part of the prepared batch, preserving its blend/depth runs.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_range(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        coverage: Option<&wgpu::TextureView>,
        depth: &wgpu::TextureView,
        batch: &HudBatch,
        size: [u32; 2],
        vertices: std::ops::Range<u32>,
        mut timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) {
        if vertices.is_empty() || size.contains(&0) {
            return;
        }
        let mut attachments = [None, None];
        attachments[0] = Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        });
        let attachment_count;
        if let Some(view) = coverage {
            attachments[1] = Some(hud_coverage::attachment(view));
            attachment_count = 2;
        } else {
            attachment_count = 1;
        }
        let runs = batch.runs_in(vertices);
        let mut at = 0;
        while at < runs.len() {
            let clip = match runs[at].1 {
                Blend::Depth(clip) | Blend::DepthGlint(clip) => Some(clip),
                _ => None,
            };
            let end = if let Some(clip) = clip {
                // The model's glint shares its pass and depth buffer.
                at + 1
                    + runs[at + 1..]
                        .iter()
                        .take_while(|(_, b)| *b == Blend::DepthGlint(clip))
                        .count()
            } else {
                runs[at..]
                    .iter()
                    .position(|(_, b)| matches!(b, Blend::Depth(_) | Blend::DepthGlint(_)))
                    .map_or(runs.len(), |n| at + n)
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HUD"),
                timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes("HUD")),
                color_attachments: &attachments[..attachment_count],
                depth_stencil_attachment: clip.map(|_| wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_vertex_buffer(0, self.buffer.slice(..));
            if let Some(clip) = clip {
                let [x, y, w, h] = clip.map(|v| (v as f32 * self.zoom) as u32);
                let (x, y) = (x.min(size[0]), y.min(size[1]));
                let (w, h) = (w.min(size[0] - x), h.min(size[1] - y));
                if w == 0 || h == 0 {
                    at = end;
                    continue;
                }
                pass.set_scissor_rect(x, y, w, h);
            }
            for (range, blend) in &runs[at..end] {
                let pipeline = match blend {
                    Blend::Picture(id) => {
                        self.draw_picture(&mut pass, *id, range.clone(), coverage.is_some());
                        continue;
                    }
                    Blend::Alpha => &self.pipeline,
                    Blend::Invert => &self.invert_pipeline,
                    Blend::Additive => &self.additive_pipeline,
                    Blend::Depth(_) => &self.depth_pipeline,
                    Blend::DepthGlint(_) => &self.depth_glint_pipeline,
                    Blend::Vignette => &self.vignette_pipeline,
                    Blend::EndPortal => &self.end_portal_pipeline,
                    Blend::CitGlint(blend) => {
                        if let Some(cit) = &self.cit_glint {
                            cit.draw(&mut pass, *blend, range.clone(), coverage.is_some());
                        }
                        continue;
                    }
                };
                let pipeline = pipeline.get(coverage.is_some());
                match &self.pack_text {
                    Some(pack_text) if *blend == Blend::Alpha => {
                        for (part, text) in pack_text::text_runs(&batch.vertices, range.clone()) {
                            if text {
                                pack_text.draw(&mut pass, part, coverage.is_some());
                                pass.set_bind_group(0, &self.group, &[]);
                            } else {
                                pass.set_pipeline(pipeline);
                                pass.draw(part, 0..1);
                            }
                        }
                    }
                    _ => {
                        pass.set_pipeline(pipeline);
                        pass.draw(range.clone(), 0..1);
                    }
                }
            }
            at = end;
        }
    }
}
