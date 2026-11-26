use super::*;

/// Post-HUD hook (`RenderGuiEvent.Post`) and drawn HUD layer bounds queries.
impl Game<'_> {
    /// After the complete in-world HUD layer walk (including under F1) and before screens,
    /// toasts, debug overlays and IME draw (`RenderGuiEvent.Post`).
    pub(in crate::game) fn native_render_gui_post(&self, draw: &mut HudBatch) {
        self.fire_render_gui_post(draw);
    }

    /// Bounding rectangle `[x, y, width, height]` in GUI pixels of a HUD layer drawn
    /// during the current HUD pass (`ScreensClientHost::hud_layer_bounds`).
    pub(in crate::game) fn hud_layer_bounds(
        &self,
        layer: &str,
    ) -> Result<Option<[f32; 4]>, mod_api::LevelReadError> {
        if !self.native_window() || self.lan_acting.is_some() {
            return Err(mod_api::LevelReadError::Unavailable);
        }
        Ok(self
            .hud_layer_bounds
            .iter()
            .rev()
            .find_map(|&(id, rect)| (id == layer).then_some(rect)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn panicking_hud_draw_discards_quads_and_reservations_before_next_listener() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
        use mod_api::{ClientHost, Hooks, ModEntry, RenderGuiPostEvent};

        const NAME: &str = "game::native_hooks::render_gui_post::tests::panicking_hud_draw_discards_quads_and_reservations_before_next_listener";
        if run_fresh_cases(NAME, "SUNRISE_GUI_POST_PANIC", &["run"]) {
            return;
        }

        struct Draws(bool);
        #[mod_api::__subscriptions]
        impl Hooks for Draws {
            fn render_gui_post(&self, _: &dyn ClientHost, event: &mut RenderGuiPostEvent<'_>) {
                let (x, y) = (2.0, 2.0);
                if self.0 {
                    event
                        .draw
                        .with_scissor([x, y, 5.0, 5.0], &mut |draw| {
                            draw.fill([x, y, 12.0, 12.0], 0xffff_0000);
                            panic!("discard my drawing");
                        })
                        .unwrap();
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
            with_client_game(|game| {
                let mut batch = HudBatch::scratch([640, 480]);
                game.native_render_gui_post(&mut batch);
                assert_eq!(batch.vertices.len(), 6);
                assert!(batch.vertices.iter().all(|v| v.color == 0xff00_ff00));
                assert_eq!(batch.vertices[0].pos, [2.0, 2.0]);
                assert!(batch.vertices.iter().any(|v| v.pos == [14.0, 14.0]));
                assert_eq!(mod_api::__private::take_hook_panics().len(), 1);
            });
        });
    }
}
