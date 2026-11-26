//! Starling's part of world_post.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Whether a vanilla world drawn below the window's size goes up with the sharp filter. It takes five
/// filtered samples for every pixel of the window, which on Mali costs about what the lower resolution
/// saved: at 50% a Mali-G68 went from 70-80 fps with the copy to 55 with it. There, and on PowerVR, the
/// copy's one sample stays. `SUNRISE_WORLD_UPSCALE`: `on` and `off` choose on any GPU.
pub(super) fn upscales_sharply(gpu: &str) -> bool {
    match std::env::var("SUNRISE_WORLD_UPSCALE").as_deref() {
        Ok("on") => true,
        Ok("off") => false,
        _ => !gpu.contains("Mali") && !gpu.contains("PowerVR"),
    }
}

impl Post {
    /// A shader pack's `final` goes into the world's target, to go up with [`Self::finish_pack`].
    pub fn upscales_pack(&self) -> bool {
        self.sharp.is_some() && self.scale < 1.0
    }

    /// Draw a shader pack's `final`, made in the world's target, onto `screen`.
    pub fn finish_pack(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        screen: &wgpu::TextureView,
        timing: Option<&mut super::super::pack_timing::PassTiming>,
    ) {
        if let (Some(target), Some(sharp)) = (&self.target, &self.sharp) {
            self.draw_onto(encoder, screen, &target.group, sharp, timing);
        }
    }
}
