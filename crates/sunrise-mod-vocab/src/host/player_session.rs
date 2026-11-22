//! The `player_session` area: who a player is and how it is connected: name, profile, address, permission, persistent data. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_session` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerSessionBackend {
    /// Side: server.
    /// The IP address the player connects from, without the port. Displaced connected
    /// players keep their address; an acting player with no known connection
    /// has an empty address. None for a stale or unavailable player. The text
    /// borrows the host; copy it only to keep it beyond that borrow.
    fn player_address(&self, player: PlayerRef) -> Option<&str>;
    /// Side: server.
    /// The stable name of a connected or acting player, borrowed from the host.
    /// None for a stale or unavailable player; displaced online players retain their identity.
    fn player_name(&self, player: PlayerRef) -> Option<&str>;
    /// Side: server.
    /// `Player.getGameProfile()`: the UUID, name and login properties (skin
    /// `textures` included) of a connected or acting player, borrowed from the host.
    /// Displaced online players keep their profile. An offline-mode player has no
    /// properties. None for a stale or unavailable player.
    fn player_profile(&self, player: PlayerRef) -> Option<GameProfile<'_>>;
    /// Side: server.
    /// The player's permission level, 0 to 4 (`Player.permissions()`,
    /// `PlayerList.getProfilePermissions`): an op's level, the
    /// single-player owner's 4 with commands allowed. A displaced connected
    /// player keeps their permission level even while their body is unavailable.
    /// None for a stale or unavailable player.
    fn player_permission_level(&self, player: PlayerRef) -> Option<u8>;
    /// Side: server.
    /// `ServerPlayer.isTextFilteringEnabled`: the text-filtering option the player's client
    /// announced (`ClientInformation.textFilteringEnabled`). False for the player running the
    /// server and for an acting player with no connection, as for a client that sent none. None
    /// for a stale or unavailable player.
    fn player_text_filtering(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// Borrows the compound mods keep on a server player (NeoForge's `Entity.getPersistentData`),
    /// saved as `NeoForgeData`. An explicitly empty compound is still present. Returns `None`
    /// if no compound is stored, the player is stale or unavailable, or this is a client.
    /// The borrow lasts only as long as the shared Host borrow; clone explicitly for a snapshot.
    fn player_persistent_data(&self, player: PlayerRef) -> Option<&Compound>;
    /// Side: both.
    /// The brand the player's client announced when it connected (`ServerPlayer.clientBrand`),
    /// empty when it sent none. Displaced connected players keep theirs; an acting player with
    /// no known connection has an empty one. None for a stale or unavailable player.
    /// The brand borrows the host.
    fn player_client_brand(&self, player: PlayerRef) -> Option<&str>;
    /// Side: server.
    /// A shared snapshot, in announcement order, of the plugin channels the client announced
    /// with `minecraft:register` and has not withdrawn with `minecraft:unregister`, during
    /// configuration and play (Bukkit's `Player.getListeningPluginChannels`). Malformed
    /// entries are skipped and at most 128 channels of up to 64 bytes are kept. Empty for a
    /// client that announced none and for an acting player with no known connection; None
    /// for a stale or unavailable player. A snapshot can outlive the host borrow and does
    /// not change when the client registers or unregisters channels later.
    fn player_announced_channels(&self, player: PlayerRef) -> Option<Arc<[String]>>;
    /// Side: server.
    /// Whether `viewer`'s client is shown the player `target` by the rules players are tracked
    /// with (`ServerPlayer.broadcastToPlayer`; Paper's `Player.canSee`): a spectator is seen
    /// only by spectators, and by them only while it watches through its own camera, and a
    /// player hidden with `player_hide` is seen only by players of at least that level.
    /// Distance and chunks are not part of it, so a player out of range is still seen. A player
    /// sees themselves. Refuses with `PlayerWorldRefusal`: `Client`, `Stale` (a handle of the
    /// two no longer identifies a player in the server's list) and `NoConnection` (a fake
    /// player is no one's viewer or target).
    fn player_sees(&self, viewer: PlayerRef, target: PlayerRef)
    -> Result<bool, PlayerWorldRefusal>;
    /// Side: server.
    /// The ping the Tab list shows, in milliseconds (`ServerCommonPacketListenerImpl.latency`:
    /// keep-alive round trips averaged three parts old to one part new). 0 until the first
    /// keep-alive answer, for a fake player and for an acting player with no known connection.
    /// Refuses with `PlayerWorldRefusal`: `Client`, and `Stale` for a handle that no longer
    /// identifies a player.
    fn player_latency(&self, player: PlayerRef) -> Result<i32, PlayerWorldRefusal>;
    /// Side: server.
    /// The permission level `player_hide` set for that player, `None` when they are shown. Kept
    /// by UUID, so a player who rejoined hidden reads it too, and read as stored in a world
    /// nobody else can join yet (it applies once the world is opened to others). Refuses with
    /// `PlayerWorldRefusal`: `Client`, and `Stale` for a handle of an earlier open of the world.
    fn player_hidden(&self, player: PlayerRef) -> Result<Option<u8>, PlayerWorldRefusal>;
    /// Side: server.
    /// The name `player_set_nickname` set for that player, borrowed from the host,
    /// `None` when there is none. Refuses with `PlayerWorldRefusal`: `Client`,
    /// and `Stale` for a stale or offline player.
    fn player_nickname(&self, player: PlayerRef) -> Result<Option<&str>, PlayerWorldRefusal>;
}
/// The handles' writes of the `player_session` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerSessionBackendMut {
    /// Side: server.
    /// Disconnects that player with `reason`, as `/kick` does
    /// (`ServerGamePacketListenerImpl.disconnect`): their client shows it, the log's "lost
    /// connection" line names it, `player_kicked` hears it and a dedicated server records a
    /// `kick` event. The connection closes when the server next polls it, so the calling hook
    /// finishes first; works for the acting player inside their own hook, and without the
    /// world published. A fake (client-less) player leaves through the same removal path:
    /// the leave line is broadcast once and `player_logged_out` fires once at the next poll.
    /// False for a stale or unavailable player, the single-player owner (`/kick` refuses the
    /// owner) and a player whose disconnect is already pending (a second call, also from
    /// `player_kicked`: the first reason stands).
    /// A second disconnect on a guest already leaving is a no-op and keeps the first reason.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    #[serialize_text]
    fn player_disconnect(&mut self, player: PlayerRef, reason: Text)
    -> Result<bool, TextFormError>;
    /// Side: server.
    /// Shows `nickname` in place of that player's name in chat (their
    /// messages, `/me`, `/msg`) and in the Tab list; None shows their name
    /// again. Hovering the shown name gives the real one, which stays in the
    /// log, commands, selectors and the server's events. Kept until a mod
    /// sets it again or the server stops (also if they leave and come back).
    /// Plain text: the mod checks it (its length, other players' names). False when the server list is unavailable,
    /// the handle is stale or the player isn't online.
    fn player_set_nickname(&mut self, player: PlayerRef, nickname: Option<&str>) -> bool;
    /// Side: server.
    /// Hides that player (a vanish) from the players below permission
    /// `level`, or shows them again (None): from the next tick those players
    /// lose them from the Tab list and the world (their client stops
    /// tracking the player, as it does a spectator), and don't read their
    /// joining or leaving; players of `level` or more still see them, and
    /// a player made an operator meanwhile sees them again. Kept by UUID, even
    /// after this handle disconnects (the player rejoins hidden), until a mod sets it again
    /// or the server stops. Works inside a player's hook too; false for a stale
    /// world epoch and on a client.
    fn player_hide(&mut self, player: PlayerRef, level: Option<u8>) -> bool;
    /// Side: server.
    /// Sets one literal NBT `key` in the server player's `NeoForgeData`; `None` removes only
    /// that key. Setting a value creates the compound if absent; removing a missing key does
    /// nothing, and removing the last key leaves a present, empty compound. Other keys are
    /// unchanged. Data is saved with the player, never sent to clients, and raises no notice.
    ///
    /// A death respawn or the first exit from the End carries only `PlayerPersisted`, whatever
    /// its type, as in NeoForge; dimension changes, later exits and teleports keep all keys.
    /// This is for ports; new mods should use attachments.
    ///
    /// Refuses with [`PlayerPersistentDataError`] and writes nothing if the player cannot be
    /// written or the entry cannot be saved. Both living and dead online players on the server
    /// accept edits, matching NeoForge's `Entity.getPersistentData()` and
    /// `ServerPlayer.restoreFrom()`. An open transaction journals the edit; aborting restores
    /// the previous value, including absence of the compound.
    fn player_set_persistent_data_key(
        &mut self,
        player: PlayerRef,
        key: &JavaString,
        value: Option<&Tag>,
    ) -> Result<(), PlayerPersistentDataError>;
}
