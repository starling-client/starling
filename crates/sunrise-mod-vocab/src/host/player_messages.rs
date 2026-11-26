//! The `player_messages` area: chat, action bar, tab list, sidebar, sounds and maps sent to one player. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' writes of the `player_messages` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerMessagesBackendMut {
    /// Side: server.
    /// A system line in that player's chat, also inside a player's hook for
    /// the acting player; false when they aren't on.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    #[serialize_text]
    fn player_tell(&mut self, player: PlayerRef, line: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// A line over that player's hotbar (the action bar), also inside a
    /// player's hook for the acting player; false when they aren't on.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    #[serialize_text]
    fn player_action_bar(&mut self, player: PlayerRef, line: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// The lines over and under that player's list of players (Tab):
    /// `ClientboundTabListPacket`, an empty text for none. False when they
    /// aren't on.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    #[serialize_text]
    fn player_tab_list(
        &mut self,
        player: PlayerRef,
        header: &Text,
        footer: &Text,
    ) -> Result<bool, TextFormError>;
    /// Side: server.
    /// Replaces this player's personal sidebar, or clears it with `None`. Only their client
    /// holds it, until cleared or disconnected. It takes the ordinary sidebar slot and the
    /// slot for their current team color, if any. Later world display packets take back the
    /// slots they write; a score update does not. On clear, the world display returns in the
    /// team slots and, if still held by the personal sidebar, the ordinary slot.
    ///
    /// Returns `Ok(false)` for a stale or offline player without checking the texts. The title
    /// and both fields of the first 15 lines must fit the trusted scoreboard text codec. If
    /// any is refused, the previous sidebar stays unchanged. Clearing an online player's
    /// already-empty personal sidebar returns `Ok(true)` without sending anything. In a
    /// transaction, an accepted change is sent only on commit, if the connection is still live.
    #[forward_api_inputs]
    fn player_set_sidebar(
        &mut self,
        player: PlayerRef,
        sidebar: Option<Sidebar>,
    ) -> Result<bool, TextFormError>;
    /// Side: server.
    /// Sends the player `uuid` the whole picture of map `map_id`: 128 x 128
    /// colours (`MapColor` ids times 4 plus the shade, row by row), scale 0,
    /// locked, no decorations, as `ClientboundMapItemDataPacket`. A filled
    /// map the mod gives with that `minecraft:map_id` then shows it in hand
    /// and in item frames; the server keeps nothing (a map id with no saved
    /// map is never sent again by the game), so send it again after the
    /// player joins. Use ids far from the world's own (they count up from 0).
    /// False for a player not here, or `colors` not 16384 long.
    fn player_send_map(&mut self, player: PlayerRef, map_id: i32, colors: &[u8]) -> bool;
    /// Side: server.
    /// `StopSoundCommand` for one player: their client stops the playing
    /// sounds that match `sound` (an event id) and `source`; `None` matches
    /// any. `true` means the request was accepted, not that a client received or played it.
    /// Inside an open transaction, delivery waits for the owning level's outermost commit in
    /// journal order. A deferred request is discarded on abort or if the player no longer
    /// resolves at commit.
    fn player_stop_sound(
        &mut self,
        player: PlayerRef,
        sound: Option<&str>,
        source: Option<SoundSource>,
    ) -> bool;
    /// Side: server.
    /// `PlaySoundCommand` for one player: plays sound event `sound` at the
    /// player, for them only, under `source`'s volume slider. The seed is the
    /// request level's random `nextLong`, drawn at the call outside a transaction or when
    /// published at the owning level's outermost commit. `true` means the request was accepted,
    /// not that a client received or played it. A deferred request is discarded on abort or if
    /// the player no longer resolves at commit, without drawing a seed.
    /// Registered events go by ID; other valid keys go by name, with volume and pitch as supplied.
    fn player_play_sound(
        &mut self,
        player: PlayerRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool;
}
