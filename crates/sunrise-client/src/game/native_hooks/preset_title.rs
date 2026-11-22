use super::*;
use crate::game::event_loop::EventLoopCtl;
use sunrise_client::Result;
use winit::keyboard::PhysicalKey;

fn preset_editor_key_code(physical: PhysicalKey) -> u32 {
    let PhysicalKey::Code(code) = physical else {
        return 0;
    };
    let Some(name) = keybinds::key_name(code) else {
        return 0;
    };
    sunrise_client::key_names::KEYBOARD
        .iter()
        .find(|(_, key_name)| *key_name == name)
        .and_then(|(id, _)| u32::try_from(*id).ok())
        .unwrap_or(0)
}

impl Game<'_> {
    pub(in crate::game) fn show_title_with_registry(
        &mut self,
        mut title: sunrise_client::title::Title,
    ) {
        title.set_additional_data_packs(self.pack.additional_data.clone());
        title.set_preset_data_source(self.pack.data_changes.clone());
        let answer = self
            .screen_hooks_heard()
            .then(|| self.title_change(Some(title.mode.id())));
        self.screen_state.build.reopened();
        self.title = Some(title);
        self.screen_activation = self.screen_activation.wrapping_add(1);
        let Some(answer) = answer else {
            self.release_mappings_on_screen_open();
            self.build_screen();
            return;
        };
        match self.screen_decision(answer) {
            ScreenDecision::ModScreen(mod_id) => self.apply_screen(Screen::Mod, Some(mod_id)),
            ScreenDecision::Vanilla(target) => self.open_screen_target(target),
            // With no world nothing else can show: a `Cancel` doesn't keep it shut.
            ScreenDecision::Keep | ScreenDecision::Open => {
                self.release_mappings_on_screen_open();
                self.build_screen();
            }
        }
        self.screen_state.observed = self.observed_now();
        self.shown_init_post();
    }

    /// Render the open bound preset editor's custom UI right before `screen_render_post`.
    pub(in crate::game) fn draw_preset_editor(&self, batch: &mut HudBatch) {
        let Some(session) = self
            .preset_editor_title()
            .and_then(|title| title.preset_editor_session())
        else {
            return;
        };
        let rect = [0.0, 0.0, batch.width, batch.height];
        let paint = batch.paint_checkpoint();
        let rendered = {
            let mut draw = crate::game::host::draw::GameDrawContext::new(self, batch);
            session.render(&mut draw, rect)
        }; // Close any open clips before removing the editor's paint.
        if !rendered {
            paint.discard(batch);
        }
    }

    /// Forward a mouse press to the open bound preset editor.
    pub(in crate::game) fn preset_editor_mouse_clicked(
        &self,
        button: i32,
        double_click: bool,
    ) -> bool {
        let Some(session) = self
            .preset_editor_title()
            .and_then(|title| title.preset_editor_session())
        else {
            return false;
        };
        session.mouse_clicked(self.cursor.0, self.cursor.1, button, double_click)
    }

    /// `KeyboardHandler.keyPress` for the bound preset screen: Pre, the editor and its normal
    /// title widgets, then Post if neither handled it. Window and control input use the same path.
    pub(in crate::game) fn preset_editor_key_pressed(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: KeyCode,
        repeat: bool,
    ) -> Result<()> {
        if self.screen_key_pressed(key, repeat) {
            return Ok(());
        }
        let physical = PhysicalKey::Code(key);
        if self
            .preset_editor_title()
            .and_then(|title| title.preset_editor_session())
            .is_some_and(|session| {
                session.key_pressed(
                    preset_editor_key_code(physical),
                    keybinds::raw_key_code(physical),
                    crate::game::glfw_modifiers(self.modifiers),
                )
            })
        {
            return Ok(());
        }
        let nav = matches!(
            key,
            KeyCode::Tab
                | KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::ArrowUp
                | KeyCode::ArrowDown
        );
        let focused = self.key_focus_key(event_loop, key);
        if focused && !nav {
            return Ok(());
        }
        if !focused {
            let size = self.gui_size();
            let shift = self.has_shift_down();
            if let Some(title) = &mut self.title {
                match key {
                    KeyCode::Escape => {
                        title.escape();
                        self.reopen_data_packs()?;
                        self.save_title_options();
                        return Ok(());
                    }
                    KeyCode::Enter | KeyCode::NumpadEnter => {
                        let (action, pressed) = title.confirm_key_size(size);
                        if pressed {
                            self.save_title_options();
                            self.audio.play_button_click();
                            self.title_action(action)?;
                            return Ok(());
                        }
                    }
                    KeyCode::Tab => title.tab_dir(shift, size),
                    KeyCode::ArrowLeft
                    | KeyCode::ArrowRight
                    | KeyCode::ArrowUp
                    | KeyCode::ArrowDown => {
                        title.arrow_key(key, size, shift);
                    }
                    _ => {}
                }
            }
        }
        // Vanilla Screen.keyPressed returns false even after moving focus.
        self.screen_key_post(key, true, repeat);
        Ok(())
    }

    /// Forward printable characters to the open bound preset editor. Returns `true` if the editor
    /// consumed any character.
    pub(in crate::game) fn preset_editor_type_text(&self, text: &str) -> bool {
        let Some(session) = self
            .preset_editor_title()
            .and_then(|title| title.preset_editor_session())
        else {
            return false;
        };
        let modifiers = crate::game::glfw_modifiers(self.modifiers);
        let mut handled = false;
        for ch in text.chars().filter(|c| !c.is_control()) {
            handled |= session.char_typed(ch, modifiers);
        }
        handled
    }
}
