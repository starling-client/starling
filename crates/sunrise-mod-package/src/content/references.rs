//! The references a content mod's files make to each other, which `Content::read` leaves for the
//! game to meet at load: a blockstate's models, a model's parent and textures, an item
//! definition's model, the ids a recipe, a tag or a loot table names in the mod's own namespaces,
//! and what a block needs to be playable (a blockstate, a loot table, a name).
//!
//! The tool checks only fields that the game's formats treat as references: models through the
//! game's `CuboidModel` decoder, blockstates and item definitions through `sunrise-assets`' decoders
//! (mod-defined types are left alone), and the other files through the shapes the recipe, tag and
//! loot loaders walk (a recipe's ingredients and result, a tag's `values`, a loot entry's `name`
//! and a state condition's `block`). Components, custom data and fields of unknown recipe types
//! are not scanned. The generic JSON reader accepts a byte-order mark and comments. A
//! file the load conditions (`neoforge:conditions`) may disable, a tag entry that is not
//! `required`, and a removal are not read: they cannot break a mod that does not have what they
//! name.
//!
//! Ids outside the mod's namespaces (vanilla's blocks, items, templates and loot functions) are
//! not checked: their tables live in the game's prepared resources, which no package tool reads.
//!
//! Severity is one rule. An **error** is a finding where the tool sees everything that decides
//! the answer and the game would certainly drop or draw the thing broken: an id of the mod's own
//! namespaces that nothing in the mod declares, in a mod with no code, no `[[jar]]` layer and no declared dependency (whose jar can supply files; the
//! namespace fence keeps every other mod out of them), a file the mod lacks when nothing else
//! could supply it, a file that is not JSON at all. A **warning** is a finding where another
//! source could supply the thing (a `[[jar]]` layer or a dependency's jar, an atlas source that renames a sprite or
//! walks a directory, a block whose template draws without a block model) or a legal mistake
//! (a block with no loot table or name); it says "could not find ... in this mod; if X supplies
//! it, ignore this". What the tool cannot judge at all is dropped, not warned: the ids a mod's
//! code declares, what the load conditions decide, a payload only a mod reads.
use super::{Content, FileReader, RenderShape};
use crate::manifest::{Manifest, Side};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use sunrise_assets::{
    blockstate::{BlockStateFile, VariantSet},
    item_model::ClientItem,
    model::{CuboidModel, Loader, TextureSlot},
};

/// What [`check_references`] found, each a sentence that names the file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Findings {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// The files supplied by a mod's listed `[[jar]]` archives, passed to [`check_references`].
///
/// Holds every normalized archive entry path and the raw contents of `assets/<ns>/lang/en_us.json`
/// files from the jars so references and translation keys resolve against the mod and its jars
/// together.
#[derive(Debug, Default, Clone)]
pub struct JarLayer {
    /// Entry paths (`/` separators, no leading `/`) inside the listed `[[jar]]` archives.
    pub files: BTreeSet<String>,
    /// Raw bytes of `assets/<ns>/lang/en_us.json` files from the jars, keyed by entry path.
    pub lang_files: BTreeMap<String, Vec<Vec<u8>>>,
}

impl JarLayer {
    /// Creates an empty jar layer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a normalized file path into the layer.
    pub fn insert_file(&mut self, path: impl Into<String>) {
        self.files.insert(path.into());
    }

    /// Inserts an `assets/<ns>/lang/en_us.json` file's contents (and records its path).
    pub fn insert_lang(&mut self, path: impl Into<String>, bytes: Vec<u8>) {
        let path = path.into();
        self.files.insert(path.clone());
        self.lang_files.entry(path).or_default().push(bytes);
    }

    /// Returns whether `path` is present in any listed jar.
    pub fn contains(&self, path: &str) -> bool {
        self.files.contains(path)
    }

    /// Collects the entry names (and `assets/<ns>/lang/en_us.json` contents) of one `.jar` zip
    /// archive into this layer, within the content limits a mod's own files have: at most
    /// [`MAX_FILES`](super::MAX_FILES) entries, [`MAX_FILE`](super::MAX_FILE) bytes per English
    /// names file and [`MAX_TOTAL`](super::MAX_TOTAL) bytes across them. A jar past a limit is an
    /// error, and the caller passes no layer.
    pub fn add_archive(&mut self, path: &std::path::Path) -> Result<(), String> {
        use super::{MAX_FILE, MAX_FILES, MAX_TOTAL};
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
        if zip.len() > MAX_FILES {
            return Err(format!("{}: more than {MAX_FILES} entries", path.display()));
        }
        let mut lang_indices = Vec::new();
        for i in 0..zip.len() {
            let entry = zip
                .by_index_raw(i)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if entry.is_dir() {
                continue;
            }
            let Ok(name) = entry.name() else {
                continue;
            };
            let normalized = name.replace('\\', "/");
            let normalized = normalized.trim_start_matches('/');
            if normalized.is_empty() {
                continue;
            }
            if is_en_us_lang(normalized) {
                lang_indices.push((i, normalized.to_owned()));
            }
            self.files.insert(normalized.to_owned());
        }
        let mut total = 0u64;
        for (i, normalized) in lang_indices {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            let mut bytes = Vec::new();
            entry
                .by_ref()
                .take(MAX_FILE.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|e| format!("{}: {normalized}: {e}", path.display()))?;
            if bytes.len() as u64 > MAX_FILE {
                return Err(format!(
                    "{}: {normalized} is over {} MiB",
                    path.display(),
                    MAX_FILE >> 20
                ));
            }
            total += bytes.len() as u64;
            if total > MAX_TOTAL {
                return Err(format!(
                    "{}: English names files are over {} MiB together",
                    path.display(),
                    MAX_TOTAL >> 20
                ));
            }
            self.lang_files.entry(normalized).or_default().push(bytes);
        }
        Ok(())
    }
}

fn is_en_us_lang(path: &str) -> bool {
    let mut parts = path.split('/');
    parts.next() == Some("assets")
        && parts.next().is_some_and(|ns| !ns.is_empty())
        && parts.next() == Some("lang")
        && parts.next() == Some("en_us.json")
        && parts.next().is_none()
}

/// The most declared ids a message lists.
const LISTED: usize = 8;

// Only check these built-in tag folders. A mod-declared registry's folder is
// `<namespace>/<path>`, and a tag's own id may contain slashes, so the file path alone
// does not say where the registry folder ends.
const CHECKED_TAG_FOLDERS: [&str; 6] = [
    "block",
    "item",
    "fluid",
    "entity_type",
    "worldgen/biome",
    "worldgen/structure",
];

struct Model {
    file: String,
    definition: CuboidModel,
}

#[derive(Clone, Copy)]
enum Asset {
    Model,
    Parent,
    Texture { inline_override: bool },
}

/// Checks the references among `content`'s files; `Err` only when a file cannot be read.
///
/// A native mod or `served_by` content package also has ids declared in code, so the ids it
/// declares in files are not taken for all of them. When `jars` is `Some`, every `[[jar]]` of the
/// manifest is present in that layer and references resolve against the mod's own files and `jars`
/// together; when `jars` is `None` and the manifest lists `[[jar]]` (or declared dependencies),
/// files those archives could supply are skipped or warned rather than failed.
pub fn check_references(
    content: &Content,
    manifest: &Manifest,
    jars: Option<&JarLayer>,
) -> Result<Findings, String> {
    let atlas = |name: &String| {
        let mut parts = name.split('/');
        parts.next() == Some("assets") && parts.nth(1) == Some("atlases") && name.ends_with(".json")
    };
    let mut check = References {
        content,
        manifest,
        reader: content.reader(),
        jars,
        code: manifest.native.is_some() || manifest.served_by.is_some(),
        unresolved_jar: !manifest.jar.is_empty() && jars.is_none(),
        has_deps: !manifest.dependencies.is_empty(),
        atlas: content.files.iter().any(atlas) || jars.is_some_and(|j| j.files.iter().any(atlas)),
        recipe_filters: content.files.iter().any(|name| {
            name.strip_prefix("data/")
                .and_then(|data| data.split_once('/'))
                .is_some_and(|(_, path)| {
                    path.starts_with("recipe_filter/") && path.ends_with(".json")
                })
        }),
        blocks: content.blocks.iter().map(|b| b.id.clone()).collect(),
        items: content
            .items
            .iter()
            .map(|i| i.id.clone())
            .chain(
                content
                    .blocks
                    .iter()
                    .filter(|b| b.item)
                    .map(|b| b.id.clone()),
            )
            .collect(),
        // A world fluid's flowing form is a fluid of its own (its block and bucket are in
        // `content.blocks` and `content.items`).
        fluids: content
            .fluids
            .iter()
            .flat_map(|f| [f.id.clone(), f.flowing_id()])
            .collect(),
        entity_types: content.entity_types.iter().map(|t| t.id.clone()).collect(),
        models: BTreeMap::new(),
        findings: Findings::default(),
    };
    for name in content.files.iter() {
        let mut parts = name.splitn(3, '/');
        let (top, namespace, rest) = (parts.next(), parts.next(), parts.next());
        let (Some(top), Some(namespace), Some(rest)) = (top, namespace, rest) else {
            continue;
        };
        let Some(path) = rest.strip_suffix(".json") else {
            continue;
        };
        let owned = content.namespaces.iter().any(|n| n == namespace);
        match (top, path.split_once('/')) {
            ("assets", Some(("blockstates", _))) if owned => check.blockstate(name)?,
            ("assets", Some(("models", model))) if owned => {
                check.read_model(name, namespace, model)?
            }
            ("assets", Some(("items", _))) if owned => check.item_definition(name)?,
            ("data", Some(("recipe", _))) if owned => check.recipe(name)?,
            ("data", Some(("loot_table", _))) if owned => check.loot_table(name)?,
            ("data", Some(("tags", tag))) => check.tag(name, tag)?,
            _ => {}
        }
    }
    check.check_models();
    check.declarations()?;
    check.side();
    // Four slots of one model naming one missing texture are one finding.
    for list in [&mut check.findings.errors, &mut check.findings.warnings] {
        let mut seen = BTreeSet::new();
        list.retain(|message| seen.insert(message.clone()));
    }
    Ok(check.findings)
}

struct References<'a> {
    content: &'a Content,
    manifest: &'a Manifest,
    reader: FileReader<'a>,
    jars: Option<&'a JarLayer>,
    code: bool,
    /// The manifest lists `[[jar]]`, and the caller did not supply their contents.
    unresolved_jar: bool,
    /// The manifest declares dependencies whose jars are not inspected here.
    has_deps: bool,
    /// The mod (or a supplied jar) has atlas sources, which can rename a sprite or take a whole
    /// directory.
    atlas: bool,
    /// A filter file may remove recipes or replace a referenced input or result before decoding.
    recipe_filters: bool,
    blocks: BTreeSet<String>,
    items: BTreeSet<String>,
    fluids: BTreeSet<String>,
    entity_types: BTreeSet<String>,
    /// Every model file of the mod's namespaces, by id.
    models: BTreeMap<String, Model>,
    findings: Findings,
}

/// A file's text and value as the game's readers take it: without a byte-order mark, and, when
/// that does not parse, without comments.
fn parse(bytes: &[u8]) -> Result<(String, Value), serde_json::Error> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match serde_json::from_str(text) {
        Ok(value) => Ok((text.to_owned(), value)),
        Err(error) => {
            let plain =
                String::from_utf8_lossy(&super::without_comments(text.as_bytes())).into_owned();
            serde_json::from_str(&plain)
                .map(|value| (plain, value))
                .map_err(|_| error)
        }
    }
}

/// Whether the file's own `neoforge:conditions` may disable it: the tool does not know which mods
/// and features the player has, so a file that carries them is not read.
fn conditional(value: &Value) -> bool {
    value.get("neoforge:conditions").is_some()
}

impl References<'_> {
    fn parsed(&mut self, name: &str) -> Result<Result<(String, Value), serde_json::Error>, String> {
        Ok(parse(&self.reader.read(name)?))
    }

    /// The text and value of a JSON file; an error finding when it is not JSON at all.
    fn json(&mut self, name: &str) -> Result<Option<(String, Value)>, String> {
        match self.parsed(name)? {
            Ok(json) => Ok(Some(json)),
            Err(e) => {
                self.findings.errors.push(format!(
                    "{name}: not valid JSON, so the game drops it: {e}."
                ));
                Ok(None)
            }
        }
    }

    /// A data file the load conditions do not disable (the tool cannot tell, so any file with
    /// `neoforge:conditions` is left alone).
    fn data(&mut self, name: &str) -> Result<Option<Value>, String> {
        Ok(self
            .json(name)?
            .map(|(_, value)| value)
            .filter(|value| !conditional(value)))
    }

    /// Whether an uninspected `[[jar]]` or a declared dependency's jar may supply missing files.
    fn unknown_jar(&self) -> bool {
        self.unresolved_jar || self.has_deps
    }

    fn has_own(&self, file: &str) -> bool {
        self.content
            .files
            .binary_search_by(|f| f.as_str().cmp(file))
            .is_ok()
    }

    fn has(&self, file: &str) -> bool {
        self.has_own(file) || self.jars.is_some_and(|jars| jars.contains(file))
    }

    /// The path of `id` when it is in one of the mod's namespaces.
    fn own<'i>(&self, id: &'i str) -> Option<(&'i str, &'i str)> {
        let (namespace, path) = id.split_once(':')?;
        self.content
            .namespaces
            .iter()
            .any(|n| n == namespace)
            .then_some((namespace, path))
    }

    /// What else could supply an asset the mod's files (and any supplied jars) lack.
    fn supplier(&self, texture: bool) -> Option<String> {
        let mut others = Vec::new();
        match (self.unresolved_jar, self.has_deps) {
            (true, true) => others.push("a [[jar]] layer or a dependency's jar"),
            (true, false) => others.push("a [[jar]] layer"),
            (false, true) => others.push("a dependency's jar"),
            (false, false) => {}
        }
        if texture && self.atlas {
            others.push("an atlas source (it renames a sprite or reads a directory)");
        }
        (!others.is_empty()).then(|| others.join(" or "))
    }

    /// `id` as a file of this mod, `assets/<ns>/<folder>/<path><extension>`: an error when the
    /// file is missing and nothing else could supply it, a warning when something could.
    fn missing(&mut self, name: &str, field: &str, id: &str, kind: Asset) {
        let Some((namespace, path)) = self.own(id) else {
            return;
        };
        let (folder, extension, what) = match kind {
            Asset::Model => ("models", ".json", "model"),
            Asset::Parent => ("models", ".json", "parent model"),
            Asset::Texture { .. } => ("textures", ".png", "texture"),
        };
        let file = format!("assets/{namespace}/{folder}/{path}{extension}");
        if self.has(&file) {
            return;
        }
        let near = near_file(self, folder, extension, id);
        let supplier = self.supplier(folder == "textures");
        let reason = match (kind, supplier) {
            (
                Asset::Texture {
                    inline_override: true,
                },
                _,
            ) => Some("an inline composite child may override the slot".to_owned()),
            (_, Some(other)) => Some(format!("{other} may supply it")),
            (_, None) => None,
        };
        match reason {
            None => self.findings.errors.push(format!(
                "{name}: the {what} {id} at {field} is not a file of this mod (expected {file}). Add the file or fix the id{near}."
            )),
            Some(reason) => self.findings.warnings.push(format!(
                "{name}: could not find the {what} {id} at {field} in this mod (expected {file}); {reason}. If not, add the file or fix the id{near}."
            )),
        }
    }

    fn blockstate(&mut self, name: &str) -> Result<(), String> {
        let Some((text, _)) = self.json(name)? else {
            return Ok(());
        };
        // A file the decoder refuses, and a definition of a mod's own type, name no model here.
        let Ok(BlockStateFile::Vanilla(definition)) = BlockStateFile::from_json(&text) else {
            return Ok(());
        };
        let variants = definition
            .variants
            .iter()
            .flatten()
            .map(|(key, set)| (format!("variants[{key:?}]"), set));
        let multipart = definition
            .multipart
            .iter()
            .flatten()
            .enumerate()
            .map(|(index, part)| (format!("multipart[{index}].apply"), &part.variant));
        for (field, set) in variants.chain(multipart) {
            match set {
                VariantSet::Single(variant) => {
                    self.missing(
                        name,
                        &format!("{field}.model"),
                        &variant.model,
                        Asset::Model,
                    );
                }
                VariantSet::Weighted(list) => {
                    for (index, (variant, _)) in list.iter().enumerate() {
                        self.missing(
                            name,
                            &format!("{field}[{index}].model"),
                            &variant.model,
                            Asset::Model,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Match the model library: read UTF-8 and let its decoder consume one model, not the whole file.
    fn read_model(&mut self, name: &str, namespace: &str, path: &str) -> Result<(), String> {
        let bytes = self.reader.read(name)?;
        let definition = match std::str::from_utf8(&bytes)
            .map_err(|error| error.to_string())
            .and_then(|text| CuboidModel::from_json(text).map_err(|error| error.to_string()))
        {
            Ok(model) => model,
            Err(error) => {
                self.findings.errors.push(format!(
                    "{name}: the game could not read this model: {error}. Fix the model JSON."
                ));
                return Ok(());
            }
        };
        self.models.insert(
            format!("{namespace}:{path}"),
            Model {
                file: name.to_owned(),
                definition,
            },
        );
        Ok(())
    }

    /// Follow parent chains from both model files and inline children. An inline override only
    /// affects that child, which may itself be hidden; it cannot prove the parent sprite is unused.
    fn check_models(&mut self) {
        let mut overridden = BTreeSet::new();
        let mut inline_overridden = BTreeSet::new();
        let mut references = Vec::new();
        for (id, model) in &self.models {
            let mut pending = vec![(&model.definition, String::new())];
            while let Some((child, field)) = pending.pop() {
                if let Some(Loader::Composite { children, .. }) = &child.loader {
                    pending.extend(children.iter().map(|(key, child)| {
                        (child.as_ref(), format!("{field}children[{key:?}]."))
                    }));
                }
                let Some(parent) = child.parent.as_deref() else {
                    continue;
                };
                references.push((
                    model.file.clone(),
                    format!("{field}parent"),
                    parent.to_owned(),
                    Asset::Parent,
                ));
                let (inline, overrides) = if field.is_empty() {
                    (false, &mut overridden)
                } else {
                    (true, &mut inline_overridden)
                };
                let mut at = Some(parent);
                for _ in 0..64 {
                    let Some(ancestor) = at else { break };
                    if !inline && ancestor == id {
                        break;
                    }
                    let Some(parent) = self.models.get(ancestor) else {
                        break;
                    };
                    for slot in child.textures.keys() {
                        if parent.definition.textures.contains_key(slot) {
                            overrides.insert((ancestor, slot.as_str()));
                        }
                    }
                    at = parent.definition.parent.as_deref();
                }
            }
        }
        for (id, model) in &self.models {
            // One diagnostic per missing sprite, preferring a slot with no override.
            let mut by_sprite = BTreeMap::new();
            for (slot, texture) in &model.definition.textures {
                let key = (id.as_str(), slot.as_str());
                if let TextureSlot::Material(material) = texture
                    && !overridden.contains(&key)
                {
                    let inline_override = inline_overridden.contains(&key);
                    let entry = by_sprite
                        .entry(&material.sprite)
                        .or_insert((slot, inline_override));
                    if !inline_override {
                        *entry = (slot, false);
                    }
                }
            }
            references.extend(
                by_sprite
                    .into_iter()
                    .map(|(sprite, (slot, inline_override))| {
                        (
                            model.file.clone(),
                            format!("textures[{slot:?}]"),
                            sprite.clone(),
                            Asset::Texture { inline_override },
                        )
                    }),
            );
        }
        for (file, field, id, kind) in references {
            self.missing(&file, &field, &id, kind);
        }
    }

    fn item_definition(&mut self, name: &str) -> Result<(), String> {
        let Some((text, _)) = self.json(name)? else {
            return Ok(());
        };
        let Ok(item) = ClientItem::from_json(&text) else {
            return Ok(());
        };
        let mut models = Vec::new();
        item.model.dependencies(&mut models);
        for model in models {
            self.missing(name, "model", &model, Asset::Model);
        }
        Ok(())
    }

    fn recipe(&mut self, name: &str) -> Result<(), String> {
        let Some(recipe) = self.data(name)? else {
            return Ok(());
        };
        let mut ids = Vec::new();
        recipe_ids(&recipe, &mut ids);
        // Do not try to reproduce the runtime's ordered filters over the merged recipe set.
        let uncertainty = self
            .recipe_filters
            .then_some("a local recipe filter may remove the recipe or replace this reference");
        for (field, id) in ids {
            self.id_in_registry(
                name,
                id,
                "item",
                &format!("the recipe item at {field}"),
                uncertainty,
            );
        }
        Ok(())
    }

    fn loot_table(&mut self, name: &str) -> Result<(), String> {
        let Some(table) = self.data(name)? else {
            return Ok(());
        };
        let mut found = Vec::new();
        loot_table_ids(&table, &mut found);
        for (registry, field, id) in found {
            self.id_in_registry(
                name,
                id,
                registry,
                &format!("the loot reference at {field}"),
                None,
            );
        }
        Ok(())
    }

    fn tag(&mut self, name: &str, tag: &str) -> Result<(), String> {
        let Some(registry) = CHECKED_TAG_FOLDERS.into_iter().find(|&folder| {
            tag.strip_prefix(folder)
                .is_some_and(|rest| rest.starts_with('/'))
        }) else {
            return Ok(());
        };
        let Some(value) = self.data(name)? else {
            return Ok(());
        };
        let Some(Value::Array(values)) = value.get("values") else {
            return Ok(());
        };
        for (index, entry) in values.iter().enumerate() {
            let (id, field) = match entry {
                Value::String(id) => (id, format!("values[{index}]")),
                // `{"id": ..., "required": false}` is allowed to name nothing, and
                // `{"id": ..., "remove": true}` takes an entry out.
                Value::Object(object)
                    if object.get("required").and_then(Value::as_bool) != Some(false)
                        && object.get("remove").and_then(Value::as_bool) != Some(true) =>
                {
                    match object.get("id") {
                        Some(Value::String(id)) => (id, format!("values[{index}].id")),
                        _ => continue,
                    }
                }
                _ => continue,
            };
            if let Some(tag) = id.strip_prefix('#') {
                self.tag_reference(name, &field, id, tag, registry);
            } else {
                self.id_in_registry(
                    name,
                    id,
                    registry,
                    &format!("the tag value at {field}"),
                    None,
                );
            }
        }
        Ok(())
    }

    fn tag_reference(&mut self, name: &str, field: &str, id: &str, tag: &str, registry: &str) {
        let Some((namespace, path)) = self.own(tag) else {
            return;
        };
        let file = format!("data/{namespace}/tags/{registry}/{path}.json");
        if self.has(&file) {
            return;
        }
        let other = match (self.unresolved_jar, self.has_deps, self.code) {
            (true, true, true) => Some("a [[jar]] layer, a dependency's jar or the mod's code"),
            (true, true, false) => Some("a [[jar]] layer or a dependency's jar"),
            (true, false, true) => Some("a [[jar]] layer or the mod's code"),
            (true, false, false) => Some("a [[jar]] layer"),
            (false, true, true) => Some("a dependency's jar or the mod's code"),
            (false, true, false) => Some("a dependency's jar"),
            (false, false, true) => Some("the mod's code"),
            (false, false, false) => None,
        };
        match other {
            None => self.findings.errors.push(format!(
                "{name}: the tag {id} at {field} is not a tag file of this mod (expected {file}), so the tag does not load. Add the file or fix the id."
            )),
            Some(other) => self.findings.warnings.push(format!(
                "{name}: could not find the tag {id} at {field} in this mod (expected {file}); if {other} supplies it, ignore this. Otherwise add the file or fix the id."
            )),
        }
    }

    /// `id` of a vanilla-shaped registry the mod declares into, when it is in the mod's namespace
    /// and the mod does not declare it: an error unless a jar may declare it or the reference
    /// may not be used. A mod with code is not asked: its code declares ids no file shows.
    fn id_in_registry(
        &mut self,
        name: &str,
        id: &str,
        registry: &str,
        what: &str,
        uncertainty: Option<&str>,
    ) {
        if self.code || self.own(id).is_none() {
            return;
        }
        let known: Vec<&str> = match registry {
            "block" => &self.blocks,
            "item" => &self.items,
            "fluid" => &self.fluids,
            "entity_type" => &self.entity_types,
            _ => return,
        }
        .iter()
        .map(String::as_str)
        .collect();
        if known.contains(&id) {
            return;
        }
        let article = if registry == "item" || registry == "entity_type" {
            "an"
        } else {
            "a"
        };
        let kind = registry.replace('_', " ");
        let near = nearest(id, &known)
            .map(|n| format!(" Did you mean {n}?"))
            .unwrap_or_default();
        let listed = self.listed(&known);
        let jar_reason = match (self.unresolved_jar, self.has_deps) {
            (true, true) => Some("a [[jar]] layer or a dependency's jar may supply it"),
            (true, false) => Some("a [[jar]] layer may supply it"),
            (false, true) => Some("a dependency's jar may supply it"),
            (false, false) => None,
        };
        let reason = uncertainty.or(jar_reason);
        if let Some(reason) = reason {
            self.findings.warnings.push(format!(
                "{name}: could not find {what} {id} among the {kind}s this mod declares ({listed}); {reason}.{near} Otherwise fix the id."
            ));
        } else {
            self.findings.errors.push(format!(
                "{name}: {what} {id} is not {article} {kind} this mod declares ({listed}).{near} Fix the id."
            ));
        }
    }

    fn listed(&self, known: &[&str]) -> String {
        if known.is_empty() {
            return "none declared".into();
        }
        let mut names: Vec<&str> = known
            .iter()
            .map(|id| {
                id.strip_prefix(self.manifest.id.as_str())
                    .and_then(|rest| rest.strip_prefix(':'))
                    .unwrap_or(id)
            })
            .collect();
        names.sort_unstable();
        let more = names.len().saturating_sub(LISTED);
        names.truncate(LISTED);
        let mut text = format!("declared: {}", names.join(", "));
        if more > 0 {
            text.push_str(&format!(", and {more} more"));
        }
        text
    }

    /// What each block and item declared in a file needs: a blockstate, an item definition, a
    /// loot table, a name.
    fn declarations(&mut self) -> Result<(), String> {
        let mut names = Vec::new();
        let content = self.content;
        for block in &content.blocks {
            let Some((namespace, path)) = self.own(&block.id) else {
                continue;
            };
            if !self.has_own(&format!("data/{namespace}/sunrise/block/{path}.json")) {
                continue;
            }
            let file = format!("data/{namespace}/sunrise/block/{path}.json");
            let state = format!("assets/{namespace}/blockstates/{path}.json");
            // Only a block drawn from a model needs a blockstate. One that declares no render
            // shape takes its template's, which the tool cannot see: a chest or a sign is drawn
            // by its block entity.
            let shape = block.properties.render_shape;
            if !self.unknown_jar()
                && shape != Some(RenderShape::Invisible)
                && shape != Some(RenderShape::EntityblockAnimated)
                && !self.has(&state)
            {
                match (shape, self.supplier(false)) {
                    (Some(_), None) => self.findings.errors.push(format!(
                        "{file}: the block {} has no model: expected {state}, which names its models. Add it, or declare render_shape INVISIBLE.",
                        block.id
                    )),
                    (shape, other) => {
                        let mut maybe = Vec::new();
                        if shape.is_none() {
                            maybe.push("its template draws it without a block model (a chest, a sign)".to_owned());
                        }
                        maybe.extend(other.map(|o| format!("{o} supplies it")));
                        self.findings.warnings.push(format!(
                            "{file}: could not find a blockstate for the block {} in this mod (expected {state}); if {}, ignore this. Otherwise add it, or declare render_shape INVISIBLE.",
                            block.id,
                            maybe.join(" or ")
                        ));
                    }
                }
            }
            if block.item {
                self.item_definition_exists(&file, &block.id, namespace, path);
            }
            let loot = format!("data/{namespace}/loot_table/blocks/{path}.json");
            if !self.unknown_jar() && !self.has(&loot) {
                self.findings.warnings.push(format!(
                    "{file}: no local loot table found for the block {} (expected {loot}). Add one if this block needs its own table.",
                    block.id
                ));
            }
            names.push((
                format!("block.{namespace}.{}", path.replace('/', ".")),
                file,
            ));
        }
        for item in &content.items {
            let Some((namespace, path)) = self.own(&item.id) else {
                continue;
            };
            let file = format!("data/{namespace}/sunrise/item/{path}.json");
            if !self.has_own(&file) {
                continue;
            }
            self.item_definition_exists(&file, &item.id, namespace, path);
            names.push((format!("item.{namespace}.{}", path.replace('/', ".")), file));
        }
        self.names(names)
    }

    fn item_definition_exists(&mut self, file: &str, id: &str, namespace: &str, path: &str) {
        let definition = format!("assets/{namespace}/items/{path}.json");
        if !self.unknown_jar() && !self.has(&definition) {
            self.findings.warnings.push(format!(
                "{file}: the item {id} has no item definition, so it is drawn as the missing model. Add {definition}."
            ));
        }
    }

    /// Each `(language key, declaring file)` has a name in its namespace's `en_us.json`.
    fn names(&mut self, wanted: Vec<(String, String)>) -> Result<(), String> {
        if self.unknown_jar() {
            return Ok(());
        }
        // `None`: no file. `Some(None)`: a file the game's lenient reader may take and this
        // one does not; no key is asked of it.
        let mut by_namespace: BTreeMap<String, Option<Option<Vec<Value>>>> = Default::default();
        for (key, file) in wanted {
            let namespace = file.split('/').nth(1).unwrap_or_default().to_owned();
            let lang = format!("assets/{namespace}/lang/en_us.json");
            if !by_namespace.contains_key(&namespace) {
                let mut found_any = false;
                let mut lenient = false;
                let mut parsed = Vec::new();
                if self.has_own(&lang) {
                    found_any = true;
                    match self.parsed(&lang)? {
                        Ok((_, value)) => parsed.push(value),
                        Err(_) => lenient = true,
                    }
                }
                if let Some(jar_entries) = self.jars.and_then(|jars| jars.lang_files.get(&lang)) {
                    for bytes in jar_entries {
                        found_any = true;
                        match parse(bytes) {
                            Ok((_, value)) => parsed.push(value),
                            Err(_) => lenient = true,
                        }
                    }
                }
                let entry = if !found_any {
                    None
                } else if lenient {
                    Some(None)
                } else {
                    Some(Some(parsed))
                };
                by_namespace.insert(namespace.clone(), entry);
            }
            let why = match &by_namespace[&namespace] {
                Some(Some(langs)) if langs.iter().any(|lang| lang.get(&key).is_some()) => continue,
                Some(None) => continue,
                Some(Some(_)) => format!("{lang} has no {key:?}"),
                None => format!("there is no {lang}"),
            };
            self.findings.warnings.push(format!(
                "{file}: no local name found for {key} ({why}). Add the key to {lang} if another language source does not supply it."
            ));
        }
        Ok(())
    }

    /// Registry content on a client-only package: joining a server compares the mods, and the
    /// package's side defaults to `client` when the manifest has no `side` line.
    fn side(&mut self) {
        let content = &self.content;
        let registry = !(content.blocks.is_empty()
            && content.items.is_empty()
            && content.effects.is_empty()
            && content.fluids.is_empty()
            && content.entity_types.is_empty()
            && content.creative_tabs.is_empty());
        if registry && self.manifest.side == Side::Client {
            self.findings.warnings.push(
                "sunrise-mod.toml: side is client (the default when the line is left out), but this mod adds blocks, items or other registry content that a server and its players must have too; set side = \"both\" or joining a server fails."
                    .into(),
            );
        }
    }
}

/// `", did you mean <id>"` for a missing file, from the files of the same folder.
fn near_file(check: &References<'_>, folder: &str, extension: &str, id: &str) -> String {
    let Some((namespace, _)) = check.own(id) else {
        return String::new();
    };
    let prefix = format!("assets/{namespace}/{folder}/");
    let known: Vec<String> = check
        .content
        .files
        .iter()
        .chain(check.jars.iter().flat_map(|jars| jars.files.iter()))
        .filter_map(|f| {
            let path = f.strip_prefix(&prefix)?.strip_suffix(extension)?;
            Some(format!("{namespace}:{path}"))
        })
        .collect();
    let known: Vec<&str> = known.iter().map(String::as_str).collect();
    nearest(id, &known)
        .map(|n| format!(" (did you mean {n}?)"))
        .unwrap_or_default()
}

/// The id among `known` within two edits of `id`, the closest first.
fn nearest<'k>(id: &str, known: &[&'k str]) -> Option<&'k str> {
    known
        .iter()
        .map(|k| (edits(id, k), *k))
        .filter(|(d, _)| *d <= 2)
        .min()
        .map(|(_, k)| k)
}

/// The number of single-character changes from `a` to `b` (Levenshtein).
fn edits(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != *cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

fn type_of(object: &Value) -> Option<&str> {
    object.get("type").and_then(Value::as_str)
}

/// The item ids of a recipe, read as the recipe kinds the game knows read them: each kind's
/// ingredient fields and its `result`. A recipe of another kind (a mod's own, with fields only it
/// reads) and the data beside the ids (`components`, `group`, `category`) name nothing.
fn recipe_ids<'v>(recipe: &'v Value, out: &mut Vec<(String, &'v str)>) {
    let Some(kind) = type_of(recipe) else {
        return;
    };
    let (ingredients, keyed, result): (&[&str], bool, bool) =
        match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "crafting_shaped" => (&[], true, true),
            "crafting_shapeless" => (&["ingredients"], false, true),
            "smelting" | "blasting" | "smoking" | "campfire_cooking" | "stonecutting" => {
                (&["ingredient"], false, true)
            }
            "smithing_transform" => (&["template", "base", "addition"], false, true),
            "smithing_trim" => (&["template", "base", "addition"], false, false),
            "crafting_transmute" => (&["input", "material"], false, true),
            _ => return,
        };
    for field in ingredients {
        if let Some(ingredient) = recipe.get(field) {
            ingredient_ids(ingredient, field, out);
        }
    }
    if keyed && let Some(Value::Object(key)) = recipe.get("key") {
        for (symbol, ingredient) in key {
            ingredient_ids(ingredient, &format!("key[{symbol:?}]"), out);
        }
    }
    if result {
        match recipe.get("result") {
            Some(Value::String(id)) => out.push(("result".to_owned(), id)),
            Some(stack) => {
                if let Some(id) = stack.get("id").and_then(Value::as_str) {
                    out.push(("result.id".to_owned(), id));
                }
            }
            None => {}
        }
    }
}

/// The item ids an ingredient names: an id, a list of ingredients, or an ingredient type's own
/// fields (`neoforge:compound`, `difference`, `components`...). A `#tag` and a type only a mod
/// reads name no id.
fn ingredient_ids<'v>(ingredient: &'v Value, path: &str, out: &mut Vec<(String, &'v str)>) {
    match ingredient {
        Value::String(id) if !id.starts_with('#') => out.push((path.to_owned(), id)),
        Value::Array(list) => {
            for (index, entry) in list.iter().enumerate() {
                ingredient_ids(entry, &format!("{path}[{index}]"), out);
            }
        }
        Value::Object(object) => {
            let kind = object
                .get("neoforge:ingredient_type")
                .or_else(|| object.get("type"))
                .and_then(Value::as_str);
            let fields: &[&str] = match kind {
                // WalkedIngredient uses `ingredients` only if `children` is absent.
                Some("neoforge:compound" | "neoforge:intersection")
                    if object.contains_key("children") =>
                {
                    &["children"]
                }
                Some("neoforge:compound" | "neoforge:intersection") => &["ingredients"],
                Some("neoforge:difference") => &["base", "subtracted"],
                Some("neoforge:custom_display") => &["base"],
                Some("neoforge:components") => &["items"],
                // A brewing slot's item predicate wraps its ingredient.
                None => &["item"],
                Some(_) => &[],
            };
            for field in fields {
                if let Some(part) = object.get(*field) {
                    ingredient_ids(part, &format!("{path}.{field}"), out);
                }
            }
        }
        _ => {}
    }
}

type LootReference<'v> = (&'static str, &'static str, &'v str);

/// The `(registry, field, id)` a loot table names: an item entry's `name`, a state condition's
/// `block`, through the table's pools, entries and conditions. Conditional pools and a loot
/// function's payload (`set_components`, `set_custom_data`) are not checked.
fn loot_table_ids<'v>(table: &'v Value, out: &mut Vec<LootReference<'v>>) {
    list(table.get("functions"), |function| {
        conditions(function.get("conditions"), out);
    });
    list(table.get("pools"), |pool| {
        if conditional(pool) {
            return;
        }
        conditions(pool.get("conditions"), out);
        list(pool.get("functions"), |f| {
            conditions(f.get("conditions"), out)
        });
        entries(pool.get("entries"), out);
    });
}

/// `each` of a list, or of the one value a field holds in its place.
fn list<'v>(value: Option<&'v Value>, mut each: impl FnMut(&'v Value)) {
    match value {
        Some(Value::Array(values)) => values.iter().for_each(each),
        Some(value @ Value::Object(_)) => each(value),
        _ => {}
    }
}

fn entries<'v>(value: Option<&'v Value>, out: &mut Vec<LootReference<'v>>) {
    list(value, |entry| {
        conditions(entry.get("conditions"), out);
        list(entry.get("functions"), |f| {
            conditions(f.get("conditions"), out)
        });
        let kind = type_of(entry).map(|t| t.strip_prefix("minecraft:").unwrap_or(t));
        match (kind, entry.get("name").and_then(Value::as_str)) {
            (Some("item"), Some(name)) => out.push(("item", "entry.name", name)),
            (Some("alternatives" | "group" | "sequence"), _) => {
                entries(entry.get("children"), out);
            }
            _ => {}
        }
    });
}

fn conditions<'v>(value: Option<&'v Value>, out: &mut Vec<LootReference<'v>>) {
    list(value, |condition| {
        let kind = type_of(condition).map(|t| t.strip_prefix("minecraft:").unwrap_or(t));
        match kind {
            Some("block_state_property") => {
                if let Some(block) = condition.get("block").and_then(Value::as_str) {
                    out.push(("block", "condition.block", block));
                }
            }
            Some("inverted") => conditions(condition.get("term"), out),
            Some("all_of" | "any_of") => conditions(condition.get("terms"), out),
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Package;

    const MANIFEST: &str = "id = \"gb\"\nname = \"gb\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    fn jar() -> String {
        format!(
            "[[jar]]\nid = \"original\"\nversion = \"1\"\nsha512 = \"{}\"\n",
            "1".repeat(128)
        )
    }
    const GOOD: &[(&str, &str)] = &[
        (
            "data/gb/sunrise/block/slab.json",
            r#"{"template":"minecraft:smooth_stone_slab"}"#,
        ),
        (
            "data/gb/sunrise/item/shard.json",
            r#"{"template":"minecraft:diamond"}"#,
        ),
        (
            "assets/gb/blockstates/slab.json",
            r#"{"variants":{"":{"model":"gb:block/slab"}}}"#,
        ),
        (
            "assets/gb/models/block/slab.json",
            r##"{"parent":"minecraft:block/slab","textures":{"side":"gb:block/stone","top":"#side"}}"##,
        ),
        (
            "assets/gb/models/item/shard.json",
            r#"{"parent":"minecraft:item/generated","textures":{"layer0":"gb:item/shard"}}"#,
        ),
        (
            "assets/gb/items/slab.json",
            r#"{"model":{"type":"minecraft:model","model":"gb:block/slab"}}"#,
        ),
        (
            "assets/gb/items/shard.json",
            r#"{"model":{"type":"minecraft:model","model":"gb:item/shard"}}"#,
        ),
        ("assets/gb/textures/block/stone.png", "png"),
        ("assets/gb/textures/item/shard.png", "png"),
        (
            "assets/gb/lang/en_us.json",
            r#"{"block.gb.slab":"Slab","item.gb.shard":"Shard"}"#,
        ),
        (
            "data/gb/loot_table/blocks/slab.json",
            r#"{"type":"minecraft:block","pools":[{"entries":[{"type":"minecraft:item","name":"gb:slab"}]}]}"#,
        ),
        (
            "data/gb/recipe/slab.json",
            r#"{"type":"minecraft:crafting_shaped","pattern":["SS"],"key":{"S":"gb:shard"},"result":{"id":"gb:slab","count":6}}"#,
        ),
        (
            "data/gb/tags/item/things.json",
            r#"{"values":["gb:shard","gb:slab","minecraft:stick"]}"#,
        ),
    ];

    /// The good mod with `changes` applied: a file replaced, or removed when its text is empty.
    fn findings_with_jars(
        manifest: &str,
        changes: &[(&str, &str)],
        jars: Option<&JarLayer>,
    ) -> Findings {
        let dir = tempfile::tempdir().unwrap();
        let mut files: Vec<(&str, &str)> = GOOD.to_vec();
        for (name, text) in changes {
            files.retain(|(n, _)| n != name);
            if !text.is_empty() {
                files.push((name, text));
            }
        }
        std::fs::write(dir.path().join(crate::manifest::MANIFEST), manifest).unwrap();
        for (name, text) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let package = Package::read(dir.path()).unwrap();
        let content = Content::read(dir.path(), &package.manifest).unwrap();
        check_references(&content, &package.manifest, jars).unwrap()
    }

    fn findings(manifest: &str, changes: &[(&str, &str)]) -> Findings {
        findings_with_jars(manifest, changes, None)
    }

    fn errors(changes: &[(&str, &str)]) -> Vec<String> {
        findings(MANIFEST, changes).errors
    }

    /// No error and no warning.
    fn clean(manifest: &str, changes: &[(&str, &str)]) {
        assert_eq!(findings(manifest, changes), Findings::default());
    }

    #[test]
    fn a_consistent_mod_has_no_findings() {
        assert_eq!(findings(MANIFEST, &[]), Findings::default());
    }

    #[test]
    fn a_missing_texture_names_the_model_and_the_file() {
        let e = errors(&[("assets/gb/textures/block/stone.png", "")]);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with("assets/gb/models/block/slab.json: the texture gb:block/stone"));
        assert!(e[0].contains("expected assets/gb/textures/block/stone.png"));
        // Two slots naming it are one finding.
        let two = r#"{"textures":{"a":"gb:block/gone","b":"gb:block/gone"}}"#;
        assert_eq!(
            errors(&[("assets/gb/models/block/slab.json", two)]).len(),
            1
        );
    }

    #[test]
    fn a_missing_model_is_named_with_a_near_miss() {
        let e = errors(&[("assets/gb/models/block/slab.json", "")]);
        // The blockstate and the item definition both name it.
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e[0].contains("gb:block/slab"));
        assert!(e[0].contains("variants[\"\"].model"));
        let e = errors(&[(
            "assets/gb/blockstates/slab.json",
            r#"{"variants":{"":{"model":"gb:block/slabb"}}}"#,
        )]);
        assert!(e[0].contains("did you mean gb:block/slab?"), "{e:?}");
    }

    #[test]
    fn a_block_without_a_blockstate_has_no_model_unless_it_draws_none() {
        // Declaring MODEL says the block is drawn from one: nothing else could supply it.
        let model = r#"{"template":"minecraft:stone","properties":{"render_shape":"MODEL"}}"#;
        let e = errors(&[
            ("assets/gb/blockstates/slab.json", ""),
            ("data/gb/sunrise/block/slab.json", model),
        ]);
        assert!(e[0].contains("the block gb:slab has no model"), "{e:?}");
        // The template's own shape is not known: a warning.
        let f = findings(MANIFEST, &[("assets/gb/blockstates/slab.json", "")]);
        assert!(f.errors.is_empty(), "{f:?}");
        assert_eq!(f.warnings.len(), 1, "{f:?}");
        assert!(f.warnings[0].contains("could not find a blockstate for the block gb:slab"));
    }

    #[test]
    fn a_block_drawn_by_nothing_or_by_its_block_entity_needs_no_blockstate() {
        for shape in ["INVISIBLE", "ENTITYBLOCK_ANIMATED"] {
            let block = format!(
                r#"{{"template":"minecraft:stone","properties":{{"render_shape":"{shape}"}}}}"#
            );
            clean(
                MANIFEST,
                &[
                    ("assets/gb/blockstates/slab.json", ""),
                    ("data/gb/sunrise/block/slab.json", &block),
                ],
            );
        }
    }

    #[test]
    fn a_recipe_item_of_this_mod_must_be_declared() {
        let recipe = r#"{"type":"minecraft:crafting_shaped","pattern":["S"],"key":{"S":"gb:shrad"},"result":{"id":"minecraft:stick","count":1}}"#;
        let e = errors(&[("data/gb/recipe/slab.json", recipe)]);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with("data/gb/recipe/slab.json:"));
        assert!(e[0].contains("key[\"S\"]"));
        assert!(e[0].contains("gb:shrad is not an item"));
        assert!(e[0].contains("Did you mean gb:shard?"));
    }

    #[test]
    fn an_ingredient_checks_only_the_selected_children_field() {
        let file = "data/gb/recipe/slab.json";
        for kind in ["compound", "intersection"] {
            let recipe = |fields: &str| {
                format!(
                    r#"{{"type":"minecraft:stonecutting","ingredient":{{"neoforge:ingredient_type":"neoforge:{kind}",{fields}}},"result":{{"id":"gb:slab"}}}}"#
                )
            };
            clean(
                MANIFEST,
                &[(
                    file,
                    &recipe(r#""children":["gb:shard"],"ingredients":["gb:missing"]"#),
                )],
            );
            for field in ["children", "ingredients"] {
                let e = errors(&[(file, &recipe(&format!(r#""{field}":["gb:missing"]"#)))]);
                assert_eq!(e.len(), 1, "{e:?}");
                assert!(e[0].starts_with(file));
                assert!(e[0].contains(&format!("ingredient.{field}[0]")));
                assert!(e[0].contains("gb:missing is not an item"));
            }
        }
    }

    #[test]
    fn local_recipe_filters_make_unused_or_replaced_references_nonfatal() {
        let file = "data/gb/recipe/slab.json";
        let recipe = r#"{"type":"minecraft:stonecutting","ingredient":"gb:missing","result":{"id":"gb:slab"}}"#;
        let filter = "data/gb/recipe_filter/nested/slab.json";
        for action in [
            r#""remove":true"#,
            r#""replace_input":{"from":"gb:missing","to":"gb:shard"}"#,
        ] {
            let text = format!(r#"{{"filters":[{{"id":"gb:slab",{action}}}]}}"#);
            let f = findings(MANIFEST, &[(file, recipe), (filter, &text)]);
            assert!(f.errors.is_empty(), "{f:?}");
            assert_eq!(f.warnings.len(), 1, "{f:?}");
            assert!(f.warnings[0].starts_with(file));
            assert!(f.warnings[0].contains("recipe filter"));
        }
        let e = errors(&[(file, recipe)]);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with(file));
        assert!(e[0].contains("gb:missing is not an item"));
    }

    #[test]
    fn a_tag_value_of_this_mod_must_be_declared() {
        let e = errors(&[(
            "data/gb/tags/item/things.json",
            r##"{"values":["gb:nope","#gb:other"]}"##,
        )]);
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e[0].contains("values[0]"));
        assert!(e[0].contains("gb:nope is not an item"));
        assert!(e[1].contains("values[1]"));
        assert!(e[1].contains("#gb:other"));
        // A vanilla tag file lists this mod's blocks; a block tag checks blocks.
        let e = errors(&[(
            "data/minecraft/tags/block/slabs.json",
            r#"{"values":["gb:slab","gb:shard"]}"#,
        )]);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("gb:shard is not a block"));
    }

    #[test]
    fn a_loot_entry_of_this_mod_must_be_declared() {
        let table = r#"{"pools":[{"entries":[{"type":"minecraft:item","name":"gb:slabs"}]}]}"#;
        let e = errors(&[("data/gb/loot_table/blocks/slab.json", table)]);
        assert!(
            e[0].contains("entry.name") && e[0].contains("gb:slabs is not an item"),
            "{e:?}"
        );
        // Through a group's children, and a state condition's block.
        let nested = r#"{"pools":[{"entries":[{"type":"minecraft:alternatives","children":[{"type":"minecraft:item","name":"gb:slab","conditions":[{"type":"minecraft:block_state_property","block":"gb:slabs"}]}]}]}]}"#;
        let e = errors(&[("data/gb/loot_table/blocks/slab.json", nested)]);
        assert!(e[0].contains("gb:slabs is not a block"), "{e:?}");
    }

    #[test]
    fn a_broken_json_file_is_an_error() {
        let e = errors(&[("data/gb/recipe/slab.json", "{")]);
        assert!(
            e[0].starts_with("data/gb/recipe/slab.json: not valid JSON"),
            "{e:?}"
        );
    }

    #[test]
    fn a_block_with_no_loot_table_or_name_is_a_warning() {
        let f = findings(
            MANIFEST,
            &[
                ("data/gb/loot_table/blocks/slab.json", ""),
                ("assets/gb/lang/en_us.json", r#"{"item.gb.shard":"Shard"}"#),
            ],
        );
        assert!(f.errors.is_empty(), "{f:?}");
        assert_eq!(f.warnings.len(), 2, "{f:?}");
        assert!(f.warnings[0].contains("no local loot table"));
        assert!(f.warnings[1].contains("has no \"block.gb.slab\""));
        let f = findings(MANIFEST, &[("assets/gb/lang/en_us.json", "")]);
        assert!(
            f.warnings
                .iter()
                .all(|w| w.contains("there is no assets/gb/lang/en_us.json"))
        );
        assert_eq!(f.warnings.len(), 2);
    }

    #[test]
    fn registry_content_on_a_client_package_is_a_warning() {
        let client = MANIFEST.replace("side = \"both\"\n", "");
        let f = findings(&client, &[]);
        assert!(f.errors.is_empty());
        assert_eq!(f.warnings.len(), 1, "{f:?}");
        assert!(f.warnings[0].contains("set side = \"both\""));
    }

    // One test per finding of the review of #2893: a valid mod the first version failed.

    /// 1. An optional entry may name nothing, and a removal names what it takes out.
    #[test]
    fn optional_and_removed_tag_entries_name_nothing() {
        let tag = r##"{"values":["gb:shard",{"id":"gb:later","required":false},{"id":"#gb:nothing","required":false},{"id":"gb:gone","remove":true}],"remove":["gb:gone","#gb:gone"]}"##;
        clean(MANIFEST, &[("data/gb/tags/item/things.json", tag)]);
        // A required entry still must exist.
        let required = r#"{"values":[{"id":"gb:later","required":true}]}"#;
        let e = errors(&[("data/gb/tags/item/things.json", required)]);
        assert!(e[0].contains("gb:later is not an item"), "{e:?}");
    }

    /// 2. A tag's registry can be two folders, as `worldgen/biome`.
    #[test]
    fn a_worldgen_tag_is_found_under_its_registry_path() {
        let cold = r##"{"values":["#gb:warm","minecraft:plains"]}"##;
        clean(
            MANIFEST,
            &[
                ("data/gb/tags/worldgen/biome/cold.json", cold),
                ("data/gb/tags/worldgen/biome/warm.json", r#"{"values":[]}"#),
            ],
        );
        let e = errors(&[("data/gb/tags/worldgen/biome/cold.json", cold)]);
        assert!(
            e[0].contains("expected data/gb/tags/worldgen/biome/warm.json"),
            "{e:?}"
        );
    }

    #[test]
    fn nested_tags_outside_the_checked_folders_produce_no_findings() {
        // A code mod can declare the registry `magic:spells/elemental`.
        let containing = "data/gb/tags/magic/spells/elemental/strong/fire.json";
        let member = "data/gb/tags/magic/spells/elemental/common/fire.json";
        let value = r##"{"values":[{"id":"#gb:common/fire"}]}"##;
        clean(
            MANIFEST,
            &[(containing, value), (member, r#"{"values":[]}"#)],
        );

        let known = "data/gb/tags/worldgen/structure/strong/fire.json";
        let e = errors(&[(known, value)]);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with(known));
        assert!(e[0].contains("data/gb/tags/worldgen/structure/common/fire.json"));
    }

    /// 3. A `[[jar]]` layer may supply what the files lack: without the jar cached, references
    ///    warn and missing blockstates are skipped; with the jar layer supplied, references
    ///    resolve against mod ∪ jar and missing files are reported.
    #[test]
    fn a_jar_layer_may_supply_what_the_files_lack() {
        let manifest = format!("{MANIFEST}{}", jar());
        let tag = r##"{"values":["#gb:from_jar","gb:from_jar"]}"##;
        let f = findings(
            &manifest,
            &[
                ("data/gb/tags/item/things.json", tag),
                ("assets/gb/textures/block/stone.png", ""),
                ("assets/gb/blockstates/slab.json", ""),
            ],
        );
        assert!(f.errors.is_empty(), "{f:?}");
        assert_eq!(f.warnings.len(), 3, "{f:?}");
        assert!(f.warnings.iter().all(|w| w.contains("[[jar]] layer")));
        assert!(
            !f.warnings.iter().any(|w| w.contains("blockstate")),
            "{f:?}"
        );
        assert!(
            f.warnings
                .iter()
                .any(|w| w.contains("could not find the tag #gb:from_jar"))
        );
        // A declared dependency's jar can do the same; the tool does not fetch it.
        let dependent = format!("{MANIFEST}[dependencies]\nchem_lib = \"1\"\n");
        let f = findings(&dependent, &[("data/gb/tags/item/things.json", tag)]);
        assert!(f.errors.is_empty(), "{f:?}");
        assert_eq!(f.warnings.len(), 2, "{f:?}");
        // The same mod without a jar breaks.
        let e = errors(&[("data/gb/tags/item/things.json", tag)]);
        assert_eq!(e.len(), 2, "{e:?}");
    }

    #[test]
    fn a_jar_layer_resolves_blockstate_and_model_and_reports_missing_ones() {
        let manifest = format!("{MANIFEST}{}", jar());
        let model_block = r#"{"template":"minecraft:stone","properties":{"render_shape":"MODEL"}}"#;
        let stripped = [
            ("data/gb/sunrise/block/slab.json", model_block),
            ("assets/gb/blockstates/slab.json", ""),
            ("assets/gb/models/block/slab.json", ""),
            ("assets/gb/items/slab.json", ""),
            ("data/gb/loot_table/blocks/slab.json", ""),
            ("assets/gb/lang/en_us.json", ""),
        ];
        // With no layer supplied and [[jar]] listed, blockstate/loot/item/lang checks are quiet.
        let no_layer = findings_with_jars(&manifest, &stripped, None);
        assert!(no_layer.errors.is_empty(), "{no_layer:?}");
        assert!(no_layer.warnings.is_empty(), "{no_layer:?}");

        // A jar layer holding the blockstate, model, item definition, loot table and lang makes
        // check completely quiet.
        let mut complete = JarLayer::new();
        complete.insert_file("assets/gb/blockstates/slab.json");
        complete.insert_file("assets/gb/models/block/slab.json");
        complete.insert_file("assets/gb/items/slab.json");
        complete.insert_file("data/gb/loot_table/blocks/slab.json");
        complete.insert_lang(
            "assets/gb/lang/en_us.json",
            br#"{"block.gb.slab":"Slab","item.gb.shard":"Shard"}"#.to_vec(),
        );
        assert_eq!(
            findings_with_jars(&manifest, &stripped, Some(&complete)),
            Findings::default()
        );

        // Missing the blockstate from the jar layer reports that exact file.
        let mut no_blockstate = complete.clone();
        no_blockstate
            .files
            .remove("assets/gb/blockstates/slab.json");
        let f = findings_with_jars(&manifest, &stripped, Some(&no_blockstate));
        assert_eq!(f.errors.len(), 1, "{f:?}");
        assert!(
            f.errors[0].contains("assets/gb/blockstates/slab.json"),
            "{f:?}"
        );
        assert!(f.warnings.is_empty(), "{f:?}");

        // Missing the block model referenced by the mod's own item definition reports that exact
        // model file.
        let mut no_model = complete.clone();
        no_model.files.remove("assets/gb/models/block/slab.json");
        let with_item_def = [
            ("data/gb/sunrise/block/slab.json", model_block),
            ("assets/gb/blockstates/slab.json", ""),
            ("assets/gb/models/block/slab.json", ""),
            ("data/gb/loot_table/blocks/slab.json", ""),
            ("assets/gb/lang/en_us.json", ""),
        ];
        let f = findings_with_jars(&manifest, &with_item_def, Some(&no_model));
        assert_eq!(f.errors.len(), 1, "{f:?}");
        assert!(
            f.errors[0].contains("assets/gb/models/block/slab.json"),
            "{f:?}"
        );
        assert!(f.warnings.is_empty(), "{f:?}");
    }

    /// 4. A world fluid's flowing form is a fluid, its bucket an item, its block a block.
    #[test]
    fn a_world_fluid_brings_its_generated_ids() {
        clean(
            MANIFEST,
            &[
                ("data/gb/sunrise/fluid/oil.json", r#"{"world":{}}"#),
                (
                    "assets/gb/lang/en_us.json",
                    r#"{"block.gb.slab":"Slab","item.gb.shard":"Shard"}"#,
                ),
                (
                    "data/minecraft/tags/fluid/water.json",
                    r#"{"values":["gb:oil","gb:flowing_oil"]}"#,
                ),
                (
                    "data/minecraft/tags/block/mineable/shovel.json",
                    r#"{"values":["gb:oil"]}"#,
                ),
                (
                    "data/gb/recipe/bucket.json",
                    r#"{"type":"minecraft:stonecutting","ingredient":"gb:oil_bucket","result":{"id":"minecraft:bucket","count":1}}"#,
                ),
            ],
        );
        let e = errors(&[
            ("data/gb/sunrise/fluid/oil.json", r#"{"world":{}}"#),
            (
                "data/minecraft/tags/fluid/water.json",
                r#"{"values":["gb:flowing_gas"]}"#,
            ),
        ]);
        assert!(e[0].contains("gb:flowing_gas is not a fluid"), "{e:?}");
    }
    /// 5. An atlas source may rename a sprite or take a directory; a child model sets its own sprite.
    #[test]
    fn atlas_sources_and_model_overrides_may_supply_a_texture() {
        let atlas =
            r#"{"sources":[{"type":"minecraft:directory","source":"stone","prefix":"block/"}]}"#;
        let f = findings(
            MANIFEST,
            &[
                ("assets/gb/atlases/blocks.json", atlas),
                ("assets/gb/textures/block/stone.png", ""),
            ],
        );
        assert!(f.errors.is_empty(), "{f:?}");
        assert_eq!(f.warnings.len(), 1, "{f:?}");
        assert!(f.warnings[0].contains("could not find the texture gb:block/stone"));
        assert!(f.warnings[0].contains("atlas source"));
        clean(
            MANIFEST,
            &[
                (
                    "assets/gb/models/block/base.json",
                    r#"{"textures":{"all":"gb:block/placeholder"}}"#,
                ),
                (
                    "assets/gb/models/block/slab.json",
                    r#"{"parent":"gb:block/base","textures":{"all":"gb:block/stone"}}"#,
                ),
            ],
        );
        // A slab model that leaves the slot alone still needs the parent's sprite.
        let e = errors(&[
            (
                "assets/gb/models/block/base.json",
                r#"{"textures":{"all":"gb:block/placeholder"}}"#,
            ),
            (
                "assets/gb/models/block/slab.json",
                r#"{"parent":"gb:block/base"}"#,
            ),
        ]);
        assert!(e[0].contains("the texture gb:block/placeholder"), "{e:?}");
    }

    #[test]
    fn an_inline_composite_child_can_override_an_inherited_sprite() {
        let base = "assets/gb/models/block/base.json";
        let slab = "assets/gb/models/block/slab.json";
        let placeholder =
            r##"{"parent":"minecraft:block/cube_all","textures":{"all":"gb:block/placeholder"}}"##;
        let composite = |slots| {
            format!(
                r#"{{"loader":"neoforge:composite","children":{{"body":{{"parent":"gb:block/base"{slots}}}}}}}"#
            )
        };
        let f = findings(
            MANIFEST,
            &[
                (base, placeholder),
                (slab, &composite(r#", "textures":{"all":"gb:block/stone"}"#)),
            ],
        );
        assert!(f.errors.is_empty(), "{f:?}");
        assert_eq!(f.warnings.len(), 1, "{f:?}");
        assert!(f.warnings[0].starts_with(base));
        assert!(f.warnings[0].contains("inline composite child"));
        assert!(f.warnings[0].contains("textures[\"all\"]"));
        let e = errors(&[(base, placeholder), (slab, &composite(""))]);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with(base));
        assert!(e[0].contains("gb:block/placeholder"));
    }

    /// 7. Payloads only a mod or the stack reads name no ids.
    #[test]
    fn opaque_payloads_are_not_scanned_for_ids() {
        clean(
            MANIFEST,
            &[
                (
                    "data/gb/recipe/machine.json",
                    r#"{"type":"gb:machine","extra":{"input":"gb:nope","items":["gb:nope"]},"result":{"id":"gb:slab"}}"#,
                ),
                (
                    "data/gb/recipe/slab.json",
                    r#"{"type":"minecraft:crafting_shapeless","ingredients":["gb:shard",{"type":"neoforge:components","items":"gb:shard","components":{"minecraft:custom_data":{"id":"gb:nope"}}}],"result":{"id":"gb:slab","components":{"minecraft:custom_data":{"item":"gb:nope","id":"gb:nope"}}}}"#,
                ),
                (
                    "data/gb/loot_table/blocks/slab.json",
                    r#"{"pools":[{"entries":[{"type":"minecraft:item","name":"gb:slab","functions":[{"function":"minecraft:set_custom_data","tag":{"type":"minecraft:item","name":"gb:nope","block":"gb:nope"}}]}]}]}"#,
                ),
                (
                    "assets/gb/blockstates/slab.json",
                    r#"{"neoforge:definition_type":"gb:custom","model":"gb:block/gone","config":{"model":"gb:block/gone"}}"#,
                ),
                (
                    "assets/gb/items/slab.json",
                    r#"{"model":{"type":"gb:custom","model":"gb:item/gone"}}"#,
                ),
            ],
        );
    }

    /// 8. A file the load conditions may disable is not read.
    #[test]
    fn files_the_load_conditions_may_disable_are_not_read() {
        let when = r#""neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"other"}]"#;
        clean(
            MANIFEST,
            &[
                (
                    "data/gb/recipe/other.json",
                    &format!(
                        r#"{{{when},"type":"minecraft:smelting","ingredient":"gb:nope","result":{{"id":"minecraft:stick"}}}}"#
                    ),
                ),
                (
                    "data/gb/tags/item/other.json",
                    &format!(r##"{{{when},"values":["gb:nope","#gb:nope"]}}"##),
                ),
                (
                    "data/gb/loot_table/blocks/slab.json",
                    &format!(
                        r#"{{"pools":[{{{when},"entries":[{{"type":"minecraft:item","name":"gb:nope"}}]}},{{"entries":[{{"type":"minecraft:item","name":"gb:slab"}}]}}]}}"#
                    ),
                ),
            ],
        );
    }

    #[test]
    fn the_model_decoder_leaves_trailing_input_unread() {
        let file = "assets/gb/models/block/slab.json";
        let trailing = r#"{"textures":{"all":"gb:block/stone"}} this is not part of the model"#;
        clean(MANIFEST, &[(file, trailing)]);
        let e = errors(&[(file, r#"{"textures":{"all":}}"#)]);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with(file));
        assert!(e[0].contains("could not read this model"));
    }

    /// 9. The generic JSON reader accepts comments, and the model decoder accepts a BOM.
    #[test]
    fn a_byte_order_mark_and_comments_are_read_as_the_game_reads_them() {
        let recipe = "\u{feff}{\n// a note\n\"type\":\"minecraft:crafting_shaped\", /* the key */\n\"pattern\":[\"SS\"],\"key\":{\"S\":\"gb:shard\"},\"result\":{\"id\":\"gb:slab\"}}";
        clean(MANIFEST, &[("data/gb/recipe/slab.json", recipe)]);
        let model = "\u{feff}{\"parent\":\"minecraft:block/cube_all\",\"textures\":{\"all\":\"gb:block/stone\"}}\n// trailing";
        clean(MANIFEST, &[("assets/gb/models/block/slab.json", model)]);
        // A comment hides nothing: the id after it is still checked.
        let e = errors(&[(
            "data/gb/recipe/slab.json",
            "{\"type\":\"minecraft:smelting\",\"ingredient\":\"gb:nope\", // note\n\"result\":{\"id\":\"gb:slab\"}}",
        )]);
        assert!(e[0].contains("gb:nope is not an item"), "{e:?}");
    }

    /// 10. A jar's entries join the layer under `/` paths, and an English names file over the
    ///     content limit fails the jar rather than leaving its names out.
    #[test]
    fn a_jar_archive_is_read_within_the_content_limits() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, entries: &[(&str, &[u8])]| {
            let path = dir.path().join(name);
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (entry, bytes) in entries {
                zip.start_file(*entry, options).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            path
        };
        let mut layer = JarLayer::new();
        let jar = write("ok.jar", &[("\\assets\\gb\\lang\\en_us.json", b"{}")]);
        layer.add_archive(&jar).unwrap();
        assert!(layer.contains("assets/gb/lang/en_us.json"), "{layer:?}");
        assert_eq!(
            layer.lang_files["assets/gb/lang/en_us.json"],
            [b"{}".to_vec()]
        );
        let big = vec![b' '; (crate::content::MAX_FILE + 1) as usize];
        let jar = write("big.jar", &[("assets/gb/lang/en_us.json", &big)]);
        let error = JarLayer::new().add_archive(&jar).unwrap_err();
        assert!(
            error.ends_with("assets/gb/lang/en_us.json is over 16 MiB"),
            "{error}"
        );
    }
}
