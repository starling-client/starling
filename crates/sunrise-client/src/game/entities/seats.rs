//! Vehicles with nothing of their own to steer or seat with: armor stands, display entities,
//! interactions and the other entities Sunrise keeps as their compound (a marker, which takes no rider at
//! all, is not one of them). `RideCommand.mount` forces the ride
//! (`startRiding(vehicle, true, true)`), so any of them takes a rider; the rider then sits at the vehicle's
//! default passenger attachment and gets off on sneak, as vanilla's `Entity` does for a vehicle that
//! overrides neither.
use super::{Game, Keys};
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;

/// `Avatar.DEFAULT_VEHICLE_ATTACHMENT`: a player sits this far below the vehicle's passenger point.
const PLAYER_VEHICLE_ATTACHMENT: f64 = 0.6;

/// Such a vehicle: where it is, and how tall and wide its box is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Seat {
    pub position: Vec3,
    pub height: f64,
    pub width: f32,
    attachment: [f64; 3],
    yaw: f32,
}

impl Seat {
    pub(super) fn general(type_slot: usize, position: Vec3, yaw: f32) -> Self {
        let catalog = super::general::general_type_catalog();
        let (width, height) = catalog.dimensions(type_slot);
        Self {
            position,
            height,
            width: width as f32,
            attachment: catalog.passenger_attachment(type_slot),
            yaw,
        }
    }

    /// `Entity.positionRider`: the passenger point less the rider's vehicle attachment.
    pub fn passenger_position(&self, rider_offset: f64) -> Vec3 {
        sunrise_client::mobs::riders::seat(self.attachment, rider_offset, self.position, self.yaw)
    }

    /// `Entity.positionRider` for a player: its vehicle attachment below the passenger point.
    pub fn player_seat(&self) -> Vec3 {
        self.passenger_position(PLAYER_VEHICLE_ATTACHMENT)
    }

    pub(super) fn position_mob(&self, mob: &mut sunrise_client::mobs::Mob) {
        let offset = sunrise_client::mobs::riders::vehicle_attachment(mob, self.width);
        sunrise_client::mobs::living::teleports::snap_to(mob, self.passenger_position(offset));
        mob.velocity = Vec3::ZERO;
        mob.fall_distance = 0.0;
        mob.ground.on_ground = false;
        mob.extra.passenger = true;
        mob.ai.nav.stop();
    }
    /// `Entity.getDismountLocationForPassenger`: `(getX(), getBoundingBox().maxY, getZ())`.
    pub fn dismount(&self) -> Vec3 {
        Vec3::new(
            self.position.x,
            self.position.y + self.height,
            self.position.z,
        )
    }
}

/// The box height of an entity Sunrise keeps as its compound, from its type's `EntityDimensions` (the
/// passenger attachment sits on top of it): displays and markers are `sized(0, 0)`, an interaction is as
/// tall as its `height` (1 by default), an area effect cloud half a block.
fn other_height(kind: &str, tag: Option<&Tag>) -> (f64, f32) {
    match kind {
        "minecraft:interaction" => {
            let field = |key: &str| match tag.and_then(|t| t.get(key)) {
                Some(Tag::Float(v)) => Some(f64::from(f32::from_bits(*v))),
                _ => None,
            };
            (
                field("height").unwrap_or(1.0),
                field("width").unwrap_or(1.0) as f32,
            )
        }
        "minecraft:area_effect_cloud" => (0.5, 6.0),
        _ => (0.0, 0.0),
    }
}

/// `Entity.couldAcceptPassenger`: false for exactly `Marker` (`Marker.java:47`) and
/// `OminousItemSpawner` (`OminousItemSpawner.java:134`), and not even a forced `startRiding` or `/ride`
/// bypasses it. A marker armor stand is an ordinary armor stand and accepts.
pub(in crate::game) fn could_accept_passenger(kind: &str) -> bool {
    !matches!(
        kind,
        "minecraft:marker" | "marker" | "minecraft:ominous_item_spawner"
    )
}

/// `EntityType.canSerialize`, which `Entity.startRiding` requires of a vehicle on the server and which a
/// save requires of a root: false for the types built with `noSave()`, of which `Game::others` can hold
/// the leash knot (`EntityTypes.java:614-618`).
pub(in crate::game) fn saves_with_chunk(kind: &str) -> bool {
    !matches!(kind, "minecraft:leash_knot")
}

impl Game<'_> {
    /// A non-steerable general row, armor stand or kept entity as a vehicle.
    pub(super) fn seat(&self, id: u32) -> Option<Seat> {
        if let Some(seat) = self.general_seat(id, false) {
            return Some(seat);
        }
        if let Some(a) = self.armor_stands.iter().find(|a| a.id == id) {
            // ArmorStand: sized(0.5F, 1.975F), half that when small, nothing as a
            // marker; the passenger point is the float height widened to double.
            let (height, width) = if a.marker {
                (0.0, 0.0)
            } else if a.small {
                (f64::from(0.9875_f32), 0.25)
            } else {
                (f64::from(1.975_f32), 0.5)
            };
            return Some(Seat {
                position: a.position,
                height,
                width,
                attachment: [0.0, height, 0.0],
                yaw: 0.0,
            });
        }
        let o = self.others.iter().find(|o| o.id == id)?;
        if !could_accept_passenger(&o.kind) {
            return None;
        }
        let (height, width) = other_height(&o.kind, self.persist_raw(id));
        Some(Seat {
            position: o.position,
            height,
            width,
            attachment: [0.0, height, 0.0],
            yaw: 0.0,
        })
    }

    pub(super) fn general_seat(&self, id: u32, previous: bool) -> Option<Seat> {
        if let Some(remote) = &self.remote {
            let row = remote.general.rows.get(&(id as i32))?;
            let ([x, y, z], [yaw, _]) = if previous {
                (row.previous, row.previous_rotation)
            } else {
                (row.position, row.rotation)
            };
            return Some(Seat::general(row.type_slot, Vec3::new(x, y, z), yaw));
        }
        let row = self.general_row(id)?;
        Some(Seat::general(row.type_slot, row.position, row.rotation[0]))
    }

    /// Seats the player in place on such a vehicle (`ServerPlayer.startRiding`: at once, as a teleport).
    pub(super) fn board_seat(&mut self, id: u32, seat: Seat) {
        let at = seat.player_seat();
        if let Some(player) = self.player.as_mut() {
            player.position = at;
            player.velocity = Vec3::ZERO;
            player.fall_distance = 0.0;
            player.teleported = true;
            player.teleported_delta = [true; 3];
        }
        self.previous_position = at;
        self.start_riding(id);
    }

    /// `LivingEntity.stopRiding` off such a vehicle: `dismountVehicle` on top of it. A vehicle that is
    /// gone lets the rider go where it is.
    pub(super) fn get_off_seat(&mut self) {
        let Some(id) = self.riding else {
            return;
        };
        if !self.native_may_leave(id) {
            return;
        }
        self.riding = None;
        let Some(seat) = self.seat(id) else {
            return;
        };
        let at = seat.dismount();
        if let Some(p) = self.player.as_mut() {
            self.previous_position = p.position;
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
    }

    /// `Player.rideTick` on such a vehicle: keep the seat (the vehicle may have been moved), get off on
    /// sneak. The seat, or None once off.
    pub(super) fn tick_seat_riding(&mut self, keys: Keys) -> Option<Vec3> {
        let vehicle = self.riding?;
        let seat = self.seat(vehicle)?;
        if keys.shift {
            self.get_off_seat();
            return None;
        }
        let at = seat.player_seat();
        if let Some(p) = self.player.as_mut() {
            self.previous_position = p.position;
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        Some(at)
    }

    /// `Entity.positionRider` for the mobs riding such a vehicle: its passenger attachment less the
    /// rider's own vehicle attachment. A server sends no position for a passenger, so every client seats
    /// it here.
    pub(super) fn seat_static_mobs(&mut self, riding: &super::passengers::Riders) {
        for (&vehicle, js) in riding {
            let Some(seat) = self.seat(vehicle) else {
                continue;
            };
            for &j in js {
                seat.position_mob(&mut self.mobs[j]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_sits_on_the_passenger_point_and_gets_off_on_top() {
        // A display: sized(0, 0), its passenger point at its position.
        let display = Seat {
            position: Vec3::new(1.5, 64.0, -2.5),
            height: 0.0,
            width: 0.0,
            attachment: [0.0; 3],
            yaw: 0.0,
        };
        assert_eq!(display.player_seat(), Vec3::new(1.5, 63.4, -2.5));
        assert_eq!(display.dismount(), Vec3::new(1.5, 64.0, -2.5));
        // An armor stand: its passenger point on top of its 1.975 box.
        let stand = Seat {
            position: Vec3::new(0.5, 64.0, 0.5),
            height: 1.975,
            width: 0.5,
            attachment: [0.0, 1.975, 0.0],
            yaw: 0.0,
        };
        assert!((stand.player_seat().y - 65.375).abs() < 1e-9);
        assert!((stand.dismount().y - 65.975).abs() < 1e-9);
    }

    /// `Entity.positionRider`: the vehicle's position plus its passenger attachment, less the
    /// rider's own vehicle attachment, with the attachment turned by the vehicle's yaw. Expected
    /// values are written out by hand, not derived from the code.
    #[test]
    fn a_rider_sits_on_the_passenger_attachment_less_its_own() {
        let seat = Seat {
            position: Vec3::new(10.0, 64.0, -3.0),
            height: 2.0,
            width: 0.5,
            attachment: [1.0, 2.0, 0.5],
            yaw: 0.0,
        };
        // Facing yaw 0 the attachment is not turned: x 10 + 1, y 64 + 2 - 0.5, z -3 + 0.5.
        assert_eq!(seat.passenger_position(0.5), Vec3::new(11.0, 65.5, -2.5));
        // A rider with no vehicle attachment of its own sits on the point itself.
        assert_eq!(seat.passenger_position(0.0), Vec3::new(11.0, 66.0, -2.5));
        // The default attachment is the top centre of the box, whatever the position.
        let top = Seat {
            attachment: [0.0, 2.0, 0.0],
            ..seat
        };
        assert_eq!(top.passenger_position(0.0), Vec3::new(10.0, 66.0, -3.0));
    }

    #[test]
    fn kept_entities_are_as_tall_as_their_type() {
        assert_eq!(other_height("minecraft:item_display", None), (0.0, 0.0));
        assert_eq!(other_height("minecraft:marker", None), (0.0, 0.0));
        assert_eq!(other_height("minecraft:interaction", None), (1.0, 1.0));
        assert_eq!(
            other_height("minecraft:area_effect_cloud", None),
            (0.5, 6.0)
        );
    }
}
