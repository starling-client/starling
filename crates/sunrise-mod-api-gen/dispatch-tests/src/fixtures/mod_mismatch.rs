mod api {
    pub struct ModMismatch;
    #[derive(Debug, PartialEq, Eq)]
    pub struct ModMismatchResolution {
        pub mod_id: &'static str,
        pub resolver: &'static str,
    }
}

#[test]
fn mod_mismatch_dispatch_sets_listener_mod_id_on_attributed_event() {
    struct Listener(&'static str);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn mod_mismatch(&self, _: HostSide<'_>, event: &mut ModMismatchEvent<'_>) {
            event
                .resolved
                .push(mod_api::mod_api::ModMismatchResolution {
                    mod_id: self.0,
                    resolver: event.mod_id,
                });
        }
    }

    install(
        vec![
            ModEntry::new("alpha", Box::new(Listener("gone"))),
            ModEntry::new("beta", Box::new(Listener("other"))),
        ]
        .into(),
    );
    let mut resolved = Vec::new();
    dispatch_mod_mismatch(HostSide::Server(&mut TestHost), &[], &mut resolved);
    assert_eq!(
        resolved,
        [
            mod_api::mod_api::ModMismatchResolution {
                mod_id: "gone",
                resolver: "alpha",
            },
            mod_api::mod_api::ModMismatchResolution {
                mod_id: "other",
                resolver: "beta",
            },
        ]
    );
}
