//! The parentless demo popup and hints sent by `ClientPacketListener.handleGameEvent`.
use super::{Game, Screen};
use sunrise_client::{
    hud::{HudAtlas, HudBatch},
    keybinds::{KeyBinds, display_name},
    text::Text,
    ui, widgets,
};

const MOVEMENT: [&str; 4] = ["key.forward", "key.left", "key.back", "key.right"];
const BUY_MINECRAFT_JAVA: &str = "https://aka.ms/BuyMinecraftJava";
// PopupScreen's default width minus two 18-pixel borders, and its row spacing.
const CONTENT_WIDTH: i32 = 214;
const BUTTON_SPACE: f32 = 6.0;
const BUTTON_WIDTH: f32 = 104.0;
const BUTTON_HEIGHT: f32 = 20.0;
const SECTION_SPACE: f32 = 12.0;
const LINE_HEIGHT: f32 = 9.0;

pub(super) struct DemoIntro {
    title: Text,
    messages: [Text; 2],
    buttons: [Text; 2],
}

impl DemoIntro {
    /// `ClientPacketListener.openDemoIntroScreen` captures the current bindings.
    fn new(keys: &KeyBinds) -> Self {
        let key = |name| display_name(keys.get(name));
        let controls = Text::empty()
            .append(Text::translate(
                "demo.help.movementShort",
                &MOVEMENT.map(key),
            ))
            .append("\n")
            .append(Text::translate("demo.help.movementMouse", &[]))
            .append("\n")
            .append(Text::translate("demo.help.jump", &[key("key.jump")]))
            .append("\n")
            .append(Text::translate(
                "demo.help.inventory",
                &[key("key.inventory")],
            ));
        Self {
            title: Text::translate("demo.help.title", &[]),
            messages: [controls, Text::translate("demo.help.fullWrapped", &[])],
            buttons: [
                Text::translate("demo.help.buy", &[]),
                Text::translate("demo.help.later", &[]),
            ],
        }
    }

    fn layout(&self, atlas: &HudAtlas, width: f32, height: f32) -> Layout {
        let sections = [
            atlas
                .font
                .wrap_components(&self.title.clone().bold(), CONTENT_WIDTH),
            atlas.font.wrap_components(&self.messages[0], CONTENT_WIDTH),
            atlas.font.wrap_components(&self.messages[1], CONTENT_WIDTH),
        ];
        let content_width = sections
            .iter()
            .flatten()
            .map(|line| atlas.font.width_text(line).ceil())
            .fold(2.0 * BUTTON_WIDTH + BUTTON_SPACE, f32::max);
        let button_y =
            sections.iter().map(Vec::len).sum::<usize>() as f32 * LINE_HEIGHT + 3.0 * SECTION_SPACE;
        let content_height = button_y + BUTTON_HEIGHT;
        let x = ((width - content_width) / 2.0).trunc();
        let y = ((height - content_height) / 2.0).trunc();
        let button_x = x + ((content_width - 2.0 * BUTTON_WIDTH - BUTTON_SPACE) / 2.0).trunc();
        Layout {
            sections,
            panel: [
                x - 18.0,
                y - 18.0,
                content_width + 36.0,
                content_height + 36.0,
            ],
            center: x + (content_width / 2.0).trunc(),
            top: y,
            button_y: y + button_y,
            button_x: [button_x, button_x + BUTTON_WIDTH + BUTTON_SPACE],
        }
    }
}

struct Layout {
    sections: [Vec<Text>; 3],
    panel: [f32; 4],
    center: f32,
    top: f32,
    button_y: f32,
    button_x: [f32; 2],
}

impl Game<'_> {
    pub(super) fn apply_demo_event(&mut self, param: f32) {
        // DEMO_EVENT compares the raw float; signed zero matches, but rounded
        // neighbors, unknown phases and NaN do not.
        if param == 0.0 {
            let intro = DemoIntro::new(&self.settings.keys);
            if self.screen == Screen::DemoIntro {
                self.demo_intro = Some(intro);
                self.commit_screen_transition();
            } else {
                self.set_screen_with(Screen::DemoIntro, |game| game.demo_intro = Some(intro));
            }
            if self.screen == Screen::DemoIntro {
                self.capture(false);
            }
            return;
        }
        let (message, mappings): (&str, &[&str]) = match param {
            101.0 => ("demo.help.movement", &MOVEMENT),
            102.0 => ("demo.help.jump", &["key.jump"]),
            103.0 => ("demo.help.inventory", &["key.inventory"]),
            104.0 => ("demo.day.6", &["key.screenshot"]),
            _ => return,
        };
        let args: Vec<_> = mappings
            .iter()
            .map(|mapping| display_name(self.settings.keys.get(mapping)))
            .collect();
        let message = Text::translate(message, &args);
        self.add_chat_line(message.clone(), sunrise_client::chat::ChatSource::Client);
        self.say_system_queued(&message);
    }

    pub(super) fn draw_demo_intro(&self, batch: &mut HudBatch) {
        let Some(intro) = &self.demo_intro else {
            return;
        };
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        let layout = intro.layout(atlas, w, h);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let [x, y, width, height] = layout.panel;
        ui::nine_slice(
            batch,
            atlas,
            "popup/background",
            x,
            y,
            width,
            height,
            u32::MAX,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        widgets::begin_screen(|| {
            // PopupScreen.getNarrationMessage joins the title with both sections.
            let [controls, info] = &intro.messages;
            let message = format!(
                "{}. {}\n{}",
                intro.title.plain(),
                controls.plain(),
                info.plain()
            );
            widgets::ScreenInfo::new(Screen::DemoIntro.id(), message)
        });
        let mut y = layout.top;
        for section in layout.sections {
            for line in section {
                let x = layout.center - (atlas.font.width_text(&line) / 2.0).trunc();
                batch.text_styled(atlas, &line, x, y, u32::MAX, true);
                y += LINE_HEIGHT;
            }
            y += SECTION_SPACE;
        }
        for (x, label) in layout.button_x.into_iter().zip(&intro.buttons) {
            let y = layout.button_y;
            ui::button(
                batch,
                atlas,
                x,
                y,
                BUTTON_WIDTH,
                BUTTON_HEIGHT,
                label,
                true,
                ui::hovered(self.cursor, x, y, BUTTON_WIDTH, BUTTON_HEIGHT),
            );
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
    }

    pub(super) fn demo_intro_click(&mut self) -> bool {
        let Some(intro) = &self.demo_intro else {
            return false;
        };
        let (w, h) = self.gui_size();
        let layout = intro.layout(&self.gui.atlas, w, h);
        let Some(button) = layout.button_x.into_iter().position(|x| {
            ui::hovered(self.cursor, x, layout.button_y, BUTTON_WIDTH, BUTTON_HEIGHT)
        }) else {
            return false;
        };
        self.play_audio_button_click();
        if button == 0 {
            self.confirm_trusted_link(BUY_MINECRAFT_JAVA, Screen::Playing);
        } else {
            self.close_demo_intro();
        }
        true
    }
    /// Closes the parentless popup to the HUD: `PopupScreen.onClose` is
    /// `Gui.setScreen(null)` for a popup without a parent, whose tail is
    /// `MouseHandler.grabMouse`, and Later's button calls `grabMouse` before
    /// `onClose`. The grab is asked for once the screen hooks let the close go
    /// ahead, as every other close to the HUD asks for it.
    pub(super) fn close_demo_intro(&mut self) {
        self.set_screen(Screen::Playing);
        if self.screen == Screen::Playing {
            self.capture(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        in_own_process,
        native_entity_refs::tests::with_game,
        remote::tests::{connect_menu_test, spawn_menu_test},
    };
    use mod_api::{ClientHost, Hooks, ModEntry, ScreenOpening, ScreenOpeningEvent};
    use serde_json::json;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use sunrise_client::chat::{ChatAbilities, ChatVisibility};
    use sunrise_narrator::{GameNarrator, Sink, Status};
    use sunrise_protocol::packets::play::{Clientbound, GameEvent, PlayerPosition};

    struct Narration(Arc<Mutex<Vec<(String, bool)>>>);
    impl Sink for Narration {
        fn say(&mut self, message: &str, interrupt: bool, _: f32) {
            self.0.lock().unwrap().push((message.to_owned(), interrupt));
        }
        fn clear(&mut self) {}
        fn active(&mut self) -> bool {
            true
        }
    }

    struct KeepDemoOpen(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for KeepDemoOpen {
        fn screen_opening(
            &self,
            _: &dyn ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            (event.current.id == "demo_intro"
                && event.new.id == "playing"
                && self.0.load(Ordering::Relaxed))
            .then_some(ScreenOpening::Cancel)
        }
    }

    #[test]
    #[ignore = "daily"]
    fn demo_packets_open_the_parentless_intro_and_narrate_only_exact_hints() {
        in_own_process(
            "game::demo_intro::tests::demo_packets_open_the_parentless_intro_and_narrate_only_exact_hints",
            || {
                let veto = Arc::new(AtomicBool::new(false));
                let mods = vec![ModEntry::new(
                    "demo_observer",
                    Box::new(KeepDemoOpen(veto.clone())),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    with_game(|game| {
                        let (_reader, _writer) = connect_menu_test(game);
                        spawn_menu_test(game);
                        game.handle_packet(Clientbound::PlayerPosition(PlayerPosition {
                            teleport: 1,
                            position: [0.5, 65.0, 0.5],
                            delta: [0.0; 3],
                            yaw: 0.0,
                            pitch: 0.0,
                            relatives: 0,
                        }))
                        .unwrap();
                        game.remote.as_mut().unwrap().level_load.finish();
                        game.focused = true;
                        game.ui.chat.set_abilities(
                            &game.gui.atlas.font,
                            ChatAbilities::new(ChatVisibility::Hidden),
                        );
                        let spoken = Arc::new(Mutex::new(Vec::new()));
                        sunrise_narrator::with_narrator(|narrator| {
                            *narrator =
                                GameNarrator::with_sink(Box::new(Narration(spoken.clone())));
                            narrator.set_status(Status::System);
                        });
                        for (mapping, key) in [
                            ("key.forward", "key.keyboard.i"),
                            ("key.left", "key.keyboard.j"),
                            ("key.back", "key.keyboard.k"),
                            ("key.right", "key.keyboard.l"),
                            ("key.jump", "key.keyboard.q"),
                            ("key.inventory", "key.keyboard.r"),
                        ] {
                            game.settings.keys.set(mapping, key);
                        }
                        let send = |game: &mut Game<'_>, param| {
                            game.handle_packet(Clientbound::GameEvent(GameEvent {
                                event: 5,
                                param,
                            }))
                            .unwrap()
                        };
                        for zero in [0.0, -0.0] {
                            game.set_screen(Screen::Paused);
                            let count = game.remote.as_ref().unwrap().chat_received;
                            send(game, zero);
                            assert_eq!(game.screen, Screen::DemoIntro);
                            let intro = game.demo_intro.as_ref().unwrap();
                            assert_eq!(intro.title.translation_key(), Some("demo.help.title"));
                            let controls = intro.messages[0].component();
                            assert_eq!(
                                controls["extra"],
                                json!([
                                    {"translate": "demo.help.movementShort", "with": [{"text": "I"}, {"text": "J"}, {"text": "K"}, {"text": "L"}]},
                                    {"text": "\n"},
                                    {"translate": "demo.help.movementMouse"},
                                    {"text": "\n"},
                                    {"translate": "demo.help.jump", "with": [{"text": "Q"}]},
                                    {"text": "\n"},
                                    {"translate": "demo.help.inventory", "with": [{"text": "R"}]},
                                ])
                            );
                            assert_eq!(
                                intro.messages[1].translation_key(),
                                Some("demo.help.fullWrapped")
                            );
                            assert_eq!(
                                intro.buttons.each_ref().map(Text::translation_key),
                                [Some("demo.help.buy"), Some("demo.help.later")]
                            );
                            // Buy opens the trusted link confirmation on the HUD; closing
                            // it is `Gui.setScreen(null)` there, which grabs the mouse.
                            let (w, h) = game.gui_size();
                            let layout = intro.layout(&game.gui.atlas, w, h);
                            game.cursor = (layout.button_x[0] + 1.0, layout.button_y + 1.0);
                            game.agent_pointer = true;
                            assert!(game.demo_intro_click());
                            assert_eq!(game.screen, Screen::ConfirmLink);
                            game.close_confirm_link();
                            assert_eq!(game.screen, Screen::Playing, "Buy returns to the HUD");
                            assert!(
                                !game.agent_pointer,
                                "closing the link confirmation requests capture"
                            );

                            game.set_screen(Screen::Paused);
                            send(game, zero);
                            assert_eq!(game.screen, Screen::DemoIntro);
                            let (w, h) = game.gui_size();
                            let layout =
                                game.demo_intro
                                    .as_ref()
                                    .unwrap()
                                    .layout(&game.gui.atlas, w, h);
                            game.cursor = (layout.button_x[1] + 1.0, layout.button_y + 1.0);
                            game.agent_pointer = true;
                            veto.store(true, Ordering::Relaxed);
                            assert!(game.demo_intro_click());
                            assert_eq!(game.screen, Screen::DemoIntro);
                            assert!(
                                game.agent_pointer,
                                "a veto leaves the popup and pointer alone"
                            );
                            veto.store(false, Ordering::Relaxed);
                            assert!(game.demo_intro_click());
                            assert_eq!(
                                game.screen,
                                Screen::Playing,
                                "Later does not return to Pause"
                            );
                            assert!(!game.agent_pointer, "Later requests capture");
                            assert_eq!(game.remote.as_ref().unwrap().chat_received, count);
                            assert!(spoken.lock().unwrap().is_empty());
                        }
                        for (phase, key, args) in [
                            (101.0, "demo.help.movement", &['I', 'J', 'K', 'L'][..]),
                            (102.0, "demo.help.jump", &['Q'][..]),
                            (103.0, "demo.help.inventory", &['R'][..]),
                            (104.0, "demo.day.6", &['Z'][..]),
                        ] {
                            if phase == 104.0 {
                                game.settings.keys.set("key.screenshot", "key.keyboard.z");
                            }
                            let count = game.remote.as_ref().unwrap().chat_received;
                            send(game, phase);
                            assert_eq!(game.screen, Screen::Playing);
                            assert_eq!(game.remote.as_ref().unwrap().chat_received, count + 1);
                            let expected = json!({"translate": key, "with": args.iter().map(|label| json!({"text": label.to_string()})).collect::<Vec<_>>()});
                            assert_eq!(game.ui.chat.recent_components(1), [expected]);
                            let chat = game.ui.chat.recent_lines(1).pop().unwrap();
                            assert_eq!(
                                std::mem::take(&mut *spoken.lock().unwrap()),
                                [(chat, false)]
                            );
                        }
                        let count = game.remote.as_ref().unwrap().chat_received;
                        for phase in [
                            0.25,
                            -0.25,
                            100.75,
                            101.0_f32.next_up(),
                            102.0_f32.next_down(),
                            103.25,
                            104.25,
                            105.0,
                            f32::NAN,
                        ] {
                            send(game, phase);
                            assert_eq!(game.screen, Screen::Playing);
                            assert_eq!(game.remote.as_ref().unwrap().chat_received, count);
                            assert!(spoken.lock().unwrap().is_empty());
                        }
                        sunrise_narrator::set_status(Status::Chat);
                        send(game, 102.0);
                        assert_eq!(game.remote.as_ref().unwrap().chat_received, count + 1);
                        assert!(
                            spoken.lock().unwrap().is_empty(),
                            "a demo hint is system narration, not chat narration"
                        );
                    });
                });
            },
        );
    }
}
