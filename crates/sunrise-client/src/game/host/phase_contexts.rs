//! Phase-scoped context trait implementations for [`Game`].
use super::Game;
use mod_api::*;
use std::sync::Arc;

impl ConfigContext for Game<'_> {
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        <Self as prelude::ConfigHost>::setting(self, mod_id, key)
    }

    fn config_files(&self) -> &'static dyn ConfigFiles {
        <Self as prelude::ConfigHost>::config_files(self)
    }
}

impl ReloadContext for Game<'_> {
    fn dist(&self) -> Dist {
        <Self as prelude::ConfigHost>::dist(self)
    }
    fn game_version(&self) -> &'static str {
        <Self as prelude::ConfigHost>::game_version(self)
    }
    fn server_data_version(&self) -> i32 {
        <Self as prelude::ConfigHost>::server_data_version(self)
    }
    fn sunrise_version(&self) -> &'static str {
        <Self as prelude::ConfigHost>::sunrise_version(self)
    }
    fn on(&self, mod_id: &str) -> bool {
        <Self as prelude::ConfigHost>::on(self, mod_id)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        <Self as prelude::ConfigHost>::setting(self, mod_id, key)
    }
    fn game_tests_enabled(&self) -> bool {
        <Self as prelude::ConfigServerHost>::game_tests_enabled(self)
    }
    fn is_loaded(&self, id: &str) -> bool {
        <Self as prelude::LevelsHost>::is_loaded(self, id)
    }
    fn item_cooldown_group(&self, stack: &ItemStack) -> String {
        <Self as prelude::PlayersHost>::item_cooldown_group(self, stack)
    }
    fn max_stack_size(&self, item: ItemKey) -> Option<u32> {
        <Self as prelude::ItemsHost>::max_stack_size(self, item)
    }
    fn max_damage(&self, item: ItemKey) -> Option<u32> {
        <Self as prelude::ItemsHost>::max_damage(self, item)
    }
    fn item_key(&self, id: &Identifier) -> Option<ItemKey> {
        <Self as prelude::ItemsHost>::item_key(self, id)
    }
    fn item_default_component(&self, item: ItemKey, component: &str) -> Option<Tag> {
        <Self as prelude::ItemsHost>::item_default_component(self, item, component)
    }
    fn hover_name(&self, stack: &ItemStack) -> Result<Text, TextFormError> {
        <Self as prelude::ItemsHost>::hover_name(self, stack)
    }
    fn item_burn_time(&self, item: ItemKey) -> Option<i32> {
        <Self as prelude::ItemsHost>::item_burn_time(self, item)
    }
    fn item_compost_chance(&self, item: ItemKey) -> Option<f32> {
        <Self as prelude::ItemsHost>::item_compost_chance(self, item)
    }
    fn entity_type_in_tag(&self, entity_type: &str, tag: &str) -> bool {
        <Self as prelude::ItemsHost>::entity_type_in_tag(self, entity_type, tag)
    }
    fn data_map_value(
        &self,
        registry: &str,
        entry: &str,
        map: &str,
    ) -> Result<Option<&Tag>, DataMapError> {
        <Self as prelude::DataMapsBackend>::data_map_value(self, registry, entry, map)
    }
    fn data_map_entries(
        &self,
        registry: &str,
        map: &str,
    ) -> Result<&[(String, Tag)], DataMapError> {
        <Self as prelude::DataMapsBackend>::data_map_entries(self, registry, map)
    }
    fn game_event_key_id(&self, key: GameEventKey) -> Result<RegistryName, GameEventKeyError> {
        <Self as prelude::KeysBackend>::game_event_key_id(self, key)
    }
    fn block_key_id(&self, key: BlockKey) -> Option<&str> {
        <Self as prelude::KeysBackend>::block_key_id(self, key)
    }
    fn item_key_id(&self, key: ItemKey) -> Option<&str> {
        <Self as prelude::KeysBackend>::item_key_id(self, key)
    }
    fn item_registry_id(&self, key: ItemKey) -> Option<u32> {
        <Self as prelude::KeysBackend>::item_registry_id(self, key)
    }
    fn effect_key_id(&self, key: EffectKey) -> Option<&str> {
        <Self as prelude::KeysBackend>::effect_key_id(self, key)
    }
    fn effect_category(&self, key: EffectKey) -> Option<EffectCategory> {
        <Self as prelude::KeysBackend>::effect_category(self, key)
    }
    fn fluid_key_id(&self, key: FluidKey) -> Option<&str> {
        <Self as prelude::KeysBackend>::fluid_key_id(self, key)
    }
    fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> Option<&str> {
        <Self as prelude::KeysBackend>::block_entity_type_key_id(self, key)
    }
    fn entity_type_key_id(&self, key: EntityTypeKey) -> Option<RegistryName> {
        <Self as prelude::KeysBackend>::entity_type_key_id(self, key)
    }
    fn menu_key_id(&self, key: MenuKey) -> Option<RegistryName> {
        <Self as prelude::KeysBackend>::menu_key_id(self, key)
    }
    fn enchantment_key_id(&self, key: EnchantmentKey) -> Option<RegistryName> {
        <Self as prelude::KeysBackend>::enchantment_key_id(self, key)
    }
    fn biome_key_id(&self, key: BiomeKey) -> Result<&str, BiomeKeyError> {
        <Self as prelude::KeysBackend>::biome_key_id(self, key)
    }
    fn dimension_key_id(&self, key: DimensionKey) -> Option<&str> {
        <Self as prelude::LevelDimensionBackend>::dimension_key_id(self, key)
    }
    fn recipe_ingredient_item_keys(
        &self,
        ingredient: &Ingredient,
    ) -> Result<Arc<[ItemKey]>, IngredientKeysError> {
        <Self as prelude::RecipesBackend>::recipe_ingredient_item_keys(self, ingredient)
    }
}

impl ClientReloadHost for Game<'_> {
    fn render_assets(&self) -> &dyn RenderAssets {
        <Self as prelude::ServicesClientHost>::render_assets(self)
    }
    fn instance_groups(&self) -> &dyn InstanceGroups {
        <Self as prelude::ServicesClientHost>::instance_groups(self)
    }
    fn instance_group_write(
        &self,
        group: InstanceGroupRef,
        first: u32,
        instances: &[ModelInstance],
    ) -> Result<(), GroupError> {
        <Self as prelude::ClientRenderClientHost>::instance_group_write(
            self, group, first, instances,
        )
    }
    fn instance_group_set_len(&self, group: InstanceGroupRef, len: u32) -> Result<(), GroupError> {
        <Self as prelude::ClientRenderClientHost>::instance_group_set_len(self, group, len)
    }
    fn instance_group_set_transform(
        &self,
        group: InstanceGroupRef,
        transform: [f32; 12],
    ) -> Result<(), GroupError> {
        <Self as prelude::ClientRenderClientHost>::instance_group_set_transform(
            self, group, transform,
        )
    }
    fn instance_group_destroy(&self, group: InstanceGroupRef) -> Result<(), GroupError> {
        <Self as prelude::ClientRenderClientHost>::instance_group_destroy(self, group)
    }
    fn item_uses_block_light(&self, stack: &ItemStack, context: ItemDisplayContext) -> bool {
        <Self as prelude::ClientRenderClientHost>::item_uses_block_light(self, stack, context)
    }
    fn block_sprites(&self, blocks: &[BlockState]) -> Vec<Option<BlockSprites>> {
        <Self as prelude::ClientRenderClientHost>::block_sprites(self, blocks)
    }
    fn texture_sprites(&self, textures: &[&str]) -> Vec<Option<SpriteRef>> {
        <Self as prelude::ClientRenderClientHost>::texture_sprites(self, textures)
    }
    fn gui_sprite(&self, sprite: &Identifier) -> Option<AtlasRegion> {
        <Self as prelude::ClientRenderClientHost>::gui_sprite(self, sprite)
    }
    fn block_sprites_version(&self) -> u64 {
        <Self as prelude::ClientRenderClientHost>::block_sprites_version(self)
    }
}

impl ClientReloadListenerContext for Game<'_> {
    fn dist(&self) -> Dist {
        <Self as prelude::ConfigHost>::dist(self)
    }
    fn game_version(&self) -> &'static str {
        <Self as prelude::ConfigHost>::game_version(self)
    }
    fn server_data_version(&self) -> i32 {
        <Self as prelude::ConfigHost>::server_data_version(self)
    }
    fn sunrise_version(&self) -> &'static str {
        <Self as prelude::ConfigHost>::sunrise_version(self)
    }
    fn game_tests_enabled(&self) -> bool {
        <Self as prelude::ConfigServerHost>::game_tests_enabled(self)
    }
    fn on(&self, mod_id: &str) -> bool {
        <Self as prelude::ConfigHost>::on(self, mod_id)
    }
    fn is_loaded(&self, id: &str) -> bool {
        <Self as prelude::LevelsHost>::is_loaded(self, id)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        <Self as prelude::ConfigHost>::setting(self, mod_id, key)
    }
    fn math_profile(&self) -> MathProfile {
        <Self as prelude::MathHost>::math_profile(self)
    }
    fn text_string(&self, text: &Text) -> String {
        <Self as prelude::TextHost>::text_string(self, text)
    }
    fn published(&self) -> Option<&dyn ClientReloadHost> {
        Some(self)
    }
}

impl ClientSetupContext for Game<'_> {
    fn config_files(&self) -> &'static dyn ConfigFiles {
        <Self as prelude::ConfigHost>::config_files(self)
    }
    fn data_files(&self) -> &'static dyn ConfigFiles {
        <Self as prelude::ConfigHost>::data_files(self)
    }
    fn translate(&self, key: &str) -> Option<String> {
        <Self as prelude::ConfigClientHost>::translate(self, key)
    }
    fn enchantments(&self) -> Vec<EnchantmentInfo> {
        <Self as prelude::ItemsClientHost>::enchantments(self)
    }
    fn potions(&self) -> Vec<PotionInfo> {
        <Self as prelude::ItemsClientHost>::potions(self)
    }
    fn enchantment_min_cost(&self, id: &str, level: u32) -> Option<i32> {
        <Self as prelude::ItemsClientHost>::enchantment_min_cost(self, id, level)
    }
    fn enchantment_supports(&self, id: &str, item: ItemKey) -> bool {
        <Self as prelude::ItemsClientHost>::enchantment_supports(self, id, item)
    }
    fn sound_events(&self) -> Vec<Identifier> {
        <Self as prelude::EffectsClientHost>::sound_events(self)
    }
    fn audio_devices(&self) -> Vec<String> {
        <Self as prelude::EffectsClientHost>::audio_devices(self)
    }
    fn audio_device(&self) -> String {
        <Self as prelude::EffectsClientHost>::audio_device(self)
    }
    fn play_local_sound(&self, sound: &SoundPlay, after: u32) -> Option<SoundRef> {
        <Self as prelude::EffectsClientHost>::play_local_sound(self, sound, after)
    }
    fn muffle_sounds(&mut self, amount: f32, lowpass: f32, ticks: u32, exempt: &[&str]) -> bool {
        <Self as prelude::EffectsClientHost>::muffle_sounds(self, amount, lowpass, ticks, exempt)
    }
    fn particle_count(&self) -> u32 {
        <Self as prelude::EffectsClientHost>::particle_count(self)
    }
    fn clear_particles(&self) -> bool {
        <Self as prelude::EffectsClientHost>::clear_particles(self)
    }
    fn sound_state(&self, sound: SoundRef) -> Option<SoundState> {
        <Self as prelude::EffectsClientBackend>::sound_state(self, sound)
    }
    fn sound_stop(&self, sound: SoundRef) -> bool {
        <Self as prelude::EffectsClientBackend>::sound_stop(self, sound)
    }
    fn sound_fade(&self, sound: SoundRef, volume: f32, ticks: u32) -> bool {
        <Self as prelude::EffectsClientBackend>::sound_fade(self, sound, volume, ticks)
    }
    fn sound_move_to(&self, sound: SoundRef, position: [f64; 3]) -> bool {
        <Self as prelude::EffectsClientBackend>::sound_move_to(self, sound, position)
    }
    fn sound_set_pitch(&self, sound: SoundRef, pitch: f32) -> bool {
        <Self as prelude::EffectsClientBackend>::sound_set_pitch(self, sound, pitch)
    }
    fn vanilla_key_mapping_binding(
        &self,
        mapping: VanillaKeyMappingRef,
    ) -> Result<&str, MappingError> {
        <Self as prelude::ClientInputClientBackend>::vanilla_key_mapping_binding(self, mapping)
    }
    fn vanilla_key_mapping_is_down(
        &self,
        mapping: VanillaKeyMappingRef,
    ) -> Result<bool, MappingError> {
        <Self as prelude::ClientInputClientBackend>::vanilla_key_mapping_is_down(self, mapping)
    }
    fn vanilla_key_mapping_physical_down(
        &self,
        mapping: VanillaKeyMappingRef,
    ) -> Result<bool, MappingError> {
        <Self as prelude::ClientInputClientBackend>::vanilla_key_mapping_physical_down(
            self, mapping,
        )
    }
    fn vanilla_key_mapping_translated_key_message(
        &self,
        mapping: VanillaKeyMappingRef,
    ) -> Result<Text, MappingError> {
        <Self as prelude::ClientInputClientBackend>::vanilla_key_mapping_translated_key_message(
            self, mapping,
        )
    }
    fn mod_key_mapping_binding(
        &self,
        mapping: ModKeyMappingRef,
    ) -> Result<(&str, ClientModifiers), ModKeyRefusal> {
        <Self as prelude::ClientInputClientBackend>::mod_key_mapping_binding(self, mapping)
    }
    fn mod_key_mapping_is_down(&self, mapping: ModKeyMappingRef) -> Result<bool, ModKeyRefusal> {
        <Self as prelude::ClientInputClientBackend>::mod_key_mapping_is_down(self, mapping)
    }
    fn mod_key_mapping_translated_key_message(
        &self,
        mapping: ModKeyMappingRef,
    ) -> Result<Text, MappingError> {
        <Self as prelude::ClientInputClientBackend>::mod_key_mapping_translated_key_message(
            self, mapping,
        )
    }
    fn mod_key_mapping_consume_click(
        &mut self,
        mapping: ModKeyMappingRef,
    ) -> Result<bool, ModKeyRefusal> {
        <Self as prelude::ClientInputClientBackendMut>::mod_key_mapping_consume_click(self, mapping)
    }
    fn mod_key_mapping_set_binding(
        &mut self,
        mapping: ModKeyMappingRef,
        key: &str,
        modifiers: ClientModifiers,
    ) -> Result<(), ModKeyRefusal> {
        <Self as prelude::ClientInputClientBackendMut>::mod_key_mapping_set_binding(
            self, mapping, key, modifiers,
        )
    }
    fn mod_key_mapping_reset_binding(
        &mut self,
        mapping: ModKeyMappingRef,
    ) -> Result<(), ModKeyRefusal> {
        <Self as prelude::ClientInputClientBackendMut>::mod_key_mapping_reset_binding(self, mapping)
    }
    fn gui_ticks(&self) -> i32 {
        <Self as prelude::ClientViewClientHost>::gui_ticks(self)
    }
    fn hud_mode(&self) -> HudMode {
        <Self as prelude::ClientViewClientHost>::hud_mode(self)
    }
    fn set_hud_mode(&mut self, mode: HudMode) {
        <Self as prelude::ClientViewClientHost>::set_hud_mode(self, mode)
    }
    fn camera_type(&self) -> CameraType {
        <Self as prelude::ClientViewClientHost>::camera_type(self)
    }
    fn set_camera_type(&self, camera_type: CameraType) {
        <Self as prelude::ClientViewClientHost>::set_camera_type(self, camera_type)
    }
    fn is_paused(&self) -> bool {
        <Self as prelude::ClientViewClientHost>::is_paused(self)
    }
    fn menu_blur_radius(&self) -> f32 {
        <Self as prelude::ScreensClientHost>::menu_blur_radius(self)
    }
    fn window_focused(&self) -> bool {
        <Self as prelude::ScreensClientHost>::window_focused(self)
    }
    fn debug_overlay_visible(&self) -> bool {
        <Self as prelude::ScreensClientHost>::debug_overlay_visible(self)
    }
    fn is_key_down(&self, key: &str) -> bool {
        <Self as prelude::ScreensClientHost>::is_key_down(self, key)
    }
    fn observed_screen_id(&self) -> Option<ScreenId> {
        <Self as prelude::ScreensClientHost>::observed_screen_id(self)
    }
    fn mod_screen(&self) -> Option<&str> {
        <Self as prelude::ScreensClientHost>::mod_screen(self)
    }
    fn screen_title(&self) -> Option<String> {
        <Self as prelude::ScreensClientHost>::screen_title(self)
    }
}
