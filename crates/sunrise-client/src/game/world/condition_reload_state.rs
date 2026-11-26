use super::*;

pub(super) struct StagedWorldTables {
    pub(super) tables: Option<WorldTables>,
    pub(super) advancements: Arc<sunrise_client::advancements::Tree>,
    pub(super) health: sunrise_client::health::HealthResources,
    pub(super) datapack_registries: Option<mod_api::DatapackRegistrySet>,
    pub(super) datapack_vanilla_values:
        Option<super::native_datapack_values::DatapackVanillaValues>,
    pub(super) data: Arc<sunrise_client::datapack::DataPacks>,
}

pub(super) struct PreparedWorldReload {
    pub(super) structure_spawns: sunrise_client::mobs::spawn_selection::StructureSpawns,
    pub(super) advancements: Arc<sunrise_client::advancements::Tree>,
    pub(super) tables: Option<WorldTables>,
    pub(super) health: sunrise_client::health::HealthResources,
    pub(super) parts: PreparedParts,
    pub(super) wire: Option<Arc<sunrise_client::lan::data::SyncData>>,
    pub(super) wire_tags: sunrise_client::lan::data::WorldTags,
    pub(super) datapack_registries: Option<mod_api::DatapackRegistrySet>,
    pub(super) datapack_vanilla_values:
        Option<super::native_datapack_values::DatapackVanillaValues>,
    pub(super) data: Arc<sunrise_client::datapack::DataPacks>,
}
