#![cfg(test)]
// Item model callbacks carry owned values and a separate runtime selection.
use crate::*;

struct NoClient;
impl Host for NoClient {}
crate::impl_host_areas!(NoClient);
impl ClientHost for NoClient {}
crate::impl_client_host_areas!(NoClient);

struct Selected;
#[crate::__subscriptions]
impl Hooks for Selected {
    fn item_model_property(&self, _: &dyn ClientHost, event: &mut ItemModelPropertyEvent<'_>) {
        event.value = event.stack.count() == 2 && event.bundle_selected == 3;
    }
    fn register_range_select_item_model_property(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterRangeSelectItemModelPropertyEvent<'_>,
    ) {
        event.register(Identifier::parse("example:selected").unwrap(), |_| {
            Ok(|_: &dyn ClientHost, input: &ItemModelPropertyInput<'_>| {
                input.bundle_selected as f32 / input.stack.count() as f32
            })
        });
    }
    fn register_select_item_model_property(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterSelectItemModelPropertyEvent<'_>,
    ) {
        event.register(Identifier::parse("example:selected").unwrap(), |_| {
            Ok((
                |when: &crate::Tag| match when {
                    crate::Tag::String(s) => Ok(s.clone()),
                    _ => Err("expected string".into()),
                },
                |_: &dyn ClientHost, input: &ItemModelPropertyInput<'_>| {
                    (input.bundle_selected >= 0).then(|| {
                        format!("{}:{}", input.bundle_selected, input.stack.count())
                            .as_str()
                            .into()
                    })
                },
            ))
        });
    }
    fn register_item_tint_sources(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterItemTintSourcesEvent<'_>,
    ) {
        event.register(Identifier::parse("example:selected").unwrap(), |_| {
            Ok(|_: &dyn ClientHost, input: &ItemTintInput<'_>| {
                if input.bundle_selected < 0 {
                    u32::MAX
                } else {
                    0xff00_0000 | ((input.bundle_selected as u32) << 16) | input.stack.count()
                }
            })
        });
    }
}

#[test]
fn model_callbacks_receive_owned_counts_and_the_exact_runtime_selection() {
    use crate::__private::game::RegistrationContextNew;
    install_parts(&[], &crate::__private::part_tests::rule_schema(), |_| false).unwrap();
    let stack = crate::__private::part_tests::stack("minecraft:stick", 2, 0);
    let mods = vec![ModEntry::new("selected", Box::new(Selected))].into();
    testing::with_mods(mods, || {
        let mut range_props = Vec::new();
        dispatch_register_range_select_item_model_property(
            &RegistrationContext::new(),
            &mut range_props,
        );
        let mut select_props = Vec::new();
        dispatch_register_select_item_model_property(
            &RegistrationContext::new(),
            &mut select_props,
        );
        let mut tint_sources = Vec::new();
        dispatch_register_item_tint_sources(&RegistrationContext::new(), &mut tint_sources);
        let config = crate::Tag::compound([] as [(&str, crate::Tag); 0]);
        let range_eval = (range_props[0].answer.decode)(&config).unwrap();
        let (select_case_decode, select_eval) = (select_props[0].answer.decode)(&config).unwrap();
        let tint_eval = (tint_sources[0].answer.decode)(&config).unwrap();
        let context = ItemModelContext::new(ItemDisplayContext::Gui, None, None, 0);
        let when = select_case_decode(&crate::Tag::String("3:2".into())).unwrap();
        for (selected, boolean, number, case, tint) in [
            (3, true, 1.5, Some(7), 0xff03_0002),
            (-1, false, -0.5, None, u32::MAX),
        ] {
            let mut value = false;
            dispatch_item_model_property(
                &NoClient,
                "example:selected",
                &stack,
                selected,
                &mut value,
            );
            assert_eq!(value, boolean);
            let prop_input = ItemModelPropertyInput::new(&stack, selected, &context);
            assert_eq!(range_eval(&NoClient, &prop_input), number);
            assert_eq!(
                select_eval(&NoClient, &prop_input, &mut std::iter::once((7, &when))),
                case
            );
            let tint_input = ItemTintInput::new(&stack, selected);
            assert_eq!(tint_eval(&NoClient, &tint_input), tint);
        }
    });
}

#[cfg(test)]
use super::*;

#[cfg(test)]
use crate::testing;
