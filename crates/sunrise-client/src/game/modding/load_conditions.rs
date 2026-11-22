//! Load-scoped registration and per-candidate condition lookup.
use super::Game;
use mod_api::__private::game::RegistrationContextNew;
use std::{collections::BTreeMap, sync::Arc};
pub(super) type ConditionTable =
    Arc<BTreeMap<mod_api::Identifier, (String, Arc<dyn mod_api::LoadCondition>)>>;
impl Game<'_> {
    pub(super) fn begin_condition_reload(
        &mut self,
        stack: &super::functions::PackStack,
        resources: &std::path::Path,
    ) -> Result<
        super::server_reload::PreparedServerReload,
        mod_api::__private::reload::ReloadStageError,
    > {
        use mod_api::__private::reload::ReloadStageError;
        if self.mods.load_condition_candidate.is_none() {
            let inventory = sunrise_mod_package::enabled::EnabledMods::read(resources)
                .map_err(|error| ReloadStageError::DataSource(error.to_string()))?;
            self.mods.load_condition_candidate = Some(
                conditions_for_mods(&self.mods, &inventory)
                    .map_err(ReloadStageError::DataSource)?,
            );
        }
        self.prepare_server_reload(stack)
    }
    pub(super) fn prepare_pack_condition_reload(
        &mut self,
        stack: &super::functions::PackStack,
    ) -> Result<
        super::server_reload::PreparedServerReload,
        mod_api::__private::reload::ReloadStageError,
    > {
        self.mods.load_condition_candidate = None;
        self.begin_condition_reload(stack, &self.options.resources.clone())
    }

    pub(super) fn stage_condition_inventory(
        &mut self,
    ) -> sunrise_client::Result<Option<sunrise_assets::resource_source::ResourceRoot>> {
        self.protect_condition_root();
        self.mods.load_condition_candidate = None;
        let inventory = sunrise_mod_package::enabled::EnabledMods::read(&self.options.resources)?;
        self.mods.load_condition_candidate = Some(conditions_for_mods(&self.mods, &inventory)?);
        self.stage_enabled_inventory()
    }

    pub(super) fn protect_condition_root(&mut self) {
        self.mods.active_condition_root = Some(sunrise_client::mod_content::retain_root(
            &self.options.resources,
        ));
    }

    pub(super) fn condition_root_needs_reload(&self) -> bool {
        self.mods
            .load_condition_candidate
            .as_ref()
            .is_some_and(|view| view.has_registered())
            && (!self.mods.selected.is_empty()
                || !self.mods.dev.is_empty()
                || !self.mods.native.packages().is_empty())
    }

    /// The conditions of a load from `root` over the candidate pack `files`.
    pub(super) fn registered_load_conditions(
        &self,
        inventory: &sunrise_mod_package::enabled::EnabledMods,
        stack: &super::functions::PackStack,
        root: &std::path::Path,
        files: &[(String, Vec<Vec<u8>>)],
    ) -> Result<sunrise_world::conditions::LoadConditions, String> {
        // `ICondition.IContext`: the world's feature flags and every registry's
        // entries as this candidate holds them. Tags join later, once this load has bound them.
        let features = || stack.features.iter().map(String::as_str);
        // Admission of a pack entry reads every condition but the registries this builds. It
        // shares the load's verdicts, so a condition that reads no registry runs once per file.
        let base = conditions_for_mods(&self.mods, inventory)?;
        let admission = base.clone().with_features(features());
        // DataStore's fixed content folders omit mod-declared registry folders.
        // Include those entries from this candidate's packs before conditions read them.
        let registry_files = candidate_registry_files(stack)?;
        let registries =
            candidate_registries(root, files.iter().chain(&registry_files), &admission);
        Ok(base.with_features(features()).with_registries(
            registries
                .iter()
                .map(|(registry, ids)| (registry.as_str(), ids.iter().map(String::as_str))),
        ))
    }
}

/// Read declared registry folders with the same scanner as the live registry loader.
/// Convert its resource paths to the content keys candidate_registries consumes.
fn candidate_registry_files(
    stack: &super::functions::PackStack,
) -> Result<Vec<(String, Vec<Vec<u8>>)>, String> {
    let declarations = mod_api::__private::datapack_registry_declarations();
    if declarations.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<_> = declarations
        .iter()
        .filter_map(|declaration| declaration.key.split_once(':'))
        .collect();
    let packs: Vec<_> = std::iter::once(&stack.content)
        .chain(stack.enabled_packs())
        .collect();
    // Admission is evaluated below, against the load-scoped conditions; no fallback.
    let files = sunrise_client::datapack::registry_files(&packs, &keys, |_, _| true)
        .map_err(|error| format!("{error:?}"))?
        .into_iter()
        .flat_map(|(file, versions)| {
            let bytes: Vec<Vec<u8>> = versions.into_iter().map(|(_, bytes)| bytes).collect();
            // A file under nested declared folders is an entry of each, as the live loader reads it.
            let entries: Vec<String> = file
                .strip_prefix("data/")
                .and_then(|rest| rest.split_once('/'))
                .map(|(namespace, path)| {
                    keys.iter()
                        .filter_map(|(ns, key)| {
                            let folder = format!("{ns}/{key}");
                            path.strip_prefix(&folder)
                                .and_then(|entry| entry.strip_prefix('/'))
                                .and_then(|entry| entry.strip_suffix(".json"))
                                .map(|entry| format!("{folder}/{namespace}:{entry}"))
                        })
                        .collect()
                })
                .unwrap_or_default();
            entries.into_iter().map(move |key| (key, bytes.clone()))
        })
        .collect();
    Ok(files)
}

/// Every registry's entries as the candidate load holds them, which `neoforge:registered` reads:
/// the built-in ones the prepared root exports (`command-registries.json`, never what an earlier
/// load added), and the entries the candidate's pack `files` add to a registry that takes them
/// (vanilla's, or a datapack registry a mod declared). So an id a pack adds counts on its first
/// load, and one a pack no longer has stops counting on the next.
fn candidate_registries<'a>(
    root: &std::path::Path,
    files: impl IntoIterator<Item = &'a (String, Vec<Vec<u8>>)>,
    admission: &sunrise_world::conditions::LoadConditions,
) -> BTreeMap<String, std::collections::BTreeSet<String>> {
    let export: serde_json::Value = std::fs::read(root.join("command-registries.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let mut registries: BTreeMap<String, std::collections::BTreeSet<String>> =
        export["by_registry"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(registry, entry)| {
                let ids = entry["ids"].as_array().into_iter().flatten();
                let ids = ids.filter_map(serde_json::Value::as_str).map(str::to_owned);
                (registry.clone(), ids.collect())
            })
            .collect();
    for declaration in mod_api::__private::datapack_registry_declarations().iter() {
        registries.entry(declaration.key.clone()).or_default();
    }
    // A pack file's key is `<folder>/<namespace>:<path>`: `worldgen/biome/test:dunes` for
    // `minecraft:worldgen/biome`, `mymod/spell/test:fire` for a mod's `mymod:spell`.
    let folder = |registry: &str| match registry.split_once(':') {
        Some(("minecraft", path)) => path.to_owned(),
        Some((namespace, path)) => format!("{namespace}/{path}"),
        None => registry.to_owned(),
    };
    let by_folder: BTreeMap<String, String> = registries
        .keys()
        .map(|registry| (folder(registry), registry.clone()))
        .collect();
    for (key, versions) in files {
        let Some(slash) = key.find(':').and_then(|colon| key[..colon].rfind('/')) else {
            continue;
        };
        let Some(registry) = by_folder.get(&key[..slash]) else {
            continue;
        };
        // The winning registry file, if its own conditions hold: an entry
        // they turn off is not there. Non-registry files belong to their loaders.
        // The path the loaders test it under (`data/<namespace>/<folder>/<path>.json`).
        let file_path = key[slash + 1..]
            .split_once(':')
            .map(|(namespace, path)| format!("data/{namespace}/{}/{path}.json", &key[..slash]))
            .unwrap_or_else(|| key.clone());
        let admitted = versions.last().is_some_and(|bytes| {
            let Ok(file) = serde_json::from_slice::<serde_json::Value>(bytes) else {
                return true;
            };
            let payload = file.get("neoforge:conditions").map(ToString::to_string);
            payload.is_none_or(|payload| {
                payload.contains("\"neoforge:registered\"")
                    || payload.contains("\"neoforge:tag_empty\"")
                    // An admission probe is not the entry's loader. A codec
                    // error omits it here; the owning loader will report it.
                    || admission.allows_file_isolated(&file, &file_path).unwrap_or(false)
            })
        });
        if !admitted {
            continue;
        }
        let id = key[slash + 1..].to_owned();
        registries
            .get_mut(registry)
            .expect("named above")
            .insert(id);
    }
    registries
}

pub(super) fn conditions_for_mods(
    sources: &super::mods::ModsState,
    inventory: &sunrise_mod_package::enabled::EnabledMods,
) -> Result<sunrise_world::conditions::LoadConditions, String> {
    if let Some(candidate) = &sources.load_condition_candidate {
        return Ok(candidate.with_mod_inventory(inventory.server_ids()));
    }
    let table = sources
        .load_conditions
        .get_or_init(|| {
            let mut registrar = mod_api::LoadConditionDeclarations::new(
                sunrise_world::conditions::is_builtin_condition,
            );
            Game::native_register_load_conditions(
                &mod_api::RegistrationContext::new(),
                &mut registrar,
            );
            registrar
                .finish()
                .map(Arc::new)
                .map_err(|error| error.message)
        })
        .clone()?;
    let conditions = sunrise_world::conditions::LoadConditions::new(inventory.server_ids());
    Ok(
        conditions.with_lookup(table.iter().map(|(id, (owner, handler))| {
            let name = id.to_string();
            let enabled = sources.code_owner_enabled(owner);
            let owner = owner.clone();
            let handler = handler.clone();
            (name.clone(), move |args: &serde_json::Value| {
                if !enabled {
                    return Ok(false);
                }
                let mut arguments = args.as_object().cloned().expect("decoded condition object");
                arguments.remove("type");
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handler.test(&mod_api::serde_json::Value::Object(arguments))
                }))
                .map_err(|_| format!("condition {name} (mod {owner}): handler panicked"))
                .and_then(|answer| {
                    answer.map_err(|error| {
                        format!("condition {name} (mod {owner}): {}", error.message)
                    })
                })
            })
        })),
    )
}

#[cfg(test)]
#[path = "load_conditions/startup_tests.rs"]
mod startup_tests;

#[cfg(test)]
#[path = "load_conditions/tests.rs"]
mod tests;

pub(super) fn content_root_with_conditions(
    root: &std::path::Path,
    jars: &std::path::Path,
    mods: &[sunrise_client::mod_content::ContentMod],
    cache: &std::path::Path,
    inventory: &sunrise_mod_package::enabled::EnabledMods,
    sources: &super::mods::ModsState,
) -> sunrise_client::Result<std::path::PathBuf> {
    let conditions = conditions_for_mods(sources, inventory)?;
    sunrise_client::mod_content::resource_root_with_conditions(
        root,
        jars,
        mods,
        cache,
        inventory,
        &conditions,
    )
}

#[cfg(test)]
#[path = "load_conditions/reload_tests.rs"]
mod reload_tests;
