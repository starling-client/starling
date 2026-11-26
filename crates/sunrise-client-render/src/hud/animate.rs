//! The GUI atlas's animated sprites, frame by frame (vanilla's or a Polytone `mode`'s).
use std::borrow::Cow;

/// Compute the atlas upload rectangle `(x, y, width, height)` and pixel payload for an animated
/// sprite using its assigned placement and border policy from atlas packing.
pub(crate) fn animation_upload_region(
    placement: super::AnimatedPlacement,
    pixels: &[u8],
) -> (u32, u32, u32, u32, Cow<'_, [u8]>) {
    match placement.border {
        super::AtlasBorder::None => (
            placement.x,
            placement.y,
            placement.w,
            placement.h,
            Cow::Borrowed(pixels),
        ),
        super::AtlasBorder::Clamp => (
            placement.x - 1,
            placement.y - 1,
            placement.w + 2,
            placement.h + 2,
            Cow::Owned(super::clamp_border_pixels(pixels, placement.w, placement.h)),
        ),
        super::AtlasBorder::Wrap => (
            placement.x - 1,
            placement.y - 1,
            placement.w + 2,
            placement.h + 2,
            Cow::Owned(super::wrap_border_pixels(pixels, placement.w, placement.h)),
        ),
    }
}

impl super::HudRenderer {
    /// Shows each animated sprite's frame: its mcmeta's, or what its Polytone `mode` picks.
    pub(crate) fn animate(
        &mut self,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        tick: u64,
        animator: &crate::gfx::animate::SpriteAnimator,
    ) {
        let inputs = animator.mode_inputs();
        let inputs = Some(&inputs);
        for (texture, animations) in self
            .atlas_texture
            .as_ref()
            .zip(Some(&mut self.sprite_animations))
            .into_iter()
            .chain(
                self.atlas_pages
                    .iter_mut()
                    .flatten()
                    .map(|p| (&p.page.texture, &mut p.page.animations)),
            )
        {
            for (placement, animation, previous) in animations {
                // `SpriteContents.AnimationState`: the frame from the mcmeta's
                // list, blended into the next one by the sub-frame's thousandths
                // when the sprite interpolates; or what a Polytone `mode` picks.
                let blend = animation.blend_with(tick, inputs);
                let state = (blend.frame, blend.next, blend.millis);
                if state == *previous {
                    continue;
                }
                let (w, h) = (placement.w, placement.h);
                let Ok(mut pixels) = animation.resized_pixels(blend.frame, w, h) else {
                    continue;
                };
                if blend.millis != 0 {
                    let Ok(next) = animation.resized_pixels(blend.next, w, h) else {
                        continue;
                    };
                    let millis = blend.millis;
                    for (a, b) in pixels.iter_mut().zip(next) {
                        *a = ((u32::from(*a) * (1000 - millis) + u32::from(b) * millis + 500)
                            / 1000) as u8;
                    }
                }
                let (origin_x, origin_y, up_w, up_h, upload_bytes) =
                    animation_upload_region(*placement, &pixels);
                uploads.texture(
                    encoder,
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: origin_x,
                            y: origin_y,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    up_w,
                    up_h,
                    &upload_bytes,
                );
                *previous = state;
            }
        }
    }
}
