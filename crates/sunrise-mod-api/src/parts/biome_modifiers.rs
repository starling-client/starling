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
        assert_eq!(declared[0].mod_id, "ruby");
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
        // Explicit valid namespaces, including `minecraft:`, succeed under the declaring mod.
        registrar
            .declare_biome_modifier("foreign:no_zombies", modifier())
            .unwrap();
        assert_eq!(registrar.biome_modifiers[1].mod_id, "ruby");
        assert_eq!(registrar.biome_modifiers[1].id, "foreign:no_zombies");
        registrar
            .declare_biome_modifier("minecraft:no_zombies", modifier())
            .unwrap();
        assert_eq!(registrar.biome_modifiers[2].mod_id, "ruby");
        assert_eq!(registrar.biome_modifiers[2].id, "minecraft:no_zombies");
        // Malformed ids refuse.
        assert_eq!(
            registrar.declare_biome_modifier("Bad Name", modifier()),
            Err(BiomeModifierError::InvalidId)
        );
        assert_eq!(
            registrar.declare_biome_modifier("foreign:Bad Name", modifier()),
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
        // The refusals name the mod; the good declarations stand.
        assert_eq!(registrar.biome_modifiers.len(), 3);
        assert_eq!(registrar.refused.len(), 4);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod ruby"))
        );
    }

    #[test]
    pub(crate) fn cross_mod_duplicate_biome_modifier_refuses() {
        struct First;
        #[mod_api::__subscriptions]
        impl Hooks for First {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_biome_modifier("shared:no_zombies", modifier())
                    .unwrap();
            }
        }
        struct Second;
        #[mod_api::__subscriptions]
        impl Hooks for Second {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                assert_eq!(
                    r.declare_biome_modifier("shared:no_zombies", modifier()),
                    Err(BiomeModifierError::Duplicate)
                );
            }
        }
        let errors = crate::parts::tests::install_parts_fixture(&[
            ModEntry::new("first", Box::new(First)),
            ModEntry::new("second", Box::new(Second)),
        ])
        .unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("mod second") && errors[0].contains("Duplicate"));
    }
}

impl BiomeModifiersRegistrar for PartRegistrar {
    fn declare_biome_modifier(
        &mut self,
        path: &str,
        modifier: crate::BiomeModifier,
    ) -> Result<(), crate::BiomeModifierError> {
        let result = (|| {
            let id = self
                .declaration_id(path)
                .map_err(|_| crate::BiomeModifierError::InvalidId)?;
            if self.biome_modifiers.iter().any(|d| d.id == id) {
                return Err(crate::BiomeModifierError::Duplicate);
            }
            modifier.check()?;
            self.biome_modifiers.push(BiomeModifierDeclaration {
                mod_id: self.mod_id.to_owned(),
                id,
                modifier,
            });
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
