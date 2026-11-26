//! The `client_chat` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ClientChatEvent`: the play listener is about to send a chat
    /// message, from the chat box or `NetworkClientHost::send_chat`. Edits are
    /// applied before signing. Return `Flow::Cancel` or clear `message` to send
    /// nothing. `original_message` stays unchanged. Does not run for commands. A message a
    /// listener sends with `send_chat` is submitted on the next event-loop wave, so it reaches
    /// this hook after the running one returns; use `message` to replace the running one.
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "ClientChatEvent",
        event_per_listener = true,
        version = 2,
        cancellable = true
    )]
    fn client_chat(
        player_id: i32,
        player_uuid: u128,
        original_message: &str,
        message: &mut String,
        cancel: &mut bool,
    );
}
