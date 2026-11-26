//! The `screen_input_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mouse press after the receiving screen handled or declined it
    /// (`ScreenEvent.MouseButtonPressed.Post`). The matching Pre must not have cancelled.
    /// `screen`, `button`, `modifiers` and `double_click` are the press Pre values, even if
    /// the press closed or replaced that screen. Every listener sees `was_click_handled`,
    /// the receiving handler's original answer. A hit on the first child counts as handled
    /// even when that child declines and does not take focus.
    ///
    /// Listeners share `result`: `TriState::Default` keeps that answer, `TriState::True` forces
    /// handled and `TriState::False` forces unhandled. The final answer controls double-click
    /// history and whether the press can reach gameplay mappings if no screen remains open.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "result",
        event_per_listener = true,
        neoforge = "ScreenEvent.MouseButtonPressed.Post",
        cancellable = false,
        version = 3
    )]
    fn screen_mouse_button_pressed_post(
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        double_click: bool,
        was_click_handled: bool,
        result: &mut TriState,
    );
    /// A mouse release after the receiving screen handled or declined it
    /// (`ScreenEvent.MouseButtonReleased.Post`). The matching Pre must not have cancelled.
    /// `screen`, `button` and `modifiers` come from the release's Pre, not the earlier press
    /// or a screen the release opened. Every listener sees `was_release_handled`, the original
    /// answer from the receiving handler. Completing quick-craft is not itself a handled
    /// release: the child still answers. An early swallowed container release is handled.
    ///
    /// Listeners share `result`: `TriState::Default` keeps that answer, `TriState::True` forces
    /// handled and `TriState::False` forces unhandled. An unhandled release can reach gameplay
    /// mappings if no screen remains open.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "result",
        event_per_listener = true,
        neoforge = "ScreenEvent.MouseButtonReleased.Post",
        cancellable = false,
        version = 3
    )]
    fn screen_mouse_button_released_post(
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        was_release_handled: bool,
        result: &mut TriState,
    );
    /// A held-button pointer movement that the receiving screen (and its mounted widgets) did not handle (`ScreenEvent.MouseDragged.Post`, vanilla's `Screen.mouseDragged` returning false): `button`, `delta` (GUI pixels, `screen.mouse` the new position) and `modifiers` as in `screen_mouse_dragged_pre`. Not cancellable. Fires on the receiving screen only when unhandled and when the matching Pre did not cancel it, on the window only, for real mouse input and not a pad's or keyboard focus's.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "listen",
        neoforge = "ScreenEvent.MouseDragged.Post",
        cancellable = false
    )]
    fn screen_mouse_dragged_post(
        screen: &ScreenView<'_>,
        button: i32,
        delta: [f32; 2],
        modifiers: u32,
    );
    /// A mouse wheel scroll that the receiving screen (and its mounted widgets) did not handle (`ScreenEvent.MouseScrolled.Post`, vanilla's `Screen.mouseScrolled` returning false): `scroll` and `modifiers` as in `screen_mouse_scrolled_pre`. Not cancellable. Fires on the receiving screen only when unhandled and when the matching Pre did not cancel it, on the window only, for real mouse input and not a pad's or keyboard focus's.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "listen",
        neoforge = "ScreenEvent.MouseScrolled.Post",
        cancellable = false
    )]
    fn screen_mouse_scrolled_post(screen: &ScreenView<'_>, scroll: [f64; 2], modifiers: u32);
    /// A key press that neither the receiving screen nor its mounted widgets took
    /// (`ScreenEvent.KeyPressed.Post`, vanilla's `Screen.keyPressed` returning false):
    /// `key`, `repeat` and `modifiers` as in `screen_key_pressed_pre`. If the mod
    /// browser's search editor declines, the container still handles raw bindings
    /// (including rebound inventory close and sort) before this hook can fire.
    /// Slot swap, pick and drop can act before an unhandled result; a translated
    /// mapping reached after Post cannot sort the container. Set `canceled` to
    /// keep the game's own key handling (bindings, hotbar and toggles) from seeing it.
    /// A key that has no vanilla name fires nothing. Fires for window key presses
    /// and the control socket's `key ... down`, `key ... repeat`, and the press
    /// phase of `key ... tap`, on the receiving screen and only when the matching
    /// Pre did not cancel it. A controller does not synthesize this hook.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 3,
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyPressed.Post",
        cancellable = true
    )]
    fn screen_key_pressed_post(
        screen: &ScreenView<'_>,
        key: &str,
        repeat: bool,
        modifiers: u32,
        canceled: &mut bool,
    );
    /// A key release that neither the receiving screen nor its mounted widgets took (`ScreenEvent.KeyReleased.Post`, vanilla's `Screen.keyReleased` returning false): `key` and `modifiers` as in `screen_key_released_pre`. Set `canceled` and the release counts as handled: the game's own key handling never sees it, so a held mapping stays down; cancel a release only when you cancelled the press it ends. Fires for window key releases and the control socket's `key ... up` and release phase of `key ... tap`, on the receiving screen and only when the matching Pre did not cancel it. A controller does not synthesize this hook.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyReleased.Post",
        cancellable = true
    )]
    fn screen_key_released_post(
        screen: &ScreenView<'_>,
        key: &str,
        modifiers: u32,
        canceled: &mut bool,
    );
    /// A typed character that neither the receiving screen nor its mounted widgets took (`ScreenEvent.CharacterTyped.Post`, vanilla's `Screen.charTyped` returning false), once per printable character that `screen_char_typed_pre` let through: `c` and `modifiers` as there. Not cancellable (matching NeoForge's `ScreenEvent.CharacterTyped.Post`, which is an unhandled post-dispatch notice). Window characters (including IME commits and text without a physical key) and the control socket's `type` fire it on the receiving screen when the matching Pre did not cancel. A held edit modifier makes a physical key a shortcut instead of text; a controller's keyboard does not synthesize this hook.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 3,
        contract = "listen",
        neoforge = "ScreenEvent.CharacterTyped.Post",
        cancellable = false
    )]
    fn screen_char_typed_post(screen: &ScreenView<'_>, c: char, modifiers: u32);
}
