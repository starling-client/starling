//! A resource pack's `core/text` on world text, as vanilla 26.3's `TEXT_POLYGON_OFFSET` and
//! `TEXT_SEE_THROUGH` draw text displays, name tags and their backgrounds. Only these two
//! paths and the GUI's text take the pack's shader; the other world text paths, Improved
//! Transparency's depth-tested text and the native world text with its polygon-offset outline
//! among them, draw with the native shaders.

use crate::core_shader::{CoreProgram, Glue};
use std::sync::Arc;

/// The entity vertices' light bytes are sky then block; vanilla's `UV2` is block then sky.
pub(crate) const WORLD_GLUE: Glue = Glue {
    inputs: &[
        ("Position", "vec3"),
        ("Color", "vec4"),
        ("UV0", "vec2"),
        ("UV2", "ivec2"),
    ],
    prelude: "layout(location = 0) in vec3 hz_pos;\n\
              layout(location = 1) in vec2 hz_uv;\n\
              layout(location = 3) in vec4 hz_color;\n\
              layout(location = 4) in uvec4 hz_light;",
    setup: "Position = hz_pos; Color = hz_color; UV0 = hz_uv; UV2 = ivec2(hz_light.y, hz_light.x);",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
};

pub(crate) const SEE_THROUGH_GLUE: Glue = Glue {
    inputs: &[("Position", "vec3"), ("Color", "vec4"), ("UV0", "vec2")],
    prelude: "layout(location = 0) in vec3 hz_pos;\n\
              layout(location = 1) in vec2 hz_uv;\n\
              layout(location = 3) in vec4 hz_color;",
    setup: "Position = hz_pos; Color = hz_color; UV0 = hz_uv;",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
};

#[derive(Default)]
pub struct TextShaders {
    pub gui: Option<Arc<CoreProgram>>,
    pub world: Option<Arc<CoreProgram>>,
    pub see_through: Option<Arc<CoreProgram>>,
    /// Each program's Polytone expression uniforms (`shader_modifiers`), resolved when it
    /// loaded: the blocks its stages read, with their programs, bound after the pipeline's own
    /// (`expression_blocks`).
    pub expressions: TextExpressions,
}

/// [`TextShaders`]' expression uniforms, program by program.
#[derive(Debug, Default)]
pub struct TextExpressions {
    pub gui: Vec<crate::polytone_post::ExpressionUniform>,
    pub world: Vec<crate::polytone_post::ExpressionUniform>,
    pub see_through: Vec<crate::polytone_post::ExpressionUniform>,
}

#[derive(Clone, Copy)]
pub(super) struct PackBlocks {
    pub(super) globals: [u8; 48],
    pub(super) fog: [f32; 6],
}

impl super::Renderer {
    pub fn set_world_text_shaders(&mut self, shaders: &TextShaders) {
        let expressions = &shaders.expressions;
        self.core_text = CoreText::new(
            &self.device,
            self.output.frame.format,
            (shaders.world.as_deref()).map(|p| (p, &expressions.world[..])),
            (shaders.see_through.as_deref()).map(|p| (p, &expressions.see_through[..])),
            &self.entity_atlas,
            (&self.lightmap_view, &self.lightmap_sampler),
        );
    }

    /// The vanilla `Globals` block a pack's core shaders read this frame.
    pub(super) fn pack_globals(&self, frame: &super::Frame) -> [u8; 48] {
        let globals = crate::post_chain::Globals {
            glint_alpha: frame.glint_alpha,
            ..frame.post_effects.globals
        };
        let size = [self.config.width, self.config.height];
        globals.bytes(size, frame.post_effects.menu_blur_radius as i32)
    }

    pub(super) fn pack_blocks(
        &self,
        frame: &super::Frame,
        fog: [f32; 4],
        ends: [f32; 4],
    ) -> PackBlocks {
        PackBlocks {
            globals: self.pack_globals(frame),
            fog: [fog[0], fog[1], fog[2], fog[3], ends[0], ends[1]],
        }
    }

    pub(super) fn bind_world_text(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        allowed: bool,
        see_through: bool,
    ) {
        let pack = self
            .core_text
            .as_ref()
            .filter(|_| allowed)
            .and_then(|core| {
                let part = if see_through {
                    core.see_through.as_ref()
                } else {
                    core.polygon_offset.as_ref()
                };
                part.map(|part| (&part.pipeline, &part.group))
            });
        match pack {
            Some((pipeline, group)) => {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
            }
            None => pass.set_bind_group(0, &self.entity_group, &[]),
        }
    }
}

/// One of the pack's world text pipelines: the world's and the see-through text are the text
/// shader with different defines, so each has its own layout and may read different blocks.
pub(super) struct Part {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) group: wgpu::BindGroup,
    /// The Polytone blocks its stages read (`PolyGlobals` and the packs' expression uniforms,
    /// `shader_modifiers`), in the shared buffer after vanilla's blocks.
    extra: super::expression_blocks::ExpressionBlocks,
}

/// The pack's world text pipelines over one uniform buffer: vanilla's four blocks once, then the
/// world text's Polytone blocks, then the see-through text's.
pub(crate) struct CoreText {
    pub(super) polygon_offset: Option<Part>,
    pub(super) see_through: Option<Part>,
    uniform: wgpu::Buffer,
}

/// What [`CoreText::new`] builds a pipeline from: its name in the log, its program with its
/// expression uniforms, and its depth state.
type PartSpec<'a> = (
    &'a str,
    Option<(
        &'a CoreProgram,
        &'a [crate::polytone_post::ExpressionUniform],
    )>,
    wgpu::DepthStencilState,
);

impl CoreText {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        world: Option<(&CoreProgram, &[crate::polytone_post::ExpressionUniform])>,
        see_through: Option<(&CoreProgram, &[crate::polytone_post::ExpressionUniform])>,
        atlas: &wgpu::TextureView,
        lightmap: (&wgpu::TextureView, &wgpu::Sampler),
    ) -> Option<Self> {
        if world.is_none() && see_through.is_none() {
            return None;
        }
        let depth = |write, compare, bias| wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias,
        };
        let offset = wgpu::DepthBiasState {
            constant: 10,
            slope_scale: 1.0,
            clamp: 0.0,
        };
        let specs: [PartSpec; 2] = [
            (
                "world",
                world,
                depth(true, wgpu::CompareFunction::GreaterEqual, offset),
            ),
            (
                "see-through",
                see_through,
                depth(false, wgpu::CompareFunction::Always, Default::default()),
            ),
        ];
        // Vanilla's blocks first, then each pipeline's Polytone blocks after the last's.
        let mut base = 1024;
        let extras = specs.each_ref().map(|(_, program, _)| {
            program.map(|(core, expressions)| {
                let extra =
                    super::expression_blocks::ExpressionBlocks::new(core, expressions, base);
                base = extra.size();
                extra
            })
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack world text uniforms"),
            size: base,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyphs = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Resource pack world text glyphs"),
            ..Default::default()
        });
        let textures = [(4, atlas, &glyphs), (8, lightmap.0, lightmap.1)];
        let mut parts = [None, None];
        for (slot, ((name, program, depth), extra)) in
            parts.iter_mut().zip(specs.into_iter().zip(extras))
        {
            let (Some((program, _)), Some(extra)) = (program, extra) else {
                continue;
            };
            let label = format!("Resource pack {name} text");
            // Each pipeline is refused on its own: a pack's shader is untrusted, and the backend
            // failing to compile it is an internal error.
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
            let part = Self::part(
                device,
                format,
                &label,
                program,
                extra,
                depth,
                (&uniform, &textures),
            );
            let internal = pollster::block_on(internal.pop());
            match internal.or(pollster::block_on(validation.pop())) {
                Some(e) => log::warn!("the resource pack's {name} text shader is not used: {e}"),
                None => *slot = Some(part),
            }
        }
        let [polygon_offset, see_through] = parts;
        (polygon_offset.is_some() || see_through.is_some()).then_some(Self {
            polygon_offset,
            see_through,
            uniform,
        })
    }

    /// One pipeline of `program` over the shared `uniform` buffer: each block and sampler bound
    /// only to the stages that read it (vanilla's blocks, then `extra`'s; the glyphs at 4 and 5,
    /// the lightmap at 8 and 9, `textures`).
    fn part(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        label: &str,
        program: &CoreProgram,
        extra: super::expression_blocks::ExpressionBlocks,
        depth: wgpu::DepthStencilState,
        (uniform, textures): (
            &wgpu::Buffer,
            &[(u32, &wgpu::TextureView, &wgpu::Sampler); 2],
        ),
    ) -> Part {
        let blocks = super::expression_blocks::vanilla_blocks(program);
        let (mut entries, mut resources) =
            super::expression_blocks::bindings(&blocks, &extra, uniform);
        let (layout, group) = super::expression_blocks::sampler_bindings(program, textures);
        entries.extend(layout);
        resources.extend(group);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &entries,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &layout,
            entries: &resources,
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
            depth_stencil: Some(depth),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Part {
            pipeline,
            group,
            extra,
        }
    }

    /// Vanilla's blocks (`data`, as `core_lines::uniforms` lays them out) once into the shared
    /// buffer, then each pipeline's Polytone blocks.
    pub(super) fn prepare(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        data: &[u8; 1024],
    ) {
        uploads.buffer(encoder, &self.uniform, 0, data);
        for part in [&self.polygon_offset, &self.see_through]
            .into_iter()
            .flatten()
        {
            part.extra.upload(uploads, encoder, &self.uniform);
        }
    }

    /// The world's and the see-through text's Polytone blocks.
    pub(super) fn expression_blocks(
        &mut self,
    ) -> impl Iterator<Item = &mut super::expression_blocks::ExpressionBlocks> {
        [self.polygon_offset.as_mut(), self.see_through.as_mut()]
            .into_iter()
            .flatten()
            .map(|part| &mut part.extra)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "core_text_expression_tests.rs"]
mod expression_tests;
