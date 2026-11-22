//! Client draw operations through the existing GUI drawing context.

use super::Game;
use mod_api::{ButtonSprites, TextFieldLayout, TextFieldView};
use sunrise_client::{
    hud::{HudBatch, rgba},
    text::Text,
};
use sunrise_items::items::ItemStack as EngineStack;

impl Game<'_> {
    /// `GuiGraphics.fillGradient` for [`mod_api::DrawContext::fill_gradient`]: `rect` is
    /// `[x, y, w, h]` and `top` / `bottom` are ARGB at `y` / `y + h`. Zero area,
    /// one-axis inversion or non-finite input draws nothing; both axes inverted flips the colours.
    pub(super) fn native_fill_gradient(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        top: u32,
        bottom: u32,
    ) {
        let [x, y, w, h] = rect;
        if !rect.iter().all(|v| v.is_finite()) || w == 0.0 || h == 0.0 || (w > 0.0) != (h > 0.0) {
            return;
        }
        let ((x, y, w, h), top, bottom) = if w < 0.0 {
            ((x + w, y + h, -w, -h), bottom, top)
        } else {
            ((x, y, w, h), top, bottom)
        };
        let abgr = sunrise_client::hud_layout::abgr;
        batch.fill_gradient(&self.gui.atlas, (x, y, w, h), abgr(top), abgr(bottom));
    }
    /// Vanilla's nine-slice widget rendering. `color` is ARGB, as `fill` uses.
    pub(super) fn native_gui_nine_slice(
        &self,
        batch: &mut HudBatch,
        sprite: &str,
        rect: [f32; 4],
        color: u32,
    ) {
        let [a, r, g, b] = color.to_be_bytes();
        sunrise_client::ui::nine_slice(
            batch,
            &self.gui.atlas,
            sprite,
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            rgba(r, g, b, a),
        );
    }
    /// A mod's button, using the same drawing and registration path as the game's screens.
    pub(super) fn native_widget_button(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        label: &Text,
        active: bool,
        highlighted: bool,
        sprites: Option<&ButtonSprites>,
    ) {
        sunrise_client::ui::button_as(
            batch,
            &self.gui.atlas,
            rect,
            label,
            active,
            highlighted,
            sprites,
            |rect| sunrise_client::widgets::Narratable::button(label.plain(), rect, active),
        );
    }
    /// A mod's text field, drawn as the game's screens draw theirs (`ui::EditBox`), from the
    /// state the mod keeps; answers the scroll it drew with and its character boundaries.
    pub(super) fn native_widget_text_field(
        &self,
        batch: &mut HudBatch,
        field: &TextFieldView,
    ) -> TextFieldLayout {
        let [x, y, w, h] = field.rect;
        let mut edit = sunrise_client::ui::EditBox::new(x, y, w, h);
        edit.max_length = field.max_length;
        edit.value = field.value.clone();
        let len = sunrise_client::ui::char_len(&edit.value);
        edit.cursor = field.cursor.min(len);
        edit.highlight = field.highlight.min(len);
        edit.set_scroll(field.scroll);
        edit.bordered = field.bordered;
        edit.focused = field.focused;
        let [a, r, g, b] = field.text_color.to_be_bytes();
        edit.text_color = rgba(r, g, b, a);
        edit.suggestion = field.suggestion.clone();
        edit.message = field.message.clone();
        edit.draw_with_caret(batch, &self.gui.atlas, None, field.caret_shown);
        let (scroll, edges) = edit.shown_edges(&self.gui.atlas);
        TextFieldLayout::new(scroll, edges)
    }
    /// A vanilla tooltip at the cursor, in GUI pixels.
    pub(super) fn native_gui_tooltip(&self, batch: &mut HudBatch, lines: &[Text], at: [f32; 2]) {
        sunrise_client::ui::tooltip(batch, &self.gui.atlas, lines, (at[0], at[1]));
    }

    /// Draw an item stack with custom count/decoration text (or None) at `at` in GUI units.
    pub(super) fn native_gui_draw_item_with(
        &self,
        batch: &mut HudBatch,
        stack: &EngineStack,
        at: [f32; 2],
        text: Option<&str>,
    ) {
        self.draw_item_model(batch, stack, at[0], at[1]);
        self.draw_item_decorations(batch, stack, at[0], at[1], text.map(str::to_owned));
    }
    /// Whether a specific key is currently held down.
    /// `key` is the vanilla physical key or mouse button name (`key.keyboard.left.shift`,
    /// `key.mouse.left`, for example). Unknown names are not down. Modifier combinations use
    /// `ClientHost::modifiers`.
    pub(in crate::game) fn is_key_down(&self, key: &str) -> bool {
        match sunrise_client::keybinds::key_code(key) {
            Some(key) => self.physical_keys.contains(&key),
            None => sunrise_client::keybinds::mouse_button(key)
                .is_some_and(|button| self.physical_buttons.contains(&button)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn tooltip_helpers_read_live_keys_and_draw_count_overrides() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            use winit::keyboard::KeyCode;

            game.physical_keys.insert(KeyCode::ShiftLeft);
            assert!(game.is_key_down("key.keyboard.left.shift"));
            assert!(!game.is_key_down("key.keyboard.right.shift"));
            assert!(!game.is_key_down("not-a-key"));
            game.physical_keys.clear();
            assert!(!game.is_key_down("key.keyboard.left.shift"));

            let stack = EngineStack::new("minecraft:diamond", 5);
            let mut automatic = HudBatch::scratch([320, 240]);
            game.native_gui_draw_item_with(&mut automatic, &stack, [16.0, 32.0], None);
            let mut custom = HudBatch::scratch([320, 240]);
            game.native_gui_draw_item_with(&mut custom, &stack, [16.0, 32.0], Some("7"));
            assert!(!automatic.vertices.is_empty());
            assert_ne!(
                bytemuck::cast_slice::<_, u8>(&automatic.vertices),
                bytemuck::cast_slice::<_, u8>(&custom.vertices),
                "custom count text must reach the actual GUI geometry"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn fifty_sprite_buttons_keep_their_pixels_and_accessibility() {
        use mod_api::client::widgets::{Button, Outcome, Widgets};
        use mod_api::{Hooks, ModEntry};
        use sunrise_client::{
            gfx::entity_pixel_test::render_hud,
            hud::{Rect, dynamic_textures::ImagePatch},
            widgets::{self, ScreenInfo, What},
        };

        const SIZE: [u32; 2] = [320, 240];
        const COUNT: usize = 50;
        const BUTTON_SIZE: [u32; 2] = [14, 15];
        let position = |i: usize| [(i as u32 % 10) * 20 + 10, (i as u32 / 10) * 20 + 10];
        let color = |i: usize, state: u32| [64 + i as u8, 16 + state as u8 * 80, 230, 255];
        let sprite_id = |i, state| format!("test-buttons:gui/sprites/{i}/{state}");

        struct ClientMod;
        #[mod_api::__subscriptions]
        impl Hooks for ClientMod {}
        mod_platform::testing::with_mods(
            vec![ModEntry::new("test-buttons", Box::new(ClientMod))].into(),
            || {
                super::super::native_entity_refs::tests::with_client_game(|game| {
                    // Put a distinct solid texel for every sprite in one reserved atlas slot.
                    let atlas = &mut game.gui.atlas;
                    let [ox, oy] = atlas.dynamic_texture_pool.as_ref().unwrap().origin(0, 0);
                    let (aw, ah) = (atlas.width as f32, atlas.height as f32);
                    let mut palette = Vec::new();
                    for state in 0..3 {
                        for i in 0..COUNT {
                            let (x, y) = (ox + i as u32, oy + state);
                            palette.extend(color(i, state));
                            atlas.sprites.insert(
                                sprite_id(i, state),
                                Rect {
                                    u0: x as f32 / aw,
                                    v0: y as f32 / ah,
                                    u1: (x + 1) as f32 / aw,
                                    v1: (y + 1) as f32 / ah,
                                    w: 1,
                                    h: 1,
                                },
                            );
                        }
                    }
                    let patch = ImagePatch {
                        origin: [ox, oy],
                        size: [COUNT as u32, 3],
                        rgba: palette.into(),
                    };

                    let mut widgets = Widgets::new();
                    let ids = std::array::from_fn::<_, COUNT, _>(|i| {
                        let [x, y] = position(i);
                        widgets.add_button(Button::new(
                            [
                                x as f32,
                                y as f32,
                                BUTTON_SIZE[0] as f32,
                                BUTTON_SIZE[1] as f32,
                            ],
                            mod_api::Text::literal("iii"),
                        ))
                    });
                    let frame = |group: &mut Widgets, mouse| {
                        let mut batch = HudBatch::scratch(SIZE);
                        batch.image_patches.push(patch.clone());
                        widgets::begin_screen(|| ScreenInfo::new("test-buttons", "Sprite buttons"));
                        let now = std::time::Instant::now();
                        group
                            .draw(
                                &mut crate::game::host::draw::GameDrawContext::new(
                                    game, &mut batch,
                                ),
                                game,
                                mouse,
                            )
                            .unwrap();
                        eprintln!("50 buttons, CPU draw-list diagnostic: {:?}", now.elapsed());
                        render_hud(&batch, &game.gui.atlas, SIZE)
                            .expect("software Vulkan button draw")
                    };
                    let pixel = |pixels: &[u8], x, y| {
                        let start = ((y * SIZE[0] + x) * 4) as usize;
                        <[u8; 4]>::try_from(&pixels[start..start + 4]).unwrap()
                    };
                    let vanilla = frame(&mut widgets, [0.0, 0.0]);
                    widgets::end_frame(1);
                    for (i, id) in ids.into_iter().enumerate() {
                        let mut art = ButtonSprites::new(sprite_id(i, 0));
                        art.highlighted = Some(sprite_id(i, 1));
                        art.disabled = Some(sprite_id(i, 2));
                        widgets.button_mut(id).unwrap().sprites = Some(art);
                    }
                    let custom = frame(&mut widgets, [0.0, 0.0]);
                    widgets::end_frame(2);
                    let button_at = |x, y| {
                        (0..COUNT).find(|&i| {
                            let [left, top] = position(i);
                            (left..left + BUTTON_SIZE[0]).contains(&x)
                                && (top..top + BUTTON_SIZE[1]).contains(&y)
                        })
                    };
                    for i in 0..COUNT {
                        let [x, y] = position(i);
                        assert_eq!(pixel(&custom, x + 1, y + 1), color(i, 0));
                        assert_ne!(pixel(&vanilla, x + 1, y + 1), color(i, 0));
                    }
                    for y in 0..SIZE[1] {
                        for x in 0..SIZE[0] {
                            if button_at(x, y).is_none() {
                                assert_eq!(
                                    pixel(&custom, x, y),
                                    pixel(&vanilla, x, y),
                                    "outside buttons at {x},{y}"
                                );
                            }
                        }
                    }

                    widgets.button_mut(ids[1]).unwrap().active = false;
                    let hovered = frame(&mut widgets, [11.0, 11.0]);
                    widgets::end_frame(3);
                    assert_eq!(pixel(&hovered, 11, 11), color(0, 1));

                    let inactive_pointer = [31.0, 11.0];
                    let disabled = frame(&mut widgets, inactive_pointer);
                    widgets::end_frame(4);
                    assert_eq!(pixel(&disabled, 31, 11), color(1, 2));
                    let recorded = widgets::last();
                    assert_eq!(recorded.len(), COUNT);
                    assert_eq!(
                        recorded.iter().filter(|button| button.enabled).count(),
                        COUNT - 1
                    );
                    let inactive = recorded
                        .iter()
                        .find(|b| {
                            (b.x..b.x + b.w).contains(&inactive_pointer[0])
                                && (b.y..b.y + b.h).contains(&inactive_pointer[1])
                        })
                        .expect("the inactive button under the pointer is registered");
                    assert!(!inactive.enabled);
                    assert_eq!(
                        [inactive.x, inactive.y, inactive.w, inactive.h],
                        [30.0, 10.0, 14.0, 15.0]
                    );
                    assert_eq!(
                        widgets.mouse_pressed(game, inactive_pointer, 0, 0),
                        Outcome::Ignored
                    );
                    assert_eq!(
                        widgets.mouse_pressed(game, [11.0, 11.0], 0, 0),
                        Outcome::Pressed(ids[0])
                    );
                    widgets.key_pressed(game, "key.keyboard.tab", 0);
                    assert_eq!(widgets.focused(), Some(ids[2]));
                    widgets::set_narrating(true);
                    let focused = frame(&mut widgets, [0.0, 0.0]);
                    widgets::end_frame(5);
                    widgets::set_narrating(false);
                    assert_eq!(pixel(&focused, 51, 11), color(2, 1));
                    let narration = widgets::take_last_narration();
                    assert_eq!(narration.items.len(), COUNT);
                    assert!(
                        matches!(&narration.items[2].what, What::Button { message, active: true } if message == "iii")
                    );
                    assert_eq!(narration.items[2].hovered, Some(true));
                    assert_eq!(narration.items[2].rect, [50.0, 10.0, 14.0, 15.0]);
                    for key in ["key.keyboard.enter", "key.keyboard.space"] {
                        assert_eq!(widgets.key_pressed(game, key, 0), Outcome::Pressed(ids[2]));
                    }
                })
            },
        );
    }

    /// A mod's widgets draw with the game's own: a button as `ui::button` draws it, and a text
    /// field as `ui::EditBox` draws it, answering where each shown character ends so a press
    /// lands on it, and scrolling on to keep the cursor in view.
    #[test]
    #[ignore = "daily"]
    fn mod_widgets_draw_as_the_games_own() {
        use mod_api::client::widgets::{TextField, Widgets};
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let label = Text::literal("Done");
            let mut ours = HudBatch::scratch([320, 240]);
            game.native_widget_button(
                &mut ours,
                [10.0, 10.0, 100.0, 20.0],
                &label,
                true,
                true,
                None,
            );
            let mut games = HudBatch::scratch([320, 240]);
            sunrise_client::ui::button(
                &mut games,
                &game.gui.atlas,
                10.0,
                10.0,
                100.0,
                20.0,
                &label,
                true,
                true,
            );
            assert!(!ours.vertices.is_empty());
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&ours.vertices),
                bytemuck::cast_slice::<_, u8>(&games.vertices)
            );

            // The view a mod's group sends for a focused field, as `Widgets::draw` builds it.
            struct Draws<'a, 'g>(&'a super::Game<'g>, HudBatch, Vec<TextFieldView>);
            impl mod_api::DrawContext for Draws<'_, '_> {
                fn widget_text_field(&mut self, field: &TextFieldView) -> TextFieldLayout {
                    self.2.push(field.clone());
                    self.0.native_widget_text_field(&mut self.1, field)
                }
            }
            let mut widgets = Widgets::new();
            let id = widgets.add_text_field(TextField::new([10.0, 40.0, 60.0, 20.0], 64));
            let long = "a long name that does not fit in sixty pixels";
            widgets.text_field_mut(id).unwrap().set_value(long);
            widgets.set_focus(&*game, Some(id));
            let mut draws = Draws(game, HudBatch::scratch([320, 240]), Vec::new());
            widgets.draw(&mut draws, &*game, [0.0, 0.0]).unwrap();
            let view = draws.2[0].clone();
            assert!(!draws.1.vertices.is_empty());
            let layout = game.native_widget_text_field(&mut HudBatch::scratch([320, 240]), &view);
            // The cursor is at the end, so the field scrolled on to show it.
            assert!(layout.scroll > 0);
            let shown = layout.edges.len() - 1;
            assert_eq!(layout.scroll + shown, long.chars().count());
            assert_eq!(
                layout.edges[0], 14.0,
                "4 pixels in from a bordered field's left"
            );
            assert!(layout.edges.windows(2).all(|w| w[0] < w[1]));
            assert!(*layout.edges.last().unwrap() <= 10.0 + 60.0 - 4.0);
        });
    }
}
