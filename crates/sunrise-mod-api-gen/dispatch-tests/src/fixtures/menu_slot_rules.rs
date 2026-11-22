//! Pickup-specific dispatch stays out of the shared listener.
struct PickupPart;
impl MenuPart for PickupPart {
    fn menu_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn menu_slot_rules(&self, _: &dyn Host, event: &mut MenuSlotRulesEvent<'_>) {
        event.may_pickup = false;
    }
}
#[test]
fn pickup_refusal_is_written_back_by_dispatch() {
    install(Vec::<ModEntry>::new().into());
    install_parts(Parts {
        menus: vec![Box::leak(Box::new(PickupPart))],
        ..Parts::default()
    });
    let (mut place, mut pickup, mut maximum) = (true, true, 64);
    dispatch_menu_slot_rules(
        &TestHost,
        calls::MenuSlotRulesCall {
            menu: "test:menu",
            slot: 0,
            container: MenuContainer::Main,
            stack: &ItemStack::default(),
            context: &MenuSlotContext::new(None, None, &|_, _| None),
            may_place: &mut place,
            may_pickup: &mut pickup,
            max_stack: &mut maximum,
        },
    );
    assert_eq!((place, pickup, maximum), (true, false, 64));
}
