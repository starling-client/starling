#[test]
fn render_gui_post_dispatches_in_mod_order_and_preserves_prior_draws() {
    struct Overlay(u32);
    #[mod_api::__subscriptions]
    impl Hooks for Overlay {
        fn render_gui_post(&self, _: &dyn ClientHost, event: &mut RenderGuiPostEvent<'_>) {
            event.draw.mark(self.0);
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Overlay(1))),
            ModEntry::new("second", Box::new(Overlay(2))),
        ]
        .into(),
    );
    let mut batch = HudBatch(vec![99]);
    dispatch_render_gui_post(&TestHost, |listen| {
        assert!(listen(&mut batch));
    });
    assert_eq!(batch.0, [99, 1, 2]);
    install(Vec::<ModEntry>::new().into());
    dispatch_render_gui_post(&TestHost, |_| panic!("no listener"));
}
