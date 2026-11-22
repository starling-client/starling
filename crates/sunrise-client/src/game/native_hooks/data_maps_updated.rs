use super::*;

impl Game<'_> {
    /// The data maps of `registry` (`minecraft:item`) changed on this side
    /// (NeoForge's `DataMapsUpdatedEvent`), after they are readable.
    pub(in crate::game) fn native_data_maps_updated(
        &mut self,
        registry: &str,
        cause: mod_api::DataMapsUpdateCause,
    ) {
        Self::fire_data_maps_updated(self, registry, cause);
    }
}
