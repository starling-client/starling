//! Read-only client session and local-player status.

use crate::{BlockPos, Identifier};

/// The connected server, not the reconnect target of an earlier session.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientServer {
    /// The integrated world's display name. External connections do not retain
    /// a server display name; this is `None`, never the connecting player's name.
    pub name: Option<String>,
    /// The joined address, with friend-invite secrets redacted. Memory
    /// connections have no address (`None`), not a made-up socket address.
    pub address: Option<String>,
    /// This window owns the connected integrated server. A loopback address
    /// alone does not make this true; a splitscreen guest does not own it.
    pub integrated_local: bool,
    /// Folder name relative to the instance's saves directory for the owned
    /// integrated world. Renaming the world in-game leaves this key unchanged.
    /// External connections and splitscreen guests return `None`.
    pub save_folder: Option<String>,
}

/// The local player's item use as the client sees it (`LivingEntity.isUsingItem`,
/// `getUsedItemHand`, `getTicksUsingItem`, `getUseItemRemainingTicks`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
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
#[derive(horizon_mod_api_macros::Wire)]
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

/// The player list as the Tab overlay draws it (`PlayerTabOverlay`), read with
/// `ClientSession::client_tab_list`: what this client has been sent, changing as the server
/// updates the list.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientTabList {
    /// The listed players in the order drawn (`getPlayerInfos`: higher list order first,
    /// spectators last, then team name and profile name), at most the 80 the overlay draws.
    pub entries: Vec<ClientTabListEntry>,
    /// The header above the names, when the server sent one.
    pub header: Option<crate::Text>,
    /// The footer below them, when the server sent one.
    pub footer: Option<crate::Text>,
}

impl ClientTabList {
    /// A list of `entries` with `header` and `footer`.
    pub fn new(
        entries: Vec<ClientTabListEntry>,
        header: Option<crate::Text>,
        footer: Option<crate::Text>,
    ) -> Self {
        Self {
            entries,
            header,
            footer,
        }
    }
}

/// One listed player (`PlayerInfo`) of a [`ClientTabList`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientTabListEntry {
    /// The profile name (`GameProfile.name`).
    pub name: String,
    /// The profile UUID.
    pub uuid: u128,
    /// The game mode the server lists them in.
    pub game_mode: crate::GameMode,
    /// The latency in milliseconds the ping bars show; negative while unknown.
    pub latency: i32,
    /// The name as the list draws it (`getNameForDisplay`): the server's list name, else the
    /// profile name in the team's prefix, suffix and colour, italic for a spectator.
    pub display: crate::Text,
}

impl ClientTabListEntry {
    /// An entry of `name` and `uuid` in `game_mode`, `latency` milliseconds away, drawn as
    /// `display`.
    pub fn new(
        name: String,
        uuid: u128,
        game_mode: crate::GameMode,
        latency: i32,
        display: crate::Text,
    ) -> Self {
        Self {
            name,
            uuid,
            game_mode,
            latency,
            display,
        }
    }
}

/// The locator bar's tracked waypoints (`ClientWaypointManager`), read with
/// `ClientSession::client_waypoints`: what this client has been sent, changing as the server
/// tracks and untracks waypoints. With a camera entity the read follows
/// `ClientWaypointManager.forEachWaypoint`: farthest first by the vanilla squared distance,
/// ties in no promised order; without one, in no promised order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
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
#[derive(horizon_mod_api_macros::Wire)]
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
#[derive(horizon_mod_api_macros::Wire)]
pub enum ClientWaypointId {
    /// The tracked entity's UUID (a player's waypoint).
    Uuid(u128),
    /// The waypoint's name (a named waypoint with no entity).
    Name(String),
}

/// What a tracked waypoint points at (`TrackedWaypoint`'s `Type` and contents).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
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
#[derive(horizon_mod_api_macros::Wire)]
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
#[derive(horizon_mod_api_macros::Wire)]
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

/// The local player's current block-breaking operation. An unavailable player
/// returns `None` from the handle, distinct from [`Self::Idle`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
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
