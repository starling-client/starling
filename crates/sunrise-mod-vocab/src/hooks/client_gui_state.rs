//! Client window focus and observed screen reads.

pub trait Notes {
    #[notes = r#"## Client window and screen state

`ClientHost::window_focused()` reports the last focus state from the window
system. `ClientHost::window_hovered()` is false until the pointer first enters
the window or the player moves it over the window; after that it follows the
window system's enter and leave. `ClientHost::window_occluded()` is the window
system's occlusion report, where the platform gives one.
`ClientHost::observed_screen_id()` returns the same ID screen hooks see,
including overlays. `None` means no view is available: there may be no open
screen or no renderer yet. It does not mean the client is outside a world.
Read again after a screen or focus change."#]
    fn window_focused();
}
