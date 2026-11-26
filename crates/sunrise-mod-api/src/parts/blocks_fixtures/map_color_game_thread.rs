/// Install parts that answer the map colour of blocks: stone shows as colour 18, and cobblestone
/// answers 200, which is no map colour.
pub fn map_color_game_thread() {
    struct Answer(u8);
    #[mod_api::sunrise_part]
    impl BlockPart for Answer {
        fn map_color(&self, _: &dyn Host, event: &mut MapColorEvent) {
            event.color = self.0;
        }
    }
    struct MapColorMod;
    #[mod_api::__subscriptions]
    impl Hooks for MapColorMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, Answer(18));
            let cobblestone = r.block_key("minecraft:cobblestone");
            r.block_part(cobblestone, Answer(200));
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("map_color_test", Box::new(MapColorMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
