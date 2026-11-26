//! Starling's part of overlay.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Renderer {
    /// A frame of a paused menu from the blurred frame `MenuBlur::keep` kept: that frame, then
    /// the menu over it, with the section copies and HUD uploads of the frame, submitted and
    /// presented as `draw_frame` presents a plain one. The world and the blur are not drawn.
    /// Adapted from Vasysik/sunrise-android (the cached path of its `Renderer::render`, commit
    /// 77b9e17f76), MIT, copyright (c) 2026 Ludovic Marie and the Horizon contributors.
    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn draw_kept_menu(
        &mut self,
        frame: &Frame,
        mut encoder: wgpu::CommandEncoder,
        has_section_copies: bool,
        target_texture: &wgpu::Texture,
        surface_frame: Option<wgpu::SurfaceTexture>,
        reconfigure: bool,
        started: Instant,
    ) -> Result<bool> {
        let size = [self.config.width, self.config.height];
        self.hud.prepare(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            frame.hud,
            size,
        );
        // `has_menu_blur_cache` holds only where the HUD draws in the frame's own format.
        let view_target = target_texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.output.frame.format),
            ..Default::default()
        });
        if let Some(blur) = &self.menu_blur {
            blur.draw_kept(&self.device, &self.queue, &mut encoder, &view_target);
        }
        self.hud.draw_range(
            &mut encoder,
            &view_target,
            None,
            &self.depth,
            frame.hud,
            size,
            frame.hud.blur_at..frame.hud.vertices.len() as u32,
            None,
        );
        self.uploads.finish(&encoder);
        if has_section_copies {
            self.section_copy_bytes = 0;
            self.section_uploads.finish(&encoder);
        }
        if let Some(image) = surface_frame
            .as_ref()
            .map(|f| &f.texture)
            .or(self.offscreen.as_ref())
        {
            self.readback.present(&self.device, &mut encoder, image);
        }
        let commands = crate::spans::time("encode", || encoder.finish());
        let submission = crate::spans::time("submit", || self.queue.submit([commands]));
        self.last_wait += self.queue.take_waited();
        let present = Instant::now();
        if let Some(surface_frame) = surface_frame {
            if self.config.present_mode == wgpu::PresentMode::Fifo {
                self.window.pre_present_notify();
            }
            self.queue.present(surface_frame);
            if reconfigure {
                self.configure_surface();
            }
        }
        if self.surface.is_none() {
            self.in_flight.push_back(submission);
            if !self.paced_elsewhere {
                self.wait_in_flight();
            }
        }
        self.last_wait += present.elapsed();
        self.queue.take_waited();
        self.last_encode = started.elapsed().saturating_sub(self.last_wait);
        Ok(true)
    }
}
