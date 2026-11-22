/// The native engine's process-lifetime selection for `sin`, `cos`, `tan`,
/// `exp`, `log`, `log10` and `pow`, read through [`crate::Host`].
///
/// This describes the algorithms [`crate::java_math`] uses, not machine identity.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum MathProfile {
    /// Platform libm for `sin`, `cos`, `tan` and `pow` (with Java's `pow`
    /// special cases); HotSpot's x86-64 `exp` algorithm; correctly rounded
    /// `log` and `log10`. Selected on targets that are neither x86-64 nor
    /// AArch64, other than Windows.
    LibmRounding,
    /// Correctly rounded `sin`, `cos`, `log` and `log10`; HotSpot's x86-64
    /// `exp` algorithm; platform libm for `tan` and `pow` (including Java's
    /// `pow` special cases). This is deliberately a mixed path, not correctly
    /// rounded math throughout. Selected on Windows targets that are neither
    /// x86-64 nor AArch64.
    CorrectlyRounded,
    /// fdlibm with fused multiply-adds for `sin` and `cos`; fdlibm for `log`
    /// and `log10`; the Rust `libm` crate for `tan`, `exp` and `pow` (with
    /// Java's special cases). The latter are not promised to be StrictMath.
    Aarch64Fma,
    /// fdlibm throughout. Reserved for a future selection: no current engine
    /// cfg branch selects this variant, including non-x86/non-AArch64 targets.
    StrictMath,
    /// HotSpot's x86-64 stubs for `sin`, `cos` and `tan` (a translation of
    /// Intel's libm) and for `exp`; correctly rounded `log` and `log10`;
    /// platform libm for `pow` (with Java's `pow` special cases). Selected on
    /// every x86-64 target, Windows included.
    HotSpotX86_64,
}
