//! The `chat_recipient` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A public chat line from `sender` (`sender_name`) is about to reach the placed player
    /// `recipient`: a player's chat after `server_chat` let it through, or a `/say` or `/me`
    /// (signed or not). For a non-player `/say` source such as the console or a command block,
    /// `sender` is the nil UUID (`0`) and `sender_name` is the source's display name.
    /// `message` is the text players see: what `server_chat` left, or the command's text.
    /// Set `hidden` and that one receiver is not sent the line at all, no chat index and no
    /// pending acknowledgement. The others still get it, signed as it was sent, so its signature
    /// still verifies for them. The hook is asked once for each placed receiver, the sender
    /// included, before any of them is sent the line, in the order the hub lists them.
    /// Whispers (`/msg`) are not public and are not heard here, and neither is the line the
    /// server's log keeps: hiding is per receiver. This is not a second `server_chat`: that
    /// hook is heard once for a line, only for typed chat, and cancels it for everyone; this one
    /// is per receiver, and covers `/say` and `/me`. With no listener nothing is asked and
    /// nothing is allocated. A `/say` requested while this hook is running fails rather than
    /// reentering the hook or sending an unchecked line. No player's state is swapped in.
    /// Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "hidden",
        neoforge = "none (NeoForge has no per-receiver server chat event)",
        cancellable = true
    )]
    fn chat_recipient(
        sender: u128,
        sender_name: &str,
        message: &str,
        recipient: u128,
        hidden: &mut bool,
    );
}
