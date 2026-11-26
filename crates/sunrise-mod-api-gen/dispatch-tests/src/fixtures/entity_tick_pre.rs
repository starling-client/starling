#[test]
fn entity_tick_pre_cancels_on_first_veto_in_mod_order() {
    struct Cancel(&'static str);
    #[mod_api::__subscriptions]
    impl Hooks for Cancel {
        fn entity_tick_pre(&self, _: HostSide<'_>, event: &mut EntityTickPreEvent<'_>) -> Flow {
            if event.kind == self.0 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    let mut canceled = false;
    install(Vec::<ModEntry>::new().into());
    dispatch_entity_tick_pre(
        HostSide::Server(&mut TestHost),
        "minecraft:overworld",
        1,
        "minecraft:zombie",
        &mut canceled,
    );
    assert!(!canceled);
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
        3,
        "minecraft:cow",
        &mut canceled,
    );
    assert!(!canceled);
    dispatch_entity_tick_pre(
        HostSide::Server(&mut TestHost),
        "minecraft:overworld",
        1,
        "minecraft:zombie",
        &mut canceled,
    );
    assert!(canceled);
}
