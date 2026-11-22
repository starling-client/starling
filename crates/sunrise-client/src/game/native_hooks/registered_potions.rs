//! `ItemsClientHost::potions`: the registered potions (`Registries.POTION`) a client mod lists.
use super::*;
use std::sync::OnceLock;

/// The registered potions with their registry ids, in registry order: built on the first read
/// and kept, since the potion registry does not change while the game runs.
pub(super) fn registered_potions() -> &'static [mod_api::PotionInfo] {
    static LIST: OnceLock<Vec<mod_api::PotionInfo>> = OnceLock::new();
    LIST.get_or_init(|| {
        sunrise_client::brewing::POTIONS
            .iter()
            .map(|potion| mod_api::PotionInfo::new(format!("minecraft:{}", potion.id)))
            .collect()
    })
}

impl Game<'_> {
    /// Registered potions, in registry order, each by its registry id.
    pub(in crate::game) fn potions(&self) -> Vec<mod_api::PotionInfo> {
        registered_potions().to_vec()
    }
}
