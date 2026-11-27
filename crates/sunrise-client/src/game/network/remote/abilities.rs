//! Client-side `LocalPlayer.abilities` and `PlayerInfo` game mode sync (`ClientPacketListener`).
use super::*;
use sunrise_client::lan::abilities::PlayerAbilities;

/// `Abilities.walkingSpeed` before the server sends one.
const WALKING_SPEED: f32 = 0.1;

/// The walking speed `AbstractClientPlayer.getFieldOfViewModifier` divides the movement speed by.
fn walking_speed(abilities: Option<&PlayerAbilities>) -> f32 {
    abilities.map_or(WALKING_SPEED, |a| a.walking_speed)
}

impl Remote {
    /// `AbstractClientPlayer.gameMode()`: resolves `PlayerInfo` once per entity lifetime.
    pub(in crate::game) fn own_player_info_game_mode(&self) -> Option<i32> {
        if let Some(cell) = self.own_player_info.get() {
            return Some(cell.load(std::sync::atomic::Ordering::Relaxed));
        }
        if let Some(cell) = self.tab_game_modes.get(&self.profile) {
            let _ = self.own_player_info.set(Arc::clone(cell));
            return Some(cell.load(std::sync::atomic::Ordering::Relaxed));
        }
        self.tab_players.get(&self.profile).map(|t| t.game_mode)
    }

    /// `AbstractClientPlayer.gameMode()` for a remote player entity in `level_players`.
    pub(in crate::game) fn remote_player_info_game_mode(&self, entity_id: i32) -> Option<i32> {
        if let Some(slot) = self.level_player_infos.get(&entity_id)
            && let Some(cell) = slot.get()
        {
            return Some(cell.load(std::sync::atomic::Ordering::Relaxed));
        }
        let uuid = self.level_players.get(&entity_id)?;
        if let Some(cell) = self.tab_game_modes.get(uuid) {
            if let Some(slot) = self.level_player_infos.get(&entity_id) {
                let _ = slot.set(Arc::clone(cell));
            }
            return Some(cell.load(std::sync::atomic::Ordering::Relaxed));
        }
        self.tab_players.get(uuid).map(|t| t.game_mode)
    }

    /// Apply `GameType.updatePlayerAbilities` onto `LocalPlayer.abilities` and mirror `game_mode`.
    pub(in crate::game) fn update_local_mode_abilities(&mut self, mode: i32) -> bool {
        self.game_mode = mode;
        self.abilities.update_for_mode(mode);
        self.abilities.instabuild
    }

    /// Copy predicted body flight and scroll speed into `Remote::abilities` before mode updates.
    pub(in crate::game) fn sync_body_flight(&mut self, flying: bool, flying_speed: f32) {
        self.abilities.flying = flying;
        self.abilities.flying_speed = flying_speed;
    }
}

impl Game<'_> {
    /// Mirror `Remote::abilities` onto the live `PlayerMotion` body.
    pub(in crate::game) fn apply_remote_abilities(&mut self) {
        let (Some(player), Some(a)) = (
            self.player.as_mut(),
            self.remote.as_ref().map(|r| &r.abilities),
        ) else {
            return;
        };
        player.invulnerable = a.invulnerable;
        player.flying = a.flying;
        player.may_fly = a.may_fly;
        player.flying_speed = a.flying_speed;
    }

    /// Sync local and remote player spectator modes from `PlayerInfo` and `MultiPlayerGameMode`.
    pub(in crate::game) fn sync_remote_player_game_mode(&mut self) {
        debug_assert!(self.remote.is_some());
        if self.player.is_some() {
            let info_spectator = self.own_player_is_spectator();
            let local_spectator = self.spectator();
            if let Some(player) = self.player.as_mut() {
                player.spectator = info_spectator;
                player.local_spectator = local_spectator;
            }
        }
        if let Some(r) = self.remote.as_ref() {
            for i in 0..self.mobs.len() {
                let id = self.mobs[i].id as i32;
                if r.level_players.contains_key(&id) {
                    let spectator = r.remote_player_info_game_mode(id) == Some(SPECTATOR);
                    self.mobs.update(i, |m| m.extra.spectator = spectator);
                }
            }
        }
    }

    /// Mirror spectator scroll-wheel `flying_speed` back into `Remote::abilities`.
    pub(in crate::game) fn set_remote_flying_speed(&mut self, speed: f32) {
        if let Some(r) = self.remote.as_mut() {
            r.abilities.flying_speed = speed;
        }
    }

    /// `Abilities.getWalkingSpeed()` of the local player: the server's, on a server.
    pub(in crate::game) fn remote_walking_speed(&self) -> f32 {
        walking_speed(self.remote.as_ref().map(|r| &r.abilities))
    }

    /// ClientPacketListener.handleRotatePlayer: the server's yaw and pitch, each added to the
    /// current one when its bit says relative, then sent back as a `Rot` move with both
    /// flags false.
    pub(super) fn remote_rotation(&mut self, p: &play::PlayerRotation) {
        self.yaw = if p.relative_yaw {
            self.yaw + p.yaw
        } else {
            p.yaw
        };
        self.pitch = (if p.relative_pitch {
            self.pitch + p.pitch
        } else {
            p.pitch
        })
        .clamp(-90.0, 90.0);
        self.snap_player_model_pitch();
        self.send(S::MovePlayerRot(play::MovePlayerRot {
            yaw: self.yaw,
            pitch: self.pitch,
            flags: 0,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_client::camera::{FovPlayer, field_of_view_modifier};

    fn fov(walking: f32, movement: f64) -> f32 {
        let player = FovPlayer {
            flying: false,
            walking_speed: walking,
            movement_speed: movement,
            using: None,
        };
        field_of_view_modifier(&player, true, 1.0)
    }

    #[test]
    fn the_field_of_view_divides_by_the_servers_walking_speed() {
        // A server that sets the walking speed to 0.2 and the movement speed to 0.2: vanilla's
        // getFieldOfViewModifier is (0.2 / 0.2 + 1) / 2 = 1. Kept at 0.1 it would be
        // (0.2 / 0.1 + 1) / 2 = 1.5, a zoomed-out view.
        let abilities = PlayerAbilities {
            walking_speed: 0.2,
            ..PlayerAbilities::default()
        };
        assert_eq!(fov(walking_speed(Some(&abilities)), 0.2), 1.0);
        // Before any packet: vanilla's default 0.1, a still player at the base speed 0.1.
        assert_eq!(walking_speed(None), 0.1);
        assert_eq!(fov(walking_speed(None), 0.1), 1.0);
    }
}

#[cfg(test)]
#[path = "gamemode_split_tests.rs"]
pub(in crate::game) mod gamemode_split_tests;
