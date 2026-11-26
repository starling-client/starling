//! A mod's own screen with a text field, two buttons, and a custom widget mounted during
//! `screen_init_post` (`mod_api::client::widgets`): F8 opens it, Tab walks the mounted widgets,
//! typing edits the field, Enter submits and closes it, Echo updates its label, the swatch
//! toggles its color on click, and Done (or Escape) closes it.

// ANCHOR: widget_screen
use mod_api::client::widgets::{Button, ButtonSprites, ScreenWidget, TextField};
use mod_api::{
    ClientHost, DrawContext, Hooks, InputKeyEvent, KeyAction, ScreenInitPostEvent,
    ScreenRenderPostEvent, Text,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// The mod's id, which names its screen.
const ID: &str = "widget_example";

/// A small custom widget that draws a bordered square and toggles its color on left click.
struct AccentSwatch {
    rect: [f32; 4],
    accent: Arc<AtomicBool>,
}

impl ScreenWidget for AccentSwatch {
    fn bounds(&self) -> [f32; 4] {
        self.rect
    }

    fn render(
        &mut self,
        _host: &dyn ClientHost,
        draw: &mut dyn DrawContext,
        _mouse: [f32; 2],
        hovered: bool,
        focused: bool,
        _partial_tick: f32,
    ) {
        let border = if hovered || focused {
            0xFFFF_FFFF
        } else {
            0xFF20_2020
        };
        let fill = if self.accent.load(Ordering::Relaxed) {
            0xFF3A_7BD5
        } else {
            0xFF55_5555
        };
        draw.fill(self.rect, border);
        draw.fill(
            [
                self.rect[0] + 1.0,
                self.rect[1] + 1.0,
                self.rect[2] - 2.0,
                self.rect[3] - 2.0,
            ],
            fill,
        );
    }

    fn mouse_clicked(
        &mut self,
        _host: &mut dyn ClientHost,
        _mouse: [f32; 2],
        button: i32,
        _modifiers: u32,
        _double_click: bool,
    ) -> bool {
        if button != 0 {
            return false;
        }
        self.accent.fetch_xor(true, Ordering::Relaxed);
        true
    }
}

/// The mod: keeps its draft text and accent toggle across resizes while the screen is open.
#[derive(Default)]
pub struct WidgetScreen {
    draft: Arc<Mutex<String>>,
    accent: Arc<AtomicBool>,
}

#[mod_api::sunrise_mod]
impl Hooks for WidgetScreen {
    fn input_key(&self, host: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
        if event.action == KeyAction::Press && event.key == "key.keyboard.f8" && host.playing() {
            if let Ok(mut draft) = self.draft.lock() {
                draft.clear();
            }
            host.mod_screen_open(ID);
        }
    }

    fn screen_init_post(&self, host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
        if host.mod_screen() != Some(ID) {
            return;
        }
        let width = event.screen.rect[2];
        let x = (width / 2.0 - 100.0).floor();
        let initial = self
            .draft
            .lock()
            .map(|draft| draft.clone())
            .unwrap_or_default();

        let on_change_draft = Arc::clone(&self.draft);
        let on_submit_draft = Arc::clone(&self.draft);
        let field = event.widgets.add_text_field(
            TextField::new([x, 40.0, 200.0, 20.0], 64)
                .value(initial)
                .hint(Text::translate("widget_example.screen.note_hint", vec![]))
                .on_change(move |text, _host| {
                    if let Ok(mut draft) = on_change_draft.lock() {
                        *draft = text.to_owned();
                    }
                })
                .on_submit(move |text, host| {
                    if let Ok(mut draft) = on_submit_draft.lock() {
                        *draft = text.to_owned();
                    }
                    host.close_screen();
                }),
        );
        event.widgets.set_initial_focus(field);

        let mut echo_art = ButtonSprites::new("widget_example:echo");
        echo_art.highlighted = Some("widget_example:echo_highlighted".into());
        echo_art.disabled = Some("widget_example:echo_disabled".into());
        echo_art.label_offset = [1.0, -1.0];

        let echo_draft = Arc::clone(&self.draft);
        event.widgets.add_button(
            Button::new(
                [x, 70.0, 98.0, 20.0],
                Text::translate("widget_example.screen.echo", vec![]),
                move |state, _host| {
                    let text = echo_draft
                        .lock()
                        .map(|draft| draft.clone())
                        .unwrap_or_default();
                    state.label = if text.is_empty() {
                        Text::translate("widget_example.screen.echo", vec![])
                    } else {
                        Text::literal(text)
                    };
                },
            )
            .sprites(echo_art),
        );
        event.widgets.add_button(Button::new(
            [x + 102.0, 70.0, 98.0, 20.0],
            Text::translate("gui.done", vec![]),
            |_state, host| {
                host.close_screen();
            },
        ));
        event.widgets.add_widget(Box::new(AccentSwatch {
            rect: [x + 206.0, 40.0, 20.0, 20.0],
            accent: Arc::clone(&self.accent),
        }));
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        if host.mod_screen() != Some(ID) {
            return;
        }
        let [width, _] = event.draw.size();
        event.draw.fill([0.0, 30.0, width, 70.0], 0x8000_0000);
    }
}
// ANCHOR_END: widget_screen
