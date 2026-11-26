//! Client tooltip component factories (`RegisterClientTooltipComponentFactoriesEvent`,
//! NeoForge's `ClientTooltipComponentManager`): maps custom [`mod_api::TooltipComponentData`] types
//! to [`PreparedTooltipComponent`]s once at client startup.

use mod_api::{
    ClientTooltipComponentFactoriesRegistrar, PreparedTooltipComponent, TooltipComponent,
    TooltipComponentFactory,
};
#[cfg(any(test, feature = "test-tooltip-factories"))]
use std::cell::RefCell;
use std::{any::TypeId, collections::HashMap, sync::OnceLock};

/// Client startup registrar collecting custom tooltip component factories before freeze.
#[derive(Default)]
pub struct TooltipComponentFactories {
    factories: HashMap<TypeId, TooltipComponentFactory>,
}

impl ClientTooltipComponentFactoriesRegistrar for TooltipComponentFactories {
    fn register_factory(&mut self, factory: TooltipComponentFactory) {
        self.factories.insert(factory.__type_id(), factory);
    }
}

impl TooltipComponentFactories {
    /// Publish a listener's factories only if its registration callback returns normally.
    pub fn register_listener(
        &mut self,
        listen: impl FnOnce(&mut dyn ClientTooltipComponentFactoriesRegistrar) -> bool,
    ) {
        let mut pending = Self::default();
        if listen(&mut pending) {
            let retired: Vec<_> = pending
                .factories
                .into_iter()
                .filter_map(|(type_id, factory)| self.factories.insert(type_id, factory))
                .collect();
            // Factory destructors run through their original callback owners after publication.
            drop(retired);
        }
    }
}

/// Frozen lookup table of client tooltip component factories published at client startup.
pub struct FrozenTooltipComponentFactories {
    factories: HashMap<TypeId, TooltipComponentFactory>,
}

static FACTORIES: OnceLock<FrozenTooltipComponentFactories> = OnceLock::new();

#[cfg(any(test, feature = "test-tooltip-factories"))]
thread_local! {
    static TEST_FACTORIES: RefCell<Option<FrozenTooltipComponentFactories>> = const { RefCell::new(None) };
}

/// Publish the client startup tooltip component factory table once, before any tooltip renders.
pub fn freeze(factories: TooltipComponentFactories) {
    if FACTORIES
        .set(FrozenTooltipComponentFactories {
            factories: factories.factories,
        })
        .is_err()
    {
        log::warn!("tooltip component factories were registered already; these are left out");
    }
}

/// Install scoped tooltip component factories on this thread for the duration of `f`, restoring
/// the previous value on return or unwind.
#[cfg(any(test, feature = "test-tooltip-factories"))]
pub fn with_test_factories<R>(factories: TooltipComponentFactories, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<FrozenTooltipComponentFactories>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let replaced = TEST_FACTORIES.with(|slot| slot.replace(self.0.take()));
            drop(replaced);
        }
    }
    let previous = TEST_FACTORIES.with(|slot| {
        slot.replace(Some(FrozenTooltipComponentFactories {
            factories: factories.factories,
        }))
    });
    let _guard = Restore(previous);
    f()
}

/// Why preparing a [`TooltipComponent`] failed before tooltip rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TooltipComponentPrepareError {
    /// No client tooltip component factory was registered for this custom data type
    /// (vanilla `ClientTooltipComponent.create` throws `IllegalArgumentException`).
    MissingFactory {
        type_name: &'static str,
        owner: Option<&'static str>,
    },
    /// A registered factory panicked or was already disabled for its owning mod.
    FactoryFailed {
        type_name: &'static str,
        owner: Option<&'static str>,
    },
    /// A bundle tooltip component carried a stack that is not a valid bundle.
    InvalidBundle { owner: Option<&'static str> },
}

impl std::fmt::Display for TooltipComponentPrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFactory { type_name, owner } => match owner {
                Some(id) => write!(
                    f,
                    "no client tooltip component factory registered for {type_name} (from mod {id})"
                ),
                None => write!(
                    f,
                    "no client tooltip component factory registered for {type_name}"
                ),
            },
            Self::FactoryFailed { type_name, owner } => match owner {
                Some(id) => write!(
                    f,
                    "tooltip component factory for {type_name} failed or is disabled (mod {id})"
                ),
                None => write!(
                    f,
                    "tooltip component factory for {type_name} failed or is disabled"
                ),
            },
            Self::InvalidBundle { owner } => match owner {
                Some(id) => write!(
                    f,
                    "bundle tooltip component stack is not a bundle (from mod {id})"
                ),
                None => write!(f, "bundle tooltip component stack is not a bundle"),
            },
        }
    }
}

impl std::error::Error for TooltipComponentPrepareError {}

/// Prepare a custom [`TooltipComponent`] using the registered client factory for its data type.
///
/// Built-in bundle components return `Err(InvalidBundle)` here (the tooltip pipeline prepares
/// them directly with `Items`). Unregistered custom types return `Err(MissingFactory)` instead of
/// panicking, matching vanilla `ClientTooltipComponent.create`'s `IllegalArgumentException`
/// semantics without crashing the client.
pub fn create(
    component: &TooltipComponent,
) -> Result<PreparedTooltipComponent, TooltipComponentPrepareError> {
    if component.as_bundle().is_some() {
        return Err(TooltipComponentPrepareError::InvalidBundle {
            owner: component.__owner_id(),
        });
    }
    let type_name = component.type_name().unwrap_or("<unknown>");
    let (Some(type_id), Some(data)) = (component.type_id(), component.__as_any()) else {
        return Err(TooltipComponentPrepareError::MissingFactory {
            type_name,
            owner: component.__owner_id(),
        });
    };
    let prepare = |factory: Option<&TooltipComponentFactory>| {
        let factory = factory.ok_or_else(|| TooltipComponentPrepareError::MissingFactory {
            type_name,
            owner: component.__owner_id(),
        })?;
        factory
            .__create(data)
            .ok_or_else(|| TooltipComponentPrepareError::FactoryFailed {
                type_name,
                owner: factory.__owner_id().or_else(|| component.__owner_id()),
            })
    };
    #[cfg(any(test, feature = "test-tooltip-factories"))]
    if let Some(found) = TEST_FACTORIES.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|frozen| frozen.factories.get(&type_id).cloned())
    }) {
        return prepare(found.as_ref());
    }
    prepare(
        FACTORIES
            .get()
            .and_then(|frozen| frozen.factories.get(&type_id)),
    )
}

/// Whether any custom tooltip component factory is registered in the active table.
pub fn has_factories() -> bool {
    #[cfg(any(test, feature = "test-tooltip-factories"))]
    if let Some(has) = TEST_FACTORIES.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|frozen| !frozen.factories.is_empty())
    }) {
        return has;
    }
    FACTORIES
        .get()
        .is_some_and(|frozen| !frozen.factories.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{
        __private::game::CallbackOwner, ClientTooltipComponent, Identifier, Text, TextFormError,
        TooltipComponentData, TooltipFont, content::crash::current_native_mod,
    };

    struct MetricData(i32);
    impl TooltipComponentData for MetricData {}

    struct UnregisteredData;
    impl TooltipComponentData for UnregisteredData {}

    struct MetricClient {
        width: i32,
        height: i32,
    }
    impl ClientTooltipComponent for MetricClient {
        fn height(&self, _font: &dyn TooltipFont) -> i32 {
            self.height
        }
        fn width(&self, _font: &dyn TooltipFont) -> i32 {
            self.width
        }
    }

    struct StubFont(Identifier);
    impl TooltipFont for StubFont {
        fn id(&self) -> &Identifier {
            &self.0
        }
        fn line_height(&self) -> i32 {
            9
        }
        fn width(&self, _text: &Text) -> Result<i32, TextFormError> {
            Ok(0)
        }
        fn split(&self, _text: &Text, _max_width: i32) -> Result<Vec<Text>, TextFormError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn factories_create_replace_error_unregistered_and_restore_on_unwind() {
        let font = StubFont(Identifier::parse("minecraft:default").unwrap());
        let data = TooltipComponent::custom(MetricData(12));
        let unknown = TooltipComponent::custom(UnregisteredData);

        with_test_factories(TooltipComponentFactories::default(), || {
            assert!(!has_factories());
            assert!(matches!(
                create(&data),
                Err(TooltipComponentPrepareError::MissingFactory { .. })
            ));

            let mut factories = TooltipComponentFactories::default();
            let registrar: &mut dyn ClientTooltipComponentFactoriesRegistrar = &mut factories;
            registrar.register::<MetricData, MetricClient>(|d| MetricClient {
                width: d.0,
                height: 10,
            });
            // Last registration wins, matching NeoForge HashMap::put.
            registrar.register::<MetricData, MetricClient>(|d| MetricClient {
                width: d.0 * 3,
                height: 20,
            });

            with_test_factories(factories, || {
                assert!(has_factories());
                let prepared = create(&data).expect("registered factory should prepare component");
                assert_eq!(prepared.width(&font), Some(36));
                assert_eq!(prepared.height(&font), Some(20));
                assert!(matches!(
                    create(&unknown),
                    Err(TooltipComponentPrepareError::MissingFactory { .. })
                ));
            });

            // Inner scope restored outer empty table.
            assert!(!has_factories());
            assert!(matches!(
                create(&data),
                Err(TooltipComponentPrepareError::MissingFactory { .. })
            ));

            // Unwind safety: a panic inside with_test_factories still restores the prior table.
            let mut inner = TooltipComponentFactories::default();
            let registrar: &mut dyn ClientTooltipComponentFactoriesRegistrar = &mut inner;
            registrar.register::<MetricData, MetricClient>(|d| MetricClient {
                width: d.0,
                height: 1,
            });
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_test_factories(inner, || {
                    assert!(has_factories());
                    panic!("expected unwind");
                });
            }));
            assert!(panicked.is_err());
            assert!(!has_factories());
        });
    }

    #[test]
    fn replacing_a_tooltip_factory_contains_the_displaced_capture_drop() {
        use std::sync::{Arc, Mutex};

        struct PanicOnDrop(Arc<Mutex<Vec<Option<&'static str>>>>);
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                self.0.lock().unwrap().push(current_native_mod());
                panic!("factory capture drop");
            }
        }

        mod_platform::__private::initialize().unwrap();
        let mut factories = TooltipComponentFactories::default();
        let registrar: &mut dyn ClientTooltipComponentFactoriesRegistrar = &mut factories;
        let dropped_under = Arc::new(Mutex::new(Vec::new()));
        let displaced_capture = PanicOnDrop(dropped_under.clone());
        let retained_capture = PanicOnDrop(dropped_under.clone());
        CallbackOwner::for_mod("displaced_tooltip_factory")
            .call("test", "register", || {
                registrar.register::<MetricData, MetricClient>(move |_| {
                    let _capture = &displaced_capture;
                    MetricClient {
                        width: 1,
                        height: 1,
                    }
                });
            })
            .unwrap();
        CallbackOwner::for_mod("replacement_tooltip_factory")
            .call("test", "register", || {
                registrar.register::<MetricData, MetricClient>(move |data| {
                    let _capture = &retained_capture;
                    MetricClient {
                        width: data.0,
                        height: 20,
                    }
                });
            })
            .expect("the replaced factory's destructor must not unwind through this registration");
        assert_eq!(
            *dropped_under.lock().unwrap(),
            [Some("displaced_tooltip_factory")]
        );
        with_test_factories(factories, || {
            let font = StubFont(Identifier::parse("minecraft:default").unwrap());
            let data = TooltipComponent::custom(MetricData(13));
            assert_eq!(create(&data).unwrap().width(&font), Some(13));
        });
        assert_eq!(
            *dropped_under.lock().unwrap(),
            [
                Some("displaced_tooltip_factory"),
                Some("replacement_tooltip_factory")
            ]
        );
    }

    #[test]
    fn panicking_factory_or_component_methods_disable_only_that_owner() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        struct PanicCreateData;
        impl TooltipComponentData for PanicCreateData {}
        struct PanicWidthData;
        impl TooltipComponentData for PanicWidthData {}
        struct PanicHeightData;
        impl TooltipComponentData for PanicHeightData {}
        struct PanicHandData;
        impl TooltipComponentData for PanicHandData {}
        struct HealthyData(i32);
        impl TooltipComponentData for HealthyData {}

        struct PanicWidthClient(Arc<AtomicUsize>);
        impl ClientTooltipComponent for PanicWidthClient {
            fn height(&self, _font: &dyn TooltipFont) -> i32 {
                10
            }
            fn width(&self, _font: &dyn TooltipFont) -> i32 {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("width boom");
            }
        }

        struct PanicHeightClient(Arc<AtomicUsize>);
        impl ClientTooltipComponent for PanicHeightClient {
            fn height(&self, _font: &dyn TooltipFont) -> i32 {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("height boom");
            }
            fn width(&self, _font: &dyn TooltipFont) -> i32 {
                10
            }
        }

        struct PanicHandClient(Arc<AtomicUsize>);
        impl ClientTooltipComponent for PanicHandClient {
            fn height(&self, _font: &dyn TooltipFont) -> i32 {
                10
            }
            fn width(&self, _font: &dyn TooltipFont) -> i32 {
                10
            }
            fn show_tooltip_with_item_in_hand(&self) -> bool {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("hand boom");
            }
        }

        let create_calls = Arc::new(AtomicUsize::new(0));
        let width_calls = Arc::new(AtomicUsize::new(0));
        let height_calls = Arc::new(AtomicUsize::new(0));
        let hand_calls = Arc::new(AtomicUsize::new(0));

        mod_platform::__private::initialize().unwrap();
        let mut factories = TooltipComponentFactories::default();
        let registrar: &mut dyn ClientTooltipComponentFactoriesRegistrar = &mut factories;
        let c_calls = create_calls.clone();
        CallbackOwner::for_mod("panic_create_mod")
            .call("test", "register", || {
                registrar.register::<PanicCreateData, MetricClient>(move |_| {
                    c_calls.fetch_add(1, Ordering::Relaxed);
                    panic!("create boom");
                });
            })
            .unwrap();
        let w_calls = width_calls.clone();
        CallbackOwner::for_mod("panic_width_mod")
            .call("test", "register", || {
                registrar.register::<PanicWidthData, PanicWidthClient>(move |_| {
                    PanicWidthClient(w_calls.clone())
                });
            })
            .unwrap();
        let h_calls = height_calls.clone();
        CallbackOwner::for_mod("panic_height_mod")
            .call("test", "register", || {
                registrar.register::<PanicHeightData, PanicHeightClient>(move |_| {
                    PanicHeightClient(h_calls.clone())
                });
            })
            .unwrap();
        let s_calls = hand_calls.clone();
        CallbackOwner::for_mod("panic_hand_mod")
            .call("test", "register", || {
                registrar.register::<PanicHandData, PanicHandClient>(move |_| {
                    PanicHandClient(s_calls.clone())
                });
            })
            .unwrap();
        CallbackOwner::for_mod("healthy_mod")
            .call("test", "register", || {
                registrar.register::<HealthyData, MetricClient>(|d| MetricClient {
                    width: d.0,
                    height: 14,
                });
            })
            .unwrap();

        let font = StubFont(Identifier::parse("minecraft:default").unwrap());
        with_test_factories(factories, || {
            let panic_create = TooltipComponent::custom(PanicCreateData);
            assert!(matches!(
                create(&panic_create),
                Err(TooltipComponentPrepareError::FactoryFailed {
                    owner: Some("panic_create_mod"),
                    ..
                })
            ));
            assert_eq!(create_calls.load(Ordering::Relaxed), 1);
            // Disabled after first panic: subsequent create skips invoking the factory.
            assert!(matches!(
                create(&panic_create),
                Err(TooltipComponentPrepareError::FactoryFailed {
                    owner: Some("panic_create_mod"),
                    ..
                })
            ));
            assert_eq!(create_calls.load(Ordering::Relaxed), 1);

            let panic_width = TooltipComponent::custom(PanicWidthData);
            let prepared = create(&panic_width).expect("factory succeeds before width panics");
            assert_eq!(prepared.width(&font), None);
            assert_eq!(width_calls.load(Ordering::Relaxed), 1);
            assert_eq!(prepared.width(&font), None);
            assert_eq!(width_calls.load(Ordering::Relaxed), 1);
            assert!(matches!(
                create(&panic_width),
                Err(TooltipComponentPrepareError::FactoryFailed {
                    owner: Some("panic_width_mod"),
                    ..
                })
            ));

            let panic_height = TooltipComponent::custom(PanicHeightData);
            let prepared = create(&panic_height).expect("factory succeeds before height panics");
            assert_eq!(prepared.height(&font), None);
            assert_eq!(height_calls.load(Ordering::Relaxed), 1);
            assert_eq!(prepared.height(&font), None);
            assert_eq!(height_calls.load(Ordering::Relaxed), 1);
            assert!(matches!(
                create(&panic_height),
                Err(TooltipComponentPrepareError::FactoryFailed {
                    owner: Some("panic_height_mod"),
                    ..
                })
            ));

            let panic_hand = TooltipComponent::custom(PanicHandData);
            let prepared = create(&panic_hand).expect("factory succeeds before hand check panics");
            assert!(!prepared.show_tooltip_with_item_in_hand());
            assert_eq!(hand_calls.load(Ordering::Relaxed), 1);
            assert!(!prepared.show_tooltip_with_item_in_hand());
            assert_eq!(hand_calls.load(Ordering::Relaxed), 1);
            assert!(matches!(
                create(&panic_hand),
                Err(TooltipComponentPrepareError::FactoryFailed {
                    owner: Some("panic_hand_mod"),
                    ..
                })
            ));

            // Sibling mod factory remains completely unaffected.
            let healthy = TooltipComponent::custom(HealthyData(48));
            let prepared = create(&healthy).expect("healthy sibling remains enabled");
            assert_eq!(prepared.width(&font), Some(48));
            assert_eq!(prepared.height(&font), Some(14));
        });
    }
}
