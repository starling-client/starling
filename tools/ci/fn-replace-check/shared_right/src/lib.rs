//! One side of the diamond: a mod using `itoa`, a dependency of `fnr_shared_tip`.
#[derive(Default)]
struct Mod;

#[mod_api::sunrise_mod]
impl mod_api::Hooks for Mod {}

/// Formats with this mod's registry dependency, for `fnr_shared_tip`.
pub fn format(value: i64) -> String {
    itoa::Buffer::new().format(value).to_owned()
}
