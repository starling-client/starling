//! Starling's part of culling_control.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Renderer {
    /// Whether frames that may take it draw cutouts without their fade (`solid_paths`).
    pub fn static_cutout_on(&self) -> bool {
        self.solid_paths
            .as_ref()
            .is_some_and(|p| p.static_cutout_on)
    }

    /// Switches drawing cutouts without their fade, for A/B checks.
    pub fn set_static_cutout(&mut self, on: bool) -> Result<()> {
        match &mut self.solid_paths {
            Some(paths) => paths.static_cutout_on = on,
            None if on => return Err("the cheaper terrain shaders are off on this device".into()),
            None => {}
        }
        if let Some(paths) = &mut self.solid_paths_sdr {
            paths.static_cutout_on = on;
        }
        Ok(())
    }

    /// Whether frames that may take it draw whole sections through less work (`solid_paths`).
    pub fn settled_terrain_on(&self) -> bool {
        self.settled_single_on || self.solid_paths.as_ref().is_some_and(|p| p.settled_on)
    }

    /// Switches drawing whole sections through less work, for A/B checks.
    pub fn set_settled_terrain(&mut self, on: bool) -> Result<()> {
        if self.solid_settled_single.is_some() {
            self.settled_single_on = on;
            return Ok(());
        }
        match &mut self.solid_paths {
            Some(paths) => paths.settled_on = on,
            None if on => return Err("the cheaper terrain shaders are off on this device".into()),
            None => {}
        }
        if let Some(paths) = &mut self.solid_paths_sdr {
            paths.settled_on = on;
        }
        Ok(())
    }

    /// Drops the blurred menu frame kept for `Frame::menu_blur_cache`: the next blurred
    /// frame draws the world again (the menu was left, or a setting changed under it).
    pub fn forget_menu_blur(&mut self) {
        if let Some(blur) = &mut self.menu_blur {
            blur.forget();
        }
    }

    /// Whether a frame of a menu blurred by `radius` with `Frame::menu_blur_cache` draws the
    /// kept blurred frame, without the world: one is kept for this window's size and radius,
    /// and no screenshot, HDR output or SDR finish of a 0.2 pack (`overlay_format`) needs the
    /// whole frame. Adapted from Vasysik/sunrise-android (`has_cached_menu_blur`, its commit
    /// 77b9e17f76), MIT.
    pub fn has_menu_blur_cache(&self, radius: f32) -> bool {
        self.config.usage.contains(wgpu::TextureUsages::COPY_SRC)
            && self.screenshot.is_none()
            && self.hdr.is_none()
            && self.output.frame.overlay_format == self.output.frame.format
            && self
                .menu_blur
                .as_ref()
                .is_some_and(|b| b.kept([self.config.width, self.config.height], radius))
    }

    /// Whether per-section draws are indirect multi-draws rather than one call per section.
    pub fn indirect_draws(&self) -> bool {
        self.multi_draw
    }

    /// Switches per-section draws between indirect multi-draws, where the device takes them,
    /// and one call per section, for A/B checks.
    pub fn set_indirect_draws(&mut self, on: bool) {
        self.multi_draw = on && self.indirect_capable;
    }

    /// Whether frames that may take it draw the sky after solid terrain (`late_sky`).
    pub fn late_sky_on(&self) -> bool {
        self.late_sky_on
    }

    /// Switches drawing the sky after solid terrain, for A/B checks.
    pub fn set_late_sky(&mut self, on: bool) -> Result<()> {
        if on && self.late_sky.is_none() {
            return Err("the late sky is off on this device".into());
        }
        self.late_sky_on = on;
        Ok(())
    }

    /// Which A/B switches this device takes: flat draws, indirect multi-draws, the late sky
    /// and occlusion culling (Starling's performance test offers only these).
    pub fn draw_choices(&self) -> [bool; 4] {
        [
            self.flat.is_some(),
            self.indirect_capable,
            self.late_sky.is_some(),
            self.cull.is_some(),
        ]
    }
}
