//! Q on the title screen quits the game through `ClientHost::request_client_quit`.

// ANCHOR: title_quit
use mod_api::{ClientHost, Flow, Hooks, ScreenKeyPressedPreEvent};

/// The mod.
#[derive(Default)]
pub struct TitleQuit;

#[mod_api::sunrise_mod]
impl Hooks for TitleQuit {
    fn screen_key_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        if event.screen.id == "title"
            && event.key == "key.keyboard.q"
            && !event.repeat
            && host.request_client_quit()
        {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}
// ANCHOR_END: title_quit
