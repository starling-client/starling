pub(crate) struct FixtureContent;
#[mod_api::__subscriptions]
impl Hooks for FixtureContent {
    fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
        for id in ["test:hoard", "test:energy", "test:fluid"] {
            r.register_attachment_type(id, false).unwrap();
        }

        r.declare_fluid("potion");
        for id in CAPABILITY_TYPES {
            r.block_entity_type(id)
                .blocks(&[&format!("test:{id}")])
                .build();
        }
    }
    fn blocks(&self, _context: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
        for block in ["gauge", "connector", "press", "hoard", "void_bin"]
            .into_iter()
            .chain(CAPABILITY_TYPES.iter().copied())
            .map(|id| mod_api::Block::new(format!("test:{id}"), "minecraft:stone"))
        {
            event.register(block).unwrap();
        }
    }
}
