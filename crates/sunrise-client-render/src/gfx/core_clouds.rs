//! A resource pack's `core/clouds` shader (26.3's `CloudRenderer`, `RenderPipelines.CLOUDS` and
//! `FLAT_CLOUDS`): the clouds drawn by the pack's GLSL from the faces `CloudRenderer.buildMesh`
//! encodes (`CloudFaces`, an `R8_SINT` texel buffer, here a storage buffer of ints) and its
//! `CloudInfo` block, as a pack that shades or reshapes the clouds expects; under Improved
//! Transparency `OIT_CLOUDS` and `OIT_FLAT_CLOUDS` draw them with the same shader compiled for
//! each `OitStage` (`CloudRenderer.renderOit`, in `Oit`'s cloud passes). Without one the clouds
//! draw natively (`clouds.rs`, `vs_cloud`).

use sunrise_glsl::{Glue, Translated};

/// `core/clouds`: no vertex input, the face and its corner from `gl_VertexIndex`.
pub(crate) const GLUE: Glue = Glue {
    inputs: &[],
    prelude: "",
    setup: "",
    // 26.3's ProjMat already has wgpu's reversed 0 to 1 depth.
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
    fragment_finish: None,
    interface: sunrise_shaders::DEVICELESS_INTERFACE_LIMITS,
};

/// The pipeline's own block and texel buffer, beside the vanilla blocks.
pub(crate) const INFO_BLOCK: &str = "CloudInfo";
pub(crate) const FACES: &str = "CloudFaces";

/// Each vanilla block's offset in the one buffer (`core_shader::BLOCKS` order), then
/// `CloudInfo`'s.
const BLOCK_OFFSETS: [u64; 4] = [0, 256, 512, 768];
const INFO_OFFSET: u64 = 1024;
const INFO_SIZE: u64 = 48;
/// The uniform bytes a frame writes: the vanilla blocks (`core_lines::uniforms`), `CloudInfo`.
const UNIFORM_BYTES: usize = (INFO_OFFSET + INFO_SIZE) as usize;
/// The faces buffer's first size, in ints (a thousand faces); it grows to what a frame needs.
const FIRST_CAPACITY: u64 = 3 * 1024;

/// Why a frame's faces don't fit: more bytes than the device binds to one storage buffer. The
/// frame's clouds then draw natively.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TooManyFaces {
    pub bytes: u64,
    pub limit: u64,
}

/// What `OitPipelineSet.build` adds to `OIT_CLOUDS_SNIPPET` for one `OitStage`: the stage's
/// defines (`OIT_SNIPPET`'s, then the stage snippet's) and the samplers its pass gives
/// (`OitRenderPassProvider`). A program reading any other is refused.
pub(crate) struct OitStageSource {
    /// The pipeline's location (`OitPipelineSet.build`'s `pipeline/oit_<stage>_clouds`).
    pub pipeline: &'static str,
    pub defines: &'static [&'static str],
    pub samplers: &'static [&'static str],
}

/// The stage samplers, in the order [`super::oit::CloudSamples::views`] gives their textures (the
/// pack text's Improved Transparency stages bind them by the same names).
pub(crate) const STAGE_SAMPLERS: [&str; 3] = ["DepthBoundsSampler", "Coeff0", "Coeff1"];

/// The defines of `CLOUDS` and `FLAT_CLOUDS`: none of their own. The backend's, which every
/// program has, the translator adds (`core_shader::DEPTH_ZERO_TO_ONE`).
pub(crate) const CLASSIC_DEFINES: [&str; 0] = [];

/// `OitStage`'s order. `OIT_COEFF_COUNT` is `LevelRenderer.OIT_COEFFICIENT_COUNT` (8), its
/// attachments `OIT_TRANSMITTANCE_TARGET_COUNT` (2), as `oit.rs` draws them. The backend's
/// define, which every program has, the translator adds.
pub(crate) const OIT_STAGES: [OitStageSource; 3] = [
    OitStageSource {
        pipeline: "minecraft:pipeline/oit_depth_bounds_clouds",
        defines: &[
            "OIT",
            "OIT_WAVELET_RANK 2",
            "OIT_COEFF_COUNT 8",
            "OIT_COEFF_ATTACHMENT_COUNT 2",
            "OIT_ALPHA_ONLY",
            "OIT_DEPTH_BOUNDS",
        ],
        samplers: &[],
    },
    OitStageSource {
        pipeline: "minecraft:pipeline/oit_transmittance_clouds",
        defines: &[
            "OIT",
            "OIT_WAVELET_RANK 2",
            "OIT_COEFF_COUNT 8",
            "OIT_COEFF_ATTACHMENT_COUNT 2",
            "OIT_ALPHA_ONLY",
            "OIT_TRANSMITTANCE",
        ],
        samplers: &[STAGE_SAMPLERS[0]],
    },
    OitStageSource {
        pipeline: "minecraft:pipeline/oit_accumulate_clouds",
        defines: &[
            "OIT",
            "OIT_WAVELET_RANK 2",
            "OIT_COEFF_COUNT 8",
            "OIT_COEFF_ATTACHMENT_COUNT 2",
            "OIT_ACCUMULATE",
        ],
        samplers: &STAGE_SAMPLERS,
    },
];

/// `RenderPipelines.CLOUDS`' location (`FLAT_CLOUDS` is made from the same program).
pub(crate) const CLASSIC_PIPELINE: &str = "minecraft:pipeline/clouds";

/// Why a translated program can't make its pipeline here, as vanilla's builder refuses a
/// descriptor missing from the pipeline's layout: it reads one of vanilla's samplers (`Sampler0`
/// to `Sampler2`), which no clouds pass binds. Checked when the packs load, so the load fails as
/// vanilla's compile failure fails its reload; the error names the declaration it is about. (A
/// colour output the device doesn't take, which vanilla doesn't check, draws natively instead:
/// `clouds_for_frame`.)
pub(crate) fn check(program: &Translated) -> Result<(), sunrise_glsl::Error> {
    for (name, binding) in crate::core_shader::SAMPLERS {
        if !(program.stages_reading(binding) | program.stages_reading(binding + 1)).is_empty() {
            return Err(program
                .declared_at(binding)
                .error(format!("{name} is not a sampler of this pipeline")));
        }
    }
    Ok(())
}

/// One program's pipelines (fancy clouds with back faces culled, flat ones without) and where
/// its group binds what.
struct Draw {
    fancy: wgpu::RenderPipeline,
    flat: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Where the shader reads `CloudFaces`; none when it doesn't (its vertex stage may place
    /// the quads from `gl_VertexIndex` alone).
    faces_binding: Option<u32>,
    info_binding: Option<u32>,
    /// The stage samplers it declares and its stage gives: the texture's binding (the sampler
    /// object's the next) and its place in [`STAGE_SAMPLERS`].
    samplers: Vec<(u32, usize)>,
    /// Vanilla's blocks' sizes as the program binds them
    /// (`expression_blocks::vanilla_block_size`), then `CloudInfo`'s.
    block_sizes: [u64; 4],
    info_size: u64,
}

/// The Improved Transparency stages' programs and their groups.
struct OitDraws {
    stages: [Draw; 3],
    /// `getClampToEdge(FilterMode.NEAREST)`, which the stages' samplers are.
    sampler: wgpu::Sampler,
    /// The groups, for the targets of the generation they were made for
    /// ([`super::oit::CloudSamples`]); none until the first frame under Improved Transparency,
    /// and again when the faces buffer is made again.
    groups: Option<(u64, [wgpu::BindGroup; 3])>,
}

/// The packs' cloud pipelines (outside and under Improved Transparency) and their buffers.
pub(crate) struct CoreClouds {
    /// `CLOUDS` and `FLAT_CLOUDS`, and their group.
    classic: Option<(Draw, wgpu::BindGroup)>,
    /// `OIT_CLOUDS` and `OIT_FLAT_CLOUDS`, where the device has Improved Transparency.
    oit: Option<Box<OitDraws>>,
    uniform: wgpu::Buffer,
    faces: wgpu::Buffer,
    /// Ints the faces buffer holds.
    capacity: u64,
    /// Whether any program reads `CloudFaces`; when none does, the faces stay on the CPU,
    /// counted.
    reads_faces: bool,
    /// The faces uploaded last, in quads, and what they were built for: while it holds, they
    /// are not built again (`CloudRenderer.prepare`).
    quads: u32,
    key: Option<crate::clouds::pack::FacesKey>,
}

impl Draw {
    /// `program`'s pipelines into `targets`, the depth written by the fancy and the flat one
    /// as `writes` says; `samplers` are the stage's.
    fn new(
        device: &wgpu::Device,
        program: &Translated,
        targets: &[Option<wgpu::ColorTargetState>],
        writes: [bool; 2],
        samplers: &[&str],
    ) -> Self {
        let binding = |name: &str| {
            program
                .other
                .iter()
                .position(|n| n == name)
                .map(|i| program.other_binding(i))
        };
        let faces_binding = binding(FACES);
        let info_binding = binding(INFO_BLOCK);
        let samplers: Vec<(u32, usize)> = (STAGE_SAMPLERS.iter().enumerate())
            .filter(|(_, name)| samplers.contains(name))
            .filter_map(|(i, name)| Some((binding(name)?, i)))
            .collect();
        let module = |label, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex = module("Resource pack clouds (vertex)", &program.vertex);
        let fragment = module("Resource pack clouds (fragment)", &program.fragment);
        let entry = |binding: u32, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty,
            count: None,
        };
        let block_sizes: [u64; 4] =
            std::array::from_fn(|i| super::expression_blocks::vanilla_block_size(program, i));
        let block = |binding: u32, size: u64| {
            entry(
                binding,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size),
                },
            )
        };
        let mut entries: Vec<wgpu::BindGroupLayoutEntry> =
            (0..4).map(|b| block(b, block_sizes[b as usize])).collect();
        let info_size = info_binding.map_or(INFO_SIZE, |b| {
            super::expression_blocks::block_size(program, b, INFO_SIZE)
        });
        entries.extend(info_binding.map(|b| block(b, info_size)));
        entries.extend(faces_binding.map(|binding| {
            entry(
                binding,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            )
        }));
        for &(binding, _) in &samplers {
            entries.push(entry(
                binding,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            ));
            entries.push(entry(
                binding + 1,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
            ));
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Resource pack clouds"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Resource pack clouds"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = |label, cull_mode, write| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vertex,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &fragment,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            fancy: pipeline("Resource pack clouds", Some(wgpu::Face::Back), writes[0]),
            flat: pipeline("Resource pack flat clouds", None, writes[1]),
            layout,
            faces_binding,
            info_binding,
            samplers,
            block_sizes,
            info_size,
        }
    }

    /// The group: the vanilla blocks and `CloudInfo` from `uniform`, the faces when the shader
    /// reads them, and the stage's textures (`views`, with `sampler`) it declares.
    fn bind(
        &self,
        device: &wgpu::Device,
        uniform: &wgpu::Buffer,
        faces: &wgpu::Buffer,
        samples: Option<(&[&wgpu::TextureView; 3], &wgpu::Sampler)>,
    ) -> wgpu::BindGroup {
        let range = |binding: u32, offset: u64, size: u64| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: uniform,
                offset,
                size: wgpu::BufferSize::new(size),
            }),
        };
        let mut entries: Vec<wgpu::BindGroupEntry> = (0..4)
            .map(|i| range(i as u32, BLOCK_OFFSETS[i], self.block_sizes[i]))
            .collect();
        entries.extend(
            self.info_binding
                .map(|b| range(b, INFO_OFFSET, self.info_size)),
        );
        entries.extend(self.faces_binding.map(|binding| wgpu::BindGroupEntry {
            binding,
            resource: faces.as_entire_binding(),
        }));
        if let Some((views, sampler)) = samples {
            for &(binding, i) in &self.samplers {
                entries.push(wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(views[i]),
                });
                entries.push(wgpu::BindGroupEntry {
                    binding: binding + 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                });
            }
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Resource pack clouds"),
            layout: &self.layout,
            entries: &entries,
        })
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, fancy: bool, group: &wgpu::BindGroup) {
        pass.set_pipeline(if fancy { &self.fancy } else { &self.flat });
        pass.set_bind_group(0, group, &[]);
    }
}

impl CoreClouds {
    /// The pipelines for `programs`, the classic ones drawing into the world's `format`, the
    /// Improved Transparency ones where the device supports it. Any backend refusal fails the load. A shader that doesn't read `CloudFaces` is drawn too, as many quads as the
    /// faces.
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        (classic, stages): (Option<&Translated>, Option<&[Translated; 3]>),
    ) -> Result<Self, String> {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack clouds"),
            size: INFO_OFFSET + 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let faces = faces_buffer(device, FIRST_CAPACITY);
        // RenderPipelines.CLOUDS: translucent, the default depth test and writes, back faces
        // culled; FLAT_CLOUDS without culling.
        // Checked when the packs loaded (`check`): what still fails here is the device's own
        // refusal, which reaches the pack reload through build_renderer.
        let classic = classic
            .map(|program| {
                crate::core_shader::scoped(device, CLASSIC_PIPELINE, || {
                    let targets = [Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })];
                    let draw = Draw::new(device, program, &targets, [true, true], &[]);
                    let group = draw.bind(device, &uniform, &faces, None);
                    (draw, group)
                })
            })
            .transpose()?;
        // OIT_CLOUDS: each stage's targets and depth test, the depth written only by the
        // bounds stage's fancy pipeline (its DEFAULT depth modifier); OIT_FLAT_CLOUDS writes
        // none.
        let oit = stages
            .filter(|_| super::oit::supported(device))
            .map(|stages| {
                let stage = |i: usize| {
                    let stage = super::oit::Stage::ALL[i];
                    let program = &stages[i];
                    let first = stage == super::oit::Stage::DepthBounds;
                    let targets = super::oit::stage_targets(stage);
                    crate::core_shader::scoped(device, OIT_STAGES[i].pipeline, || {
                        Draw::new(
                            device,
                            program,
                            &targets,
                            [first, false],
                            OIT_STAGES[i].samplers,
                        )
                    })
                };
                Ok::<_, String>(Box::new(OitDraws {
                    stages: [stage(0)?, stage(1)?, stage(2)?],
                    sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                        label: Some("Resource pack clouds"),
                        ..Default::default()
                    }),
                    groups: None,
                }))
            })
            .transpose()?;
        let reads_faces = classic
            .iter()
            .map(|(d, _)| d)
            .chain(oit.iter().flat_map(|o| &o.stages))
            .any(|d| d.faces_binding.is_some());
        Ok(Self {
            classic,
            oit,
            uniform,
            faces,
            capacity: FIRST_CAPACITY,
            reads_faces,
            quads: 0,
            key: None,
        })
    }

    /// Whether `CLOUDS` is drawn from the pack's shader.
    pub(crate) fn classic(&self) -> bool {
        self.classic.is_some()
    }

    /// Whether `OIT_CLOUDS` is drawn from the pack's shader.
    pub(crate) fn improved_transparency(&self) -> bool {
        self.oit.is_some()
    }

    /// The frame's blocks (`vanilla`, as `core_lines::uniforms` makes them, then `CloudInfo`)
    /// and the clouds' faces (`clouds::pack`), uploaded: the faces built and uploaded only when
    /// `CloudRenderer.prepare` would build them again. The quads to draw; or why the faces
    /// don't fit the device, which leaves the frame's clouds to the native mesh.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        vanilla: &[u8; INFO_OFFSET as usize],
        input: &crate::clouds::Input<'_>,
    ) -> Result<u32, TooManyFaces> {
        let key = crate::clouds::pack::faces_key(input);
        if self.key.as_ref() != Some(&key) {
            let faces = crate::clouds::pack::faces(input);
            let ints = faces.len() as u64;
            if self.reads_faces && ints > self.capacity {
                // The faces' own bytes against what the device binds; the buffer grows to the
                // next power of two within that.
                let limit = device.limits().max_storage_buffer_binding_size;
                if ints * 4 > limit {
                    // The faces kept are still the last key's: the next frame tries again.
                    return Err(TooManyFaces {
                        bytes: ints * 4,
                        limit,
                    });
                }
                let capacity = ints.next_power_of_two().min(limit / 4);
                self.capacity = capacity;
                self.faces = faces_buffer(device, capacity);
                if let Some((draw, group)) = self.classic.as_mut() {
                    *group = draw.bind(device, &self.uniform, &self.faces, None);
                }
                if let Some(oit) = self.oit.as_mut() {
                    oit.groups = None;
                }
            }
            if self.reads_faces && !faces.is_empty() {
                uploads.buffer(encoder, &self.faces, 0, bytemuck::cast_slice(&faces));
            }
            self.quads = (faces.len() / 3) as u32;
            self.key = Some(key);
        }
        let mut data = [0u8; UNIFORM_BYTES];
        data[..INFO_OFFSET as usize].copy_from_slice(vanilla);
        data[INFO_OFFSET as usize..].copy_from_slice(&crate::clouds::pack::info(input));
        uploads.buffer(encoder, &self.uniform, 0, &data);
        Ok(self.quads)
    }

    /// The Improved Transparency stages' groups for `samples`' targets, made again only when
    /// those targets or the faces buffer were.
    pub(super) fn bind_oit(
        &mut self,
        device: &wgpu::Device,
        samples: &super::oit::CloudSamples<'_>,
    ) {
        let Some(oit) = self.oit.as_mut() else {
            return;
        };
        if oit
            .groups
            .as_ref()
            .is_some_and(|(made, _)| *made == samples.generation)
        {
            return;
        }
        let views = &samples.views;
        let group = |i: usize| {
            oit.stages[i].bind(
                device,
                &self.uniform,
                &self.faces,
                Some((views, &oit.sampler)),
            )
        };
        oit.groups = Some((samples.generation, [group(0), group(1), group(2)]));
    }

    /// The faces prepared, from `indices` (the renderer's quad indices, which hold them):
    /// `CLOUDS` for fancy clouds, `FLAT_CLOUDS` for fast ones. The caller sets its own pipeline
    /// and group 0 again after.
    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        fancy: bool,
        indices: &wgpu::Buffer,
    ) {
        let Some((draw, group)) = self.classic.as_ref().filter(|_| self.quads > 0) else {
            return;
        };
        draw.draw(pass, fancy, group);
        self.draw_quads(pass, indices);
    }

    /// `CloudRenderer.renderOit` for `stage`, in the pass `Oit` begins for it: `OIT_CLOUDS`'s
    /// or `OIT_FLAT_CLOUDS`'s pipeline of the stage ([`CoreClouds::bind_oit`] first).
    pub(super) fn draw_oit(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        stage: super::oit::Stage,
        fancy: bool,
        indices: &wgpu::Buffer,
    ) {
        let Some(oit) = self.oit.as_ref().filter(|_| self.quads > 0) else {
            return;
        };
        let Some((_, groups)) = oit.groups.as_ref() else {
            return;
        };
        oit.stages[stage as usize].draw(pass, fancy, &groups[stage as usize]);
        self.draw_quads(pass, indices);
    }

    fn draw_quads(&self, pass: &mut wgpu::RenderPass<'_>, indices: &wgpu::Buffer) {
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.quads * 6, 0, 0..1);
    }
}

fn faces_buffer(device: &wgpu::Device, ints: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Resource pack cloud faces"),
        size: ints * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Whether a pack's clouds draw this frame in place of the native mesh: the pack's shader is
/// drawn for the frame's path (`pack_shader` outside Improved Transparency,
/// `pack_oit_shader` under it), the clouds are on, and nothing else draws them its own way (a
/// shader pack, a native mod's clouds, the title screen), nor needs the native mesh (HDR's
/// cloud marks follow it).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CloudsFrame {
    pub pack_shader: bool,
    pub pack_oit_shader: bool,
    pub clouds: bool,
    pub improved_transparency: bool,
    pub shader_pack: bool,
    pub title: bool,
    pub cloud_marks: bool,
}

impl CloudsFrame {
    pub(crate) fn pack_draws(self) -> bool {
        let shader = if self.improved_transparency {
            self.pack_oit_shader
        } else {
            self.pack_shader
        };
        shader && self.clouds && !self.shader_pack && !self.title && !self.cloud_marks
    }
}

/// Whether the native mesh is prepared this frame (`draw_frame`): the clouds are the game's own,
/// and no pack's faces were prepared in its place.
pub(crate) fn native_mesh(pack_quads: Option<u32>) -> bool {
    pack_quads.is_none()
}

/// Whether the classic world pass draws `quads` clouds, a pack's or the native mesh's: not where
/// a shader pack draws them its own way, nor under Improved Transparency, whose stages draw them.
pub(crate) fn classic_pass_draws(quads: u32, shader_pack_draws: bool, oit_on: bool) -> bool {
    quads > 0 && !shader_pack_draws && !oit_on
}

/// What `Oit`'s cloud passes draw (`Extras::clouds`, `CloudRenderer.renderOit`): a pack's
/// `OIT_CLOUDS` when its faces were prepared this frame, else the native mesh when it has quads.
pub(crate) fn oit_clouds<'a, F: ?Sized>(
    pack: Option<&'a F>,
    native: &'a F,
    native_quads: u32,
) -> Option<&'a F> {
    match pack {
        Some(pack) => Some(pack),
        None => (native_quads > 0).then_some(native),
    }
}

/// The clouds on the renderer: a pack's `core/clouds`, or the native mesh.
impl super::Renderer {
    /// The resource packs' `core/clouds` (`HudAtlas::clouds_shader`), or the native clouds. As
    /// `set_lines_shader`: a pack's GLSL writes vanilla's gamma, so a frame of linear light keeps
    /// the native clouds.
    pub fn set_clouds_shader(
        &mut self,
        programs: Option<&crate::clouds::pack::Programs>,
    ) -> Result<(), String> {
        self.core_clouds = programs
            .map(|p| clouds_for_frame(&self.device, self.output.frame, p))
            .transpose()?
            .flatten();
        Ok(())
    }
}

/// Link every required pipeline before deciding whether this frame draws the pack's clouds. A
/// program past the device's limits (`sunrise_glsl::PastDeviceLimit`), among the pipelines the
/// device makes, leaves the native clouds.
fn clouds_for_frame(
    device: &wgpu::Device,
    frame: super::FrameConfiguration,
    programs: &crate::clouds::pack::Programs,
) -> Result<Option<CoreClouds>, String> {
    let fits = |program, pipeline, targets: &[wgpu::TextureFormat]| {
        crate::core_shader::within_device_limits(device, program, pipeline, targets)
    };
    let oit = super::oit::supported(device);
    let stage_fits = |i: usize| {
        let targets: Vec<wgpu::TextureFormat> =
            (super::oit::stage_targets(super::oit::Stage::ALL[i]).iter())
                .filter_map(|t| t.as_ref().map(|t| t.format))
                .collect();
        fits(&programs.oit[i], OIT_STAGES[i].pipeline, &targets)
    };
    if !fits(&programs.classic, CLASSIC_PIPELINE, &[frame.format])
        || (oit && !(0..3).all(stage_fits))
    {
        return Ok(None);
    }
    let clouds = CoreClouds::new(
        device,
        frame.format,
        (Some(&programs.classic), Some(&programs.oit)),
    )?;
    Ok((!frame.linear_light).then_some(clouds))
}

#[cfg(test)]
#[path = "core_clouds_tests.rs"]
mod tests;
