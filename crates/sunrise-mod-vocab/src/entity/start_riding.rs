//! Why a mount through [`EntityRef::start_riding`](crate::EntityRef::start_riding)
//! seated nothing.

/// Why [`EntityRef::start_riding`](crate::EntityRef::start_riding) did not complete
/// the requested mount; each cause has its own name. A refusal at the checks before
/// `entity_mount` changes nothing, but one named after it does not undo what a hook
/// moved or removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StartRidingRefusal {
    /// A handle is from another world epoch, is held out, or names no entity
    /// of the server level in place: the entity was removed, is dying, or is
    /// in another level (a player in another level too). Also the answer with
    /// no game backend.
    NoEntity,
    /// The vehicle is a player, forced or not. Vanilla's `Entity.startRiding`
    /// refuses a vehicle whose entity type is not saved with the level before
    /// it reads `force`, and the player type is one: a player's save keeps no
    /// passengers, so nothing may ride a player.
    PlayerVehicle,
    /// The rider is not a player, mob or declared general entity.
    UnsupportedRider,
    /// The entity cannot carry passengers: for example, a marker, an
    /// unsupported family, or a general type declared `NoSave`.
    UnsupportedVehicle,
    /// The rider and the vehicle are the same entity.
    SameEntity,
    /// The rider already rides this vehicle.
    AlreadyRiding,
    /// The vehicle rides the rider, directly or through other vehicles
    /// (`Entity.startRiding`'s `hasPassenger`/`isPassengerOfSameVehicle` walk).
    Cycle,
    /// Unforced, the rider may not ride now (`Entity.canRide`): a sneaking
    /// player, a mob or general entity in its boarding cooldown, or a warden or wither.
    /// `force` mounts it anyway.
    CannotRide,
    /// The vehicle has no room (`Entity.canAddPassenger`): a mob at its
    /// species' count, a seat already taken, or a boat or minecart at its
    /// storage limit, which even `force` keeps.
    Full,
    /// A mod's `entity_mount` hook canceled the mount, or the dismount from
    /// the rider's old vehicle, or changed the rider so that it no longer
    /// applies. The hook fires only after every other check passed, so a
    /// refusal at those checks reaches no hook.
    Canceled,
}

impl std::fmt::Display for StartRidingRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoEntity => "a handle names no entity of this server level",
            Self::PlayerVehicle => "nothing may ride a player",
            Self::UnsupportedRider => "only a player, mob or general entity can ride",
            Self::UnsupportedVehicle => "the vehicle takes no passenger",
            Self::SameEntity => "an entity cannot ride itself",
            Self::AlreadyRiding => "the rider already rides this vehicle",
            Self::Cycle => "the vehicle rides the rider",
            Self::CannotRide => "the rider may not ride now",
            Self::Full => "the vehicle has no room",
            Self::Canceled => "a mod canceled the mount",
        })
    }
}

impl std::error::Error for StartRidingRefusal {}

impl crate::NoBackend for StartRidingRefusal {
    fn no_backend() -> Self {
        Self::NoEntity
    }
}
