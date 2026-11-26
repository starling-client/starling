//! Starling's part of startup_menu.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Loading {
    pub(super) fn touch_layout_path() -> PathBuf {
        std::env::current_dir()
            .unwrap_or_default()
            .join(super::super::touch::LAYOUT_FILE)
    }

    /// The touch layout editor in place of the options, on the layout as it was saved: the
    /// controls can be laid out while the game is set up, before a world.
    pub(super) fn open_menu_touch_editor(&mut self) {
        let mut config = std::fs::read_to_string(Self::touch_layout_path()).map_or_else(
            |_| sunrise_client::touch::LayoutConfig::default(),
            |text| sunrise_client::touch::LayoutConfig::from_json(&text),
        );
        config.offer_custom(self.menu_custom_button_size());
        self.save_menu_settings();
        self.menu.options_open = false;
        self.menu.dragging = false;
        self.menu.touch_editor = Some((Default::default(), config));
    }

    /// The canvas open before a game: no imports run yet, and what the inbox gets is its picture.
    pub(in super::super) fn menu_button_image(&mut self) {
        if !self.menu.options_open || !self.menu.options.painting() {
            return;
        }
        match sunrise_client::import::take_picture() {
            Some(Ok((w, h, pixels))) => self.menu.options.import_button_image(w, h, &pixels),
            Some(Err(e)) => log::warn!("Sunrise: {e}"),
            None => {}
        }
    }

    pub(super) fn menu_custom_button_size(&self) -> f32 {
        let (w, h) = self.window.as_ref().map_or((1.0, 1.0), |w| {
            let s = w.inner_size();
            (s.width.max(1) as f32, s.height.max(1) as f32)
        });
        super::super::touch::custom_button_size(w, h)
    }

    pub(in super::super) fn menu_touch_editing(&self) -> bool {
        self.menu.touch_editor.is_some()
    }

    /// Done: the layout saved, the options back.
    pub(super) fn close_menu_touch_editor(&mut self) {
        let Some((_, config)) = self.menu.touch_editor.take() else {
            return;
        };
        if let Err(e) = std::fs::write(Self::touch_layout_path(), config.to_json()) {
            log::warn!("Sunrise: the touch layout was not saved: {e}");
        }
        self.menu.options_open = true;
    }

    /// While the editor shows it takes every touch; the back key is its Done.
    pub(in super::super) fn menu_touch_editor_event(&mut self, event: &WindowEvent) -> bool {
        if self.menu.touch_editor.is_none() {
            return false;
        }
        let size = self.window.as_ref().map_or([1.0, 1.0], |w| {
            let s = w.inner_size();
            [s.width.max(1) as f32, s.height.max(1) as f32]
        });
        match event {
            WindowEvent::Touch(touch) => {
                let p = [touch.location.x as f32, touch.location.y as f32];
                use sunrise_client::touch::EditorAction;
                let custom = self.menu_custom_button_size();
                let action = self
                    .menu
                    .touch_editor
                    .as_mut()
                    .and_then(|(editor, config)| {
                        let action =
                            editor.event(config, size[0], size[1], touch.id, touch.phase, p);
                        if action == Some(EditorAction::Reset) {
                            config.offer_custom(custom);
                        }
                        action
                    });
                if matches!(action, Some(EditorAction::Done | EditorAction::Add)) {
                    self.close_menu_touch_editor();
                }
                if action == Some(EditorAction::Add) {
                    self.settings.custom_buttons = super::super::touch::saved_custom_buttons();
                    self.menu.options.add_custom_button(&mut self.settings);
                }
            }
            WindowEvent::KeyboardInput { event: key, .. }
                if key.state == ElementState::Pressed
                    && key.logical_key
                        == winit::keyboard::Key::Named(winit::keyboard::NamedKey::BrowserBack) =>
            {
                self.close_menu_touch_editor();
            }
            // The mouse the fingers would make, and the rest: the editor's.
            WindowEvent::MouseInput { .. } | WindowEvent::CursorMoved { .. } => {}
            _ => return false,
        }
        true
    }

    /// The editor over the setup or the title: the controls as the game lays them out, named
    /// as the game names them (mods add none before a world).
    pub(in super::super) fn draw_menu_touch_editor(&self, batch: &mut HudBatch, size: [u32; 2]) {
        let (Some(gui), Some((editor, config))) = (&self.gui, &self.menu.touch_editor) else {
            return;
        };
        let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
        let mut layout = sunrise_client::touch::Layout::from_config(config, w, h, batch.scale);
        layout.dpad = config.dpad.unwrap_or(false);
        let face = |target: sunrise_client::touch::Target| match target {
            sunrise_client::touch::Target::Button(b) => (b.label().to_owned(), None),
            sunrise_client::touch::Target::Extra(i) => {
                let key = &config.extra[i].key;
                let own = key
                    .strip_prefix(sunrise_client::touch::CUSTOM_BUTTONS)
                    .and_then(|k| k.strip_prefix(':'))
                    .and_then(|id| config.custom.iter().find(|b| b.id.to_string() == id));
                (own.map_or_else(|| key.clone(), |b| b.label()), None)
            }
            sunrise_client::touch::Target::Stick => (String::new(), None),
        };
        // Starling's own buttons (packaging/touch), which every atlas carries.
        let sprite = |name: &str| {
            let key = format!("starling:gui/sprites/touch/{name}");
            gui.atlas.sprites.contains_key(&key).then_some(key)
        };
        let skin = |target: sunrise_client::touch::Target| match target {
            sunrise_client::touch::Target::Button(b) => sprite(b.id()),
            sunrise_client::touch::Target::Stick if !layout.dpad => sprite("stick"),
            _ => None,
        };
        let mut ui = sunrise_client::ui_theme::Ui::new(&gui.atlas, batch, (-1.0, -1.0));
        super::super::touch::draw_editor(
            &mut ui,
            &layout,
            config,
            editor,
            &face,
            &|_| sunrise_client::touch::Shape::Round,
            &skin,
        );
        ui.finish();
    }

    /// While the credits roll, a lifted finger closes them, as Escape does: a phone has no
    /// Escape, and the touch's mouse clicks do nothing to them.
    pub(in super::super) fn credits_touch(&mut self, touch: &winit::event::Touch) -> bool {
        if self.menu.win.is_none() {
            return false;
        }
        if touch.phase == winit::event::TouchPhase::Ended {
            self.menu.win = None;
        }
        true
    }

    /// Whether a finger landing at `touch` is on a button of the options (a cycle button, a row of
    /// Video Settings), which a drag starting sideways would press: it only scrolls.
    pub(in super::super) fn menu_options_button_at(&mut self, touch: &winit::event::Touch) -> bool {
        if !self.menu.options_open || self.gui.is_none() {
            return false;
        }
        let batch = HudBatch::new(
            self.menu_size(),
            sunrise_client::hud::GuiScale {
                requested: 0,
                unicode: false,
            },
        );
        let cursor = (
            touch.location.x as f32 / batch.scale,
            touch.location.y as f32 / batch.scale,
        );
        self.menu
            .options
            .list_button_at(&self.settings, (batch.width, batch.height), cursor)
    }

    /// Whether `touch` lands on a slider of the options' settings pages, which it takes only
    /// moving clearly sideways (`TouchPointer::prefer_slider`).
    pub(in super::super) fn menu_options_slider_at(&mut self, touch: &winit::event::Touch) -> bool {
        if !self.menu.options_open || self.gui.is_none() {
            return false;
        }
        let batch = HudBatch::new(
            self.menu_size(),
            sunrise_client::hud::GuiScale {
                requested: 0,
                unicode: false,
            },
        );
        let cursor = (
            touch.location.x as f32 / batch.scale,
            touch.location.y as f32 / batch.scale,
        );
        self.menu
            .options
            .list_slider_at(&self.settings, (batch.width, batch.height), cursor)
    }

    /// A new generation's loader (a pack reload, or the setup's prepared files): its messages are
    /// the ones read from now on, the last generation's `Done` being delivered or not.
    pub(in super::super) fn follow_loader(
        &mut self,
        receiver: Receiver<LoaderMsg>,
        handle: JoinHandle<()>,
    ) {
        self.receiver = receiver;
        self.handle = Some(handle);
        self.loaded = None;
        self.delivered = false;
    }
}
