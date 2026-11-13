//! The `player_name_format` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player's display name was formatted. Replace `displayname` to change it.
    /// Both sides: `context.side()` says which. It runs while a chat line or a command's feedback is built. Answer from the arguments and `context` alone, and fast: `context` holds the side, the registries and the settings of the resource generation, and a `/reload` builds the next one.
    #[hook(
        side = "both",
        contract = "collect",
        out = "displayname",
        context = "loop",
        neoforge = "PlayerEvent.NameFormat",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::lan::hub::native_player_name_format"
    )]
    fn player_name_format();
}
