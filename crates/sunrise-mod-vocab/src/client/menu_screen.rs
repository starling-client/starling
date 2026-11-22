use crate::{ClientHost, DrawContext, Text, client::types::MenuScreenLayout};
use std::{cell::RefCell, sync::atomic::AtomicBool};

type Draw = dyn FnMut(&dyn ClientHost, MenuScreenFrame<'_>, &mut dyn DrawContext) + Send;
type Click = dyn FnMut(&dyn ClientHost, MenuScreenFrame<'_>, bool) -> Option<i32> + Send;
type Tooltip =
    dyn FnMut(&dyn ClientHost, MenuScreenFrame<'_>, Option<u32>, &mut Vec<(Text, u32)>) + Send;

// no wire: a window-owned presentation with client-only callbacks.
/// Presentation of one open menu. The window owns it and its callbacks until
/// that opening closes. Capture owned values from [`MenuScreenOpen`] in the
/// callbacks; read current pointer coordinates and live menu data from each
/// [`MenuScreenFrame`]. Opening bytes do not persist on the client host.
// exhaustive: private fields keep the layout and its window-owned callbacks together behind the builder.
pub struct MenuScreen {
    layout: MenuScreenLayout,
    draw: Option<RefCell<Box<Draw>>>,
    click: Option<RefCell<Box<Click>>>,
    tooltip: Option<RefCell<Box<Tooltip>>>,
}

impl MenuScreen {
    /// Give the open window its layout. Sunrise validates it against the active menu
    /// before displaying it. Use positive finite dimensions and finite slot positions.
    pub fn new(layout: MenuScreenLayout) -> Self {
        Self {
            layout,
            draw: None,
            click: None,
            tooltip: None,
        }
    }

    /// Draw over the default panel and slot frames, before labels and item stacks.
    /// Set `layout.own_background` before construction to omit the default panel.
    /// The draw context is scoped to this call; draw in scaled GUI pixels.
    pub fn with_draw(
        mut self,
        draw: impl FnMut(&dyn ClientHost, MenuScreenFrame<'_>, &mut dyn DrawContext) + Send + 'static,
    ) -> Self {
        self.draw = Some(RefCell::new(Box::new(draw)));
        self
    }

    /// Handle a press that did not hit a slot. The boolean is true for right click,
    /// false for left click. Return `Some(id)` to consume the press and send a
    /// menu button to the server. The server still validates the window, menu
    /// and ID before `MenuPart::menu_button`. Return `None` to pass the press on.
    pub fn with_click(
        mut self,
        click: impl FnMut(&dyn ClientHost, MenuScreenFrame<'_>, bool) -> Option<i32> + Send + 'static,
    ) -> Self {
        self.click = Some(RefCell::new(Box::new(click)));
        self
    }

    /// Edit the rich text/color lines after item tooltip hooks. The slot is a menu
    /// index (including player inventory slots), or `None` outside a mapped slot.
    /// Empty slots and screen space start without lines. The item's images and
    /// component gathering, and the carried stack's visibility rule, are unchanged.
    pub fn with_tooltip(
        mut self,
        tooltip: impl FnMut(&dyn ClientHost, MenuScreenFrame<'_>, Option<u32>, &mut Vec<(Text, u32)>)
        + Send
        + 'static,
    ) -> Self {
        self.tooltip = Some(RefCell::new(Box::new(tooltip)));
        self
    }
}

// no wire: a callback borrows the current client menu's live data.
/// Current geometry and data for one menu screen callback. All coordinates are
/// in scaled GUI pixels. Pointer positions can be outside the panel.
// exhaustive: the game constructs this view; mods only read it
pub struct MenuScreenFrame<'a> {
    /// The panel's top-left corner.
    pub origin: [f32; 2],
    /// Absolute pointer coordinates, truncated toward zero.
    pub pointer: [i32; 2],
    /// The scaled width and height of the GUI (not of the panel).
    pub gui_size: [i32; 2],
    /// Pointer position relative to `origin`, without integer truncation.
    pub at: [f32; 2],
    /// Live data slots. Block-backed menus carry signed shorts as in vanilla, with
    /// indices `0..=i16::MAX`. The negotiated blockless payload carries `i32` values
    /// with indices `0..=u16::MAX`. Use `get`: slots may not have arrived yet.
    pub data: &'a [i32],
}

// no wire: game-side ownership and panic state for a single client opening.
/// An installed screen with the owner resolved at construction time. Engine only.
// exhaustive: private fields bind the screen to the installed mod and its per-opening panic state.
pub struct OwnedMenuScreen {
    screen: MenuScreen,
    owner: &'static str,
    index: usize,
    draw_panicked: AtomicBool,
    click_panicked: AtomicBool,
    tooltip_panicked: AtomicBool,
}

impl OwnedMenuScreen {
    /// Bind the resulting instance to its factory's owner.
    pub fn new(owner: &'static str, index: usize, screen: MenuScreen) -> Self {
        Self {
            screen,
            owner,
            index,
            draw_panicked: AtomicBool::new(false),
            click_panicked: AtomicBool::new(false),
            tooltip_panicked: AtomicBool::new(false),
        }
    }

    /// The registering mod's id, used for diagnostics and panic attribution.
    pub fn owner(&self) -> &'static str {
        self.owner
    }

    /// Read the layout from the window-owned screen.
    pub fn layout(&self) -> &MenuScreenLayout {
        &self.screen.layout
    }

    /// Access the layout for open-time validation and normalization.
    pub fn layout_mut(&mut self) -> &mut MenuScreenLayout {
        &mut self.screen.layout
    }

    /// Whether to build a draw context for this frame.
    pub fn has_draw(&self) -> bool {
        self.screen.draw.is_some() && self.callback_enabled(&self.draw_panicked)
    }

    /// Run the opening's draw callback, if enabled.
    pub fn draw(
        &self,
        host: &dyn ClientHost,
        frame: MenuScreenFrame<'_>,
        ctx: &mut dyn DrawContext,
    ) {
        if let Some(mut draw) = self
            .screen
            .draw
            .as_ref()
            .and_then(|f| f.try_borrow_mut().ok())
        {
            self.call(&self.draw_panicked, "menu_screen_draw", || {
                draw(host, frame, ctx)
            });
        }
    }

    /// A button returned by this opening's enabled click callback, if any.
    pub fn click(
        &self,
        host: &dyn ClientHost,
        frame: MenuScreenFrame<'_>,
        right: bool,
    ) -> Option<i32> {
        let mut click = self.screen.click.as_ref()?.try_borrow_mut().ok()?;
        self.call(&self.click_panicked, "menu_screen_click", || {
            click(host, frame, right)
        })
        .flatten()
    }

    /// Whether this opening wants to see the current tooltip lines.
    pub fn has_tooltip(&self) -> bool {
        self.screen.tooltip.is_some() && self.callback_enabled(&self.tooltip_panicked)
    }

    /// Run the tooltip callback. Only take the edited lines when it succeeds.
    pub fn tooltip(
        &self,
        host: &dyn ClientHost,
        frame: MenuScreenFrame<'_>,
        slot: Option<u32>,
        lines: &mut Vec<(Text, u32)>,
    ) -> bool {
        let Some(mut tooltip) = self
            .screen
            .tooltip
            .as_ref()
            .and_then(|f| f.try_borrow_mut().ok())
        else {
            return false;
        };
        self.call(&self.tooltip_panicked, "menu_screen_tooltip", || {
            tooltip(host, frame, slot, lines)
        })
        .is_some()
    }

    fn callback_enabled(&self, poisoned: &AtomicBool) -> bool {
        crate::callback_owner::enabled(self.index, poisoned)
    }

    fn call<T>(
        &self,
        poisoned: &AtomicBool,
        operation: &'static str,
        invoke: impl FnOnce() -> T,
    ) -> Option<T> {
        if !self.callback_enabled(poisoned) {
            return None;
        }
        crate::callback_owner::call(
            self.owner,
            self.index,
            poisoned,
            "menu screen",
            operation,
            invoke,
        )
        .ok()
    }
}

// no wire: the factory borrows the active client's menu metadata only for construction.
/// The active menu contract, offered once before the window shows its screen.
// exhaustive: the game constructs this view; mods only read it
pub struct MenuScreenOpen<'a> {
    /// The server's container id for this opening. It can be reused after close.
    pub window: i32,
    /// The active menu type's namespaced identifier.
    pub menu: &'a str,
    /// The verified mod block whose container opens this menu, if any.
    pub block: Option<&'a str>,
    /// The position supplied for this opening, or its verified clicked opener.
    pub pos: Option<[i32; 3]>,
    /// The server's title as an API text component, if available.
    pub title: Option<&'a crate::Text>,
    /// The number of physical container slots.
    pub backing_slots: usize,
    /// The number of appended player inventory slots, including the hotbar.
    pub player_slots: usize,
    /// The declared live data slot count for a mod menu type, or `0` for a stock
    /// menu whose data slots arrive dynamically. This is not the number received:
    /// consult [`MenuScreenFrame::data`] for the available values and wire range.
    pub data_slots: usize,
    /// The server's one-time payload for this opening, or an empty slice if no
    /// payload matched. Decode it in the factory and retain only owned values.
    /// Read live updates from [`MenuScreenFrame::data`], not from these bytes.
    pub extra: &'a [u8],
}

/// Constructs a fresh presentation for an opening, not a shared screen instance.
/// Register it with [`crate::MenusRegistrar::register_menu_screen`].
pub trait MenuScreenFactory: Send + Sync + 'static {
    /// Build the screen for `open`, or return `None` to decline the opening.
    /// Called on the client before the screen is installed. `host` reads client state;
    /// controls for the new screen (such as local slot moves) are not available yet.
    fn create(&self, host: &dyn crate::ClientHost, open: MenuScreenOpen<'_>) -> Option<MenuScreen>;
}

impl<F> MenuScreenFactory for F
where
    F: Fn(&dyn crate::ClientHost, MenuScreenOpen<'_>) -> Option<MenuScreen> + Send + Sync + 'static,
{
    fn create(&self, host: &dyn crate::ClientHost, open: MenuScreenOpen<'_>) -> Option<MenuScreen> {
        self(host, open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct EmptyDraw;
    impl DrawContext for EmptyDraw {}

    #[test]
    fn draw_panic_disables_only_draw_leaving_click_and_tooltip_active() {
        let draw_runs = Arc::new(AtomicUsize::new(0));
        let draw_runs_clone = draw_runs.clone();
        let screen = MenuScreen::new(MenuScreenLayout {
            width: 176.0,
            height: 166.0,
            ..Default::default()
        })
        .with_draw(move |_, _, _| {
            draw_runs_clone.fetch_add(1, Ordering::Relaxed);
            panic!("draw exploded");
        })
        .with_click(|_, _, right| Some(if right { 2 } else { 1 }))
        .with_tooltip(|_, _, _, lines| lines.push((Text::literal("tip"), 0xFFFFFFFF)));

        let owned = OwnedMenuScreen::new("screen_mod", 0, screen);
        let host = crate::testing::FakeHost::new();
        let frame = || MenuScreenFrame {
            origin: [0.0, 0.0],
            pointer: [10, 10],
            gui_size: [320, 240],
            at: [10.0, 10.0],
            data: &[],
        };
        let mut ctx = EmptyDraw;

        assert!(owned.has_draw());
        assert!(owned.has_tooltip());

        owned.draw(&host, frame(), &mut ctx);
        assert_eq!(draw_runs.load(Ordering::Relaxed), 1);
        assert!(!owned.has_draw(), "panicking draw must be disabled");

        // Second draw is skipped.
        owned.draw(&host, frame(), &mut ctx);
        assert_eq!(draw_runs.load(Ordering::Relaxed), 1);

        // Click and tooltip on the same opening remain enabled and functional.
        assert_eq!(owned.click(&host, frame(), false), Some(1));
        assert_eq!(owned.click(&host, frame(), true), Some(2));
        assert!(owned.has_tooltip());
        let mut lines = Vec::new();
        assert!(owned.tooltip(&host, frame(), Some(0), &mut lines));
        assert_eq!(lines.len(), 1);
    }
}
