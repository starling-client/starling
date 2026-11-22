//! What asking a portal where it leads answers, and why it refuses.
use crate::{LevelRef, PortalShape};

/// Where an entity entering a portal arrives, from `LevelRef::portal_destination` and
/// `LevelRef::find_or_create_portal_exit`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct PortalExit {
    /// The level it arrives in.
    pub level: LevelRef,
    /// The bottom centre of its box there.
    pub pos: [f64; 3],
    /// What happens to its yaw and pitch.
    pub rotation: PortalRotation,
    /// The nether portal it arrives in (its bottom-left inside block, width, height and axis);
    /// None for an End portal and an End gateway.
    pub exit_portal: Option<PortalShape>,
}

impl PortalExit {
    /// An answer.
    pub fn new(
        level: LevelRef,
        pos: [f64; 3],
        rotation: PortalRotation,
        exit_portal: Option<PortalShape>,
    ) -> Self {
        Self {
            level,
            pos,
            rotation,
            exit_portal,
        }
    }
}

/// How one angle of an entity changes through a portal, in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PortalAngle {
    /// Add to the entity's angle.
    Turn(f32),
    /// Replace the entity's angle.
    Set(f32),
}

/// How an entity's yaw and pitch change through a portal. The caller clamps the resulting pitch
/// to -90 to 90, as dimension travel does.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct PortalRotation {
    /// The yaw: turned 0 or 90 between nether portals, 0 through a gateway, the world spawn's
    /// yaw out of the End, set to west (90) into it.
    pub yaw: PortalAngle,
    /// The pitch: turned by the world spawn's pitch out of the End, otherwise turned by 0.
    pub pitch: PortalAngle,
}

impl PortalRotation {
    /// A rotation.
    pub fn new(yaw: PortalAngle, pitch: PortalAngle) -> Self {
        Self { yaw, pitch }
    }
}

/// Why a portal destination is not answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PortalError {
    /// The block is no nether portal, End portal or End gateway.
    NotAPortal,
    /// The handle is stale, or the destination level does not exist.
    LevelMissing,
    /// The `allow_entering_nether_using_portals` game rule forbids the trip into the Nether.
    EnteringNotAllowed,
    /// No exit portal was found, and for `find_or_create_portal_exit` none could be built.
    NoExit,
    /// Inside a transaction: a built portal cannot be rolled back.
    InTransaction,
    /// The entity's width or height is not a finite, non-negative number a float holds.
    InvalidSize,
    /// The destination level is not open, or a chunk the search or the build reads is not loaded.
    ChunksNotLoaded,
    /// This side or build cannot answer.
    Unavailable,
}

impl crate::NoBackend for PortalError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
