use super::*;
thread_local! {
    static EXPECTED: std::cell::Cell<mod_api::Hand> = const { std::cell::Cell::new(mod_api::Hand::Main) };
    static SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
struct Part;
impl mod_api::ItemPart for Part {
    fn item_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[Some(mod_api::HookId::UseItem)]
    }
    fn use_item(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::UseItemEvent<'_>,
    ) -> mod_api::Flow {
        assert_eq!(event.hand, EXPECTED.get());
        SEEN.set(true);
        mod_api::Flow::Cancel
    }
}
pub(super) fn register(registrar: &mut dyn mod_api::Registrar) {
    let item = registrar.item_key("minecraft:stone");
    registrar.item_part(item, Part);
}
#[test]
fn generated_guest_item_dispatch_preserves_both_hands_and_claim() {
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    install_counter();
    let stack = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1).unwrap();
    for hand in [mod_api::Hand::Main, mod_api::Hand::Off] {
        EXPECTED.set(hand);
        SEEN.set(false);
        let mut claimed = false;
        mod_api::dispatch_use_item(
            &mut World,
            mod_api::__private::UseItemCall {
                stack: &stack,
                hand,
                claimed: &mut claimed,
            },
        );
        assert!(SEEN.get());
        assert!(claimed);
    }
}
