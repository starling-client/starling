//! A client start publishes its models only once the world-rest worker it started is joined, so
//! the renderer table never reads the rest tables before they exist (#1377).
use crate::game::execution_role::ExecutionRole;
use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};

fn with_loading_client(run: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let mut gui = None;
    // As the loading screen loads: the rest tables on their own worker.
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        true,
        ExecutionRole::Client,
        &Progress::new(),
        &mut |ui, _| gui = Some(ui),
    )
    .unwrap();
    assert!(
        loaded.rest_loader.is_some(),
        "a client start loads the rest on a worker"
    );
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let saves = tempfile::tempdir().unwrap();
    options.game_dir = Some(saves.path().to_path_buf());
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        gui.or(loaded.gui).unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    game.rest_loader = loaded.rest_loader;
    assert!(game.options.is_client_start());
    run(&mut game);
}

#[test]
#[ignore = "daily"]
fn a_client_start_joins_the_world_rest_worker_before_it_resolves_the_renderers() {
    with_loading_client(|game| {
        game.publish_initial_client_models(vec![
            crate::game::client_reload::PreparedClientReload::Vanilla(()),
        ])
        .unwrap();
        assert!(game.rest_loader.is_none(), "the worker was joined");
        assert!(game.res.rest.get().is_some(), "the rest tables are there");
        assert!(
            game.res.mob_types().contains_key("minecraft:zombie"),
            "the renderer table resolved against the loaded mob types"
        );
    });
}

#[test]
#[ignore = "daily"]
fn title_ticks_join_world_resources_without_the_desktop_model_publication() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Reader(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Reader {
        fn client_tick_post(
            &self,
            host: &mut dyn mod_api::ClientHost,
            _: &mut mod_api::ClientTickPostEvent,
        ) {
            // This Host method reads items in
            // WorldRest, even when there is no open level.
            let stone = host
                .item_key(&mod_api::Identifier::parse("minecraft:stone").unwrap())
                .unwrap();
            assert_eq!(host.max_stack_size(stone), Some(64));
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let reads = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "title_reader",
        Box::new(Reader(reads.clone())),
    )]
    .into();
    with_loading_client(|game| {
        mod_platform::testing::with_mods(mods, || {
            // The scoped mod fixture has its own key table, independent of startup's.
            mod_api::__private::resolve_items(["minecraft:stone"]).unwrap();
            // Alternate window frontends need not have run publish_initial_client_models.
            game.title = Some(game.new_title(game.save.root.clone()));
            assert!(game.rest_loader.is_some());
            game.client_tick_steps(0, None).unwrap();
            assert!(game.rest_loader.is_some(), "an empty frame need not wait");
            game.client_tick_steps(1, None).unwrap();
            assert!(game.rest_loader.is_none(), "the tick joined the loader");
            assert!(
                game.res.rest.get().is_some(),
                "menu hooks have their tables"
            );
            assert_eq!(
                reads.load(Ordering::Relaxed),
                1,
                "the menu listener ran successfully, not skipped or quarantined"
            );
        });
    });
}
