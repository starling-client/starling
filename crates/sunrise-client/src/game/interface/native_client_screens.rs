//! A client mod's screen control (`ClientHost::close_screen` and
//! `ClientHost::open_inventory`) and the sign it edits
//! (`ClientHost::sign_editor`): the same paths Escape and the inventory key
//! take, so the packets and follow-ups are vanilla's.
use super::Game;
use crate::game::Screen;
use mod_api::{BlockPos, SignEditView};
use winit::keyboard::KeyCode;

impl Game<'_> {
    /// `Minecraft.setScreen` through Escape for a container screen, and the
    /// screen's own close for any other: Escape closes a container with the
    /// menu's `ServerboundContainerClosePacket` first (a bare local close would
    /// leave the server's menu open), while a pause menu closes to the game.
    /// The sign editor and a book are overlays beside the screen: Escape on the sign editor
    /// sends its `ServerboundSignUpdatePacket`, and on a lectern's book the menu's close. A
    /// confirmation, a server's pack prompt or a dialog opened over any of them handles
    /// Escape first. Declining a required pack disconnects rather than returning to the
    /// screen underneath. False when no screen is open, or the foreground screen does not
    /// close with Escape.
    pub(in crate::game) fn close_screen(&mut self) -> bool {
        if self.player.is_none() {
            return false;
        }
        if let Some(closed) = self.close_foreground() {
            return closed;
        }
        if self.sign_edit.is_some() {
            self.close_sign_editor();
            return true;
        }
        // A book or a lectern's book is the same kind of overlay: its screen's onClose (a
        // lectern's closes its menu, sending the container close).
        if self.book.is_some() {
            self.close_book();
            return true;
        }
        if self.screen == Screen::Playing {
            return false;
        }
        let before = self.screen;
        // The Escape path itself, so every screen's own close and follow-ups
        // (and the screen hooks' closing notices) are exactly the key's.
        let _ = self.key_pressed(KeyCode::Escape, KeyCode::Escape);
        self.screen != before
    }
    /// Escape on a screen opened in front of the others, in the order `window_event` gives
    /// it the key: the death-exit confirmation answers no after its delay, a command's or
    /// link's confirmation answers no, `CodeOfConductScreen` stays, `PackConfirmScreen`
    /// declines (disconnecting for a required pack), and a dialog runs its own Escape.
    /// Returns whether it closed, or `None` when no such screen is open.
    fn close_foreground(&mut self) -> Option<bool> {
        if self.screen == Screen::Dead
            && let Some(shown) = self.ui.death_confirm
        {
            if shown.elapsed().as_millis() >= 1000
                && let Err(e) = self.death_confirm_answer(false)
            {
                log::warn!("Sunrise: closing the death-exit confirmation: {e:#}");
            }
            return Some(self.ui.death_confirm.is_none());
        }
        if self.command_prompt.is_some() {
            self.command_prompt_key(KeyCode::Escape);
            return Some(self.command_prompt.is_none());
        }
        if self.link_prompt.is_some() {
            self.answer_link_prompt(false);
            return Some(true);
        }
        if self.code_of_conduct.is_some() {
            return Some(false);
        }
        if self.server_packs.prompt.is_some() {
            if let Err(e) = self.answer_pack_prompt(false) {
                log::warn!("Sunrise: declining the server's resource pack: {e:#}");
            }
            return Some(true);
        }
        if self.dialog_open() {
            let before = self.dialog_screen_key();
            if let Err(e) = self.dialog_key(KeyCode::Escape, None, None) {
                log::warn!("Sunrise: closing the dialog: {e:#}");
            }
            return Some(!self.dialog_open() || self.dialog_screen_key() != before);
        }
        None
    }
    /// `Minecraft.handleKeybinds` for `key.inventory`: the player's inventory
    /// opens locally and nothing is sent (the player menu is always open on the
    /// server), or, riding a vehicle with its own inventory, the key queues the
    /// open-inventory player command instead (`ServerboundPlayerCommandPacket`,
    /// sent at the player's next tick as the key's press is). False with a
    /// screen already open, for a spectator, in a portal transition, or with no
    /// world.
    pub(in crate::game) fn open_inventory(&mut self) -> bool {
        if self.player.is_none()
            || self.screen != Screen::Playing
            || self.sign_edit.is_some()
            || self.spectator()
            || self.in_portal_transition()
        {
            return false;
        }
        if self.remote.is_some() && (self.riding_mount_with_menu() || self.riding_chest_boat()) {
            self.queue_mapping_click("key.inventory");
            return true;
        }
        self.open_inventory_key_action();
        self.screen != Screen::Playing
    }
    /// `AbstractSignEditScreen`'s sign: its block position and the side the
    /// editor is on. None when no sign editor is open, or outside a world.
    pub(in crate::game) fn sign_editor(&self) -> Option<SignEditView> {
        self.player.as_ref()?;
        let edit = self.sign_edit.as_ref()?;
        Some(SignEditView {
            pos: BlockPos::new(edit.pos.0, edit.pos.1, edit.pos.2),
            front: edit.front,
        })
    }
}

#[cfg(test)]
mod client_screen_tests {
    use super::*;
    use crate::game::books::BookScreen;
    use crate::game::native_entity_refs::tests::with_client_game;
    use crate::game::remote::tests::connect_menu_test;
    use sunrise_client::{
        containers::BlockEntity,
        items::ItemStack,
        mobs::Mob,
        signs::{SignData, SignText},
    };
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::connection::ConnReader;
    use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};

    /// What the window sent since the last call, as play packets.
    fn sent(reader: &mut ConnReader) -> Vec<S> {
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let mut packets = Vec::new();
        while let Ok((id, body)) = reader.recv() {
            packets.extend(S::decode(id, &body).ok());
        }
        packets
    }

    /// A window logged in to the overworld at (0.5, 65, 0.5), its player made
    /// by the server's position packet, with everything the login sent drained.
    fn logged_in(game: &mut Game<'_>) -> ConnReader {
        let (mut reader, _writer) = connect_menu_test(game);
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 1,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [0.5, 65.0, 0.5],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: 0,
        }))
        .unwrap();
        sent(&mut reader);
        reader
    }

    /// The chest menu's network id, from the prepared registries report.
    fn chest_menu() -> i32 {
        menu_id("minecraft:generic_9x3")
    }
    fn menu_id(name: &str) -> i32 {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let report = sunrise_protocol::registry::Builtin::find(&root)
            .expect("the prepared resources carry registries.json");
        let builtin = sunrise_protocol::registry::Builtin::load(&report).unwrap();
        builtin.get("minecraft:menu").unwrap().id(name).unwrap()
    }

    /// LecternScreen.onClose closes its menu (`closeContainer`): a lectern opened by the server
    /// closes through the screen close with the menu's id, and its book overlay is gone.
    #[test]
    #[ignore = "daily"]
    fn closing_a_lectern_sends_its_menu_id() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            let menu = menu_id("minecraft:lectern");
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 3,
                menu,
                title: sunrise_core::nbt::Tag::String("Lectern".into()),
            }))
            .unwrap();
            sent(&mut reader);

            assert!(game.close_screen(), "the lectern's book is open");
            assert!(game.book.is_none());
            let Some(S::ContainerClose(close)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::ContainerClose(_)))
            else {
                panic!("the lectern's menu close goes out");
            };
            assert_eq!(close.container, 3);
        });
    }

    /// A link's `ConfirmLinkScreen` over a lectern is the foreground screen: closing it answers
    /// no and leaves the lectern's book and the server's menu open, with nothing sent. The next
    /// close is the lectern's own.
    #[test]
    #[ignore = "daily"]
    fn a_confirmation_over_a_lectern_closes_alone() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 3,
                menu: menu_id("minecraft:lectern"),
                title: sunrise_core::nbt::Tag::String("Lectern".into()),
            }))
            .unwrap();
            // Screen.clickUrlAction with Prompt on Links on, as a page's link opens it.
            (game.settings.chat_links, game.settings.chat_links_prompt) = (true, true);
            game.click_url("https://example.org".into());
            assert!(
                game.link_prompt.is_some(),
                "the confirmation is over the book"
            );
            sent(&mut reader);
            fn menu(game: &Game<'_>) -> Option<i32> {
                game.remote.as_ref().unwrap().menu.as_ref().map(|m| m.0)
            }

            assert!(game.close_screen(), "the confirmation was open");
            assert!(game.link_prompt.is_none());
            assert!(
                matches!(
                    game.book,
                    Some(BookScreen::View {
                        lectern: Some(_),
                        ..
                    })
                ),
                "the lectern's book stays open"
            );
            assert_eq!(menu(game), Some(3), "the server's lectern menu stays open");
            assert!(sent(&mut reader).is_empty(), "no menu close went out");

            assert!(game.close_screen(), "the lectern is the screen now");
            assert!(game.book.is_none());
            assert_eq!(menu(game), None);
            assert!(
                sent(&mut reader)
                    .iter()
                    .any(|p| matches!(p, S::ContainerClose(c) if c.container == 3))
            );
        });
    }

    /// `BookViewScreen` and `BookEditScreen` close as `Screen.onClose`: the book goes and
    /// nothing is sent, neither a menu close (no menu is open) nor the edited pages.
    #[test]
    #[ignore = "daily"]
    fn closing_an_ordinary_book_sends_nothing() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.open_book(BookScreen::View {
                pages: Vec::new(),
                page: 0,
                lectern: None,
            });
            assert!(game.book.is_some());
            sent(&mut reader);
            assert!(game.close_screen(), "the written book was open");
            assert!(game.book.is_none());
            assert!(
                sent(&mut reader).is_empty(),
                "a written book's close sends nothing"
            );

            *game.held_mut() = Some(ItemStack::new("minecraft:writable_book", 1));
            let stack = game.held().unwrap().clone();
            assert!(game.remote_use_book(&stack), "the editor opened");
            game.book_input(None, Some("unsaved"));
            sent(&mut reader);
            assert!(game.close_screen(), "the editor was open");
            assert!(game.book.is_none());
            assert!(
                sent(&mut reader).is_empty(),
                "Escape in the editor sends no edit and no menu close"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn closing_a_chest_sends_its_menu_id_and_closes_the_client_menu() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            let menu = chest_menu();
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 7,
                menu,
                title: sunrise_core::nbt::Tag::String("Chest".into()),
            }))
            .unwrap();
            assert_eq!(game.screen, Screen::Inventory);
            assert!(game.screen_title().is_some(), "the server's menu is open");
            sent(&mut reader);

            assert!(game.close_screen(), "a chest screen is open");
            assert_eq!(game.screen, Screen::Playing);
            assert!(game.screen_title().is_none(), "the client menu is closed");
            let Some(S::ContainerClose(close)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::ContainerClose(_)))
            else {
                panic!("the container close with the menu's id goes out");
            };
            assert_eq!(close.container, 7);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn closing_the_inventory_sends_the_close_for_id_zero() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.set_screen(Screen::Inventory);
            assert!(game.close_screen());
            assert_eq!(game.screen, Screen::Playing);
            let Some(S::ContainerClose(close)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::ContainerClose(_)))
            else {
                panic!("the player container close goes out");
            };
            assert_eq!(close.container, 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn closing_with_no_screen_reports_false_and_sends_nothing() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert_eq!(game.screen, Screen::Playing);
            assert!(!game.close_screen());
            assert!(sent(&mut reader).is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn opening_the_inventory_is_local_and_needs_no_screen() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert!(game.open_inventory());
            assert_eq!(game.screen, Screen::Inventory);
            assert!(
                sent(&mut reader).is_empty(),
                "the inventory opens without telling the server"
            );
            assert!(!game.open_inventory(), "a screen is already open");
            assert_eq!(game.screen, Screen::Inventory);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn opening_the_inventory_riding_a_horse_sends_the_player_command() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            // The player's keys act once the client told the server it loaded.
            game.streaming
                .insert_remote(&mut game.world, sunrise_client::chunk::Chunk::empty(0, 0));
            game.poll_stationary_client().unwrap();
            sent(&mut reader); // PlayerLoaded precedes the inventory action.
            let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let mut horse = Mob::new(42, kind, Vec3::new(0.5, 65.0, 0.5), 0.0);
            horse.extra.tame = true;
            game.mobs.push(horse);
            game.riding = Some(42);
            // An overlay counts as an open screen even when a mount would otherwise
            // make open_inventory report that its command was queued.
            let at = (3, 64, -2);
            game.world
                .block_entities
                .insert(at, BlockEntity::Sign(SignData::default()));
            game.remote_open_sign_editor(play::OpenSignEditor {
                pos: [3, 64, -2],
                front: true,
            });
            assert!(!game.open_inventory(), "the sign editor is already open");
            assert!(game.sign_editor().is_some());
            assert_eq!(game.screen, Screen::Playing);
            assert!(
                sent(&mut reader).is_empty(),
                "no vehicle menu command was sent"
            );
            assert!(game.close_screen());
            sent(&mut reader);
            assert!(game.open_inventory());
            // The key's press is replayed at the player's next tick, so the
            // command goes out there, as the key's does.
            game.tick_remote_level().unwrap();
            let Some(S::PlayerCommand(command)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::PlayerCommand(_)))
            else {
                panic!("the open-inventory command goes out");
            };
            assert_eq!((command.id, command.action), (7, 5));
            assert_eq!(command.data, 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_sign_editor_reports_its_sign_and_closing_sends_the_update() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            let at = (3, 64, -2);
            let sign = SignData {
                front: SignText {
                    lines: [
                        "first".into(),
                        "second".into(),
                        "third".into(),
                        "fourth".into(),
                    ],
                    ..SignText::default()
                },
                ..SignData::default()
            };
            game.world
                .block_entities
                .insert(at, BlockEntity::Sign(sign));
            game.remote_open_sign_editor(play::OpenSignEditor {
                pos: [at.0, at.1, at.2],
                front: true,
            });
            assert_eq!(
                game.sign_editor(),
                Some(SignEditView {
                    pos: BlockPos::new(3, 64, -2),
                    front: true,
                })
            );

            assert!(game.close_screen());
            assert_eq!(game.sign_editor(), None);
            let Some(S::SignUpdate(update)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::SignUpdate(_)))
            else {
                panic!("the sign update goes out with the close");
            };
            assert_eq!(update.pos, [3, 64, -2]);
            assert!(update.front);
            assert_eq!(
                update.lines,
                [
                    "first".to_owned(),
                    "second".to_owned(),
                    "third".to_owned(),
                    "fourth".to_owned()
                ]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn no_sign_editor_reports_no_sign() {
        with_client_game(|game| {
            logged_in(game);
            assert_eq!(game.sign_editor(), None);
        });
    }
}
