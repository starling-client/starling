//! The `screen_input_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mouse press after the receiving screen handled or declined it
    /// (`ScreenEvent.MouseButtonPressed.Post`). The matching Pre must not have cancelled.
    /// `screen`, `button`, `modifiers` and `double_click` are the press Pre values, even if
    /// the press closed or replaced that screen. Every listener sees `was_click_handled`,
    /// the screen handler's original answer.
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
        version = 2
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
    /// or a screen the release opened. Every listener sees `was_release_handled`, the screen
    /// handler's original answer.
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
        version = 2
    )]
    fn screen_mouse_button_released_post(
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        was_release_handled: bool,
        result: &mut TriState,
    );
    /// A held-button pointer movement has gone through the open screen's drag handlers (`ScreenEvent.MouseDragged.Post`, vanilla's `Screen.mouseDragged`): `button`, `delta` (GUI pixels, `screen.mouse` the new position) and `modifiers` as in `screen_mouse_dragged_pre`. Not cancellable. Sunrise's drag handlers report nothing back, so it fires whether or not one of them used the drag. Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the game handled the input, so an input that closed the screen fires nothing.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
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
    /// The wheel turned over the open screen and the screen's scroll handlers have run (`ScreenEvent.MouseScrolled.Post`, vanilla's `Screen.mouseScrolled`): `scroll` and `modifiers` as in `screen_mouse_scrolled_pre`. Not cancellable. Sunrise's scroll handlers report nothing back, so it fires whether or not one of them scrolled. Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the game handled the input, so an input that closed the screen fires nothing.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.MouseScrolled.Post",
        cancellable = false
    )]
    fn screen_mouse_scrolled_post(screen: &ScreenView<'_>, scroll: [f64; 2], modifiers: u32);
    /// A key press that no screen took (`ScreenEvent.KeyPressed.Post`, vanilla's `Screen.keyPressed` returning false): `key`, `repeat` and `modifiers` as in `screen_key_pressed_pre`. Set `canceled` and the press counts as handled: the game's own key handling (its key bindings, hotbar and toggles) never sees it. It does not fire for a key a screen took (a text box, the focus keys, Escape-close, an inventory key, a merchant or recipe-book key). A key that has no vanilla name fires nothing. The control socket's `key` fires it too, for a press (never a release). Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the game handled the input, so an input that closed the screen fires nothing.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyPressed.Post",
        cancellable = true,
        version = 2
    )]
    fn screen_key_pressed_post(
        screen: &ScreenView<'_>,
        key: &str,
        repeat: bool,
        modifiers: u32,
        canceled: &mut bool,
    );
    /// A key release that no screen took (`ScreenEvent.KeyReleased.Post`, vanilla's `Screen.keyReleased` returning false): `key` and `modifiers` as in `screen_key_released_pre`. Set `canceled` and the release counts as handled: the game's own key handling never sees it, so a held mapping stays down; cancel a release only when you cancelled the press it ends. Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the game handled the input, so an input that closed the screen fires nothing.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
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
    /// A typed character that no text box took (`ScreenEvent.CharacterTyped.Post`, vanilla's `Screen.charTyped` returning false), once per printable character that `screen_char_typed_pre` let through: `c` and `modifiers` as there. Set `canceled` and it counts as handled (nothing further reads it, so it changes nothing). It does not fire for text a box took, nor on Options and ordinary title screens, where the key's text is routed in place between their boxes. A bound `world_preset_editor` also fires it for unhandled input-method and control-socket text. A held edit modifier makes the key a shortcut, so its character is not taken and never fires it. Same screens as the key press. Fires on the window only, while a screen of the game's or the title's is open (the screens the `_pre` input hooks cover), for real mouse or keyboard input and not a pad's or the keyboard focus's, and only when the matching Pre did not cancel it. It fires on the screen open after the key acted, as for the Pre.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.CharacterTyped.Post",
        cancellable = true,
        version = 2
    )]
    fn screen_char_typed_post(
        screen: &ScreenView<'_>,
        c: char,
        modifiers: u32,
        canceled: &mut bool,
    );
}
