//! Engine paths used by vanilla Math call sites, including machine-dependent rounding.
//! These mirror sunrise_core::jmath; they are not a promise of universal HotSpot
//! bit equality. Use strict_math for established portable StrictMath operations.
pub use sunrise_core::jmath::{
    acos, asin, atan, atan2, cbrt, cos, exp, hypot, log, log10, pow, round, round_f32, sin, tan,
};
