//! A wasm mod to check that a phone runs wasm mods on both of the game's threads: the server
//! thread says in the chat every ten seconds how many ticks it ran, and the game's thread marks
//! that line when it reaches the chat.
use mod_api::{
    ClientChatReceivedEvent, ClientHost, Flow, Hooks, ServerHost, ServerTickEvent, Text,
    text::TextContent,
};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Mod;

const SAID: &str = "wasm probe: ";

static SERVER_TICKS: AtomicU64 = AtomicU64::new(0);
static SEEN: AtomicU64 = AtomicU64::new(0);

#[mod_api::sunrise_mod]
impl Hooks for Mod {
    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if !matches!(event.phase, mod_api::ServerTickPhase::ServerTickEnd { .. }) {
            return;
        }
        let ticks = SERVER_TICKS.fetch_add(1, Ordering::Relaxed) + 1;
        if ticks % 200 == 0 {
            host.messages()
                .broadcast(Text::literal(format!("{SAID}{ticks} server ticks")));
        }
    }

    /// In a world only: a chat line comes from its server, so the world's resources are in.
    fn client_chat_received(
        &self,
        _: &mut dyn ClientHost,
        event: &mut ClientChatReceivedEvent<'_>,
    ) -> Flow {
        let TextContent::Literal(text) = &event.message.content else {
            return Flow::Continue;
        };
        if event.system && text.starts_with(SAID) {
            let seen = SEEN.fetch_add(1, Ordering::Relaxed) + 1;
            *event.message = Text::literal(format!("{text}, seen by the game thread ({seen})"));
        }
        Flow::Continue
    }
}
