//! The loading screen's own renderer: the title panorama, the HUD batch, and nothing else.
//!
//! `Renderer::new` builds every pipeline the world will ever need before it returns, and the
//! loading screen used to wait for all of them: on a PowerVR B-Series that is 11 seconds before
//! anything reaches the screen, and the window stays on whatever was behind it. What the loading
//! screen actually draws is a cube and some text — `loading_frame` hands the renderer a `Frame`
//! with `title: true` and every other field empty.
//!
//! So this opens the device itself, keeps the two pipelines it draws with, and hands the open
//! device, queue and surface to the world's renderer once the resources are ready
//! (`into_opened`). The world's pipelines are then built once, instead of once for the boot
//! screen, again for the title artwork and again for the world.

use std::sync::Arc;

use wgpu::util::DeviceExt as _;

use super::{Opened, panorama, upload};
use crate::Result;
use crate::hud::{HudBatch, HudRenderer};
use winit::window::Window;

/// What the loading screen draws with, and what it hands on afterwards.
pub(crate) struct BootRenderer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    device_lost: Arc<std::sync::atomic::AtomicBool>,
    config: wgpu::SurfaceConfiguration,
    capabilities: wgpu::SurfaceCapabilities,
    hud: HudRenderer,
    uploads: upload::Uploads,
    panorama: panorama::Panorama,
    depth: wgpu::TextureView,
    /// Kept so the atlas and glint views stay alive for the HUD's bind group.
    _textures: [wgpu::Texture; 3],
}

impl BootRenderer {
    /// Opens the device for the window and builds the two pipelines the loading screen draws
    /// with. `atlas` is the boot screen's HUD atlas (the menu bundle's, once it has come);
    /// `faces` the title panorama, a flat colour until then.
    pub(crate) async fn new(
        window: Arc<Window>,
        atlas: (&[u8], u32, u32),
        faces: Arc<panorama::Faces>,
        api: crate::graphics::GraphicsApi,
    ) -> Result<Self> {
        let _step = crate::spans::startup::step("boot.open_device");
        // One sprite layer: the boot screen draws no blocks. `Renderer::new` checks the real
        // count again on this adapter when it takes the device over.
        let Opened {
            instance,
            surface,
            adapter,
            device,
            queue,
            device_lost,
            configured,
        } = super::open_device(&window, false, api, 1).await?;
        let (surface, (config, capabilities)) = match (surface, configured) {
            (Some(surface), Some(configured)) => (surface, configured),
            // Offscreen has no loading screen to draw; the caller builds the world's renderer.
            _ => return Err("the boot renderer needs a window surface".into()),
        };
        let _pipelines = crate::spans::startup::step("boot.pipelines");
        let (atlas_pixels, atlas_width, atlas_height) = atlas;
        let atlas_texture = texture(
            &device,
            &queue,
            "Boot HUD atlas",
            atlas_pixels,
            atlas_width,
            atlas_height,
        );
        // The boot screen draws nothing enchanted, so one transparent pixel stands in for both
        // glint textures; the HUD's bind group still wants them.
        static NO_GLINT: [u8; 4] = [0; 4];
        let glint_texture = texture(&device, &queue, "Boot glint", &NO_GLINT, 1, 1);
        let glint_armor_texture = texture(&device, &queue, "Boot armor glint", &NO_GLINT, 1, 1);
        let glint_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Boot glint"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mut hud = HudRenderer::new(
            &device,
            config.format,
            &atlas_texture.create_view(&Default::default()),
            &glint_texture.create_view(&Default::default()),
            &glint_armor_texture.create_view(&Default::default()),
            &glint_sampler,
            // The coverage layer is the world's; the loading screen draws straight to the window.
            false,
        )?;
        hud.atlas_texture = Some(atlas_texture.clone());
        let gl = adapter.get_info().backend == wgpu::Backend::Gl;
        let panorama = panorama::Panorama::new(&device, config.format, faces, gl);
        let depth = super::Renderer::depth(&device, &config);
        let uploads = upload::Uploads::new(&device);
        Ok(Self {
            instance,
            adapter,
            surface,
            device,
            queue,
            device_lost,
            config,
            capabilities,
            hud,
            uploads,
            panorama,
            depth,
            _textures: [atlas_texture, glint_texture, glint_armor_texture],
        })
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        let max = self.device.limits().max_texture_dimension_2d;
        let (width, height) = (width.clamp(1, max), height.clamp(1, max));
        if (width, height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth = super::Renderer::depth(&self.device, &self.config);
    }

    /// One frame of the loading screen: the panorama behind, the batch over it.
    pub(crate) fn render(&mut self, batch: &HudBatch, yaw: f32) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // A window being resized or not yet shown: the next frame draws it.
            _ => return Ok(()),
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.config.format),
            ..Default::default()
        });
        self.panorama.prepare(
            &self.device,
            &self.queue,
            self.config.width,
            self.config.height,
            yaw,
            10.0,
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Boot screen"),
            });
        self.hud.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            batch,
            [self.config.width, self.config.height],
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Boot panorama"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.panorama.draw(&mut pass);
        }
        self.hud.draw_range(
            &mut encoder,
            &view,
            None,
            &self.depth,
            batch,
            [self.config.width, self.config.height],
            0..batch.vertices.len() as u32,
            None,
        );
        self.uploads.finish(&encoder);
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(())
    }

    /// The device, queue and surface for `Renderer::new` to take over, so the world's renderer
    /// does not open a second one. The panorama's cube goes back to the driver first: the world's
    /// renderer makes its own.
    pub(crate) fn into_opened(mut self) -> Opened {
        self.panorama.release();
        Opened {
            instance: self.instance,
            surface: Some(self.surface),
            adapter: self.adapter,
            device: self.device,
            queue: self.queue,
            device_lost: self.device_lost,
            configured: Some((self.config, self.capabilities)),
        }
    }
}

fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> wgpu::Texture {
    device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        pixels,
    )
}
