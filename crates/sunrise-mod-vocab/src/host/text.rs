//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The methods of the `text` area on both sides, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait TextHost {
    /// Side: both.
    /// `Component.getString()`: resolve translations, arguments, keybinds and
    /// siblings to plain characters. Styles do not add characters; literal
    /// legacy formatting codes remain.
    ///
    /// On the client and its integrated server this uses the client process's
    /// active language, resource-pack layers and key bindings, not a remote
    /// player's language. A dedicated server uses built-in English, then
    /// adds English from its enabled mods and data packs once before
    /// `server_starting`. Keybind components there return the literal ID.
    /// On a dedicated server, a component-valued translation replaces its
    /// arguments and fallback.
    /// Available outside drawing, including setup and pack discovery (before
    /// language resources load, translations resolve to their fallback or key).
    /// In-memory text reads do not enforce the 512-node serialization depth cap
    /// or `ExtraCodecs` profile/property bounds.
    #[forward_api_inputs]
    fn text_string(&self, text: &Text) -> String;
    /// Side: both.
    /// Reads a text component as `/tellraw` and `ComponentSerialization.CODEC` take it: one
    /// JSON or SNBT value (`"plain"`, `{"text":"Hi","color":"gold"}`, `["a",{"text":"b"}]`,
    /// `translate`, `extra`, click and hover events), with nothing but whitespace after it
    /// (Paper's `GsonComponentSerializer.deserialize`). None when it does not parse as one component:
    /// an object needs one content key (a `type` binds alone, a null `type` counts as absent),
    /// `text` must be a string, `extra` a non-empty list, and a style field that is present and
    /// wrong refuses the whole component. Command, URL and page click actions use `command`, `url`
    /// and `page`, respectively; a legacy `value`-only click for those actions is refused.
    /// `open_url` requires a well-formed HTTP(S) URI; a malformed URI or another scheme
    /// refuses the component (`Ok(None)`).
    /// `show_item` refuses `minecraft:air`; a component removal (`!id`) requires an object value.
    /// An object whose only content key is `index` is refused,
    /// as in vanilla's codec; Sunrise follows vanilla there.
    fn text_from_json(&self, json: &str) -> Result<Option<Text>, TextFormError>;
    /// Side: both.
    /// Encode one component with `ComponentSerialization.CODEC` and `NbtOps`.
    /// An unstyled literal with no siblings encodes as a string. Translations and
    /// styles are stored, not resolved. Refuses malformed fields and nesting
    /// beyond vanilla NBT's 512 containers.
    #[forward_api_inputs]
    fn text_to_nbt(&self, text: &Text) -> Result<Tag, TextFormError>;
    /// Side: both.
    /// Read a component with `ComponentSerialization.CODEC` and `NbtOps`.
    /// A string is literal, including `§` codes. Numeric style flags are true when
    /// nonzero, and a custom click payload keeps its original NBT tag types.
    /// `Ok(None)` means the tag does not decode as a component; nesting beyond the
    /// 512 containers allowed by vanilla NBT returns [`TextFormError::TooDeep`].
    #[forward_api_inputs]
    #[forward_api_output]
    fn text_from_nbt(&self, tag: &Tag) -> Result<Option<Text>, TextFormError>;
    /// Side: both.
    /// Reads legacy formatting codes (`§a`, `§l`, `§r`...) as the font draws them
    /// (`Style.applyLegacyFormat`; Bukkit's `ChatColor` strings): a colour code resets the
    /// decorations, `§r` resets everything, an unknown code is dropped. `&` codes are not
    /// read; a mod that takes them replaces `&` with `§` first, as
    /// `ChatColor.translateAlternateColorCodes` does.
    fn text_from_legacy(&self, legacy: &str) -> Result<Text, TextFormError>;
}
