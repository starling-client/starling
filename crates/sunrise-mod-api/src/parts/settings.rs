//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::*;

    #[test]
    pub(crate) fn settings_are_owned_and_bad_or_duplicate_ones_are_refused() {
        let mut registrar = super::PartRegistrar::new("first");
        let first_on = registrar
            .register_setting(SettingSpec::toggle("on", "first.on", true))
            .unwrap();
        assert_eq!(first_on.owner(), "first");
        assert_eq!(
            registrar.register_setting(SettingSpec::toggle("on", "first.on", false)),
            Err(SettingError::Duplicate("on".into()))
        );
        assert_eq!(
            registrar.register_setting(SettingSpec::range(
                "speed",
                "first.speed",
                1.0,
                0.0,
                0.1,
                0.5
            )),
            Err(SettingError::EmptyRange)
        );
        registrar.mod_id = "second";
        let second_on = registrar
            .register_setting(SettingSpec::toggle("on", "second.on", false))
            .unwrap();
        assert_eq!(second_on.owner(), "second");
        assert_ne!(first_on, second_on);
        let owners: Vec<_> = registrar
            .parts
            .settings
            .iter()
            .map(|(owner, s)| (*owner, s.key.as_str()))
            .collect();
        assert_eq!(owners, [("first", "on"), ("second", "on")]);
        assert!(
            registrar.refused.is_empty(),
            "a refused setting is the mod's error, not startup's"
        );
    }

    #[test]
    pub(crate) fn categories_are_owned_and_unknown_or_empty_ones_are_refused() {
        let mut registrar = super::PartRegistrar::new("first");
        registrar
            .register_setting(SettingSpec::toggle("on", "first.on", true))
            .unwrap();
        registrar
            .register_setting(SettingSpec::toggle("off", "first.off", false))
            .unwrap();
        assert_eq!(registrar.categorize_setting("on", "first.group"), Ok(()));
        assert_eq!(
            registrar.categorize_setting("on", ""),
            Err(SettingError::EmptyLabel)
        );
        assert_eq!(
            registrar.categorize_setting("missing", "first.group"),
            Err(SettingError::UnknownSetting("missing".into()))
        );
        // A later call replaces the category instead of adding a second entry.
        assert_eq!(registrar.categorize_setting("on", "first.other"), Ok(()));
        assert_eq!(
            registrar.parts.setting_categories,
            [("first", "on".to_owned(), "first.other".to_owned())]
        );
        // Another mod cannot categorize the first mod's setting.
        registrar.mod_id = "second";
        assert_eq!(
            registrar.categorize_setting("on", "second.group"),
            Err(SettingError::UnknownSetting("on".into()))
        );
        assert_eq!(registrar.parts.setting_categories.len(), 1);
    }
}

impl SettingsRegistrar for PartRegistrar {
    fn register_setting(
        &mut self,
        setting: crate::SettingSpec,
    ) -> Result<crate::SettingKey, crate::SettingError> {
        setting.check()?;
        if self
            .parts
            .settings
            .iter()
            .any(|(owner, prior)| *owner == self.mod_id && prior.key == setting.key)
        {
            return Err(crate::SettingError::Duplicate(setting.key));
        }
        let slot = self.parts.settings.len();
        self.parts.settings.push((self.mod_id, setting));
        Ok(mod_vocab::__private::setting_key(self.mod_id, slot))
    }

    fn categorize_setting(&mut self, key: &str, category: &str) -> Result<(), crate::SettingError> {
        if category.is_empty() {
            return Err(crate::SettingError::EmptyLabel);
        }
        if !self
            .parts
            .settings
            .iter()
            .any(|(owner, spec)| *owner == self.mod_id && spec.key.as_str() == key)
        {
            return Err(crate::SettingError::UnknownSetting(key.to_owned()));
        }
        if let Some(entry) = self
            .parts
            .setting_categories
            .iter_mut()
            .find(|(owner, k, _)| *owner == self.mod_id && k.as_str() == key)
        {
            entry.2 = category.to_owned();
        } else {
            self.parts
                .setting_categories
                .push((self.mod_id, key.to_owned(), category.to_owned()));
        }
        Ok(())
    }

    fn register_config_screen(
        &mut self,
        factory: Arc<dyn crate::ConfigScreenFactory>,
    ) -> Result<(), String> {
        if self
            .parts
            .config_screens
            .iter()
            .any(|prior| prior.owner == self.mod_id)
        {
            return Err(format!(
                "mod {} registers a second config screen",
                self.mod_id
            ));
        }
        self.parts.config_screens.push(ConfigScreenRegistration {
            owner: self.mod_id,
            index: self.mod_index,
            factory,
            panicked: Default::default(),
        });
        Ok(())
    }
}
