//! Static serializer resources adapted to the owning server loot generation.
use super::api_values::{TryIntoApi, TryIntoEngine};
use mod_api::{
    GlobalLootModifier, GlobalLootModifierSerializer, Identifier, LootModifierError,
    LootModifierSettings,
};
use std::{collections::HashMap, path::Path, sync::Arc};
use sunrise_assets::resource_source::ResourceSource;
use sunrise_world::{
    loot::{
        self,
        global_modifiers::{
            AddTableModifier, DecodedModifier, GlobalLootModifiers, ModifierApply, ModifierFile,
            ModifierRoll,
        },
    },
    native_load_context::LoadContext,
};

pub(super) struct Serializers(HashMap<String, Arc<dyn GlobalLootModifierSerializer>>);
impl Serializers {
    pub(super) fn capture() -> Self {
        Self(
            mod_api::__private::global_loot_modifier_serializer_declarations()
                .into_iter()
                .collect(),
        )
    }
    fn decode(
        &self,
        kind: &str,
        data: &serde_json::Value,
        load: &LoadContext,
    ) -> Result<(DecodedModifier, Arc<dyn GlobalLootModifier>), String> {
        if kind == "neoforge:add_table" {
            let settings = LootModifierSettings::decode(data).map_err(|error| error.to_string())?;
            let table = data
                .get("table")
                .and_then(serde_json::Value::as_str)
                .ok_or("table: expected a loot table id")?;
            let table = Identifier::parse(table).map_err(|error| format!("table: {error}"))?;
            let decoded = DecodedModifier {
                priority: settings.priority,
                condition: settings.condition.clone(),
                apply: Arc::new(AddTableModifier {
                    table: table.to_string(),
                }),
            };
            return Ok((decoded, Arc::new(ApiAddTable { settings, table })));
        }
        let serializer = self
            .0
            .get(kind)
            .ok_or_else(|| format!("type: unknown global loot modifier serializer {kind}"))?;
        let reader = sunrise_client_render::api_context::LoadReader(load);
        let instance = serializer
            .decode(data, &reader.context())
            .map_err(|error| error.to_string())?;
        let decoded = DecodedModifier {
            priority: instance.priority().map_err(|error| error.to_string())?,
            condition: instance.condition().map_err(|error| error.to_string())?,
            apply: Arc::new(Adapter(instance.clone())),
        };
        Ok((decoded, instance))
    }
    pub(super) fn prepare(
        &self,
        files: Vec<ModifierFile>,
        conditions: &sunrise_world::conditions::LoadConditions,
        load: &LoadContext,
    ) -> Result<
        (
            Arc<GlobalLootModifiers>,
            Arc<mod_api::GlobalLootModifierManager>,
        ),
        String,
    > {
        use mod_api::__private::game::GlobalLootModifierManagerNew;
        let instances = std::cell::RefCell::new(HashMap::new());
        let generation = Arc::new(GlobalLootModifiers::load(
            files,
            conditions,
            |id, kind, data| {
                let (decoded, instance) = self.decode(kind, data, load)?;
                instances.borrow_mut().insert(id.to_owned(), instance);
                Ok(decoded)
            },
            |error| log::error!("{error}"),
        ));
        let mut instances = instances.into_inner();
        let entries = generation
            .ids()
            .map(|id| {
                let key = Identifier::parse(id).map_err(|error| error.to_string())?;
                let instance = instances
                    .remove(id)
                    .ok_or_else(|| format!("prepared modifier {id} missing its API resource"))?;
                Ok((key, instance))
            })
            .collect::<Result<_, String>>()?;
        let manager = Arc::new(mod_api::GlobalLootModifierManager::prepared(
            entries,
            generation.clone(),
        ));
        Ok((generation, manager))
    }
}
struct ApiAddTable {
    settings: LootModifierSettings,
    table: Identifier,
}
impl GlobalLootModifier for ApiAddTable {
    fn priority(&self) -> Result<i32, LootModifierError> {
        Ok(self.settings.priority)
    }
    fn condition(&self) -> Result<Option<serde_json::Value>, LootModifierError> {
        Ok(self.settings.condition.clone())
    }
    fn apply(
        &self,
        context: &mut dyn mod_api::LootModifierContext,
        generated: Vec<mod_api::LootStack>,
    ) -> Result<Vec<mod_api::LootStack>, LootModifierError> {
        let appended = context.roll_raw(&self.table)?;
        Ok(sunrise_world::loot::global_modifiers::append_raw(
            &generated, appended,
        ))
    }
}

struct Adapter(Arc<dyn GlobalLootModifier>);
impl ModifierApply for Adapter {
    fn apply(
        &self,
        roll: &mut dyn ModifierRoll,
        generated: &[loot::LootStack],
    ) -> Result<Vec<loot::LootStack>, String> {
        let snapshot = roll
            .facts()
            .try_into_api()
            .map_err(|error| format!("context: {error:?}"))?;
        let generated = generated
            .iter()
            .map(TryIntoApi::try_into_api)
            .collect::<Result<_, _>>()
            .map_err(|error| format!("generated: {error:?}"))?;
        let mut context = RollAdapter { roll, snapshot };
        self.0
            .apply(&mut context, generated)
            .map_err(|error| error.to_string())?
            .iter()
            .map(|stack| {
                stack
                    .try_into_engine(&())
                    .map_err(|error| format!("output: {error:?}"))
            })
            .collect()
    }
}
struct RollAdapter<'a> {
    roll: &'a mut dyn ModifierRoll,
    snapshot: mod_api::LootContext,
}
impl mod_api::LootModifierContext for RollAdapter<'_> {
    fn snapshot(&self) -> mod_api::LootContext {
        self.snapshot.clone()
    }
    fn random(&mut self) -> &mut dyn mod_api::RandomSource {
        self
    }
    fn roll_raw(
        &mut self,
        table: &Identifier,
    ) -> Result<Vec<mod_api::LootStack>, LootModifierError> {
        self.roll
            .roll_raw(&table.to_string())
            .map_err(|error| LootModifierError::new("roll_raw", error))?
            .iter()
            .map(|stack| {
                stack
                    .try_into_api()
                    .map_err(|error| LootModifierError::new("roll_raw", format!("{error:?}")))
            })
            .collect()
    }
}
impl mod_api::RandomSource for RollAdapter<'_> {
    fn next_int(&mut self) -> i32 {
        self.roll.next_int()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.roll.next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.roll.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.roll.next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.roll.next_double()
    }
    fn next_boolean(&mut self) -> bool {
        self.roll.next_boolean()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.roll.next_gaussian()
    }
}

/// Root resources are lowest, then passive content, native contributions and
/// the enabled packs. Readers run only after the loader selects each winning id.
pub(super) fn files(
    root: &Path,
    stack: &super::functions::PackStack,
    contributed: Vec<ModifierFile>,
) -> Vec<ModifierFile> {
    let mut files = Vec::new();
    for namespace in sunrise_world::json_folder::namespaces(&root.join("data")) {
        let directory = root.join("data").join(&namespace).join("loot_modifiers");
        let folder = match sunrise_world::json_folder::read(&directory) {
            Ok(folder) => folder,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                log::warn!("loot modifier folder {}: {error}", directory.display());
                continue;
            }
        };
        for (index, file) in folder.files.iter().enumerate() {
            let id = format!("{namespace}:{}", file.name);
            let path = file.path.display().to_string();
            let folder = folder.clone();
            files.push(ModifierFile {
                id,
                pack: "resources".into(),
                path,
                read: Box::new(move || {
                    folder.files[index]
                        .bytes()
                        .map(<[u8]>::to_vec)
                        .map_err(|error| error.to_string())
                }),
            });
        }
    }
    append_pack(&mut files, &stack.content.id, stack.content.source());
    files.extend(contributed);
    for pack in stack.enabled_packs() {
        append_pack(&mut files, &pack.id, pack.source());
    }
    files
}
fn append_pack(files: &mut Vec<ModifierFile>, pack: &str, source: &ResourceSource) {
    for path in source.list_under("data", &["loot_modifiers"]) {
        let Some((namespace, rest)) = path
            .strip_prefix("data/")
            .and_then(|path| path.split_once('/'))
        else {
            continue;
        };
        let Some(name) = rest
            .strip_prefix("loot_modifiers/")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let id = format!("{namespace}:{name}");
        let source = source.clone();
        let read_path = path.clone();
        files.push(ModifierFile {
            id,
            pack: pack.into(),
            path,
            read: Box::new(move || source.read(&read_path).map_err(|error| error.to_string())),
        });
    }
}
/// Preserve native contributions even though DataStore does not interpret this
/// independently discovered resource folder. Their existing order is priority.
pub(super) fn contributed(files: &[(String, Vec<u8>)]) -> Vec<ModifierFile> {
    files
        .iter()
        .filter_map(|(key, bytes)| {
            let id = key.strip_prefix("loot_modifiers/")?.to_owned();
            let Some((namespace, name)) = id.split_once(':') else {
                log::warn!(
                    "loot modifier {id} (native contributions): expected a namespace:path file id"
                );
                return None;
            };
            let path = format!("data/{namespace}/loot_modifiers/{name}.json");
            let bytes = bytes.clone();
            Some(ModifierFile {
                id,
                pack: "native contributions".into(),
                path,
                read: Box::new(move || Ok(bytes)),
            })
        })
        .collect()
}

/// The reload retains its own generation, never a global live manager. Initial
/// world opening binds the snapshot after its tables exist; reloads bind before
/// any listener apply. A failed preparation simply drops this unbound candidate.
pub(super) fn register_anchor(
    registry: &mut mod_api::ServerListenerRegistry,
) -> Result<Arc<mod_api::GlobalLootModifierManager>, mod_api::ReloadOrderError> {
    use mod_api::ServerReloadListeners;
    let generation = Arc::new(mod_api::GlobalLootModifierManager::default());
    let id = mod_api::GlobalLootModifierManager::key().id().clone();
    registry.add_retained(
        id,
        mod_api::FileToIdConverter::json("loot_modifiers").unwrap(),
        generation.clone(),
        generation.clone(),
    )?;
    Ok(generation)
}
pub(super) fn bind_retained(
    resources: &dyn mod_api::ServerReloadResources,
    generation: &mod_api::GlobalLootModifierManager,
) {
    use mod_api::__private::game::GlobalLootModifierManagerNew;
    if let Some(retained) = resources.listener(&mod_api::GlobalLootModifierManager::key()) {
        retained.bind_from(generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{
        __private::game::ServerReloadListenersOrder, ServerListenerRegistry, ServerReloadListeners,
    };

    #[test]
    fn reload_views_follow_the_registered_loot_anchor() {
        let mut listeners = ServerListenerRegistry::new();
        let functions = Identifier::parse("minecraft:function_library").unwrap();
        let loot = Identifier::parse("neoforge:loot_modifiers").unwrap();
        assert_eq!(listeners.get_last_vanilla_listener(), &functions);

        register_anchor(&mut listeners).unwrap();

        assert_eq!(listeners.get_last_vanilla_listener(), &functions);
        assert!(listeners.get_registry().iter().any(|id| id == &loot));
        assert!(listeners.get_graph().iter().next().is_none());

        let mod_listener = Identifier::parse("test_mod:after").unwrap();
        listeners
            .add_listener(
                mod_listener.clone(),
                mod_api::FileToIdConverter::json("machines").unwrap(),
                Arc::new(mod_api::GlobalLootModifierManager::default()),
            )
            .unwrap();
        let ordered = listeners.finish().unwrap().listeners;
        assert_eq!(
            ordered
                .into_iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            [functions, loot, mod_listener]
        );
    }
}
