//! Read-only client session and local-player status.

use crate::{BlockPos, GameMode, Identifier, Text, TextFormError, Vec3};
use core::ops::ControlFlow;

/// Why a client-session read could not run.
// no wire: this refusal is returned by in-process client-session reads.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientSessionReadError {
    /// The current phase or backend does not allow this read.
    Unavailable,
    /// A text in the read could not be carried to the API (`TextFormError`).
    Text(TextFormError),
}

impl From<TextFormError> for ClientSessionReadError {
    fn from(error: TextFormError) -> Self {
        Self::Text(error)
    }
}

impl crate::NoBackend for ClientSessionReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

// no wire: borrows the current client connection and integrated world.
/// The connected server, not the reconnect target of an earlier session.
/// Its text borrows the host; copy it only to retain it beyond that borrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ClientServer<'a> {
    /// The integrated world's current display name. External connections do not retain
    /// a server display name; this is `None`, never the connecting player's name.
    pub name: Option<&'a str>,
    /// The joined address, with friend-invite secrets redacted. Memory
    /// connections have no address (`None`), not a made-up socket address.
    pub address: Option<&'a str>,
    /// This window owns the connected integrated server. A loopback address
    /// alone does not make this true.
    pub integrated_local: bool,
    /// Folder name relative to the instance's saves directory for the owned
    /// integrated world. Renaming the world in-game leaves this key unchanged.
    /// Connections to a server this window does not own return `None`.
    pub save_folder: Option<&'a str>,
}

/// The local player's item use as the client sees it (`LivingEntity.isUsingItem`,
/// `getUsedItemHand`, `getTicksUsingItem`, `getUseItemRemainingTicks`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ItemUse {
    /// The hand holding the item in use.
    pub hand: crate::Hand,
    /// Ticks the item has been used so far.
    pub ticks_used: i32,
    /// Ticks left until the use completes; with `ticks_used`, the item's use duration.
    pub ticks_remaining: i32,
}

impl ItemUse {
    /// An item use of `hand`, `ticks_used` ticks old with `ticks_remaining` left.
    pub fn new(hand: crate::Hand, ticks_used: i32, ticks_remaining: i32) -> Self {
        Self {
            hand,
            ticks_used,
            ticks_remaining,
        }
    }
}

/// Existing local frame counters, sampled only when requested by a mod.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientFrameStats {
    /// The game's most recently computed frames per second (updated about once
    /// a second), not an inverse of a single frame time.
    pub fps: f32,
    /// Frames rendered by this window since it started, including menus.
    pub rendered_frames: u64,
    /// The most recent completed frame's wall interval in nanoseconds, including
    /// waits. `None` before a frame sample exists or after the chart was reset.
    pub frame_time_ns: Option<u64>,
}

/// The title, subtitle and action bar on screen now, read with
/// `ClientSession::client_titles`: what this client has been sent, changing as the server
/// sends titles.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientTitles {
    /// The title; `None` once its timer has run out.
    pub title: Option<Text>,
    /// The subtitle; shown only with a title.
    pub subtitle: Option<Text>,
    /// The action bar; `None` once its timer has run out.
    pub action_bar: Option<Text>,
}

impl ClientTitles {
    /// Titles from their parts.
    pub fn new(title: Option<Text>, subtitle: Option<Text>, action_bar: Option<Text>) -> Self {
        Self {
            title,
            subtitle,
            action_bar,
        }
    }
}

/// One boss bar the window shows, read with `ClientSession::client_boss_bars`. Unlike
/// `ClientBossBar`, the bar a `customize_boss_event_progress` event hands a mod, this is the
/// bar as the window holds it: name and progress only.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientBossBarEntry {
    /// The bar's name.
    pub name: Text,
    /// 0 to 1, as drawn (the lerped value).
    pub progress: f32,
}

impl ClientBossBarEntry {
    /// A boss bar entry from its parts.
    pub fn new(name: Text, progress: f32) -> Self {
        Self { name, progress }
    }
}

/// The local player's current block-breaking operation. An unavailable player
/// returns `None` from the handle, distinct from [`Self::Idle`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockBreakingState {
    /// No block is currently being broken.
    Idle,
    /// The client's current breaking target and predicted progress.
    Breaking {
        /// The target block.
        position: BlockPos,
        /// The accumulated breaking fraction, as the client holds it.
        progress: f32,
        /// The visible crack stage, 0 through 9; absent while progress is zero.
        stage: Option<u8>,
    },
}

/// The live player list as the Tab overlay draws it (`PlayerTabOverlay`), read with
/// [`crate::ClientSession::client_tab_list`]. It reflects what the server has sent.
// no wire: the view lends the current list and overlay text in-process.
pub trait ClientTabList {
    /// Visit the listed players in drawn order (`getPlayerInfos`: higher list order first,
    /// then non-spectators before spectators, team and profile name), at most the 80 it draws.
    /// Each row is valid only during its callback. Return `Break(())` to stop; an empty list
    /// returns `Continue(())` without calling the visitor.
    fn visit_entries(
        &self,
        visit: &mut dyn FnMut(&dyn ClientTabListEntry) -> ControlFlow<()>,
    ) -> ControlFlow<()>;

    /// Copy the header the server set. `Ok(None)` means absent or cleared; a conversion
    /// refusal affects only this copy.
    fn copy_header(&self) -> Result<Option<Text>, TextFormError>;

    /// Copy the footer the server set. `Ok(None)` means absent or cleared; a conversion
    /// refusal affects only this copy.
    fn copy_footer(&self) -> Result<Option<Text>, TextFormError>;
}

/// One listed player (`PlayerInfo`) as the Tab overlay shows it.
// no wire: a row lends the player's current information for one callback.
pub trait ClientTabListEntry {
    /// Borrow the profile name (`GameProfile.name`); copy it only if retaining it.
    fn name(&self) -> &str;

    /// The profile UUID.
    fn uuid(&self) -> u128;

    /// The game mode the server reported.
    fn game_mode(&self) -> GameMode;

    /// The latency in milliseconds the ping icon shows; negative when unknown.
    fn latency(&self) -> i32;

    /// Copy the name as drawn (`getNameForDisplay`): the display name the server sent, or
    /// the profile name with its team's prefix, suffix and colour; italic for a spectator.
    /// A conversion refusal affects only this copy; there is no plaintext fallback.
    fn copy_display(&self) -> Result<Text, TextFormError>;
}

/// The locator bar's tracked waypoints (`ClientWaypointManager`), read with
/// `ClientSession::client_waypoints`: what this client has been sent, changing as the server
/// tracks and untracks waypoints. With a camera entity the read follows
/// `ClientWaypointManager.forEachWaypoint`: farthest first by the vanilla squared distance,
/// ties in no promised order; without one, in no promised order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientWaypoints {
    /// The tracked waypoints, farthest from the camera entity first by the vanilla squared
    /// distance (ties in no promised order); no promised order without a camera entity.
    pub entries: Vec<ClientWaypointEntry>,
}

impl ClientWaypoints {
    /// Waypoints from their entries.
    pub fn new(entries: Vec<ClientWaypointEntry>) -> Self {
        Self { entries }
    }
}

/// One tracked waypoint (`TrackedWaypoint`) as the locator bar holds it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientWaypointEntry {
    /// The waypoint's identifier: the tracked entity's UUID, or its name when it has no entity.
    pub id: ClientWaypointId,
    /// What the waypoint points at: a block, a chunk, a bearing, or nothing.
    pub target: ClientWaypointTarget,
    /// The waypoint's icon style (`Waypoint.Icon.style`, a `waypoint_style` asset id such as
    /// `minecraft:default`).
    pub style: Identifier,
    /// The waypoint's icon colour as `0xRRGGBB`, as `living_set_waypoint_color` takes it;
    /// `None` when the server sent none (the bar then colours the dot from the waypoint's id).
    pub color: Option<u32>,
}

impl ClientWaypointEntry {
    /// A waypoint from its parts.
    pub fn new(
        id: ClientWaypointId,
        target: ClientWaypointTarget,
        style: Identifier,
        color: Option<u32>,
    ) -> Self {
        Self {
            id,
            target,
            style,
            color,
        }
    }
}

/// A tracked waypoint's identifier (`Either<UUID, String>`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ClientWaypointId {
    /// The tracked entity's UUID (a player's waypoint).
    Uuid(u128),
    /// The waypoint's name (a named waypoint with no entity).
    Name(String),
}

/// What a tracked waypoint points at (`TrackedWaypoint`'s `Type` and contents).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ClientWaypointTarget {
    /// An untracked waypoint (`TrackedWaypoint.empty`).
    Empty,
    /// A block position (`Vec3iWaypoint`).
    Block(BlockPos),
    /// A chunk (`ChunkWaypoint`).
    Chunk([i32; 2]),
    /// A bearing in radians (`AzimuthWaypoint`).
    Azimuth(f32),
}

/// The listed players' skins as the Tab overlay draws them, read with
/// `ClientSession::client_player_skins`: what this client has been sent, changing as the server
/// updates the list and skins download.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientPlayerSkins {
    /// The listed players' skins, in no order; join on `uuid`.
    pub entries: Vec<ClientPlayerSkin>,
}

impl ClientPlayerSkins {
    /// Skins from their entries.
    pub fn new(entries: Vec<ClientPlayerSkin>) -> Self {
        Self { entries }
    }
}

/// One listed player's skin (`PlayerInfo`) as the Tab overlay draws it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientPlayerSkin {
    /// The profile UUID.
    pub uuid: u128,
    /// The skin texture path as the Tab overlay draws it: a downloaded skin (`skins/<hash>`) or
    /// the profile's default (`entity/player/{slim,wide}/<name>`). Draw the face with a blit of
    /// uv [8, 8], src [8, 8] over tex [64, 64], then the hat at uv [40, 8].
    pub skin: String,
    /// Whether the skin uses the slim (Alex) model.
    pub slim: bool,
}

impl ClientPlayerSkin {
    /// A player's skin from its parts.
    pub fn new(uuid: u128, skin: String, slim: bool) -> Self {
        Self { uuid, skin, slim }
    }
}

/// The local player's fishing hook, read with `ClientSession::client_fishing_hook`:
/// its position and whether a fish is biting.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientFishingHook {
    /// The bobber's position, as the client tracks it.
    pub position: Vec3,
    /// A fish is biting (`FishingHook.biting`): the server's DATA_BITING.
    pub biting: bool,
}

impl ClientFishingHook {
    /// A fishing hook from its parts.
    pub fn new(position: Vec3, biting: bool) -> Self {
        Self { position, biting }
    }
}
