//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_input` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ClientInputClientHost {
    /// Side: client.
    /// Where the mouse is, in GUI pixels.
    fn cursor(&self) -> [f32; 2];
    /// Side: client.
    /// Resolve `owner`'s registered mapping `id` once after registration, usually in `client_setup`.
    /// Keep the handle for subsequent calls; save the owner and id, not the handle. Any mod can
    /// resolve, read, rebind or reset any registered mapping.
    /// Returns [`ModKeyRefusal::NotRegistered`] for an unknown owner or id and
    /// [`ModKeyRefusal::Unavailable`] when there is no client backend in this host.
    fn mod_key_mapping(&self, owner: &str, id: &str) -> Result<ModKeyMappingRef, ModKeyRefusal>;
    /// Side: client.
    /// Playing, with nothing over the world: keys and zoom apply.
    fn playing(&self) -> bool;
    /// Side: client.
    /// Resolve a vanilla mapping name such as `key.jump`, not a binding name such as
    /// `key.keyboard.space`. Keep the handle for subsequent reads and writes. An unknown name returns
    /// [`MappingError::Unknown`]; a host or phase without client input returns
    /// [`MappingError::Unavailable`].
    fn vanilla_key_mapping(&self, name: &str) -> Result<VanillaKeyMappingRef, MappingError>;
    /// Side: client.
    /// Milliseconds on the game's own clock (`Util.getMillis`), counting from the game's start:
    /// what a text field's cursor blinks by and what tells a double click from two clicks.
    /// Only differences between two readings mean anything.
    fn millis(&self) -> u64;
    /// Side: client.
    /// The current window modifier keys in the same GLFW encoding as input hooks.
    /// Cleared when the window loses focus; available even outside a world.
    fn modifiers(&self) -> ClientModifiers;
    /// Side: client.
    /// The clipboard's text (`KeyboardHandler.getClipboard`), empty when it holds none,
    /// while the game handles a key, mouse or typed-text event: a mod's key or screen hook
    /// pasting into its own field. None at any other moment (a tick, a frame, a packet),
    /// so no mod reads the clipboard without the player doing something, and None on a
    /// server.
    fn clipboard_text(&self) -> Option<String>;
    /// Side: client.
    /// Puts `text` on the clipboard (`KeyboardHandler.setClipboard`) on the same terms as
    /// `native_clipboard_text` reads it. False, with nothing copied, at any other moment or
    /// on a server; empty text copies nothing, as vanilla's.
    fn set_clipboard_text(&self, text: &str) -> bool;
}

/// The live client reads behind [`VanillaKeyMappingRef`] and [`ModKeyMappingRef`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait ClientInputClientBackend {
    /// Side: client.
    /// The current binding name (`key.keyboard.w`, `key.mouse.left`, or
    /// `key.keyboard.unknown` when unbound), borrowed for the life of the host borrow.
    /// Reads through a foreign client return [`MappingError::Stale`]; an unavailable
    /// backend returns [`MappingError::Unavailable`].
    fn vanilla_key_mapping_binding(
        &self,
        mapping: VanillaKeyMappingRef,
    ) -> Result<&str, MappingError>;
    /// Side: client.
    /// The logical held state (`KeyMapping.isDown`) of any resolved vanilla mapping, including
    /// controller input, toggles and logical overrides.
    /// Stale and unavailable handles are refused as for [`VanillaKeyMappingRef::binding`].
    fn vanilla_key_mapping_is_down(
        &self,
        mapping: VanillaKeyMappingRef,
    ) -> Result<bool, MappingError>;
    /// Side: client.
    /// Whether the current binding's key or mouse button is physically down. Ignores
    /// toggles, conflict context, modifiers and logical overrides. False when the binding
    /// has no physical key or button. Stale and unavailable handles are refused as for
    /// [`VanillaKeyMappingRef::binding`].
    fn vanilla_key_mapping_physical_down(
        &self,
        mapping: VanillaKeyMappingRef,
    ) -> Result<bool, MappingError>;
    /// Side: client.
    /// Builds the current binding's Controls label on demand (`KeyMapping.getTranslatedKeyMessage`).
    /// An invalid component returns [`MappingError::Label`]; stale and unavailable handles
    /// are refused as for [`VanillaKeyMappingRef::binding`].
    fn vanilla_key_mapping_translated_key_message(
        &self,
        mapping: VanillaKeyMappingRef,
    ) -> Result<Text, MappingError>;
    /// Side: client.
    /// Lends the current binding name and returns its saved or default modifiers. The name is
    /// borrowed for the host borrow. A replaced client or roster returns [`ModKeyRefusal::Stale`];
    /// an unavailable backend returns [`ModKeyRefusal::Unavailable`].
    fn mod_key_mapping_binding(
        &self,
        mapping: ModKeyMappingRef,
    ) -> Result<(&str, ClientModifiers), ModKeyRefusal>;
    /// Side: client.
    /// Whether the mapping is held (`KeyMapping.isDown`): its raw held state is true and its
    /// conflict context and bound modifiers are active. Opening a screen or overlay clears
    /// held state via `KeyMapping.releaseAll`; this read does not add an extra screen-open
    /// suppression beyond the mapping's conflict context.
    /// Refusals are as for [`ModKeyMappingRef::binding`].
    fn mod_key_mapping_is_down(&self, mapping: ModKeyMappingRef) -> Result<bool, ModKeyRefusal>;
    /// Side: client.
    /// Builds the current Controls label on demand, with the bound modifiers in Controls order.
    /// Returns [`MappingError::Stale`] for a replaced client or roster, [`MappingError::Unavailable`]
    /// for an unavailable backend, or [`MappingError::Label`] for an invalid component.
    fn mod_key_mapping_translated_key_message(
        &self,
        mapping: ModKeyMappingRef,
    ) -> Result<Text, MappingError>;
}

/// The live client writes behind [`VanillaKeyMappingRef`] and [`ModKeyMappingRef`].
// hidden: a handle's backend, reached through `ClientBackendMut`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait ClientInputClientBackendMut {
    /// Side: client.
    /// Holds any resolved vanilla mapping down or lets it go (`KeyMapping.setDown`), without
    /// adding a click. For a hold-style mapping this replaces its logical state until the next
    /// input for that mapping. It does not press or release the physical key or add a separate
    /// hold that can outlast its release. Sneak, sprint, attack and use with their toggle option
    /// on flip only on `true`, as `ToggleKeyMapping.setDown` does.
    ///
    /// In a world, the client tick's key step acts on gameplay mappings with all its checks: a
    /// held use repeats every four ticks, and a held attack goes on breaking the block. With a
    /// screen or overlay open this still updates the mapping, but the game does not poll
    /// keybinds. Opening a screen releases the mappings (`KeyMapping.releaseAll`).
    /// `key_mapping_pre` is not asked: it is about physical keys.
    ///
    /// An unavailable backend returns [`MappingError::Unavailable`], and a handle from
    /// another client returns [`MappingError::Stale`].
    fn vanilla_key_mapping_set_down(
        &mut self,
        mapping: VanillaKeyMappingRef,
        down: bool,
    ) -> Result<(), MappingError>;
    /// Side: client.
    /// Adds one click to any resolved vanilla mapping (`KeyMapping.click`), without holding it.
    /// In a world, an attack, use or pick click acts on a subsequent client tick, through the
    /// checks of `startAttack` and `startUseItem` (the miss cooldown, busy hands, the item and
    /// the target under the crosshair), sending the same packets as the player's own click.
    /// Inventory and screen-opening mappings select a hotbar slot, drop an item, open the
    /// inventory, swap hands or open chat only when the keybind tick consumes the click.
    /// With a screen or overlay open the click still counts, but keybinds are not polled.
    /// `interaction_key_mapping_triggered` fires at the action as for
    /// the player's click; `key_mapping_pre` is not asked.
    ///
    /// Attack, use, pick, inventory, swap-hands and drop clicks drain completely on the client
    /// tick (`while (key.consumeClick())`), with no fixed per-tick or queue-length cap. Each
    /// hotbar key keeps its own click counter and a selection drain consumes one click per key
    /// in order from key 1 to key 9 (`if (keyHotbarSlots[i].consumeClick())`), so the highest
    /// key clicked wins and further clicks wait for later drains.
    ///
    /// Refusals are as for [`VanillaKeyMappingRef::set_down`].
    fn vanilla_key_mapping_click(
        &mut self,
        mapping: VanillaKeyMappingRef,
    ) -> Result<(), MappingError>;
    /// Side: client.
    /// Takes one pending click (`KeyMapping.consumeClick`), returning `false` if none remain.
    /// Reads and decrements the raw `KeyMapping` click count without clearing or suppressing
    /// it when a screen is open. Physical presses, auto-repeats and mouse presses add clicks
    /// when no screen is open and the mapping's context and modifiers are active; window and
    /// control-socket input count alike, and opening a screen or overlay drops pending clicks
    /// via `KeyMapping.releaseAll`.
    /// Refusals are as for [`ModKeyMappingRef::binding`].
    fn mod_key_mapping_consume_click(
        &mut self,
        mapping: ModKeyMappingRef,
    ) -> Result<bool, ModKeyRefusal>;
    /// Side: client.
    /// Binds the mapping to `key` (`key.keyboard.z`, `key.mouse.left`, or `key.keyboard.unknown`
    /// for none) with shift, control and alt modifiers, as Controls does. A modifier bound to
    /// itself is the key, not a modifier held with itself. The new binding answers from the next
    /// input and is saved to `sunrise-mods.txt`. Any mod can rebind any registered mapping.
    /// In addition to the refusals of [`ModKeyMappingRef::binding`], returns
    /// [`ModKeyRefusal::UnknownKey`] or [`ModKeyRefusal::UnknownModifiers`]. No refusal changes it.
    fn mod_key_mapping_set_binding(
        &mut self,
        mapping: ModKeyMappingRef,
        key: &str,
        modifiers: ClientModifiers,
    ) -> Result<(), ModKeyRefusal>;
    /// Side: client.
    /// Restores the declared default key and modifiers as Controls' Reset does and saves
    /// `sunrise-mods.txt`. Refusals are as for [`ModKeyMappingRef::set_binding`], except this
    /// operation has no key or modifier arguments to validate.
    fn mod_key_mapping_reset_binding(
        &mut self,
        mapping: ModKeyMappingRef,
    ) -> Result<(), ModKeyRefusal>;
}
