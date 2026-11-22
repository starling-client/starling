//! An entity riding a player (`Entity.startRiding` with a player as the
//! vehicle): why [`EntityRef::ride_player`](crate::EntityRef::ride_player)
//! seated nothing.

/// Why [`EntityRef::ride_player`](crate::EntityRef::ride_player) left the
/// rider where it was. Every refusal changes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum RidePlayerRefusal {
    /// A handle is stale or names no entity of the server level in place, or
    /// the player is in another level. Also the answer with no game backend.
    NoEntity,
    /// The vehicle handle names an entity that is not a player.
    NotAPlayer,
    /// The rider is a player. `/ride` refuses this too
    /// (`commands.ride.mount.failure.cant_ride_players`), and the game has
    /// no seat for a player on a player.
    PlayerRider,
    /// The rider is not a mob (an item, a boat, a projectile...), which the
    /// game cannot seat on a player.
    UnsupportedRider,
    /// The rider and the vehicle are the same entity.
    SameEntity,
    /// The rider already rides this player.
    AlreadyRiding,
    /// The player rides the rider, directly or through other vehicles
    /// (`Entity.startRiding`'s `hasPassenger`/`isPassengerOfSameVehicle`
    /// walk).
    Cycle,
    /// Unforced, the player already carries a passenger: `Player` keeps
    /// `Entity.canAddPassenger`, one passenger. `force` seats it anyway.
    Full,
    /// Unforced, the rider may not ride now (`Entity.canRide`): its boarding
    /// cooldown is running, or it is a warden or a wither.
    CannotRide,
    /// A mod's `entity_mount` hook canceled the mount, or the dismount from
    /// the rider's old vehicle, or the hook changed the rider or the player
    /// so that it no longer applies.
    Canceled,
}

impl std::fmt::Display for RidePlayerRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoEntity => "a handle names no entity of this server level",
            Self::NotAPlayer => "the vehicle is not a player",
            Self::PlayerRider => "a player cannot ride a player",
            Self::UnsupportedRider => "only a mob can ride a player",
            Self::SameEntity => "an entity cannot ride itself",
            Self::AlreadyRiding => "the rider already rides this player",
            Self::Cycle => "the player rides the rider",
            Self::Full => "the player already carries a passenger",
            Self::CannotRide => "the rider may not ride now",
            Self::Canceled => "a mod canceled the mount",
        })
    }
}

impl std::error::Error for RidePlayerRefusal {}

impl crate::NoBackend for RidePlayerRefusal {
    fn no_backend() -> Self {
        Self::NoEntity
    }
}
