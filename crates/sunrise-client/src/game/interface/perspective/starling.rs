//! Starling's part of perspective.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    pub(super) fn player_preview(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        size: f32,
        offset_y: f32,
        cursor: (f32, f32),
        spin: f32,
    ) {
        if let Some(m) = self.player_model.as_ref() {
            let data = self.res.mob_data();
            self.mob_preview(m, data, batch, rect, size, offset_y, cursor, spin, true);
        }
    }

    /// `player_ears_geometry` with the data source the model's animations come from: the title
    /// screen's player has its own (`mob_preview`), before the world's mob data.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn player_ears_geometry_from(
        &self,
        m: &Mob,
        name: &str,
        camera: glam::DVec3,
        partial: f32,
        (data, light): (&dyn sunrise_client::mobs::render::RenderDataSource, u32),
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        if name != "deadmau5" {
            return;
        }
        let sprites = &self.gui.atlas.sprites;
        // The body's own fallback: a skin with no sprite draws the default.
        let Some(rect) = m
            .skin
            .as_deref()
            .and_then(|s| sprites.get(s))
            .or_else(|| sprites.get(&m.kind.texture))
        else {
            return;
        };
        sunrise_client::mobs::render::ears(
            m,
            partial,
            sunrise_client::mobs::render::base_transform(m, partial, camera),
            [rect.u0, rect.v0, rect.u1, rect.v1],
            light,
            data,
            out,
        );
    }
}
