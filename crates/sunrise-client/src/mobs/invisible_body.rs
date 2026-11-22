//! `LivingEntityRenderer.submit` for an invisible body its viewer still sees.
use super::{
    Atlas, FrameDraw, Mob, Part, RenderDataSource, Species, base_transform, body_color,
    render_visible, render_with_frame,
};
use crate::mesher::Vertex;

/// `Entity.isInvisibleTo` is false for a spectator, and for a teammate of a
/// team that sees friendly invisibles, so the body it cannot show as visible
/// draws `forceTransparent`: `RenderTypes.entityTranslucentCull` at `baseColor`
/// 654311423, `ARGB.multiply`ed with `getModelTint` (`body_color`), and its
/// layers draw as they would for a visible body.
#[allow(clippy::too_many_arguments)]
pub fn render_seen_invisible(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &dyn RenderDataSource,
    extra: &[(&Part, [f32; 4], u32, u32, bool)],
    frame: &mut FrameDraw<'_>,
    out: &mut Vec<Vertex>,
) {
    // `IllusionerRenderer.isBodyVisible` is true always: an invisible illusioner
    // draws its mirrors in the normal render type, not a translucent body.
    if m.species() == Species::Illusioner {
        render_with_frame(m, alpha, camera, atlas, light, data, extra, frame, out);
        return;
    }
    let tint = (body_color(m, alpha) & 0x00FF_FFFF) | 0x2600_0000;
    render_visible(
        m,
        alpha,
        [base_transform(m, alpha, camera)],
        atlas,
        light,
        data,
        extra,
        Some(tint),
        frame,
        out,
        None,
    );
}
