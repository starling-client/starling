use super::*;

impl Game<'_> {
    /// The profile jmath's operations dispatch on (`jmath::PLATFORM`), in the
    /// API's own type.
    pub(in crate::game) fn math_profile(&self) -> mod_api::MathProfile {
        selected_profile()
    }
}

fn selected_profile() -> mod_api::MathProfile {
    match sunrise_core::jmath::PLATFORM {
        sunrise_core::jmath::Platform::Aarch64 => mod_api::MathProfile::Aarch64Fma,
        sunrise_core::jmath::Platform::Windows => mod_api::MathProfile::CorrectlyRounded,
        sunrise_core::jmath::Platform::Libm => mod_api::MathProfile::LibmRounding,
    }
}

#[cfg(test)]
#[path = "math/tests.rs"]
mod tests;
