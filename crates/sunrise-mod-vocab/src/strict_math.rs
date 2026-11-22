//! Pure, StrictMath-exact operations.
//!
//! The fdlibm `sin`, `cos`, `tan`, `exp` and `pow` algorithms use ordered IEEE
//! double operations without FMA, platform libm or `Host` calls. NaN payloads are unspecified.
pub use sunrise_core::jmath::fdlibm::{cbrt, cos, exp, hypot, log, log10, pow, sin, tan};
pub use sunrise_core::jmath::{acos, asin, atan, atan2};
#[cfg(test)]
#[path = "strict_math_tests.rs"]
mod tests;
