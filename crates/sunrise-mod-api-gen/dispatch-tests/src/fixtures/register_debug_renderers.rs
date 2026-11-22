mod stubs {
    #[derive(Debug, PartialEq)]
    pub(super) struct DebugRenderer(pub(super) &'static str);
}

#[test]
fn registered_debug_renderers_see_entry_states_and_keep_order() {
    struct States(&'static [&'static str]);
    impl mod_api::mod_api::__private::DebugEntryStates for States {
        fn is_enabled(&self, id: &str) -> bool {
            self.0.contains(&id)
        }
    }

    struct Registers {
        switch: Option<&'static str>,
        label: &'static str,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Registers {
        fn register_debug_renderers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterDebugRenderersEvent<'_>,
        ) {
            assert!(
                event.renderers.is_empty(),
                "each mod receives its own batch"
            );
            if self.switch.is_none_or(|id| event.entries.is_enabled(id)) {
                event.renderers.push(DebugRenderer(self.label));
            }
        }
    }

    install(
        vec![
            ModEntry::new(
                "first",
                Box::new(Registers {
                    switch: Some("debug:pipes"),
                    label: "pipes",
                }),
            ),
            ModEntry::new(
                "second",
                Box::new(Registers {
                    switch: Some("debug:off"),
                    label: "off",
                }),
            ),
            ModEntry::new(
                "third",
                Box::new(Registers {
                    switch: None,
                    label: "always",
                }),
            ),
        ]
        .into(),
    );
    let mut renderers = Vec::new();
    dispatch_register_debug_renderers(
        &RegistrationContext("refresh"),
        &States(&["debug:pipes"]),
        &mut renderers,
    );
    let item = |label, mod_id| Answered {
        answer: DebugRenderer(label),
        mod_id,
    };
    assert_eq!(renderers, [item("pipes", "first"), item("always", "third")]);
}
