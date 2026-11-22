//! The `screen_mouse_button_pressed_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0093

pub trait Hooks {
    /// A press about to go to `Screen.mouseClicked` (`ScreenEvent.MouseButtonPressed.Pre`).
    /// `screen` is the receiving screen and `screen.mouse` is the pointer position.
    /// `button` is the GLFW index (0 left, 1 right, 2 middle, 3 back, 4 forward; beyond those,
    /// the platform's own number). `modifiers` are GLFW bits (shift 1, control 2, alt 4, super 8).
    /// The receiver and press Post get the same `double_click`: true for a real press when
    /// the last handled real press was of this button on this screen instance less than 250 ms
    /// ago. Cancel to skip the screen handler, Post and gameplay mappings; a cancelled real
    /// press still counts as handled for double-click detection.
    ///
    /// It fires before anything the screen does with the press, on the window
    /// only, and while a screen of the game's or the title's is open (Options
    /// and the pack list opened from the title screen count, with no player
    /// and no `menu`; the title's own screens, their
    /// `screen.id` from the title state machine). It does not fire on the
    /// connect and loading screens, nor for a server
    /// dialog, the server pack, link and code-of-conduct prompts, a book or a
    /// sign editor drawn over gameplay, nor while a world loads or saves: the
    /// click there stays the game's own. A gamepad click or a click the
    /// keyboard's focus sends is not the mouse and fires nothing.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_screen_mouse_button_pressed_pre",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.MouseButtonPressed.Pre",
        cancellable = true,
        version = 2
    )]
    fn screen_mouse_button_pressed_pre(
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        double_click: bool,
        canceled: &mut bool,
    );
}
