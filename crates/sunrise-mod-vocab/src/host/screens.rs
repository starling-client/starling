//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `screens` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ScreensClientHost {
    /// Submit a notification to the window's toast manager. The next toast update offers
    /// it to `toast_add` as `ToastKind::Mod` before inserting or replacing anything.
    /// An accepted matching kind restarts the display time in place; a veto leaves any
    /// existing toast untouched. Calls from `toast_add` itself wait for the following update.
    /// Refuses the whole request if its text or item icon cannot be represented.
    fn add_toast(&self, toast: ModToast) -> Result<(), AddToastError>;
    /// Side: client.
    /// Holds new toasts back while `held`, or lets them go: they wait in the toast queue, as
    /// vanilla's do while no slot is free (`ToastManager.freeSlotCount`), and show once no
    /// enabled mod holds them, instead of being dropped as a canceled `toast_add` drops them.
    /// Toasts already on screen go on and leave as usual. Dynamic FPS holds them while the
    /// window is in the background. Each mod's hold is its own; a disabled mod's holds
    /// nothing. False, changing nothing, outside a mod's call.
    fn set_toasts_held(&self, held: bool) -> bool;
    /// A copy of the open `world_preset_editor` screen's working dimension stems.
    /// Stays available to screen hooks while `world_preset_editor` is open alongside the bound
    /// [`PresetEditor`].
    fn preset_editor_dimensions(&self) -> Option<Compound>;
    /// Replace the open `world_preset_editor` screen's working dimension stems. Requires an
    /// overworld, valid dimension ids and type/generator declarations. Returns false outside
    /// `world_preset_editor` or for malformed data; Done commits, Cancel and Escape discard the
    /// copy. Stays available to screen hooks alongside [`PresetEditorDraft::set_dimensions`].
    fn set_preset_editor_dimensions(&self, dimensions: Compound) -> bool;
    /// Side: client.
    /// The position of the currently shown mod menu opening. Use
    /// [`crate::MenuScreenOpen::pos`] while constructing its screen instead.
    /// An explicit absent position from the server stays `None`.
    fn screen_pos(&self) -> Option<[i32; 3]>;
    /// Side: client.
    /// Where the open screen draws outside its `rect`, and where mods draw
    /// over it: the recipe book's panel, the creative tab rows, the active
    /// effects beside the inventory, then whatever `screen_exclusion_areas`
    /// adds. Each is `[x, y, width, height]` in GUI pixels; an empty or
    /// non-finite one is dropped. A mod that lays out its own panels
    /// (a recipe viewer) keeps them clear of these. Client-side; empty with
    /// no screen up.
    fn screen_exclusion(&self) -> Vec<[f32; 4]>;
    /// Side: client.
    /// The full stack in the slot under the pointer on the open container or creative screen
    /// (`AbstractContainerScreen.getHoveredSlot().getItem()`), including creative catalog
    /// display slots; `None` over no slot, an empty slot, or with no container or creative
    /// screen open.
    fn screen_hovered_stack(&self) -> Option<ItemStack>;
    /// Side: client.
    /// Where the shown mod menu screen for `menu` puts menu slot `slot` from the next
    /// frame (`Slot.x`, `Slot.y` in GUI pixels from the screen's top left, the item's top left
    /// corner): where `set_screen_slot` last put it, else where the screen declared it. `slot`
    /// ranges over `0..(container_len + player_slots)` as `set_screen_slot` documents. None
    /// for an inactive/hidden slot, a slot the menu does not have, another menu type's screen,
    /// or no shown mod screen.
    fn screen_slot(&self, menu: &str, slot: usize) -> Option<[f32; 2]>;
    /// Side: client.
    /// Moves menu slot `slot` of the open screen of mod menu type `menu` to `pos`, or
    /// deactivates/hides it with None, as a Java screen that lays its slots out again sets
    /// `Slot.x`/`Slot.y` or toggles `Slot.isActive()` (Tinkers' station layouts). `slot`
    /// ranges over `0..(container_len + player_slots)`, in menu order
    /// (`AbstractContainerMenu.addStandardInventorySlots`): `0..container_len` indexes
    /// `MenuScreenLayout::slots` (including slots declared `None` at open, which start
    /// inactive and may be activated later with `Some([x, y])`), and the `player_slots` the
    /// menu declared (`MenuScreenOpen::player_slots`) follow: the main inventory's three rows,
    /// then the hotbar. A player slot starts at the screen's `inventory_x`/`inventory_y` grid.
    /// From the next frame the slot is a vanilla slot at that position: the hovered slot (the
    /// 18 by 18 box around it), its highlight, its contents, its empty-slot icon and the
    /// recipe book's ghost items; a hidden one is none of these, and is neither a drag target
    /// nor swapped by a number key. A position outside the image is allowed. The change lasts for this
    /// opening: closing the screen and opening it again (or a replacement, even one that reuses
    /// the same window id) starts every container and player slot from its declared place,
    /// shown. Any mod may read, move, activate or hide slots of the open screen for `menu`.
    /// This is client-only and the server never consults it: a locally hidden slot keeps its
    /// item and a shift-click from another slot may still fill it, which is the menu's own
    /// quick-move answer; nothing on the client filters that. It takes `&self`, so a screen
    /// callback may call it every frame; repeating a slot's current place changes nothing and
    /// costs no layout work. Each [`ScreenSlotRefusal`] leaves the layout unchanged.
    fn set_screen_slot(
        &self,
        menu: &str,
        slot: usize,
        pos: Option<[f32; 2]>,
    ) -> Result<(), ScreenSlotRefusal>;
    /// Side: client.
    /// Requests vanilla's menu background blur for this frame, with a per-frame guard
    /// so blur is applied at most once per frame. Returns true if blur was newly
    /// requested this frame, or false if it was already applied/requested this frame.
    /// API @API:screen_transparent_background@.
    fn apply_screen_blur(&self) -> bool;
    /// Side: client.
    /// The user's configured menu background blur radius (`menuBackgroundBlurriness` in options.txt,
    /// usually 0.0 to 10.0; default 5.0). 0.0 means blur is turned off. API @API:screen_transparent_background@.
    fn menu_blur_radius(&self) -> f32;
    /// Side: client.
    /// Opens a screen of mod `mod_id`'s own through the normal screen-transition
    /// path (`Gui.setScreen`): `screen_opening` and `screen_closing` fire for the
    /// transition and `screen_init_post` runs once it opens, so the mod mounts
    /// widgets in `screen_init_post`, draws in `screen_render_post`, and takes its
    /// clicks, scroll, keys, and typing there; the mouse is freed, and Escape
    /// closes it. Every mod's own screen has the `ScreenView.id` `mod` and sets
    /// `ScreenView.mod_screen` to its owning mod id, so a mod's screen hooks
    /// check `event.screen.mod_screen == Some("my_mod")` (or live `mod_screen()`
    /// to query the currently open screen). It
    /// doesn't pause a single-player game (as the inventory doesn't). Can replace
    /// any open vanilla screen, title screen, overlay (book, sign editor, dialog,
    /// or prompt), or another mod's screen, running the replaced screen's close
    /// cleanup once hooks allow the transition. Replacing a server-pack prompt
    /// leaves the server's request unanswered, as in vanilla `Gui.setScreen`.
    /// Returns `true` when `mod_id`'s
    /// screen is open, and `false` when cancelled or redirected by
    /// `screen_opening`, while a world or connection is loading, or without a
    /// client window (a headless/dedicated server, the server thread, or
    /// guest-acting contexts). A gamepad can close it (B, Start) and activate
    /// focused mounted controls. That activation does not dispatch screen mouse hooks.
    fn mod_screen_open(&mut self, mod_id: &str) -> bool;
    /// Side: client.
    /// Closes mod `mod_id`'s own screen, if it is open: to the Mods screen when its
    /// [`ConfigScreenFactory`](crate::ConfigScreenFactory) opened it, otherwise as Escape
    /// closes a screen (back to the game, or to the title screen with no world).
    fn mod_screen_close(&mut self, mod_id: &str);
    /// Side: client.
    /// Takes (`grab` true) or gives back (`grab` false) the pointer for the calling mod's own
    /// open screen: the cursor is hidden and held, and the screen hears the device's relative
    /// motion only as `screen_pointer_motion`, unscaled (not as `screen_mouse_moved_pre`, and
    /// never as camera turns), while it stays a screen that draws, takes keys (Escape
    /// included) and has widgets. Vanilla's own screens never hold a grabbed pointer
    /// (`MouseHandler.grabMouse` closes the screen), so this is a Sunrise capability with no
    /// vanilla limit. A mod that wants click-to-capture grabs from its click hook and releases
    /// with `false`.
    ///
    /// Returns `true` when the pointer is (or, for `false`, is no longer) held by this mod's
    /// screen. Returns `false` and changes nothing when the open screen belongs to another
    /// mod, when no screen is open, or, for a grab, when the window does not have focus
    /// (`MouseHandler.grabMouse` never grabs an inactive window) or while a context menu is
    /// open ([`open_context_menu`](Self::open_context_menu)). A grab the system then
    /// refuses shows as `pointer_grabbed` false. Releasing a screen that holds no grab returns `true`.
    ///
    /// The grab ends by itself when the screen closes by any route, when the window loses
    /// focus, and when the mod unloads. Refocusing does not take it back: call again. The
    /// grab takes effect at the start of the next input event or frame, and
    /// [`pointer_grabbed`](Self::pointer_grabbed) already reports it.
    fn set_pointer_grab(&self, grab: bool) -> bool;
    /// Side: client.
    /// Whether the calling mod's own open screen holds the pointer grab
    /// ([`set_pointer_grab`](Self::set_pointer_grab)). `false` once the grab has ended by
    /// itself, and `false` for a mod whose screen is not the open one.
    fn pointer_grabbed(&self) -> bool;
    /// Side: client.
    /// Opens a context menu of `entries` with its top left at `at` (GUI pixels, usually the
    /// pointer of the right-click that asked for it), over the open screen: drawn above every
    /// widget, tooltip and clip, moved left or up to stay inside the window, and taking all
    /// mouse and keyboard input until it closes. A click on a row picks it, a click elsewhere
    /// or Escape closes it; the arrow keys move between rows, Right or Enter opens a nested
    /// menu, Left closes one. Picking an action closes the menu, then runs it. Replaces a menu
    /// already open. Returns false, with nothing opened, when no screen is open, for an empty
    /// list, while a pointer grab is active or pending ([`set_pointer_grab`](Self::set_pointer_grab)),
    /// or outside a mod's call; while a menu is open, `set_pointer_grab(true)` returns false
    /// until the menu closes. Closes with the screen. Vanilla and NeoForge have no such widget;
    /// this is Sunrise's.
    fn open_context_menu(&mut self, at: [f32; 2], entries: Vec<ContextMenuEntry>) -> bool;
    /// Side: client.
    /// Closes the open context menu without picking anything; false when none is open.
    fn close_context_menu(&mut self) -> bool;
    /// Side: client.
    /// Closes the open screen as its Escape key does: a container screen
    /// closes as the player's container close (`ServerboundContainerClosePacket`
    /// with the menu's id, then the client menu closed), a pause menu closes
    /// to the game, and another screen runs its own close. The sign editor
    /// sends its sign update (`ServerboundSignUpdatePacket`), as Escape does.
    /// A book closes with nothing sent; a lectern's book closes its menu
    /// (`ServerboundContainerClosePacket`). A confirmation, a server's pack
    /// prompt or a dialog open over another screen is the one that closes.
    /// Declining a required pack disconnects rather than returning to the
    /// screen underneath. The screen hooks fire as for Escape.
    /// The death screen, the code of conduct and a dialog Escape cannot
    /// close do not close this way. False when no screen is open (or,
    /// outside a world, when it cannot act).
    fn close_screen(&mut self) -> bool;
    /// Side: client.
    /// Opens the player's inventory as the inventory key does: locally, with
    /// no packet (the player menu is always open on the server). Riding a
    /// vehicle with its own inventory sends the open-inventory player command
    /// (`ServerboundPlayerCommandPacket`), as the key does. False with a
    /// screen already open, for a spectator, in a portal transition, or with
    /// no world.
    fn open_inventory(&mut self) -> bool;
    /// Side: client.
    /// The sign the sign editor is on, its block position and side; None when
    /// no sign editor is open or outside a world.
    fn sign_editor(&self) -> Option<SignEditView>;
    /// Side: client.
    /// The mod whose own screen is currently open, if one is (Escape, or
    /// another screen replacing it, closes it). This is a live query for the
    /// screen open right now; inside `screen_closing` or `screen_*_post` hooks,
    /// read `event.screen.mod_screen` to identify the screen that received the
    /// event even after a synchronous replacement or close.
    fn mod_screen(&self) -> Option<&str>;
    /// Side: client.
    /// The open menu's title (the block's name, or the one it was given),
    /// as plain text.
    fn screen_title(&self) -> Option<String>;
    /// Side: client.
    /// The last window focus state reported by the window system.
    fn window_focused(&self) -> bool;
    /// Side: client.
    /// Whether the pointer is over the game window, focused or not, as the window system last
    /// reported its enter and leave. Vanilla 26.3 has a cursor-enter event only; the leave
    /// comes from the window system. With [`Self::window_focused`] false, true means the window
    /// is hovered in the background. False before the pointer has entered the window.
    fn window_hovered(&self) -> bool;
    /// Side: client.
    /// Whether the window is fully hidden (occluded): covered by other windows, on another
    /// desktop, or minimized, where the window system reports it (some report only minimizing,
    /// and some nothing). A hidden window's frames are not seen, so a mod may draw less.
    fn window_occluded(&self) -> bool;
    /// Side: client.
    /// The same screen identifier provided to screen hooks.
    fn observed_screen_id(&self) -> Option<ScreenId>;
    /// Side: client.
    /// Whether the client draws its debug overlay this frame. Always-on debug entries can
    /// make it visible with F3 off. F1 hides the overlay unless a screen is open.
    fn debug_overlay_visible(&self) -> bool;
    /// Side: client.
    /// Whether the resource reload overlay (vanilla's `LoadingOverlay`) is up: from a resource
    /// pack reload's start, whether the player's own selection or a server's packs, until the
    /// overlay has faded out after the reload (`Gui.overlay()`). While it
    /// covers the whole window, between its fade-in and its fade-out, the game draws the
    /// overlay alone and renders no world or screen behind it.
    fn reload_overlay_visible(&self) -> bool;
    /// Side: client.
    /// Adds the calling mod's `text` to the end of F3's frame rate line (vanilla's
    /// `DebugEntryFps`, the line that reads `60 fps T: 120 ...`), after a space, where a mod
    /// shows a state of its own (Dynamic FPS's reduced frame rate) without an entry of its own
    /// that starts off. It stays until the mod sets another text, or `None` or an empty text,
    /// which takes it away; set it when the state changes, not every frame. Several mods' texts follow in mod
    /// load order, and a disabled mod's is left out. Legacy colour codes (§) work; line breaks
    /// become spaces. False, setting nothing, outside a mod's call.
    fn set_debug_fps_text(&self, text: Option<&str>) -> bool;
    /// Side: client.
    /// Whether a specific key is currently held down.
    /// `key` is the vanilla physical key or mouse button name (`key.keyboard.left.shift`,
    /// `key.mouse.left`, `key.mouse.4`, for example).
    /// Unknown names are not down. Modifier combinations use `ClientHost::modifiers`.
    fn is_key_down(&self, key: &str) -> bool;
    /// Side: client.
    /// Bounding rectangle `[x, y, width, height]` in GUI pixels of a HUD layer's
    /// geometry after viewport and scissor clipping, before other layers can cover it.
    /// Use a vanilla id such as `"minecraft:effects"` or a registered mod layer id.
    /// Returns `Ok(None)` if the layer has not painted visible geometry, including
    /// before it draws, if it is canceled, or outside the HUD pass (including screen render hooks).
    /// Returns `Err(LevelReadError::Unavailable)` outside a client window.
    fn hud_layer_bounds(&self, layer: &str) -> Result<Option<[f32; 4]>, LevelReadError>;
}
