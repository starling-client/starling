//! The `chat_recipient` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Filters one public delivery: a player's line after `server_chat` let it through,
    /// or `/say` or `/me` (signed or not). The hook asks once per placed network player who
    /// accepts player chat, including the sender if eligible, in join order. For a non-player
    /// command source, `sender` is zero and `sender_name` is the source's display name.
    /// `message` is the text after decoration or command expansion, without the chat-type prefix.
    /// Return [`Flow::Cancel`](crate::Flow::Cancel) to omit just this delivery. Other recipients
    /// still have the line, which was logged before the first recipient was asked. A player who
    /// disconnects before their turn is not asked. The veto does not alter the signed body, or add
    /// an index or pending acknowledgement for the omitted copy. Whispers (`/msg`) are not public.
    /// A command run from this hook that sends public chat goes through the same delivery checks,
    /// including this hook, once the running line has been offered to every recipient. A line
    /// that repeats one that led to it (a relay answering its own answer) is not delivered. A panicking listener is reported and is disabled for the session;
    /// the other listeners' decisions still apply.
    /// Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "hidden",
        neoforge = "none (NeoForge has no per-receiver server chat event)",
        cancellable = true,
        event_per_listener = true,
        version = 3
    )]
    fn chat_recipient(
        sender: u128,
        sender_name: &str,
        message: &str,
        recipient: u128,
        hidden: &mut bool,
    );
}
