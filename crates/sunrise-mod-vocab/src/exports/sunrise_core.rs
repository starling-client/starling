pub use sunrise_core::{game_mode::GameMode, geometry::Direction, position::BlockPos};

/// Native engine Java Math paths. Results can differ by machine as vanilla's do.
/// [`Host::math_profile`](crate::Host::math_profile) describes the engine's selection.
pub mod java_math;
/// Platform-independent, host-call-free StrictMath functions, including sin/cos/tan/exp/pow.
pub mod strict_math;
pub use sunrise_core::nbt::{Error as NbtError, NETWORK_NBT_QUOTA};
