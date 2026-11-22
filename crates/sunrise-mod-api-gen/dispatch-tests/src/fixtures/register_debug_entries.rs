mod stubs {
    #[derive(Debug, PartialEq)]
    pub(super) struct DebugEntry(pub(super) &'static str);
}

#[test]
fn registered_debug_entries_retain_their_owner_in_registration_order() {
    struct Registers(&'static [&'static str]);
    #[mod_api::__subscriptions]
    impl Hooks for Registers {
        fn register_debug_entries(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterDebugEntriesEvent<'_>,
        ) {
            assert!(event.entries.is_empty(), "each mod receives its own batch");
            event
                .entries
                .extend(self.0.iter().map(|&id| DebugEntry(id)));
        }
    }

    install(
        vec![
            ModEntry::new("first", Box::new(Registers(&["debug:cost", "debug:queue"]))),
            ModEntry::new("second", Box::new(Registers(&["debug:pipes"]))),
        ]
        .into(),
    );
    let mut entries = Vec::new();
    dispatch_register_debug_entries(&RegistrationContext("bootstrap"), &mut entries);
    let entry = |id, mod_id| Answered {
        answer: DebugEntry(id),
        mod_id,
    };
    assert_eq!(
        entries,
        [
            entry("debug:cost", "first"),
            entry("debug:queue", "first"),
            entry("debug:pipes", "second"),
        ]
    );
}
