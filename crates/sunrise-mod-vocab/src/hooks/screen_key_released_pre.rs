//! The `screen_key_released_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0120

pub trait Hooks {
    /// Before the currently open screen handles a named key's release.
    /// On the control socket, a release can reach the current title screen without a matching
    /// press on that screen (for example, if the press closed the world preset editor).
    /// A released modifier no longer contributes to `modifiers` on that route; the bit
    /// stays set only if the other key for the same modifier is still held.
    /// Cancel bypasses screen handling; physical keys and mapped keyboard hold/toggle state release.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_screen_key_released_pre",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyReleased.Pre",
        cancellable = true,
        version = 2
    )]
    fn screen_key_released_pre(
        screen: &ScreenView<'_>,
        key: &str,
        modifiers: u32,
        canceled: &mut bool,
    );
}
