//! Scoped world-data biome views belong to the same installed snapshot as data maps.
use super::Game;

impl Game<'_> {
    pub(in crate::game) fn loaded_biome_keys(&self) -> Option<&mod_api::__private::BiomeKeys> {
        self.loaded_data_maps().ok().map(|maps| &*maps.biome_keys)
    }

    pub(in crate::game) fn biome(
        &self,
        id: &str,
    ) -> Result<mod_api::BiomeView<'_>, mod_api::BiomeKeyError> {
        let maps = self
            .loaded_data_maps()
            .map_err(|_| mod_api::BiomeKeyError::Unavailable)?;
        maps.biome_keys.view(id)
    }

    pub(in crate::game) fn biome_view(
        &self,
        key: &mod_api::BiomeKey,
    ) -> Result<mod_api::BiomeView<'_>, mod_api::BiomeKeyError> {
        let maps = self
            .loaded_data_maps()
            .map_err(|_| mod_api::BiomeKeyError::Unavailable)?;
        maps.biome_keys.view_key(key)
    }

    pub(in crate::game) fn biome_view_id(
        &self,
        view: mod_api::BiomeView<'_>,
    ) -> Result<&str, mod_api::BiomeKeyError> {
        let maps = self
            .loaded_data_maps()
            .map_err(|_| mod_api::BiomeKeyError::Unavailable)?;
        maps.biome_keys.id(view)
    }
    /// The installed snapshot's keys, empty before world data is installed.
    pub(in crate::game) fn biome_key_snapshot(
        &self,
    ) -> std::sync::Arc<mod_api::__private::BiomeKeys> {
        self.world_tables
            .as_ref()
            .map(|tables| tables.data_maps.biome_keys.clone())
            .unwrap_or_default()
    }
    /// Mod features answer `FeatureContext::biome` with the keys `biome_key` gives: publish the
    /// installed snapshot to every level's generation workers and to the live level's
    /// placements, as each data load installs it.
    pub(in crate::game) fn share_biome_keys(&self) {
        if !self.options.execution_role.is_logical_server() {
            return;
        }
        let keys = self.biome_key_snapshot();
        self.res.behaviours().mod_biome_keys().publish(keys.clone());
        for terrain in self
            .end
            .locator()
            .iter()
            .chain(self.levels.parked_terrains())
        {
            terrain.set_biome_keys(&keys);
        }
    }
}
