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

/// Refuses placement and pickup.
struct RefusingPart;
impl MenuPart for RefusingPart {
    fn menu_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn menu_slot_rules(&self, _: &dyn Host, event: &mut MenuSlotRulesEvent<'_>) {
        event.may_place = false;
        event.may_pickup = false;
    }
}
/// Allows placement and pickup, whatever it was told.
struct AllowingPart;
impl MenuPart for AllowingPart {
    fn menu_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn menu_slot_rules(&self, _: &dyn Host, event: &mut MenuSlotRulesEvent<'_>) {
        event.may_place = true;
        event.may_pickup = true;
    }
}
/// The rules `parts` answer, in that order, from the slot's own `(may_place, may_pickup)`.
fn answered(parts: Vec<&'static dyn MenuPart>, own: (bool, bool)) -> (bool, bool) {
    install(Vec::<ModEntry>::new().into());
    install_parts(Parts {
        menus: parts,
        ..Parts::default()
    });
    let ((mut place, mut pickup), mut maximum) = (own, 64);
    dispatch_menu_slot_rules(
        &TestHost,
        calls::MenuSlotRulesCall {
            menu: "minecraft:inventory",
            slot: 6,
            container: MenuContainer::Main,
            stack: &ItemStack::default(),
            context: &MenuSlotContext::new(None, None, &|_, _| None),
            may_place: &mut place,
            may_pickup: &mut pickup,
            max_stack: &mut maximum,
        },
    );
    (place, pickup)
}
#[test]
fn a_part_refusal_stands_against_a_later_part() {
    let refusing: &'static dyn MenuPart = Box::leak(Box::new(RefusingPart));
    let allowing: &'static dyn MenuPart = Box::leak(Box::new(AllowingPart));
    assert_eq!(
        answered(vec![refusing, allowing], (true, true)),
        (false, false)
    );
    assert_eq!(
        answered(vec![allowing, refusing], (true, true)),
        (false, false)
    );
    // A Part may still allow what the slot's own rule refuses, when no Part refuses.
    assert_eq!(answered(vec![allowing], (false, true)), (true, true));
}
