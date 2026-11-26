//! A key press while a world is open (`KeyboardHandler.keyPress` and the gameplay mappings).
use super::inventory_input::drop_whole_stack;
use super::*;
use sunrise_client::keybinds::VanillaMappingSlot;

const GUI_KEYS: [&str; 5] = [
    "key.toggleGui",
    "key.advancements",
    "key.socialInteractions",
    "key.chat",
    "key.command",
];

impl Game<'_> {
    /// `new InventoryScreen(player)`: the player's inventory, or in creative
    /// mode the creative inventory `InventoryScreen.init` swaps in.
    pub(super) fn open_player_inventory(&mut self) {
        if self.instabuild() {
            self.set_screen(Screen::Creative);
            self.refresh_creative_tabs();
            self.creative_screen.reopen();
            self.refresh_saved_hotbars();
        } else {
            self.set_screen(Screen::Inventory);
        }
        self.capture(false);
    }
    /// A key press while a world is open (keyboard or control socket). `key`
    /// is the vanilla default the pressed mapping translates to, `raw` the
    /// physical key as pressed (the same where the caller had no window key).
    /// The input dispatcher runs a receiving screen's key actions separately.
    pub(super) fn key_pressed(&mut self, key: KeyCode, raw: KeyCode) -> Result<()> {
        // Scripted presses and controller presses are delivered to the same mappings
        // as window input; direct screen keys remain immediate.
        if !self.mod_mappings_screen_open() && !self.replay.playing() {
            let key = if key == KeyCode::Enter {
                KeyCode::KeyT
            } else {
                key
            };
            if let Some(slot) = VanillaMappingSlot::by_default_key(key)
                && Self::tick_mapping(slot.definition().0)
            {
                self.vanilla_mappings.click(slot);
                return Ok(());
            }
        }
        self.immediate_key_pressed(key, raw)
    }

    pub(super) fn tick_mapping(name: &str) -> bool {
        matches!(
            name,
            "key.togglePerspective"
                | "key.smoothCamera"
                | "key.toggleSpectatorShaderEffects"
                | "key.inventory"
                | "key.quickActions"
                | "key.swapOffhand"
                | "key.drop"
        ) || GUI_KEYS.contains(&name)
            || name.starts_with("key.hotbar.")
    }

    pub(super) fn immediate_key_pressed(&mut self, key: KeyCode, raw: KeyCode) -> Result<()> {
        self.direct_craft_stop();
        if self.screen == Screen::Playing && self.replay.playing() {
            self.studio_key(key);
            return Ok(());
        }
        // `CreativeModeInventoryScreen.keyPressed` clears `ignoreTextInput` on
        // every key, before it decides anything.
        self.creative_ignore_text = false;
        if self.sleeping.is_some()
            && self.screen == Screen::Playing
            && !matches!(
                key,
                KeyCode::F1 | KeyCode::F2 | KeyCode::F11 | KeyCode::KeyT | KeyCode::Slash
            )
        {
            if matches!(key, KeyCode::Escape | KeyCode::ShiftLeft) {
                self.wake_up(false, true);
            }
            return Ok(());
        }
        if sunrise_client::keybinds::key_name(key)
            .is_some_and(|name| self.hud_editor_key_press(name) || self.shader_pack_key_press(name))
        {
            return Ok(());
        }
        // Escape cancels a press on the item panel; one that is a drag takes
        // the Escape, and the screen stays open.
        if key == KeyCode::Escape && self.mod_browser_drag_escape() {
            return Ok(());
        }
        if key == KeyCode::Escape && self.mod_browser_back() {
            return Ok(());
        }
        match key {
            KeyCode::Escape => match self.screen {
                Screen::Options => {
                    self.options_key(KeyCode::Escape, None)?;
                }
                Screen::Playing => {
                    if self.replay.playing() && self.captured {
                        self.capture(false);
                        return Ok(());
                    }
                    self.pause_focus = None;
                    self.set_screen(Screen::Paused);
                    self.pause_menu = true;
                    self.capture(false);
                }
                Screen::Creative => {
                    self.pad_clear_focus();
                    self.mod_browser_closed();
                    self.carried = None;
                    self.set_screen_with(Screen::Playing, |g| g.close_creative_menu());
                    self.capture(true);
                }
                Screen::Paused | Screen::Inventory => {
                    // RecipeBookComponent.keyPressed: a narrow window's open
                    // book takes Escape to close itself, not the screen.
                    if self.screen == Screen::Inventory && self.recipe_book_escape() {
                        return Ok(());
                    }
                    // AbstractContainerScreen.onClose closes the menu (on a server, with
                    // a ServerboundContainerClosePacket); PauseScreen.onClose only
                    // swaps the screen.
                    if self.screen == Screen::Inventory {
                        self.close_container();
                    } else {
                        self.pad_clear_focus();
                    }
                    self.pause_focus = None;
                    self.set_screen(Screen::Playing);
                    self.pause_menu = true;
                    self.capture(true);
                }
                Screen::GameModeSwitcher => self.close_gamemode_switcher(false),
                Screen::Dead => {
                    if self.ui.death_confirm.is_some() {
                        self.death_confirm_answer(false)?;
                    }
                }
                Screen::Stats | Screen::Friends => self.set_screen(Screen::Paused),
                Screen::Social => self.close_social(),
                Screen::ServerLinks => self.set_screen(Screen::Paused),
                // ConfirmScreen.keyPressed: Escape answers no; both return to
                // the Server Links screen they came from.
                Screen::ConfirmLink => self.close_confirm_link(),
                Screen::DemoIntro => self.close_demo_intro(),
                Screen::DialogWarning => self.set_screen(Screen::ServerLinks),
                Screen::Advancements => self.close_advancements(),
                Screen::WorldOptions => self.world_options_escape(),
                Screen::CommandBlock => self.close_command_block(),
                Screen::BlockEditor => self.close_block_editor(),
                Screen::Chat => self.close_chat(chat::ChatExit::Intentional),
                Screen::Win => self.close_win_screen()?,
                Screen::DebugOptions => self.close_debug_options(),
                Screen::Mod => self.close_mod_screen(),
                Screen::HudEditor => self.close_hud_editor(),
            },
            KeyCode::F11 => self.toggle_fullscreen(),
            KeyCode::KeyE
                if self.screen == Screen::Creative && self.creative_screen.tab().search => {}
            KeyCode::KeyE => match self.screen {
                Screen::Creative | Screen::Inventory => self.close_inventory_key(),
                _ => {}
            },
            _ if self.screen == Screen::Paused && self.pause_menu => {
                let _ = self.pause_key(key)?;
            }
            _ if self.screen == Screen::Dead => {
                let _ = self.death_key(key)?;
            }
            _ if self.screen == Screen::Stats => self.stats_key(raw),
            _ => {}
        }
        Ok(())
    }
    /// Minecraft.handleKeybinds and Gui.handleKeybinds, in their polling order.
    /// Opening a screen releases the live mappings, including clicks not yet reached.
    pub(super) fn handle_play_keybinds(&mut self) {
        while self.consume_mapping_click("key.togglePerspective") {
            self.cycle_camera();
        }
        while self.consume_mapping_click("key.smoothCamera") {
            self.smooth_camera_action();
        }
        for name in GUI_KEYS {
            if name == "key.command" && self.mod_mappings_screen_open() {
                break;
            }
            while self.consume_mapping_click(name) {
                match name {
                    "key.toggleGui" => self.toggle_hide_gui(),
                    "key.advancements" => self.open_advancements(false),
                    "key.socialInteractions" if self.is_multiplayer_server() => {
                        self.hide_social_tutorial();
                        self.open_social(false);
                    }
                    "key.socialInteractions" => {
                        let message = sunrise_client::text::Text::translate(
                            "multiplayer.socialInteractions.not_available",
                            &[],
                        );
                        self.titles.set_overlay(message.clone(), false);
                        self.say_system_now(&message);
                    }
                    "key.chat" => self.open_chat_method(""),
                    "key.command" => self.open_chat_method("/"),
                    _ => unreachable!("listed GUI key"),
                }
            }
        }
        while self.consume_mapping_click("key.toggleSpectatorShaderEffects") {
            self.toggle_spectator_post_effect();
        }
        self.select_queued_hotbar();
        while self.consume_mapping_click("key.inventory") {
            self.open_inventory_key_action();
        }
        while self.consume_mapping_click("key.quickActions") {
            self.open_quick_actions();
        }
        while self.consume_mapping_click("key.swapOffhand") {
            if !self.own_player_is_spectator() {
                if self.remote.is_some() {
                    self.remote_swap_hands();
                } else {
                    self.own_swap_hands();
                }
            }
        }
        while self.consume_mapping_click("key.drop") {
            if !self.own_player_is_spectator() {
                if self.remote.is_some() {
                    let all = self.has_control_down() || self.input_takes_stack_drop();
                    self.remote_drop(all);
                } else {
                    let all = drop_whole_stack(self.modifiers) || self.input_takes_stack_drop();
                    self.drop_held_item(all);
                }
            }
        }
    }

    /// The inventory key either asks the vehicle's server menu or opens the local inventory.
    pub(super) fn open_inventory_key_action(&mut self) {
        if self.in_portal_transition() {
            return;
        }
        if self.remote.is_some() && (self.riding_mount_with_menu() || self.riding_chest_boat()) {
            self.remote_request_mount_inventory();
        } else if self.remote.is_some() || (!self.spectator() && !self.open_ridden_mount_menu()) {
            if self.tutorial.open_inventory() {
                self.persist_tutorial_step();
            }
            self.recipe_ui.reset_screen();
            self.craft_size = 2;
            self.open_player_inventory();
        }
    }

    /// handlePlayerAction SWAP_ITEM_WITH_OFFHAND for the window's own player
    /// with no server: the mods hear it as a server's players
    /// (`player_swap_hands`), and may stop it or change what each hand gets.
    /// Then setItemInHand for the off hand, then the main hand. The stacks are
    /// copied only for a listener: `heard` holds the old main and off hands.
    fn own_swap_hands(&mut self) {
        if self.spectator() {
            return;
        }
        let heard = mod_api::__private::listened(mod_api::HookId::PlayerSwapHands)
            .then(|| (self.inventory[self.selected].clone(), self.offhand.clone()));
        if let Some((old_main, old_off)) = &heard {
            let (mut to_main, mut to_off, mut cancel) = (old_off.clone(), old_main.clone(), false);
            let (uuid, name) = (self.own_uuid(), self.own_profile_name());
            self.native_player_swap_hands(uuid, &name, &mut to_main, &mut to_off, &mut cancel);
            if cancel {
                return;
            }
            (self.inventory[self.selected], self.offhand) = (to_main, to_off);
        } else {
            std::mem::swap(&mut self.offhand, &mut self.inventory[self.selected]);
        }
        self.hand_stack_replaced(self.selected);
        self.hand_stack_replaced(shield::OFFHAND_SLOT);
        let (offhand, main) = (self.offhand.clone(), self.inventory[self.selected].clone());
        let (old_main, old_off) = match &heard {
            Some((old_main, old_off)) => (old_main.as_ref(), old_off.as_ref()),
            None => (offhand.as_ref(), main.as_ref()),
        };
        self.player_on_equip("offhand", old_off, offhand.as_ref());
        self.player_on_equip("mainhand", old_main, main.as_ref());
        // handlePlayerAction ends with stopUsingItem.
        if self.using.is_some() {
            self.stop_active_item_use();
        }
    }
}

#[cfg(test)]
mod swap_hands_tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{Flow, Hooks, PlayerSwapHandsEvent, ServerHost};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use sunrise_client::items::ItemStack;

    /// Puts a torch in the off hand instead of the sword, or cancels; counts the calls.
    struct Torch(Arc<AtomicBool>, Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Torch {
        fn player_swap_hands(
            &self,
            _: &mut dyn ServerHost,
            event: &mut PlayerSwapHandsEvent<'_>,
        ) -> Flow {
            self.1.fetch_add(1, Ordering::SeqCst);
            if self.0.load(Ordering::SeqCst) {
                return Flow::Cancel;
            }
            assert!(event.to_main_hand.is_none());
            assert_eq!(event.to_off_hand.as_ref().map(|s| s.count()), Some(1));
            let torch = mod_api::__private::item_key("minecraft:torch").unwrap();
            *event.to_off_hand = Some(mod_api::ItemStack::new(torch, 1).unwrap());
            Flow::Continue
        }
    }

    /// The window's own swap contract: a cancel
    /// keeps both hands, and a listener's stack lands in the off hand.
    #[test]
    #[ignore = "daily"]
    fn the_own_player_swap_asks_the_listener_for_each_hand() {
        let (cancel, calls) = (
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicUsize::new(0)),
        );
        let torch = Torch(Arc::clone(&cancel), Arc::clone(&calls));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("torch", Box::new(torch))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let selected = game.selected;
                game.inventory[selected] = Some(ItemStack::new("minecraft:diamond_sword", 1));
                game.offhand = None;
                game.set_mode_id(3);
                game.own_swap_hands();
                assert_eq!(calls.load(Ordering::SeqCst), 0);
                assert_eq!(
                    game.inventory[selected].as_ref().map(|s| &*s.item),
                    Some("minecraft:diamond_sword")
                );
                assert!(game.offhand.is_none());
                game.set_mode_id(0);
                game.own_swap_hands();
                assert_eq!(
                    game.inventory[selected].as_ref().map(|s| &*s.item),
                    Some("minecraft:diamond_sword")
                );
                assert!(game.offhand.is_none());
                cancel.store(false, Ordering::SeqCst);
                game.own_swap_hands();
                assert!(game.inventory[selected].is_none());
                assert_eq!(
                    game.offhand.as_ref().map(|s| &*s.item),
                    Some("minecraft:torch")
                );
            });
        });
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// The swap through a LAN host's registered connection: the packet goes
    /// through the hub's inbox and the game's packet pass
    /// (`tick_lan_packets`), and what the guest's client is sent comes off
    /// its sender. The hook is heard once a press; a cancel sends the client
    /// both hands as they were, and a listener's stack reaches its off-hand
    /// slot.
    #[test]
    #[ignore = "daily"]
    fn a_guest_swap_through_its_connection_is_heard_once_and_sends_the_hands() {
        use crate::game::play_ping_tests::{fixture, output};
        use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
        let (cancel, calls) = (
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicUsize::new(0)),
        );
        let torch = Torch(Arc::clone(&cancel), Arc::clone(&calls));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("torch", Box::new(torch))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let (mut game, receivers) = fixture(&mut resources);
            let selected = game.lan.as_ref().unwrap().hub.guests[0].selected;
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[selected] = Some(ItemStack::new("minecraft:diamond_sword", 1));
                g.offhand = None;
            });
            // One press through the connection's inbox; the hands (main, off)
            // in the last inventory the client was sent.
            let press = |game: &mut crate::game::Game<'_>| {
                let swap = S::PlayerAction(play::PlayerAction {
                    action: 7,
                    pos: [0; 3],
                    direction: 0,
                    sequence: 0,
                });
                let host = game.lan.as_mut().unwrap();
                host.hub
                    .guests
                    .update_connection(1, |g| {
                        g.inbox
                            .push_back(sunrise_client::lan::guest::InboxItem::Packet(swap, None))
                    })
                    .unwrap();
                game.level_pass.first = false;
                game.level_pass.packets_drained = false;
                game.tick_lan_packets();
                let hub = &game.lan.as_ref().unwrap().hub;
                sunrise_client::lan::flush_all(hub.guests.iter().map(|g| &g.sender));
                output(&receivers[0])
                    .into_iter()
                    .filter_map(|p| match p {
                        C::ContainerSetContent(c) if c.container == 0 => {
                            Some((c.items[36 + selected].clone(), c.items[45].clone()))
                        }
                        _ => None,
                    })
                    .next_back()
                    .expect("the client was sent its inventory")
            };
            let hub = &mut game.lan.as_mut().unwrap().hub;
            let sword = hub.stack(&Some(ItemStack::new("minecraft:diamond_sword", 1)));
            let torch = hub.stack(&Some(ItemStack::new("minecraft:torch", 1)));
            assert!(sword.is_some() && torch.is_some());
            // Cancelled: both hands as they were.
            assert_eq!(press(&mut game), (sword.clone(), None));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(
                g.inventory[selected].as_ref().map(|s| &*s.item),
                Some("minecraft:diamond_sword")
            );
            assert!(g.offhand.is_none());
            // A listener's torch for the off hand: the sword is gone and the
            // client is sent the torch.
            cancel.store(false, Ordering::SeqCst);
            assert_eq!(press(&mut game), (None, torch));
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert!(g.inventory[selected].is_none());
            assert_eq!(
                g.offhand.as_ref().map(|s| &*s.item),
                Some("minecraft:torch")
            );
        });
    }
}

#[cfg(test)]
mod pause_close_tests {
    use crate::game::{Screen, native_entity_refs::tests::with_game};
    use sunrise_protocol::packets::play::Serverbound as S;
    use winit::keyboard::KeyCode;

    /// On a server, Escape out of the inventory closes its menu with a
    /// ServerboundContainerClosePacket; Escape out of the pause menu sends
    /// nothing (PauseScreen.onClose only swaps the screen).
    #[test]
    #[ignore = "daily"]
    fn escape_out_of_the_pause_menu_closes_no_container() {
        with_game(|game| {
            let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
            peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
                .unwrap();
            while peer.recv().is_ok() {}
            for (screen, closes) in [(Screen::Paused, 0), (Screen::Inventory, 1)] {
                game.set_screen(screen);
                game.key_pressed(KeyCode::Escape, KeyCode::Escape).unwrap();
                assert_eq!(game.screen, Screen::Playing);
                let mut sent = 0;
                while let Ok((id, bytes)) = peer.recv() {
                    if matches!(S::decode(id, &bytes), Ok(S::ContainerClose(_))) {
                        sent += 1;
                    }
                }
                assert_eq!(sent, closes, "Escape out of {screen:?}");
            }
        });
    }
}

#[cfg(test)]
mod creative_close_tests {
    use crate::game::{Screen, native_entity_refs::tests::with_game};
    use mod_api::{ClientHost, Hooks, ModEntry, ScreenOpening, ScreenOpeningEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use sunrise_protocol::packets::play::Serverbound as S;
    use winit::keyboard::KeyCode;

    /// Keeps the open screen while `veto` is set.
    struct Veto(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for Veto {
        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            _event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            self.0
                .load(Ordering::SeqCst)
                .then_some(ScreenOpening::Cancel)
        }
    }

    /// On a server, leaving the creative inventory closes its menu, window 0, with one
    /// ServerboundContainerClosePacket (LocalPlayer.closeContainer), by Escape and by the
    /// inventory key; a close a mod vetoes keeps the screen and tells the server nothing.
    #[test]
    #[ignore = "daily"]
    fn leaving_the_creative_inventory_tells_the_server() {
        let veto = Arc::new(AtomicBool::new(true));
        let mods: mod_api::Mods = vec![ModEntry::new("veto", Box::new(Veto(veto.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
                peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
                    .unwrap();
                let closes = |peer: &mut sunrise_protocol::connection::ConnReader| {
                    let mut closed = Vec::new();
                    while let Ok((id, bytes)) = peer.recv() {
                        if let Ok(S::ContainerClose(p)) = S::decode(id, &bytes) {
                            closed.push(p.container);
                        }
                    }
                    closed
                };
                veto.store(false, Ordering::SeqCst);
                game.set_screen(Screen::Creative);
                closes(&mut peer);
                veto.store(true, Ordering::SeqCst);
                game.key_pressed(KeyCode::Escape, KeyCode::Escape).unwrap();
                assert_eq!(
                    game.screen,
                    Screen::Creative,
                    "a vetoed close keeps the screen"
                );
                assert!(closes(&mut peer).is_empty());
                veto.store(false, Ordering::SeqCst);
                for key in [KeyCode::Escape, KeyCode::KeyE] {
                    game.set_screen(Screen::Creative);
                    closes(&mut peer);
                    game.key_pressed(key, key).unwrap();
                    assert_eq!(game.screen, Screen::Playing, "{key:?}");
                    assert_eq!(closes(&mut peer), [0], "{key:?}");
                }
            });
        });
    }
}
