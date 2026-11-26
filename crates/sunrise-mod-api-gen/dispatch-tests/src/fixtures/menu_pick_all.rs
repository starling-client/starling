//! `menu_pick_all`: a Part's refusal stands against the Parts after it.
/// Keeps every slot out of the gather.
struct KeepsOut;
impl MenuPart for KeepsOut {
    fn menu_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn menu_pick_all(&self, _: &dyn Host, event: &mut MenuPickAllEvent<'_>) {
        event.allowed = false;
    }
}
/// Lets every slot into the gather, whatever it was told.
struct LetsIn;
impl MenuPart for LetsIn {
    fn menu_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn menu_pick_all(&self, _: &dyn Host, event: &mut MenuPickAllEvent<'_>) {
        event.allowed = true;
    }
}
/// What `parts` answer, in that order, for a slot the gather would take from.
fn gathers(parts: Vec<&'static dyn MenuPart>) -> bool {
    install(Vec::<ModEntry>::new().into());
    install_parts(Parts {
        menus: parts,
        ..Parts::default()
    });
    let mut allowed = true;
    dispatch_menu_pick_all(
        &TestHost,
        calls::MenuPickAllCall {
            menu: "test:menu",
            slot: 3,
            container: MenuContainer::Main,
            carried: &ItemStack::default(),
            stack: &ItemStack::default(),
            context: &MenuSlotContext::new(None, None, &|_, _| None),
            allowed: &mut allowed,
        },
    );
    allowed
}
#[test]
fn a_pick_all_refusal_stands_against_a_later_part() {
    let keeps_out: &'static dyn MenuPart = Box::leak(Box::new(KeepsOut));
    let lets_in: &'static dyn MenuPart = Box::leak(Box::new(LetsIn));
    assert!(!gathers(vec![keeps_out, lets_in]));
    assert!(!gathers(vec![lets_in, keeps_out]));
    assert!(gathers(vec![lets_in]));
}
