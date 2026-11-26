/// Install parts that decide what stops a sneaking player's slide down a climbable: a ladder is
/// scaffolding to every player, and scaffolding is not.
pub fn scaffolding_game_thread() {
    struct Ladder;
    #[mod_api::sunrise_part]
    impl BlockPart for Ladder {
        fn is_scaffolding(&self, _: &dyn LoopContext, event: &mut IsScaffoldingEvent) {
            event.scaffolding = true;
        }
    }
    struct Scaffolding;
    #[mod_api::sunrise_part]
    impl BlockPart for Scaffolding {
        fn is_scaffolding(&self, _: &dyn LoopContext, event: &mut IsScaffoldingEvent) {
            event.scaffolding = false;
        }
    }
    struct ScaffoldingMod;
    #[mod_api::__subscriptions]
    impl Hooks for ScaffoldingMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let ladder = r.block_key("minecraft:ladder");
            r.block_part(ladder, Ladder);
            let scaffolding = r.block_key("minecraft:scaffolding");
            r.block_part(scaffolding, Scaffolding);
        }
    }
    let mods: crate::Mods =
        vec![ModEntry::new("scaffolding_test", Box::new(ScaffoldingMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
