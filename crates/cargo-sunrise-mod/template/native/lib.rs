//! The native mod: `cargo sunrise-mod build` compiles this crate against the pinned game, which calls `hooks()`.
//! Each method of `Hooks` is one moment in the game; implement the ones the mod needs
//! (docs/modding/src/start/writing-hooks.md). Need a moment no hook covers? See
//! docs/modding/src/concepts/native-route.md.
use mod_api::{self, Hooks, PlayerLoggedInEvent, ServerHost, Text};

#[derive(Default)]
struct Mod;

#[mod_api::sunrise_mod]
impl Hooks for Mod {
    /// A player joined the server (NeoForge's `PlayerLoggedInEvent`): greet them.
    fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
        if let Some(player) = host.player(event.player) {
            let _ = player.tell(host, Text::literal(greeting(event.name)));
        }
    }
}

fn greeting(player: &str) -> String {
    format!("{id} says hello, {player}!")
}

// `cargo test` runs this without the game: keep the logic a hook uses in plain functions.
#[cfg(test)]
mod tests {
    #[test]
    fn greets_the_player_by_name() {
        assert_eq!(super::greeting("Alex"), "{id} says hello, Alex!");
    }
}
