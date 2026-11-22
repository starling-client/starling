//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::*;

    #[cfg(test)]
    fn modifier() -> BiomeModifier {
        BiomeModifier {
            biomes: vec!["#minecraft:is_forest".to_string()],
            operation: BiomeModifierOperation::RemoveSpawns {
                entity_types: vec!["minecraft:zombie".to_string()],
            },
        }
    }

    /// A mod's registration reaches the table the game applies once at load.
    #[test]
    pub(crate) fn a_declared_modifier_is_installed_for_the_game_to_apply() {
        struct Quiet;
        #[mod_api::__subscriptions]
        impl Hooks for Quiet {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_biome_modifier("no_zombies", modifier()).unwrap();
            }
        }
        crate::parts::tests::install_parts_fixture(&[ModEntry::new("ruby", Box::new(Quiet))])
            .unwrap();
        let declared = crate::__private::biome_modifier_declarations();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].id, "ruby:no_zombies");
        assert_eq!(declared[0].modifier, modifier());
        assert_eq!(
            declared[0].file(),
            "data/ruby/neoforge/biome_modifier/no_zombies.json"
        );
    }

    #[test]
    fn nonfinite_climate_is_refused_directly_and_at_registration() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for temperature in [true, false] {
                let modifier = BiomeModifier {
                    biomes: vec!["minecraft:plains".into()],
                    operation: BiomeModifierOperation::ModifyBiome {
                        temperature: temperature.then_some(value),
                        downfall: (!temperature).then_some(value),
                        has_precipitation: None,
                        temperature_modifier: None,
                        sky_color: None,
                        fog_color: None,
                        water_color: None,
                        water_fog_color: None,
                        foliage_color: None,
                        grass_color: None,
                        dry_foliage_color: None,
                        ambient_particles: None,
                        ambient_loop: None,
                        ambient_additions: None,
                        ambient_mood: None,
                        background_music: None,
                        music_volume: None,
                    },
                };
                assert_eq!(modifier.check(), Err(BiomeModifierError::InvalidValue));
                let mut registrar = super::PartRegistrar::new("ruby");
                assert_eq!(
                    registrar.declare_biome_modifier("climate", modifier),
                    Err(BiomeModifierError::InvalidValue)
                );
            }
        }
    }

    #[test]
    pub(crate) fn biome_modifier_declarations_validate_and_refuse_duplicates() {
        let mut registrar = super::PartRegistrar::new("ruby");
        registrar
            .declare_biome_modifier("no_zombies", modifier())
            .unwrap();
        // The same id twice is a duplicate, spelled bare or with the namespace.
        assert_eq!(
            registrar.declare_biome_modifier("ruby:no_zombies", modifier()),
            Err(BiomeModifierError::Duplicate)
        );
        // The id stays in the declaring mod's namespace.
        assert_eq!(
            registrar.declare_biome_modifier("foreign:no_zombies", modifier()),
            Err(BiomeModifierError::InvalidId)
        );
        // The modifier itself is checked.
        let bad = BiomeModifier {
            biomes: Vec::new(),
            ..modifier()
        };
        assert_eq!(
            registrar.declare_biome_modifier("nothing", bad),
            Err(BiomeModifierError::EmptyList)
        );
        // The refusals name the mod; the good declaration stands.
        assert_eq!(registrar.biome_modifiers.len(), 1);
        assert_eq!(registrar.refused.len(), 3);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod ruby"))
        );
    }

    #[test]
    fn served_by_biome_modifier_delegates_to_serving_host_only() {
        let packages = vec![
            PackageInfo::new(
                "host".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["host".into()],
                vec![],
            ),
            PackageInfo::new(
                "other".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["other".into()],
                vec![],
            ),
            PackageInfo::new(
                "runes".into(),
                "1".into(),
                PackageKind::Content,
                Side::Both,
                vec!["runes".into()],
                vec![],
            )
            .with_served_by(Some("host".into())),
        ];
        crate::load_state::with_test_package_inventory(packages, || {
            let mut registrar = super::PartRegistrar::new("host");
            registrar
                .declare_biome_modifier("runes:no_zombies", modifier())
                .unwrap();
            assert_eq!(registrar.biome_modifiers[0].id, "runes:no_zombies");
            assert_eq!(
                registrar.biome_modifiers[0].file(),
                "data/runes/neoforge/biome_modifier/no_zombies.json"
            );
            assert_eq!(
                registrar.declare_biome_modifier("minecraft:no_zombies", modifier()),
                Err(BiomeModifierError::InvalidId)
            );
            registrar.mod_id = "other";
            assert_eq!(
                registrar.declare_biome_modifier("runes:other_mod", modifier()),
                Err(BiomeModifierError::InvalidId)
            );
            assert_eq!(registrar.biome_modifiers.len(), 1);
        });
    }
}

impl BiomeModifiersRegistrar for PartRegistrar {
    fn declare_biome_modifier(
        &mut self,
        path: &str,
        modifier: crate::BiomeModifier,
    ) -> Result<(), crate::BiomeModifierError> {
        let result = (|| {
            let (namespace, name) = path.split_once(':').unwrap_or((self.mod_id, path));
            // The id becomes a file path under the mod's data folder.
            let path_ok = name
                .chars()
                .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.' | '/'))
                && !name.contains("..")
                && !name.contains("//")
                && !name.starts_with('/')
                && !name.ends_with('/');
            if namespace.is_empty()
                || name.is_empty()
                || namespace == "minecraft"
                || !owns_namespace(self.mod_id, namespace)
                || !path_ok
            {
                return Err(crate::BiomeModifierError::InvalidId);
            }
            let id = format!("{namespace}:{name}");
            if self.biome_modifiers.iter().any(|d| d.id == id) {
                return Err(crate::BiomeModifierError::Duplicate);
            }
            modifier.check()?;
            self.biome_modifiers
                .push(BiomeModifierDeclaration { id, modifier });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} biome modifier {path:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}
