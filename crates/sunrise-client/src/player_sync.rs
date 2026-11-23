//! LocalPlayer.sendPosition and PositionMoveRotation.calculateAbsolute.
use sunrise_core::geometry::Vec3;
use sunrise_protocol::packets::play::{self, PlayerPosition, Serverbound as S, relative};

/// Only movement updates advance this baseline. A teleport acknowledgement is
/// separate and must not suppress the next tick's corrected position or look.
#[derive(Default)]
pub struct PositionSender {
    position: [f64; 3],
    rotation: (f32, f32),
    on_ground: bool,
    collision: bool,
    reminder: u32,
    /// A look the server was told outside the updates, when it may not be the baseline's.
    told: Option<(f32, f32)>,
}

impl PositionSender {
    /// The position the server was last told (the baseline of the next move).
    pub fn last_sent(&self) -> [f64; 3] {
        self.position
    }
    /// The rotation the server was last told by an update.
    pub fn last_rotation(&self) -> (f32, f32) {
        self.rotation
    }
    /// The server took a look from another packet (a teleport's acknowledgement, the view's look
    /// before a hit): the next update sends its own look if it is not that one, even when the
    /// baseline has it. With Free Direction steering the look sent is not the view's, and an
    /// acknowledgement with the view's look would otherwise stand for every move after it.
    pub fn told_rotation(&mut self, rotation: (f32, f32)) {
        self.told = Some(rotation);
    }
    #[sunrise_mod_api_macros::replaceable]
    pub fn update(
        &mut self,
        position: [f64; 3],
        rotation: (f32, f32),
        on_ground: bool,
        collision: bool,
    ) -> Option<S> {
        self.reminder += 1;
        let d: [f64; 3] = std::array::from_fn(|i| position[i] - self.position[i]);
        let moved =
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 2.0e-4 * 2.0e-4 || self.reminder >= 20;
        let rotated = rotation.0 - self.rotation.0 != 0.0
            || rotation.1 - self.rotation.1 != 0.0
            || self.told.take().is_some_and(|told| told != rotation);
        let flags = u8::from(on_ground) | u8::from(collision) << 1;
        let [x, y, z] = position;
        let (yaw, pitch) = rotation;
        let packet = if moved && rotated {
            Some(S::MovePlayerPosRot(play::MovePlayerPosRot {
                x,
                y,
                z,
                yaw,
                pitch,
                flags,
            }))
        } else if moved {
            Some(S::MovePlayerPos(play::MovePlayerPos { x, y, z, flags }))
        } else if rotated {
            Some(S::MovePlayerRot(play::MovePlayerRot { yaw, pitch, flags }))
        } else if self.on_ground != on_ground || self.collision != collision {
            Some(S::MovePlayerStatus(play::MovePlayerStatus { flags }))
        } else {
            None
        };
        if moved {
            self.position = position;
            self.reminder = 0;
        }
        if rotated {
            self.rotation = rotation;
        }
        self.on_ground = on_ground;
        self.collision = collision;
        packet
    }
}

/// Rotate the old velocity before adding the relative delta components.
/// Angles deliberately use Java's double toRadians followed by a float cast.
pub fn teleport_velocity(
    mut velocity: Vec3,
    old_rotation: (f32, f32),
    new_rotation: (f32, f32),
    packet: &PlayerPosition,
) -> Vec3 {
    if packet.relatives & relative::ROTATE_DELTA != 0 {
        let pitch = f64::from(old_rotation.1 - new_rotation.1).to_radians() as f32;
        let yaw = f64::from(old_rotation.0 - new_rotation.0).to_radians() as f32;
        velocity = velocity.rotate_x(pitch).rotate_y(yaw);
    }
    let old = [velocity.x, velocity.y, velocity.z];
    let flags = [relative::DELTA_X, relative::DELTA_Y, relative::DELTA_Z];
    let result: [f64; 3] = std::array::from_fn(|i| {
        if packet.relatives & flags[i] != 0 {
            old[i] + packet.delta[i]
        } else {
            packet.delta[i]
        }
    });
    Vec3::new(result[0], result[1], result[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_told_elsewhere_is_sent_again_when_the_move_has_another() {
        let mut sender = PositionSender::default();
        sender.update([0.0; 3], (30.0, 0.0), true, false);
        // A teleport acknowledged with the view's look (0) while the moves steer at 30.
        sender.told_rotation((0.0, 0.0));
        assert!(matches!(
            sender.update([0.2, 0.0, 0.0], (30.0, 0.0), true, false),
            Some(S::MovePlayerPosRot(play::MovePlayerPosRot {
                yaw: 30.0,
                ..
            }))
        ));
        assert!(matches!(
            sender.update([0.4, 0.0, 0.0], (30.0, 0.0), true, false),
            Some(S::MovePlayerPos(_))
        ));
        // Told the look the move has: nothing more to send than vanilla would.
        sender.told_rotation((30.0, 0.0));
        assert!(matches!(
            sender.update([0.6, 0.0, 0.0], (30.0, 0.0), true, false),
            Some(S::MovePlayerPos(_))
        ));
    }
}
