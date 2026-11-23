//! Mods in the game (`docs/modding/src/how-to/content-mods.md`): content mods' blocks, items, data and
//! assets (`content_root`), and the Mods page's rows. Code mods are native (`native_hooks.rs`)
//! with content-only packages loaded separately.
use super::{
    Game, execution_role::ExecutionRole, owner_link::OwnerRequest,
    registration_capability_coverage as coverage,
};
mod menu_books;
pub(in crate::game) mod package_files;
pub(in crate::game) mod runtime_files;
mod standalone;
use mod_api::__private::{DeclarationKind, DeclarationRefusal, DeclarationSink, game::*};
use mod_api::{Answered, DeclarationError, LoadingMessage, RegistrationContext};
use standalone::collect_standalone_models;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use sunrise_client::{
    lang,
    mobs::spawner::SpawnerTypeFacts,
    mod_content::DeclaredContent,
    mod_list::{self, Installed},
};
use sunrise_mod_package::manifest::{MissingJar, collect_jar_requirements, missing_jars};
use sunrise_world::{
    item_defaults::ItemData,
    native_load_context::{LoadContext, LoadInputs},
};

struct ContentRequest<'a> {
    path: &'a Path,
    manifest: &'a sunrise_mod_package::Manifest,
    declared: Vec<String>,
    admitted: Option<&'a package_files::PackageContent>,
}

/// Open package sources and read their declarations in parallel, keeping results in request order.
fn read_package_inputs<T: Sync, U: Send>(requests: &[T], read: impl Fn(&T) -> U + Sync) -> Vec<U> {
    if requests.is_empty() {
        return Vec::new();
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let threads = sunrise_threads::worker_count().clamp(1, requests.len());
    let mut done: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(request) = requests.get(index) else {
                            return done;
                        };
                        done.push((index, read(request)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|e| std::panic::resume_unwind(e))
            })
            .collect()
    });
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, result)| result).collect()
}

fn read_contents(
    requests: &[ContentRequest<'_>],
) -> Vec<Result<sunrise_mod_package::Content, String>> {
    read_package_inputs(
        requests,
        |ContentRequest {
             path,
             manifest,
             declared,
             admitted,
         }| {
            let content = match admitted {
                Some(source) => source.read_declaring(manifest, declared),
                None => sunrise_mod_package::Content::read_declaring(path, manifest, declared),
            };
            if declared.is_empty()
                && let Ok(content) = &content
            {
                remember_verdict(path, content);
            }
            content
        },
    )
}

#[derive(Default)]
pub(super) struct CodeSwitches {
    disabled: std::collections::BTreeSet<String>,
    applied: std::sync::atomic::AtomicBool,
}

#[derive(Clone, Default)]
pub(super) struct ModsState {
    pub(super) active_condition_root: Option<Arc<()>>,
    pub(super) load_condition_candidate: Option<sunrise_world::conditions::LoadConditions>,
    pub(super) load_conditions:
        Arc<std::sync::OnceLock<Result<crate::game::load_conditions::ConditionTable, String>>>,
    pub(super) profession_declarations:
        Arc<std::sync::OnceLock<Vec<(String, mod_api::ProfessionDefinition)>>>,
    pub(super) loot_modifier_serializers:
        Arc<std::sync::OnceLock<super::global_loot_modifiers::Serializers>>,
    /// Saved switches gate the first data inventory while registration keeps
    /// its existing startup ordering. Runtime dispatch owns later switches.
    pub(super) code_switches: Option<Arc<CodeSwitches>>,
    /// The original prepared input is fixed for this engine load, shared by
    /// the window and its server. A later data reload must not overlay an
    /// already generated root recursively.
    pub(super) content_base: Arc<std::sync::OnceLock<PathBuf>>,
    /// `--dev-mod PATH`: content mods always on.
    pub(super) dev: Vec<PathBuf>,
    pub(super) native: std::sync::Arc<sunrise_mod_package::runtime::Runtime>,
    pub(super) installation_side: sunrise_mod_package::manifest::Side,
    pub(super) overrides_dir: PathBuf,
    pub(super) load_notices: Arc<Mutex<Vec<sunrise_mod_package::dependencies::Notice>>>,
    pub(super) selected: std::sync::Arc<[sunrise_mod_package::selection::SelectedPackage]>,
    /// The registration inventory and the file indexes it shares with startup content loading.
    pub(super) startup_admission: Option<Arc<package_files::StartupAdmission>>,
    /// The content mods the loaded resources were built with: a switch the
    /// game can apply live moves it to the player's choice (`switch_mods`).
    pub(super) applied: Option<std::collections::BTreeSet<String>>,
    /// A live switch's resource reload is running: when it applied, the server
    /// reloads its data from the new root. Its start, for the log.
    pub(super) live: Option<std::time::Instant>,
    /// The content mods installed, and those of them with only assets and
    /// data (`mod_content::live_switch`): they switch with a world open.
    pub(super) content_ids: std::collections::BTreeSet<String>,
    pub(super) live_ids: std::collections::BTreeSet<String>,
    /// The server's data comes from a root a live switch built, not the one
    /// `res` was loaded from: the world tables are built from it even with no
    /// data pack (`stage_world_tables`).
    pub(super) data_switched: bool,
    /// Structured load refusals, rendered only at the existing Mods-page boundary.
    pub(super) refused: Arc<Mutex<Vec<MissingJar>>>,
    /// Served content packages withheld at startup because their `served_by` host was disabled:
    /// `(content_id, host_id)`.
    pub(super) disabled_served: Arc<[(String, String)]>,
    /// Native hosts with active or withheld served packages at startup.
    pub(super) restart_bound_hosts: BTreeSet<String>,
    /// The entries of the mods' datapack registries this world session has published, at the
    /// generation of the load or reload that built them; `None` when no mod declared one.
    pub(super) datapack_registries: Option<Arc<mod_api::DatapackRegistrySet>>,
    /// Starling: what the content build tells the loading screen while it works
    /// ([`ContentNote`]'s line), and `None` once it is done. The menu's loader sets it.
    pub(super) note: Option<Arc<dyn Fn(Option<String>) + Send + Sync>>,
}

impl ModsState {
    pub(super) fn capture_code_switches(&mut self, disabled: &std::collections::BTreeSet<String>) {
        self.code_switches = Some(Arc::new(CodeSwitches {
            disabled: disabled.clone(),
            applied: Default::default(),
        }));
    }

    pub(super) fn startup_code_disabled(&self, id: &str) -> bool {
        self.code_switches
            .as_ref()
            .is_some_and(|switches| switches.disabled.contains(id))
    }

    /// Compute the effective disabled native set for the live session and any restart-bound
    /// serving hosts whose saved toggle differs from their applied startup state.
    pub(super) fn reconcile_code_switches(
        &self,
        wanted_disabled: &std::collections::BTreeSet<String>,
    ) -> (std::collections::BTreeSet<String>, Vec<String>) {
        let mut effective = wanted_disabled.clone();
        let mut waiting = Vec::new();
        for host in &self.restart_bound_hosts {
            let applied_off = self.startup_code_disabled(host);
            let wanted_off = wanted_disabled.contains(host);
            if applied_off != wanted_off {
                waiting.push(host.clone());
                if applied_off {
                    effective.insert(host.clone());
                } else {
                    effective.remove(host);
                }
            }
        }
        (effective, waiting)
    }

    /// Whether native mod `id` is effectively disabled in this running session (respecting
    /// restart-bound serving hosts).
    pub(super) fn code_disabled(
        &self,
        wanted_disabled: &std::collections::BTreeSet<String>,
        id: &str,
    ) -> bool {
        if self.restart_bound_hosts.contains(id) {
            self.startup_code_disabled(id)
        } else {
            wanted_disabled.contains(id)
        }
    }

    pub(super) fn apply_code_switches(&self, disabled: &std::collections::BTreeSet<String>) {
        let (effective, _) = self.reconcile_code_switches(disabled);
        for id in mod_api::__private::installed_code_ids() {
            mod_api::__private::set_mod_enabled(id, !effective.contains(id));
        }
        if let Some(switches) = &self.code_switches {
            switches
                .applied
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    pub(super) fn code_owner_enabled(&self, owner: &str) -> bool {
        let initially_disabled = self.code_switches.as_ref().is_some_and(|switches| {
            !switches.applied.load(std::sync::atomic::Ordering::Acquire)
                && switches.disabled.contains(owner)
        });
        !initially_disabled
            && mod_api::__private::mod_index(owner).is_some_and(mod_api::__private::mod_index_on)
    }

    /// One admission rule for initial composition and later code-only toggles.
    /// Content acceptance belongs to the generation; code admission belongs to
    /// the installed dispatch entries, including their live enabled switches.
    fn enabled_inventory(
        &self,
        accepted_content: impl Fn(&str) -> bool,
    ) -> sunrise_mod_package::enabled::EnabledMods {
        let native = self
            .native
            .packages()
            .iter()
            .map(|package| &package.manifest)
            .filter(|manifest| self.code_owner_enabled(&manifest.id));
        let content = self
            .selected
            .iter()
            .map(|package| &package.package.manifest)
            .filter(|manifest| accepted_content(&manifest.id));
        sunrise_mod_package::enabled::EnabledMods::new(
            native
                .chain(content)
                .map(|manifest| (manifest.id.clone(), manifest.side)),
        )
    }

    pub(super) fn select(
        &mut self,
        game_dir: &std::path::Path,
        enabled: &std::collections::BTreeSet<String>,
    ) {
        let mut disabled_served = Vec::new();
        self.selected =
            sunrise_mod_package::selection::content_packages(game_dir, enabled, &self.dev)
                .filter(|selected| {
                    let manifest = &selected.package.manifest;
                    if self
                        .native
                        .built()
                        .iter()
                        .any(|built| built.id == manifest.id)
                    {
                        return false;
                    }
                    if let Some(host) = manifest.served_by.as_deref()
                        && self.startup_code_disabled(host)
                    {
                        log::warn!(
                            "Sunrise: served mod {} not loaded because its serving host {} is \
                             disabled; enable {} and restart",
                            manifest.id,
                            host,
                            host
                        );
                        disabled_served.push((manifest.id.clone(), host.to_owned()));
                        return false;
                    }
                    true
                })
                .collect();
        self.disabled_served = disabled_served.into();
        self.restart_bound_hosts = self
            .selected
            .iter()
            .filter_map(|p| p.package.manifest.served_by.clone())
            .chain(self.disabled_served.iter().map(|(_, host)| host.clone()))
            .collect();
        self.refused = Default::default();
    }

    pub(super) fn admit_startup_packages(
        &mut self,
        jar_store: &Path,
    ) -> sunrise_client::Result<Arc<package_files::StartupAdmission>> {
        let refused = self.missing_jar_refusals(jar_store)?;
        if let Some(missing) = self.fatal_missing_jar(&refused) {
            return Err(missing.clone().into());
        }
        let admission = Arc::new(package_files::StartupAdmission::open(self, refused)?);
        self.startup_admission = Some(admission.clone());
        Ok(admission)
    }

    fn missing_jar_refusals(&self, jar_store: &Path) -> sunrise_client::Result<Vec<MissingJar>> {
        let requirements = collect_jar_requirements(
            self.native
                .packages()
                .iter()
                .map(|p| &p.manifest)
                .chain(self.selected.iter().map(|p| &p.package.manifest))
                .map(|m| (m.id.as_str(), m.jar.as_slice())),
        )?;
        Ok(missing_jars(jar_store, &requirements)?)
    }

    fn fatal_missing_jar<'a>(&self, refused: &'a [MissingJar]) -> Option<&'a MissingJar> {
        refused.iter().find(|missing| {
            missing.owners.iter().any(|id| {
                self.native.built().iter().any(|built| built.id == id)
                    || self.selected.iter().any(|p| {
                        p.package.manifest.id == *id && p.package.manifest.served_by.is_some()
                    })
            })
        })
    }

    fn startup_admission(&self) -> Option<&package_files::StartupAdmission> {
        self.startup_admission
            .as_deref()
            .filter(|admission| admission.matches_selection(&self.selected))
    }

    fn report_refusals(&self, rows: &mut [Installed]) {
        let refused = self.refused.lock().unwrap();
        for row in rows {
            let mut problems: Vec<_> = refused
                .iter()
                .filter(|missing| missing.owners.contains(&row.id))
                .map(ToString::to_string)
                .collect();
            if let Some(detail) = self
                .startup_admission
                .as_deref()
                .and_then(|a| a.namespace_refusal(&row.id))
            {
                problems.push(format!("Can't be read: {detail}"));
            }
            for (content_id, host_id) in self.disabled_served.iter() {
                if *content_id == row.id {
                    problems.push(format!(
                        "Not loaded: serving host {host_id} is disabled; enable {host_id} and restart"
                    ));
                }
            }
            if !problems.is_empty() {
                row.problem = Some(problems.join("\n"));
            }
        }
    }
}

mod item_data_load;
pub(super) use item_data_load::{LoadRole, load_item_data};

/// The built-in registries' names in ID order, with Sunrise's own data components.
fn builtin_registry_names(root: &Path) -> sunrise_client::Result<BTreeMap<String, Vec<String>>> {
    let mut registries: BTreeMap<_, _> = builtin_registries(root)?
        .registries
        .into_iter()
        .map(|(name, ids)| (name, ids.names))
        .collect();
    if let Some(types) = registries.get_mut("minecraft:data_component_type") {
        types.extend(sunrise_protocol::item::custom::entries().map(|(_, name)| name.to_owned()));
    }
    Ok(registries)
}

/// Capture registration's stable registries, settings and stock capability type owners.
pub(super) fn install_registration_data(
    root: &Path,
    runtime_files: BTreeMap<String, Arc<dyn mod_api::RuntimeFileSource>>,
    package_files: BTreeMap<String, Arc<dyn mod_api::PackageFileSource>>,
) -> sunrise_client::Result<()> {
    let packages = mod_api::__private::loaded_mods();
    let directory = mod_api::InstanceDirectory::from_path(&Game::game_dir())
        .expect("the host path has an encoding the mod API carries");
    // Android starts before the game files exist (its setup screen prepares them, no launcher did):
    // with none yet there is nothing to read, and no native mod runs in the APK's own game.
    let names = match builtin_registry_names(root) {
        Ok(names) => names,
        Err(_) if cfg!(target_os = "android") && !root.join("sunrise-resources.json").is_file() => {
            BTreeMap::new()
        }
        Err(e) => return Err(e),
    };
    (mod_api::__private::platform::platform()?.install_registration_data)(
        mod_api::__private::RegistrationData::new(
            names,
            sunrise_world::native_load_context::capture_settings(
                &Game::mod_data_root(),
                packages.ids(),
            ),
            directory,
            crate::entry::runtime_environment::get().clone(),
        )
        .with_capability_coverage(coverage::builtin, coverage::block_entity)
        .with_runtime_files(runtime_files)
        .with_package_files(package_files),
    );
    Ok(())
}

fn builtin_registries(root: &Path) -> sunrise_client::Result<sunrise_protocol::registry::Builtin> {
    let path = sunrise_protocol::registry::Builtin::find(root).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "registries.json not found")
    })?;
    Ok(sunrise_protocol::registry::Builtin::load(&path)?)
}

/// Freeze native component IDs before reading content or opening a connection.
/// With no mod registering any, nothing is installed, which every reader
/// treats as no custom components, and `registries.json` isn't read.
pub(super) fn register_components(root: &std::path::Path) -> sunrise_client::Result<()> {
    let mut names = Vec::new();
    Game::native_component_types(&RegistrationContext::new(), &mut names);
    if names.is_empty() {
        return Ok(());
    }
    let builtin = builtin_registries(root)?;
    let types = builtin
        .get("minecraft:data_component_type")
        .ok_or("missing data component registry")?;
    let first = i32::try_from(types.names.len())?;
    sunrise_protocol::item::custom::install(first, names)
        .map_err(|error| format!("invalid native component registration: {error}"))?;
    Ok(())
}

/// Finish every once-only content declaration on the registration thread.
pub(super) fn freeze_declarations() {
    let _ = native_declared();
}

#[derive(Clone, Debug)]
pub(crate) struct DeclarationFailures(Vec<DeclarationRefusal>);

impl DeclarationFailures {
    pub(super) fn new(refusals: Vec<DeclarationRefusal>) -> Self {
        Self(refusals)
    }

    fn messages(&self) -> impl Iterator<Item = LoadingMessage> {
        use sunrise_client::text::Text;
        let quoted = |value: &str| Text::literal(format!("{value:?}"));
        self.0.iter().map(move |refusal| {
            let key = match refusal.kind {
                DeclarationKind::Block => "sunrise.mods.declaration.block",
                DeclarationKind::Item => "sunrise.mods.declaration.item",
            };
            let detail = match &refusal.error {
                DeclarationError::RegistrationClosed { id } => {
                    Text::translate("sunrise.mods.declaration.closed", &[quoted(id)])
                }
                DeclarationError::InvalidId { id } => {
                    Text::translate("sunrise.mods.declaration.invalid_id", &[quoted(id)])
                }
                DeclarationError::UnownedNamespace { id, namespace } => Text::translate(
                    "sunrise.mods.declaration.unowned_namespace",
                    &[quoted(id), quoted(namespace)],
                ),
                DeclarationError::Rejected { id, reason } => Text::translate(
                    "sunrise.mods.declaration.rejected",
                    &[quoted(id), Text::literal(reason.clone())],
                ),
                error => Text::literal(error.to_string()),
            };
            let detail = detail.plain().replace(['\r', '\n'], " ");
            LoadingMessage::new(key, &[&refusal.mod_id, &detail])
        })
    }

    pub(crate) fn lines(&self) -> Vec<String> {
        use sunrise_client::text::Text;
        let render = || {
            self.messages()
                .map(|message| {
                    let args: Vec<_> = message
                        .args()
                        .iter()
                        .map(|arg| Text::literal(arg.clone()))
                        .collect();
                    Text::translate(message.key(), &args).plain()
                })
                .collect()
        };
        if lang::current().is_some() {
            render()
        } else {
            lang::scoped(lang::Language::load(&[], lang::DEFAULT), render).0
        }
    }
}

impl std::fmt::Display for DeclarationFailures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.lines().join("\n"))
    }
}
impl std::error::Error for DeclarationFailures {}

#[derive(Clone, Default)]
struct NativeDeclared {
    refusals: Vec<DeclarationRefusal>,
    batches: BTreeMap<&'static str, DeclaredContent>,
}

impl NativeDeclared {
    fn stage<D, T>(
        &mut self,
        listen: &mut dyn FnMut(&mut DeclarationSink<'_, D>) -> bool,
        mut validate: impl FnMut(D) -> Result<T, DeclarationError>,
        mut publish: impl FnMut(&mut DeclaredContent, T),
    ) {
        let mut entries = Vec::new();
        if listen(&mut |Answered { mod_id, answer, .. }| {
            entries.push((mod_id, validate(answer)?));
            Ok(())
        }) {
            for (owner, value) in entries {
                publish(self.batches.entry(owner).or_default(), value);
            }
        }
    }
}

/// The same native declarations serve startup and every pack reload.
fn native_declared() -> std::borrow::Cow<'static, NativeDeclared> {
    let collect = || {
        use super::api_values::TryIntoEngine;
        let mut declared = NativeDeclared::default();
        let context = RegistrationContext::new();
        Game::native_blocks(&context, &mut |listen| {
            declared.stage(
                listen,
                |answer| {
                    answer
                        .try_into_engine(&())
                        .and_then(|b| b.validate_local())
                        .map_err(|reason| DeclarationError::Rejected {
                            id: answer.id,
                            reason,
                        })
                },
                |batch, block| batch.blocks.push(block),
            );
        });
        Game::native_items(&context, &mut |listen| {
            declared.stage(
                listen,
                |answer| {
                    answer
                        .try_into_engine(&())
                        .and_then(|i| i.validate_local())
                        .map_err(|reason| DeclarationError::Rejected {
                            id: answer.id,
                            reason,
                        })
                },
                |batch, item| batch.items.push(item),
            );
        });
        declared.refusals = mod_api::__private::take_declaration_refusals(&context);
        for (owner, kind) in super::api_values::block_entity_declarations() {
            declared.batches.entry(owner).or_default().types.push(kind);
        }
        for &(owner, ref draft) in mod_api::__private::declared_world_fluids() {
            let fluid = draft.try_into_engine(&()).unwrap();
            let batch = declared.batches.entry(owner).or_default();
            if let Some(block) = fluid.world_block() {
                // The generated template and fields are fixed by FluidLook, not the mod.
                batch.blocks.push(block.validate_local().unwrap());
            }
            if let Some(bucket) = fluid.world_bucket() {
                batch.items.push(bucket.validate_local().unwrap());
            }
            batch.fluids.push(fluid);
        }
        declared
    };
    // Production freezes one loaded mod set for startup and reload. Tests replace
    // that set in thread-local scopes, so a process-wide snapshot would retain
    // whichever fixture reached content loading first, even on another thread.
    #[cfg(not(test))]
    {
        static DECLARED: std::sync::OnceLock<NativeDeclared> = std::sync::OnceLock::new();
        std::borrow::Cow::Borrowed(DECLARED.get_or_init(collect))
    }
    #[cfg(test)]
    {
        // Test fixtures bypass startup; collect with registration open as startup does.
        let _registration = mod_platform::__private::RegistrationGuard::open();
        std::borrow::Cow::Owned(collect())
    }
}
/// The native mods' standalone models (`native_standalone_models`), set once
/// before the resources first load.
pub(super) fn register_standalone_models(role: ExecutionRole) {
    if let Some(registrations) = collect_standalone_models(role) {
        sunrise_client::models::register_standalone(registrations);
    }
    register_fluid_models(role);
}

/// Recollect before each resource bake, retaining registration-phase attribution.
pub(super) fn register_fluid_models(role: ExecutionRole) {
    if let Some(models) = collect_fluid_models(role) {
        sunrise_client::fluid_models::register(models);
    }
}

fn collect_fluid_models(
    role: ExecutionRole,
) -> Option<Vec<sunrise_mod_package::content::FluidModel>> {
    if !role.is_client() {
        return None;
    }
    let mut models = Vec::new();
    Game::native_register_fluid_models(&RegistrationContext::new(), &mut models);
    Some(
        models
            .into_iter()
            .map(|model| sunrise_mod_package::content::FluidModel {
                fluid: model.fluid,
                still: model.still,
                flowing: model.flowing,
                overlay: model.overlay,
            })
            .collect(),
    )
}

pub(super) fn collect_key_mappings<'declared>(
    role: ExecutionRole,
    declared: &'declared [(&'static str, mod_api::KeyRegistration)],
) -> &'declared [(&'static str, mod_api::KeyRegistration)] {
    if role.is_client() { declared } else { &[] }
}

/// Freeze the native mods' entity model layer definitions at startup, then bake their skull
/// types (`native_create_skull_models`) from those definitions or vanilla's under `vanilla_root`.
pub(super) fn register_layer_definitions(
    role: ExecutionRole,
    vanilla_root: &std::path::Path,
) -> sunrise_client::Result<()> {
    if !role.is_client() {
        return Ok(());
    }
    let mut suppliers = Vec::new();
    Game::native_register_layer_definitions(&RegistrationContext::new(), &mut suppliers);
    sunrise_client::mobs::model_layers::register(suppliers, vanilla_root)?;
    let mut models = Vec::new();
    Game::native_create_skull_models(&RegistrationContext::new(), &mut models);
    sunrise_client::skull_models::register(models, vanilla_root);
    Ok(())
}
/// Freeze the native mods' F3 text callbacks and renderer toggles at client startup.
pub(super) fn register_debug_entries(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut entries = Vec::new();
    Game::native_register_debug_entries(&RegistrationContext::new(), &mut entries);
    sunrise_client::debug_entries::register(entries);
}
/// The native mods' map decoration types (`native_map_decoration_types`), frozen once at start on
/// every side.
pub(super) fn register_map_decoration_types() {
    let mut types = Vec::new();
    Game::native_map_decoration_types(&RegistrationContext::new(), &mut types);
    sunrise_client::map_decoration_types::register(types, Game::is_vanilla_decoration_sprite);
}
/// Install the native mods' HUD draw callbacks among the vanilla anchors once at client startup.
pub(super) fn register_gui_layers(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut layers = sunrise_client::gui_layers::GuiLayers::default();
    Game::native_register_gui_layers(&RegistrationContext::new(), &mut layers);
    sunrise_client::gui_layers::freeze(layers);
}
/// Collect the native mods' owned debug renderers and their owners on each list refresh.
pub(super) fn debug_renderers(
    role: ExecutionRole,
    entries: &dyn mod_api::__private::DebugEntryStates,
) -> Vec<mod_api::Answered<mod_api::DebugRenderer>> {
    let mut renderers = Vec::new();
    if role.is_client() {
        Game::native_register_debug_renderers(&RegistrationContext::new(), entries, &mut renderers);
    }
    renderers
}
/// The native mods' numeric item model properties
/// (`native_register_range_select_item_model_property`), kept once at start, before
/// resources load, so `range_dispatch` models on them read the mod's number.
pub(super) fn register_range_select_item_model_properties(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut properties = Vec::new();
    Game::native_register_range_select_item_model_property(
        &RegistrationContext::new(),
        &mut properties,
    );
    sunrise_client::item_render::install_range_property_factories(properties);
}
/// The native mods' categorical item model properties
/// (`native_register_select_item_model_property`), kept once at start, before resources
/// load, so `select` models on them read the mod's value.
pub(super) fn register_select_item_model_properties(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut properties = Vec::new();
    Game::native_register_select_item_model_property(&RegistrationContext::new(), &mut properties);
    sunrise_client::item_render::install_select_property_factories(properties);
}
/// The types native mods register for resource definitions, kept once at start, before
/// resources load: item tint sources (`native_register_item_tint_sources`), so `tints` entries on
/// them read the mod's colour; atlas sprite sources (`native_register_sprite_sources`), so atlas
/// definitions' entries on them list the mod's sprites; model loaders
/// (`native_register_model_loaders`), so models naming one get the mod's quads; item model node
/// types (`native_register_item_models`), so item definitions' nodes of them draw what the mod
/// answers; and animation targets and interpolations (`native_register_json_animation_types`),
/// bound directly to their callbacks so animation files resolve and convert them at parse time.
/// Block-state
/// definition types (`native_register_block_state_models`), so blockstate files naming one get
/// the models the mod picks; and special item model factories
/// (`native_register_special_model_renderer`), so each special model node of a registered type
/// bakes its own renderer.
pub(super) fn register_resource_sources(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut sources = Vec::new();
    Game::native_register_item_tint_sources(&RegistrationContext::new(), &mut sources);
    sunrise_client::item_render::install_item_tint_factories(sources);
    let mut sources = Vec::new();
    Game::native_register_sprite_sources(&RegistrationContext::new(), &mut sources);
    sunrise_client_render::mod_sprite_sources::register(sources);
    let mut loaders = Vec::new();
    Game::native_register_model_loaders(&RegistrationContext::new(), &mut loaders);
    sunrise_client_render::mod_model_loaders::register(loaders);
    let mut models = Vec::new();
    Game::native_register_item_models(&RegistrationContext::new(), &mut models);
    sunrise_client::item_render::install_item_model_factories(models);
    let mut types = Vec::new();
    Game::native_register_json_animation_types(&RegistrationContext::new(), &mut types);
    sunrise_client_render::mod_animations::register(types);
    let mut definitions = Vec::new();
    Game::native_register_block_state_models(&RegistrationContext::new(), &mut definitions);
    sunrise_client_render::mod_block_state_models::register(definitions);
    register_special_model_renderers();
}
/// The native mods' special item model renderers (`native_register_special_model_renderer`),
/// kept once at start, before resources load: their types are what a `minecraft:special` model
/// may name besides vanilla's, or in place of one. A later registration of a type replaces the
/// earlier one, and the log names both mods.
fn register_special_model_renderers() {
    let mut registry = mod_api::SpecialModelRendererRegistry::default();
    Game::native_register_special_model_renderer(&RegistrationContext::new(), &mut registry);
    for (kind, owner) in registry.initial_owners() {
        if sunrise_assets::item_model::SpecialModel::is_builtin_type(&kind.to_string()) {
            log::warn!(
                "Sunrise: special model type {kind} registered by the game is replaced by {}'s registration",
                owner.unwrap_or("the game")
            );
        }
    }
    let (factories, replaced) = registry.into_renderers();
    for replaced in replaced {
        let name = |id: Option<&str>| id.unwrap_or("the game").to_owned();
        log::warn!(
            "Sunrise: special model type {} registered by {} is replaced by {}'s registration",
            replaced.kind,
            name(replaced.earlier),
            name(replaced.later)
        );
    }
    sunrise_client::item_render::install_special_model_factories(
        factories
            .into_iter()
            .filter_map(|(kind, (owner, factory))| {
                let owner = owner?;
                Some((
                    kind,
                    sunrise_client::item_render::OwnedSpecialModelFactory::new(owner, factory),
                ))
            })
            .collect(),
    );
}
/// The client presentation registrations, kept once at start, before resources load: the
/// dimension transition screens, the entity spectator shaders, the render pipelines and the entity
/// renderers.
pub(super) fn register_client_presentations(role: ExecutionRole) -> sunrise_client::Result<()> {
    register_preset_editors(role);
    register_dimension_transition_screens(role);
    register_entity_spectator_shaders(role);
    register_render_pipelines(role)?;
    register_entity_renderers(role);
    Ok(())
}

/// Resolve client dimension-transition registrations once, before the first level opens.
fn register_dimension_transition_screens(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut screens = Vec::new();
    Game::native_register_dimension_transition_screen(&RegistrationContext::new(), &mut screens);
    sunrise_client_render::dimension_transition::register(screens);
}

/// Resolve client spectator post-effect registrations once, before the first level opens.
pub(super) fn register_entity_spectator_shaders(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut shaders = Vec::new();
    Game::native_register_entity_spectator_shaders(&RegistrationContext::new(), &mut shaders);
    sunrise_client_render::spectator_shaders::register(shaders);
}

pub(super) fn register_preset_editors(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut editors = Vec::new();
    Game::native_register_preset_editors(&RegistrationContext::new(), &mut editors);
    sunrise_client::title::preset_editors::register(editors);
}

/// Keep the mods' render pipeline registrations once at startup, before resources load: every
/// load builds them from its own shader files.
pub(super) fn register_render_pipelines(role: ExecutionRole) -> sunrise_client::Result<()> {
    if !role.is_client() {
        return Ok(());
    }
    let mut pipelines = Vec::new();
    Game::native_register_render_pipelines(&RegistrationContext::new(), &mut pipelines);
    sunrise_client_render::mod_pipelines::register(pipelines)?;
    Ok(())
}

/// The native mods' meshing registrations, kept once at start, before resources load: colour
/// resolvers, then the block tint sources that read them, then section geometry renderers.
pub(super) fn register_meshing(role: ExecutionRole) {
    register_color_resolvers(role);
    register_block_tint_sources(role);
    add_section_geometry(role);
}

/// The native mods' colour resolvers (`native_register_color_resolvers`), kept once at start,
/// before the block tint sources that read them and before resources load.
fn register_color_resolvers(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::ColorResolverRegistry::default();
    Game::native_register_color_resolvers(&RegistrationContext::new(), &mut registry);
    sunrise_client::mesher::register_color_resolvers(registry.into_resolvers());
}

/// The native mods' block tint sources (`native_register_block_tint_sources`), kept once at
/// start, before resources load; every resource load applies them to its mesh tables.
fn register_block_tint_sources(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::BlockTintRegistry::default();
    Game::native_register_block_tint_sources(&RegistrationContext::new(), &mut registry);
    sunrise_client::mesher::register_block_tint_sources(registry.into_layers());
}
/// The native mods' section geometry renderers (`native_add_section_geometry`), kept once at
/// start, before resources load; every mesh of a section asks them to add their quads.
fn add_section_geometry(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::SectionGeometryRegistry::default();
    Game::native_add_section_geometry(&RegistrationContext::new(), &mut registry);
    sunrise_client::mesher::register_section_geometry(registry.into_renderers());
}
/// The native mods' entity renderers (`native_register_entity_renderers`), kept once at start,
/// before resources load; every resource load resolves each type's renderer.
pub(super) fn register_entity_renderers(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::EntityRendererRegistry::default();
    Game::native_register_entity_renderers(&RegistrationContext::new(), &mut registry);
    super::native_entity_renderers::register(registry.into_renderers());
}
/// The native mods' particle providers (`native_register_particle_providers`) and render groups
/// (`native_register_particle_groups`) are registered once at startup. Each particle engine
/// constructs its groups once; sprite-set loads associate types with those groups and providers.
pub(super) fn register_particles(role: ExecutionRole) -> sunrise_client::Result<()> {
    if !role.is_client() {
        return Ok(());
    }
    let mut registry = mod_api::ParticleProviderRegistry::default();
    Game::native_register_particle_providers(&RegistrationContext::new(), &mut registry);
    sunrise_client::particles::register_particle_providers(registry.into_providers());
    let mut group_factories = Vec::new();
    Game::native_register_particle_groups(&RegistrationContext::new(), &mut group_factories);
    sunrise_client::particles::register_particle_groups(group_factories)?;
    Ok(())
}

/// What [`content_root`] tells a loading screen while it works.
pub(super) enum ContentNote<'a> {
    /// A mod's assets coming from Modrinth: its name, and the bytes so far and in all.
    Fetching {
        name: &'a str,
        done: u64,
        total: u64,
    },
    /// The mods' root being built: their ids.
    Building(&'a str),
}

impl ContentNote<'_> {
    /// The loading screen's line for it.
    pub(super) fn text(&self) -> String {
        let mb = |bytes: u64| bytes as f64 / 1_000_000.0;
        match self {
            ContentNote::Fetching { name, done, total } => format!(
                "Downloading {name}'s textures and models from Modrinth: {:.1} of {:.1} MB",
                mb(*done),
                mb(*total)
            ),
            ContentNote::Building(names) => {
                format!("Preparing mods: {names} (the first time takes a minute)")
            }
        }
    }
}

/// `root` with the content mods that are on layered over it
/// (docs/modpacks.md): their blocks, items, data and assets. Read once at
/// start, before the resources load; a mod whose content is refused is left
/// out here and its row on the Mods page says why.
/// The merged content root and the accepted content mods behind it, before
/// any process-wide registration. A caller that only reads the overlay (the
/// reload tests) must not freeze the mod registries for the rest of the
/// process.
pub(super) struct ContentOverlay {
    /// The root with the mods' content merged in.
    pub(super) root: sunrise_assets::resource_source::ResourceRoot,
    /// The accepted content mods, in overlay order.
    pub(super) mods: Vec<sunrise_client::mod_content::ContentMod>,
    /// The protocol entries of the accepted mods.
    pub(super) announced: Vec<sunrise_protocol::mod_list::Entry>,
    /// Whether the protocol's local mod list is still unset.
    pub(super) announce: bool,
    /// The accepted content-only mods, each with its version, for the worlds saved to record.
    pub(super) content_mods: Vec<sunrise_client::save::level_mods::ModRecord>,
}

/// A package's primary namespace is not an alias another layer can acquire.
fn refuse_primary_namespace_claims(
    manifests: &[&sunrise_mod_package::Manifest],
) -> sunrise_client::Result<()> {
    let mut primary: std::collections::HashSet<&str> = manifests
        .iter()
        .map(|manifest| manifest.id.as_str())
        .collect();
    for id in mod_api::__private::installed_code_ids() {
        primary.insert(id);
    }
    for manifest in manifests {
        if let Some(namespace) = manifest
            .namespaces
            .iter()
            .find(|ns| primary.contains(ns.as_str()))
        {
            return Err(format!(
                "mod {} claims namespace {namespace}, which is the primary namespace of mod {namespace}; turn one of them off",
                manifest.id,
            ).into());
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServedContentStage {
    Read,
    Admit,
}

#[derive(Debug)]
struct ServedContentLoadError {
    mod_id: String,
    stage: ServedContentStage,
    detail: String,
}

impl std::fmt::Display for ServedContentLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stage = match self.stage {
            ServedContentStage::Read => "read",
            ServedContentStage::Admit => "admit",
        };
        write!(
            f,
            "Sunrise: served mod {} content failed to {stage}: {}",
            self.mod_id, self.detail
        )
    }
}

impl std::error::Error for ServedContentLoadError {}

#[derive(Debug, PartialEq, Eq)]
struct DisabledServingHost {
    content: String,
    host: String,
}

impl std::fmt::Display for DisabledServingHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "served mod {} cannot load while its serving host {} is disabled",
            self.content, self.host
        )
    }
}

impl std::error::Error for DisabledServingHost {}

/// Validate namespace ownership and `served_by` rules before installing the
/// frozen `PackageInfo` inventory that registration hooks read.
pub(super) fn validate_startup_packages(sources: &ModsState) -> sunrise_client::Result<()> {
    for selected in sources.selected.iter() {
        let manifest = &selected.package.manifest;
        if let Some(host) = manifest.served_by.as_deref()
            && sources.startup_code_disabled(host)
        {
            return Err(Box::new(DisabledServingHost {
                content: manifest.id.clone(),
                host: host.to_owned(),
            }));
        }
    }
    let manifests: Vec<&sunrise_mod_package::Manifest> = sources
        .native
        .packages()
        .iter()
        .map(|p| &p.manifest)
        .chain(sources.selected.iter().map(|p| &p.package.manifest))
        .collect();
    refuse_primary_namespace_claims(&manifests)?;
    sunrise_mod_package::dependencies::validate_served_by(&manifests)?;
    Ok(())
}

/// The owning package ID for an admitted content layer: the serving native host for a
/// `served_by` content package, otherwise the layer itself.
fn content_layer_owner<'a>(
    layer_id: &'a str,
    selected: &'a [sunrise_mod_package::selection::SelectedPackage],
) -> &'a str {
    selected
        .iter()
        .find(|p| p.package.manifest.id == layer_id)
        .and_then(|p| p.package.manifest.served_by.as_deref())
        .unwrap_or(layer_id)
}

/// Maps each admitted content layer's namespaces to its owning package ID:
/// the serving native host for `served_by` content packages, otherwise the layer itself.
fn content_owner_pairs<'a>(
    mods: &'a [sunrise_client::mod_content::ContentMod],
    selected: &'a [sunrise_mod_package::selection::SelectedPackage],
) -> Vec<(&'a str, &'a str)> {
    mods.iter()
        .flat_map(|layer| {
            let owner = content_layer_owner(&layer.id, selected);
            layer
                .content
                .namespaces
                .iter()
                .map(move |namespace| (namespace.as_str(), owner))
        })
        .collect()
}

/// Builds the content overlay: every enabled or dev content mod's files
/// merged over `root`. Registration of what the mods declare is
/// [`content_root`]'s step.
pub(super) fn content_overlay(
    root: &std::path::Path,
    sources: &ModsState,
    jar_store: &std::path::Path,
) -> sunrise_client::Result<ContentOverlay> {
    let admission = sources.startup_admission();
    let refused = match admission {
        Some(admission) => admission.refused.clone(),
        None => sources.missing_jar_refusals(jar_store)?,
    };
    content_overlay_with_refusals(root, sources, jar_store, admission, &refused)
}

fn content_overlay_with_refusals(
    root: &Path,
    sources: &ModsState,
    jar_store: &Path,
    admission: Option<&package_files::StartupAdmission>,
    refused: &[MissingJar],
) -> sunrise_client::Result<ContentOverlay> {
    use sunrise_client::mod_content::{self, ContentMod};
    let original_base = root.to_owned();

    *sources.refused.lock().unwrap() = refused.to_vec();
    if let Some(missing) = sources.fatal_missing_jar(refused) {
        return Err(missing.clone().into());
    }

    let mut known = None;
    let mut mods: Vec<ContentMod> = Vec::new();
    let mut content_mods = Vec::new();
    let announce = !sunrise_protocol::mod_list::is_set();
    let mut announced: Vec<_> = sources
        .native
        .built()
        .iter()
        .map(|m| {
            let runtime = sources
                .native
                .packages()
                .iter()
                .find(|p| p.manifest.id == m.id)
                .and_then(|p| p.manifest.runtime.as_ref());
            announced_entry(m.id, m.version, m.side, runtime)
        })
        .collect();
    struct PackageSource<'a> {
        path: &'a std::path::Path,
        manifest: &'a sunrise_mod_package::Manifest,
        content_only: bool,
    }
    let native = sources.native.packages().iter().map(|p| PackageSource {
        path: &p.path,
        manifest: &p.manifest,
        content_only: false,
    });
    let content = sources.selected.iter().map(|p| PackageSource {
        path: &p.path,
        manifest: &p.package.manifest,
        content_only: true,
    });
    let mut seen = std::collections::HashSet::new();
    let packages: Vec<_> = native
        .chain(content)
        .filter(|source| {
            !refused
                .iter()
                .any(|missing| missing.owners.contains(&source.manifest.id))
        })
        .filter(|source| {
            let code = source.manifest.native.is_some();
            !code || seen.insert(source.manifest.id.as_str())
        })
        .collect();
    let manifests: Vec<_> = packages.iter().map(|source| source.manifest).collect();
    refuse_primary_namespace_claims(&manifests)?;
    let overrides = sunrise_mod_package::dependencies::Overrides::read(&sources.overrides_dir)?;
    let resolution = sunrise_mod_package::dependencies::resolve_manifests(
        &manifests,
        sources.installation_side,
        &overrides,
    )
    .map_err(|error| error.to_string())?;
    let order = resolution.order;
    // Hooks are queried on the calling thread, never from a content reader.
    let declared = native_declared();
    let requests: Vec<_> = order
        .iter()
        .map(|&index| {
            let source = &packages[index];
            ContentRequest {
                path: source.path,
                manifest: source.manifest,
                declared: super::native_content_items::owned_item_ids(
                    source.manifest,
                    &declared.batches,
                ),
                admitted: admission.and_then(|a| a.package(&source.manifest.id, source.path)),
            }
        })
        .collect();
    let read = read_contents(&requests);
    for (index, content) in order.into_iter().zip(read) {
        let source = &packages[index];
        let m = source.manifest;
        let content = match content {
            Ok(content) => content,
            Err(e) => {
                if m.served_by.is_some() {
                    return Err(Box::new(ServedContentLoadError {
                        mod_id: m.id.clone(),
                        stage: ServedContentStage::Read,
                        detail: e,
                    }));
                }
                log::warn!("Sunrise: mod {} content not loaded: {e}", m.id);
                continue;
            }
        };
        // What every client must have refuses a server package outright, code or content alone.
        if let Some(e) = content.server_only_refusal(m) {
            return Err(e.into());
        }
        let known = match &mut known {
            Some(types) => types,
            None => match mod_content::known_components(root) {
                Ok(types) => known.insert(types),
                Err(e) => {
                    log::warn!("Sunrise: content mods not loaded: {e}");
                    return Err(e);
                }
            },
        };
        let candidate = ContentMod {
            id: m.id.clone(),
            content,
            jars: m.jar.clone(),
        };
        match mod_content::accept_mod(&mut mods, candidate, known) {
            Ok(_) if source.content_only => {
                content_mods.push(sunrise_client::save::level_mods::ModRecord {
                    id: m.id.clone(),
                    version: m.version.to_string(),
                });
                announced.push(announced_entry(
                    &m.id,
                    &m.version.to_string(),
                    m.side,
                    m.runtime.as_ref(),
                ));
            }
            Ok(_) => {}
            Err(e) if m.served_by.is_some() => {
                return Err(Box::new(ServedContentLoadError {
                    mod_id: m.id.clone(),
                    stage: ServedContentStage::Admit,
                    detail: e.to_string(),
                }));
            }
            Err(e) => log::warn!("Sunrise: mod {} content not loaded: {e}", m.id),
        }
    }
    let mut refusals = declared.refusals.clone();
    if !declared.batches.is_empty() {
        let known = match &mut known {
            Some(types) => types,
            None => known.insert(mod_content::known_components(root)?),
        };
        let admission = mod_content::accept_declared(
            &mut mods,
            &declared.batches,
            &manifests,
            known,
            &mut refusals,
        );
        if !refusals.is_empty() {
            return Err(DeclarationFailures::new(refusals).into());
        }
        admission?;
        // A file item's remainder named a code-declared item that was refused: that mod
        // alone is left out, not every content mod through the overlay's check.
        for e in mod_content::drop_dangling_remainders(&mut mods) {
            log::warn!("Sunrise: {e}");
        }
    } else if !refusals.is_empty() {
        return Err(DeclarationFailures::new(refusals).into());
    }
    for m in &manifests {
        let suppliers: Vec<&sunrise_mod_package::Manifest> = std::iter::once(*m)
            .chain(
                manifests
                    .iter()
                    .copied()
                    .filter(|content| content.served_by.as_deref() == Some(m.id.as_str())),
            )
            .collect();
        let batch_declarations =
            declared
                .batches
                .get(m.id.as_str())
                .into_iter()
                .flat_map(|batch| {
                    batch
                        .blocks
                        .iter()
                        .map(|b| ("block", b.get().id.as_str()))
                        .chain(batch.items.iter().map(|i| ("item", i.get().id.as_str())))
                        .chain(
                            batch
                                .types
                                .iter()
                                .map(|t| ("block entity type", t.id.as_str())),
                        )
                        .chain(batch.fluids.iter().map(|f| ("fluid", f.id.as_str())))
                });
        let registrar_declarations = mod_api::__private::client_required_declarations(&m.id);
        let all_declarations = batch_declarations.chain(
            registrar_declarations
                .iter()
                .map(|(kind, id)| (*kind, id.as_str())),
        );
        if let Some(refusal) = m.server_only_declaration_for(&suppliers, all_declarations) {
            return Err(Box::new(refusal));
        }
        if m.side == sunrise_mod_package::manifest::Side::Server
            && let Some(requirement) = mod_api::__private::client_requirement(&m.id)
        {
            return Err(format!(
                "{}: a side = \"server\" mod {requirement}; declare side = \"both\"",
                m.id
            )
            .into());
        }
    }
    validate_entity_type_families(&mods, super::general::general_type_catalog())?;
    // Content validation may refuse a package. Revalidate against what really
    // loaded so dependents cannot start with a missing required content mod.
    let loaded: Vec<_> = manifests
        .into_iter()
        .filter(|package| {
            mods.iter().any(|content| content.id == package.id)
                || sources
                    .native
                    .built()
                    .iter()
                    .any(|native| native.id == package.id)
        })
        .collect();
    let resolved = sunrise_mod_package::dependencies::resolve_manifests(
        &loaded,
        sources.installation_side,
        &overrides,
    )
    .map_err(|error| error.to_string())?;
    // The game's derived caches (`logging::cache_dir`): the folder the game runs in, or Android's
    // private files. Android's game folder is in shared storage, which refuses the overlay's links:
    // a phone with a code or content mod on fell back to its setup ("Permission denied").
    let cache = sunrise_client::logging::cache_dir();
    // Content admission and the live code-dispatch boundary jointly determine
    // what loaded. Namespaces and raw startup metadata cannot answer this.
    let inventory = sources.enabled_inventory(|id| mods.iter().any(|loaded| loaded.id == id));
    let prepared = super::load_conditions::content_root_with_conditions(
        root, jar_store, &mods, &cache, &inventory, sources,
    )?;
    let root = mod_content::resolved_resources(root, prepared, jar_store, &mods, cache)?;
    let _ = sources.content_base.set(original_base);
    *sources.load_notices.lock().unwrap() = resolved.warnings;
    Ok(ContentOverlay {
        root,
        mods,
        announced,
        announce,
        content_mods,
    })
}

pub(in crate::game) fn resolve_particle_providers(
    source: &sunrise_assets::resource_source::ResourceSource,
) -> Result<(), String> {
    if mod_api::__private::particles_resolved() {
        return Ok(());
    }
    // Resolve all particle providers once, alongside the other load-owned registries.
    let mut particles: Vec<String> = sunrise_client::particles::ParticleType::ALL
        .iter()
        .map(|ty| format!("minecraft:{}", ty.name()))
        .collect();
    particles.extend(
        source
            .list_under("assets", &["particles"])
            .into_iter()
            .filter_map(|path| {
                let path = path.strip_prefix("assets/")?;
                let (namespace, path) = path.split_once("/particles/")?;
                let path = path.strip_suffix(".json")?;
                let id = format!("{namespace}:{path}");
                mod_api::Identifier::parse_namespaced(&id)?;
                // A vanilla-namespace file the game has no particle for is a leftover of an
                // older resource extraction, not a mod's declaration: skip it, never refuse startup.
                if namespace == "minecraft"
                    && sunrise_client::particles::ParticleType::from_name(&id).is_none()
                {
                    log::warn!("Sunrise: ignoring {id}: no such vanilla particle in this version");
                    return None;
                }
                Some(id)
            }),
    );
    mod_api::__private::resolve_particles(particles.iter().map(String::as_str), |id| {
        sunrise_client::particles::ParticleType::from_name(id)
            .map(|ty| mod_api::__private::ParticleKind::Vanilla(ty as u32))
            .or_else(|| {
                (!id.starts_with("minecraft:")).then_some(mod_api::__private::ParticleKind::Custom)
            })
    })
    .map_err(|errors| errors.join("\n"))?;
    Ok(())
}
#[cfg(test)]
mod particle_registry_reload_tests {
    #[test]
    fn an_unlisted_vanilla_provider_file_does_not_refuse_startup() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let source = sunrise_assets::resource_source::ResourceSource::indexed(
            [(
                "assets/minecraft/particles/ambient_entity_effect.json".into(),
                sunrise_assets::resource_source::Resource::bytes(Vec::new()),
            )]
            .into(),
        );
        super::resolve_particle_providers(&source).unwrap();
    }

    #[test]
    fn repeated_particle_content_load_keeps_the_published_keys() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let source = sunrise_assets::resource_source::ResourceSource::indexed(
            [(
                "assets/invalid namespace/particles/bad name.json".into(),
                sunrise_assets::resource_source::Resource::bytes(Vec::new()),
            )]
            .into(),
        );
        super::resolve_particle_providers(&source).unwrap();
        let first = mod_api::__private::particle_vanilla_key(
            sunrise_client::particles::ParticleType::Smoke as u32,
        )
        .unwrap();
        super::resolve_particle_providers(&source).unwrap();
        assert_eq!(
            mod_api::__private::particle_vanilla_key(
                sunrise_client::particles::ParticleType::Smoke as u32
            ),
            Some(first)
        );
    }
}

/// The content root with everything the mods declare registered against the
/// process: block entities, mob effects, entity templates and menu definitions,
/// fluid looks, and the protocol's local mod list. Scalar registry keys are
/// published only after the gameplay loader chooses its resource generation.
pub(in crate::game) fn content_root(
    root: &std::path::Path,
    sources: &ModsState,
) -> sunrise_client::Result<sunrise_assets::resource_source::ResourceRoot> {
    let _step = sunrise_client::spans::startup::step("mods.content_root");
    let jar_store = sunrise_client::logging::game_home().join("jars");
    let overlay = {
        let _step = sunrise_client::spans::startup::step("mods.overlay");
        content_overlay(root, sources, &jar_store)?
    };
    let world_fluids = sunrise_client::mod_content::register_world_fluids(&overlay.mods)?;
    register_block_entity_types(overlay.root.prepared(), &overlay.mods)?;
    if overlay.announce {
        sunrise_protocol::mod_list::set_local(overlay.announced);
    }
    // What every level.dat saved from here on records, and what a world is checked against
    // before it opens (`level_mods`).
    sunrise_client::save::level_mods::set_content_mods(Some(overlay.content_mods));
    if overlay.root.prepared() != root {
        log::info!(
            "Sunrise: content from {} mod(s) in {}",
            overlay.mods.len(),
            overlay.root.prepared().display()
        );
    }
    register_mob_effects(&overlay.mods, &sources.selected);
    resolve_particle_providers(overlay.root.source())?;
    mod_api::__private::resolve_effects(
        sunrise_client_core::potions::all_effects()
            .map(|def| sunrise_client_core::potions::namespaced(def.id)),
        |id| sunrise_client_core::potions::effect(id).map(|def| def.index),
    )
    .map_err(|errors| errors.join("\n"))?;
    mod_api::__private::validate_effect_part_owners(|id| {
        sunrise_client_core::potions::effect(id)?.owner
    })
    .map_err(|errors| errors.join("\n"))?;
    let behavior: serde_json::Value =
        sunrise_world::json_file::read(&overlay.root.prepared().join("block-behaviours.json"))?;
    let fluids = behavior["fluids"]
        .as_array()
        .ok_or("missing fluid registry")?;
    let mut fluids = fluids
        .iter()
        .map(|value| value.as_str().ok_or("invalid fluid registry entry"))
        .collect::<Result<Vec<_>, _>>()?;
    fluids.extend(world_fluids.iter().map(String::as_str));
    mod_api::__private::resolve_fluids(
        &fluids,
        &sunrise_world::fluid::FluidKind::ALL.map(|kind| kind.name()),
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    register_entity_types(&overlay.mods);
    register_menu_types(&overlay.mods)?;
    sunrise_client::mod_content::set_fluid_looks(&overlay.mods);
    // Starling: the mods adding blocks or items this process loaded, which a world's own set is
    // compared with (world_content.rs).
    super::world_content::set_running(
        (overlay.mods.iter())
            .filter(|m| !m.content.blocks.is_empty() || !m.content.items.is_empty())
            .map(|m| m.id.clone())
            .collect(),
    );
    // And every content mod it loaded: one adding nothing is no world's content (world_content.rs
    // `world_default`).
    super::world_content::set_loaded(overlay.mods.iter().map(|m| m.id.clone()).collect());
    if let Some(note) = &sources.note {
        note(None);
    }
    sunrise_client::mod_content::set_fluid_interactions(&overlay.mods);
    // Freeze ownership with the startup registries, before preparing Parts. Pack and data
    // reloads can change layers but cannot replace the registered blocks and block entity types.
    mod_api::__private::install_content_owners(content_owner_pairs(
        &overlay.mods,
        &sources.selected,
    ));
    Ok(overlay.root)
}

/// Publish the registry orders mods iterate from the accepted gameplay root.
/// Read spawn facts for those same entity keys before publishing any of them.
/// Native general types follow the prepared types in API order, not in the
/// protocol ID table; their facts come from the same frozen general catalog.
pub(in crate::game) fn register_iterated_registries(
    root: &Path,
) -> sunrise_client::Result<HashMap<String, SpawnerTypeFacts>> {
    use sunrise_protocol::registry::Builtin;
    let builtin = Builtin::load(&root.join("registries.json"))?;
    let ids = &builtin
        .get("minecraft:entity_type")
        .ok_or("missing entity type registry")?
        .names;
    let prepared = ids.iter().map(String::as_str).filter(|id| !id.is_empty());
    let general = super::general::general_type_catalog();
    let mut spawner_types = SpawnerTypeFacts::load_registered(root, prepared.clone())?;
    for slot in 0..general.len() {
        let (width, height) = general.dimensions(slot);
        spawner_types.insert(
            general.key(slot).to_owned(),
            SpawnerTypeFacts::general_entity(width, height),
        );
    }
    let rows = super::entity_type_rows::entity_type_rows(root);
    mod_api::__private::resolve_entity_types(prepared.chain(general.keys()), |id| {
        rows.contains(id)
            .then(|| rows.is_living(id))
            .or_else(|| general.slot(id).map(|_| false))
    })
    .map_err(|errors| errors.join("\n"))?;
    if let Some(ids) = builtin.get("minecraft:game_event") {
        mod_api::__private::resolve_game_events(ids.names.iter().map(String::as_str))
            .map_err(|errors| errors.join("\n"))?;
    }
    if let Some(ids) = builtin.get("minecraft:menu") {
        mod_api::__private::resolve_menus(ids.names.iter().map(String::as_str))
            .map_err(|errors| errors.join("\n"))?;
    }
    Ok(spawner_types)
}

fn register_block_entity_types(
    root: &std::path::Path,
    mods: &[sunrise_client::mod_content::ContentMod],
) -> sunrise_client::Result<()> {
    use sunrise_client::block_entity_types::{self, BlockEntityTypes};
    use sunrise_protocol::registry::Builtin;
    let path = Builtin::find(root).ok_or("registries.json not found")?;
    let builtin = Builtin::load(&path)?;
    let names = &builtin
        .get("minecraft:block_entity_type")
        .ok_or("missing block entity type registry")?
        .names;
    let membership = mods
        .iter()
        .flat_map(|m| &m.content.block_entity_types)
        .flat_map(|kind| {
            kind.blocks
                .iter()
                .map(|block| (block.clone(), kind.id.clone()))
        })
        .collect();
    let mut types = BlockEntityTypes::new(membership, names.clone());
    let blocks = builtin.get("minecraft:block");
    for block in blocks
        .into_iter()
        .flat_map(|ids| &ids.names)
        .filter(|name| name.starts_with("minecraft:"))
    {
        if let Some(kind) = sunrise_client::lan::chunks::block_entity_type(block) {
            types.template(block.clone(), block.clone(), kind.to_owned());
        }
    }
    for block in mods.iter().flat_map(|m| &m.content.blocks) {
        if let Some(kind) = sunrise_client::lan::chunks::block_entity_type(&block.template) {
            types.template(block.id.clone(), block.template.clone(), kind.to_owned());
        }
    }
    mod_api::__private::resolve_block_entity_types(types.names(), |id| types.key(id))
        .map_err(|errors| errors.join("\n"))?;
    types.bind_tick_parts(|id| {
        mod_api::__private::block_entity_type_listens(mod_api::HookId::BlockEntityTick, id)
    });
    let additions: Vec<(String, String)> = if let Some(current) = block_entity_types::current() {
        current
            .memberships()
            .map(|(block, kind)| (kind.to_owned(), block.to_owned()))
            .collect()
    } else {
        let keys: Vec<_> = types
            .names()
            .filter_map(mod_api::__private::block_entity_type_key)
            .collect();
        let mut additions = Vec::new();
        Game::native_block_entity_type_add_blocks(
            &RegistrationContext::new(),
            &keys,
            &mut additions,
        );
        additions
            .into_iter()
            .map(|(kind, block)| {
                Ok((
                    mod_api::__private::block_entity_type_name(kind)
                        .ok_or("unknown block entity type key")?
                        .to_owned(),
                    mod_api::__private::block_key_id(block)
                        .ok_or("unknown block key")?
                        .to_owned(),
                ))
            })
            .collect::<sunrise_client::Result<_>>()?
    };
    if additions.is_empty() {
        keep_or_install(types);
        return Ok(());
    }
    // Empty registration needs no block IDs or class data; requested additions do.
    let blocks = blocks.ok_or("missing block registry")?;
    let behavior: serde_json::Value =
        sunrise_world::json_file::read(&root.join("block-behaviours.json"))?;
    let classes: std::collections::BTreeMap<&str, Vec<&str>> = behavior["blocks"]
        .as_array()
        .ok_or("missing block behaviors")?
        .iter()
        .filter_map(|entry| {
            Some((
                entry["name"].as_str()?,
                entry["class"]
                    .as_array()?
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect(),
            ))
        })
        .collect();
    // Use the original valid sets for every addition; listener order cannot relax the constraint.
    let bases: std::collections::BTreeMap<String, String> = types
        .names()
        .filter_map(|kind| {
            let mut valid = types
                .memberships()
                .filter(|(_, k)| *k == kind)
                .filter_map(|(b, _)| classes.get(b));
            let first = valid.next()?;
            let rest: Vec<_> = valid.collect();
            first
                .iter()
                .find(|class| rest.iter().all(|chain| chain.contains(class)))
                .map(|class| (kind.to_owned(), (*class).to_owned()))
        })
        .collect();
    for (kind, block) in additions {
        let (kind, block) = (kind.as_str(), block.as_str());
        if blocks.id(block).is_none() {
            return Err(format!("unknown block {block}").into());
        }
        if let Some(base) = bases.get(kind)
            && !classes
                .get(block)
                .is_some_and(|chain| chain.contains(&base.as_str()))
        {
            return Err(format!("block {block} must inherit {base} to be valid for {kind}").into());
        }
        types.add_valid(block, kind)?;
    }
    keep_or_install(types);
    Ok(())
}

/// Starling: the table is installed once per process. A later build that differs (a resource
/// reload, like turning a shader pack on, with a content mod in the world) keeps the one in use,
/// as its refusal says a restart is what changes it, instead of failing the whole reload.
fn keep_or_install(types: sunrise_client::block_entity_types::BlockEntityTypes) {
    if let Err(error) = sunrise_client::block_entity_types::install(types) {
        log::warn!("Sunrise: {error}; keeping the block entity types in use until a restart");
    }
}

/// Manifest identity for the configuration-phase mod check.
fn announced_entry(
    id: &str,
    version: &str,
    side: mod_api::Side,
    runtime: Option<&sunrise_mod_package::manifest::RuntimeTable>,
) -> sunrise_protocol::mod_list::Entry {
    use mod_api::Side;
    use sunrise_protocol::mod_list;
    mod_list::Entry {
        id: id.to_owned(),
        version: version.to_owned(),
        side: match side {
            Side::Both => mod_list::Side::Both,
            Side::Server => mod_list::Side::Server,
            // `Side` is open to new kinds; until the list carries one, such a mod stays local.
            _ => mod_list::Side::Client,
        },
        runtime: runtime.map(sunrise_mod_package::manifest::RuntimeTable::digest),
    }
}

fn mob_effect_entries<'a>(
    mods: &'a [sunrise_client::mod_content::ContentMod],
    selected: &'a [sunrise_mod_package::selection::SelectedPackage],
) -> Vec<(&'a str, &'a sunrise_mod_package::content::MobEffect)> {
    let mut effects: Vec<_> = mods
        .iter()
        .flat_map(|m| {
            let owner = content_layer_owner(&m.id, selected);
            m.content.effects.iter().map(move |effect| (owner, effect))
        })
        .collect();
    effects.sort_by(|(_, a), (_, b)| a.id.cmp(&b.id));
    effects
}

/// The mods' mob effects join vanilla's (`brewing::effect`), once the root
/// holding their network ids is built. The first root that declares effects
/// sets them, until restart: a pack reload after a Mods switch builds a root
/// with others. A root declaring none freezes nothing, and a game without
/// mob effects reads the same empty list either way.
fn register_mob_effects(
    mods: &[sunrise_client::mod_content::ContentMod],
    selected: &[sunrise_mod_package::selection::SelectedPackage],
) {
    use sunrise_client::brewing::{self, Category};
    use sunrise_mod_package::content::EffectCategory;
    let effects = mob_effect_entries(mods, selected);
    if effects.is_empty() {
        return;
    }
    let ids: Vec<_> = effects
        .iter()
        .map(|(owner, e)| (e.id.clone(), (*owner).to_owned()))
        .collect();
    let set = brewing::register_mod_effects(effects.into_iter().map(|(owner, e)| {
        let category = match e.category {
            EffectCategory::Beneficial => Category::Beneficial,
            EffectCategory::Harmful => Category::Harmful,
            EffectCategory::Neutral => Category::Neutral,
        };
        (
            e.id.clone(),
            e.color,
            category,
            e.sort_order,
            owner.to_owned(),
        )
    }));
    if !set
        && brewing::mod_effects()
            .iter()
            .map(|e| (e.id, e.owner))
            .ne(ids
                .iter()
                .map(|(id, owner)| (id.as_str(), Some(owner.as_str()))))
    {
        log::warn!("Sunrise: the mods' mob effects changed; restart the game to use them");
    }
}

/// Content and general types share the entity-type key space even though their definitions
/// enter through different paths. Refuse the collision before building the content root.
fn validate_entity_type_families(
    mods: &[sunrise_client::mod_content::ContentMod],
    general: &super::general::GeneralTypeCatalog,
) -> Result<(), String> {
    for package in mods {
        for entity in &package.content.entity_types {
            if general.slot(&entity.id).is_some() {
                return Err(format!(
                    "mod {} declares entity type {} as both a content and a general entity type",
                    package.id, entity.id,
                ));
            }
        }
    }
    Ok(())
}

/// the mods' entity types' templates, which the tracker reads their settings
/// from, and their thrown types, as their mob effects: set once, from the
/// startup root. Living types are mobs, loaded with vanilla's from the
/// content root's `entity-types.json`.
fn register_entity_types(mods: &[sunrise_client::mod_content::ContentMod]) {
    use sunrise_client::{lan::tracking, projectiles};
    let mut types = sunrise_client::mod_content::entity_types(mods);
    let templates = types.iter().map(|t| (t.id.clone(), t.template.clone()));
    let templates_set = tracking::register_templates(templates);
    let templates_changed = !templates_set
        && tracking::templates()
            .iter()
            .copied()
            .ne(types.iter().map(|t| (t.id.as_str(), t.template.as_str())));
    types.retain(|t| t.item.is_some());
    let set = projectiles::register_thrown_types(
        types
            .iter()
            .filter_map(|t| Some((t.id.clone(), t.item.clone()?, t.damage))),
    );
    let thrown_changed = !set
        && projectiles::thrown_types()
            .iter()
            .map(|t| t.id)
            .ne(types.iter().map(|t| t.id.as_str()));
    if templates_changed || thrown_changed {
        log::warn!("Sunrise: the mods' entity types changed; restart the game to use them");
    }
}

/// the mods' menu types, as their entity types: set once, from the startup
/// root.
fn register_menu_types(
    mods: &[sunrise_client::mod_content::ContentMod],
) -> sunrise_client::Result<()> {
    use sunrise_client::containers;
    let menus = sunrise_client::mod_content::menu_types(mods);
    menu_books::validate_registered(&menus.iter().map(|&(menu, _)| menu).collect::<Vec<_>>())?;
    let set = containers::register_mod_menu_types(menus.iter().map(|&(m, owner)| {
        let results = m.results.iter().map(|&r| usize::from(r)).collect();
        let inputs = m.inputs.iter().copied().map(usize::from).collect();
        let crafting = m.crafting.map(|c| containers::ModCrafting {
            grid: c.grid.into(),
            size: c.size.into(),
            result: c.result.into(),
        });
        let links = m.links.map(|l| containers::ModLinks {
            from: l.from.into(),
            count: l.count.into(),
        });
        (
            m.id.clone(),
            owner.to_owned(),
            m.slots.into(),
            m.data_slots,
            results,
            inputs,
            crafting,
            links,
            m.player_inventory,
            m.owned_slots.clone(),
        )
    }));
    if !set
        && containers::mod_menu_types()
            .iter()
            .map(|m| m.id)
            .ne(menus.iter().map(|(m, _)| m.id.as_str()))
    {
        log::warn!("Sunrise: the mods' menu types changed; restart the game to use them");
    }
    Ok(())
}

/// The embedded inventory, formatted for the crash report.
fn native_mods_detail(built: &[mod_api::__private::native::BuiltMod]) -> Option<String> {
    let list = built
        .iter()
        .map(|m| format!("{} {}", m.id, m.version))
        .collect::<Vec<_>>()
        .join(", ");
    (!list.is_empty()).then_some(list)
}

/// Whether a content mod's files can switch with a world open (`mod_content::live_switch`);
/// `None` when its content can't be read.
type Verdict = Option<std::result::Result<(), &'static str>>;

/// Each mod's [`Verdict`] by its path and modification time. Reading a mod's content walks and
/// checks every file (seconds for a large pack), and a start read each mod three times: the
/// content root, then the window's and the server thread's `start_mods`.
/// shortcut: a file edited inside an unpacked mod folder keeps the folder's time; `--dev-mod`
/// folders are never remembered, a player's are replaced whole by the launcher.
static VERDICTS: std::sync::Mutex<
    std::collections::BTreeMap<(PathBuf, std::time::SystemTime), Verdict>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());

fn verdict_key(path: &std::path::Path) -> Option<(PathBuf, std::time::SystemTime)> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    Some((path.to_owned(), modified))
}

/// Remembers the verdict of `content`, read from `path` as [`sunrise_mod_package::Content::read`] does.
fn remember_verdict(path: &std::path::Path, content: &sunrise_mod_package::Content) {
    if let Some(key) = verdict_key(path) {
        let verdict = Some(sunrise_client::mod_content::live_switch(content));
        VERDICTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, verdict);
    }
}

/// The mod at `path`'s [`Verdict`], read once per version of it (never remembered for `dev`).
fn verdict(path: &std::path::Path, manifest: &sunrise_mod_package::Manifest, dev: bool) -> Verdict {
    let key = if dev { None } else { verdict_key(path) };
    if let Some(key) = &key
        && let Some(verdict) = VERDICTS.lock().unwrap_or_else(|e| e.into_inner()).get(key)
    {
        return *verdict;
    }
    let verdict = sunrise_mod_package::Content::read(path, manifest)
        .ok()
        .map(|content| sunrise_client::mod_content::live_switch(&content));
    if let Some(key) = key {
        VERDICTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, verdict);
    }
    verdict
}

/// Notes a content mod's id, and whether its files can switch with a world
/// open (`mod_content::live_switch`).
fn live_switch_of(
    path: &std::path::Path,
    package: &sunrise_mod_package::Package,
    dev: bool,
    content_ids: &mut std::collections::BTreeSet<String>,
    live_ids: &mut std::collections::BTreeSet<String>,
) {
    if package.is_native() {
        return;
    }
    let id = &package.manifest.id;
    content_ids.insert(id.clone());
    if package.manifest.served_by.is_some() {
        log::debug!("Sunrise: mod {id} switches on a restart: served by a native mod");
        return;
    }
    match verdict(path, &package.manifest, dev) {
        // Jar layers can declare worldgen that Content.files does not list.
        Some(Ok(())) if package.manifest.jar.is_empty() => {
            live_ids.insert(id.clone());
        }
        Some(Err(why)) => log::debug!("Sunrise: mod {id} switches on a restart: {why}"),
        _ => {}
    }
}

/// Installed native mods not compiled into this executable get an explanatory row.
fn not_built(
    dir: &std::path::Path,
    built: &[mod_api::__private::native::BuiltMod],
    rows: &[Installed],
) -> Vec<Installed> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    // Starling's `.hnmod` (the catalog's and Open with's, as in 0.8.0) and Sunrise's `.smod`.
    paths.retain(|p| p.extension().is_some_and(|e| e == "hnmod" || e == "smod"));
    paths.sort();
    let mut out: Vec<Installed> = Vec::new();
    for path in paths {
        let bundle = match sunrise_mod_package::index::native::Bundle::read(&path) {
            Ok(bundle) => bundle,
            // Said on its row: a file left out without a word looks like the folder isn't read.
            Err(e) => {
                let title = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let shown = |row: &Installed| row.id.is_empty() && row.title == title;
                if !rows.iter().chain(&out).any(shown) {
                    out.push(Installed {
                        title,
                        id: String::new(),
                        description: String::new(),
                        problem: Some(format!("Not a native mod Starling can read: {e}")),
                        dev: false,
                        native: None,
                    });
                }
                continue;
            }
        };
        let (id, version) = (bundle.manifest.id.clone(), bundle.manifest.version.clone());
        let listed = rows.iter().chain(&out).any(|row| row.id == id);
        if !listed && !built.iter().any(|b| b.id == id) {
            out.push(Installed {
                title: format!("{} {version}", display_name(&id)),
                id,
                description: String::new(),
                problem: Some(mod_list::NOT_BUILT.to_owned()),
                dev: false,
                native: None,
            });
        }
    }
    out
}

pub(super) struct PreparedLoopContext {
    game: Arc<sunrise_world::native_load_context::LoopContext>,
    audio: Option<sunrise_world::native_load_context::LoopContext>,
    /// A server's context for the features it generates (`alter_ground`).
    worldgen: Option<sunrise_world::native_load_context::WorldgenContext>,
}

/// A native mod's name from its id, which is all its bundle says of it (the name is in its code):
/// `carry_block` is "Carry block", as the game built with it names `Wrench` "Wrench".
fn display_name(id: &str) -> String {
    let words = id.replace(['_', '-'], " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// What a live content mod switch changes on the server (`Game::apply_mods_reload`).
pub(super) struct ModsReload {
    root: PathBuf,
    data_source: sunrise_assets::resource_source::ResourceSource,
    data_changes: sunrise_assets::resource_source::ResourceSource,
    additional_data: Arc<[Arc<sunrise_client::datapack::DataPack>]>,
    enabled: std::collections::BTreeSet<String>,
    selected: Arc<[sunrise_mod_package::selection::SelectedPackage]>,
    refused: Arc<Mutex<Vec<MissingJar>>>,
}

impl Game<'_> {
    #[cfg(test)]
    pub(super) fn initialize_mod_runtime_for_tests() {
        static INITIALIZED: std::sync::Once = std::sync::Once::new();
        INITIALIZED.call_once(|| {
            super::api_values::install_test_platform();
            mod_platform::__private::initialize()
                .expect("engine test runtime initializes before mod dispatch");
        });
    }

    /// Publish the initial generation's captured settings to its loop readers.
    /// An integrated server has a distinct origin and captures its own inputs.
    pub(super) fn refresh_loop_context(&mut self) -> sunrise_client::Result<()> {
        let role = self.options.execution_role;
        let load = self.res.core.item_data.context();
        let context = if load.origin() == role.data_load_origin() {
            self.loop_context_readers(load.captured_loop_context(role.loop_side()))
        } else {
            self.prepare_loop_context(&self.options.resources)?
        };
        self.install_loop_context(context);
        Ok(())
    }

    pub(super) fn prepare_loop_context(
        &self,
        resources: &std::path::Path,
    ) -> sunrise_client::Result<PreparedLoopContext> {
        let side = self.options.execution_role.loop_side();
        let inventory = sunrise_mod_package::enabled::EnabledMods::read(resources)?;
        let ids: Vec<_> = if self.is_logical_server() {
            inventory.server_ids().collect()
        } else {
            inventory.client_ids().collect()
        };
        let item_data = self.res.core.item_data.context();
        Ok(self.loop_context_readers(item_data.loop_context(side, ids.iter().copied())))
    }

    fn loop_context_readers(
        &self,
        context: sunrise_world::native_load_context::LoopContext,
    ) -> PreparedLoopContext {
        let game = Arc::new(context);
        let audio = self
            .options
            .execution_role
            .is_client()
            .then(|| game.as_ref().clone());
        let worldgen = self.is_logical_server().then(|| game.worldgen_context());
        PreparedLoopContext {
            game,
            audio,
            worldgen,
        }
    }

    pub(super) fn install_loop_context(&mut self, context: PreparedLoopContext) {
        self.protect_condition_root();
        self.loop_context = context.game;
        self.refresh_recipe_property_context();
        self.setting_overrides.borrow_mut().clear();
        if let Some(audio) = context.audio {
            self.audio.set_loop_context(audio);
        }
        if let Some(worldgen) = context.worldgen {
            sunrise_world::native_worldgen_hooks::publish_context(Arc::new(worldgen));
        }
    }

    /// Stage a fresh root when admission changes or registered conditions need reevaluation.
    /// The caller publishes it after every server reload listener prepared.
    pub(super) fn stage_enabled_inventory(
        &self,
    ) -> sunrise_client::Result<Option<sunrise_assets::resource_source::ResourceRoot>> {
        let previous = sunrise_mod_package::enabled::EnabledMods::read(&self.options.resources)?;
        let next = self
            .mods
            .enabled_inventory(|id| previous.client_ids().any(|loaded| loaded == id));
        if previous == next && !self.condition_root_needs_reload() {
            return Ok(None);
        }
        let base = self
            .mods
            .content_base
            .get()
            .cloned()
            .unwrap_or_else(|| self.vanilla_root());
        let jars = sunrise_client::logging::game_home().join("jars");
        let refused = self.mods.missing_jar_refusals(&jars)?;
        let overlay = content_overlay_with_refusals(
            &base,
            &self.mods,
            &jars,
            self.mods.startup_admission.as_deref(),
            &refused,
        )?;
        Ok(Some(overlay.root))
    }

    /// The Mods page opened: native mods copied into the folder since the game started (the Files
    /// app, a computer) get their rows, as the catalog's do. Only a phone compiles them.
    #[cfg(target_os = "android")]
    pub(super) fn list_copied_native_mods(&mut self) {
        self.list_new_native_mods();
    }
    #[cfg(not(target_os = "android"))]
    pub(super) fn list_copied_native_mods(&mut self) {}

    /// Installed mod `i` was turned on or off on the Mods page: a native one changes which build
    /// the next start runs (`native_builds.rs`). Only a phone has builds.
    #[cfg(target_os = "android")]
    pub(super) fn mod_toggled(&mut self, i: usize) {
        self.native_mod_toggled(i);
    }
    #[cfg(not(target_os = "android"))]
    pub(super) fn mod_toggled(&mut self, _i: usize) {}

    /// A button of the Mods page's game with native mods (`native_builds.rs`).
    #[cfg(target_os = "android")]
    pub(super) fn native_game_button(&mut self, act: mod_list::NativeGameAct) {
        self.native_game_act(act);
    }
    #[cfg(not(target_os = "android"))]
    pub(super) fn native_game_button(&mut self, _act: mod_list::NativeGameAct) {}

    /// A native mod the catalog just put in the folder: its row on the Mods page, with Compile.
    #[cfg(target_os = "android")]
    pub(super) fn list_new_native_mods(&mut self) {
        let native_dir = Self::game_dir().join(mod_list::NATIVE_DIR);
        let unbuilt = not_built(
            &native_dir,
            self.mods.native.built(),
            &self.settings.mods.installed,
        );
        self.settings.mods.installed.extend(unbuilt);
        self.offer_compile();
    }

    /// Lists the installed mods on the Mods page. Content mods load with the resources
    /// (`content_root`); native ones are built into the game (`native_hooks.rs`). Unsupported
    /// files get the same unreadable-file row as any other invalid package.
    pub(super) fn start_mods(&mut self) {
        let dir = Self::game_dir().join(mod_list::DIR);
        let mut paths = sunrise_mod_package::find(&dir);
        paths.retain(|p| !self.mods.dev.contains(p));
        paths.extend(self.mods.dev.iter().cloned());
        let (mut content_ids, mut live_ids) = Default::default();
        let mut info = std::collections::BTreeMap::new();
        self.settings.mods.installed = paths
            .iter()
            .map(|path| {
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                let package = match sunrise_mod_package::Package::read(path) {
                    Ok(package) => package,
                    Err(e) => {
                        log::warn!("Sunrise: mod {file} can't be read: {e}");
                        return Installed {
                            id: String::new(),
                            title: file.into_owned(),
                            description: String::new(),
                            problem: Some("Made for another version: needs an update".to_owned()),
                            dev: false,
                            native: None,
                        };
                    }
                };
                let m = &package.manifest;
                live_switch_of(
                    path,
                    &package,
                    self.mods.dev.contains(path),
                    &mut content_ids,
                    &mut live_ids,
                );
                info.insert(
                    m.id.clone(),
                    mod_list::ModInfo::of(m, package.icon.as_ref()),
                );
                Installed {
                    id: m.id.clone(),
                    title: format!("{} {}", m.name, m.version),
                    description: m.description.clone(),
                    problem: None,
                    dev: self.mods.dev.contains(path),
                    native: None,
                }
            })
            .collect();
        for (id, info) in &mut info {
            info.warnings = mod_api::__private::registration_warnings(id)
                .into_iter()
                .map(|w| (w.key().to_owned(), w.args().to_vec()))
                .collect();
        }
        self.settings.mods.info = info;
        (self.mods.content_ids, self.mods.live_ids) = (content_ids, live_ids);
        if self.mods.applied.is_none() {
            self.mods.applied = Some(self.settings.mods.enabled.clone());
        }
        if self.mods.code_switches.is_none() {
            self.mods
                .capture_code_switches(&self.settings.mods.disabled);
        }
        // A mod with a native version built in says so instead, and isn't news.
        self.list_native_mods();
        #[cfg(target_os = "android")]
        self.list_native_builds_row();
        self.sync_native_switches();
        let built = self.mods.native.built();
        let built_mods = native_mods_detail(built);
        if let Some(list) = built_mods {
            mod_api::content::crash::set_detail("Native Mods", list);
        }

        let native_dir = Self::game_dir().join(mod_list::NATIVE_DIR);
        let unbuilt = not_built(&native_dir, built, &self.settings.mods.installed);
        self.settings.mods.installed.extend(unbuilt);
        self.mods.report_refusals(&mut self.settings.mods.installed);
        #[cfg(target_os = "android")]
        self.offer_compile();
        self.settings
            .mods
            .apply_dependency_notices(&self.mods.load_notices.lock().unwrap());
    }
    /// The content mods the resources load with: the ones applied, which a
    /// switch that needs a restart doesn't change.
    pub(super) fn applied_mods(&self) -> std::collections::BTreeSet<String> {
        self.mods
            .applied
            .clone()
            .unwrap_or_else(|| self.settings.mods.enabled.clone())
    }
    /// The player turned content mods on or off (Options > Mods, or an
    /// install from Browse Mods): those with only assets and data apply now
    /// (docs/design/mod-hotload.md), through the resource reload and then the
    /// server's data reload (`mods_reloaded`); those that add blocks, items or
    /// worldgen wait for a restart, and the Mods page says so.
    ///
    /// Starling: with `apply` false only the Mods page's note follows the switches. Options apply
    /// them as they close, in one reload: a reload for every switch put the loading screen up
    /// between two taps, and the switches made under it were lost.
    pub(super) fn switch_mods(&mut self, apply: bool) {
        let Some(applied) = self.mods.applied.clone() else {
            return;
        };
        let wanted = self.settings.mods.enabled.clone();
        // Ordinary native mods' `mod.<id>` lines switch live (`sync_native_switches`);
        // a serving host bound to active or withheld `served_by` packages, and any content
        // mod with blocks, items, worldgen, or `served_by`, keep their startup state until
        // a restart.
        let content = &self.mods.content_ids;
        let mut next = wanted.clone();

        let mut waiting = Vec::new();
        for id in applied.symmetric_difference(&wanted) {
            if !content.contains(id) || self.mods.live_ids.contains(id) {
                continue;
            }
            log::info!("Sunrise: mod {id} needs a restart to switch");
            waiting.push(id.clone());
            if applied.contains(id) {
                next.insert(id.clone());
            } else {
                next.remove(id);
            }
        }
        let (_, host_waiting) = self
            .mods
            .reconcile_code_switches(&self.settings.mods.disabled);
        for id in host_waiting {
            log::info!("Sunrise: serving host mod {id} needs a restart to switch");
            if !waiting.contains(&id) {
                waiting.push(id);
            }
        }
        self.ui.options.mods_note = (!waiting.is_empty())
            .then(|| lang::tr("sunrise.mods.switch.restart", &[&waiting.join(", ")]));
        if !apply {
            return;
        }
        let changed = applied
            .symmetric_difference(&next)
            .any(|id| content.contains(id));
        if !changed {
            self.mods.applied = Some(next);
            return;
        }
        log::info!(
            "Sunrise: content mods switched live: {:?} -> {:?}",
            applied,
            next
        );
        self.mods.applied = Some(next);
        self.mods.live = Some(std::time::Instant::now());
        self.reload_packs();
    }
    /// A live switch's resources applied: the server reloads its data packs
    /// from the new root, as `/reload` does (recipes, tags, loot tables,
    /// advancements, functions), and sends them to its players.
    pub(super) fn mods_reloaded(&mut self, applied: bool) {
        let Some(started) = self.mods.live.take() else {
            return;
        };
        let resources_ms = started.elapsed().as_millis();
        if !applied {
            return;
        }
        let reload = ModsReload {
            root: self.options.resources.clone(),
            data_source: self.pack.data_source.clone(),
            data_changes: self.pack.data_changes.clone(),
            additional_data: self.pack.additional_data.clone(),
            enabled: self.applied_mods(),
            selected: self.mods.selected.clone(),
            refused: self.mods.refused.clone(),
        };
        log::info!("Sunrise: content mods switched live: resources reloaded in {resources_ms} ms");
        if let Some(server) = &self.integrated {
            server.tell(OwnerRequest::ReloadMods(Box::new(reload)));
        } else if self.remote.is_none() {
            // Starling: with no server yet (the title screen) the reload runs on this game, where
            // the switches are the player's: they keep what the player chose, a mod waiting for a
            // restart or a switch made during the reload too, not only what loaded.
            let wanted = self.settings.mods.enabled.clone();
            self.apply_mods_reload(reload);
            self.settings.mods.enabled = wanted;
        }
    }
    /// The server's side of a live switch: the data packs reload from the new root.
    pub(super) fn apply_mods_reload(&mut self, reload: ModsReload) {
        let at = std::time::Instant::now();
        self.options.resources = reload.root;
        self.pack.data_source = reload.data_source;
        self.pack.data_changes = reload.data_changes;
        self.pack.additional_data = reload.additional_data;
        self.settings.mods.enabled = reload.enabled.clone();
        self.mods.applied = Some(reload.enabled);
        self.mods.selected = reload.selected;
        self.mods.refused = reload.refused;
        self.mods.data_switched = true;
        if self.world_root.as_os_str().is_empty() {
            return;
        }
        if let Err(error) = self.reload_datapacks(true, &self.level.enabled_features.clone()) {
            log::warn!(
                "Sunrise: content mods data reload failed: {}",
                error.diagnostic()
            );
            return;
        }
        log::info!(
            "Sunrise: content mods switched live: server data reloaded in {} ms",
            at.elapsed().as_millis()
        );
    }
    /// Apply the player's switches at the canonical native dispatch boundary.
    pub(super) fn sync_native_switches(&self) {
        let (effective, _) = self
            .mods
            .reconcile_code_switches(&self.settings.mods.disabled);
        self.instance_groups
            .borrow_mut()
            .retire_disabled(&effective);
        self.mods.apply_code_switches(&effective);
    }
}

/// Mob types loaded from a scratch root whose `entity-types.json` has a
/// zombie, a villager, a wandering trader, a wolf, a cat and a parrot, and the
/// rows a content root writes for a mod's types (`mod_content`): `ruby:ghoul`
/// (a zombie with 40 health), `ruby:settler` (a villager as it is),
/// `ruby:merchant` (a wandering trader), the tameable `ruby:hound`,
/// `ruby:tabby` and `ruby:macaw` (a wolf, a cat and a parrot) and `ruby:bolt`
/// (a thrown snowball, not a mob).
#[cfg(test)]
pub(super) fn living_mod_types()
-> std::collections::HashMap<String, std::sync::Arc<sunrise_client::mobs::MobType>> {
    let zombie =
        r#""width":0.6,"height":1.95,"eye_height":1.74,"category":"monster","fire_immune":false"#;
    let villager =
        r#""width":0.6,"height":1.95,"eye_height":1.62,"category":"misc","fire_immune":false"#;
    let trader =
        r#""width":0.6,"height":1.95,"eye_height":1.62,"category":"creature","fire_immune":false"#;
    let wolf =
        r#""width":0.6,"height":0.85,"eye_height":0.68,"category":"creature","fire_immune":false"#;
    let cat =
        r#""width":0.6,"height":0.7,"eye_height":0.35,"category":"creature","fire_immune":false"#;
    let parrot =
        r#""width":0.5,"height":0.9,"eye_height":0.54,"category":"creature","fire_immune":false"#;
    let table = format!(
        r#"{{"minecraft:zombie":{{{zombie},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.23000000417232513}}}},
        "minecraft:villager":{{{villager},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.5}}}},
        "minecraft:wandering_trader":{{{trader},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.7}}}},
        "minecraft:wolf":{{{wolf},"attributes":{{"minecraft:max_health":8.0,"minecraft:movement_speed":0.30000001192092896}}}},
        "minecraft:cat":{{{cat},"attributes":{{"minecraft:max_health":10.0,"minecraft:movement_speed":0.30000001192092896}}}},
        "minecraft:parrot":{{{parrot},"attributes":{{"minecraft:max_health":6.0,"minecraft:movement_speed":0.20000000298023224}}}},
        "minecraft:snowball":{{"width":0.25,"height":0.25,"eye_height":0.0,"category":"misc","fire_immune":false}},
        "ruby:ghoul":{{{zombie},"attributes":{{"minecraft:max_health":40.0,"minecraft:movement_speed":0.23000000417232513}},"template":"minecraft:zombie"}},
        "ruby:settler":{{{villager},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.5}},"template":"minecraft:villager"}},
        "ruby:merchant":{{{trader},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.7}},"template":"minecraft:wandering_trader"}},
        "ruby:hound":{{{wolf},"attributes":{{"minecraft:max_health":8.0,"minecraft:movement_speed":0.30000001192092896}},"template":"minecraft:wolf"}},
        "ruby:tabby":{{{cat},"attributes":{{"minecraft:max_health":10.0,"minecraft:movement_speed":0.30000001192092896}},"template":"minecraft:cat"}},
        "ruby:macaw":{{{parrot},"attributes":{{"minecraft:max_health":6.0,"minecraft:movement_speed":0.20000000298023224}},"template":"minecraft:parrot"}},
        "ruby:bolt":{{"width":0.25,"height":0.25,"eye_height":0.0,"category":"misc","fire_immune":false,"template":"minecraft:snowball"}}}}"#
    );
    // A folder per call: tests run in parallel in one process, and each removes its own.
    static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "sunrise-living-mod-types-{}-{call}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("entity-types.json"), table).unwrap();
    // No attribute catalog: the rows' values stay as written.
    let attributes = &sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
        "data_version": sunrise_world::DATA_VERSION,
        "attributes": [],
        "entities": {},
    }))
    .unwrap();
    let types = sunrise_client::mobs::MobTypes::load_for_server(&root, attributes).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    types
        .types
        .into_iter()
        .map(|(id, t)| (id, std::sync::Arc::new(t)))
        .collect()
}

/// The block states native mods add to vanilla POI types (`native_extend_poi_types`),
/// resolved against `registry` once its block definitions are published and before any
/// world's POI tables are built; collected again when the resources reload. Each refusal
/// is logged once.
pub(super) fn extend_poi_types(registry: &sunrise_world::registry::BlockRegistry) {
    let (additions, refusals) = collect_poi_extensions(registry);
    for refusal in refusals
        .into_iter()
        .chain(sunrise_world::poi::extend(additions))
    {
        log::warn!("Sunrise: POI type extension refused: {refusal}");
    }
}

/// Ask the mods, then turn each predicate into the complete states of `registry` it
/// selects, in registration order. A tag predicate is refused here: additions name blocks.
pub(in crate::game) fn collect_poi_extensions(
    registry: &sunrise_world::registry::BlockRegistry,
) -> (Vec<sunrise_world::poi::PoiExtension>, Vec<String>) {
    let Some(metadata) = mod_api::__private::block_definitions() else {
        return (Vec::new(), Vec::new());
    };
    let mut extensions = mod_api::PoiTypeExtensionRegistry::new(metadata);
    Game::native_extend_poi_types(&RegistrationContext::new(), &mut extensions);
    let mut additions = Vec::new();
    let mut refusals = Vec::new();
    for (poi, predicate) in extensions.into_entries() {
        let record = mod_api::StatePredicateRecord::from(&predicate);
        let block = match record.target {
            mod_api::StatePredicateTarget::Block(block) => block,
            mod_api::StatePredicateTarget::Tag(_) => {
                refusals.push(format!(
                    "{poi} asked for a block tag's states; an addition names a block"
                ));
                continue;
            }
        };
        // The predicate was built on this metadata, so its block is known.
        let Ok(name) = mod_api::BlockMetadata::name(metadata, block) else {
            continue;
        };
        for index in 0..registry.len() {
            let state = registry
                .state(sunrise_world::registry::BlockStateId(index as u32))
                .expect("registry index is in bounds");
            if state.name == name
                && record.constraints.iter().all(|constraint| {
                    state.properties.get(constraint.name.as_str()) == Some(&constraint.value)
                })
            {
                additions.push(sunrise_world::poi::PoiExtension {
                    state: state.clone(),
                    poi: poi.to_string(),
                });
            }
        }
    }
    (additions, refusals)
}

/// The fluid contents native mods give their cauldron blocks
/// (`native_register_cauldron_fluid_content`), resolved against `registry` once its block
/// definitions are published, in one table with vanilla's water and lava, which the fluid
/// capability reads; collected again when the resources reload. Each refusal is logged once.
pub(super) fn register_cauldron_fluid_contents(registry: &sunrise_world::registry::BlockRegistry) {
    let (contents, refusals) = collect_cauldron_fluid_contents(registry);
    for refusal in refusals {
        log::warn!("Sunrise: cauldron fluid content refused: {refusal}");
    }
    crate::game::machines::cauldron_contents::install(contents);
}

/// Ask the mods, then resolve each registration against `registry` into the table after
/// vanilla's contents, in registration order: a block or fluid already in the table, an
/// unknown block or fluid, a level property the block lacks or that isn't an integer, and a
/// total of 0 or one the levels don't divide evenly are refused, with their reasons.
pub(in crate::game) fn collect_cauldron_fluid_contents(
    registry: &sunrise_world::registry::BlockRegistry,
) -> (
    crate::game::machines::cauldron_contents::CauldronContents,
    Vec<String>,
) {
    use crate::game::machines::cauldron_contents::{CauldronContent, CauldronContents};
    let mut registrations = mod_api::CauldronFluidContentRegistry::default();
    Game::native_register_cauldron_fluid_content(&RegistrationContext::new(), &mut registrations);
    let mut contents = CauldronContents::vanilla();
    let mut refusals = Vec::new();
    for registration in registrations.into_entries() {
        let resolved = match (
            mod_api::__private::block_key_id(registration.block),
            mod_api::__private::fluid_id(registration.fluid),
        ) {
            (None, _) => Err("an unknown block was given a cauldron fluid content".to_owned()),
            (Some(block), None) => Err(format!("{block} was given an unknown fluid")),
            (Some(block), Some(fluid)) => CauldronContent::resolve(
                registry,
                block,
                fluid,
                registration.total,
                registration.level.as_deref(),
            ),
        };
        if let Err(refusal) = resolved.and_then(|content| contents.add(content)) {
            refusals.push(refusal);
        }
    }
    (contents, refusals)
}

/// Registered callbacks indexed directly by the slots resolved in enchantment content.
pub(in crate::game) static ENCHANTMENT_DAMAGE_EFFECTS: std::sync::OnceLock<
    Vec<std::sync::Arc<dyn mod_api::EnchantmentDamageEffect>>,
> = std::sync::OnceLock::new();

pub(super) fn register_enchantment_damage_effects() {
    let mut effects = mod_api::EnchantmentDamageRegistry::default();
    Game::native_register_enchantment_damage_effects(
        &mod_api::RegistrationContext::new(),
        &mut effects,
    );
    let (ids, callbacks): (Vec<_>, Vec<_>) = effects
        .into_entries()
        .into_iter()
        .map(|(id, effect)| (id.to_string(), effect))
        .unzip();
    sunrise_items::enchant::register_damage_effects(ids);
    assert!(
        ENCHANTMENT_DAMAGE_EFFECTS.set(callbacks).is_ok(),
        "damage effects already registered"
    );
}

#[cfg(test)]
pub(in crate::game) mod tests {
    #[test]
    fn entity_registry_and_spawner_facts_cannot_publish_different_generations() {
        use super::register_iterated_registries;
        use std::fs;

        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let first = tempfile::tempdir().unwrap();
        let next = tempfile::tempdir().unwrap();
        for (root, entity) in [
            (first.path(), "minecraft:arrow"),
            (next.path(), "minecraft:egg"),
        ] {
            fs::write(
                root.join("registries.json"),
                format!(
                    r#"{{"minecraft:entity_type":{{"entries":{{"{entity}":{{"protocol_id":0}}}}}}}}"#
                ),
            )
            .unwrap();
            fs::write(
                root.join("entity-types.json"),
                format!(r#"{{"{entity}":{{"category":"misc","width":0.25,"height":0.25}}}}"#),
            )
            .unwrap();
        }

        let facts = register_iterated_registries(first.path()).unwrap();
        assert!(facts.contains_key("minecraft:arrow"));
        let arrow = mod_api::__private::resolved_entity_type_key("minecraft:arrow").unwrap();
        // The new reports are internally consistent. They still must not replace the
        // spawner facts while the API continues to expose the old registry's keys.
        let Err(error) = register_iterated_registries(next.path()) else {
            panic!("published a second set of spawner facts against the frozen entity keys");
        };
        assert!(error.to_string().contains("frozen entity-type registry"));
        assert_eq!(
            mod_api::__private::resolved_entity_type_key("minecraft:arrow"),
            Some(arrow)
        );
        assert!(mod_api::__private::resolved_entity_type_key("minecraft:egg").is_none());
    }

    /// A mod's live-switch verdict is read once per version of the mod: a later look finds
    /// it without reading the files again, a `--dev-mod` is always read, and the mod
    /// replaced (a new modification time) is read again.
    #[test]
    fn a_mods_live_switch_verdict_is_read_once_per_version_of_it() {
        let dir = std::env::temp_dir().join(format!("sunrise-verdict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ruby = dir.join("ruby");
        std::fs::create_dir_all(ruby.join("assets/ruby/lang")).unwrap();
        std::fs::create_dir_all(ruby.join("data/ruby")).unwrap();
        std::fs::write(ruby.join("assets/ruby/lang/en_us.json"), "{}").unwrap();
        let manifest = sunrise_mod_package::Manifest::parse(
            "id = \"ruby\"\nname = \"Ruby\"\nversion = \"0.1.0\"\napi = \"0.2\"\n\
             side = \"both\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        assert_eq!(verdict(&ruby, &manifest, false), Some(Ok(())));
        // Worldgen added below `data/ruby`: the mod folder's own time stays.
        let worldgen = ruby.join("data/ruby/worldgen/configured_feature");
        std::fs::create_dir_all(&worldgen).unwrap();
        std::fs::write(worldgen.join("ore.json"), "{}").unwrap();
        let switches_on_restart = Some(Err("sunrise.mods.switch.worldgen"));
        assert_eq!(verdict(&ruby, &manifest, true), switches_on_restart);
        assert_eq!(verdict(&ruby, &manifest, false), Some(Ok(())));
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        let mut folder = std::fs::OpenOptions::new();
        folder.read(true);
        // Windows opens a folder only with FILE_FLAG_BACKUP_SEMANTICS, and sets its time only
        // with write access.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            folder.write(true).custom_flags(0x0200_0000);
        }
        folder.open(&ruby).unwrap().set_modified(later).unwrap();
        assert_eq!(verdict(&ruby, &manifest, false), switches_on_restart);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn native_declaration_diagnostics_report_each_bad_submission() {
        struct DeclaresBadContent;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresBadContent {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                let _ = event.register(mod_api::Block::new("foreign:lamp", "minecraft:stone"));
            }
            fn items(&self, _: &RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
                let _ = event.register(mod_api::Item::new("Bad Key", "minecraft:stick"));
            }
        }
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "maker",
                Box::new(DeclaresBadContent),
            )]
            .into(),
            || {
                let error = content_overlay(Path::new(""), &ModsState::default(), Path::new(""))
                    .err()
                    .expect("bad declarations refuse the overlay");
                let report = error.downcast_ref::<DeclarationFailures>().unwrap();
                let messages: Vec<_> = report.messages().collect();
                assert_eq!(messages.len(), 2);
                assert_eq!(messages[0].key(), "sunrise.mods.declaration.block");
                assert_eq!(messages[1].key(), "sunrise.mods.declaration.item");
                assert_eq!(
                    report.0,
                    [
                        DeclarationRefusal::new(
                            "maker",
                            DeclarationKind::Block,
                            DeclarationError::UnownedNamespace {
                                id: "foreign:lamp".into(),
                                namespace: "foreign".into(),
                            }
                        ),
                        DeclarationRefusal::new(
                            "maker",
                            DeclarationKind::Item,
                            DeclarationError::InvalidId {
                                id: "maker:Bad Key".into()
                            }
                        ),
                    ]
                );
            },
        );
    }

    #[test]
    fn native_declaration_local_errors_return_to_the_listener() {
        struct Declares;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                let mut block = mod_api::Block::new("bright", "minecraft:stone");
                block.properties.light = Some(16);
                assert!(matches!(
                    event.register(block),
                    Err(DeclarationError::Rejected { id, reason })
                        if id == "maker:bright" && !reason.is_empty()
                ));
            }
            fn items(&self, _: &RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
                let mut item = mod_api::Item::new("spoon", "minecraft:stick");
                item.components = vec![("lore".into(), None), ("minecraft:lore".into(), None)];
                assert!(
                    matches!(event.register(item), Err(DeclarationError::Rejected { id, reason })
                    if id == "maker:spoon" && reason.contains("duplicate component id minecraft:lore"))
                );
            }
        }
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("maker", Box::new(Declares))].into(),
            || {
                let declared = native_declared();
                assert!(declared.batches.is_empty());
                assert_eq!(
                    declared
                        .refusals
                        .iter()
                        .map(|r| (r.mod_id.as_str(), r.kind, r.error.id()))
                        .collect::<Vec<_>>(),
                    [
                        ("maker", DeclarationKind::Block, "maker:bright"),
                        ("maker", DeclarationKind::Item, "maker:spoon"),
                    ]
                );
            },
        );
    }

    #[test]
    fn native_declaration_reports_collection_collision_and_component_refusals_together() {
        struct Declares;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                let _ = event.register(mod_api::Block::new("foreign:lamp", "minecraft:stone"));
                event
                    .register(mod_api::Block::new("tool", "minecraft:stone"))
                    .unwrap();
            }
            fn items(&self, _: &RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
                event
                    .register(mod_api::Item::new("tool", "minecraft:stick"))
                    .unwrap();
                let mut item = mod_api::Item::new("bad_component", "minecraft:stick");
                item.components
                    .push(("maker:unknown".into(), Some(mod_api::Tag::Int(1))));
                event.register(item).unwrap();
            }
        }
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("maker", Box::new(Declares))].into(),
            || {
                let error = content_overlay(Path::new(""), &ModsState::default(), Path::new(""))
                    .err()
                    .expect("invalid native batch must refuse startup");
                let report = error.downcast_ref::<DeclarationFailures>().unwrap();
                assert_eq!(report.0.len(), 3, "{:?}", report.0);
                assert_eq!(
                    report.0[0],
                    DeclarationRefusal::new(
                        "maker",
                        DeclarationKind::Block,
                        DeclarationError::UnownedNamespace {
                            id: "foreign:lamp".into(),
                            namespace: "foreign".into()
                        }
                    )
                );
                for (record, id, detail) in [
                    (&report.0[1], "maker:tool", "block's item"),
                    (&report.0[2], "maker:bad_component", "maker:unknown"),
                ] {
                    assert_eq!(
                        (&*record.mod_id, record.kind),
                        ("maker", DeclarationKind::Item)
                    );
                    assert!(
                        matches!(&record.error, DeclarationError::Rejected { id: rejected, reason }
                        if rejected == id && reason.contains(detail)),
                        "{record:?}"
                    );
                }
            },
        );
    }

    #[test]
    fn panicking_native_declarations_publish_neither_prefixes_nor_diagnostics() {
        struct Declares {
            panics: bool,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                event
                    .register(mod_api::Block::new("pot", "minecraft:stone"))
                    .unwrap();
                if self.panics {
                    let _ = event.register(mod_api::Block::new("Bad Block", "minecraft:stone"));
                    panic!("failed after an invalid block declaration");
                }
            }
            fn items(&self, _: &RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
                event
                    .register(mod_api::Item::new("tool", "minecraft:stick"))
                    .unwrap();
                if self.panics {
                    let _ = event.register(mod_api::Item::new("Bad Item", "minecraft:stick"));
                    panic!("failed after an invalid item declaration");
                }
            }
        }

        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("failed", Box::new(Declares { panics: true })),
                mod_api::ModEntry::new("healthy", Box::new(Declares { panics: false })),
            ]
            .into(),
            || {
                let declared = native_declared();
                assert!(declared.refusals.is_empty());
                assert_eq!(
                    declared.batches.keys().copied().collect::<Vec<_>>(),
                    ["healthy"]
                );
                let batch = &declared.batches["healthy"];
                assert_eq!(batch.blocks[0].get().id, "healthy:pot");
                assert_eq!(batch.items[0].get().id, "healthy:tool");
                assert_eq!(mod_api::__private::take_hook_panics().len(), 2);
            },
        );
    }

    #[test]
    fn native_declarations_follow_the_scoped_mod_fixture() {
        struct DeclaresBlock;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresBlock {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                event
                    .register(mod_api::Block::new("test:blastproof", "minecraft:stone"))
                    .unwrap();
            }
        }
        mod_platform::testing::with_mods(Vec::new().into(), || {
            assert!(native_declared().batches.is_empty());
        });
        let mods = vec![mod_api::ModEntry::new("test", Box::new(DeclaresBlock))].into();
        mod_platform::testing::with_mods(mods, || {
            assert!(
                native_declared().batches["test"]
                    .blocks
                    .iter()
                    .any(|b| b.get().id == "test:blastproof")
            );
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            assert!(native_declared().batches.is_empty());
        });
    }

    use super::*;
    use mod_api::__private::native::BuiltMod;
    use sunrise_mod_package::runtime::Runtime;

    #[test]
    fn a_package_cannot_claim_another_packages_primary_namespace() {
        let manifest = |id, aliases| {
            sunrise_mod_package::Manifest::parse(&format!(
                "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\nnamespaces=[{aliases}]"
            ))
            .unwrap()
        };
        let owner = manifest("owner", "");
        let claimant = manifest("claimant", "'owner'");
        for layers in [[&owner, &claimant], [&claimant, &owner]] {
            assert_eq!(
                refuse_primary_namespace_claims(&layers)
                    .unwrap_err()
                    .to_string(),
                "mod claimant claims namespace owner, which is the primary namespace of mod owner; turn one of them off"
            );
        }
    }

    #[test]
    fn served_by_startup_validation_owners_live_switch_and_fatal_overlay_failures() {
        const HOST_BUILT: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Both,
        }];
        const SERVER_HOST_BUILT: &[BuiltMod] = &[BuiltMod {
            id: "horizon_srv",
            version: "1.0.0",
            side: mod_api::Side::Server,
        }];

        let f = Fixture::new("served-by");
        let make_native = |dir_name: &str,
                           id: &str,
                           side: &str,
                           serves: bool,
                           built: &'static [BuiltMod]| {
            let compiled = f.0.join(dir_name);
            let pkg = compiled.join(mod_list::NATIVE_DIR).join(id);
            std::fs::create_dir_all(pkg.join("src")).unwrap();
            std::fs::write(
                pkg.join("Cargo.toml"),
                format!("[package]\nname='{id}'\nversion='1.0.0'\n"),
            )
            .unwrap();
            std::fs::write(pkg.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
            std::fs::write(
                pkg.join("sunrise-mod.toml"),
                format!(
                    "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='{side}'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nserves_content={serves}\n",
                    "a".repeat(40),
                ),
            )
            .unwrap();
            Runtime::load(&compiled.join("sunrise-engine"), built).unwrap()
        };
        let make_content = |file_name: &str, manifest: &str, files: &[(&str, &[u8])]| {
            let path = f.0.join(file_name);
            std::fs::write(
                &path,
                sunrise_mod_package::Package::write_files(manifest, files).unwrap(),
            )
            .unwrap();
            sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&path).unwrap(),
                path,
            }
        };

        let host_runtime = Arc::new(make_native(
            "host-ok",
            "horizon_java",
            "both",
            true,
            HOST_BUILT,
        ));
        let host_unopted = Arc::new(make_native(
            "host-no",
            "horizon_java",
            "both",
            false,
            HOST_BUILT,
        ));
        let runes_ok = make_content(
            "runes.smod",
            "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nnamespaces=['runes_extra']\nserved_by='horizon_java'\n",
            &[("assets/runes/lang/en_us.json", b"{}")],
        );
        let plain_ok = make_content(
            "plain.smod",
            "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n",
            &[("assets/plain/lang/en_us.json", b"{}")],
        );

        // validate_startup_packages accepts valid served_by without clash and refuses unopted/missing hosts.
        let both_selected: Arc<[sunrise_mod_package::selection::SelectedPackage]> =
            vec![runes_ok, plain_ok].into();
        let valid_state = ModsState {
            native: host_runtime.clone(),
            selected: both_selected.clone(),
            ..Default::default()
        };
        validate_startup_packages(&valid_state).unwrap();

        let runes_only: Arc<[sunrise_mod_package::selection::SelectedPackage]> =
            vec![sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&both_selected[0].path).unwrap(),
                path: both_selected[0].path.clone(),
            }]
            .into();
        let unopted_state = ModsState {
            native: host_unopted,
            selected: runes_only.clone(),
            ..Default::default()
        };
        let unopted_err = validate_startup_packages(&unopted_state).unwrap_err();
        assert!(
            matches!(
                unopted_err.downcast_ref::<sunrise_mod_package::dependencies::Error>(),
                Some(sunrise_mod_package::dependencies::Error::InvalidServer {
                    owner,
                    target,
                    reason: sunrise_mod_package::dependencies::InvalidServerReason::HostDoesNotServeContent,
                }) if owner == "runes" && target == "horizon_java"
            ),
            "{unopted_err}"
        );

        let missing_host_state = ModsState {
            selected: runes_only,
            ..Default::default()
        };
        let missing_err = validate_startup_packages(&missing_host_state).unwrap_err();
        assert!(
            matches!(
                missing_err.downcast_ref::<sunrise_mod_package::dependencies::Error>(),
                Some(sunrise_mod_package::dependencies::Error::InvalidServer {
                    owner,
                    target,
                    reason: sunrise_mod_package::dependencies::InvalidServerReason::MissingHost,
                }) if owner == "runes" && target == "horizon_java"
            ),
            "{missing_err}"
        );

        // live_switch_of requires restart for served_by content even with assets only.
        let (mut content_ids, mut live_ids) = Default::default();
        live_switch_of(
            &both_selected[0].path,
            &both_selected[0].package,
            false,
            &mut content_ids,
            &mut live_ids,
        );
        live_switch_of(
            &both_selected[1].path,
            &both_selected[1].package,
            false,
            &mut content_ids,
            &mut live_ids,
        );
        assert!(content_ids.contains("runes") && content_ids.contains("plain"));
        assert!(!live_ids.contains("runes"));
        assert!(live_ids.contains("plain"));

        // content_owner_pairs maps served namespaces to the host and disables with the host.
        let mut runes_content = sunrise_mod_package::Content::declared("runes");
        runes_content.namespaces.push("runes_extra".into());
        let layers = vec![
            sunrise_client::mod_content::ContentMod {
                id: "horizon_java".into(),
                content: sunrise_mod_package::Content::declared("horizon_java"),
                jars: Vec::new(),
            },
            sunrise_client::mod_content::ContentMod {
                id: "runes".into(),
                content: runes_content,
                jars: Vec::new(),
            },
            sunrise_client::mod_content::ContentMod {
                id: "plain".into(),
                content: sunrise_mod_package::Content::declared("plain"),
                jars: Vec::new(),
            },
        ];
        let owners = content_owner_pairs(&layers, &valid_state.selected);
        assert_eq!(
            owners,
            [
                ("horizon_java", "horizon_java"),
                ("runes", "horizon_java"),
                ("runes_extra", "horizon_java"),
                ("plain", "plain"),
            ]
        );
        struct Quiet;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Quiet {}
        mod_platform::__private::initialize().unwrap();
        mod_platform::testing::with_dispatch(
            vec![mod_api::ModEntry::new("horizon_java", Box::new(Quiet))].into(),
            || {
                assert!(mod_api::__private::set_mod_enabled("horizon_java", false));
                mod_api::__private::install_content_owners(owners.iter().copied());
                assert!(mod_api::__private::content_owner_disabled("runes:altar"));
                assert!(mod_api::__private::content_owner_disabled(
                    "runes_extra:rune"
                ));
                assert!(!mod_api::__private::content_owner_disabled("plain:block"));
            },
        );

        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();

        // Missing jar on a served content package is fatal.
        let runes_missing_jar = make_content(
            "runes-missing-jar.smod",
            &format!(
                "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n[[jar]]\nid='orig'\nversion='1'\nsha512='{}'\n",
                "9".repeat(128),
            ),
            &[],
        );
        let missing_jar_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![runes_missing_jar].into(),
            ..Default::default()
        };
        let err = content_overlay(&base, &missing_jar_state, &store)
            .err()
            .unwrap();
        assert!(err.downcast_ref::<MissingJar>().is_some(), "{err}");

        // Read error on a served content package is fatal.
        let runes_bad_json = make_content(
            "runes-bad-json.smod",
            "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
            &[("data/runes/sunrise/item/broken.json", b"{not json")],
        );
        let bad_json_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![runes_bad_json].into(),
            ..Default::default()
        };
        let err = content_overlay(&base, &bad_json_state, &store)
            .err()
            .unwrap();
        let load_err = err
            .downcast_ref::<ServedContentLoadError>()
            .expect("typed ServedContentLoadError");
        assert_eq!(load_err.mod_id, "runes");
        assert_eq!(load_err.stage, ServedContentStage::Read);

        // A served package whose crafting_remainder names an owned-namespace item passes
        // package-only Content::read (build/check), yet fatally fails load-time content_overlay
        // if the serving host never declares that item.
        let runes_missing_remainder = make_content(
            "runes-missing-rem.smod",
            "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
            &[(
                "data/runes/sunrise/item/stew.json",
                br#"{"template":"minecraft:mushroom_stew","crafting_remainder":"runes:host_bowl"}"#,
            )],
        );
        sunrise_mod_package::Content::read(
            &runes_missing_remainder.path,
            &runes_missing_remainder.package.manifest,
        )
        .expect("package-only read allows host-supplied remainder in owned namespace");
        let missing_rem_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![runes_missing_remainder].into(),
            ..Default::default()
        };
        let err = content_overlay(&base, &missing_rem_state, &store)
            .err()
            .unwrap();
        let load_err = err
            .downcast_ref::<ServedContentLoadError>()
            .expect("undeclared host crafting_remainder fails load fatally");
        assert_eq!(load_err.mod_id, "runes");
        assert_eq!(load_err.stage, ServedContentStage::Read);
        assert!(load_err.detail.contains("runes:host_bowl"), "{load_err}");

        // accept_mod error (unknown component) on a served content package is fatal.
        let runes_bad_component = make_content(
            "runes-bad-component.smod",
            "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
            &[(
                "data/runes/sunrise/item/bad.json",
                br#"{"template":"minecraft:stick","components":{"example:unknown_component":1}}"#,
            )],
        );
        let bad_component_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![runes_bad_component].into(),
            ..Default::default()
        };
        let err = content_overlay(&base, &bad_component_state, &store)
            .err()
            .unwrap();
        let load_err = err
            .downcast_ref::<ServedContentLoadError>()
            .expect("typed ServedContentLoadError");
        assert_eq!(load_err.mod_id, "runes");
        assert_eq!(load_err.stage, ServedContentStage::Admit);
        assert!(load_err.detail.contains("unknown_component"), "{load_err}");

        // server_only_declaration catches a side="server" host declaring a block in a served namespace.
        let server_runtime = Arc::new(make_native(
            "host-srv",
            "horizon_srv",
            "server",
            true,
            SERVER_HOST_BUILT,
        ));
        let server_runes = make_content(
            "server-runes.smod",
            "id='srv_runes'\nname='SrvRunes'\nversion='1.0.0'\napi='0.2'\nside='server'\ncapabilities=['registries']\nserved_by='horizon_srv'\n",
            &[],
        );
        struct ServerHostDeclares;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ServerHostDeclares {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                event
                    .register(mod_api::Block::new("srv_runes:altar", "minecraft:stone"))
                    .unwrap();
            }
        }
        let server_packages = vec![
            mod_api::PackageInfo::new(
                "horizon_srv".into(),
                "1.0.0".into(),
                mod_api::PackageKind::Code,
                mod_api::Side::Server,
                vec!["horizon_srv".into()],
                vec![],
            ),
            mod_api::PackageInfo::new(
                "srv_runes".into(),
                "1.0.0".into(),
                mod_api::PackageKind::Content,
                mod_api::Side::Server,
                vec!["srv_runes".into()],
                vec![],
            )
            .with_served_by(Some("horizon_srv".into())),
        ];
        let server_state = ModsState {
            native: server_runtime,
            selected: vec![server_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "horizon_srv",
                Box::new(ServerHostDeclares),
            )]
            .into(),
            || {
                mod_platform::__private::with_test_package_inventory(server_packages, || {
                    let err = content_overlay(&base, &server_state, &store).err().unwrap();
                    assert_eq!(
                        err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                        Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                            declarer: "horizon_srv".into(),
                            supplier: "srv_runes".into(),
                            kind: "block".into(),
                            id: "srv_runes:altar".into(),
                        }),
                        "{err}"
                    );
                });
            },
        );

        // A side="both" host declaring a block or required client-bound payload in a
        // side="server" served package's namespace must also be refused.
        let both_server_runes = make_content(
            "both-server-runes.smod",
            "id='srv_runes'\nname='SrvRunes'\nversion='1.0.0'\napi='0.2'\nside='server'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
            &[],
        );
        let both_server_state = ModsState {
            native: host_runtime,
            selected: vec![both_server_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        let both_server_packages = || {
            vec![
                mod_api::PackageInfo::new(
                    "horizon_java".into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Code,
                    mod_api::Side::Both,
                    vec!["horizon_java".into()],
                    vec![],
                ),
                mod_api::PackageInfo::new(
                    "srv_runes".into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Content,
                    mod_api::Side::Server,
                    vec!["srv_runes".into()],
                    vec![],
                )
                .with_served_by(Some("horizon_java".into())),
            ]
        };
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "horizon_java",
                Box::new(ServerHostDeclares),
            )]
            .into(),
            || {
                mod_platform::__private::with_test_package_inventory(
                    both_server_packages(),
                    || {
                        let err = content_overlay(&base, &both_server_state, &store)
                            .err()
                            .unwrap();
                        assert_eq!(
                        err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                        Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                            declarer: "horizon_java".into(),
                            supplier: "srv_runes".into(),
                            kind: "block".into(),
                            id: "srv_runes:altar".into(),
                        }),
                        "{err}"
                    );
                    },
                );
            },
        );

        struct BothHostPayload;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for BothHostPayload {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                r.payload_handlers()
                    .registrar("1")
                    .play_to_client(mod_api::PayloadId("srv_runes:sync".into()), None);
            }
        }
        mod_platform::__private::with_test_package_inventory(both_server_packages(), || {
            let payload_mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "horizon_java",
                Box::new(BothHostPayload),
            )]
            .into();
            mod_platform::__private::part_tests::install_parts_fixture(&payload_mods).unwrap();
            mod_platform::testing::with_mods(payload_mods, || {
                let err = content_overlay(&base, &both_server_state, &store)
                    .err()
                    .unwrap();
                assert_eq!(
                    err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                    Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                        declarer: "horizon_java".into(),
                        supplier: "srv_runes".into(),
                        kind: "payload".into(),
                        id: "srv_runes:sync".into(),
                    }),
                    "{err}"
                );
            });
        });
    }

    #[test]
    fn served_by_startup_refuses_general_entity_type_in_server_namespace() {
        use std::sync::Mutex;
        const HOST_BUILT: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Both,
        }];
        let f = Fixture::new("served-by-entity-type-side");
        let compiled = f.0.join("host");
        let pkg = compiled.join(mod_list::NATIVE_DIR).join("horizon_java");
        std::fs::create_dir_all(pkg.join("src")).unwrap();
        std::fs::write(
            pkg.join("Cargo.toml"),
            "[package]\nname='horizon_java'\nversion='1.0.0'\n",
        )
        .unwrap();
        std::fs::write(pkg.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
        std::fs::write(
            pkg.join("sunrise-mod.toml"),
            format!(
                "id='horizon_java'\nname='Horizon'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nserves_content=true\n",
                "a".repeat(40),
            ),
        )
        .unwrap();
        let host_runtime =
            Arc::new(Runtime::load(&compiled.join("sunrise-engine"), HOST_BUILT).unwrap());
        let make_content = |file_name: &str, id: &str, side: &str| {
            let path = f.0.join(file_name);
            let manifest = format!(
                "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='{side}'\ncapabilities=['registries']\nserved_by='horizon_java'\n"
            );
            std::fs::write(
                &path,
                sunrise_mod_package::Package::write_files(&manifest, &[]).unwrap(),
            )
            .unwrap();
            sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&path).unwrap(),
                path,
            }
        };
        let srv_runes = make_content("srv-runes.smod", "srv_runes", "server");
        let both_runes = make_content("both-runes.smod", "runes", "both");
        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        let server_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![srv_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        let both_state = ModsState {
            native: host_runtime,
            selected: vec![both_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        validate_startup_packages(&server_state).unwrap();
        validate_startup_packages(&both_state).unwrap();

        struct DeclaresEntity {
            target: &'static str,
            outcome: Arc<Mutex<Option<Result<(), mod_api::GeneralEntityRegistrationError>>>>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresEntity {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                let result = r.declare_general_entity(
                    self.target,
                    mod_api::GeneralEntityCategory::Misc,
                    0.8,
                    1.9,
                    None,
                    mod_api::GeneralSavePolicy::Save,
                    true,
                    false,
                    None,
                    8,
                    3,
                    1,
                    &[],
                );
                *self.outcome.lock().unwrap() = Some(result);
            }
        }

        let inventory = |content_id: &str, content_side: mod_api::Side| {
            vec![
                mod_api::PackageInfo::new(
                    "horizon_java".into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Code,
                    mod_api::Side::Both,
                    vec!["horizon_java".into()],
                    vec![],
                ),
                mod_api::PackageInfo::new(
                    content_id.into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Content,
                    content_side,
                    vec![content_id.into()],
                    vec![],
                )
                .with_served_by(Some("horizon_java".into())),
            ]
        };

        // 1. Real startup path refuses at admit_parts with GeneralEntityRegistrationError::WrongSide.
        let outcome = Arc::new(Mutex::new(None));
        let mods = [mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresEntity {
                target: "srv_runes:drone",
                outcome: outcome.clone(),
            }),
        )];
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Server),
            || {
                assert!(crate::game::part_admission::admit_parts(&mods).is_err());
            },
        );
        assert_eq!(
            *outcome.lock().unwrap(),
            Some(Err(mod_api::GeneralEntityRegistrationError::WrongSide))
        );

        // 2. Defense-in-depth: even if installed without the server-side inventory check,
        // content_overlay refuses with typed ServerOnlyDeclaration.
        let outcome_bypass = Arc::new(Mutex::new(None));
        let mods_bypass: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresEntity {
                target: "srv_runes:drone",
                outcome: outcome_bypass.clone(),
            }),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Both),
            || {
                crate::game::part_admission::admit_parts(&mods_bypass).unwrap();
                assert_eq!(*outcome_bypass.lock().unwrap(), Some(Ok(())));
                mod_platform::testing::with_mods(mods_bypass, || {
                    let err = content_overlay(&base, &server_state, &store).err().unwrap();
                    assert_eq!(
                        err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                        Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                            declarer: "horizon_java".into(),
                            supplier: "srv_runes".into(),
                            kind: "entity type".into(),
                            id: "srv_runes:drone".into(),
                        }),
                        "{err}"
                    );
                });
            },
        );

        // 3. Positive control: a side="both" served package passes both admit_parts and content_overlay.
        let outcome_ok = Arc::new(Mutex::new(None));
        let mods_ok: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresEntity {
                target: "runes:drone",
                outcome: outcome_ok.clone(),
            }),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("runes", mod_api::Side::Both),
            || {
                crate::game::part_admission::admit_parts(&mods_ok).unwrap();
                assert_eq!(*outcome_ok.lock().unwrap(), Some(Ok(())));
                mod_platform::testing::with_mods(mods_ok, || {
                    content_overlay(&base, &both_state, &store).unwrap();
                });
            },
        );
    }

    #[test]
    fn served_by_startup_refuses_attribute_and_client_required_declarations_in_server_namespace() {
        use std::sync::Mutex;
        const HOST_BUILT: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Both,
        }];
        let f = Fixture::new("served-by-attribute-side");
        let compiled = f.0.join("host");
        let pkg = compiled.join(mod_list::NATIVE_DIR).join("horizon_java");
        std::fs::create_dir_all(pkg.join("src")).unwrap();
        std::fs::write(
            pkg.join("Cargo.toml"),
            "[package]\nname='horizon_java'\nversion='1.0.0'\n",
        )
        .unwrap();
        std::fs::write(pkg.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
        std::fs::write(
            pkg.join("sunrise-mod.toml"),
            format!(
                "id='horizon_java'\nname='Horizon'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nserves_content=true\n",
                "a".repeat(40),
            ),
        )
        .unwrap();
        let host_runtime =
            Arc::new(Runtime::load(&compiled.join("sunrise-engine"), HOST_BUILT).unwrap());
        let make_content = |file_name: &str, id: &str, side: &str| {
            let path = f.0.join(file_name);
            let manifest = format!(
                "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='{side}'\ncapabilities=['registries']\nserved_by='horizon_java'\n"
            );
            std::fs::write(
                &path,
                sunrise_mod_package::Package::write_files(&manifest, &[]).unwrap(),
            )
            .unwrap();
            sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&path).unwrap(),
                path,
            }
        };
        let srv_runes = make_content("srv-runes.smod", "srv_runes", "server");
        let both_runes = make_content("both-runes.smod", "runes", "both");
        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        let server_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![srv_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        let both_state = ModsState {
            native: host_runtime,
            selected: vec![both_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        validate_startup_packages(&server_state).unwrap();
        validate_startup_packages(&both_state).unwrap();

        let inventory = |content_id: &str, content_side: mod_api::Side| {
            vec![
                mod_api::PackageInfo::new(
                    "horizon_java".into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Code,
                    mod_api::Side::Both,
                    vec!["horizon_java".into()],
                    vec![],
                ),
                mod_api::PackageInfo::new(
                    content_id.into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Content,
                    content_side,
                    vec![content_id.into()],
                    vec![],
                )
                .with_served_by(Some("horizon_java".into())),
            ]
        };

        struct DeclaresAttribute {
            target: &'static str,
            outcome: Arc<Mutex<Option<Result<(), mod_api::AttributeRegistrationError>>>>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresAttribute {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                let result = r.declare_attribute(
                    self.target,
                    10.0,
                    0.0,
                    100.0,
                    true,
                    mod_api::Sentiment::Positive,
                    Default::default(),
                );
                *self.outcome.lock().unwrap() = Some(result);
            }
        }

        // 1. Real startup path refuses attribute declaration in server served namespace at admit_parts.
        let outcome = Arc::new(Mutex::new(None));
        let attr_mods = [mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresAttribute {
                target: "srv_runes:mana",
                outcome: outcome.clone(),
            }),
        )];
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Server),
            || {
                assert!(crate::game::part_admission::admit_parts(&attr_mods).is_err());
            },
        );
        assert_eq!(
            *outcome.lock().unwrap(),
            Some(Err(mod_api::AttributeRegistrationError::WrongSide))
        );

        // 2. Defense-in-depth: content_overlay refuses attribute declaration with ServerOnlyDeclaration.
        let outcome_bypass = Arc::new(Mutex::new(None));
        let attr_bypass: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresAttribute {
                target: "srv_runes:mana",
                outcome: outcome_bypass.clone(),
            }),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Both),
            || {
                crate::game::part_admission::admit_parts(&attr_bypass).unwrap();
                assert_eq!(*outcome_bypass.lock().unwrap(), Some(Ok(())));
                mod_platform::testing::with_mods(attr_bypass, || {
                    let err = content_overlay(&base, &server_state, &store).err().unwrap();
                    assert_eq!(
                        err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                        Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                            declarer: "horizon_java".into(),
                            supplier: "srv_runes".into(),
                            kind: "attribute".into(),
                            id: "srv_runes:mana".into(),
                        }),
                        "{err}"
                    );
                });
            },
        );

        // 3. Custom stat in a server served namespace passes register and is refused at content_overlay.
        struct DeclaresCustomStat(&'static str);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresCustomStat {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                r.register_custom_stat(self.0).unwrap();
            }
        }
        let stat_mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresCustomStat("srv_runes:casts")),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Server),
            || {
                crate::game::part_admission::admit_parts(&stat_mods).unwrap();
                mod_platform::testing::with_mods(stat_mods, || {
                    let err = content_overlay(&base, &server_state, &store).err().unwrap();
                    assert_eq!(
                        err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                        Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                            declarer: "horizon_java".into(),
                            supplier: "srv_runes".into(),
                            kind: "custom stat".into(),
                            id: "srv_runes:casts".into(),
                        }),
                        "{err}"
                    );
                });
            },
        );

        // 4. Server-only (unsynced) attachment in a server served namespace is allowed,
        // while synced attachment and datapack registry are refused.
        struct DeclaresUnsynced;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresUnsynced {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                r.register_attachment_type("srv_runes:internal", false)
                    .unwrap();
            }
        }
        let unsynced_mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresUnsynced),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Server),
            || {
                crate::game::part_admission::admit_parts(&unsynced_mods).unwrap();
                mod_platform::testing::with_mods(unsynced_mods, || {
                    content_overlay(&base, &server_state, &store).unwrap();
                });
            },
        );

        // 5. Positive control: all client-required declarations in a side="both" served namespace pass.
        struct DeclaresAllBoth;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresAllBoth {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                r.declare_attribute(
                    "runes:mana",
                    10.0,
                    0.0,
                    100.0,
                    true,
                    mod_api::Sentiment::Positive,
                    Default::default(),
                )
                .unwrap();
                r.register_custom_stat("runes:casts").unwrap();
                r.register_attachment_type("runes:charge", true).unwrap();
                r.declare_datapack_registry(
                    "runes:spell",
                    mod_api::DatapackRegistrySync::Required,
                    vec![mod_api::DatapackField::required(
                        "power",
                        mod_api::DatapackFieldKind::Int { min: 0, max: 100 },
                    )],
                )
                .unwrap();
            }
        }
        let ok_mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresAllBoth),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("runes", mod_api::Side::Both),
            || {
                crate::game::part_admission::admit_parts(&ok_mods).unwrap();
                mod_platform::testing::with_mods(ok_mods, || {
                    content_overlay(&base, &both_state, &store).unwrap();
                });
            },
        );
    }

    #[test]
    fn served_by_both_sides_validate_locally_and_agree_on_mod_list() {
        const HOST_BOTH: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Both,
        }];
        const HOST_CLIENT: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Client,
        }];
        const HOST_SERVER: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Server,
        }];
        let f = Fixture::new("served-by-mod-list");
        let make_native = |dir_name: &str, side: &str, built: &'static [BuiltMod]| {
            let compiled = f.0.join(dir_name);
            let pkg = compiled.join(mod_list::NATIVE_DIR).join("horizon_java");
            std::fs::create_dir_all(pkg.join("src")).unwrap();
            std::fs::write(
                pkg.join("Cargo.toml"),
                "[package]\nname='horizon_java'\nversion='1.0.0'\n",
            )
            .unwrap();
            std::fs::write(pkg.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
            std::fs::write(
                pkg.join("sunrise-mod.toml"),
                format!(
                    "id='horizon_java'\nname='Horizon'\nversion='1.0.0'\napi='0.2'\nside='{side}'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nserves_content=true\n",
                    "a".repeat(40),
                ),
            )
            .unwrap();
            Runtime::load(&compiled.join("sunrise-engine"), built).unwrap()
        };
        let path = f.0.join("runes.smod");
        std::fs::write(
            &path,
            sunrise_mod_package::Package::write_files(
                "id='runes'\nname='Runes'\nversion='2.1.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        let selected: Arc<[sunrise_mod_package::selection::SelectedPackage]> =
            vec![sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&path).unwrap(),
                path,
            }]
            .into();

        // A client-only or server-only host is refused locally for side="both" served content.
        for (bad_side, expected_host, built) in [
            (
                "client",
                sunrise_mod_package::manifest::Side::Client,
                HOST_CLIENT,
            ),
            (
                "server",
                sunrise_mod_package::manifest::Side::Server,
                HOST_SERVER,
            ),
        ] {
            let bad_state = ModsState {
                native: Arc::new(make_native(&format!("host-{bad_side}"), bad_side, built)),
                selected: selected.clone(),
                ..Default::default()
            };
            let err = validate_startup_packages(&bad_state).unwrap_err();
            assert!(
                matches!(
                    err.downcast_ref::<sunrise_mod_package::dependencies::Error>(),
                    Some(sunrise_mod_package::dependencies::Error::InvalidServer {
                        owner,
                        target,
                        reason: sunrise_mod_package::dependencies::InvalidServerReason::SideNotCovered {
                            host,
                            content: sunrise_mod_package::manifest::Side::Both,
                        },
                    }) if owner == "runes" && target == "horizon_java" && *host == expected_host
                ),
                "{bad_side}: {err}"
            );
        }

        // Valid side="both" host + served content validates on both sides and produces matching
        // sunrise:mod_list payloads directly from content_overlay admission.
        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        let both_host = Arc::new(make_native("host-both", "both", HOST_BOTH));
        let server_state = ModsState {
            native: both_host.clone(),
            selected: selected.clone(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        let client_state = ModsState {
            native: both_host,
            selected: selected.clone(),
            installation_side: sunrise_mod_package::manifest::Side::Client,
            ..Default::default()
        };
        validate_startup_packages(&server_state).unwrap();
        validate_startup_packages(&client_state).unwrap();

        let server_announced = content_overlay(&base, &server_state, &store)
            .unwrap()
            .announced;
        let client_announced = content_overlay(&base, &client_state, &store)
            .unwrap()
            .announced;
        let server_wire = sunrise_protocol::mod_list::decode(&sunrise_protocol::mod_list::encode(
            &server_announced,
        ))
        .unwrap();
        let client_wire = sunrise_protocol::mod_list::decode(&sunrise_protocol::mod_list::encode(
            &client_announced,
        ))
        .unwrap();
        assert!(sunrise_protocol::mod_list::compare(&server_wire, &client_wire).is_empty());

        // A client missing the served package is rejected by sunrise:mod_list comparison.
        let client_without_runes = ModsState {
            native: server_state.native.clone(),
            installation_side: sunrise_mod_package::manifest::Side::Client,
            ..Default::default()
        };
        validate_startup_packages(&client_without_runes).unwrap();
        let partial_announced = content_overlay(&base, &client_without_runes, &store)
            .unwrap()
            .announced;
        let partial_wire = sunrise_protocol::mod_list::decode(&sunrise_protocol::mod_list::encode(
            &partial_announced,
        ))
        .unwrap();
        let diff = sunrise_protocol::mod_list::compare(&server_wire, &partial_wire);
        assert_eq!(diff.missing, ["runes 2.1.0"]);
    }

    #[test]
    fn served_by_host_switch_is_restart_bound_and_withholds_served_content_when_disabled_at_startup()
     {
        const BUILT_HOSTS: &[BuiltMod] = &[
            BuiltMod {
                id: "horizon_java",
                version: "1.0.0",
                side: mod_api::Side::Both,
            },
            BuiltMod {
                id: "plain_native",
                version: "1.0.0",
                side: mod_api::Side::Both,
            },
        ];
        let f = Fixture::new("served-by-host-switch");
        let compiled = f.0.join("compiled");
        for (id, serves) in [("horizon_java", true), ("plain_native", false)] {
            let pkg = compiled.join(mod_list::NATIVE_DIR).join(id);
            std::fs::create_dir_all(pkg.join("src")).unwrap();
            std::fs::write(
                pkg.join("Cargo.toml"),
                format!("[package]\nname='{id}'\nversion='1.0.0'\n"),
            )
            .unwrap();
            std::fs::write(pkg.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
            std::fs::write(
                pkg.join("sunrise-mod.toml"),
                format!(
                    "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='both'\n\
                     capabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\n\
                     serves_content={serves}\n",
                    "a".repeat(40),
                ),
            )
            .unwrap();
        }
        let native =
            Arc::new(Runtime::load(&compiled.join("sunrise-engine"), BUILT_HOSTS).unwrap());

        let mods_dir = f.0.join(mod_list::DIR);
        std::fs::create_dir_all(&mods_dir).unwrap();
        std::fs::write(
            mods_dir.join("runes.smod"),
            sunrise_mod_package::Package::write_files(
                "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\n\
                 capabilities=['registries']\nserved_by='horizon_java'\n",
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            mods_dir.join("plain.smod"),
            sunrise_mod_package::Package::write_files(
                "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\n\
                 capabilities=['registries']\n",
                &[],
            )
            .unwrap(),
        )
        .unwrap();

        let enabled = std::collections::BTreeSet::from(["runes".into(), "plain".into()]);

        // Session 1: host starts enabled; toggling it off is restart-bound while an ordinary
        // native mod switches off live.
        let mut active = ModsState {
            native: native.clone(),
            ..Default::default()
        };
        active.capture_code_switches(&std::collections::BTreeSet::new());
        active.select(&f.0, &enabled);
        validate_startup_packages(&active).unwrap();
        assert_eq!(active.selected.len(), 2);

        let wanted_disabled =
            std::collections::BTreeSet::from(["horizon_java".into(), "plain_native".into()]);
        let (effective, waiting) = active.reconcile_code_switches(&wanted_disabled);
        assert_eq!(waiting, ["horizon_java"]);
        assert!(!effective.contains("horizon_java"));
        assert!(effective.contains("plain_native"));
        assert!(!active.code_disabled(&wanted_disabled, "horizon_java"));
        assert!(active.code_disabled(&wanted_disabled, "plain_native"));

        struct Quiet;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Quiet {}
        mod_platform::__private::initialize().unwrap();
        mod_platform::testing::with_dispatch(
            vec![
                mod_api::ModEntry::new("horizon_java", Box::new(Quiet)),
                mod_api::ModEntry::new("plain_native", Box::new(Quiet)),
            ]
            .into(),
            || {
                mod_api::__private::install_content_owners([("runes", "horizon_java")]);
                active.apply_code_switches(&wanted_disabled);
                assert!(mod_api::__private::mod_index_on(
                    mod_api::__private::mod_index("horizon_java").unwrap()
                ));
                assert!(!mod_api::__private::mod_index_on(
                    mod_api::__private::mod_index("plain_native").unwrap()
                ));
                assert!(!mod_api::__private::content_owner_disabled("runes:altar"));
            },
        );

        // Session 2 (after restart with host disabled): select() withholds served content,
        // reports the disabled host on its Mods-screen row, passes startup validation, and keeps
        // re-enabling the host restart-bound so both reload together.
        let mut restarted = ModsState {
            native: native.clone(),
            ..Default::default()
        };
        let startup_disabled = std::collections::BTreeSet::from(["horizon_java".into()]);
        restarted.capture_code_switches(&startup_disabled);
        restarted.select(&f.0, &enabled);
        validate_startup_packages(&restarted).unwrap();
        let selected_ids: Vec<_> = restarted
            .selected
            .iter()
            .map(|p| p.package.manifest.id.as_str())
            .collect();
        assert_eq!(selected_ids, ["plain"]);

        let mut rows = vec![Installed {
            id: "runes".into(),
            title: "Runes 1.0.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: None,
        }];
        restarted.report_refusals(&mut rows);
        let problem = rows[0]
            .problem
            .as_deref()
            .expect("withheld served mod shows Mods-screen problem");
        assert!(problem.contains("horizon_java"), "{problem}");

        let reenable = std::collections::BTreeSet::new();
        let (effective_reenable, waiting_reenable) = restarted.reconcile_code_switches(&reenable);
        assert_eq!(waiting_reenable, ["horizon_java"]);
        assert!(effective_reenable.contains("horizon_java"));
        assert!(restarted.code_disabled(&reenable, "horizon_java"));

        // Defend the invariant if a caller bypasses select() with a disabled host still selected.
        let mut bypassed = active.clone();
        bypassed.capture_code_switches(&startup_disabled);
        let err = validate_startup_packages(&bypassed).unwrap_err();
        assert_eq!(
            err.downcast_ref::<DisabledServingHost>(),
            Some(&DisabledServingHost {
                content: "runes".into(),
                host: "horizon_java".into(),
            }),
            "{err}"
        );
    }

    #[test]
    fn served_by_mob_effect_owner_is_serving_host() {
        let f = Fixture::new("served-by-mob-effect");
        let runes_path = f.0.join("runes.smod");
        std::fs::write(
            &runes_path,
            sunrise_mod_package::Package::write_files(
                "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\n\
                 capabilities=['registries']\nserved_by='horizon_java'\n",
                &[(
                    "data/runes/sunrise/mob_effect/ward.json",
                    br#"{"category":"beneficial","color":65280}"#,
                )],
            )
            .unwrap(),
        )
        .unwrap();
        let plain_path = f.0.join("plain.smod");
        std::fs::write(
            &plain_path,
            sunrise_mod_package::Package::write_files(
                "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\n\
                 capabilities=['registries']\n",
                &[(
                    "data/plain/sunrise/mob_effect/glow.json",
                    br#"{"category":"neutral","color":16777215}"#,
                )],
            )
            .unwrap(),
        )
        .unwrap();
        let runes_pkg = sunrise_mod_package::Package::read(&runes_path).unwrap();
        let plain_pkg = sunrise_mod_package::Package::read(&plain_path).unwrap();
        let layers = vec![
            sunrise_client::mod_content::ContentMod {
                id: "runes".into(),
                content: sunrise_mod_package::Content::read(&runes_path, &runes_pkg.manifest)
                    .unwrap(),
                jars: Vec::new(),
            },
            sunrise_client::mod_content::ContentMod {
                id: "plain".into(),
                content: sunrise_mod_package::Content::read(&plain_path, &plain_pkg.manifest)
                    .unwrap(),
                jars: Vec::new(),
            },
        ];
        let selected = vec![
            sunrise_mod_package::selection::SelectedPackage {
                package: runes_pkg,
                path: runes_path,
            },
            sunrise_mod_package::selection::SelectedPackage {
                package: plain_pkg,
                path: plain_path,
            },
        ];
        let entries = mob_effect_entries(&layers, &selected);
        let owners: Vec<_> = entries
            .iter()
            .map(|(owner, effect)| (effect.id.as_str(), *owner))
            .collect();
        assert_eq!(
            owners,
            [("plain:glow", "plain"), ("runes:ward", "horizon_java")]
        );

        type Trace = Arc<Mutex<Vec<(&'static str, &'static str)>>>;
        #[derive(Clone)]
        struct WardTick<const PERIOD: i32> {
            package: &'static str,
            trace: Trace,
        }
        impl<const PERIOD: i32> WardTick<PERIOD> {
            fn entry(package: &'static str, trace: &Trace) -> mod_api::ModEntry {
                mod_api::ModEntry::new(
                    package,
                    Box::new(Self {
                        package,
                        trace: trace.clone(),
                    }),
                )
            }
        }
        #[mod_api::sunrise_part]
        impl<const PERIOD: i32> mod_api::MobEffectPart for WardTick<PERIOD> {
            fn should_apply_effect_tick_this_tick(
                &self,
                _: &dyn mod_api::Host,
                event: &mut mod_api::ShouldApplyEffectTickThisTickEvent,
            ) {
                self.trace.lock().unwrap().push((self.package, "cadence"));
                event.apply = event.tick_count % PERIOD == 0;
            }

            fn apply_effect_tick(
                &self,
                _: &dyn mod_api::Host,
                event: &mut mod_api::ApplyEffectTickEvent<'_>,
            ) {
                self.trace.lock().unwrap().push((self.package, "action"));
                event.heal(PERIOD as f32);
            }
        }
        #[mod_api::__subscriptions]
        impl<const PERIOD: i32> mod_api::Hooks for WardTick<PERIOD> {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                let key = r.effect_key("runes:ward");
                r.mob_effect_part(&key, self.clone()).unwrap();
            }
        }
        let owner_of = |id: &str| {
            owners
                .iter()
                .find_map(|(effect_id, owner)| (*effect_id == id).then_some(*owner))
        };
        let trace = Arc::new(Mutex::new(Vec::new()));
        // Put the dependent first in normal dispatch order; the effect owner must run first.
        let mods: mod_api::Mods = vec![
            WardTick::<3>::entry("dependent", &trace),
            WardTick::<2>::entry("horizon_java", &trace),
        ]
        .into();
        let package = |id: &str, kind, dependencies| {
            mod_api::PackageInfo::new(
                id.into(),
                "1.0.0".into(),
                kind,
                mod_api::Side::Both,
                vec![id.into()],
                dependencies,
            )
        };
        mod_platform::__private::with_test_package_inventory(
            vec![
                package("horizon_java", mod_api::PackageKind::Code, vec![]),
                package("runes", mod_api::PackageKind::Content, vec![])
                    .with_served_by(Some("horizon_java".into())),
                package(
                    "dependent",
                    mod_api::PackageKind::Code,
                    vec![mod_api::PackageDependency::new(
                        "horizon_java".into(),
                        "*".into(),
                        mod_api::DependencyKind::Required,
                        mod_api::DependencyOrder::Before,
                        mod_api::Side::Both,
                        None,
                        None,
                    )],
                ),
            ],
            || {
                mod_platform::testing::with_mods(mods.clone(), || {
                    mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
                    mod_api::__private::resolve_effects(["runes:ward"], |id| {
                        (id == "runes:ward").then_some(0)
                    })
                    .unwrap();
                    let effect = mod_api::__private::effect_key("runes:ward").unwrap();
                    mod_api::__private::validate_effect_part_owners(owner_of).expect(
                        "the serving host and its dependent may attach to the served effect",
                    );

                    let mut changes = Vec::new();
                    let mut keep = true;
                    let active_effects = [effect];
                    mod_api::__private::dispatch_mob_effect_tick(
                        &mod_platform::testing::FakeHost::new(),
                        6,
                        effect,
                        0,
                        |effect, amplifier| mod_api::__private::ApplyEffectTickCall {
                            effect,
                            dimension: "minecraft:overworld",
                            entity: Some(7),
                            amplifier,
                            holder: mod_api::LivingRef::new(mod_api::EntityRef::new(
                                mod_api::WorldEpoch::new(1),
                                7,
                            )),
                            health: 10.0,
                            max_health: 20.0,
                            food: None,
                            active_effects: &active_effects,
                            changes: &mut changes,
                            keep: &mut keep,
                        },
                    );
                    assert_eq!(
                        *trace.lock().unwrap(),
                        [
                            ("horizon_java", "cadence"),
                            ("horizon_java", "action"),
                            ("dependent", "cadence"),
                            ("dependent", "action"),
                        ]
                    );
                    assert_eq!(
                        changes,
                        [
                            mod_api::EffectTickChange::Heal(2.0),
                            mod_api::EffectTickChange::Heal(3.0)
                        ]
                    );
                });
            },
        );

        let unrelated: mod_api::Mods = vec![WardTick::<2>::entry("third", &trace)].into();
        mod_platform::testing::with_mods(unrelated.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&unrelated).unwrap();
            let refused = mod_api::__private::validate_effect_part_owners(owner_of)
                .expect_err("unrelated mod without dependency on serving host must be refused");
            assert_eq!(refused.len(), 1, "{refused:?}");
            assert!(
                refused[0].contains("third") && refused[0].contains("horizon_java"),
                "{refused:?}"
            );
        });
    }

    #[test]
    fn only_clients_collect_registered_keys() {
        let declared = [(
            "sample",
            mod_api::KeyRegistration::new("zoom", "key.keyboard.z"),
        )];
        assert_eq!(
            collect_key_mappings(ExecutionRole::Client, &declared).len(),
            1
        );
        assert!(collect_key_mappings(ExecutionRole::IntegratedServer, &declared).is_empty());
        assert!(collect_key_mappings(ExecutionRole::DedicatedServer, &declared).is_empty());
    }

    mod inventory {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/game/modding/mods/mod_inventory_tests.rs"
        ));
    }

    mod recipe_priorities {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/game/modding/mods/recipe_priorities_tests.rs"
        ));
    }

    const BUILT: &[BuiltMod] = &[BuiltMod {
        id: "machine_sample",
        version: "0.3.0",
        side: mod_api::Side::Both,
    }];

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("sunrise-game-mods-{}-{label}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn runtime(&self) -> Runtime {
            let dir = self.0.join("compiled");
            let package = dir.join(mod_list::NATIVE_DIR).join("machine_sample");
            let file = std::fs::File::open(self.0.join("machine_sample.smod")).unwrap();
            zip::ZipArchive::new(file)
                .unwrap()
                .extract(&package)
                .unwrap();
            Runtime::load(&dir.join("sunrise-engine"), BUILT).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    /// Runs `test` again in a child process whose working directory is its own
    /// empty folder: the overlay's `.sunrise-cache` lives under the home, set here to
    /// the working directory, so children sharing one (the temp dir) race on that cache.
    pub(in crate::game) fn run_in_child(test: &str, mode: &str) {
        let cwd = std::env::temp_dir().join(format!(
            "sunrise-game-mods-cwd-{}-{mode}",
            std::process::id()
        ));
        std::fs::create_dir_all(&cwd).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .current_dir(&cwd)
            .args(["--include-ignored", "--exact", test, "--nocapture"])
            .env("SUNRISE_HOME", &cwd)
            .env(mode, "1")
            .status()
            .unwrap();
        std::fs::remove_dir_all(&cwd).unwrap();
        assert!(status.success());
    }

    #[test]
    fn a_package_cannot_declare_one_type_in_both_entity_families() {
        let general = crate::game::general::GeneralTypeCatalog::freeze(&[
            mod_api::__private::GeneralEntityTypeDeclaration {
                mod_id: "ruby".into(),
                id: "ruby:robot".into(),
                category: mod_api::GeneralEntityCategory::Misc,
                width: 0.5,
                height: 1.0,
                passenger_attachment: None,
                save_policy: mod_api::GeneralSavePolicy::Save,
                summonable: true,
                pickable: false,
                piston_push_reaction: mod_api::PushReaction::PushPull,
                tracking_chunks: 8,
                update_interval: 3,
                payload_version: 1,
                required_features: Vec::new(),
            },
        ])
        .unwrap();
        let mut content = sunrise_mod_package::content::Content::declared("ruby");
        let mut entity: sunrise_mod_package::content::EntityType =
            serde_json::from_str(r#"{"template":"minecraft:zombie"}"#).unwrap();
        entity.id = "ruby:robot".into();
        content.entity_types.push(entity);
        let mut package = sunrise_client::mod_content::ContentMod {
            id: "ruby".into(),
            content,
            jars: Vec::new(),
        };
        assert_eq!(
            validate_entity_type_families(std::slice::from_ref(&package), &general),
            Err("mod ruby declares entity type ruby:robot as both a content and a general entity type".into()),
        );
        package.content.entity_types[0].id = "ruby:zombie".into();
        assert_eq!(validate_entity_type_families(&[package], &general), Ok(()));
    }

    /// A content mod's living type is a mob of its template's species under
    /// its own id, with its own attributes: it saves its id with every field
    /// its template's class saves, loads back as itself, and rolls its own
    /// loot table. A thrown type stays a projectile.
    #[test]
    fn a_mod_living_type_is_its_templates_mob_with_its_own_attributes() {
        use sunrise_client::mobs::{Mob, Species, drops::death_loot_table};
        use sunrise_core::geometry::Vec3;
        let types = living_mod_types();
        let (zombie, ghoul) = (&types["minecraft:zombie"], &types["ruby:ghoul"]);
        assert_eq!(ghoul.species, Species::Zombie);
        assert_eq!(ghoul.class_id(), "minecraft:zombie");
        assert_eq!(types["ruby:settler"].species, Species::Villager);
        assert!(!types.contains_key("ruby:bolt"), "a thrown type is no mob");
        assert_eq!((zombie.max_health, ghoul.max_health), (20.0, 40.0));
        assert_eq!(ghoul.speed.to_bits(), zombie.speed.to_bits());
        assert_eq!(
            (ghoul.sounds, ghoul.texture.as_str()),
            (zombie.sounds, zombie.texture.as_str())
        );

        let at = Vec3::new(0.5, 64.0, 0.5);
        let (zombie, ghoul) = (
            Mob::new(1, zombie.clone(), at, 0.0),
            Mob::new(2, ghoul.clone(), at, 0.0),
        );
        assert_eq!(ghoul.health, 40.0);
        let saved = |m: &Mob| {
            sunrise_client::entity_nbt::mob_to_nbt(
                m,
                [0, 0, 0, 2],
                None,
                None,
                |_| None,
                &Default::default(),
            )
        };
        let keys = |tag: &sunrise_core::nbt::Tag| -> Vec<String> {
            let sunrise_core::nbt::Tag::Compound(map) = tag else {
                panic!("a mob saves a compound");
            };
            map.keys().map(|k| k.to_string_lossy()).collect()
        };
        let (zombie_tag, ghoul_tag) = (saved(&zombie), saved(&ghoul));
        let Some(sunrise_core::nbt::Tag::String(id)) = ghoul_tag.get("id") else {
            panic!("a saved mob has its id");
        };
        assert_eq!(id.to_string_lossy(), "ruby:ghoul");
        assert_eq!(
            keys(&ghoul_tag),
            keys(&zombie_tag),
            "it saves what a zombie saves"
        );
        for key in ["IsBaby", "CanBreakDoors", "DrownedConversionTime"] {
            assert!(keys(&ghoul_tag).iter().any(|k| k == key), "{key}");
        }
        let loaded = sunrise_client::entity_nbt::mob_from_nbt(&ghoul_tag, &types, 3, None)
            .expect("it loads back");
        assert_eq!(
            (loaded.kind.id.as_str(), loaded.health),
            ("ruby:ghoul", 40.0)
        );
        assert_eq!(
            death_loot_table(&ghoul, true).as_deref(),
            Some("ruby:entities/ghoul")
        );
        assert_eq!(
            death_loot_table(&zombie, true).as_deref(),
            Some("minecraft:entities/zombie")
        );
    }

    /// A content mod's tameable type (#4031) is its template's mob: a wolf, a
    /// cat or a parrot, tamed and sitting, saves its owner, sitting and its
    /// template's own field (a collar's colour, a parrot's variant) under its
    /// own id and loads back as itself with all of them.
    #[test]
    fn a_mod_tameable_type_saves_and_loads_its_owner() {
        use sunrise_client::mobs::{Mob, Species};
        use sunrise_core::geometry::Vec3;
        let types = living_mod_types();
        let owner = [1, 2, 3, 4];
        use sunrise_core::nbt::Tag;
        for (id, species, own) in [
            ("ruby:hound", Species::Wolf, ("CollarColor", Tag::Byte(14))),
            ("ruby:tabby", Species::Cat, ("CollarColor", Tag::Byte(3))),
            ("ruby:macaw", Species::Parrot, ("Variant", Tag::Int(2))),
        ] {
            let kind = types[id].clone();
            assert_eq!(kind.species, species, "{id}");
            let mut pet = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            pet.extra.tame = true;
            pet.extra.owner = Some(owner);
            pet.extra.sitting = true;
            match species {
                Species::Parrot => pet.extra.variant = 2,
                _ => pet.extra.color = if species == Species::Wolf { 14 } else { 3 },
            }
            let tag = sunrise_client::entity_nbt::mob_to_nbt(
                &pet,
                [0, 0, 0, 1],
                None,
                None,
                |_| None,
                &Default::default(),
            );
            let get = |key: &str| tag.get(key).cloned();
            assert_eq!(
                get("id"),
                Some(sunrise_core::nbt::Tag::String(id.into())),
                "{id}"
            );
            assert_eq!(
                get("Owner"),
                Some(sunrise_core::nbt::Tag::IntArray(owner.to_vec())),
                "{id}"
            );
            assert_eq!(
                get("Sitting"),
                Some(sunrise_core::nbt::Tag::Byte(1)),
                "{id}"
            );
            assert_eq!(get(own.0), Some(own.1), "{id}");
            let loaded = sunrise_client::entity_nbt::mob_from_nbt(&tag, &types, 2, None)
                .expect("it loads back");
            assert_eq!(
                (
                    loaded.kind.id.as_str(),
                    loaded.species(),
                    loaded.extra.tame,
                    loaded.extra.owner,
                    loaded.extra.sitting,
                    (loaded.extra.color, loaded.extra.variant)
                ),
                (
                    id,
                    species,
                    true,
                    Some(owner),
                    true,
                    (pet.extra.color, pet.extra.variant)
                )
            );
        }
    }

    /// A mod's declared attribute set (#845, `EntityAttributeCreationEvent`)
    /// reaches everything a mob of the type is: the fields it spawns with,
    /// the finalize defaults and the live attribute map's catalog row —
    /// built from the template's, so the reads of #1469 and #1493 answer the
    /// declared values with no second copy.
    #[test]
    #[ignore = "daily"]
    fn a_mods_declared_entity_attributes_reach_spawn_and_the_live_map() {
        use sunrise_client::mobs::Mob;
        use sunrise_core::geometry::Vec3;
        use sunrise_world::attributes::entity::Catalog;

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &sunrise_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        let declared = [
            ("minecraft:max_health".to_string(), 55.0),
            ("minecraft:movement_speed".to_string(), 0.4),
        ];
        let merged = catalog
            .with_entity_row("ruby:ghoul", "minecraft:zombie", &declared)
            .unwrap();
        let base = |catalog: &Catalog, entity: &str, name: &str| {
            catalog.default_base(entity, catalog.id(name).unwrap())
        };
        // The declared values, and the template's row under them.
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:max_health"),
            Some(55.0)
        );
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:movement_speed"),
            Some(0.4)
        );
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:attack_damage"),
            base(&catalog, "minecraft:zombie", "minecraft:attack_damage")
        );
        // The live map builds for the type and answers the declared values.
        let live = sunrise_client::living_attributes::LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged.clone()),
                "ruby:ghoul",
                "minecraft:max_health",
                sunrise_world::attribute_map::ValueKind::Computed,
                Vec::new
            ),
            Some(55.0)
        );
        // The type's fields spawn with them: the merged row takes the type's
        // attribute map and fields (`take_attribute_row`).
        let mut types = living_mod_types();
        let ghoul = types.get_mut("ruby:ghoul").unwrap();
        Arc::make_mut(ghoul)
            .take_attribute_row(Arc::new(merged.entity_row("ruby:ghoul").unwrap()), &merged);
        let ghoul = Mob::new(7, types["ruby:ghoul"].clone(), Vec3::ZERO, 0.0);
        assert_eq!(ghoul.health, 55.0, "the spawn's health");
        assert_eq!(ghoul.attrs.max_health, 55.0);
        assert_eq!(ghoul.attrs.speed, 0.4);
        assert_eq!(
            ghoul.kind.attributes.get("minecraft:max_health"),
            Some(&55.0),
            "the finalize defaults read the declared value"
        );
    }

    /// #812: a mod's declared attribute type joins the catalog before the
    /// rows resolve (a mod entity's set names its own new attribute), and
    /// the live map, an item's modifier, the saves and the sync set carry it
    /// by string id — a guest's catalog built from the same mods decodes the
    /// same network id.
    #[test]
    #[ignore = "daily"]
    fn a_mods_declared_attribute_reaches_rows_the_live_map_saves_and_sync() {
        use sunrise_client::lan;
        use sunrise_client::living_attributes::{self, LivingAttributes};
        use sunrise_core::nbt::Tag;
        use sunrise_world::attribute_map::ValueKind;
        use sunrise_world::attributes::{
            Definition, Sentiment,
            entity::{AddedAttribute, Catalog},
        };

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &sunrise_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        let vanilla = catalog.attributes().count();
        // Two mods' attributes, declared in an order that is not the sorted
        // one; one syncable, one not.
        let declared = |mod_id: &str, id: &str, default, syncable, sentiment| AddedAttribute {
            mod_id: mod_id.into(),
            id: id.into(),
            definition: Definition::ranged(default, 0.0, 100.0).unwrap(),
            syncable,
            tooltip: sunrise_world::attributes::Tooltip {
                sentiment,
                ..Default::default()
            },
        };
        let grown = catalog
            .with_attributes(&[
                declared("zeta", "zeta:fury", 15.0, false, Sentiment::default()),
                declared("ruby", "ruby:mana", 10.0, true, Sentiment::Negative),
            ])
            .unwrap();
        // After every vanilla attribute, sorted by mod id: ruby's first
        // though zeta's declared first, and vanilla ids never shift.
        let (mana, fury) = (
            grown.id("ruby:mana").unwrap(),
            grown.id("zeta:fury").unwrap(),
        );
        assert_eq!(mana.registry_id() as usize, vanilla);
        assert_eq!(fury.registry_id() as usize, vanilla + 1);
        assert_eq!(
            grown.id("minecraft:max_health"),
            catalog.id("minecraft:max_health")
        );
        // A mod entity's row names its own new attribute (#845's rows
        // resolve after the attributes join).
        let merged = grown
            .with_entity_row(
                "ruby:ghoul",
                "minecraft:zombie",
                &[("ruby:mana".into(), 42.0), ("zeta:fury".into(), 7.0)],
            )
            .unwrap();
        let live = LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged.clone()),
                "ruby:ghoul",
                "ruby:mana",
                ValueKind::Computed,
                Vec::new
            ),
            Some(42.0)
        );
        // An item's `attribute_modifiers` entry naming the mod attribute
        // reaches the live map (the worn-stack path's own write).
        let mut map =
            living_attributes::new_live(&std::sync::Arc::new(merged.clone()), "ruby:ghoul")
                .unwrap();
        assert_eq!(map_value(&mut map, mana), 42.0);
        living_attributes::set_modifier(&mut map, "ruby:mana", "ruby:ring", 8.0, "add_value", true)
            .unwrap();
        assert_eq!(map_value(&mut map, mana), 50.0);
        // The sync set: a syncable attribute joins it, a non-syncable one
        // never does (`AttributeMap.getSyncableAttributes`).
        living_attributes::set_modifier(&mut map, "zeta:fury", "zeta:mark", 1.0, "add_value", true)
            .unwrap();
        let synced: std::collections::HashSet<i32> = lan::attributes::syncable(&map)
            .iter()
            .map(|s| s.attribute)
            .collect();
        assert!(synced.contains(&mana.registry_id()));
        assert!(!synced.contains(&fury.registry_id()));
        // Save → load round trip by string id.
        let saved = Tag::List(living_attributes::pack(&map, true));
        let mut again =
            living_attributes::new_live(&std::sync::Arc::new(merged.clone()), "ruby:ghoul")
                .unwrap();
        living_attributes::apply_saved(&mut again, Some(&saved)).unwrap();
        // The saved base; the ring's transient modifier is equipment, not
        // saved — as vanilla's `AttributeInstance.Packed` keeps permanent
        // modifiers only.
        assert_eq!(map_value(&mut again, mana), 42.0);
        // A save whose attribute the loaded catalog does not know is
        // skipped, as vanilla warns and skips: no panic, the map whole.
        let mut vanilla_map =
            living_attributes::new_live(&std::sync::Arc::new(catalog.clone()), "minecraft:zombie")
                .unwrap();
        living_attributes::apply_saved(&mut vanilla_map, Some(&saved)).unwrap();
        assert!(lan::attributes::syncable(&vanilla_map).is_empty());
        // The network id decodes on a guest's catalog built from the same
        // mods declaring in another order: the snapshot's numbers find the
        // same attributes.
        let guest_catalog = catalog
            .with_attributes(&[
                declared("ruby", "ruby:mana", 10.0, true, Sentiment::Negative),
                declared("zeta", "zeta:fury", 15.0, false, Sentiment::default()),
            ])
            .unwrap()
            .with_entity_row(
                "ruby:ghoul",
                "minecraft:zombie",
                &[("ruby:mana".into(), 42.0), ("zeta:fury".into(), 7.0)],
            )
            .unwrap();
        let snapshots = lan::attributes::syncable(&map);
        let mut guest =
            living_attributes::new_live(&std::sync::Arc::new(guest_catalog), "ruby:ghoul").unwrap();
        lan::attributes::apply(&mut guest, &snapshots);
        assert_eq!(map_value(&mut guest, mana), 50.0);

        fn map_value(
            map: &mut sunrise_client::living_attributes::Live,
            id: sunrise_world::attributes::entity::AttributeId,
        ) -> f64 {
            // The first read creates the instance, as `getInstance` does.
            let key = map
                .keys()
                .iter()
                .find(|key| key.attribute() == id)
                .cloned()
                .unwrap();
            let Ok(Some(handle)) = map.get_instance(Some(key)) else {
                panic!("the type has the attribute")
            };
            map.instance(&handle).unwrap().value()
        }
    }

    /// A declared set's merged row — the template's with the declared set on
    /// top — is the type's one record: a content row's own value (the
    /// ghoul's 40 health) gives way to the template's, so a mob of the type
    /// spawns with what the live attribute map answers (#845, review).
    #[test]
    #[ignore = "daily"]
    fn a_declared_type_spawns_with_its_merged_row_not_its_content_row() {
        use sunrise_client::mobs::Mob;
        use sunrise_core::geometry::Vec3;
        use sunrise_world::attributes::entity::Catalog;

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &sunrise_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        // Only the speed is declared: the merged row keeps the template's
        // health (a zombie's 20), not the content row's 40.
        let declared = [("minecraft:movement_speed".to_string(), 0.4)];
        let merged = catalog
            .with_entity_row("ruby:ghoul", "minecraft:zombie", &declared)
            .unwrap();
        let mut types = living_mod_types();
        let ghoul = types.get_mut("ruby:ghoul").unwrap();
        assert_eq!(ghoul.max_health, 40.0, "the content row's own value");
        Arc::make_mut(ghoul)
            .take_attribute_row(Arc::new(merged.entity_row("ruby:ghoul").unwrap()), &merged);
        let ghoul = Mob::new(7, types["ruby:ghoul"].clone(), Vec3::ZERO, 0.0);
        assert_eq!(
            ghoul.health, 20.0,
            "the template's health through the merged row"
        );
        assert_eq!(ghoul.attrs.max_health, 20.0);
        assert_eq!(ghoul.attrs.speed, 0.4, "the declared speed");
        let live = sunrise_client::living_attributes::LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged),
                "ruby:ghoul",
                "minecraft:max_health",
                sunrise_world::attribute_map::ValueKind::Computed,
                Vec::new
            ),
            Some(20.0),
            "the live map answers the same row"
        );
    }

    /// A mod that adds an attribute vanilla zombies lack (#846,
    /// `EntityAttributeModificationEvent`) changes the zombie type's default:
    /// a zombie spawned after load answers it through the live attribute map
    /// and carries it in its finalize defaults, while the rest of its row
    /// and its spawn fields stay the zombie's.
    #[test]
    fn a_mods_attribute_addition_reaches_new_vanilla_zombies() {
        use mod_api::{Hooks, ModEntry, Registrar, RegistrationContext};
        use sunrise_client::mobs::Mob;
        use sunrise_core::geometry::Vec3;
        use sunrise_world::attribute_map::ValueKind;
        use sunrise_world::attributes::entity::Catalog;

        struct Lucky;
        #[mod_api::__subscriptions]
        impl Hooks for Lucky {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                registrar
                    .modify_entity_attributes(
                        "minecraft:zombie",
                        vec![("minecraft:luck".to_string(), Some(2.5))],
                    )
                    .unwrap();
            }
        }
        crate::game::api_values::install_parts(&[ModEntry::new("lucky", Box::new(Lucky))]).unwrap();

        let bits = |n: f64| format!("{:016x}", n.to_bits());
        let catalog = Arc::new(
            Catalog::from_json(&serde_json::json!({
                "data_version": sunrise_world::DATA_VERSION,
                "attributes": [
                    {"id": "minecraft:max_health", "default": bits(20.0), "min": bits(1.0), "max": bits(1024.0), "syncable": true},
                    {"id": "minecraft:movement_speed", "default": bits(0.7), "min": bits(0.0), "max": bits(1024.0), "syncable": true},
                    {"id": "minecraft:luck", "default": bits(0.0), "min": bits(-1024.0), "max": bits(1024.0), "syncable": true}
                ],
                "entities": {
                    "minecraft:zombie": {
                        "minecraft:max_health": bits(20.0),
                        "minecraft:movement_speed": bits(0.23000000417232513)
                    }
                }
            }))
            .unwrap(),
        );
        let read = |catalog: &Arc<Catalog>, name: &str| {
            sunrise_client::living_attributes::LivingAttributes::default().read(
                catalog,
                "minecraft:zombie",
                name,
                ValueKind::Computed,
                Vec::new,
            )
        };
        assert_eq!(
            read(&catalog, "minecraft:luck"),
            None,
            "vanilla zombies lack it"
        );

        // core.motion, then rest.mobs, as the resource load runs them.
        let modifications = mod_api::__private::entity_attribute_modifications();
        assert_eq!(modifications.len(), 1);
        let added = Arc::new(
            super::super::resource_load::with_mod_attribute_additions(&catalog, modifications)
                .unwrap(),
        );
        let mut types: std::collections::HashMap<_, _> = living_mod_types()
            .into_iter()
            .map(|(id, kind)| (id, Arc::unwrap_or_clone(kind)))
            .collect();
        super::super::resource_load::take_modified_attribute_rows(
            &mut types,
            &added,
            modifications,
        );

        let zombie = Mob::new(
            7,
            Arc::new(types["minecraft:zombie"].clone()),
            Vec3::ZERO,
            0.0,
        );
        assert_eq!(read(&added, "minecraft:luck"), Some(2.5), "the live map");
        assert_eq!(
            zombie.kind.attributes.get("minecraft:luck"),
            Some(&2.5),
            "the finalize defaults"
        );
        assert_eq!(read(&added, "minecraft:max_health"), Some(20.0));
        assert_eq!(zombie.health, 20.0, "the spawn's health");
        assert_eq!(zombie.attrs.speed, 0.23000000417232513);
        // Types nobody modified keep their rows.
        assert_eq!(
            types["minecraft:villager"].attributes.get("minecraft:luck"),
            None
        );
    }

    /// An addition the catalog cannot place (an attribute it does not know,
    /// or a type with no attribute row) refuses the resource load, naming
    /// the type.
    #[test]
    fn a_mods_attribute_addition_to_an_unknown_row_refuses_the_load() {
        use mod_api::__private::EntityAttributeModification;
        use sunrise_world::attributes::entity::Catalog;

        let catalog = Catalog::from_json(&serde_json::json!({
            "data_version": sunrise_world::DATA_VERSION,
            "attributes": [
                {"id": "minecraft:luck", "default": "0000000000000000", "min": "c090000000000000", "max": "4090000000000000", "syncable": true}
            ],
            "entities": {"minecraft:zombie": {}}
        }))
        .unwrap();
        let addition = |entity: &str, attribute: &str| EntityAttributeModification {
            entity: entity.into(),
            attributes: vec![(attribute.into(), None)],
        };
        let error = super::super::resource_load::with_mod_attribute_additions(
            &catalog,
            &[addition("minecraft:zombie", "lucky:magic_resistance")],
        )
        .err()
        .unwrap();
        assert!(error.contains("minecraft:zombie"), "{error}");
        assert!(error.contains("lucky:magic_resistance"), "{error}");
        let error = super::super::resource_load::with_mod_attribute_additions(
            &catalog,
            &[addition("ruby:ghoul", "minecraft:luck")],
        )
        .err()
        .unwrap();
        assert!(error.contains("ruby:ghoul"), "{error}");
    }

    /// A content mod's type from the wandering trader, the neutral template
    /// whose behaviour is all goals: a trader under its own id, with a
    /// trader's goals, that saves what a trader saves and loads back.
    #[test]
    fn a_mod_trader_type_is_a_wandering_trader_under_its_own_id() {
        use sunrise_client::mobs::{Mob, Species};
        use sunrise_core::nbt::Tag;
        let types = living_mod_types();
        let merchant = &types["ruby:merchant"];
        assert_eq!(merchant.species, Species::WanderingTrader);
        assert_eq!(merchant.class_id(), "minecraft:wandering_trader");
        let at = sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5);
        let trader = Mob::new(1, types["minecraft:wandering_trader"].clone(), at, 0.0);
        let merchant = Mob::new(2, merchant.clone(), at, 0.0);
        assert!(merchant.ai.goals.has("Float") && trader.ai.goals.has("Float"));
        let saved = |m: &Mob| {
            sunrise_client::entity_nbt::mob_to_nbt(
                m,
                [0, 0, 0, 2],
                None,
                None,
                |_| None,
                &Default::default(),
            )
        };
        let keys = |tag: &Tag| -> Vec<String> {
            let Tag::Compound(map) = tag else {
                panic!("a mob saves a compound");
            };
            map.keys().map(|k| k.to_string_lossy()).collect()
        };
        let tag = saved(&merchant);
        assert_eq!(
            keys(&tag),
            keys(&saved(&trader)),
            "it saves what a trader saves"
        );
        let loaded =
            sunrise_client::entity_nbt::mob_from_nbt(&tag, &types, 3, None).expect("it loads back");
        assert_eq!(
            (loaded.kind.id.as_str(), loaded.species()),
            ("ruby:merchant", Species::WanderingTrader)
        );
    }

    #[test]
    fn fluid_models_recollection_changes_model_identifiers() {
        use mod_api::{FluidModel, Hooks, ModEntry, RegisterFluidModelsEvent};
        use mod_platform::testing::with_mods;
        use std::sync::atomic::{AtomicUsize, Ordering};
        #[derive(Default)]
        struct Models(Arc<AtomicUsize>);
        #[mod_api::sunrise_mod]
        impl Hooks for Models {
            fn register_fluid_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterFluidModelsEvent<'_>,
            ) {
                let generation = self.0.load(Ordering::SeqCst);
                let mut model = FluidModel::default();
                model.fluid = "reload_fluid:oil".into();
                model.still = Some(format!("reload_fluid:block/oil_{generation}"));
                event.register(model);
            }
        }
        let generation = Arc::new(AtomicUsize::new(0));
        with_mods(
            vec![ModEntry::new(
                "reload_fluid",
                Box::new(Models(generation.clone())),
            )]
            .into(),
            || {
                mod_platform::__private::with_test_packages(&["reload_fluid"], || {
                    sunrise_client::fluid_models::set_world_looks([
                        &sunrise_mod_package::content::FluidLook {
                            id: "reload_fluid:oil".into(),
                            sprite: None,
                            tint: None,
                            world: Some(Default::default()),
                        },
                    ]);
                    register_fluid_models(ExecutionRole::Client);
                    assert_eq!(
                        sunrise_client::fluid_models::resolved()[0].still,
                        "reload_fluid:block/oil_0"
                    );
                    generation.store(1, Ordering::SeqCst);
                    register_fluid_models(ExecutionRole::Client);
                    assert_eq!(
                        sunrise_client::fluid_models::resolved()[0].still,
                        "reload_fluid:block/oil_1"
                    );
                    // An item's fluid layer follows the same reload.
                    let item_still = || {
                        sunrise_client::fluid_models::item_appearance("reload_fluid:oil")
                            .map(|appearance| appearance.still)
                    };
                    assert_eq!(item_still().as_deref(), Some("reload_fluid:block/oil_1"));
                    sunrise_client::fluid_models::register(vec![]);
                    sunrise_client::fluid_models::set_world_looks([]);
                    assert_eq!(item_still(), None);
                });
            },
        );
    }

    #[test]
    fn fluid_models_collect_only_on_the_client() {
        use mod_api::FluidModel;
        use mod_api::{Hooks, ModEntry, RegisterFluidModelsEvent};
        use mod_platform::testing::with_mods;
        #[derive(Default)]
        struct Models;
        #[mod_api::sunrise_mod]
        impl Hooks for Models {
            fn register_fluid_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterFluidModelsEvent<'_>,
            ) {
                let mut model = FluidModel::default();
                model.fluid = "registration_test:oil".into();
                model.flowing = Some("registration_test:block/oil_flow".into());
                model.overlay = Some("registration_test:block/oil_overlay".into());
                event.register(model);
            }
        }
        with_mods(
            vec![ModEntry::new("registration_test", Box::new(Models))].into(),
            || {
                mod_platform::__private::with_test_packages(&["registration_test"], || {
                    for server in [
                        ExecutionRole::IntegratedServer,
                        ExecutionRole::DedicatedServer,
                    ] {
                        assert_eq!(collect_fluid_models(server), None);
                    }
                    let models = collect_fluid_models(ExecutionRole::Client).unwrap();
                    assert_eq!(models.len(), 1);
                    assert_eq!(models[0].fluid, "registration_test:oil");
                    assert_eq!(
                        models[0].flowing.as_deref(),
                        Some("registration_test:block/oil_flow")
                    );
                    assert_eq!(
                        models[0].overlay.as_deref(),
                        Some("registration_test:block/oil_overlay")
                    );
                });
            },
        );
    }

    fn smod(dir: &std::path::Path, id: &str) {
        let manifest = format!(
            "id = '{id}'\nname = '{id}'\nversion = '0.3.0'\napi = '0.2'\nside = 'both'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let files: &[(&str, &[u8])] = &[
            (
                "Cargo.toml",
                b"[package]\nname='different_crate'\nversion='9.9.9'\n",
            ),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        let bytes = sunrise_mod_package::Package::write_files(&manifest, files).unwrap();
        std::fs::write(dir.join(format!("{id}.smod")), bytes).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn content_overlay_validates_dependencies_and_orders_layers() {
        const MODE: &str = "SUNRISE_CONTENT_DEPENDENCY_TEST";
        if std::env::var_os(MODE).is_none() {
            run_in_child(
                "game::mods::tests::content_overlay_validates_dependencies_and_orders_layers",
                MODE,
            );
            return;
        }
        let f = Fixture::new("content-dependencies");
        let write = |id: &str, version: &str, dependencies: &str| {
            let dir = f.0.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("sunrise-mod.toml"), format!(
                "id='{id}'\nname='{id}'\nversion='{version}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{dependencies}"
            )).unwrap();
            dir
        };
        let addon = write("alpha", "1", "[dependencies]\nzbase='^1'");
        let base_mod = write("zbase", "1", "");
        let base = f.0.join("resources");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        let mut sources = ModsState {
            dev: vec![addon.clone(), base_mod.clone()],
            ..Default::default()
        };
        let enabled = Default::default();
        sources.select(&f.0, &enabled);
        let overlay = content_overlay(&base, &sources, &f.0.join("jars")).unwrap();
        assert_eq!(
            overlay
                .mods
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["zbase", "alpha"]
        );
        sources.dev = vec![addon.clone()];
        sources.select(&f.0, &enabled);
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("alpha") && error.contains("zbase"));
        write(
            "alpha",
            "1",
            "[dependencies]\nzbase={version='^1', optional=true}",
        );
        sources.select(&f.0, &enabled);
        assert!(content_overlay(&base, &sources, &f.0.join("jars")).is_ok());
        write("zbase", "2", "");
        sources.dev.push(base_mod.clone());
        sources.select(&f.0, &enabled);
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("installed version is 2"));
        write("zbase", "1", "[dependencies]\nalpha='^1'");
        sources.select(&f.0, &enabled);
        assert!(
            content_overlay(&base, &sources, &f.0.join("jars"))
                .err()
                .unwrap()
                .to_string()
                .contains("cycle")
        );
    }

    /// A `side = "server"` content package goes through the real loader: a registry
    /// declaration fails the load by name even when it would not parse, and assets load.
    #[test]
    #[ignore = "daily"]
    fn a_server_package_is_refused_at_load_for_registry_files_and_loads_with_assets() {
        const MODE: &str = "SUNRISE_SERVER_PACKAGE_TEST";
        if std::env::var_os(MODE).is_none() {
            run_in_child(
                "game::mods::tests::a_server_package_is_refused_at_load_for_registry_files_and_loads_with_assets",
                MODE,
            );
            return;
        }
        let f = Fixture::new("server-package");
        let package = f.0.join("srv");
        let write = |file: &str, text: &str| {
            let path = package.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "sunrise-mod.toml",
            "id='srv'\nname='srv'\nversion='1'\napi='0.2'\nside='server'\ncapabilities=['registries']\n",
        );
        write("data/srv/sunrise/item/gem.json", "{ not json");
        // Merging a mod's assets reads the prepared root's block reports, so this is the real one.
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let mut sources = ModsState {
            dev: vec![package.clone()],
            ..Default::default()
        };
        let enabled = Default::default();
        sources.select(&f.0, &enabled);
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap()
            .to_string();
        assert_eq!(
            error,
            "srv: a side = \"server\" mod carries data/srv/sunrise/item/gem.json, which every \
             client must have; declare side = \"both\""
        );
        std::fs::remove_dir_all(package.join("data")).unwrap();
        write("assets/srv/lang/en_us.json", "{}");
        sources.select(&f.0, &enabled);
        let overlay = content_overlay(&base, &sources, &f.0.join("jars")).unwrap();
        assert_eq!(
            overlay
                .mods
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["srv"]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn native_content_follows_the_build_and_misplaced_content_follows_its_selection() {
        const MODE: &str = "SUNRISE_MODS_SELECTION_TEST";
        let mode = std::env::var(MODE).ok();
        if mode.is_none() {
            // Inventory and content registries are process-wide, as in a real startup.
            for mode in ["compiled", "vanilla"] {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .current_dir(std::env::temp_dir())
                    .args(["--include-ignored", "--exact", "game::mods::tests::native_content_follows_the_build_and_misplaced_content_follows_its_selection", "--nocapture"])
                    .env(MODE, mode).status().unwrap();
                assert!(status.success());
            }
            return;
        }
        let f = Fixture::new(mode.as_deref().unwrap());
        smod(&f.0, "machine_sample");
        let native = if mode.as_deref() == Some("compiled") {
            f.runtime()
        } else {
            Runtime::default()
        };
        let here = f.0.join("unrelated-game");
        let misplaced = here.join(mod_list::NATIVE_DIR);
        std::fs::create_dir_all(&misplaced).unwrap();
        smod(&misplaced, "machine_sample");
        let manifest = "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n";
        let plain = sunrise_mod_package::Package::write_files(manifest, &[]).unwrap();
        std::fs::write(misplaced.join("plain.smod"), plain).unwrap();
        let base = f.0.join("resources");
        std::fs::create_dir_all(&base).unwrap();
        // A prepared root's registry report: block entity registration reads it (B1).
        std::fs::write(
            base.join("registries.json"),
            "{\"minecraft:block_entity_type\":{\"entries\":{}}}",
        )
        .unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        std::fs::write(base.join("block-behaviours.json"), r#"{"fluids":[]}"#).unwrap();
        std::env::set_current_dir(&here).unwrap();
        let mut sources = ModsState {
            native: std::sync::Arc::new(native),
            ..Default::default()
        };
        let enabled = std::collections::BTreeSet::from(["plain".to_owned()]);
        sources.select(&here, &enabled);
        let generated = content_root(&base, &sources).unwrap();
        assert_ne!(generated.prepared(), base);
        assert_eq!(
            sunrise_mod_package::enabled::EnabledMods::read(generated.prepared())
                .unwrap()
                .server_ids()
                .collect::<Vec<_>>(),
            ["plain"],
        );
        let local = sunrise_protocol::mod_list::local();
        let ids: Vec<_> = local.iter().map(|m| m.id.as_str()).collect();
        if mode.as_deref() == Some("compiled") {
            assert_eq!(ids, ["machine_sample", "plain"]);
            assert_eq!(local[0].version, "0.3.0");
        } else {
            assert_eq!(ids, ["plain"]);
        }
        // Out of the fixture: Windows won't remove a process's working directory.
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_shared_missing_jar_stops_native_load_and_refuses_each_content_mod_row() {
        const MODE: &str = "SUNRISE_JAR_REFUSAL_TEST";
        if std::env::var_os(MODE).is_none() {
            // Native declarations, like the real startup, belong to one process.
            run_in_child(
                "game::mods::tests::a_shared_missing_jar_stops_native_load_and_refuses_each_content_mod_row",
                MODE,
            );
            return;
        }
        let f = Fixture::new("shared-jar-refusal");
        let jar = sunrise_mod_package::manifest::JarRef::new(
            "original".into(),
            "1".into(),
            "9".repeat(128),
        )
        .unwrap();
        for id in ["machine_sample", "alpha", "beta", "gamma"] {
            let native = id == "machine_sample";
            let mut text = format!(
                "id='{id}'\nname='{id}'\nversion='0.3.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n"
            );
            if native {
                text.push_str(&format!(
                    "[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
                    "a".repeat(40),
                    "b".repeat(40)
                ));
            }
            if id != "gamma" {
                text.push_str(&format!(
                    "[[jar]]\nid='original'\nversion='1'\nsha512='{}'\n",
                    jar.sha512()
                ));
            }
            let code: &[(&str, &[u8])] = if native {
                &[
                    ("Cargo.toml", b"[package]\nname='example'\nversion='1'\n"),
                    ("src/lib.rs", b"pub fn hooks() {}"),
                ]
            } else {
                &[]
            };
            std::fs::write(
                f.0.join(format!("{id}.smod")),
                sunrise_mod_package::Package::write_files(&text, code).unwrap(),
            )
            .unwrap();
        }
        let selected: Arc<[sunrise_mod_package::selection::SelectedPackage]> =
            ["alpha", "beta", "gamma"]
                .into_iter()
                .map(|id| {
                    let path = f.0.join(format!("{id}.smod"));
                    sunrise_mod_package::selection::SelectedPackage {
                        package: sunrise_mod_package::Package::read(&path).unwrap(),
                        path,
                    }
                })
                .collect();
        let native = ModsState {
            native: Arc::new(f.runtime()),
            selected: selected.clone(),
            ..Default::default()
        };
        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        let error = content_overlay(&base, &native, &store).err().unwrap();
        let missing = error.downcast_ref::<MissingJar>().unwrap();
        assert_eq!(missing.owners, ["machine_sample", "alpha", "beta"]);
        assert_eq!(missing.jar, jar);
        assert_eq!(missing.path, jar.cache_path(&store));
        assert!(!store.exists());

        let content = ModsState {
            selected,
            ..Default::default()
        };
        let overlay = content_overlay(&base, &content, &store).unwrap();
        assert_ne!(overlay.root.prepared(), base);
        assert_eq!(
            sunrise_mod_package::enabled::EnabledMods::read(overlay.root.prepared())
                .unwrap()
                .server_ids()
                .collect::<Vec<_>>(),
            ["gamma"],
        );
        assert_eq!(
            overlay
                .mods
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["gamma"]
        );
        let mut rows: Vec<_> = ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|id| Installed {
                id: id.into(),
                title: id.into(),
                description: String::new(),
                problem: None,
                dev: false,
                native: None,
            })
            .collect();
        content.report_refusals(&mut rows);
        for row in &rows[..2] {
            let problem = row.problem.as_deref().unwrap();
            assert!(problem.contains("alpha, beta: original 1:"));
            assert!(problem.contains(&jar.cache_path(&store).display().to_string()));
        }
        assert!(rows[2].problem.is_none());
    }

    #[test]
    fn the_crash_report_lists_the_embedded_native_mods() {
        let built = [
            BUILT[0],
            BuiltMod {
                id: "pipes",
                version: "2.0",
                side: mod_api::Side::Client,
            },
        ];
        assert_eq!(
            native_mods_detail(&built).as_deref(),
            Some("machine_sample 0.3.0, pipes 2.0")
        );
        assert_eq!(native_mods_detail(&[]), None);
    }

    #[test]
    fn native_mods_installed_but_not_built_get_a_row_saying_so() {
        let f = Fixture::new("not-built");
        for id in ["veinminer", "zoom", "minimap", "machine_sample"] {
            smod(&f.0, id);
        }
        std::fs::write(f.0.join("notes.txt"), "not a mod").unwrap();
        std::fs::write(f.0.join("broken.smod"), "not a zip").unwrap();
        let rows = not_built(&f.0, &[], &[]);
        // The file that isn't a bundle gets a row named after it, saying why, with no Compile.
        let (broken, rows): (Vec<_>, Vec<_>) = rows.into_iter().partition(|r| r.id.is_empty());
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].title, "broken.smod");
        assert!(
            broken[0]
                .problem
                .as_deref()
                .is_some_and(|p| p.starts_with("Not a native mod Starling can read"))
        );
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["machine_sample", "minimap", "veinminer", "zoom"]);
        assert_eq!(rows[2].title, "Veinminer 0.3.0");
        assert_eq!(rows[2].problem.as_deref(), Some(mod_list::NOT_BUILT));
        assert!(rows.iter().all(|r| r.native.is_none() && !r.dev));
        let listed = Installed {
            id: "minimap".into(),
            ..rows[0].clone()
        };
        // The broken file's row, listed already, isn't added again when the page reads the folder.
        let rows = not_built(&f.0, BUILT, &[listed, broken[0].clone()]);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["veinminer", "zoom"]);
        assert!(not_built(&f.0.join("missing"), &[], &[]).is_empty());
    }

    /// A root with mods that declare no mob effects leaves the registry open for the first one
    /// that does.
    #[test]
    fn a_root_without_mob_effects_does_not_freeze_the_registry() {
        use sunrise_client::{brewing, mod_content::ContentMod};
        // Another test may have set the process-wide list already.
        if !brewing::mod_effects().is_empty() {
            return;
        }
        let f = Fixture::new("no-effects");
        std::fs::write(
            f.0.join("sunrise-mod.toml"),
            "id = 'plain'\nname = 'plain'\nversion = '1.0'\napi = '0.2'\ncapabilities = ['registries']\n",
        )
        .unwrap();
        let package = sunrise_mod_package::Package::read(&f.0).unwrap();
        let plain = ContentMod {
            id: "plain".into(),
            content: sunrise_mod_package::content::Content::read(&f.0, &package.manifest).unwrap(),
            jars: Vec::new(),
        };
        register_mob_effects(&[plain], &[]);
        // The list `potions::tests` registers, so it holds whichever test runs first.
        assert!(brewing::register_mod_effects([(
            "test:effect".to_owned(),
            0,
            brewing::Category::Neutral,
            None,
            "test-potions".into()
        )]));
    }
}

#[cfg(test)]
mod concurrent_content_tests {
    use super::*;
    #[test]
    fn mods_read_at_once_keep_their_order_and_their_own_errors() {
        let dir = std::env::temp_dir().join(format!("sunrise-read-at-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths: Vec<PathBuf> = (0..12).map(|i| dir.join(format!("m{i}"))).collect();
        for (i, path) in paths.iter().enumerate() {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(
                path.join("sunrise-mod.toml"),
                format!(
                    "id = \"m{i}\"\nname = \"M\"\nversion = \"0.1.0\"\napi = \"0.2\"\n\
                     side = \"both\"\ncapabilities = [\"registries\"]\n"
                ),
            )
            .unwrap();
            // Mod i has i + 1 language files; mod 5 also writes into another mod's namespace.
            let lang = path.join(format!("assets/m{i}/lang"));
            std::fs::create_dir_all(&lang).unwrap();
            for n in 0..=i {
                std::fs::write(lang.join(format!("l{n}.json")), "{}").unwrap();
            }
            if i == 5 {
                std::fs::create_dir_all(path.join("assets/m0/lang")).unwrap();
                std::fs::write(path.join("assets/m0/lang/x.json"), "{}").unwrap();
            }
        }
        let packages: Vec<(&PathBuf, sunrise_mod_package::Package)> = paths
            .iter()
            .map(|p| (p, sunrise_mod_package::Package::read(p).unwrap()))
            .collect();
        let requests: Vec<_> = packages
            .iter()
            .map(|(path, package)| ContentRequest {
                path,
                manifest: &package.manifest,
                declared: Vec::new(),
                admitted: None,
            })
            .collect();
        let read = read_contents(&requests);
        assert_eq!(read.len(), 12);
        for (i, content) in read.iter().enumerate() {
            if i == 5 {
                assert!(content.as_ref().unwrap_err().contains("own namespaces"));
                continue;
            }
            let files = &content.as_ref().unwrap().files;
            assert_eq!(files.len(), i + 1, "mod {i}");
            assert!(
                files
                    .iter()
                    .all(|f| f.starts_with(&format!("assets/m{i}/")))
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod server_side_tests {
    use mod_api::*;

    struct Reloads;
    #[mod_api::__subscriptions]
    impl Hooks for Reloads {
        fn add_client_reload_listeners(
            &self,
            _: &dyn ClientReloadListenerContext,
            _: &mut AddClientReloadListenersEvent<'_>,
        ) {
        }
    }
    struct Hider;
    #[mod_api::sunrise_part]
    impl BlockPart for Hider {
        fn block_render_visibility(&self, _: &dyn ClientHost, _: &mut BlockRenderVisibilityEvent) {}
    }
    struct HidesLogs;
    #[mod_api::__subscriptions]
    impl Hooks for HidesLogs {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.block_part_tag("minecraft:logs", Hider);
        }
    }
    struct Pings;
    #[mod_api::__subscriptions]
    impl Hooks for Pings {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let mut payloads = r.payload_handlers();
            let mut required = payloads.registrar("1");
            required.play_to_client(PayloadId("pings:ping".into()), None);
            required.play_to_client(PayloadId("pings:pong".into()), None);
            required
                .optional()
                .play_to_client(PayloadId("pings:hint".into()), None);
        }
    }
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.payload_handlers()
                .registrar("1")
                .optional()
                .play_to_client(PayloadId("quiet:hint".into()), None);
        }
    }

    /// What a joining client must have is named for its own mod only: a client hook, a part
    /// listening to one, the first required clientbound payload. An optional payload is not.
    #[test]
    fn what_a_client_must_have_is_named_for_its_mod_only() {
        let mods: Mods = vec![
            ModEntry::new("reloads", Box::new(Reloads)),
            ModEntry::new("hides_logs", Box::new(HidesLogs)),
            ModEntry::new("pings", Box::new(Pings)),
            ModEntry::new("quiet", Box::new(Quiet)),
        ]
        .into();
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            assert_eq!(
                mod_api::__private::client_requirement("reloads").unwrap(),
                "listens to the client hook add_client_reload_listeners"
            );
            assert_eq!(
                mod_api::__private::client_requirement("hides_logs").unwrap(),
                format!(
                    "attaches the part {}, which listens to the client hook block_render_visibility",
                    std::any::type_name::<Hider>()
                )
            );
            assert_eq!(
                mod_api::__private::client_requirement("pings").unwrap(),
                "sends the required payload pings:ping to clients"
            );
            assert_eq!(mod_api::__private::client_requirement("quiet"), None);
        });
    }
}
