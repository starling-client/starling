use super::*;
use crate::gfx::hud_coverage::{self, Pipeline};

pub struct HudRenderer {
    pub(super) atlas_pages: Vec<Option<AtlasPageGpu>>,
    pub(super) pictures: HashMap<u64, (Arc<pictures::Picture>, wgpu::BindGroup)>,
    /// Client mods' dynamic textures, shared with the renderers that follow this one on the
    /// same device, and the bind groups of those this renderer draws, with the revision of the
    /// texture each was made for.
    pub(super) dynamic: dynamic_textures::Store,
    pub(super) dynamic_groups: HashMap<u64, (u64, wgpu::BindGroup)>,
    /// Reused by every frame's `prepare_dynamic`.
    pub(super) dynamic_drawn: Vec<u64>,
    pub(super) dynamic_changed: Vec<(u64, Option<(wgpu::TextureView, u64)>)>,
    /// What a dynamic texture released after its draw was submitted draws as: vanilla's missing
    /// texture, made when first needed.
    pub(super) dynamic_missing: Option<wgpu::BindGroup>,
    pub sprite_animations: Vec<(
        AnimatedPlacement,
        crate::sprites::SpriteAnimation,
        (usize, usize, u32),
    )>,
    /// The shared atlas texture, for run-time glyph uploads.
    pub atlas_texture: Option<wgpu::Texture>,
    pub(super) pipeline: Pipeline,
    /// A standalone picture's: `pipeline` without the glint markers at u >= 2.
    pub(super) picture_pipeline: Pipeline,
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
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        view: &wgpu::TextureView,
        glint_view: &wgpu::TextureView,
        glint_armor_view: &wgpu::TextureView,
        lightmap_view: &wgpu::TextureView,
        glint_sampler: &wgpu::Sampler,
        coverage: bool,
        pages: Vec<Option<UploadedAtlasPage>>,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
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
        let group_for = |view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(lightmap_view),
                    },
                ],
            })
        };
        let group = group_for(view);
        let atlas_pages = pages
            .into_iter()
            .map(|page| {
                page.map(|page| AtlasPageGpu {
                    group: group_for(&page.view),
                    page,
                })
            })
            .collect();
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HUD layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Unorm8x4,3=>Float32,4=>Float32x2,5=>Uint32];
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
        let picture_pipeline = create(
            "HUD picture",
            wgpu::BlendState::ALPHA_BLENDING,
            "fs_picture",
            None,
        );
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
            atlas_pages,
            pictures: HashMap::new(),
            dynamic: Default::default(),
            dynamic_groups: HashMap::new(),
            dynamic_drawn: Vec::new(),
            dynamic_changed: Vec::new(),
            dynamic_missing: None,
            atlas_texture: None,
            sprite_animations: Vec::new(),
            pipeline,
            picture_pipeline,
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
    /// expression uniforms its stages read, `shader_modifiers`), or natively; or the device's
    /// refusal of `GUI_TEXT`, a pipeline vanilla requires, which fails the pack reload.
    pub fn set_text_shader(
        &mut self,
        device: &wgpu::Device,
        shader: Option<(
            &sunrise_glsl::Translated,
            &[crate::polytone_post::ExpressionUniform],
        )>,
    ) -> std::result::Result<(), String> {
        let made = shader.map(|shader| {
            pack_text::PackText::new(device, self.format, &self.atlas_view, shader, self.coverage)
        });
        self.pack_text = made.transpose()?.flatten();
        Ok(())
    }
    /// The GUI text's Polytone blocks, when a pack's text shader draws it.
    pub(crate) fn text_expression_blocks(
        &mut self,
    ) -> Option<&mut crate::gfx::expression_blocks::ExpressionBlocks> {
        self.pack_text.as_mut().map(|text| &mut text.extra)
    }
    /// Stage the glyph `patches` into the shared atlas texture.
    pub fn upload_glyph_patches(
        &self,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        patches: &[crate::font::Patch],
    ) {
        let Some(texture) = &self.atlas_texture else {
            return;
        };
        for p in patches {
            if p.x.saturating_add(p.width) <= texture.width()
                && p.y.saturating_add(p.height) <= texture.height()
            {
                uploads.texture(
                    encoder,
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: p.x,
                            y: p.y,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    p.width,
                    p.height,
                    &p.rgba,
                );
            }
        }
    }
    /// The dynamic textures the renderers on this device share: a renderer made on the same
    /// device takes them over with [`Self::share_dynamic`].
    pub fn dynamic(&self) -> &dynamic_textures::Store {
        &self.dynamic
    }
    /// Use `store`, the previous renderer's, so a resource reload keeps every dynamic texture.
    pub fn share_dynamic(&mut self, store: dynamic_textures::Store) {
        self.dynamic = store;
    }
    /// Stage the batch's glyph patches into the shared atlas texture, then apply its dynamic
    /// texture commands in order.
    pub fn upload_patches(
        &self,
        device: &wgpu::Device,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        batch: &HudBatch,
    ) {
        if let Some(texture) = &self.atlas_texture {
            for p in &batch.patches {
                if p.x.saturating_add(p.width) <= texture.width()
                    && p.y.saturating_add(p.height) <= texture.height()
                {
                    uploads.texture(
                        encoder,
                        wgpu::TexelCopyTextureInfo {
                            texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: p.x,
                                y: p.y,
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        p.width,
                        p.height,
                        &p.rgba,
                    );
                }
            }
        }
        self.dynamic.apply(
            device,
            uploads,
            encoder,
            self.atlas_texture.as_ref(),
            &batch.dynamic_commands,
        );
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
        self.upload_patches(device, uploads, encoder, batch);
        self.prepare_dynamic(device, uploads, encoder, batch);
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
        let runs: SmallVec<[(std::ops::Range<u32>, Blend); 16]> = batch.runs_in(vertices).collect();
        let mut at = 0;
        while at < runs.len() {
            let clip = runs[at].1.scissor();
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
            let full = [0, 0, size[0], size[1]];
            let depth_clip = clip.map_or(full, |clip| {
                let [x, y, w, h] = clip.map(|v| (v as f32 * self.zoom) as u32);
                [
                    x.min(size[0]),
                    y.min(size[1]),
                    x.saturating_add(w).min(size[0]),
                    y.saturating_add(h).min(size[1]),
                ]
            });
            let mut previous = full;
            for (range, blend) in &runs[at..end] {
                for (range, clip) in batch.scissors_in(range.clone()) {
                    let [x0, y0, x1, y1] = clip.map_or(depth_clip, |rect| {
                        let scale = batch.scale * self.zoom;
                        // Fractional engine clips retain the old pixel-centre coverage;
                        // integer GUI rectangles at vanilla's integer scale are exact.
                        let [l, t, r, b] = rect.map(|v| (v * scale - 0.5).ceil() as u32);
                        let [x0, y0, x1, y1] = depth_clip;
                        [l.max(x0), t.max(y0), r.min(x1), b.min(y1)]
                    });
                    if x0 >= x1 || y0 >= y1 {
                        continue;
                    }
                    let scissor = [x0, y0, x1 - x0, y1 - y0];
                    if scissor != previous {
                        pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
                        previous = scissor;
                    }
                    let pipeline = match blend {
                        Blend::Picture(id) => {
                            self.draw_picture(&mut pass, *id, range, coverage.is_some());
                            continue;
                        }
                        Blend::Dynamic(handle) => {
                            self.draw_dynamic(&mut pass, *handle, range, coverage.is_some());
                            continue;
                        }
                        Blend::AtlasPage(page, blend) => {
                            let Some(page) = self.atlas_pages.get(*page).and_then(Option::as_ref)
                            else {
                                continue;
                            };
                            let pipeline = match blend {
                                AtlasBlend::Alpha => &self.pipeline,
                                AtlasBlend::Invert => &self.invert_pipeline,
                                AtlasBlend::Additive => &self.additive_pipeline,
                                AtlasBlend::Vignette => &self.vignette_pipeline,
                            };
                            pass.set_bind_group(0, &page.group, &[]);
                            pass.set_pipeline(pipeline.get(coverage.is_some()));
                            pass.draw(range, 0..1);
                            pass.set_bind_group(0, &self.group, &[]);
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
                                cit.draw(&mut pass, *blend, range, coverage.is_some());
                            }
                            continue;
                        }
                    };
                    let pipeline = pipeline.get(coverage.is_some());
                    match &self.pack_text {
                        Some(pack_text) if *blend == Blend::Alpha => {
                            for (part, text) in pack_text::text_runs(&batch.vertices, range) {
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
                            pass.draw(range, 0..1);
                        }
                    }
                }
            }
            at = end;
        }
    }
}
