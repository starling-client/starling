/// Install the test mods for this thread. API unit tests run against fake hosts;
/// the exported fixture for the prepared game admits only its real subjects.
pub fn game_thread() -> crate::Mods {
    install_shared_parts(cfg!(test))
}

pub(crate) fn install_shared_parts(mock_subjects: bool) -> crate::Mods {
    let mods: crate::Mods = vec![
        ModEntry::new("doubler", Box::new(Doubler { mock_subjects })),
        ModEntry::new("chemist", Box::new(Chemist)),
        ModEntry::new("test", Box::new(FixtureContent)),
    ]
    .into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mod_vocab::__private::resolve_fluids(
        &[
            "minecraft:water",
            "minecraft:lava",
            "minecraft:flowing_water",
            "minecraft:flowing_lava",
            "minecraft:empty",
        ],
        &[],
    )
    .unwrap();
    mod_vocab::__private::resolve_block_keys(
        [
            "minecraft:air",
            "minecraft:granite",
            "test:gauge",
            "test:other",
            "test:connector",
            "test:press",
            "test:void_bin",
            "test:ordered",
            "minecraft:stone",
        ],
        |_| {
            sunrise_mod_runtime::block_definitions::BlockDefinition::new(&[&Default::default()], 0)
                .map_err(|e| format!("{e:?}"))
        },
    )
    .unwrap();
    mark_game_thread();
    mods
}
