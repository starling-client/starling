/// Where the player is asking `canStartSwimming`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum SwimmingCheck {
    /// `Entity.updateSwimming` is considering entry. It still excludes passengers.
    Entry,
    /// `Entity.isInShallowWater` is being consulted for local sprint eligibility.
    ShallowWater,
}
