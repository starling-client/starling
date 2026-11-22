//! Datapack registry reads through a level handle.
use super::Game;
use mod_api::LevelRef;

impl Game<'_> {
    /// The datapack registry set of this level's world session: on the logical server, the
    /// server's own set for both the active dimension and any parked loaded dimension
    /// (matching vanilla `Level.registryAccess()` on `ServerLevel`); on a client, the set its
    /// server sent at the join for the active client level.
    fn level_datapack_set(
        &self,
        level: LevelRef,
    ) -> Option<std::sync::Arc<mod_api::DatapackRegistrySet>> {
        if self.is_logical_server() {
            self.native_level_world(level)?;
            self.mods.datapack_registries.clone()
        } else {
            if !self.level_handle_current(level) || level.dimension() != self.native_level_key() {
                return None;
            }
            sunrise_client::datapack_net::received()
        }
    }

    /// The entry ids of a mod's datapack registry as the world last loaded it.
    pub(super) fn level_datapack_entries(
        &self,
        level: LevelRef,
        registry: &str,
    ) -> Option<Vec<String>> {
        let entries = self
            .level_datapack_set(level)?
            .entries(registry)
            .ok()?
            .into_iter()
            .map(str::to_owned)
            .collect();
        Some(entries)
    }

    /// Visit the published entry `entry` of `registry`.
    pub(super) fn level_datapack_read(
        &self,
        level: LevelRef,
        registry: &str,
        entry: &str,
        visit: &mut dyn FnMut(&mod_api::DatapackEntry),
    ) -> Result<(), mod_api::DatapackReadError> {
        let set = self
            .level_datapack_set(level)
            .ok_or(mod_api::DatapackReadError::UnknownRegistry)?;
        if !set.registry_available(registry) {
            return Err(mod_api::DatapackReadError::UnknownRegistry);
        }
        visit(set.get(registry, entry)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use mod_api::{DatapackReadError as E, *};

    #[test]
    #[ignore = "daily"]
    fn level_registry_reads_check_world_session_and_generation() {
        struct RegistryMod;
        #[mod_api::__subscriptions]
        impl Hooks for RegistryMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_datapack_registry(
                    "spell",
                    DatapackRegistrySync::Required,
                    vec![DatapackField::required(
                        "power",
                        DatapackFieldKind::Int { min: 0, max: 100 },
                    )],
                )
                .unwrap();
            }
        }
        mod_platform::testing::with_mods(
            vec![ModEntry::new("example", Box::new(RegistryMod))].into(),
            || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    game.install_live_features();
                    let level = game.level(game.dim.id()).unwrap();
                    let candidate = |power: i64| {
                        DatapackCandidate::build(
                            mod_api::__private::datapack_registry_declarations(),
                            &[(
                                "data/addon/example/spell/fire.json".into(),
                                vec![(
                                    "pack".to_owned(),
                                    format!(r#"{{"power":{power}}}"#).into_bytes(),
                                )],
                            )],
                        )
                        .unwrap()
                    };
                    game.mods.datapack_registries = Some(std::sync::Arc::new(
                        DatapackRegistrySet::from_candidate(candidate(7)),
                    ));
                    assert_eq!(
                        game.level_datapack_entries(level, "example:spell"),
                        Some(vec!["addon:fire".into()])
                    );
                    let mut visited_refusal = false;
                    let mut unexpected_visit = |_: &DatapackEntry| visited_refusal = true;
                    for invalid in [
                        LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension()),
                        LevelRef::new(level.epoch(), DimensionKey::NETHER),
                    ] {
                        assert_eq!(game.level_datapack_entries(invalid, "example:spell"), None);
                        assert_eq!(
                            game.level_datapack_read(
                                invalid,
                                "example:spell",
                                "addon:fire",
                                &mut unexpected_visit,
                            ),
                            Err(E::UnknownRegistry)
                        );
                    }
                    let mut visits = 0;
                    game.level_datapack_read(level, "example:spell", "addon:fire", &mut |entry| {
                        assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(7))]);
                        visits += 1;
                    })
                    .unwrap();
                    assert_eq!(visits, 1);
                    // Open and park the Nether; parked loaded server dimensions share registryAccess.
                    game.enter_level(crate::game::Dim::Nether).unwrap();
                    game.enter_level(crate::game::Dim::Overworld).unwrap();
                    let parked = LevelRef::new(level.epoch(), DimensionKey::NETHER);
                    assert_eq!(
                        game.level_datapack_entries(parked, "example:spell"),
                        Some(vec!["addon:fire".into()])
                    );
                    let mut parked_visits = 0;
                    game.level_datapack_read(parked, "example:spell", "addon:fire", &mut |entry| {
                        assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(7))]);
                        parked_visits += 1;
                    })
                    .unwrap();
                    assert_eq!(parked_visits, 1);
                    assert_eq!(
                        game.level_datapack_read(
                            level,
                            "example:missing",
                            "addon:fire",
                            &mut unexpected_visit,
                        ),
                        Err(E::UnknownRegistry)
                    );
                    assert_eq!(
                        game.level_datapack_read(
                            level,
                            "example:spell",
                            "addon:missing",
                            &mut unexpected_visit,
                        ),
                        Err(E::MissingEntry)
                    );
                    game.mods.datapack_registries = Some(std::sync::Arc::new(
                        DatapackRegistrySet::from_candidate(candidate(9)),
                    ));
                    let mut reloaded_visits = 0;
                    game.level_datapack_read(parked, "example:spell", "addon:fire", &mut |entry| {
                        assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(9))]);
                        reloaded_visits += 1;
                    })
                    .unwrap();
                    assert_eq!(reloaded_visits, 1);
                    assert!(!visited_refusal);
                });
            },
        );
    }
}
