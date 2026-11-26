#[test]
fn a_drop_reaches_every_listener_whole_and_in_order() {
    use std::path::PathBuf;
    struct Listener(&'static str);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn screen_files_dropped(
            &self,
            _: &dyn ClientHost,
            event: &mut ScreenFilesDroppedEvent<'_>,
        ) {
            assert_eq!(
                event.paths,
                [PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.png")]
            );
            LOG.with(|log| log.borrow_mut().push(self.0));
        }
    }
    thread_local! {
        static LOG: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    let mods: Vec<ModEntry> = vec![
        ModEntry::new("first", Box::new(Listener("first"))),
        ModEntry::new("second", Box::new(Listener("second"))),
    ];
    install(mods.into());
    let paths = [PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.png")];
    dispatch_screen_files_dropped(&TestHost, &ScreenView(&()), &paths);
    assert_eq!(LOG.with(|log| log.take()), ["first", "second"]);
}
