mod api {
    pub trait CreativeTabOutput {
        fn record(&mut self, marker: &'static str);
    }
}

impl mod_api::mod_api::CreativeTabOutput for Vec<&'static str> {
    fn record(&mut self, marker: &'static str) {
        self.push(marker);
    }
}

struct CreativeEditor {
    name: &'static str,
    panics: bool,
}

#[mod_api::__subscriptions]
impl Hooks for CreativeEditor {
    fn build_creative_mode_tab_contents(
        &self,
        _host: HostSide<'_>,
        event: &mut BuildCreativeModeTabContentsEvent<'_>,
    ) {
        event.output.record(self.name);
        assert!(!self.panics, "the editor panics after completing an edit");
    }
}

#[test]
fn a_panicking_creative_editor_leaves_completed_edits_and_only_loses_its_own_callback() {
    install(
        Vec::from(
            [("broken", true), ("healthy", false)].map(|(name, panics)| {
                ModEntry::new(name, Box::new(CreativeEditor { name, panics }))
            }),
        )
        .into(),
    );
    let mut output = Vec::new();
    for tab in ["test:first", "test:second"] {
        mod_api::dispatch_build_creative_mode_tab_contents(
            HostSide::Server(&mut TestHost),
            tab,
            &[],
            mod_api::mod_api::CreativeTabPermissions::Normal,
            &mut output,
        );
    }
    assert_eq!(output, ["broken", "healthy", "healthy"]);
}
