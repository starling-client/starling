//! `player_respawn_position` (NeoForge's `PlayerRespawnPositionEvent`): where
//! a respawning server player ends up. As in `PlayerList.respawn`, the destination
//! and any redirect are resolved before the replacement is created.
use super::respawn_anchor::RespawnConfig;
use super::{Dim, Game};
use sunrise_core::dimension::Dimension as LevelDimension;
use sunrise_core::geometry::Vec3;

/// Where a respawn ends, as the server found it: a level, a point and the
/// player's facing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Respawn {
    pub dim: Dim,
    pub at: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}

/// What the mods decided about a respawn.
#[derive(Clone, Copy, Debug)]
pub(super) struct RespawnDecision {
    /// Where the player goes: the found point, or the mods' own.
    pub to: Respawn,
    /// Whether the fresh player keeps its respawn point
    /// (`copyOriginalSpawnPosition`).
    pub copy: bool,
}

impl Dim {
    /// The level the world has for `level`; None for a `Custom` level it lacks.
    pub(super) fn from_level(level: LevelDimension) -> Option<Self> {
        Some(match level {
            LevelDimension::Overworld => Self::Overworld,
            LevelDimension::Nether => Self::Nether,
            LevelDimension::End => Self::End,
            LevelDimension::Custom(key) => {
                Self::exists(key.id()).then(|| Self::from_id(key.id()))?
            }
        })
    }
}

impl Game<'_> {
    /// `PlayerRespawnPositionEvent`, once per respawn: guest `conn`
    /// respawns at `found`, final now. `missing` is the respawn point whose
    /// bed or anchor was gone (`TeleportTransition.missingRespawnBlock`).
    /// Every place that finishes a respawn asks here; None when no mod
    /// listens. A moved destination is not block-checked and doesn't fire
    /// again.
    pub(super) fn respawn_position(
        &mut self,
        conn: u32,
        found: Respawn,
        missing: Option<RespawnConfig>,
        from_end_fight: bool,
    ) -> Option<RespawnDecision> {
        if !mod_api::__private::listened(mod_api::HookId::PlayerRespawnPosition) {
            return None;
        }
        let uuid = self.command_guest(conn)?.uuid;
        let original = [found.at.x, found.at.y, found.at.z];
        let (mut level, mut position) = (found.dim.level(), original);
        let (mut yaw, mut pitch) = (found.yaw, found.pitch);
        let mut copy = missing.is_none();
        self.with_cloning_player(conn, |game| {
            let player = game.player(uuid)?;
            game.native_player_respawn_position(mod_api::__private::PlayerRespawnPositionCall {
                player,
                original_level: found.dim.level(),
                original_position: original,
                missing_respawn_block: missing.is_some(),
                from_end_fight,
                level: &mut level,
                position: &mut position,
                yaw: &mut yaw,
                pitch: &mut pitch,
                copy_original_spawn_position: &mut copy,
            });
            Some(())
        })
        .flatten()?;
        // A level the world lacks leaves the player at the whole found point: the mod's
        // coordinates are for that level.
        let to = match Dim::from_level(level) {
            Some(dim) => Respawn {
                dim,
                at: Vec3::new(position[0], position[1], position[2]),
                yaw,
                pitch,
            },
            None => {
                log::warn!("Sunrise: a mod chose a respawn in a level the world does not have");
                found
            }
        };
        Some(RespawnDecision { to, copy })
    }
}
