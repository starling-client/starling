use super::*;
/// Owner-bound settings retained for live access and the client's mod pages.
pub fn settings() -> &'static [(&'static str, crate::SettingSpec)] {
    installed().map_or(&[], |installed| &installed.keys.settings)
}

/// The setting `key` mod `mod_id` registered, if any.
pub fn setting(mod_id: &str, key: &str) -> Option<&'static crate::SettingSpec> {
    settings()
        .iter()
        .find(|(owner, spec)| *owner == mod_id && spec.key == key)
        .map(|(_, spec)| spec)
}

/// The category mod `mod_id` gave its setting `key`, if any.
pub fn setting_category(mod_id: &str, key: &str) -> Option<&'static str> {
    installed().and_then(|installed| {
        installed
            .keys
            .setting_categories
            .iter()
            .find(|(owner, k, _)| *owner == mod_id && k.as_str() == key)
            .map(|(_, _, category)| category.as_str())
    })
}

/// Mint an owner-bound [`SettingKey`](crate::SettingKey) during registration.
pub fn setting_key(owner: &'static str, slot: usize) -> crate::SettingKey {
    crate::SettingKey { owner, slot }
}

/// Resolve an owner-bound [`SettingKey`](crate::SettingKey) to its installed `(owner, spec)`.
pub fn setting_entry(
    key: crate::SettingKey,
) -> Option<&'static (&'static str, crate::SettingSpec)> {
    let entry = settings().get(key.slot)?;
    (entry.0 == key.owner).then_some(entry)
}

/// Resolve an installed `(owner, key)` pair to its minted [`SettingKey`](crate::SettingKey).
pub fn setting_key_by_name(mod_id: &str, key: &str) -> Option<crate::SettingKey> {
    settings()
        .iter()
        .enumerate()
        .find(|(_, (owner, spec))| *owner == mod_id && spec.key == key)
        .map(|(slot, (owner, _))| crate::SettingKey { owner, slot })
}

/// What the Mods screen's Config button opens for the mod that registered it
/// ([`SettingsRegistrar::register_config_screen`]; NeoForge's `IConfigScreenFactory`). A
/// closure is one.
pub trait ConfigScreenFactory: Send + Sync + 'static {
    /// Config was pressed for this mod on the Mods screen, on the title screen or in a world
    /// (`IConfigScreenFactory.createScreen`). Prepare what your screen shows and return `true`
    /// to open your own screen over the Mods screen, as
    /// [`ClientHost::mod_screen_open`](crate::ClientHost::mod_screen_open) opens it:
    /// `screen_opening` can still cancel or replace it, and closing it (Escape or
    /// `mod_screen_close`) returns to the Mods screen with your mod still selected (the
    /// `parent` NeoForge passes). `false` keeps the Mods screen.
    fn create_screen(&self, host: &dyn crate::ClientHost) -> bool;
}

impl<F> ConfigScreenFactory for F
where
    F: Fn(&dyn crate::ClientHost) -> bool + Send + Sync + 'static,
{
    fn create_screen(&self, host: &dyn crate::ClientHost) -> bool {
        self(host)
    }
}

// no wire: frozen registration and panic state holding a mod's callback.
/// A mod's config screen factory, as the engine keeps it.
#[derive(Clone)]
pub(crate) struct ConfigScreenRegistration {
    pub(crate) owner: &'static str,
    pub(crate) index: usize,
    pub(crate) factory: Arc<dyn ConfigScreenFactory>,
    pub(crate) panicked: Arc<std::sync::atomic::AtomicBool>,
}

/// The config screen factory of mod `mod_id`, while the mod is enabled.
fn config_screen(mod_id: &str) -> Option<&'static ConfigScreenRegistration> {
    installed()?
        .keys
        .config_screens
        .iter()
        .find(|registration| registration.owner == mod_id)
        .filter(|registration| crate::mod_index_on(registration.index))
}

/// Whether the enabled mod `mod_id` registered a config screen factory.
pub fn has_config_screen(mod_id: &str) -> bool {
    config_screen(mod_id).is_some()
}

/// Ask mod `mod_id`'s config screen factory whether its screen opens: `None` when it has none,
/// an error when it panicked (it is not asked again).
pub fn create_config_screen(
    host: &dyn crate::ClientHost,
    mod_id: &str,
) -> Option<Result<bool, String>> {
    let registration = config_screen(mod_id)?;
    Some(crate::callback_owner::call(
        registration.owner,
        registration.index,
        &registration.panicked,
        "config screen factory",
        "config_screen_factory",
        || registration.factory.create_screen(host),
    ))
}

/// Registrar operations for settings.
pub trait SettingsRegistrar {
    /// Register a setting (NeoForge's `ModConfigSpec` entries). Called from `Hooks::register`.
    /// Returns an owner-bound [`SettingKey`](crate::SettingKey) used to read and write the setting
    /// on [`ClientHost`](crate::ClientHost) or [`ServerHost`](crate::ServerHost). The client also
    /// shows it on the calling mod's page in Options > Mods. A setting
    /// [`SettingSpec::check`](crate::SettingSpec::check) refuses, or a key the mod registered
    /// already, is not registered and comes back as the error.
    fn register_setting(
        &mut self,
        setting: crate::SettingSpec,
    ) -> Result<crate::SettingKey, crate::SettingError>;

    /// Give a setting the mod registered already a section on its page in Options > Mods:
    /// settings sharing a category are drawn under one heading, in registration order.
    /// Called from `Hooks::register`, after `register_setting` for the same key; a key
    /// the calling mod never registered, or an empty category, is refused.
    fn categorize_setting(&mut self, key: &str, category: &str) -> Result<(), crate::SettingError>;

    /// Register the screen the Mods screen's Config button opens for the calling mod
    /// (NeoForge's `IConfigScreenFactory`, registered with `ModContainer.registerExtensionPoint`).
    /// Called from `Hooks::register`. Config then shows for the mod even with no settings, and
    /// opens this screen instead of the settings page. A mod registers one: a second is not
    /// registered and comes back as the error.
    fn register_config_screen(
        &mut self,
        factory: Arc<dyn ConfigScreenFactory>,
    ) -> Result<(), String>;
}
