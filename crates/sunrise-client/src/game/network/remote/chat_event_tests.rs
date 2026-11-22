use crate::game::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
use mod_api::Flow;
use sunrise_core::nbt::Tag;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C},
};

struct Censor;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Censor {
    fn client_chat_received(
        &self,
        _: &mut dyn mod_api::ClientHost,
        event: &mut mod_api::ClientChatReceivedEvent<'_>,
    ) -> Flow {
        use mod_api::text::TextContent;
        let text = match &event.message.content {
            TextContent::Literal(text) => text,
            // Player chat arrives decorated with the sender and message.
            TextContent::Translatable { key, arguments, .. } if key == "chat.type.text" => {
                let TextContent::Literal(text) = &arguments[1].content else {
                    panic!("expected literal player message fixture");
                };
                text
            }
            _ => return Flow::Continue,
        };
        if text.contains("drop") {
            return Flow::Cancel;
        }
        // A system line carries no player message, so only it gets the tag.
        if event.system && event.player_message.is_none() {
            *event.message = mod_api::Text::literal(format!("[server] {text}"));
        }
        Flow::Continue
    }
}

/// `ClientChatReceivedEvent`: a mod's replacement is the line the chat
/// shows, a cancelled system line never shows, and a cancelled message
/// held by Chat Delay lets the next one through in the same tick.
#[test]
#[ignore = "daily"]
fn mods_replace_and_cancel_received_chat() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "test-chat-received",
            Box::new(Censor),
        )]
        .into(),
        || {
            with_game(|game| {
                game.options.execution_role = ExecutionRole::Client;
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "ChatTest").unwrap();
                let lines = |game: &crate::game::Game| -> Vec<String> {
                    let chat = &game.remote.as_ref().unwrap().chat;
                    chat.iter().map(|(line, _)| line.clone()).collect()
                };
                let system = |content: &str| {
                    C::SystemChat(play::SystemChat {
                        content: Tag::String(content.into()),
                        overlay: false,
                    })
                };
                game.handle_packet(system("hello")).unwrap();
                game.handle_packet(system("drop me")).unwrap();
                assert_eq!(lines(game), ["[server] hello"]);

                game.settings.chat_delay = 1.0;
                let disguised = |message: &str| {
                    C::DisguisedChat(play::DisguisedChat {
                        message: Tag::String(message.into()),
                        chat_type: play::ChatTypeBound {
                            chat_type: 0,
                            name: Tag::String("Steve".into()),
                            target: None,
                        },
                    })
                };
                game.handle_packet(disguised("drop this")).unwrap();
                game.handle_packet(disguised("hi")).unwrap();
                assert_eq!(lines(game).len(), 1, "Chat Delay holds both");
                game.tick_chat_queue();
                let shown = lines(game);
                assert_eq!(shown.len(), 2, "{shown:?}");
                assert!(
                    shown[1].contains("hi") && !shown[1].contains("drop"),
                    "{shown:?}"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn mod_chat_needs_a_joined_player_and_what_the_chat_option_allows() {
    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_err());
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "ChatTest").unwrap();
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_err());
        game.remote.as_mut().unwrap().joined = true;
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "  ").is_err());
        // Full, then Commands Only, then Hidden.
        game.settings.chat_visibility = 0;
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "hello").is_ok());
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_ok());
        game.settings.chat_visibility = 1;
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "hello").is_err());
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_ok());
        game.settings.chat_visibility = 2;
        assert!(mod_api::prelude::NetworkClientHost::send_chat(game, "/warp spawn").is_err());
    });
}
