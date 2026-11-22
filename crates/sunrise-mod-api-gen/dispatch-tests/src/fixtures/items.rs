//! Item declarations use the same per-mod ownership boundary as blocks.
#[test]
fn item_dispatch_records_each_owner_with_its_submission() {
    struct Declares;
    #[mod_api::__subscriptions]
    impl Hooks for Declares {
        fn items(&self, _: &RegistrationContext, event: &mut ItemsEvent<'_>) {
            event
                .register(mod_api::mod_api::Item { id: "spoon".into() })
                .unwrap();
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Declares)),
            ModEntry::new("second", Box::new(Declares)),
        ]
        .into(),
    );
    DECLARATION_OWNERS.with_borrow_mut(Vec::clear);
    let mut items = Vec::new();
    dispatch_items(&RegistrationContext("bootstrap"), &mut |listen| {
        assert!(listen(&mut |item| {
            items.push(item);
            Ok(())
        }));
    });
    assert_eq!(
        items
            .iter()
            .map(|item| (item.mod_id, item.answer.id.as_str()))
            .collect::<Vec<_>>(),
        [
            ("first", "first/checked/spoon"),
            ("second", "second/checked/spoon")
        ]
    );
    DECLARATION_OWNERS.with_borrow(|calls| {
        assert_eq!(
            calls,
            &[
                ("first".into(), "spoon".into()),
                ("second".into(), "spoon".into())
            ]
        )
    });
}
