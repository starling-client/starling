#[test]
fn entity_tick_pre_collects_each_mods_cancels_for_the_family_in_mod_order() {
    struct Cancel(&'static str);
    #[mod_api::__subscriptions]
    impl Hooks for Cancel {
        fn entity_tick_pre(&self, _: HostSide<'_>, event: &mut EntityTickPreEvent<'_>) {
            for (&entity, &kind) in event.entities.iter().zip(event.kinds) {
                if kind == self.0 {
                    event.canceled.push(entity);
                }
            }
        }
    }
    let entities = [1, 2, 3, 4];
    let kinds = [
        "minecraft:zombie",
        "minecraft:item",
        "minecraft:cow",
        "minecraft:item",
    ];
    let mut canceled = vec![9];
    install(Vec::<ModEntry>::new().into());
    dispatch_entity_tick_pre(
        HostSide::Server(&mut TestHost),
        "minecraft:overworld",
        &entities,
        &kinds,
        &mut canceled,
    );
    assert_eq!(canceled, [9]);
    install(
        vec![
            ModEntry::new("items", Box::new(Cancel("minecraft:item"))),
            ModEntry::new("zombies", Box::new(Cancel("minecraft:zombie"))),
        ]
        .into(),
    );
    dispatch_entity_tick_pre(
        HostSide::Server(&mut TestHost),
        "minecraft:overworld",
        &entities,
        &kinds,
        &mut canceled,
    );
    assert_eq!(canceled, [9, 2, 4, 1]);
}
