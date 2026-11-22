use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// After the whole HUD walk, including under F1 (`RenderGuiEvent.Post`). Place a box with
    /// `corners.place` (boxes stack away from their edge) and
    /// draw it with `native_fill` and `native_text`. Skip a corner
    /// `native_corner_shown` says is taken.
    pub(in crate::game) fn native_render_gui_post(
        &self,
        draw: &mut HudBatch,
        corners: &mut Corners,
    ) {
        self.fire_render_gui_post(draw, corners);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn panicking_hud_draw_discards_quads_and_reservations_before_next_listener() {
        use mod_api::{ClientHost, Corner, Hooks, ModEntry, RenderGuiPostEvent};

        struct Draws(bool);
        #[mod_api::__subscriptions]
        impl Hooks for Draws {
            fn render_gui_post(&self, _: &dyn ClientHost, event: &mut RenderGuiPostEvent<'_>) {
                let (x, y) = event.corners.place(Corner::TopLeft, 12.0, 12.0);
                if self.0 {
                    event.draw.push_clip([x, y, 5.0, 5.0]).unwrap();
                    event.draw.fill([x, y, 12.0, 12.0], 0xffff_0000);
                    panic!("discard my drawing");
                }
                event.draw.fill([x, y, 12.0, 12.0], 0xff00_ff00);
            }
        }

        let mods = vec![
            ModEntry::new("failed_paint", Box::new(Draws(true))),
            ModEntry::new("healthy_paint", Box::new(Draws(false))),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                let mut batch = HudBatch::scratch([640, 480]);
                let mut corners = Corners::new(batch.width, batch.height, 0.0);
                game.native_render_gui_post(&mut batch, &mut corners);
                assert_eq!(batch.vertices.len(), 6);
                assert!(batch.vertices.iter().all(|v| v.color == 0xff00_ff00));
                assert_eq!(batch.vertices[0].pos, [2.0, 2.0]);
                assert_eq!(mod_api::__private::take_hook_panics().len(), 1);
            });
        });
    }
}
