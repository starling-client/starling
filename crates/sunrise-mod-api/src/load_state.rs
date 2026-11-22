//! Platform-owned startup registration state and load snapshots.
use crate::__private::game::Fields;
#[cfg(test)]
use crate::__private::game::{RegistrationContextNew, ServerReloadListenersOrder};
use crate::*;
use mod_vocab::__private::RegistrationData;
#[cfg(test)]
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

thread_local! {
    static REGISTRATION_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether startup registration is currently open on this thread.
pub(crate) fn is_registration_open() -> bool {
    REGISTRATION_OPEN.with(std::cell::Cell::get)
}

/// Thread-local guard marking startup registration open until dropped, including on unwind.
pub struct RegistrationGuard {
    previous: bool,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl RegistrationGuard {
    /// Opens startup registration on the current thread until the returned guard is dropped.
    pub fn open() -> Self {
        let _ = crate::__private::initialize();
        let previous = REGISTRATION_OPEN.replace(true);
        Self {
            previous,
            _thread: std::marker::PhantomData,
        }
    }
}

impl Drop for RegistrationGuard {
    fn drop(&mut self) {
        REGISTRATION_OPEN.set(self.previous);
    }
}

struct PackageInventory {
    entries: Arc<[PackageInfo]>,
    authority: Arc<mod_vocab::__private::NamespaceAuthority>,
}

impl PackageInventory {
    fn from_vec(packages: Vec<PackageInfo>) -> Self {
        let entries: Arc<[PackageInfo]> = packages.into();
        let authority = Arc::new(mod_vocab::__private::NamespaceAuthority::from_packages(
            &entries,
        ));
        Self { entries, authority }
    }
}

static PACKAGES: OnceLock<PackageInventory> = OnceLock::new();
static DEPENDENCIES: OnceLock<Arc<mod_vocab::__private::EffectiveDependencies>> = OnceLock::new();

/// Install the host's dependency graph projection before any registration.
pub fn install_dependencies(effective: mod_vocab::__private::EffectiveDependencies) {
    assert!(
        DEPENDENCIES.set(Arc::new(effective)).is_ok(),
        "package dependencies already installed"
    );
}

pub(crate) fn effective_dependencies() -> Arc<mod_vocab::__private::EffectiveDependencies> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(effective) = TEST_DEPENDENCIES.with_borrow(Clone::clone) {
        return effective;
    }
    DEPENDENCIES.get().cloned().unwrap_or_default()
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_DEPENDENCIES: std::cell::RefCell<Option<Arc<mod_vocab::__private::EffectiveDependencies>>> = const { std::cell::RefCell::new(None) };
}

/// Run a fixture with explicitly resolved dependency rows.
#[cfg(any(test, feature = "testing"))]
pub fn with_test_dependencies<R>(
    effective: mod_vocab::__private::EffectiveDependencies,
    f: impl FnOnce() -> R,
) -> R {
    let previous = TEST_DEPENDENCIES.replace(Some(Arc::new(effective)));
    let result = f();
    TEST_DEPENDENCIES.set(previous);
    result
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_PACKAGES: std::cell::RefCell<Option<Arc<PackageInventory>>> = const { std::cell::RefCell::new(None) };
}
/// Runs `f` with only the packages `ids` installed, for tests.
#[cfg(any(test, feature = "testing"))]
pub fn with_test_packages<R>(ids: &[&str], f: impl FnOnce() -> R) -> R {
    let packages: Vec<_> = ids
        .iter()
        .map(|id| {
            PackageInfo::new(
                (*id).to_owned(),
                "1".to_owned(),
                PackageKind::Code,
                crate::Side::Client,
                vec![(*id).to_owned()],
                vec![],
            )
        })
        .collect();
    with_test_package_inventory(packages, f)
}

/// Runs `f` with exactly these packages installed, dependencies included, for tests.
#[cfg(any(test, feature = "testing"))]
pub fn with_test_package_inventory<R>(packages: Vec<PackageInfo>, f: impl FnOnce() -> R) -> R {
    // Default fixtures model an unmodified installation of these declarations.
    let mut effective = mod_vocab::__private::EffectiveDependencies::default();
    for package in &packages {
        for dependency in package.dependencies() {
            match dependency.kind() {
                DependencyKind::Optional => effective.insert(package.id(), dependency.id(), true),
                DependencyKind::Required => effective.insert(package.id(), dependency.id(), false),
                _ => {}
            }
        }
    }
    let previous = TEST_PACKAGES.replace(Some(Arc::new(PackageInventory::from_vec(packages))));
    let _ = crate::__private::initialize();
    let result = with_test_dependencies(effective, f);
    TEST_PACKAGES.set(previous);
    result
}

fn inventory(native: &[PackageInfo], content: &[PackageInfo]) -> Vec<PackageInfo> {
    let mut packages = native.to_vec();
    for selected in content {
        if !packages.iter().any(|p| p.id() == selected.id()) {
            packages.push(selected.clone());
        }
    }
    // The code inventory follows the one installed native dispatch slice;
    // content-only packages remain after it in their selected layer order.
    packages.sort_by_key(|p| {
        crate::__private::mods()
            .iter()
            .position(|entry| entry.fields().id == p.id())
            .unwrap_or(usize::MAX)
    });
    packages
}

/// Records the package inventory once at startup; a second call panics.
pub fn install_packages(native: &[PackageInfo], content: &[PackageInfo]) {
    let _ = crate::__private::initialize();
    assert!(
        PACKAGES
            .set(PackageInventory::from_vec(inventory(native, content)))
            .is_ok(),
        "package inventory already installed"
    );
}

static REGISTRATION_DATA: OnceLock<RegistrationData> = OnceLock::new();

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_REGISTRATION_DATA: std::cell::RefCell<Option<RegistrationData>> = const { std::cell::RefCell::new(None) };
}
/// Runs `f` with `data` as the registration data, for tests.
#[cfg(any(test, feature = "testing"))]
pub fn with_test_registration_data<R>(data: RegistrationData, f: impl FnOnce() -> R) -> R {
    let _ = crate::__private::initialize();
    TEST_REGISTRATION_DATA.set(Some(data));
    let result = f();
    TEST_REGISTRATION_DATA.set(None);
    result
}

pub(crate) fn registration_data() -> Option<RegistrationData> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(data) = TEST_REGISTRATION_DATA.with_borrow(Clone::clone) {
        return Some(data);
    }
    REGISTRATION_DATA.get().cloned()
}

pub(crate) fn visible_packages() -> Option<crate::LoadedMods> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(packages) = TEST_REGISTRATION_DATA.with_borrow(|slot| {
        slot.as_ref()
            .and_then(RegistrationData::visible_mods)
            .cloned()
    }) {
        return Some(packages);
    }
    REGISTRATION_DATA
        .get()
        .and_then(RegistrationData::visible_mods)
        .cloned()
}

/// Records the registries and settings registration reads, once at startup, after the
/// package inventory and before the first registration hook; a second call panics.
pub fn install_registration_data(data: RegistrationData) {
    let _ = crate::__private::initialize();
    assert!(
        REGISTRATION_DATA.set(data).is_ok(),
        "registration data already installed"
    );
}

pub(crate) fn current_packages() -> Arc<[PackageInfo]> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(packages) =
        TEST_PACKAGES.with_borrow(|slot| slot.as_ref().map(|p| p.entries.clone()))
    {
        return packages;
    }
    PACKAGES
        .get()
        .map(|p| p.entries.clone())
        .unwrap_or_else(|| Arc::from([]))
}

pub(crate) fn current_namespace_authority() -> Arc<mod_vocab::__private::NamespaceAuthority> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(authority) =
        TEST_PACKAGES.with_borrow(|slot| slot.as_ref().map(|p| p.authority.clone()))
    {
        return authority;
    }
    PACKAGES
        .get()
        .map(|p| p.authority.clone())
        .unwrap_or_default()
}

pub(crate) fn owns_namespace(owner: &str, namespace: &str) -> bool {
    #[cfg(any(test, feature = "testing"))]
    if let Some(owns) =
        TEST_PACKAGES.with_borrow(|slot| slot.as_ref().map(|p| p.authority.owns(owner, namespace)))
    {
        return owns;
    }
    PACKAGES
        .get()
        .map_or(owner == namespace, |p| p.authority.owns(owner, namespace))
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestReader {
    pub registries: std::collections::BTreeMap<String, Arc<[String]>>,
    pub tags: Option<Arc<[(String, Arc<[String]>)]>>,
}
#[cfg(test)]
impl LoadReader for TestReader {
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.registries.get(name).map(AsRef::as_ref)
    }
    fn fluid_tags(&self) -> Option<&[(String, Arc<[String]>)]> {
        self.tags.as_deref()
    }
    fn setting(&self, _: &str, _: &str) -> Option<&str> {
        None
    }
}
#[cfg(test)]
impl LoopReader for TestReader {
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.registries.get(name).map(AsRef::as_ref)
    }
    fn setting(&self, _: &str, _: &str) -> Option<&str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_dispatch_slice_keeps_order_in_registration_config_and_reload() {
        crate::__private::initialize().unwrap();
        use crate::*;
        use std::sync::Mutex;
        struct Trace {
            id: &'static str,
            calls: Arc<Mutex<Vec<(&'static str, &'static str)>>>,
        }
        #[crate::__subscriptions]
        impl Hooks for Trace {
            fn register(&self, _: &mut dyn Registrar, _: &RegistrationContext) {
                assert!(crate::registration_open());
                self.calls.lock().unwrap().push(("register", self.id));
            }
            fn config_loading(&self, _: &dyn ConfigContext, _: &mut ConfigLoadingEvent<'_>) {
                assert!(!crate::registration_open());
                self.calls.lock().unwrap().push(("config", self.id));
            }
            fn add_server_reload_listeners(
                &self,
                _: &dyn ReloadContext,
                _: &mut AddServerReloadListenersEvent<'_>,
            ) {
                assert!(!crate::registration_open());
                self.calls.lock().unwrap().push(("reload", self.id));
            }
        }
        struct Server;
        impl Host for Server {}
        mod_api::impl_host_areas!(Server);
        impl ConfigContext for Server {}
        impl ReloadContext for Server {}
        // The loader supplies a dependency-resolved slice; dispatch must retain it.
        let order = [1, 0, 2];
        let calls = Arc::new(Mutex::new(Vec::new()));
        let ids = ["z", "lib", "a"];
        let mods: Mods = order
            .into_iter()
            .map(|i| {
                ModEntry::new(
                    ids[i],
                    Box::new(Trace {
                        id: ids[i],
                        calls: calls.clone(),
                    }),
                )
            })
            .collect::<Vec<_>>()
            .into();
        crate::__private::install(mods.clone());
        assert!(!crate::registration_open());
        crate::__private::install_parts(
            &mods,
            &crate::__private::part_tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        assert!(!crate::registration_open());
        let host = Server;
        for _world in 0..2 {
            dispatch_config_loading(
                &host,
                &crate::__private::built::ConfigFile {
                    mod_id: "lib".into(),
                    kind: ConfigKind::Server,
                    file: "lib-server.toml".into(),
                    text: String::new(),
                }
                .into(),
            );
            for _reload in 0..2 {
                let mut listeners = ServerListenerRegistry::new();
                dispatch_add_server_reload_listeners(&host, |listen| {
                    listeners.register_listener(listen)
                });
            }
        }
        let expected = [
            "register", "config", "reload", "reload", "config", "reload", "reload",
        ]
        .into_iter()
        .flat_map(|moment| ["lib", "z", "a"].map(|id| (moment, id)))
        .collect::<Vec<_>>();
        assert_eq!(*calls.lock().unwrap(), expected);
    }

    #[test]
    fn admitted_owners_protect_primary_ids_and_keep_first_alias_claims() {
        crate::__private::initialize().unwrap();
        use mod_vocab::__private::{
            content_owner_disabled, install_content_owners, set_mod_enabled,
        };

        struct Quiet;
        #[crate::__subscriptions]
        impl crate::Hooks for Quiet {}

        let packages = [
            ("owner", vec!["owner", "shared"]),
            ("claimant", vec!["claimant", "shared", "owner"]),
        ]
        .into_iter()
        .map(|(id, namespaces)| {
            PackageInfo::new(
                id.into(),
                "1".into(),
                PackageKind::Code,
                crate::Side::Both,
                namespaces.into_iter().map(str::to_owned).collect(),
                vec![],
            )
        })
        .collect();
        with_test_package_inventory(packages, || {
            crate::testing::with_dispatch(
                vec![
                    crate::ModEntry::new("owner", Box::new(Quiet)),
                    crate::ModEntry::new("claimant", Box::new(Quiet)),
                ]
                .into(),
                || {
                    assert!(set_mod_enabled("owner", false));
                    assert!(content_owner_disabled("owner:machine"));
                    assert!(!content_owner_disabled("shared:machine")); // raw metadata is not admission

                    install_content_owners([
                        ("owner", "claimant"),
                        ("shared", "owner"),
                        ("shared", "claimant"),
                        ("assets_only", "content_only"),
                        ("assets_only", "owner"),
                    ]);
                    assert!(content_owner_disabled("owner:machine"));
                    assert!(content_owner_disabled("shared:machine"));
                    assert!(!content_owner_disabled("assets_only:machine"));
                },
            );
        });
    }

    #[test]
    fn inventory_keeps_first_content_package_after_code_mods() {
        crate::__private::initialize().unwrap();
        let native = [PackageInfo::new(
            "runtime".into(),
            "3".into(),
            PackageKind::Code,
            crate::Side::Client,
            vec!["runtime".into()],
            vec![],
        )];
        let package = |version: &str| {
            PackageInfo::new(
                "plain".into(),
                version.into(),
                PackageKind::Content,
                crate::Side::Client,
                vec!["plain".into()],
                vec![],
            )
        };
        let entries = inventory(&native, &[package("1"), package("2")]);
        TEST_PACKAGES.set(Some(Arc::new(PackageInventory::from_vec(entries))));
        let view = RegistrationContext::new().mods().clone();
        TEST_PACKAGES.set(None);
        assert_eq!(view.ids().collect::<Vec<_>>(), ["runtime", "plain"]);
        let runtime = view.get("runtime").unwrap();
        assert_eq!(runtime.kind(), PackageKind::Code);
        assert_eq!(runtime.version(), "3");
        assert_eq!(runtime.side(), crate::Side::Client);
        let plain = view.get("plain").unwrap();
        assert_eq!(plain.kind(), PackageKind::Content);
        assert_eq!(plain.version(), "1");
        assert_eq!(plain.side(), crate::Side::Client);
        assert_eq!(plain.namespaces().collect::<Vec<_>>(), ["plain"]);
        assert!(!plain.depends_on("runtime"));
        assert!(view.get("missing").is_none());
    }

    #[test]
    fn a_loop_context_reads_its_side_registries_and_mods() {
        crate::__private::initialize().unwrap();
        let source = TestReader {
            registries: std::collections::BTreeMap::from([(
                "item".to_owned(),
                vec!["a:b".into()].into(),
            )]),
            tags: None,
        };
        with_test_packages(&["one", "two"], || {
            let context = OwningLoop::new(LoopSide::Client, &source, None);
            assert_eq!(context.side(), LoopSide::Client);
            assert_eq!(context.registry("item"), Some(&["a:b".to_owned()][..]));
            assert_eq!(context.registry("block"), None);
            assert_eq!(context.setting("one", "mode"), None);
            assert_eq!(context.mods().ids().collect::<Vec<_>>(), ["one", "two"]);
        });
    }

    #[test]
    fn registration_hooks_read_one_snapshot_directly() {
        crate::__private::initialize().unwrap();
        use crate::*;
        use std::{
            path::{Path, PathBuf},
            sync::Mutex,
        };

        /// Every answer the context gives this hook, for the keys the test names.
        fn probe(cx: &RegistrationContext, expected_dir: &Path) -> Vec<String> {
            let directory = cx.instance_dir().and_then(InstanceDirectory::to_path_buf);
            let registry = |name| format!("{:?}", cx.registry(name));
            let setting = |id, key| format!("{:?}", cx.setting(id, key));
            vec![
                format!(
                    "{:?}",
                    directory.map(|dir| dir.as_os_str() == expected_dir.as_os_str())
                ),
                registry("minecraft:item"),
                registry("minecraft:missing"),
                registry("item"),
                registry(""),
                setting("reader", "mode"),
                setting("reader", "unset"),
                setting("other", "mode"),
                setting("unlisted", "flag"),
                setting("absent", "mode"),
                setting("reader", ""),
                setting("reader", ".."),
                setting("reader", "../mode"),
                setting("reader", "a\\b"),
            ]
        }
        struct Reader {
            registered: Arc<Mutex<Vec<String>>>,
            expected_dir: PathBuf,
        }
        #[crate::__subscriptions]
        impl Hooks for Reader {
            fn register(&self, _: &mut dyn Registrar, cx: &RegistrationContext) {
                *self.registered.lock().unwrap() = probe(cx, &self.expected_dir);
            }
            fn component_types(
                &self,
                cx: &RegistrationContext,
                event: &mut ComponentTypesEvent<'_>,
            ) {
                for answer in probe(cx, &self.expected_dir) {
                    event.register(answer);
                }
            }
        }

        #[cfg(unix)]
        let expected_dir = {
            use std::os::unix::ffi::OsStrExt;
            PathBuf::from(std::ffi::OsStr::from_bytes(b"/mods/\xffinstance"))
        };
        #[cfg(windows)]
        let expected_dir = {
            use std::os::windows::ffi::OsStringExt;
            let mut units: Vec<u16> = r"C:\mods\instance".encode_utf16().collect();
            units.push(0xd800);
            PathBuf::from(std::ffi::OsString::from_wide(&units))
        };
        #[cfg(not(any(unix, windows)))]
        let expected_dir = PathBuf::from("instance");

        let data = RegistrationData::new(
            BTreeMap::from([(
                "minecraft:item".to_owned(),
                vec!["minecraft:air".to_owned(), "minecraft:stone".to_owned()],
            )]),
            BTreeMap::from([
                (
                    "reader".to_owned(),
                    BTreeMap::from([("mode".to_owned(), "fast".to_owned())]),
                ),
                (
                    "other".to_owned(),
                    BTreeMap::from([("mode".to_owned(), "slow".to_owned())]),
                ),
                (
                    "unlisted".to_owned(),
                    BTreeMap::from([("flag".to_owned(), "on".to_owned())]),
                ),
            ]),
            InstanceDirectory::from_path(&expected_dir).unwrap(),
            crate::RuntimeEnvironment::new(
                crate::Dist::Client,
                "26.3".into(),
                "test".into(),
                false,
            ),
        );

        let registered = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "reader",
            Box::new(Reader {
                registered: registered.clone(),
                expected_dir,
            }),
        )]
        .into();
        crate::__private::install(mods.clone());
        let expected = [
            "Some(true)",
            r#"Some(["minecraft:air", "minecraft:stone"])"#,
            "None",
            "None",
            "None",
            r#"Some("fast")"#,
            "None",
            r#"Some("slow")"#,
            r#"Some("on")"#,
            "None",
            "None",
            "None",
            "None",
            "None",
        ];
        with_test_packages(&["reader", "other"], || {
            with_test_registration_data(data, || {
                let cx = RegistrationContext::new();
                assert_eq!(cx.dist(), Dist::Client);
                assert_eq!(cx.game_version(), "26.3");
                assert_eq!(cx.sunrise_version(), "test");
                assert!(!cx.game_tests_enabled());
                let items: &[String] = cx.registry("minecraft:item").unwrap();
                assert_eq!(items, ["minecraft:air", "minecraft:stone"]);
                let mode: &str = cx.setting("reader", "mode").unwrap();
                assert_eq!(mode, "fast");
                crate::__private::install_parts(
                    &mods,
                    &crate::__private::part_tests::rule_schema(),
                    |_| false,
                )
                .unwrap();
                assert_eq!(*registered.lock().unwrap(), expected);
                let mut types = Vec::new();
                dispatch_component_types(&RegistrationContext::new(), &mut types);
                assert_eq!(types, expected, "a later step reads the same snapshot");
            });
            // No engine snapshot: every read returns None.
            crate::__private::install_parts(
                &mods,
                &crate::__private::part_tests::rule_schema(),
                |_| false,
            )
            .unwrap();
            let absent = registered.lock().unwrap().clone();
            assert!(absent.iter().all(|entry| entry == "None"));
        });
    }
}
