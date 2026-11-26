//! Mounted widgets for vanilla and mod screens (`ScreenEvent.Init.Post`).
//!
//! Mods add buttons ([`Button`]), single-line text fields ([`TextField`]), and custom widgets
//! ([`ScreenWidget`]) through [`ScreenWidgets`] during `Hooks::screen_init_post`
//! (`Screen.addRenderableWidget`), rather than drawing widgets manually or forwarding screen input
//! hooks by hand. The engine owns drawing, hover, focus, Tab traversal, controller navigation,
//! narration, and mouse/keyboard dispatch for mounted widgets on every screen, clearing and
//! rebuilding them when the screen initializes or rebuilds its children, not on a layout-only resize.
//!
//! Built-in buttons and text fields match vanilla's `Button` (`Button.Builder` / `Button.OnPress`)
//! and `EditBox`, sharing the same editing core ([`TextEdit`]) as the game's own fields.
//!
//! ```
//! use mod_api::client::widgets::{Button, TextField};
//! use mod_api::Text;
//! let _name = TextField::new([20.0, 20.0, 150.0, 20.0], 32)
//!     .value("Steve")
//!     .hint(Text::translate("mymod.screen.player_name", vec![]));
//! let done = Button::new(
//!     [20.0, 50.0, 150.0, 20.0],
//!     Text::translate("gui.done", vec![]),
//!     |state, _host| {
//!         state.active = false;
//!     },
//! );
//! assert!(done.state().active);
//! ```

use crate::{ClientHost, DrawContext, ItemStack, Text};
use std::fmt;

#[path = "widgets_text_edit.rs"]
mod text_edit;
pub use text_edit::{
    EditKey, TextEdit, byte_index, char_len, filter_text, is_allowed_chat_character,
    java_whitespace, multiline_word_position, utf16_len, utf16_prefix, word_position,
};

/// Custom GUI art for a [`Button`], using the sprite ids accepted by [`DrawContext::sprite`].
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct ButtonSprites {
    /// The normal sprite. If this is selected but missing, uses the vanilla sprite for that state.
    pub normal: String,
    /// The hovered or focused sprite. An absent or unknown id falls back to `normal`.
    pub highlighted: Option<String>,
    /// The inactive sprite, even when highlighted. An absent or unknown id falls back to `normal`.
    pub disabled: Option<String>,
    /// `[x, y]` in GUI pixels, added to the label's draw position, not its hitbox or overflow
    /// clip. If either coordinate is non-finite, the whole offset is ignored.
    pub label_offset: [f32; 2],
}

impl ButtonSprites {
    /// Uses `normal` in every state, with no label offset.
    pub fn new(normal: impl Into<String>) -> Self {
        Self {
            normal: normal.into(),
            highlighted: None,
            disabled: None,
            label_offset: [0.0; 2],
        }
    }
}

/// Identifier of a child on the current screen during `Hooks::screen_init_post`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// no wire: screen-local widget handle issued by the engine during `screen_init_post`, never serialized.
// exhaustive: opaque per-screen handle whose inner value is private; mods only compare and pass it back.
pub struct ScreenWidgetId(u64);

impl ScreenWidgetId {
    /// Build a widget identifier from an engine-assigned raw id.
    // hidden: only the engine assigns mounted widget identifiers during `screen_init_post`.
    #[doc(hidden)]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Read the engine-assigned raw id.
    // hidden: only the engine maps widget identifiers back to its mounted widget table.
    #[doc(hidden)]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Mutable state of a mounted [`Button`], passed to its [`ButtonPress`] callback.
#[derive(Clone, Debug, PartialEq)]
// no wire: live mounted-button state mutated in-process by its press callback.
#[non_exhaustive]
pub struct ButtonState {
    /// Where it is, `[x, y, width, height]` in GUI pixels (vanilla's are 20 high).
    /// A press callback's non-finite rectangle is ignored, retaining the previous bounds.
    pub rect: [f32; 4],
    /// Its label, centred and scrolled when it does not fit.
    pub label: Text,
    /// An inactive button is greyed out, takes no press and is skipped by Tab.
    pub active: bool,
    /// A hidden button is neither drawn nor interactive.
    pub visible: bool,
    /// Custom art; `None` draws vanilla sprites with no label offset.
    pub sprites: Option<ButtonSprites>,
}

/// Press callback for a mounted [`Button`] (`Button.OnPress`).
// no wire: in-process callback invoked by the client when a mounted button is pressed.
pub type ButtonPress = dyn FnMut(&mut ButtonState, &mut dyn ClientHost) + Send + 'static;

/// A push button (`Button`) mounted on a screen during `Hooks::screen_init_post`.
// no wire: owns an in-process press callback (`ButtonPress`).
// exhaustive: its fields are private; mods build it with `Button::new` or `Button::inactive`.
pub struct Button {
    state: ButtonState,
    on_press: Option<Box<ButtonPress>>,
}

impl Button {
    /// An active, visible button with vanilla art and a press callback (`Button.Builder`).
    pub fn new(
        rect: [f32; 4],
        label: impl Into<Text>,
        on_press: impl FnMut(&mut ButtonState, &mut dyn ClientHost) + Send + 'static,
    ) -> Self {
        Self {
            state: ButtonState {
                rect,
                label: label.into(),
                active: true,
                visible: true,
                sprites: None,
            },
            on_press: Some(Box::new(on_press)),
        }
    }

    /// An inactive, visible button with no press callback and vanilla art.
    pub fn inactive(rect: [f32; 4], label: impl Into<Text>) -> Self {
        Self {
            state: ButtonState {
                rect,
                label: label.into(),
                active: false,
                visible: true,
                sprites: None,
            },
            on_press: None,
        }
    }

    /// Set whether the button accepts presses and Tab focus.
    pub fn active(mut self, active: bool) -> Self {
        self.state.active = active;
        self
    }

    /// Set whether the button is drawn and interactive.
    pub fn visible(mut self, visible: bool) -> Self {
        self.state.visible = visible;
        self
    }

    /// Use custom GUI art instead of vanilla's button sprites.
    pub fn sprites(mut self, sprites: ButtonSprites) -> Self {
        self.state.sprites = Some(sprites);
        self
    }

    /// Inspect the button's configured state before mounting it.
    pub fn state(&self) -> &ButtonState {
        &self.state
    }

    /// Unpack the button into its state and press callback for the engine's widget store.
    // hidden: only the engine's screen widget store unpacks a `Button` when mounted.
    #[doc(hidden)]
    pub fn into_parts(self) -> (ButtonState, Option<Box<ButtonPress>>) {
        (self.state, self.on_press)
    }
}

impl fmt::Debug for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Button")
            .field("state", &self.state)
            .field("has_on_press", &self.on_press.is_some())
            .finish()
    }
}

/// Change or submit callback for a mounted [`TextField`].
// no wire: in-process callback invoked by the client when a mounted text field changes or submits.
pub type TextFieldCallback = dyn FnMut(&str, &mut dyn ClientHost) + Send + 'static;

/// Extracted fields of a [`TextField`] for the engine's screen widget store.
// no wire: engine-only handoff record carrying in-process callbacks.
// hidden: only the engine's screen widget store unpacks a `TextField` when mounted.
#[doc(hidden)]
#[non_exhaustive]
pub struct TextFieldParts {
    /// Where it is, `[x, y, width, height]` in GUI pixels (vanilla's are 20 high).
    pub rect: [f32; 4],
    /// Initial text content.
    pub value: String,
    /// Optional hint text shown when the field is empty (`EditBox.setHint`).
    pub hint: Option<Text>,
    /// Maximum number of UTF-16 code units accepted (`EditBox.setMaxLength`, `insertText`).
    pub max_length: usize,
    /// Whether the player may edit the text (`EditBox.setEditable`).
    pub editable: bool,
    /// Whether the field is active and accepts Tab focus.
    pub active: bool,
    /// Whether the field is drawn and interactive.
    pub visible: bool,
    /// Whether the vanilla `widget/text_field` frame is drawn (`EditBox.setBordered`).
    pub bordered: bool,
    /// Called whenever the field's text changes (`EditBox.setResponder`).
    pub on_change: Option<Box<TextFieldCallback>>,
    /// Called when Enter is pressed while the field is focused.
    pub on_submit: Option<Box<TextFieldCallback>>,
}

/// A single-line text field (`EditBox`) mounted on a screen during `Hooks::screen_init_post`.
/// The controller keyboard edits the focused field without forwarding completion
/// or suggestion/history navigation to a native Chat editor behind it.
// no wire: owns in-process change and submit callbacks (`TextFieldCallback`).
// exhaustive: its fields are private; mods build it with `TextField::new`.
pub struct TextField {
    rect: [f32; 4],
    value: String,
    hint: Option<Text>,
    max_length: usize,
    editable: bool,
    active: bool,
    visible: bool,
    bordered: bool,
    on_change: Option<Box<TextFieldCallback>>,
    on_submit: Option<Box<TextFieldCallback>>,
}

impl TextField {
    /// An empty, editable, active, visible, bordered text field taking up to `max_length`
    /// UTF-16 code units (`EditBox.setMaxLength` and `EditBox.insertText`).
    pub fn new(rect: [f32; 4], max_length: usize) -> Self {
        Self {
            rect,
            value: String::new(),
            hint: None,
            max_length,
            editable: true,
            active: true,
            visible: true,
            bordered: true,
            on_change: None,
            on_submit: None,
        }
    }

    /// Set the initial text content (`EditBox.setValue`).
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    /// Set hint text drawn while the field is empty (`EditBox.setHint`).
    pub fn hint(mut self, hint: impl Into<Text>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Set whether typing and edits can modify the field's text (`EditBox.setEditable`).
    pub fn editable(mut self, editable: bool) -> Self {
        self.editable = editable;
        self
    }

    /// Set whether the field is active and accepts Tab focus.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Set whether the field is drawn and interactive.
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Set whether the vanilla `widget/text_field` frame is drawn (`EditBox.setBordered`).
    pub fn bordered(mut self, bordered: bool) -> Self {
        self.bordered = bordered;
        self
    }

    /// Install a callback invoked whenever the field's text changes (`EditBox.setResponder`).
    pub fn on_change(
        mut self,
        callback: impl FnMut(&str, &mut dyn ClientHost) + Send + 'static,
    ) -> Self {
        self.on_change = Some(Box::new(callback));
        self
    }

    /// Install a callback invoked when Enter is pressed while the field is focused.
    pub fn on_submit(
        mut self,
        callback: impl FnMut(&str, &mut dyn ClientHost) + Send + 'static,
    ) -> Self {
        self.on_submit = Some(Box::new(callback));
        self
    }

    /// Unpack the text field into its fields and callbacks for the engine's widget store.
    // hidden: only the engine's screen widget store unpacks a `TextField` when mounted.
    #[doc(hidden)]
    pub fn into_parts(self) -> TextFieldParts {
        TextFieldParts {
            rect: self.rect,
            value: self.value,
            hint: self.hint,
            max_length: self.max_length,
            editable: self.editable,
            active: self.active,
            visible: self.visible,
            bordered: self.bordered,
            on_change: self.on_change,
            on_submit: self.on_submit,
        }
    }
}

impl fmt::Debug for TextField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextField")
            .field("rect", &self.rect)
            .field("value", &self.value)
            .field("hint", &self.hint)
            .field("max_length", &self.max_length)
            .field("editable", &self.editable)
            .field("active", &self.active)
            .field("visible", &self.visible)
            .field("bordered", &self.bordered)
            .field("has_on_change", &self.on_change.is_some())
            .field("has_on_submit", &self.on_submit.is_some())
            .finish()
    }
}

/// What a context menu entry runs when the player picks it (`ClientHost::open_context_menu`).
// no wire: in-process callback invoked by the client when a context menu action is picked.
pub type ContextMenuAction = dyn FnOnce(&mut dyn ClientHost) + Send + 'static;

/// One row of a context menu: an action, a nested menu, or a separator line.
// no wire: the entries hold the mod's closures and stay in this process.
#[non_exhaustive]
pub enum ContextMenuEntry {
    /// A row the player can pick: the menu closes, then `on_select` runs.
    Action {
        /// The row's text.
        label: Text,
        /// An item drawn before the label.
        icon: Option<ItemStack>,
        /// A disabled row is drawn greyed and can't be picked.
        enabled: bool,
        /// Runs once when the player picks the row, after the menu closed.
        on_select: Box<ContextMenuAction>,
    },
    /// A row that opens `entries` beside it (hover, the right arrow key, or Enter).
    Submenu {
        /// The row's text, drawn with an arrow after it.
        label: Text,
        /// An item drawn before the label.
        icon: Option<ItemStack>,
        /// A disabled row is drawn greyed and doesn't open.
        enabled: bool,
        /// The nested menu's rows; an empty list doesn't open.
        entries: Vec<ContextMenuEntry>,
    },
    /// A thin line between groups of rows.
    Separator,
}

impl ContextMenuEntry {
    /// An enabled action with no icon.
    pub fn action(
        label: impl Into<Text>,
        on_select: impl FnOnce(&mut dyn ClientHost) + Send + 'static,
    ) -> Self {
        Self::Action {
            label: label.into(),
            icon: None,
            enabled: true,
            on_select: Box::new(on_select),
        }
    }
    /// An enabled nested menu with no icon.
    pub fn submenu(label: impl Into<Text>, entries: Vec<ContextMenuEntry>) -> Self {
        Self::Submenu {
            label: label.into(),
            icon: None,
            enabled: true,
            entries,
        }
    }
    /// A separator line.
    pub fn separator() -> Self {
        Self::Separator
    }
    /// The same row with `stack` drawn before its label (nothing for a separator).
    pub fn icon(mut self, stack: ItemStack) -> Self {
        if let Self::Action { icon, .. } | Self::Submenu { icon, .. } = &mut self {
            *icon = Some(stack);
        }
        self
    }
    /// The same row enabled or disabled (nothing for a separator).
    pub fn enabled(mut self, on: bool) -> Self {
        if let Self::Action { enabled, .. } | Self::Submenu { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }
}

impl std::fmt::Debug for ContextMenuEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Action { label, enabled, .. } => f
                .debug_struct("Action")
                .field("label", label)
                .field("enabled", enabled)
                .finish_non_exhaustive(),
            Self::Submenu {
                label,
                enabled,
                entries,
                ..
            } => f
                .debug_struct("Submenu")
                .field("label", label)
                .field("enabled", enabled)
                .field("entries", entries)
                .finish_non_exhaustive(),
            Self::Separator => f.write_str("Separator"),
        }
    }
}

/// A custom widget mounted on a screen during `Hooks::screen_init_post`
/// (`Renderable` + `GuiEventListener`).
///
/// The engine owns focus, hover, Tab traversal, drawing order, and input dispatch for mounted
/// widgets. Override the methods the widget reacts to; every input handler defaults to `false`
/// (unhandled). A pointer press goes only to the first hit child; returning `false` from
/// `mouse_clicked` leaves focus unchanged.
// no wire: in-process custom widget trait object owned by the client screen while it stays open.
pub trait ScreenWidget: Send + 'static {
    /// Where the widget is, `[x, y, width, height]` in GUI pixels.
    fn bounds(&self) -> [f32; 4];

    /// Whether the widget is drawn and receives pointer/keyboard input. Defaults to `true`.
    fn visible(&self) -> bool {
        true
    }

    /// Whether the widget is interactive and eligible for Tab focus. Defaults to `true`.
    fn active(&self) -> bool {
        true
    }

    /// Draw the widget during screen rendering (`Renderable.render`).
    fn render(
        &mut self,
        host: &dyn ClientHost,
        draw: &mut dyn DrawContext,
        mouse: [f32; 2],
        hovered: bool,
        focused: bool,
        partial_tick: f32,
    ) {
        let _ = (host, draw, mouse, hovered, focused, partial_tick);
    }

    /// Handle a mouse button press (`GuiEventListener.mouseClicked`).
    ///
    /// Return `true` when the widget handled the click.
    fn mouse_clicked(
        &mut self,
        host: &mut dyn ClientHost,
        mouse: [f32; 2],
        button: i32,
        modifiers: u32,
        double_click: bool,
    ) -> bool {
        let _ = (host, mouse, button, modifiers, double_click);
        false
    }

    /// Handle a mouse button release (`GuiEventListener.mouseReleased`).
    ///
    /// Return `true` when the widget handled the release.
    fn mouse_released(
        &mut self,
        host: &mut dyn ClientHost,
        mouse: [f32; 2],
        button: i32,
        modifiers: u32,
    ) -> bool {
        let _ = (host, mouse, button, modifiers);
        false
    }

    /// Handle pointer motion while a mouse button is held (`GuiEventListener.mouseDragged`).
    ///
    /// Return `true` when the widget handled the drag.
    fn mouse_dragged(
        &mut self,
        host: &mut dyn ClientHost,
        mouse: [f32; 2],
        button: i32,
        delta: [f32; 2],
        modifiers: u32,
    ) -> bool {
        let _ = (host, mouse, button, delta, modifiers);
        false
    }

    /// Handle a mouse wheel scroll (`GuiEventListener.mouseScrolled`).
    ///
    /// Return `true` when the widget handled the scroll.
    fn mouse_scrolled(
        &mut self,
        host: &mut dyn ClientHost,
        mouse: [f32; 2],
        scroll: [f64; 2],
        modifiers: u32,
    ) -> bool {
        let _ = (host, mouse, scroll, modifiers);
        false
    }

    /// Handle a key press (`GuiEventListener.keyPressed`).
    ///
    /// Return `true` when the widget handled the key.
    fn key_pressed(
        &mut self,
        host: &mut dyn ClientHost,
        key: &str,
        repeat: bool,
        modifiers: u32,
    ) -> bool {
        let _ = (host, key, repeat, modifiers);
        false
    }

    /// Handle a key release (`GuiEventListener.keyReleased`).
    ///
    /// Return `true` when the widget handled the key release.
    fn key_released(&mut self, host: &mut dyn ClientHost, key: &str, modifiers: u32) -> bool {
        let _ = (host, key, modifiers);
        false
    }

    /// Handle a typed character (`GuiEventListener.charTyped`).
    ///
    /// Return `true` when the widget handled the character.
    fn char_typed(&mut self, host: &mut dyn ClientHost, c: char, modifiers: u32) -> bool {
        let _ = (host, c, modifiers);
        false
    }

    /// Whether this widget wants text input focus priority over screen hotkeys (such as `E` to
    /// close inventory screens) while focused. This does not consume IME preedit; custom widgets
    /// receive committed characters via [`Self::char_typed`]. Defaults to `false`.
    fn captures_input(&self) -> bool {
        false
    }

    /// Called when this widget gains or loses keyboard focus (`GuiEventListener.setFocused`).
    fn set_focused(&mut self, focused: bool) {
        let _ = focused;
    }
}

/// An initialized child listed by [`ScreenWidgets::children`].
#[derive(Clone, Debug, PartialEq)]
// no wire: live screen-local child snapshot containing an opaque ScreenWidgetId.
#[non_exhaustive]
pub struct ScreenChild {
    /// The screen-local handle used to remove the child.
    pub id: ScreenWidgetId,
    /// `[x, y, width, height]` in GUI pixels at registration.
    pub rect: [f32; 4],
    /// The child's label, if it supplies one.
    pub label: Option<Text>,
}

impl ScreenChild {
    /// Build the value exposed by the engine's registrar.
    // hidden: only the engine issues ScreenWidgetId handles for initialized children.
    #[doc(hidden)]
    pub fn new(id: ScreenWidgetId, rect: [f32; 4], label: Option<Text>) -> Self {
        Self { id, rect, label }
    }
}

/// Screen-scoped widget registrar passed to `Hooks::screen_init_post` (`ScreenEvent.Init.Post`).
///
/// Widgets added here live as long as the current screen stays open and are cleared and rebuilt
/// whenever the screen re-initializes (for example on a resize that rebuilds children).
/// Mounting a widget with non-finite bounds returns an ID that is not present in the screen.
// no wire: borrowed registrar scoped to a single `screen_init_post` dispatch.
pub trait ScreenWidgets {
    /// Registered children in initialization order (`ScreenEvent.Init.getListenersList`).
    /// Includes earlier listeners' accepted edits and this listener's staged edits. The pause
    /// screen (`PauseScreen.init`) and the title screen (`TitleScreen.init`, with the engine's
    /// own buttons after Minecraft Realms and its bottom notice last) list their own controls
    /// first, each labelled with its translation key.
    fn children(&self) -> &[ScreenChild];

    /// Mount a vanilla-style push button on the screen (`Screen.addRenderableWidget`).
    fn add_button(&mut self, button: Button) -> ScreenWidgetId;

    /// Mount a vanilla-style single-line text field on the screen (`Screen.addRenderableWidget`).
    fn add_text_field(&mut self, field: TextField) -> ScreenWidgetId;

    /// Mount a custom widget on the screen (`Screen.addRenderableWidget`).
    fn add_widget(&mut self, widget: Box<dyn ScreenWidget>) -> ScreenWidgetId;

    /// Ask the screen to focus `id` when initialization completes (`Screen.setInitialFocus`).
    ///
    /// Returns `true` when `id` names a widget this listener mounted that can take focus.
    fn set_initial_focus(&mut self, id: ScreenWidgetId) -> bool;

    /// Remove a listed child (`ScreenEvent.Init.removeListener`), including one registered by
    /// the game or mounted by an earlier listener. Returns `true` if the child was present.
    /// This listener's removals take effect only if it completes normally.
    fn remove(&mut self, id: ScreenWidgetId) -> bool;
}

#[cfg(test)]
#[path = "widgets_tests.rs"]
mod tests;
