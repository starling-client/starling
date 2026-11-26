use super::*;

impl Game<'_> {
    /// Whether the pointer is over the window, focused or not (the window system's enter and
    /// leave events).
    pub(in crate::game) fn window_hovered(&self) -> bool {
        self.window_hovered
    }
    /// Whether the window is fully hidden, as the window system reports occlusion.
    pub(in crate::game) fn window_occluded(&self) -> bool {
        self.window_occluded
    }
    /// The last window focus state reported by the window system.
    pub(in crate::game) fn window_focused(&self) -> bool {
        self.focused
    }

    /// The same screen identifier provided to screen hooks.
    pub(in crate::game) fn observed_screen_id(&self) -> Option<ScreenId> {
        self.observed_screen_view().map(|view| ScreenId {
            id: view.id.to_owned(),
        })
    }
}
