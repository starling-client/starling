//! A mod presses vanilla gameplay mappings with `VanillaKeyMappingRef::set_down` and
//! `VanillaKeyMappingRef::click`. The client tick's key step acts on them with its checks
//! and sends the packets the player's own keys send.
use super::super::super::Screen;
use super::*;
use mod_api::prelude::ClientInputClientHost;
use mod_api::{
    ClientHost, InteractionKeyMappingTriggeredEvent, MappingError, VanillaKeyMappingRef,
};
use std::sync::OnceLock;

#[path = "input_burst_tests.rs"]
mod input_burst_tests;

/// The packets the client sends, and the hands the interaction hook hears.
struct Heard {
    packets: Arc<Mutex<Vec<String>>>,
    triggered: Arc<Mutex<Vec<Hand>>>,
    click_in_callback: Arc<std::sync::atomic::AtomicBool>,
    attack_mapping: Arc<OnceLock<VanillaKeyMappingRef>>,
}
#[mod_api::__subscriptions]
impl Hooks for Heard {
    fn client_packet_send(
        &self,
        _: &dyn ClientHost,
        e: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> Flow {
        self.packets.lock().unwrap().push(e.packet.id.to_string());
        Flow::Continue
    }
    fn interaction_key_mapping_triggered(
        &self,
        host: &mut dyn ClientHost,
        e: &mut InteractionKeyMappingTriggeredEvent,
    ) -> Flow {
        self.triggered.lock().unwrap().push(e.hand);
        if self
            .click_in_callback
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            self.attack_mapping.get().unwrap().click(host).unwrap();
        }
        Flow::Continue
    }
}

/// A survival player on a server's client, playing with nothing open, the tick's first packets
/// (the held slot) sent: `test` gets the game, the packets sent since and the hands triggered.
fn with_pressing_game(test: impl FnOnce(&mut Game<'_>, &Mutex<Vec<String>>, &Mutex<Vec<Hand>>)) {
    with_callback_game(false, |game, packets, triggered, _| {
        test(game, packets, triggered);
    });
}
fn with_callback_game(
    click: bool,
    test: impl FnOnce(
        &mut Game<'_>,
        &Mutex<Vec<String>>,
        &Mutex<Vec<Hand>>,
        Option<VanillaKeyMappingRef>,
    ),
) {
    let packets = Arc::new(Mutex::new(Vec::new()));
    let triggered = Arc::new(Mutex::new(Vec::new()));
    let attack_mapping = Arc::new(OnceLock::new());
    let mods = vec![mod_api::ModEntry::new(
        "presses",
        Box::new(Heard {
            packets: packets.clone(),
            triggered: triggered.clone(),
            click_in_callback: Arc::new(std::sync::atomic::AtomicBool::new(click)),
            attack_mapping: attack_mapping.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            game.set_local_mode(sunrise_client::command::args::GameMode::Survival);
            game.title = None;
            game.screen = Screen::Playing;
            game.remote.as_mut().unwrap().loaded_sent = true;
            let attack = click.then(|| {
                let attack = game.vanilla_key_mapping("key.attack").unwrap();
                attack_mapping.set(attack).unwrap();
                attack
            });
            // The tick's first packet, the held slot (`MultiPlayerGameMode.tick`), then its keys.
            game.remote_send_carried();
            game.remote_handle_input();
            packets.lock().unwrap().clear();
            test(game, &packets, &triggered, attack);
        })
    });
}

/// What was sent since the last call.
fn sent(packets: &Mutex<Vec<String>>) -> Vec<String> {
    std::mem::take(&mut *packets.lock().unwrap())
}

fn stone_under_the_crosshair(game: &mut Game<'_>) {
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(0, 64, 0, stone).unwrap();
    game.forced_hit = Some(hit());
}

/// Holding use with a bow drawn starts one use and repeats nothing while the bow is in use;
/// letting go releases it, as letting go of the key does.
#[test]
#[ignore = "daily"]
fn holding_use_draws_a_bow_once_and_letting_go_releases_it() {
    with_pressing_game(|game, packets, triggered| {
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:bow", 1));
        let use_mapping = game.vanilla_key_mapping("key.use").unwrap();
        game.inventory[9] = Some(ItemStack::new("minecraft:arrow", 64));
        let sequence = game.remote.as_ref().unwrap().sequence;
        assert_eq!(use_mapping.set_down(game, true), Ok(()));
        assert!(game.mapped_key_down("key.use"));
        assert_eq!(
            sent(packets),
            Vec::<String>::new(),
            "nothing before the tick"
        );
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:use_item"]);
        assert_eq!(game.remote.as_ref().unwrap().sequence - sequence, 1);
        assert!(game.using.is_some());
        assert_eq!(*triggered.lock().unwrap(), [Hand::Main]);
        for _ in 0..5 {
            game.use_delay = 0;
            game.remote_handle_input();
        }
        assert_eq!(
            sent(packets),
            Vec::<String>::new(),
            "no repeat while drawing"
        );
        assert_eq!(use_mapping.set_down(game, false), Ok(()));
        assert!(!game.mapped_key_down("key.use"));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:player_action"]);
        assert!(game.using.is_none());
        assert_eq!(*triggered.lock().unwrap(), [Hand::Main]);
    });
}

/// A held use key uses again when `Minecraft.rightClickDelay` reads 0: each use sets it to 4,
/// and a mod's `set_right_click_delay(0)` lets the next key step use again at once. Outside a
/// client world there is no delay to read or set.
#[test]
#[ignore = "daily"]
fn a_mod_shortens_the_right_click_delay_of_a_held_use() {
    with_interaction_game(false, |game| {
        assert_eq!(game.right_click_delay(), None);
        assert!(!game.set_right_click_delay(0));
    });
    with_pressing_game(|game, packets, _| {
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:snowball", 16));
        let use_mapping = game.vanilla_key_mapping("key.use").unwrap();
        assert_eq!(use_mapping.set_down(game, true), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:use_item"]);
        assert_eq!(game.right_click_delay(), Some(4));
        // Held, the key waits out the delay the use set.
        game.remote_handle_input();
        assert_eq!(sent(packets), Vec::<String>::new());
        // A mod's 0 lets the held key use again on the next key step, which sets 4 again.
        assert!(game.set_right_click_delay(0));
        assert_eq!(game.right_click_delay(), Some(0));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:use_item"]);
        assert_eq!(game.right_click_delay(), Some(4));
        // A longer delay holds the key back as well.
        assert!(game.set_right_click_delay(20));
        game.remote_handle_input();
        assert_eq!(sent(packets), Vec::<String>::new());
        assert_eq!(game.right_click_delay(), Some(20));
    });
}

/// A click on a block in reach starts the break with a prediction sequence, then the punch; with
/// the key not held, the same tick's `continueAttack(false)` stops it, as a tap between two ticks
/// does. Held, the break goes on, punching each tick, and the release stops it.
#[test]
#[ignore = "daily"]
fn clicking_attack_on_a_block_starts_the_break_and_punches() {
    with_pressing_game(|game, packets, triggered| {
        let attack_mapping = game.vanilla_key_mapping("key.attack").unwrap();
        stone_under_the_crosshair(game);
        let sequence = game.remote.as_ref().unwrap().sequence;
        assert_eq!(attack_mapping.click(game), Ok(()));
        game.remote_handle_input();
        assert_eq!(
            sent(packets),
            [
                "minecraft:player_action",
                "minecraft:punch",
                "minecraft:player_action"
            ]
        );
        assert_eq!(game.remote.as_ref().unwrap().sequence - sequence, 1);
        assert_eq!(*triggered.lock().unwrap(), [Hand::Main]);

        let sequence = game.remote.as_ref().unwrap().sequence;
        assert_eq!(attack_mapping.set_down(game, true), Ok(()));
        assert_eq!(attack_mapping.click(game), Ok(()));
        game.remote_handle_input();
        // The same tick's `continueAttack(true)` digs on and punches again, as a held press does.
        assert_eq!(
            sent(packets),
            [
                "minecraft:player_action",
                "minecraft:punch",
                "minecraft:punch"
            ]
        );
        assert_eq!(game.remote.as_ref().unwrap().sequence - sequence, 1);
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:punch"], "the held key digs on");
        assert_eq!(attack_mapping.set_down(game, false), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:player_action"]);
    });
}

/// A click on nothing punches and sets the miss cooldown (not in creative); a second click inside
/// it does nothing.
#[test]
#[ignore = "daily"]
fn clicking_attack_on_nothing_punches_and_waits_out_the_miss_cooldown() {
    with_pressing_game(|game, packets, _| {
        let attack_mapping = game.vanilla_key_mapping("key.attack").unwrap();
        game.forced_hit = None;
        assert_eq!(attack_mapping.set_down(game, true), Ok(()));
        assert_eq!(attack_mapping.click(game), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:punch"]);
        // 10, less the tick's own count down after `handleKeybinds`.
        assert_eq!(game.clicks.miss_time, 9);
        assert_eq!(attack_mapping.click(game), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), Vec::<String>::new(), "inside the cooldown");
        assert_eq!(game.clicks.miss_time, 8);

        game.clicks.miss_time = 0;
        game.set_local_mode(sunrise_client::command::args::GameMode::Creative);
        assert_eq!(attack_mapping.click(game), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:punch"]);
        assert_eq!(game.clicks.miss_time, 0, "no cooldown in creative");
    });
}

/// A screen does not poll mapping clicks, and its opening releases pending plain key clicks.
#[test]
#[ignore = "daily"]
fn a_screen_drops_the_clicks_and_holds_nothing() {
    with_pressing_game(|game, packets, triggered| {
        let attack = game.vanilla_key_mapping("key.attack").unwrap();
        let pick = game.vanilla_key_mapping("key.pickItem").unwrap();
        stone_under_the_crosshair(game);
        pick.click(game).unwrap();
        game.set_screen(Screen::Inventory);
        assert_eq!(game.pending_mapping_clicks("key.pickItem"), 0);
        pick.click(game).unwrap();
        attack.set_down(game, true).unwrap();
        game.remote_handle_input();
        assert!(sent(packets).is_empty());
        assert!(triggered.lock().unwrap().is_empty());
        assert_eq!(game.pending_mapping_clicks("key.pickItem"), 1);
        assert_eq!(attack.is_down(game), Ok(true));
    });
}

/// A resolved hotbar mapping selects its own slot on the next tick even after rebinding.
/// Unknown names refuse resolution, every resolved vanilla mapping accepts `is_down`, `set_down`
/// and `click` under `ClientSetup` access (with or without a world), and unavailable phases
/// refuse without acting.
#[test]
#[ignore = "daily"]
fn the_host_presses_by_name_and_refuses_the_others() {
    with_pressing_game(|game, packets, _| {
        let hotbar = ClientInputClientHost::vanilla_key_mapping(game, "key.hotbar.3").unwrap();
        game.settings.keys.set("key.hotbar.3", "key.keyboard.4");
        assert_eq!(hotbar.click(game), Ok(()));
        game.remote_handle_input();
        assert_eq!(game.selected, 2);
        assert_eq!(sent(packets), Vec::<String>::new());
        // `MultiPlayerGameMode.tick` tells the server first thing in the next tick.
        game.remote_send_carried();
        assert_eq!(sent(packets), ["minecraft:set_carried_item"]);

        for key in ["nope", ""] {
            assert_eq!(
                ClientInputClientHost::vanilla_key_mapping(game, key),
                Err(MappingError::Unknown { key: key.into() })
            );
        }
        // Every resolved vanilla mapping supports uniform `is_down`, `set_down` and `click`.
        for key in [
            "key.screenshot",
            "key.fullscreen",
            "key.chat",
            "key.spectatorHotbar",
            "key.pickItem",
        ] {
            let mapping = game.vanilla_key_mapping(key).unwrap();
            assert_eq!(mapping.is_down(game), Ok(false));
            assert_eq!(mapping.set_down(game, true), Ok(()));
            assert_eq!(mapping.is_down(game), Ok(true));
            assert_eq!(mapping.set_down(game, false), Ok(()));
            assert_eq!(mapping.is_down(game), Ok(false));
            assert_eq!(mapping.click(game), Ok(()));
            if key == "key.chat" {
                assert_eq!(game.screen, Screen::Playing);
                game.remote_handle_input();
                assert_eq!(game.screen, Screen::Chat);
                game.set_screen(Screen::Playing);
            }
        }

        let forward = game.vanilla_key_mapping("key.forward").unwrap();
        assert_eq!(forward.is_down(game), Ok(false));
        assert_eq!(game.pending_mapping_clicks("key.hotbar.3"), 0);
        // An open screen suppresses polling, not changes to the logical mappings.
        game.player = None;
        game.title = Some(game.new_title(game.save.root.clone()));
        assert!(game.mod_mappings_screen_open());
        let hotbar_slot = game.vanilla_mapping_slot(hotbar).unwrap();
        let before_clicks = game.vanilla_mappings.pending_clicks(hotbar_slot);
        {
            assert_eq!(forward.set_down(game, true), Ok(()));
            assert_eq!(forward.is_down(game), Ok(true));
            assert_eq!(forward.set_down(game, false), Ok(()));
            assert_eq!(forward.is_down(game), Ok(false));
            assert_eq!(hotbar.click(game), Ok(()));
        }
        assert_eq!(
            game.vanilla_mappings.pending_clicks(hotbar_slot),
            before_clicks + 1
        );
        assert!(game.vanilla_mappings.consume_click(hotbar_slot));
    });
}

#[test]
#[ignore = "daily"]
fn a_callback_click_survives_until_the_next_tick() {
    with_callback_game(true, |game, _, triggered, attack| {
        stone_under_the_crosshair(game);
        attack.unwrap().click(game).unwrap();
        game.remote_handle_input();
        assert_eq!(triggered.lock().unwrap().len(), 1);
        // The callback's second click is not processed recursively in this tick.
        game.remote_handle_input();
        assert_eq!(triggered.lock().unwrap().len(), 2);
        game.remote_handle_input();
        assert_eq!(triggered.lock().unwrap().len(), 2);
    });
}

#[test]
#[ignore = "daily"]
fn opening_a_screen_releases_mod_holds() {
    with_pressing_game(|game, _, _| {
        use sunrise_client::held::Source;
        use winit::event::MouseButton;
        use winit::keyboard::{KeyCode, ModifiersState};

        let forward_mapping = game.vanilla_key_mapping("key.forward").unwrap();
        let sneak_mapping = game.vanilla_key_mapping("key.sneak").unwrap();
        let attack_mapping = game.vanilla_key_mapping("key.attack").unwrap();
        let use_mapping = game.vanilla_key_mapping("key.use").unwrap();
        game.mapped_key_event(KeyCode::KeyW, true, false).unwrap();
        assert!(forward_mapping.is_down(game).unwrap());
        forward_mapping.set_down(game, false).unwrap();
        assert!(!forward_mapping.is_down(game).unwrap());
        forward_mapping.set_down(game, true).unwrap();
        game.mapped_key_event(KeyCode::KeyW, false, false).unwrap();
        assert!(!forward_mapping.is_down(game).unwrap());
        forward_mapping.set_down(game, true).unwrap();
        attack_mapping.set_down(game, true).unwrap();
        use_mapping.set_down(game, true).unwrap();
        assert!(game.mapped_key_down("key.attack") && game.mapped_key_down("key.use"));
        game.physical_buttons
            .extend([MouseButton::Left, MouseButton::Right]);
        game.set_screen(Screen::Inventory);
        // No redraw/reconciliation before the screen closes. Controller releases
        // forwarded for cleanup cannot give the hold back to the mouse in a screen.
        for button in [MouseButton::Left, MouseButton::Right] {
            game.mouse_mapping(button, false, Source::Pad).unwrap();
        }
        game.set_screen(Screen::Playing);
        assert!(!game.mapped_key_down("key.attack") && !game.mapped_key_down("key.use"));
        assert!(!forward_mapping.is_down(game).unwrap());

        // ModifiersChanged can contain an unrelated change, or be the OS's only
        // report of a real release. Only the recovered sneak release takes over.
        game.settings.toggle_crouch = false;
        game.physical_keys.insert(KeyCode::ShiftLeft);
        game.update_modifiers(ModifiersState::SHIFT | ModifiersState::ALT);
        game.mapped_key_event(KeyCode::ShiftLeft, true, false)
            .unwrap();
        sneak_mapping.set_down(game, true).unwrap();
        game.update_modifiers(ModifiersState::SHIFT);
        game.heal_hold_latches();
        assert!(sneak_mapping.is_down(game).unwrap());
        game.update_modifiers(ModifiersState::empty());
        assert!(!sneak_mapping.is_down(game).unwrap());
    });
}

/// Q in creative mode: `MultiPlayerGameMode.dropItem` sends the drop and nothing else, so the
/// end of the tick sends no creative slot edit for the stack it left (only the creative screen
/// sends those).
#[test]
#[ignore = "daily"]
fn a_creative_drop_sends_no_creative_slot() {
    with_pressing_game(|game, packets, _| {
        game.set_local_mode(sunrise_client::command::args::GameMode::Creative);
        game.forced_hit = None;
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 16));
        // The setup's stack is the server's as well.
        game.remote_synced();
        game.key_pressed(
            winit::keyboard::KeyCode::KeyQ,
            winit::keyboard::KeyCode::KeyQ,
        )
        .unwrap();
        game.remote_handle_input();
        game.remote_send_state(0);
        let sent = sent(packets);
        assert!(
            sent.iter().any(|p| p == "minecraft:player_action"),
            "{sent:?}"
        );
        assert!(
            !sent.iter().any(|p| p == "minecraft:set_creative_mode_slot"),
            "{sent:?}"
        );
        assert_eq!(
            game.inventory[game.selected].as_ref().map(|s| s.count),
            Some(15)
        );
    });
}

/// A creative toolbar load and Q in one tick (`handleKeybinds`: the number keys' loop, then
/// key.drop): the load sends the new stack at once (`handleCreativeModeItemAdd`), so the drop
/// that follows drops what the server now holds, and the end of the tick sends nothing more.
#[test]
#[ignore = "daily"]
fn a_toolbar_load_goes_out_before_a_drop_in_its_tick() {
    with_pressing_game(|game, packets, _| {
        game.set_local_mode(sunrise_client::command::args::GameMode::Creative);
        game.forced_hit = None;
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 16));
        game.remote_synced();
        // handleHotbarLoadOrSave puts the saved stack in the slot and sends it.
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:dirt", 16));
        game.remote_creative_slot_add(game.selected);
        game.key_pressed(
            winit::keyboard::KeyCode::KeyQ,
            winit::keyboard::KeyCode::KeyQ,
        )
        .unwrap();
        game.remote_handle_input();
        game.remote_send_state(0);
        let sent = sent(packets);
        let at = |id: &str| sent.iter().position(|p| p == id);
        assert_eq!(
            sent.iter()
                .filter(|p| *p == "minecraft:set_creative_mode_slot")
                .count(),
            1,
            "{sent:?}"
        );
        assert!(
            at("minecraft:set_creative_mode_slot") < at("minecraft:player_action"),
            "{sent:?}"
        );
        assert_eq!(
            game.inventory[game.selected]
                .as_ref()
                .map(|s| (s.item.as_ref(), s.count)),
            Some(("minecraft:dirt", 15))
        );
    });
}

/// `MultiPlayerGameMode.tick` comes first in `Minecraft.tick`, before the GUI ticks its
/// screens: a slot chosen since the last tick goes out ahead of what a screen sends as it
/// closes in that tick (a sign editor whose sign is gone, `AbstractSignEditScreen.removed`).
#[test]
#[ignore = "daily"]
fn the_held_slot_goes_out_before_a_closing_screens_packet() {
    with_pressing_game(|game, packets, _| {
        game.open_sign_editor((0, 64, 40), true);
        sent(packets);
        game.selected = (game.selected + 1) % 9;
        game.tick_remote_level().unwrap();
        let sent = sent(packets);
        let at = |id: &str| sent.iter().position(|p| p == id);
        assert!(at("minecraft:sign_update").is_some(), "{sent:?}");
        assert!(at("minecraft:set_carried_item").is_some(), "{sent:?}");
        assert!(
            at("minecraft:set_carried_item") < at("minecraft:sign_update"),
            "{sent:?}"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_mod_releases_the_toolbar_mapping_while_its_physical_key_stays_down() {
    with_pressing_game(|game, _, _| {
        use winit::keyboard::KeyCode;
        let mapping = game
            .vanilla_key_mapping("key.saveToolbarActivator")
            .unwrap();
        game.physical_keys.insert(KeyCode::KeyC);
        game.mapped_key_event(KeyCode::KeyC, true, false).unwrap();
        assert!(game.mapped_key_down("key.saveToolbarActivator"));
        mapping.set_down(game, false).unwrap();
        assert_eq!(mapping.physical_down(game), Ok(true));
        assert_eq!(mapping.is_down(game), Ok(false));
        assert!(!game.mapped_key_down("key.saveToolbarActivator"));
    });
}
