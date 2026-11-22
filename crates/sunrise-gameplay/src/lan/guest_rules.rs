//! A guest's spectator camera (`ServerPlayer.camera`).
use sunrise_core::geometry::Aabb;

/// Where a camera entity is, which way it looks and its box.
pub struct CameraExtra {
    pub position: [f64; 3],
    pub rotation: [f32; 2],
    pub bounds: Aabb,
}

/// `ServerPlayer.camera` of one guest other than itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuestCamera {
    /// The camera entity's network id.
    pub id: i32,
    /// The guest's teleport counter once the view moved to the camera: a
    /// later teleport is `teleportTo(.., resetCamera = true)`.
    pub teleports: i32,
}

/// Whether `ServerPlayer.tick` hands the view back to the player: the
/// camera is gone (`!isAlive`), the player wants to stop riding (sneaks),
/// left spectator mode (`setGameMode`) or was teleported (`resetCamera`).
pub fn camera_ends(alive: bool, sneaking: bool, spectator: bool, teleported: bool) -> bool {
    !alive || sneaking || !spectator || teleported
}

#[cfg(test)]
mod tests {
    use super::camera_ends;

    /// A living camera is followed until the player sneaks, leaves
    /// spectator mode or is teleported; a dead one ends it at once.
    #[test]
    fn camera_ends_like_server_player_tick() {
        assert!(!camera_ends(true, false, true, false));
        assert!(camera_ends(false, false, true, false));
        assert!(camera_ends(true, true, true, false));
        assert!(camera_ends(true, false, false, false));
        assert!(camera_ends(true, false, true, true));
    }
}
