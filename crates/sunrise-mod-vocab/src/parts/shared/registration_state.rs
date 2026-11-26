//! Registration state, separate from dispatch operations.
use super::*;

/// What every mod's [`Hooks::register`](crate::Hooks::register) attached.
#[derive(Default)]
pub(crate) struct Installed {
    /// Names this installation among all of the process's; never reused, unlike an address.
    pub(crate) epoch: u64,
    pub(crate) dependencies: Arc<crate::__private::EffectiveDependencies>,
    pub(crate) effect_ticks: crate::parts::effects::ResolvedEffectTicks,
    pub(crate) configs: Vec<crate::configs::RegisteredConfig>,
    pub(crate) menu_screen_factories: HashMap<String, crate::parts::menus::MenuScreenRegistration>,
    pub(crate) menu_recipe_books: HashMap<String, crate::parts::menus::MenuBookDeclaration>,
    pub(crate) recipe_book_types: HashMap<String, String>,
    pub(crate) recipe_book_fields: HashMap<String, (String, String)>,
    pub(crate) recipe_book_filters: HashMap<String, crate::RecipeBookFilterLook>,
    pub(crate) game_rule_categories: Vec<String>,
    pub(crate) game_rules: Vec<crate::GameRuleDecl>,
    pub(crate) attachment_types:
        HashMap<&'static str, HashMap<&'static str, crate::RegisteredAttachment>>,
    pub(crate) custom_stats: Vec<String>,
    pub(crate) custom_stat_owners: Vec<&'static str>,
    /// Enchanting table scan offsets the mods declared, in load order.
    pub(crate) enchanting_table_offsets: Vec<[i32; 3]>,
    pub(crate) game_rule_slots: Vec<u32>,
    pub(crate) keys: Parts,
    pub(crate) tags: TagParts,
    pub(crate) block_entity_types: Vec<(&'static str, BlockEntityTypeDecl)>,
    pub(crate) block_entity_ticker_selectors:
        Vec<crate::parts::block_entities::RegisteredBlockEntityTickerSelector>,
    pub(crate) block_entity_transforms:
        HashMap<String, crate::parts::block_entities::RegisteredBlockEntityTransform>,
    pub(crate) entity_attributes: Vec<EntityAttributeDeclaration>,
    pub(crate) world_fluids: Vec<(&'static str, (String, crate::WorldFluid))>,
    pub(crate) fluid_interactions: Vec<crate::__private::RegisteredFluidInteraction>,
    pub(crate) entity_attribute_modifications: Vec<EntityAttributeModification>,
    pub(crate) attributes: Vec<super::attributes::AttributeDeclaration>,
    pub(crate) attribute_widenings: Vec<super::attributes::AttributeBoundsWidening>,
    pub(crate) general_entity_types: Vec<crate::parts::entity_types::GeneralEntityTypeDeclaration>,
    pub(crate) spawn_placements: Vec<SpawnPlacementDeclaration>,
    pub(crate) entity_part_states: Vec<crate::entity::part_state::EntityPartStateDeclaration>,
    pub(crate) biome_modifiers: Vec<BiomeModifierDeclaration>,
    pub(crate) biome_regions: Vec<BiomeRegionDeclaration>,
    pub(crate) datapack_registries:
        Vec<crate::world::datapack_registry::DatapackRegistryDeclaration>,
    pub(crate) registry_keys: sunrise_mod_runtime::registry_keys::Requests,
    pub(crate) effects: OnceLock<sunrise_mod_runtime::registry_keys::Effects>,
    pub(crate) resolved_block_entity_types: OnceLock<sunrise_mod_runtime::registry_keys::Names>,
    pub(crate) fluids: OnceLock<sunrise_mod_runtime::registry_keys::Names>,
    pub(crate) resolved_items: OnceLock<sunrise_mod_runtime::registry_keys::Names>,
    pub(crate) resolved_game_events: OnceLock<sunrise_mod_runtime::registry_keys::Names>,
    pub(crate) resolved_entity_types: OnceLock<sunrise_mod_runtime::registry_keys::EntityTypes>,
    /// Attribute suppliers from the accepted core tables, paired with this installation's keys.
    pub(crate) entity_type_defaults:
        std::sync::RwLock<Option<Arc<crate::entity::entity_type_defaults::Suppliers>>>,
    // Re-resolution retains slots within one installation, never across fixture epochs.
    pub(crate) resolved_menus: std::sync::RwLock<Option<sunrise_mod_runtime::registry_keys::Names>>,
    pub(crate) resolved_blocks: OnceLock<Arc<sunrise_mod_runtime::block_definitions::Blocks>>,
    pub(crate) particles: OnceLock<sunrise_mod_runtime::registry_keys::Particles>,
    /// The ticket controllers the mods registered; a [`TicketController`]'s slot indexes it.
    pub(crate) controllers: Vec<ControllerInfo>,
    pub(crate) consume_effects: Vec<crate::parts::RegisteredConsumeEffect>,
    pub(crate) arm_poses: Vec<crate::client::arm_pose::PoseInfo>,
    pub(crate) structure_types: Vec<crate::world::structure_type::StructureTypeInfo>,
    pub(crate) pool_element_types: Vec<crate::world::pool_element::PoolElementTypeInfo>,
    pub(crate) structure_processor_types:
        Vec<crate::world::structure_processor::StructureProcessorTypeInfo>,
    pub(crate) rule_test_types: Vec<crate::world::rule_test::RuleTestTypeInfo>,
    pub(crate) pos_rule_test_types: Vec<crate::world::rule_test::PosRuleTestTypeInfo>,
    pub(crate) feature_types: Vec<crate::world::feature_type::FeatureTypeInfo>,
    pub(crate) placement_modifier_types:
        Vec<crate::world::placement_modifier::PlacementModifierTypeInfo>,
    pub(crate) loot_modifier_serializers:
        Vec<crate::parts::loot_modifiers::LootModifierSerializerInfo>,
    pub(crate) loot_function_types: Vec<crate::parts::loot_types::LootFunctionTypeInfo>,
    pub(crate) loot_condition_types: Vec<crate::parts::loot_types::LootConditionTypeInfo>,
    pub(crate) dimensions: Vec<crate::world::dimension::DimensionInfo>,
    /// The world clocks the mods registered, as `namespace:path` ids.
    pub(crate) world_clocks: Vec<Box<str>>,
    pub(crate) chunk_generators: Vec<crate::world::chunk_generator::ChunkGeneratorInfo>,
    pub(crate) biome_sources: Vec<crate::world::biome_source::BiomeSourceInfo>,
    /// Loading warnings for each owning mod, in registration order.
    pub(crate) warnings: Vec<(&'static str, Vec<crate::LoadingMessage>)>,
    pub(crate) hook_bits: Vec<bool>,
    pub(crate) tag_hook_bits: Vec<bool>,
}
