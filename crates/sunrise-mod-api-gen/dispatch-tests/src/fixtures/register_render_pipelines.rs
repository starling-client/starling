mod api {
    #[derive(Debug, PartialEq, Eq)]
    pub struct RenderPipeline(pub &'static str);
    #[derive(Debug, PartialEq, Eq)]
    pub struct RenderSetup(pub &'static str);
    #[derive(Debug, PartialEq, Eq)]
    pub struct ComputePipeline(pub &'static str);
}

#[test]
fn register_render_pipelines_dispatch_attributes_and_discards_failed_listener_tail() {
    struct Listener(bool);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn register_render_pipelines(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterRenderPipelinesEvent<'_>,
        ) {
            event
                .pipelines
                .push(mod_api::mod_api::RenderPipeline(event.mod_id));
            event
                .render_types
                .push(mod_api::mod_api::RenderSetup(event.mod_id));
            event
                .compute_pipelines
                .push(mod_api::mod_api::ComputePipeline(event.mod_id));
            if self.0 {
                panic!("the fixture's pipeline listener unwinds");
            }
        }
    }

    install(
        vec![
            ModEntry::new("first", Box::new(Listener(false))),
            ModEntry::new("failed", Box::new(Listener(true))),
            ModEntry::new("second", Box::new(Listener(false))),
        ]
        .into(),
    );
    let mut pipelines = Vec::new();
    let mut render_types = Vec::new();
    let mut compute_pipelines = Vec::new();
    dispatch_register_render_pipelines(
        &RegistrationContext("bootstrap"),
        &mut render_types,
        &mut compute_pipelines,
        &mut pipelines,
    );
    assert_eq!(
        pipelines,
        [
            mod_api::mod_api::RenderPipeline("first"),
            mod_api::mod_api::RenderPipeline("second"),
        ]
    );
    assert_eq!(
        render_types,
        [
            mod_api::mod_api::RenderSetup("first"),
            mod_api::mod_api::RenderSetup("second"),
        ]
    );
    assert_eq!(
        compute_pipelines,
        [
            mod_api::mod_api::ComputePipeline("first"),
            mod_api::mod_api::ComputePipeline("second"),
        ]
    );
}
