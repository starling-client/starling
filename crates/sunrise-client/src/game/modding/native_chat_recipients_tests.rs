//! `chat_recipient`: asked once for each placed receiver before a public line is sent, and a
//! receiver it hides is sent nothing while the others still get the line.
use super::native_entity_refs::tests::with_game;
use mod_api::{ChatRecipientEvent, CommandResult, Flow, Hooks, PermissionLevel, ServerHost};
use std::sync::{Arc, Mutex, atomic::AtomicI32, mpsc};
use sunrise_client::lan::hub::chat::ChatMessage;
use sunrise_client::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest};
use sunrise_client::text::Text;
use sunrise_core::position::{BlockPos, ChunkPos};
use sunrise_protocol::packets::{configuration::ClientInformation, play};

type Asked = Arc<Mutex<Vec<(u128, String, String, u128)>>>;

/// Hides every line that says "secret" from guest 13, and records what it is asked.
struct Ignorer(Asked);

#[mod_api::__subscriptions]
impl Hooks for Ignorer {
    fn chat_recipient(
        &self,
        _host: &mut dyn ServerHost,
        event: &mut ChatRecipientEvent<'_>,
    ) -> Flow {
        self.0.lock().unwrap().push((
            event.sender,
            event.sender_name.to_owned(),
            event.message.to_owned(),
            event.recipient,
        ));
        if event.message.contains("secret") && event.recipient == 13 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

struct Echo {
    asked: Asked,
    nested: Arc<Mutex<Option<CommandResult>>>,
}

#[mod_api::__subscriptions]
impl Hooks for Echo {
    fn chat_recipient(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ChatRecipientEvent<'_>,
    ) -> Flow {
        self.asked.lock().unwrap().push((
            event.sender,
            event.sender_name.to_owned(),
            event.message.to_owned(),
            event.recipient,
        ));
        // Only retry the outer message so a missing guard fails without overflowing the stack.
        if event.message == "outer" && event.recipient == 12 {
            let result = host
                .commands()
                .run_command_with_output("execute run say nested");
            *self.nested.lock().unwrap() = Some(result);
        }
        Flow::Continue
    }
}

/// Commands run by a listener must not contribute to the block's result callback.
struct Queries(Arc<Mutex<Vec<bool>>>);

#[mod_api::__subscriptions]
impl Hooks for Queries {
    fn chat_recipient(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ChatRecipientEvent<'_>,
    ) -> Flow {
        if event.recipient == 12 {
            let console = host.commands().run_command("time query gametime");
            let player = host.players().player_by_name("guest12").unwrap();
            let entity = player.entity().run_command(
                host,
                "time query gametime",
                PermissionLevel::from_level(2),
            );
            self.0.lock().unwrap().extend([console, entity]);
        }
        Flow::Continue
    }
}

/// Guests 12 (the sender) and 13 on a LAN host, with what each is sent.
fn two_guests(game: &mut super::Game<'_>) -> (mpsc::Receiver<Out>, mpsc::Receiver<Out>) {
    let net = NetConfig {
        listen: false,
        announce: false,
        lod_distance: 0,
        ..NetConfig::lan(0, String::new(), 3)
    };
    let config = sunrise_client::lan::hub::HubConfig {
        default_game_mode: 0,
        guest_commands: true,
        force_game_mode: false,
        hardcore: false,
        view_distance: 2,
        simulation_distance: 2,
        dimension: game.dim.id().to_owned(),
        allow_flight: true,
        seed: 0,
        max_players: 3,
        integrated: true,
        levels: game.level_shapes,
        level_seeds: game.levels.runtime.seeds(),
    };
    let settings = super::lan::LanSettings {
        port: 0,
        guest_commands: true,
        force_game_mode: false,
    };
    game.lan_host(settings, net, config).unwrap();
    let mut join = |conn: u32| {
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: format!("guest{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            [0.5, 64.0, 0.5],
            2,
        );
        guest.placed = true;
        guest.permission = 4;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        rx
    };
    (join(12), join(13))
}

fn packets(rx: &mpsc::Receiver<Out>) -> impl Iterator<Item = play::Clientbound> + '_ {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            play::Clientbound::decode(id, reader.rest()).ok()
        })
}

/// How many public chat lines (signed or disguised) a guest was sent since the last call.
fn lines(rx: &mpsc::Receiver<Out>) -> usize {
    packets(rx)
        .filter(|packet| {
            matches!(
                packet,
                play::Clientbound::PlayerChat(_) | play::Clientbound::DisguisedChat(_)
            )
        })
        .count()
}

fn signed(content: &str) -> ChatMessage {
    ChatMessage {
        sender: 12,
        index: 0,
        signature: None,
        content: content.to_owned(),
        timestamp: 0,
        salt: 0,
        last_seen: Vec::new(),
        unsigned_content: None,
    }
}

/// A player's line: asked once per receiver, the sender included, with the text players see; the
/// hidden receiver is sent nothing and the others still are; a line nobody hides reaches all.
#[test]
#[ignore = "daily"]
fn a_chat_line_hidden_from_one_receiver_reaches_the_others() {
    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "ignorer",
        Box::new(Ignorer(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (sender, other) = two_guests(game);
            lines(&sender);
            lines(&other);
            let bound = play::ChatTypeBound {
                chat_type: 0,
                name: sunrise_client::lan::data::component_nbt(
                    &Text::literal("guest12").component(),
                ),
                target: None,
            };
            let say = |game: &mut super::Game<'_>, content: &str| {
                let line = format!("<guest12> {content}");
                game.lan_broadcast_chat(&signed(content), &bound, line, "guest12", content)
                    .unwrap();
            };
            say(game, "a secret");
            assert_eq!(lines(&sender), 1, "the sender still sees it");
            assert_eq!(lines(&other), 0, "the hidden receiver is not sent it");
            assert_eq!(
                std::mem::take(&mut *asked.lock().unwrap()),
                [
                    (12, "guest12".to_owned(), "a secret".to_owned(), 12),
                    (12, "guest12".to_owned(), "a secret".to_owned(), 13),
                ],
                "asked once per receiver, the sender included"
            );
            say(game, "hello");
            assert_eq!(lines(&sender), 1);
            assert_eq!(lines(&other), 1, "a line no mod hides reaches everyone");
        });
    });
}

/// `/say` and `/me` are public too, signed or not: the hidden receiver is not sent them either,
/// and `/msg` is not asked about at all.
#[test]
#[ignore = "daily"]
fn say_and_me_are_asked_per_receiver_and_a_whisper_is_not() {
    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "ignorer",
        Box::new(Ignorer(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (sender, other) = two_guests(game);
            lines(&sender);
            lines(&other);
            assert!(game.guest_message_command(12, "say a secret", None));
            assert_eq!(lines(&sender), 1);
            assert_eq!(lines(&other), 0, "/say, unsigned: hidden from guest 13");
            assert!(game.guest_message_command(12, "me waves", None));
            assert_eq!(lines(&sender), 1);
            assert_eq!(lines(&other), 1, "/me: nothing hides it");
            assert_eq!(asked.lock().unwrap().len(), 4, "two lines, two receivers");
            asked.lock().unwrap().clear();
            assert!(game.guest_message_command(12, "msg guest13 a secret", None));
            assert!(asked.lock().unwrap().is_empty(), "a whisper is not public");
            assert_eq!(lines(&other), 1, "and it reaches its target");
        });
    });
}

fn place_say_block(game: &mut super::Game<'_>) -> BlockPos {
    game.build_registries();
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.as_console(|game| game.run_command(
        r#"setblock 5 100 2 command_block[facing=up]{Command:"say block secret",CustomName:"Announcer",conditionMet:1b}"#,
    )));
    BlockPos::new(5, 100, 2)
}

#[test]
#[ignore = "daily"]
fn console_and_command_block_say_can_be_hidden_per_receiver() {
    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "ignorer",
        Box::new(Ignorer(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (first, second) = two_guests(game);
            let block = place_say_block(game);
            let level = game.level(game.dim.id()).unwrap();
            game.fake_player(level, 14, "robot").unwrap();
            lines(&first);
            lines(&second);

            game.dedicated_console_say("console secret".into());
            assert_eq!(lines(&first), 1);
            assert_eq!(lines(&second), 0);
            game.command_block_tick(block);
            assert_eq!(lines(&first), 1);
            assert_eq!(lines(&second), 0);
            game.lan.as_mut().unwrap().hub.update_guest(13, |guest| {
                guest.info.chat_visibility = 1; // Commands Only
            });
            game.dedicated_console_say("public announcement".into());
            assert_eq!(lines(&first), 1);
            assert_eq!(lines(&second), 0, "Commands Only does not show /say");
            assert_eq!(
                *asked.lock().unwrap(),
                [
                    ("Server", "console secret", 12),
                    ("Server", "console secret", 13),
                    ("Announcer", "block secret", 12),
                    ("Announcer", "block secret", 13),
                    ("Server", "public announcement", 12),
                    ("Server", "public announcement", 13),
                ]
                .map(|(name, message, receiver)| (
                    0,
                    name.to_owned(),
                    message.to_owned(),
                    receiver
                )),
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn dispatcher_say_filters_the_acting_guests_captured_copy() {
    use super::lan_actions::GuestCommand;
    use play::Clientbound as C;
    use sunrise_client::scoreboard::{comp_text, tag_to_json};

    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "ignorer",
        Box::new(Ignorer(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (peer, actor) = two_guests(game);
            let chat = |rx| {
                packets(rx)
                    .filter_map(|packet| match packet {
                        C::DisguisedChat(p) => Some((false, p.message)),
                        C::SystemChat(p) => Some((true, p.content)),
                        _ => None,
                    })
                    .map(|(system, tag)| (system, comp_text(&tag_to_json(&tag)).plain()))
                    .collect::<Vec<_>>()
            };
            chat(&peer);
            chat(&actor);
            for (message, should_capture) in [("a secret", false), ("hello", true)] {
                assert!(game.run_guest_command(GuestCommand {
                    conn: 13,
                    command: format!("execute run say {message}"),
                    permission: 4,
                    integrated: true,
                    quiet: false,
                }));
                assert_eq!(chat(&peer), [(false, message.to_owned())]);
                let actor_lines = chat(&actor);
                if should_capture {
                    assert_eq!(actor_lines, [(true, "[guest13] hello".to_owned())]);
                } else {
                    assert!(
                        actor_lines.is_empty(),
                        "the veto also hides captured output: {actor_lines:?}"
                    );
                }
            }
            assert_eq!(
                *asked.lock().unwrap(),
                [
                    ("a secret", 12),
                    ("a secret", 13),
                    ("hello", 12),
                    ("hello", 13)
                ]
                .map(|(message, recipient)| (
                    13,
                    "guest13".to_owned(),
                    message.to_owned(),
                    recipient
                )),
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn listener_commands_do_not_count_as_command_block_successes() {
    let results = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "queries",
        Box::new(Queries(results.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let _receivers = two_guests(game);
            game.reset_entity_residency();
            let pos = place_say_block(game);
            game.command_block_tick(pos);
            assert_eq!(
                *results.lock().unwrap(),
                [true, true],
                "both listener queries ran"
            );
            let sunrise_client::containers::BlockEntity::CommandBlock(block) =
                &game.world.block_entities[&(pos.x, pos.y, pos.z)]
            else {
                panic!("expected the command block");
            };
            assert_eq!(block.success_count, 1, "only the block's /say counts");
        });
    });
}

#[test]
#[ignore = "daily"]
fn nested_say_fails_without_sending_an_unchecked_line() {
    let asked = Asked::default();
    let nested = Arc::new(Mutex::new(None));
    let mods = vec![mod_api::ModEntry::new(
        "echo",
        Box::new(Echo {
            asked: asked.clone(),
            nested: nested.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (first, second) = two_guests(game);
            lines(&first);
            lines(&second);
            game.dedicated_console_say("outer".into());
            let result = nested.lock().unwrap().take().unwrap();
            assert!(!result.success, "nested /say must fail");
            assert_eq!(result.feedback.len(), 1, "the command reports its failure");
            assert_eq!(lines(&first), 1);
            assert_eq!(lines(&second), 1);
            assert_eq!(
                *asked.lock().unwrap(),
                [12, 13].map(|recipient| (0, "Server".to_owned(), "outer".to_owned(), recipient)),
                "both delivered copies were asked; no nested copy was asked or sent",
            );
        });
    });
}
