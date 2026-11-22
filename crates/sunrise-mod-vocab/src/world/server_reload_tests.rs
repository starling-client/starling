use super::*;
fn id(value: &str) -> Identifier {
    Identifier::parse(value).unwrap()
}
fn files() -> FileToIdConverter {
    FileToIdConverter::json("machines").unwrap()
}

#[test]
fn registry_views_and_explicit_order_before_vanilla() {
    let mut listeners = ServerListenerRegistry::new();
    let [functions, mod_first, mod_second, mod_after] = [
        "minecraft:function_library",
        "test_mod:first",
        "test_mod:second",
        "test_mod:after",
    ]
    .map(id);
    assert!(listeners.get_registry().iter().eq([&functions]));
    assert!(listeners.get_graph().iter().next().is_none());

    listeners
        .add_listener(mod_first.clone(), files(), Arc::new(Boundary))
        .unwrap();
    listeners
        .add_listener(mod_second.clone(), files(), Arc::new(Boundary))
        .unwrap();
    listeners
        .add_listener(mod_after.clone(), files(), Arc::new(Boundary))
        .unwrap();
    assert_eq!(listeners.get_last_vanilla_listener(), &functions);
    assert!(
        listeners
            .get_registry()
            .iter()
            .eq([&functions, &mod_first, &mod_second, &mod_after,])
    );
    listeners
        .add_dependency(mod_second.clone(), VanillaServerListeners::functions())
        .unwrap();
    listeners
        .add_dependency(VanillaServerListeners::functions(), mod_after.clone())
        .unwrap();
    assert!(
        listeners
            .get_graph()
            .iter()
            .eq([(&mod_second, &functions), (&functions, &mod_after),])
    );
    let sorted = listeners.finish().unwrap().listeners;
    assert_eq!(
        sorted.into_iter().map(|entry| entry.id).collect::<Vec<_>>(),
        [mod_second, functions, mod_first, mod_after]
    );
}

#[test]
fn panicking_server_reload_registration_discards_only_its_ordered_and_retained_tails() {
    use crate::{AddServerReloadListenersEvent, Hooks, Host, ModEntry, ReloadContext};
    struct Context;
    impl Host for Context {}
    crate::impl_host_areas!(Context);
    impl ReloadContext for Context {}
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Stored(&'static str);
    impl ServerReloadListener for Stored {
        fn prepare(
            &self,
            _: &ServerReloadContext,
        ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
            Ok(Box::new(|| {}))
        }
    }
    struct Register(Option<Arc<AtomicBool>>);
    #[mod_api::__subscriptions]
    impl Hooks for Register {
        fn add_server_reload_listeners(
            &self,
            _: &dyn ReloadContext,
            event: &mut AddServerReloadListenersEvent<'_>,
        ) {
            let reg = &mut *event.listeners;
            let shared = ListenerKey::new(id("test_mod:shared"));
            if let Some(wrote_tail) = &self.0 {
                reg.add_retained_listener(&shared, files(), Arc::new(Stored("discarded")))
                    .unwrap();
                // This edge connects two earlier rows. Rolling back only the new node would leave a cycle.
                reg.add_dependency(id("test_mod:seeded"), VanillaServerListeners::functions())
                    .unwrap();
                wrote_tail.store(true, Ordering::Relaxed);
                panic!("server registrar failed after adding retained data and an edge");
            }
            reg.add_retained_listener(&shared, files(), Arc::new(Stored("healthy")))
                .unwrap();
            reg.add_dependency(id("test_mod:seeded"), shared.id().clone())
                .unwrap();
        }
    }

    let mut listeners = ServerListenerRegistry::new();
    let functions = VanillaServerListeners::functions();
    let seeded = ListenerKey::new(id("test_mod:seeded"));
    let shared = ListenerKey::<Stored>::new(id("test_mod:shared"));
    (&mut listeners as &mut dyn ServerReloadListeners)
        .add_retained_listener(&seeded, files(), Arc::new(Stored("seeded")))
        .unwrap();
    listeners
        .add_dependency(functions.clone(), seeded.id().clone())
        .unwrap();
    let wrote_tail = Arc::new(AtomicBool::new(false));
    crate::testing::with_dispatch(
        vec![
            ModEntry::new("failed", Box::new(Register(Some(wrote_tail.clone())))),
            ModEntry::new("healthy", Box::new(Register(None))),
        ]
        .into(),
        || {
            crate::dispatch_add_server_reload_listeners(&Context, |listen| {
                listeners.register_listener(listen);
            });
            assert!(wrote_tail.load(Ordering::Relaxed));
            assert_eq!(
                crate::__private::take_hook_panics(),
                [crate::__private::HookPanicReport {
                    mod_id: "failed",
                    hook: "add_server_reload_listeners",
                }]
            );
        },
    );
    let ordered = listeners.finish().unwrap();
    let expected = [&functions, seeded.id(), shared.id()];
    assert!(
        ordered
            .listeners
            .iter()
            .map(|listener| &listener.id)
            .filter(|id| expected.contains(id))
            .eq(expected)
    );
    assert_eq!(ordered.retained.len(), 2);
    let mut resources = RetainedListeners::default();
    resources.install(ordered.retained, &Default::default());
    let resources = &resources as &dyn ServerReloadResources;
    assert_eq!(resources.listener(&seeded).unwrap().0, "seeded");
    assert_eq!(resources.listener(&shared).unwrap().0, "healthy");
}

#[test]
fn duplicate_objects_cannot_make_the_reverse_lookup_ambiguous() {
    let mut listeners = ServerListenerRegistry::new();
    let listener = Arc::new(Boundary);
    listeners
        .add_listener(id("one:listener"), files(), listener.clone())
        .unwrap();
    assert_eq!(
        listeners.add_listener(id("two:listener"), files(), listener),
        Err(ReloadOrderError::DuplicateObject {
            id: id("two:listener"),
            registered_as: id("one:listener"),
        })
    );
}

#[test]
fn retained_listeners_read_back_typed_from_the_resources() {
    struct OtherListener;
    impl ServerReloadListener for OtherListener {
        fn prepare(
            &self,
            _: &ServerReloadContext,
        ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
            Ok(Box::new(|| {}))
        }
    }
    let mut listeners = ServerListenerRegistry::new();
    let original = Arc::new(Boundary);
    let key = ListenerKey::new(id("test_mod:kept"));
    (&mut listeners as &mut dyn ServerReloadListeners)
        .add_retained_listener(&key, files(), original.clone())
        .unwrap();
    assert_eq!(key.id(), &id("test_mod:kept"));
    let ordered = listeners.finish().unwrap();
    let mut resources = RetainedListeners::default();
    resources.install(ordered.retained, &Default::default());
    let kept = (&resources as &dyn ServerReloadResources)
        .listener(&key)
        .unwrap();
    assert!(Arc::ptr_eq(&original, &kept));
    assert!(
        (&resources as &dyn ServerReloadResources)
            .listener(&ListenerKey::<OtherListener>::new(id("test_mod:kept")))
            .is_none()
    );
    assert!(
        (&resources as &dyn ServerReloadResources)
            .listener(&ListenerKey::<Boundary>::new(id("test_mod:absent")))
            .is_none()
    );
}

#[test]
fn reload_replaces_the_retained_store_whole() {
    let mut listeners = ServerListenerRegistry::new();
    let first = ListenerKey::<Boundary>::new(id("test_mod:first"));
    (&mut listeners as &mut dyn ServerReloadListeners)
        .add_retained_listener(&first, files(), Arc::new(Boundary))
        .unwrap();
    let mut resources = RetainedListeners::default();
    resources.install(listeners.finish().unwrap().retained, &Default::default());
    assert!(
        (&resources as &dyn ServerReloadResources)
            .listener(&first)
            .is_some()
    );

    let later = ServerListenerRegistry::new().finish().unwrap();
    resources.install(later.retained, &Default::default());
    assert!(
        (&resources as &dyn ServerReloadResources)
            .listener(&first)
            .is_none()
    );
}

#[test]
fn retained_registration_follows_the_same_rules_as_plain_listeners() {
    let mut listeners = ServerListenerRegistry::new();
    let listener = Arc::new(Boundary);
    listeners
        .add_listener(id("one:listener"), files(), listener.clone())
        .unwrap();
    assert_eq!(
        (&mut listeners as &mut dyn ServerReloadListeners).add_retained_listener(
            &ListenerKey::new(id("two:listener")),
            files(),
            listener
        ),
        Err(ReloadOrderError::DuplicateObject {
            id: id("two:listener"),
            registered_as: id("one:listener"),
        })
    );
    let key = ListenerKey::<Boundary>::new(id("one:kept"));
    (&mut listeners as &mut dyn ServerReloadListeners)
        .add_retained_listener(&key, files(), Arc::new(Boundary))
        .unwrap();
    assert!(
        (&mut listeners as &mut dyn ServerReloadListeners)
            .add_retained_listener(
                &ListenerKey::<Boundary>::new(id("one:kept")),
                files(),
                Arc::new(Boundary)
            )
            .is_err()
    );
    let ordered = listeners.finish().unwrap();
    assert_eq!(
        ordered
            .listeners
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        [
            VanillaServerListeners::functions(),
            id("one:listener"),
            id("one:kept"),
        ]
    );
    assert_eq!(ordered.retained.len(), 1);
    drop(key);
}
