//! The loading screen's own renderer: the title panorama, the HUD batch, and nothing else.
//!
//! `Renderer::new` builds every pipeline the world will ever need before it returns, and the
//! loading screen used to wait for all of them: on a PowerVR B-Series that is 11 seconds before
//! anything reaches the screen, and the window stays on whatever was behind it. What the loading
//! screen actually draws is a cube and some text: `loading_frame` hands the renderer a `Frame`
//! with `title: true` and every other field empty.
//!
//! So this opens the device itself, keeps the two pipelines it draws with, and lends the open
//! device and queue to the world's renderer once the resources are ready (`lend`): that one is
//! built on another thread while this one goes on drawing the title, and takes the window's
//! surface at the handover (`give_surface`). The world's pipelines are then built once, instead
//! of once for the boot screen, again for the title artwork and again for the world, and none of
//! them on the thread the title answers on.

use std::sync::Arc;

use wgpu::util::DeviceExt as _;

use super::{Opened, panorama, upload};
use crate::Result;
use crate::hud::{HudBatch, HudRenderer};
use winit::window::Window;

/// What the loading screen draws with, and what it hands on afterwards.
pub struct BootRenderer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    window: Arc<Window>,
    /// None between `suspend` and `resume`: Android destroys the window in the background, and
    /// the surface has to go with it (`Renderer::suspend` says why it is not kept as a spare).
    surface: Option<wgpu::Surface<'static>>,
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
    textures: [wgpu::Texture; 3],
    /// The window is in the background, so no frame draws (`Renderer::detached`).
    pub detached: bool,
    /// `graphics::RENDERER_MARKER`, written before this opened the device: removed with the first
    /// frame presented, as `Renderer` does. This is the renderer that opens the device now, so a
    /// driver that dies opening it or in its first submit still sends the next start to OpenGL ES.
    start_marker: Option<std::path::PathBuf>,
}

/// What the boot screen draws with, made again when the menu bundle replaces the boot atlas.
struct Drawing {
    hud: HudRenderer,
    panorama: panorama::Panorama,
    textures: [wgpu::Texture; 3],
}

impl Drawing {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        gl: bool,
        atlas: (&[u8], u32, u32),
        faces: Arc<panorama::Faces>,
    ) -> Result<Self> {
        let (atlas_pixels, atlas_width, atlas_height) = atlas;
        let atlas_texture = texture(
            device,
            queue,
            "Boot HUD atlas",
            atlas_pixels,
            atlas_width,
            atlas_height,
        );
        // The loading screen draws nothing enchanted, so one transparent pixel stands in for
        // both glint textures; the HUD's bind group still wants them.
        static NO_GLINT: [u8; 4] = [0; 4];
        let glint_texture = texture(device, queue, "Boot glint", &NO_GLINT, 1, 1);
        let glint_armor_texture = texture(device, queue, "Boot armor glint", &NO_GLINT, 1, 1);
        let glint_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Boot glint"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mut hud = HudRenderer::new(
            device,
            format,
            &atlas_texture.create_view(&Default::default()),
            &glint_texture.create_view(&Default::default()),
            &glint_armor_texture.create_view(&Default::default()),
            &glint_sampler,
            // The coverage layer is the world's; the loading screen draws straight to the window.
            false,
        )?;
        hud.atlas_texture = Some(atlas_texture.clone());
        Ok(Self {
            hud,
            panorama: panorama::Panorama::new(device, format, faces, gl),
            textures: [atlas_texture, glint_texture, glint_armor_texture],
        })
    }
}

impl BootRenderer {
    /// Opens the device for the window and builds the two pipelines the loading screen draws
    /// with. `atlas` is the boot screen's HUD atlas (the menu bundle's, once it has come);
    /// `faces` the title panorama, a flat colour until then.
    pub async fn new(
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
        let gl = adapter.get_info().backend == wgpu::Backend::Gl;
        let Drawing {
            hud,
            panorama,
            textures,
        } = Drawing::new(&device, &queue, config.format, gl, atlas, faces)?;
        let depth = super::Renderer::depth(&device, &config);
        let uploads = upload::Uploads::new(&device);
        Ok(Self {
            instance,
            adapter,
            window,
            surface: Some(surface),
            device,
            queue,
            device_lost,
            config,
            capabilities,
            hud,
            uploads,
            panorama,
            depth,
            textures,
            detached: false,
            start_marker: None,
        })
    }

    /// Removes `marker` (`graphics::RENDERER_MARKER`) once a frame has been presented, as
    /// `Renderer::forget_start_marker`: the start went through.
    pub fn forget_start_marker(&mut self, marker: std::path::PathBuf) {
        self.start_marker = Some(marker);
    }

    /// The window is going away (Android destroys it in the background): drop its surface while
    /// the window it was made from still exists, exactly as `Renderer::suspend` does.
    pub fn suspend(&mut self) {
        self.surface = None;
        self.detached = true;
    }

    /// The window is back, maybe at another size: a new surface on it.
    pub fn resume(&mut self, width: u32, height: u32) -> Result<()> {
        let max = self.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > max || height > max {
            return Ok(());
        }
        let resized = [width, height] != [self.config.width, self.config.height];
        self.config.width = width;
        self.config.height = height;
        let surface = self.instance.create_surface(self.window.clone())?;
        surface.configure(&self.device, &self.config);
        self.surface = Some(surface);
        if resized {
            self.depth = super::Renderer::depth(&self.device, &self.config);
        }
        self.detached = false;
        Ok(())
    }

    /// The menu bundle has come: its atlas and title panorama replace the boot screen's. The
    /// world's renderer was built again here, for an atlas and a cube â€” two pipelines on a device
    /// that is already open, which is what this costs instead.
    pub fn set_ui(&mut self, atlas: (&[u8], u32, u32), faces: Arc<panorama::Faces>) -> Result<()> {
        let _step = crate::spans::startup::step("boot.set_ui");
        let gl = self.adapter.get_info().backend == wgpu::Backend::Gl;
        let Drawing {
            hud,
            panorama,
            textures,
        } = Drawing::new(
            &self.device,
            &self.queue,
            self.config.format,
            gl,
            atlas,
            faces,
        )?;
        self.hud = hud;
        self.panorama = panorama;
        self.textures = textures;
        Ok(())
    }

    /// The window the loading screen lays itself out for.
    pub fn size(&self) -> [u32; 2] {
        [self.config.width, self.config.height]
    }

    /// The adapter this opened, for the diagnostics the loading screen reports.
    pub fn info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let max = self.device.limits().max_texture_dimension_2d;
        let (width, height) = (width.clamp(1, max), height.clamp(1, max));
        if (width, height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
        self.depth = super::Renderer::depth(&self.device, &self.config);
    }

    /// One frame of the loading screen: the panorama behind, the batch over it.
    pub fn render(&mut self, batch: &HudBatch, yaw: f32) -> Result<()> {
        // The borrow of the surface ends with the acquisition, so the arm below may resize.
        let frame = match self
            .surface
            .as_ref()
            .map(wgpu::Surface::get_current_texture)
        {
            // Suspended: there is no window to draw on until `resume`.
            None => return Ok(()),
            Some(
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame),
            ) => frame,
            // The window was resized, or its swapchain went out of date: configure the surface
            // for the size the window has now and let the next frame draw it. The loading screen
            // has no resize event of its own, so this is where a resize is noticed.
            Some(_) => {
                let size = self.window.inner_size();
                self.resize(size.width, size.height);
                // `resize` returns early when the size has not changed, but a swapchain that is
                // out of date wants the surface configured again either way.
                if let Some(surface) = &self.surface {
                    surface.configure(&self.device, &self.config);
                }
                return Ok(());
            }
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
                // The panorama's pipeline is the world pass's and wants its depth attachment
                // (Depth32Float), cleared as that pass clears it: 0.0, the far side of a reversed
                // depth. Without one, wgpu refuses the draw as a validation error, which ends the
                // process. The HUD clears its own depth after, so nothing here needs keeping.
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
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
        if let Some(marker) = self.start_marker.take() {
            let _ = std::fs::remove_file(marker);
        }
        Ok(())
    }

    /// The device and the window's format, for the world's renderer to be made on while this one
    /// goes on drawing (`Renderer::new`, on another thread). The surface stays here until the
    /// handover (`give_surface`): a window has one, and the loading screen draws on it meanwhile.
    pub fn lend(&self) -> Lent {
        Lent(Opened {
            instance: self.instance.clone(),
            surface: None,
            adapter: self.adapter.clone(),
            device: self.device.clone(),
            queue: self.queue.clone(),
            device_lost: self.device_lost.clone(),
            configured: Some((self.config.clone(), self.capabilities.clone())),
        })
    }

    /// The window's surface for the world's renderer (`Renderer::adopt_surface`), the loading
    /// screen's last frame still on it. None while the window is in the background: the world's
    /// renderer then makes one when it comes back, as after any `suspend`.
    pub fn give_surface(self) -> Option<wgpu::Surface<'static>> {
        self.surface
    }
}

/// The boot renderer's device lent to the world's renderer (`BootRenderer::lend`).
pub struct Lent(Opened);

impl Lent {
    pub(crate) fn into_opened(self) -> Opened {
        self.0
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
