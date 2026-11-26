//! Starling's part of surface_lifecycle.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Renderer {
    /// The window is going away (Android destroys it in the background): drop its surface now,
    /// while the window it was made from still exists, and draw nothing until `resume`. Not kept
    /// as the spare (`release_surface`): `resume` would take it back, on a window that is gone,
    /// and draw to nothing (a black screen on the way back from the browser or the file picker).
    pub fn suspend(&mut self) {
        self.surface = None;
        *self
            .spare_surface
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.suspended = true;
        self.detached = true;
    }

    /// The loading screen's surface (`BootRenderer::give_surface`) for a renderer made on its
    /// device while it drew. It goes where `resume` takes its surface from, so the window keeps the
    /// one surface it has: a second one on the same window fails on Android (the window is in use)
    /// and on Wayland.
    pub fn adopt_surface(
        &mut self,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<()> {
        *self
            .spare_surface
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(surface);
        self.resume(width, height)
    }

    /// The window is back, maybe at another size: a new surface on it.
    pub fn resume(&mut self, width: u32, height: u32) -> Result<()> {
        if !drawable_size(width, height, self.device.limits().max_texture_dimension_2d) {
            return Ok(());
        }
        let resized = [width, height] != [self.config.width, self.config.height];
        self.config.width = width;
        self.config.height = height;
        self.recreate_surface()?;
        if resized {
            self.depth = Self::depth(&self.device, &self.config);
        }
        self.suspended = false;
        self.detached = false;
        Ok(())
    }
}
