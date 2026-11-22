//! The engine's process-lifetime math selection.
#![allow(unused_variables)]
use crate::*;
/// Math selection reads available through [`Host`] on both sides.
#[sunrise_mod_api_macros::host_area]
pub trait MathHost {
    /// Side: both.
    /// The native engine's fixed selection for the seven platform-dependent
    /// Java Math operations. This read allocates nothing on the native path.
    /// It identifies behavior, not the machine. See [`MathProfile`] for the algorithms
    /// selected by each profile.
    fn math_profile(&self) -> MathProfile;
}
