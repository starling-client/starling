//! Restricted views for registration, data loading, configuration, reload, and client setup.
use std::{collections::BTreeMap, sync::Arc};

/// Fallback empty [`crate::ConfigFiles`] service returned by default context implementations.
struct EmptyConfigFiles;

impl crate::ConfigFiles for EmptyConfigFiles {
    fn read(&self, _: &crate::ConfigPath) -> Result<Vec<u8>, crate::ConfigFileError> {
        Err(crate::ConfigFileError::NoCallingMod)
    }
    fn write(&self, _: &crate::ConfigPath, _: &[u8]) -> Result<(), crate::ConfigFileError> {
        Err(crate::ConfigFileError::NoCallingMod)
    }
    fn list(
        &self,
        _: &crate::ConfigPath,
        _: Option<&str>,
        _: u32,
    ) -> Result<Vec<crate::ConfigEntry>, crate::ConfigFileError> {
        Err(crate::ConfigFileError::NoCallingMod)
    }
    fn stat(
        &self,
        _: &crate::ConfigPath,
    ) -> Result<Option<crate::ConfigStat>, crate::ConfigFileError> {
        Err(crate::ConfigFileError::NoCallingMod)
    }
    fn remove(&self, _: &crate::ConfigPath) -> Result<bool, crate::ConfigFileError> {
        Err(crate::ConfigFileError::NoCallingMod)
    }
}

const EMPTY_CONFIG_FILES: EmptyConfigFiles = EmptyConfigFiles;

/// Context for config registration and initial loading (`register_configs`, `config_loading`).
/// Exposes mod settings and the calling mod's `config/` files, not a game host.
/// Config reloading and unloading instead lend the polling or stopping game's host.
pub trait ConfigContext {
    /// A mod's registered setting `key`. Returns `None` for another mod's id or an
    /// unregistered key.
    fn setting(&self, _mod_id: &str, _key: &str) -> Option<String> {
        None
    }
    /// The calling mod's own files in the game's `config/` folder.
    fn config_files(&self) -> &'static dyn crate::ConfigFiles {
        &EMPTY_CONFIG_FILES
    }
}

/// Phase-scoped read-only context for data-reload hooks (`add_server_reload_listeners`,
/// `data_maps_updated`, `default_components_bound`, `tags_updated`).
///
/// Exposes process metadata, completed registries, tags, recipes, data maps, and published
/// data resources without world, level, entity, or mutation access. Use `block_states()` and
/// `fluid_states()` for table lookups, and pass this context to [`crate::ItemResource::damageable`],
/// [`crate::ItemResource::foil`], or [`crate::ItemResource::repairable`] for item metadata.
pub trait ReloadContext:
    crate::prelude::BlockStatesHost
    + crate::HostBlockStates
    + crate::HostFluidStates
    + crate::HostItemResourceMetadata
    + crate::prelude::KeysHost
    + crate::prelude::TagsHost
    + crate::prelude::TextHost
    + crate::prelude::RecipesHost
    + crate::prelude::RecipePropertiesHost
    + crate::prelude::ResourcesHost
    + crate::prelude::MathHost
    + crate::prelude::PayloadTokensHost
    + crate::prelude::PayloadDropsHost
{
    /// Physical process, not the logical side of this hook.
    fn dist(&self) -> crate::Dist {
        crate::Dist::DedicatedServer
    }
    /// Exact Minecraft version supported by this process.
    fn game_version(&self) -> &'static str {
        ""
    }
    /// The engine's `DataVersion` written into level data and chunks.
    fn server_data_version(&self) -> i32 {
        0
    }
    /// Sunrise version printed in crash reports.
    fn sunrise_version(&self) -> &'static str {
        ""
    }
    /// Whether the player left mod `mod_id` enabled.
    fn on(&self, _mod_id: &str) -> bool {
        false
    }
    /// A mod's registered setting `key`.
    fn setting(&self, _mod_id: &str, _key: &str) -> Option<String> {
        None
    }
    /// Startup opt-in for test-only content.
    fn game_tests_enabled(&self) -> bool {
        false
    }
    /// Whether mod `id` is loaded in this process.
    fn is_loaded(&self, _id: &str) -> bool {
        false
    }
    /// Cooldown group id for `stack`.
    fn item_cooldown_group(&self, _stack: &crate::ItemStack) -> String {
        String::new()
    }
    /// Default max stack size of `item`.
    fn max_stack_size(&self, _item: crate::ItemKey) -> Option<u32> {
        None
    }
    /// Default durability of `item`.
    fn max_damage(&self, _item: crate::ItemKey) -> Option<u32> {
        None
    }
    /// Resolve an item identifier to its process-local key.
    fn item_key(&self, _id: &crate::Identifier) -> Option<crate::ItemKey> {
        None
    }
    /// Item `item`'s default value of component `component`.
    fn item_default_component(
        &self,
        _item: crate::ItemKey,
        _component: &str,
    ) -> Option<crate::Tag> {
        None
    }
    /// `ItemStack.getHoverName()` as a text component.
    fn hover_name(&self, _stack: &crate::ItemStack) -> Result<crate::Text, crate::TextFormError> {
        Err(crate::TextFormError::Unavailable)
    }
    /// Item `item`'s cooking-fuel burn time in ticks.
    fn item_burn_time(&self, _item: crate::ItemKey) -> Option<i32> {
        None
    }
    /// Item `item`'s chance to raise a composter's level.
    fn item_compost_chance(&self, _item: crate::ItemKey) -> Option<f32> {
        None
    }
    /// Whether `entity_type` is in entity-type tag `tag`.
    fn entity_type_in_tag(&self, _entity_type: &str, _tag: &str) -> bool {
        false
    }
    /// `DefaultAttributes.getSupplier(type).getValue(attribute)`: the entity type's default
    /// attribute value within the attribute's range, including installed mod types and additions.
    /// `None` before publication, or for an unknown type, a missing supplier or attribute.
    fn entity_type_default_attribute(&self, entity_type: &str, attribute: &str) -> Option<f64> {
        crate::__private::entity_type_default_attribute(entity_type, attribute)
    }
    /// Published NBT value of `entry` in `registry`'s data map `map`.
    fn data_map_value(
        &self,
        _registry: &str,
        _entry: &str,
        _map: &str,
    ) -> Result<Option<&crate::Tag>, crate::DataMapError> {
        Err(crate::DataMapError::Unavailable)
    }
    /// Published NBT entries of data map `map` in `registry`, sorted by entry ID.
    fn data_map_entries(
        &self,
        _registry: &str,
        _map: &str,
    ) -> Result<&[(String, crate::Tag)], crate::DataMapError> {
        Err(crate::DataMapError::Unavailable)
    }
    /// Pin the exact id of an installed game-event slot.
    fn game_event_key_id(
        &self,
        _key: crate::GameEventKey,
    ) -> Result<crate::RegistryName, crate::GameEventKeyError> {
        Err(crate::GameEventKeyError::Unavailable)
    }
    /// Borrow the canonical block id for `key`.
    fn block_key_id(&self, _key: crate::BlockKey) -> Option<&str> {
        None
    }
    /// Borrow the canonical item id for `key`.
    fn item_key_id(&self, _key: crate::ItemKey) -> Option<&str> {
        None
    }
    /// Numeric item registry id for `key`.
    fn item_registry_id(&self, _key: crate::ItemKey) -> Option<u32> {
        None
    }
    /// Borrow the canonical effect id for `key`.
    fn effect_key_id(&self, _key: crate::EffectKey) -> Option<&str> {
        None
    }
    /// Registered `MobEffect.getCategory()` for `key`.
    fn effect_category(&self, _key: crate::EffectKey) -> Option<crate::EffectCategory> {
        None
    }
    /// Borrow the canonical fluid id for `key`.
    fn fluid_key_id(&self, _key: crate::FluidKey) -> Option<&str> {
        None
    }
    /// Borrow the canonical block-entity-type id for `key`.
    fn block_entity_type_key_id(&self, _key: crate::BlockEntityTypeKey) -> Option<&str> {
        None
    }
    /// Pin the canonical entity-type id for `key`.
    fn entity_type_key_id(&self, _key: crate::EntityTypeKey) -> Option<crate::RegistryName> {
        None
    }
    /// Pin the canonical menu-type id for `key`.
    fn menu_key_id(&self, _key: crate::MenuKey) -> Option<crate::RegistryName> {
        None
    }
    /// Pin the canonical enchantment id for `key`.
    fn enchantment_key_id(&self, _key: crate::EnchantmentKey) -> Option<crate::RegistryName> {
        None
    }
    /// Borrow the canonical biome id for `key`.
    fn biome_key_id(&self, _key: crate::BiomeKey) -> Result<&str, crate::BiomeKeyError> {
        Err(crate::BiomeKeyError::Unavailable)
    }
    /// Borrow the canonical dimension id for `key`.
    fn dimension_key_id(&self, _key: crate::DimensionKey) -> Option<&str> {
        None
    }
    /// Exact load-time item key membership for a component-independent `ingredient`.
    fn recipe_ingredient_item_keys(
        &self,
        _ingredient: &crate::Ingredient,
    ) -> Result<Arc<[crate::ItemKey]>, crate::IngredientKeysError> {
        Err(crate::IngredientKeysError::UnknownItem)
    }
}

/// Fallback empty client render-assets / instance-groups service returned by default
/// [`ClientReloadHost`] implementations (such as startup reload registration before a game exists).
struct EmptyClientReloadServices;

impl crate::RenderAssets for EmptyClientReloadServices {
    fn model(&self, _: &crate::Identifier) -> Result<crate::ModelRef, crate::ModelError> {
        Err(crate::ModelError::UnknownModel)
    }
    fn render_pipeline(
        &self,
        _: &crate::Identifier,
    ) -> Result<crate::RenderPipelineRef, crate::RenderPipelineError> {
        Err(crate::RenderPipelineError::Unknown)
    }
    fn animation(&self, _: &crate::Identifier) -> Option<crate::AnimationHolder> {
        None
    }
    fn block_state_model(
        &self,
        _: crate::BlockState,
    ) -> Result<crate::ModelRef, crate::ModelError> {
        Err(crate::ModelError::UnknownModel)
    }
    fn block_state_models(
        &self,
        _: crate::BlockState,
    ) -> Result<crate::BakedModelKeys<'_>, crate::ModelError> {
        Err(crate::ModelError::UnknownModel)
    }
    fn baked_model(
        &self,
        _: crate::BakedModelKey,
    ) -> Result<crate::BakedModelView<'_>, crate::ModelError> {
        Err(crate::ModelError::UnknownModel)
    }
}

impl crate::InstanceGroups for EmptyClientReloadServices {
    fn create_group(
        &self,
        _: crate::ModelRef,
        _: u32,
    ) -> Result<crate::InstanceGroupRef, crate::GroupError> {
        Err(crate::GroupError::Stale)
    }
}

const EMPTY_CLIENT_RELOAD_SERVICES: EmptyClientReloadServices = EmptyClientReloadServices;

/// Phase-scoped read-only host for client resource reload hooks
/// (`client_resources_reloaded`, and [`ClientReloadListenerContext::published`] on later
/// reloads).
///
/// Named `ClientReloadHost` rather than `ClientReloadContext` because [`crate::ClientReloadContext`]
/// is the per-listener asset file reader passed to `ClientReloadListener::prepare`.
pub trait ClientReloadHost: ReloadContext + crate::prelude::ItemParticlesClientHost {
    /// Published client render assets: standalone models, render pipelines, animations,
    /// and baked block-state models.
    fn render_assets(&self) -> &dyn crate::RenderAssets {
        &EMPTY_CLIENT_RELOAD_SERVICES
    }
    /// Persistent model instance groups.
    fn instance_groups(&self) -> &dyn crate::InstanceGroups {
        &EMPTY_CLIENT_RELOAD_SERVICES
    }
    /// Atomically overwrite retained model copies in `group`.
    fn instance_group_write(
        &self,
        _group: crate::InstanceGroupRef,
        _first: u32,
        _instances: &[crate::ModelInstance],
    ) -> Result<(), crate::GroupError> {
        Err(crate::GroupError::Stale)
    }
    /// Set the drawn length of `group`.
    fn instance_group_set_len(
        &self,
        _group: crate::InstanceGroupRef,
        _len: u32,
    ) -> Result<(), crate::GroupError> {
        Err(crate::GroupError::Stale)
    }
    /// Replace the transform of `group`.
    fn instance_group_set_transform(
        &self,
        _group: crate::InstanceGroupRef,
        _transform: [f32; 12],
    ) -> Result<(), crate::GroupError> {
        Err(crate::GroupError::Stale)
    }
    /// Retire retained model copies in `group`.
    fn instance_group_destroy(
        &self,
        _group: crate::InstanceGroupRef,
    ) -> Result<(), crate::GroupError> {
        Err(crate::GroupError::Stale)
    }
    /// Whether `stack`'s resolved model uses block lighting in `context`.
    fn item_uses_block_light(
        &self,
        _stack: &crate::ItemStack,
        _context: crate::ItemDisplayContext,
    ) -> bool {
        false
    }
    /// Resolved block sprites for each requested block state.
    fn block_sprites(&self, blocks: &[crate::BlockState]) -> Vec<Option<crate::BlockSprites>> {
        vec![None; blocks.len()]
    }
    /// Resolved block-atlas sprites for each requested texture name.
    fn texture_sprites(&self, textures: &[&str]) -> Vec<Option<crate::SpriteRef>> {
        vec![None; textures.len()]
    }
    /// GUI atlas region for `sprite`.
    fn gui_sprite(&self, _sprite: &crate::Identifier) -> Option<crate::AtlasRegion> {
        None
    }
    /// Monotonic version of the published block-sprites table.
    fn block_sprites_version(&self) -> u64 {
        0
    }
}

/// Phase-scoped context for `Hooks::add_client_reload_listeners`.
///
/// Initial client resource loading runs on a loader thread before any `Game` or published
/// asset tables exist, so this trait directly exposes only the process, mod-roster,
/// settings, math-profile, and local text facts available on both startup and later
/// reloads. On later reloads, [`published`](Self::published) returns `Some(&dyn ClientReloadHost)`
/// for reading the currently published client assets and registries; on initial startup it
/// returns `None`.
pub trait ClientReloadListenerContext {
    /// Physical process, not the logical side of this hook.
    fn dist(&self) -> crate::Dist {
        crate::Dist::Client
    }
    /// Exact Minecraft version supported by this process.
    fn game_version(&self) -> &'static str {
        ""
    }
    /// The engine's `DataVersion` written into level data and chunks.
    fn server_data_version(&self) -> i32 {
        0
    }
    /// Sunrise version printed in crash reports.
    fn sunrise_version(&self) -> &'static str {
        ""
    }
    /// Startup opt-in for test-only content.
    fn game_tests_enabled(&self) -> bool {
        false
    }
    /// Whether the player left mod `mod_id` enabled.
    fn on(&self, _mod_id: &str) -> bool {
        false
    }
    /// Whether mod `id` is loaded in this process.
    fn is_loaded(&self, _id: &str) -> bool {
        false
    }
    /// A mod's registered setting `key`. Returns `None` for another mod's id or an
    /// unregistered key.
    fn setting(&self, _mod_id: &str, _key: &str) -> Option<String> {
        None
    }
    /// Math backend profile active in this process.
    fn math_profile(&self) -> crate::MathProfile;
    /// Plain string form of `text` using the currently active client language (or fallback
    /// keys during initial startup before language assets are published).
    fn text_string(&self, _text: &crate::Text) -> String {
        String::new()
    }
    /// Published client assets and registries from the previous completed load.
    ///
    /// Returns `None` on initial startup before any client resources have been published,
    /// and `Some` on later reloads (F3+T, resource-pack changes, server resource packs).
    fn published(&self) -> Option<&dyn ClientReloadHost> {
        None
    }
}

/// Phase-scoped context for `Hooks::client_setup`.
///
/// Extends [`ClientReloadHost`] with client setup services (options, key mappings, audio,
/// local sound handles, text measurement, translations, client registries, config/data files,
/// and non-world window/view/screen options) without exposing world-only level, entity, or
/// gameplay mutation methods.
pub trait ClientSetupContext:
    ClientReloadHost
    + crate::prelude::AudioClientHost
    + crate::prelude::ClientInputClientHost
    + crate::prelude::ClientNarratorClientHost
    + crate::prelude::ClientOptionsClientHost
    + crate::prelude::ClientTextClientHost
    + crate::prelude::DrawClientHost
    + crate::prelude::PayloadsClientHost
    + crate::prelude::ScoreboardClientHost
{
    /// The calling mod's own files in the game's `config/` folder.
    fn config_files(&self) -> &'static dyn crate::ConfigFiles {
        &EMPTY_CONFIG_FILES
    }
    /// The calling mod's bulk files in `sunrise-mod-data/<mod id>/`.
    fn data_files(&self) -> &'static dyn crate::ConfigFiles {
        &EMPTY_CONFIG_FILES
    }
    /// Translate a language key in the client's current language.
    fn translate(&self, _key: &str) -> Option<String> {
        None
    }
    /// Registered enchantments, in registry order.
    fn enchantments(&self) -> Vec<crate::EnchantmentInfo> {
        Vec::new()
    }
    /// Registered potions, in registry order.
    fn potions(&self) -> Vec<crate::PotionInfo> {
        Vec::new()
    }
    /// `Enchantment.getMinCost(level)` for `id`.
    fn enchantment_min_cost(&self, _id: &str, _level: u32) -> Option<i32> {
        None
    }
    /// Whether enchantment `id` supports `item`.
    fn enchantment_supports(&self, _id: &str, _item: crate::ItemKey) -> bool {
        false
    }
    /// Every sound event the client's resources have (`SoundManager.getAvailableSounds`).
    fn sound_events(&self) -> Vec<crate::Identifier> {
        Vec::new()
    }
    /// The output devices sound can play on (`SoundEngine.getAvailableSoundDevices`).
    fn audio_devices(&self) -> Vec<String> {
        Vec::new()
    }
    /// The output device the options ask for (`Options.soundDevice`).
    fn audio_device(&self) -> String {
        String::new()
    }
    /// Play a local client sound (`SoundEngine.playDelayed`), returning its control handle.
    fn play_local_sound(&self, _sound: &crate::SoundPlay, _after: u32) -> Option<crate::SoundRef> {
        None
    }
    /// Muffle client sounds over `ticks` client ticks, leaving `exempt` events clear.
    fn muffle_sounds(
        &mut self,
        _amount: f32,
        _lowpass: f32,
        _ticks: u32,
        _exempt: &[&str],
    ) -> bool {
        false
    }
    /// The particles alive in the client's particle engine (`ParticleEngine.countParticles`).
    fn particle_count(&self) -> u32 {
        0
    }
    /// Clear active client particles and emitters (`ParticleEngine.clearParticles`).
    fn clear_particles(&self) -> bool {
        false
    }
    /// Current playback state of a local sound handle started with `play_local_sound`.
    fn sound_state(&self, _sound: crate::SoundRef) -> Option<crate::SoundState> {
        None
    }
    /// Stop a local sound handle.
    fn sound_stop(&self, _sound: crate::SoundRef) -> bool {
        false
    }
    /// Fade a local sound handle's volume to `volume` over `ticks` unpaused ticks.
    fn sound_fade(&self, _sound: crate::SoundRef, _volume: f32, _ticks: u32) -> bool {
        false
    }
    /// Move a playing positional local sound handle to `position`.
    fn sound_move_to(&self, _sound: crate::SoundRef, _position: [f64; 3]) -> bool {
        false
    }
    /// Set a playing local sound handle's pitch.
    fn sound_set_pitch(&self, _sound: crate::SoundRef, _pitch: f32) -> bool {
        false
    }
    /// Current binding name for a resolved vanilla key mapping handle.
    fn vanilla_key_mapping_binding(
        &self,
        _mapping: crate::VanillaKeyMappingRef,
    ) -> Result<&str, crate::MappingError> {
        Err(crate::MappingError::Unavailable)
    }
    /// Logical held state for a resolved vanilla gameplay key mapping handle.
    fn vanilla_key_mapping_is_down(
        &self,
        _mapping: crate::VanillaKeyMappingRef,
    ) -> Result<bool, crate::MappingError> {
        Err(crate::MappingError::Unavailable)
    }
    /// Physical key/button held state for a resolved vanilla key mapping handle.
    fn vanilla_key_mapping_physical_down(
        &self,
        _mapping: crate::VanillaKeyMappingRef,
    ) -> Result<bool, crate::MappingError> {
        Err(crate::MappingError::Unavailable)
    }
    /// Translated Controls label for a resolved vanilla key mapping handle.
    fn vanilla_key_mapping_translated_key_message(
        &self,
        _mapping: crate::VanillaKeyMappingRef,
    ) -> Result<crate::Text, crate::MappingError> {
        Err(crate::MappingError::Unavailable)
    }
    /// Current binding name and modifiers for a resolved mod key mapping handle.
    fn mod_key_mapping_binding(
        &self,
        _mapping: crate::ModKeyMappingRef,
    ) -> Result<(&str, crate::ClientModifiers), crate::ModKeyRefusal> {
        Err(crate::ModKeyRefusal::Unavailable)
    }
    /// Held state for a resolved mod key mapping handle.
    fn mod_key_mapping_is_down(
        &self,
        _mapping: crate::ModKeyMappingRef,
    ) -> Result<bool, crate::ModKeyRefusal> {
        Err(crate::ModKeyRefusal::Unavailable)
    }
    /// Translated Controls label for a resolved mod key mapping handle.
    fn mod_key_mapping_translated_key_message(
        &self,
        _mapping: crate::ModKeyMappingRef,
    ) -> Result<crate::Text, crate::MappingError> {
        Err(crate::MappingError::Unavailable)
    }
    /// Consume one pending click on a resolved mod key mapping handle.
    fn mod_key_mapping_consume_click(
        &mut self,
        _mapping: crate::ModKeyMappingRef,
    ) -> Result<bool, crate::ModKeyRefusal> {
        Err(crate::ModKeyRefusal::Unavailable)
    }
    /// Rebind an owned mod key mapping handle and persist it to `sunrise-mods.txt`.
    fn mod_key_mapping_set_binding(
        &mut self,
        _mapping: crate::ModKeyMappingRef,
        _key: &str,
        _modifiers: crate::ClientModifiers,
    ) -> Result<(), crate::ModKeyRefusal> {
        Err(crate::ModKeyRefusal::Unavailable)
    }
    /// Reset an owned mod key mapping handle to its declared default binding.
    fn mod_key_mapping_reset_binding(
        &mut self,
        _mapping: crate::ModKeyMappingRef,
    ) -> Result<(), crate::ModKeyRefusal> {
        Err(crate::ModKeyRefusal::Unavailable)
    }
    /// The HUD tick counter (`Hud.getGuiTicks`).
    fn gui_ticks(&self) -> i32 {
        0
    }
    /// Effective HUD and hand visibility mode.
    fn hud_mode(&self) -> crate::HudMode {
        crate::HudMode::AllVisible
    }
    /// Select HUD and hand visibility mode.
    fn set_hud_mode(&mut self, _mode: crate::HudMode) {}
    /// Current camera perspective (`Options.getCameraType`).
    fn camera_type(&self) -> crate::CameraType {
        crate::CameraType::FirstPerson
    }
    /// Request the window's camera perspective.
    fn set_camera_type(&self, _camera_type: crate::CameraType) {}
    /// Whether the client is currently paused.
    fn is_paused(&self) -> bool {
        false
    }
    /// Configured menu background blur radius (`menuBackgroundBlurriness`).
    fn menu_blur_radius(&self) -> f32 {
        0.0
    }
    /// Last window focus state reported by the window system.
    fn window_focused(&self) -> bool {
        false
    }
    /// Whether the debug overlay is currently visible.
    fn debug_overlay_visible(&self) -> bool {
        false
    }
    /// Whether a physical key or mouse button is currently down.
    fn is_key_down(&self, _key: &str) -> bool {
        false
    }
    /// The same screen identifier provided to screen hooks, if a screen is open.
    fn observed_screen_id(&self) -> Option<crate::ScreenId> {
        None
    }
    /// The mod whose custom screen is currently open, if any.
    fn mod_screen(&self) -> Option<&str> {
        None
    }
    /// The open screen's title as plain text, if any.
    fn screen_title(&self) -> Option<String> {
        None
    }
}

/// The owning boundary that starts a data load, as reported by [`DataLoadContext::origin`].
// no wire: identifies the owner of a local data load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataLoadOrigin {
    /// Server data loading, including a reload.
    ServerDataLoad,
    /// Client resource preloading.
    ClientPreload,
}

/// The side of the local resource-generation loop, as reported by [`LoopContext::side`].
// no wire: identifies the owner of a local loop, not an installation or join contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoopSide {
    /// A server loop.
    Server,
    /// A client loop.
    Client,
}

/// Whether startup registration is currently open on this thread.
///
/// Use this in a scripting binding before accepting declarations: a script called, for example,
/// from `Hooks::player_logged_in` is too late. This read does not grant a registrar or reopen
/// registration. Only the relevant registration hook can submit its declarations.
pub fn registration_open() -> bool {
    crate::dispatch_scope::backend().is_some_and(|backend| (backend.registration_open)())
}

/// Whether a loaded package contains compiled code or content alone.
// no wire: describes an entry in the local package inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackageKind {
    /// The package carries compiled code.
    Code,
    /// The package carries content alone: data and assets.
    Content,
}

/// Read-only metadata from the local package inventory, in dispatch order through [`LoadedMods`].
/// Read it directly; to send or store package facts, copy the fields you need into your own record.
///
/// ```compile_fail,E0277
/// use mod_api::{PackageInfo, wire::Writer};
/// fn write_package(writer: &mut Writer, package: &PackageInfo) {
///     writer.put(package);
/// }
/// ```
// no wire: this snapshot is for inspecting packages in the current process.
#[derive(Clone)]
#[non_exhaustive]
pub struct PackageInfo {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) kind: PackageKind,
    pub(crate) side: crate::Side,
    /// Its own namespace, then the others it declares.
    pub(crate) namespaces: Vec<String>,
    /// Its `[dependencies]` rows, sorted by id.
    pub(crate) dependencies: Vec<crate::PackageDependency>,
    pub(crate) details: crate::PackageDetails,
    /// Native runtime mod that serves code-side parts for this content package, if any.
    pub(crate) served_by: Option<String>,
}

impl PackageInfo {
    /// An owned package snapshot supplied by the loader.
    pub fn new(
        id: String,
        version: String,
        kind: PackageKind,
        side: crate::Side,
        namespaces: Vec<String>,
        dependencies: Vec<crate::PackageDependency>,
    ) -> Self {
        Self {
            id,
            version,
            kind,
            side,
            namespaces,
            dependencies,
            details: crate::PackageDetails::default(),
            served_by: None,
        }
    }
    /// The same snapshot with the manifest's descriptive fields.
    pub fn with_details(mut self, details: crate::PackageDetails) -> Self {
        self.details = details;
        self
    }
    /// The same snapshot with its serving native runtime mod ID, for content packages.
    pub fn with_served_by(mut self, served_by: Option<String>) -> Self {
        self.served_by = served_by;
        self
    }
    /// The native runtime mod that serves code-side parts for this content package, if any.
    pub fn served_by(&self) -> Option<&str> {
        self.served_by.as_deref()
    }
    /// Package identifier.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The package's version.
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Whether this package contains code.
    pub fn kind(&self) -> PackageKind {
        self.kind
    }
    /// The package's declared side.
    pub fn side(&self) -> crate::Side {
        self.side
    }
    /// The package's own namespace followed by any additional namespaces.
    pub fn namespaces(&self) -> impl Iterator<Item = &str> {
        self.namespaces.iter().map(String::as_str)
    }
    /// Whether this package declares a dependency on `id`.
    pub fn depends_on(&self, id: &str) -> bool {
        self.dependencies.iter().any(|d| d.id() == id)
    }
    /// Its declared dependencies, sorted by id.
    pub fn dependencies(&self) -> &[crate::PackageDependency] {
        &self.dependencies
    }
    /// The display name; empty when the snapshot carries no manifest details.
    pub fn name(&self) -> &str {
        &self.details.name
    }
    /// The description, or empty.
    pub fn description(&self) -> &str {
        &self.details.description
    }
    /// The authors, in manifest order.
    pub fn authors(&self) -> &[String] {
        &self.details.authors
    }
    /// The license name, or empty.
    pub fn license(&self) -> &str {
        &self.details.license
    }
    /// The home page link.
    pub fn homepage(&self) -> Option<&str> {
        self.details.homepage.as_deref()
    }
    /// Where to report problems.
    pub fn issues(&self) -> Option<&str> {
        self.details.issues.as_deref()
    }
    /// The icon's path inside the package, such as `icon.png`.
    pub fn icon_path(&self) -> Option<&str> {
        self.details.icon_path.as_deref()
    }
    /// One value of its manifest's `[properties]` table. Only the complete manifest's byte
    /// limit applies; there is no separate property count or size limit.
    pub fn property(&self, key: &str) -> Option<&crate::PropertyValue> {
        self.details.properties.get(key)
    }
}

// no wire: shared immutable loader inventory; contexts query it through LoadReader rather than transferring its Arc storage.
/// The instance directory, registries and settings that registration reads: captured by the
/// engine once, before the first registration hook, and the same for every registration step.
/// Registry and settings lookups are binary searches over sorted lists.
#[derive(Clone)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RegistrationData {
    instance_dir: crate::InstanceDirectory,
    runtime: crate::RuntimeEnvironment,
    /// Built-in registry names in ID order, by registry name.
    pub(crate) registries: Arc<[(String, Arc<[String]>)]>,
    /// Each loaded mod's settings, by mod ID then key.
    pub(crate) settings: Arc<[(String, Arc<[(String, String)]>)]>,
    /// The same load-owned behavior owners that supply the game's default providers.
    capability_coverage: Option<fn(crate::ProvidedBy, &str, &str) -> bool>,
    block_entity_coverage: Option<fn(&str, &str) -> bool>,
    /// Each package's checked runtime files, by mod ID.
    runtime_files: Arc<std::collections::BTreeMap<String, Arc<dyn crate::RuntimeFileSource>>>,
    /// Each enabled package's content files, by mod ID.
    package_files: Arc<std::collections::BTreeMap<String, Arc<dyn crate::PackageFileSource>>>,
    /// Registration exposes only the enabled and admitted rows, not all installed code.
    visible_mods: Option<LoadedMods>,
}

impl RegistrationData {
    /// The engine's snapshot: `registries` maps a registry name to its names in ID order,
    /// `settings` a mod ID to its settings, and `instance_dir` is the startup game directory.
    pub fn new(
        registries: BTreeMap<String, Vec<String>>,
        settings: BTreeMap<String, BTreeMap<String, String>>,
        instance_dir: crate::InstanceDirectory,
        runtime: crate::RuntimeEnvironment,
    ) -> Self {
        Self {
            instance_dir,
            runtime,
            capability_coverage: None,
            block_entity_coverage: None,
            runtime_files: Default::default(),
            package_files: Default::default(),
            visible_mods: None,
            registries: registries
                .into_iter()
                .map(|(name, ids)| (name, ids.into()))
                .collect(),
            settings: settings
                .into_iter()
                .map(|(id, values)| (id, values.into_iter().collect()))
                .collect(),
        }
    }
    /// The engine's initial load shares the published registration settings
    /// rather than capturing the same startup files again on its workers.
    /// Without a registration publication, the load owns a fresh capture.
    pub fn initial_settings_snapshot() -> Option<Arc<[(String, Arc<[(String, String)]>)]>> {
        registration_data().map(|data| data.settings)
    }
    /// Bind each package's checked runtime files, by mod ID: what
    /// [`RegistrationContext::runtime_files`] serves.
    pub fn with_runtime_files(
        mut self,
        files: std::collections::BTreeMap<String, Arc<dyn crate::RuntimeFileSource>>,
    ) -> Self {
        self.runtime_files = Arc::new(files);
        self
    }
    pub(crate) fn runtime_source(&self, mod_id: &str) -> Option<Arc<dyn crate::RuntimeFileSource>> {
        self.runtime_files.get(mod_id).cloned()
    }
    /// Bind each enabled package's content files, by mod ID: what
    /// [`RegistrationContext::package_files`] serves.
    pub fn with_package_files(
        mut self,
        files: std::collections::BTreeMap<String, Arc<dyn crate::PackageFileSource>>,
    ) -> Self {
        let installed = LoadedMods::new();
        self.visible_mods = Some(LoadedMods {
            entries: installed
                .iter()
                .filter(|p| files.contains_key(p.id()))
                .cloned()
                .collect(),
            authority: installed.authority,
        });
        self.package_files = Arc::new(files);
        self
    }
    pub(crate) fn package_source(&self, mod_id: &str) -> Option<Arc<dyn crate::PackageFileSource>> {
        self.package_files.get(mod_id).cloned()
    }
    /// The startup-enabled snapshot shared by the mod-facing phase contexts.
    // hidden: backend access to the snapshot; mods use their context's `mods()`.
    #[doc(hidden)]
    pub fn visible_mods(&self) -> Option<&LoadedMods> {
        self.visible_mods.as_ref()
    }
    /// Bind the game's pure type-coverage projection. It reads no world or runtime position.
    pub fn with_capability_coverage(
        mut self,
        coverage: fn(crate::ProvidedBy, &str, &str) -> bool,
        block_entity_coverage: fn(&str, &str) -> bool,
    ) -> Self {
        self.capability_coverage = Some(coverage);
        self.block_entity_coverage = Some(block_entity_coverage);
        self
    }
    fn instance_dir(&self) -> &crate::InstanceDirectory {
        &self.instance_dir
    }
    fn registry(&self, name: &str) -> Option<&[String]> {
        let at = self
            .registries
            .binary_search_by(|(known, _)| known.as_str().cmp(name))
            .ok()?;
        Some(self.registries[at].1.as_ref())
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        let mod_at = self
            .settings
            .binary_search_by(|(known, _)| known.as_str().cmp(mod_id))
            .ok()?;
        let values = &self.settings[mod_at].1;
        let at = values
            .binary_search_by(|(known, _)| known.as_str().cmp(key))
            .ok()?;
        Some(values[at].1.as_str())
    }
}

fn registration_data() -> Option<RegistrationData> {
    crate::dispatch_scope::backend().and_then(|backend| (backend.registration_data)())
}

pub(crate) fn game_capability_registered(by: crate::ProvidedBy, name: &str, id: &str) -> bool {
    if matches!(by, crate::ProvidedBy::Item)
        && crate::parts::installation::builtin_item_capability_registered(name, id)
    {
        return true;
    }
    registration_data()
        .and_then(|data| data.capability_coverage)
        .is_some_and(|coverage| coverage(by, name, id))
}

pub(crate) fn block_entity_covers(kind: &str, block: &str) -> bool {
    registration_data()
        .and_then(|data| data.block_entity_coverage)
        .is_some_and(|coverage| coverage(kind, block))
}
/// Read-only view of the startup package inventory, shared: cloning it copies no package.
// no wire: the loader publishes the inventory locally; callers borrow or clone the view.
#[derive(Clone)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct LoadedMods {
    pub(crate) entries: Arc<[PackageInfo]>,
    pub(crate) authority: Arc<crate::parts::NamespaceAuthority>,
}

impl LoadedMods {
    #[cfg(test)]
    pub(crate) fn from_entries(entries: Arc<[PackageInfo]>) -> Self {
        let authority = Arc::new(crate::parts::NamespaceAuthority::from_packages(&entries));
        Self { entries, authority }
    }

    pub(crate) fn new() -> Self {
        #[cfg(test)]
        if let Some((entries, authority)) = TEST_PACKAGES.with_borrow(Clone::clone) {
            return Self { entries, authority };
        }
        let (entries, authority) = crate::dispatch_scope::backend().map_or_else(
            || (Arc::from([]), Arc::default()),
            |backend| ((backend.packages)(), (backend.namespace_authority)()),
        );
        Self { entries, authority }
    }

    fn enabled_at_startup() -> Self {
        crate::dispatch_scope::backend()
            .and_then(|backend| (backend.visible_packages)())
            .unwrap_or_else(Self::new)
    }

    /// Package IDs in dispatch order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(PackageInfo::id)
    }

    /// Packages in dispatch order.
    pub fn iter(&self) -> impl Iterator<Item = &PackageInfo> {
        self.entries.iter()
    }

    /// Find a package by ID. This linear lookup is intended for registration, not hot paths.
    pub fn get(&self, id: &str) -> Option<&PackageInfo> {
        self.entries.iter().find(|entry| entry.id() == id)
    }
}

/// A registration phase's view. Declarations go on the hook's event; there is
/// no world, handle constructor, transaction or game Host at this point.
///
/// Every registration hook (`register`, `component_types`, `blocks`, `items`,
/// `standalone_models` and the other `Register*` hooks) reads the same frozen view,
/// captured once before the first of them runs, so an answer never depends on
/// dispatch order or on which hook asks:
///
/// - [`mods`](Self::mods): the enabled, admitted packages at startup.
/// - [`registry`](Self::registry): the built-in registries, as `registries.json` lists them.
///   Entries that mods declare during registration are not in it; a mod already has its own
///   declarations on its event or registrar.
/// - [`setting`](Self::setting): the settings on disk for the loaded packages when registration
///   began. Later writes are not seen.
///
/// ```compile_fail
/// use mod_api::{Host, RegistrationContext};
/// fn game(_: &dyn Host) {}
/// fn registration(context: &RegistrationContext) { game(context); }
/// ```
///
/// ```compile_fail
/// use mod_api::{prelude::EntitiesHost, RegistrationContext};
/// fn registration(context: &RegistrationContext) { let _ = EntitiesHost::entity(context, 1); }
/// ```
///
/// ```compile_fail
/// use mod_api::{HostTransaction, RegistrationContext};
/// fn registration(context: &mut RegistrationContext) {
///     let _: Result<(), ()> = context.transaction(|_, _| Ok(()));
/// }
/// ```
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: the phase context owns local registration state and the startup screen.
pub struct RegistrationContext {
    pub(crate) mods: LoadedMods,
    pub(crate) data: Option<RegistrationData>,
    pub(crate) declaration_refusals:
        Arc<std::sync::Mutex<Vec<crate::__private::DeclarationRefusal>>>,
    pub(crate) listener: Option<&'static str>,
    /// The game's startup screen, for [`progress`](Self::progress) and
    /// [`message`](Self::message); none without a game.
    pub(crate) startup: Option<Arc<dyn crate::StartupScreen>>,
    pub(crate) runtime: Option<crate::RuntimeEnvironment>,
}

impl RegistrationContext {
    pub(crate) fn new() -> Self {
        let data = registration_data();
        let runtime = data.as_ref().map(|data| data.runtime.clone());
        Self {
            mods: LoadedMods::new(),
            data,
            declaration_refusals: Default::default(),
            listener: None,
            startup: crate::world::startup_progress::installed(),
            runtime,
        }
    }

    pub(crate) fn for_mod(&self, mod_id: &'static str) -> Self {
        Self {
            mods: self.mods.clone(),
            data: self.data.clone(),
            declaration_refusals: Arc::clone(&self.declaration_refusals),
            listener: Some(mod_id),
            startup: self.startup.clone(),
            runtime: self.runtime.clone(),
        }
    }

    /// Physical process, independent of the logical side of a hook.
    pub fn dist(&self) -> crate::Dist {
        self.runtime().dist()
    }
    /// Exact Minecraft version captured before registration.
    pub fn game_version(&self) -> &str {
        self.runtime().game_version()
    }
    /// Sunrise version, identical to the crash report's version.
    pub fn sunrise_version(&self) -> &str {
        self.runtime().sunrise_version()
    }
    /// Startup opt-in for test-only content; does not gate test registration.
    pub fn game_tests_enabled(&self) -> bool {
        self.runtime().game_tests_enabled()
    }
    fn runtime(&self) -> &crate::RuntimeEnvironment {
        self.runtime
            .as_ref()
            .expect("registration runtime environment is installed")
    }

    /// The instance directory captured before registration, or `None` when no registration
    /// snapshot is installed. This is data only, not permission to access a filesystem.
    /// The value preserves the host OS's raw path encoding, including non-UTF-8 names.
    pub fn instance_dir(&self) -> Option<&crate::InstanceDirectory> {
        Some(self.data.as_ref()?.instance_dir())
    }

    /// Packages enabled and admitted at startup, independent of the event or registrar.
    /// Native code switched off at startup still registers but does not appear here.
    pub fn mods(&self) -> &LoadedMods {
        self.data
            .as_ref()
            .and_then(|data| data.visible_mods.as_ref())
            .unwrap_or(&self.mods)
    }

    /// Borrows a built-in registry's names in ID order, or returns `None` if the registry
    /// is unavailable or no registration snapshot is installed.
    pub fn registry(&self, name: &str) -> Option<&[String]> {
        self.data.as_ref()?.registry(name)
    }

    /// Borrows a mod's captured setting as it was when registration began, or returns `None`
    /// when it was not captured or no registration snapshot is installed.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.data.as_ref()?.setting(mod_id, key)
    }
}

/// A data load's read-only view. The loader freezes the metadata for this
/// resource generation before dispatch. The metadata getters lend that data
/// without file I/O. Copy data only if it must outlive the context borrow.
/// Reading already-loaded data never creates this context.
///
/// ```compile_fail
/// use mod_api::{DataLoadContext, Host};
/// fn game(_: &dyn Host) {}
/// fn load(context: &dyn DataLoadContext) { game(context); }
/// ```
///
/// ```compile_fail
/// use mod_api::{prelude::EntitiesHost, DataLoadContext};
/// fn load(context: &dyn DataLoadContext) { let _ = EntitiesHost::entity(context, 1); }
/// ```
///
/// ```compile_fail
/// use mod_api::{DataLoadContext, HostTransaction};
/// fn load(context: &mut dyn DataLoadContext) {
///     let _: Result<(), ()> = context.transaction(|_, _| Ok(()));
/// }
/// ```
pub trait DataLoadContext {
    /// The phase selected at the owning load boundary.
    fn origin(&self) -> DataLoadOrigin;

    /// Borrows registry names in ID order, or returns `None` if the registry
    /// is unavailable.
    fn registry(&self, name: &str) -> Option<&[String]>;

    /// Resolve a fluid name against this load's registered fluid identities.
    fn fluid_key(&self, id: &str) -> Option<crate::FluidKey>;

    /// Borrows the recipe pass's fluid tags (`data/<ns>/tags/fluid`), with each
    /// tag's members in source order. `None` means the load has no recipe
    /// overlay; `Some(&[])` means the overlay has no fluid tags.
    fn fluid_tags(&self) -> Option<&[(String, Arc<[String]>)]> {
        None
    }

    /// Borrows a captured setting, or returns `None` if it was not captured.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;

    /// Borrows the loaded-mod inventory in dispatch order.
    fn mods(&self) -> &LoadedMods;
}

/// Immutable resource-generation reads supplied by an owning loader. Return
/// borrows into the captured data; the getters must not read files.
pub trait LoadReader: Sync {
    /// Borrows registry names in ID order, or returns `None` if unavailable.
    fn registry(&self, name: &str) -> Option<&[String]>;
    /// Borrows recipe fluid tags with members in source order. Return `None`
    /// without a recipe overlay and `Some(&[])` when it has no fluid tags.
    fn fluid_tags(&self) -> Option<&[(String, Arc<[String]>)]>;
    /// Borrows a captured setting, or returns `None` if it was not captured.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;
}

// no wire: borrowed phase-scoped host readers; authority stays with their owning thread.
/// The game's owning load, as a [`DataLoadContext`].
// hidden: the game's side of the context, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private; only the game builds it
pub struct OwningLoad<'a> {
    source: &'a dyn LoadReader,
    origin: DataLoadOrigin,
    mods: LoadedMods,
}

impl<'a> OwningLoad<'a> {
    /// Borrow a loader's immutable reader at its owning boundary.
    pub fn new(origin: DataLoadOrigin, source: &'a dyn LoadReader) -> Self {
        Self {
            source,
            origin,
            mods: LoadedMods::enabled_at_startup(),
        }
    }
}

impl DataLoadContext for OwningLoad<'_> {
    fn origin(&self) -> DataLoadOrigin {
        self.origin
    }
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.source.registry(name)
    }
    fn fluid_key(&self, id: &str) -> Option<crate::FluidKey> {
        crate::content::registry_keys::fluid_key(id)
    }
    fn fluid_tags(&self) -> Option<&[(String, Arc<[String]>)]> {
        self.source.fluid_tags()
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.source.setting(mod_id, key)
    }
    fn mods(&self) -> &LoadedMods {
        &self.mods
    }
}

/// The frozen resource-generation view of a hook fired inside a menu, recipe or
/// player-list loop: the side, registries, settings, mod list and authoritative item
/// tags. A loop without a tag-bearing snapshot (for example, the sound engine) returns
/// `None` from tag reads even after world tags sync. The side that owns the loop builds
/// this view; its answers never change while held. Metadata getters return borrows
/// tied to this context and do no file I/O.
///
/// ```compile_fail
/// use mod_api::{Host, LoopContext};
/// fn game(_: &dyn Host) {}
/// fn each(context: &dyn LoopContext) { game(context); }
/// ```
///
/// ```compile_fail
/// use mod_api::{HostTransaction, LoopContext};
/// fn each(context: &mut dyn LoopContext) {
///     let _: Result<(), ()> = context.transaction(|_, _| Ok(()));
/// }
/// ```
pub trait LoopContext {
    /// Membership in this generation's authoritative item tags. `None` means the
    /// snapshot is unavailable or a member of this tag could not be bound. Absent
    /// or empty tags in a loaded snapshot answer `Some(false)`. Reads allocate
    /// nothing and perform no file I/O.
    fn is_in_tag(&self, item: crate::ItemKey, tag: crate::TagKey) -> Option<bool>;

    /// Borrow the bound item members in authoritative order for this generation.
    /// `None` means the snapshot was unavailable or a member of this tag could not
    /// be bound at publication; no partial list is returned. A missing or empty tag
    /// in a loaded snapshot returns `Some(&[])`. On clients these are the server's
    /// synced tags, never the local pack's. Reads allocate nothing.
    fn items_in_tag(&self, tag: crate::TagKey) -> Option<&[crate::ItemKey]>;

    /// Whether a mod-created recipe property set accepts an item ID. Unknown
    /// sets/items and vanilla set keys return false. Use this in
    /// `menu_slot_rules` for the same filter on server and client prediction.
    /// Answers come from the owning load, replaced by the server's synced
    /// generation on clients; no allocation or file IO occurs during a read.
    fn mod_recipe_property_contains(&self, set: &str, item: crate::ItemKey) -> bool;

    /// The side whose loop fires the hook.
    fn side(&self) -> LoopSide;

    /// Borrows registry names in ID order, or returns `None` if the registry
    /// is unavailable.
    fn registry(&self, name: &str) -> Option<&[String]>;

    /// Borrows a captured setting, or returns `None` if it was not captured.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;

    /// Borrows the loaded-mod inventory in dispatch order.
    fn mods(&self) -> &LoadedMods;
}

#[path = "loop_item_tags.rs"]
mod loop_item_tags;
pub use loop_item_tags::BoundItemTags;

/// Immutable resource-generation reads supplied by an owning loop. The getters
/// borrow the captured metadata and must not read files. Item tags are bound
/// separately when the generation is published.
pub trait LoopReader: Sync {
    /// Mod-created recipe property membership from the owning generation.
    fn mod_recipe_property_contains(&self, set: &str, item: crate::ItemKey) -> bool {
        let _ = (set, item);
        false
    }
    /// Borrows registry names in ID order, or returns `None` if unavailable.
    fn registry(&self, name: &str) -> Option<&[String]>;
    /// Borrows a captured setting, or returns `None` if it was not captured.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;
}

// no wire: borrowed phase-scoped host readers; authority stays with their owning thread.
/// The game's loop, as a [`LoopContext`].
// hidden: the game's side of the context, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private; only the game builds it
pub struct OwningLoop<'a> {
    source: &'a dyn LoopReader,
    item_tags: Option<&'a BoundItemTags>,
    side: LoopSide,
    mods: LoadedMods,
}

impl<'a> OwningLoop<'a> {
    /// Borrow a loop's immutable reader and its already-bound tags at the owning boundary.
    pub fn new(
        side: LoopSide,
        source: &'a dyn LoopReader,
        item_tags: Option<&'a BoundItemTags>,
    ) -> Self {
        Self {
            source,
            item_tags,
            side,
            mods: LoadedMods::enabled_at_startup(),
        }
    }
}

impl LoopContext for OwningLoop<'_> {
    fn is_in_tag(&self, item: crate::ItemKey, tag: crate::TagKey) -> Option<bool> {
        self.item_tags?.contains(tag, item)
    }
    fn items_in_tag(&self, tag: crate::TagKey) -> Option<&[crate::ItemKey]> {
        self.item_tags?.items(tag)
    }
    fn mod_recipe_property_contains(&self, set: &str, item: crate::ItemKey) -> bool {
        self.source.mod_recipe_property_contains(set, item)
    }
    fn side(&self) -> LoopSide {
        self.side
    }
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.source.registry(name)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.source.setting(mod_id, key)
    }
    fn mods(&self) -> &LoadedMods {
        &self.mods
    }
}

/// The view of a hook fired while a feature generates, where no game is at
/// hand: on a worldgen worker thread, or on the server thread when a sapling
/// grows. It lends the server's registries, settings and mod list from the
/// frozen resource generation. The metadata borrows are tied to this host
/// and the getters do no file I/O. There are no writes, no level or entity
/// handles and no transactions: the hook's arguments provide access to the
/// generating region.
///
/// ```compile_fail
/// use mod_api::{Host, WorldgenHost};
/// fn game(_: &dyn Host) {}
/// fn generate(host: &dyn WorldgenHost) { game(host); }
/// ```
///
/// ```compile_fail
/// use mod_api::{HostTransaction, WorldgenHost};
/// fn generate(host: &mut dyn WorldgenHost) {
///     let _: Result<(), ()> = host.transaction(|_, _| Ok(()));
/// }
/// ```
pub trait WorldgenHost {
    /// Borrows registry names in ID order, or returns `None` if the registry
    /// is unavailable.
    fn registry(&self, name: &str) -> Option<&[String]>;

    /// Borrows a captured setting, or returns `None` if it was not captured.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;

    /// Borrows the loaded-mod inventory in dispatch order.
    fn mods(&self) -> &LoadedMods;

    /// `BlockState.setValue`: `state` with its property `key` set to `value`; `state` itself
    /// when it already has that value, None when its block has no such property or value.
    fn state_with_property(
        &self,
        state: crate::BlockState,
        key: &str,
        value: &str,
    ) -> Option<crate::BlockState>;
}

// no wire: a process-local borrow of frozen worldgen metadata.
/// Immutable world-generation reads supplied by the owning engine boundary.
// hidden: engine plumbing, re-exported through __private for the game.
#[doc(hidden)]
pub trait WorldgenReader: Sync {
    /// Borrows registry names in ID order.
    fn registry(&self, name: &str) -> Option<&[String]>;
    /// Borrows a captured setting, without file I/O.
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str>;
}

// no wire: borrowed phase-scoped host readers; authority stays with their owning thread.
/// The server's world generation, as a [`WorldgenHost`].
// hidden: the game's side of the context, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private; only the game builds it
pub struct OwningWorldgen<'a> {
    source: &'a dyn WorldgenReader,
    state_with_property: &'a StateWithProperty<'a>,
    mods: LoadedMods,
}

/// `BlockState.setValue` as the game answers it for [`OwningWorldgen`].
type StateWithProperty<'a> =
    dyn Fn(crate::BlockState, &str, &str) -> Option<crate::BlockState> + 'a;

impl<'a> OwningWorldgen<'a> {
    /// Borrow the owning engine's frozen metadata reader and block-state query.
    pub fn new(
        source: &'a dyn WorldgenReader,
        state_with_property: &'a StateWithProperty<'a>,
    ) -> Self {
        Self {
            source,
            state_with_property,
            mods: LoadedMods::enabled_at_startup(),
        }
    }
}

impl WorldgenHost for OwningWorldgen<'_> {
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.source.registry(name)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.source.setting(mod_id, key)
    }
    fn mods(&self) -> &LoadedMods {
        &self.mods
    }
    fn state_with_property(
        &self,
        state: crate::BlockState,
        key: &str,
        value: &str,
    ) -> Option<crate::BlockState> {
        (self.state_with_property)(state, key, value)
    }
}

#[cfg(test)]
thread_local! {
    static TEST_PACKAGES: std::cell::RefCell<Option<(Arc<[PackageInfo]>, Arc<crate::parts::NamespaceAuthority>)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn test_owns_namespace(owner: &str, namespace: &str) -> Option<bool> {
    TEST_PACKAGES.with_borrow(|slot| {
        slot.as_ref()
            .map(|(_, authority)| authority.owns(owner, namespace))
    })
}

#[cfg(test)]
pub(crate) fn test_namespace_authority() -> Option<Arc<crate::parts::NamespaceAuthority>> {
    TEST_PACKAGES.with_borrow(|slot| slot.as_ref().map(|(_, authority)| authority.clone()))
}

#[cfg(test)]
pub(crate) fn with_test_package_inventory<R>(
    packages: Vec<PackageInfo>,
    f: impl FnOnce() -> R,
) -> R {
    let entries: Arc<[PackageInfo]> = packages.into();
    let authority = Arc::new(crate::parts::NamespaceAuthority::from_packages(&entries));
    let previous = TEST_PACKAGES.replace(Some((entries, authority)));
    let result = f();
    TEST_PACKAGES.set(previous);
    result
}

#[cfg(test)]
pub(crate) use crate::testing::TestReader;

#[test]
fn the_registration_context_lists_installed_packages_with_their_details() {
    let details = crate::PackageDetails {
        name: "Machines".to_owned(),
        properties: BTreeMap::from([("ratio".to_owned(), crate::PropertyValue::Float(0.5))]),
        ..Default::default()
    };
    let packages = vec![
        PackageInfo::new(
            "machines".to_owned(),
            "2".to_owned(),
            PackageKind::Code,
            crate::Side::Both,
            vec!["machines".to_owned()],
            vec![crate::PackageDependency::new(
                "base".to_owned(),
                "^1".to_owned(),
                crate::DependencyKind::Required,
                crate::DependencyOrder::Before,
                crate::Side::Both,
                None,
                None,
            )],
        )
        .with_details(details),
        PackageInfo::new(
            "base".to_owned(),
            "1".to_owned(),
            PackageKind::Content,
            crate::Side::Client,
            vec!["base".to_owned()],
            vec![],
        ),
    ];
    with_test_package_inventory(packages, || {
        let context = RegistrationContext::new();
        assert_eq!(
            context.mods().ids().collect::<Vec<_>>(),
            ["machines", "base"]
        );
        let machines = context.mods().get("machines").unwrap();
        assert_eq!(machines.name(), "Machines");
        assert_eq!(machines.kind(), PackageKind::Code);
        assert_eq!(
            machines.property("ratio"),
            Some(&crate::PropertyValue::Float(0.5))
        );
        assert!(machines.depends_on("base"));
        assert_eq!(
            machines.dependencies()[0].order(),
            crate::DependencyOrder::Before
        );
        let base = context.mods().get("base").unwrap();
        assert_eq!(base.kind(), PackageKind::Content);
        assert!(context.mods().get("missing").is_none());
    });
}
