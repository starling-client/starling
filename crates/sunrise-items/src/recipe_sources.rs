//! Shared recipe resource merge, conditions and filters.
use crate::Result;
use serde_json::Value;
use std::path::Path;
use sunrise_world::loot::PackOverlay;

/// A recipe's whole id from its name in the tables: vanilla's are kept bare
/// (`oak_planks`), everyone else's whole (`mymod:crusher/iron`). The book,
/// the network copy, the advancements' JSON and brewing know a recipe by
/// this id.
pub fn qualified(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// The data path of recipe `name` (`recipe_sources::sources`' ids).
pub fn source_path(name: &str) -> String {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    format!("data/{namespace}/recipe/{path}.json")
}

/// Why a recipe file's `type` could not be decoded.
#[derive(Debug, PartialEq, Eq)]
pub enum TypeRefusal {
    NotAnObject,
    MissingType,
    MalformedId(String),
}

impl std::fmt::Display for TypeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnObject => f.write_str("recipe is not a JSON object"),
            Self::MissingType => f.write_str("recipe has a missing or non-string type"),
            Self::MalformedId(id) => write!(f, "malformed recipe serializer id {id}"),
        }
    }
}

/// Decodes a recipe's `type` into its canonical `namespace:path`, in place:
/// a bare path and `:path` are `minecraft`'s (`Identifier.parse`), never the
/// recipe file's own namespace. Every recipe enters the game through this,
/// from disk (`sources`) or from a server (`lan::recipes::ContentPart`), so
/// that every reader sees the one spelling.
pub fn decode_type(recipe: &mut Value) -> std::result::Result<(), TypeRefusal> {
    let recipe = recipe.as_object_mut().ok_or(TypeRefusal::NotAnObject)?;
    let text = recipe
        .get("type")
        .and_then(Value::as_str)
        .ok_or(TypeRefusal::MissingType)?;
    let id = sunrise_core::identifier::Identifier::parse(text)
        .map_err(|error| TypeRefusal::MalformedId(format!("{text:?} ({error})")))?;
    let canonical = format!("{}:{}", id.namespace(), id.path());
    recipe.insert("type".into(), canonical.into());
    Ok(())
}

/// `sources`, or its refusals as one error: for readers that only log.
pub fn sources(root: &Path, overlay: &PackOverlay) -> Result<Vec<(String, Value)>> {
    let (sources, refused) = sources_collecting(root, overlay)?;
    if refused.is_empty() {
        return Ok(sources);
    }
    let lines: Vec<_> = refused.into_iter().map(|(_, line)| line).collect();
    Err(format!("recipes refused:\n  {}", lines.join("\n  ")).into())
}

/// The recipe files the tables are built from, each id once: vanilla's by
/// bare name, then the content mods' as `<ns>:<name>`, the world's packs'
/// over them (`overlay`), parsed, and only those whose load conditions
/// hold. A server sends some of them to its clients (`recipe_content`).
/// A malformed JSON file refuses the whole load; no partial set is returned.
/// A file whose `type` is refused (`decode_type`) is left out and listed with
/// its reason, by name, for the caller's one report of every failed recipe.
pub fn sources_collecting(
    root: &Path,
    overlay: &PackOverlay,
) -> Result<(Vec<(String, Value)>, Vec<(String, String)>)> {
    // Vanilla's recipes by bare name, then content mods' as `<ns>:<name>`.
    let mut sources: Vec<(String, Vec<u8>)> = Vec::new();
    // Load conditions: `mod_loaded` asks which mods' data is
    // in this root (`minecraft`, always, and the content mods').
    // `mod_loaded` asks which mods are loaded, not which namespaces
    // they own: a mod that also owns a namespace doesn't make that
    // namespace another mod.
    let inventory = sunrise_mod_package::enabled::EnabledMods::read(root)?;
    let conditions = overlay
        .conditions
        .with_mod_inventory(inventory.server_ids());
    for file in &sunrise_world::json_folder::read(&root.join("data/minecraft/recipe"))?.files {
        sources.push((file.name.clone(), file.bytes()?.to_vec()));
    }
    for namespace in crate::content_namespaces::namespaces(root) {
        let dir = root.join(format!("data/{namespace}/recipe"));
        if dir.is_dir() {
            for file in &sunrise_world::json_folder::read(&dir)?.files {
                sources.push((format!("{namespace}:{}", file.name), file.bytes()?.to_vec()));
            }
        }
    }
    // The world's packs' recipes over the game's own: the top pack's
    // file for each id replaces it (`SimpleJsonResourceReloadListener`
    // sees one resource per id, the resource stack's top).
    for (id, files) in overlay.kind("recipe") {
        let Some((ns, path)) = id.split_once(':') else {
            continue;
        };
        let name = if ns == "minecraft" {
            path.to_owned()
        } else {
            id.to_owned()
        };
        let bytes = PackOverlay::top(files).to_vec();
        match sources.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = bytes,
            None => sources.push((name, bytes)),
        }
    }
    let filters = crate::recipe_filters::RecipeFilters::load(root, overlay);
    let mut parsed = Vec::with_capacity(sources.len());
    let mut errors = Vec::new();
    let mut refused = Vec::new();
    for (name, bytes) in sources {
        let id = qualified(&name);
        let (namespace, path) = id.split_once(':').expect("qualified recipe ID");
        let path = format!("data/{namespace}/recipe/{path}.json");
        let mut v: Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                errors.push(format!("recipes: {path} refused: {error}"));
                continue;
            }
        };
        match conditions.allows_file_isolated(&v, &path) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) => {
                errors.push(format!("{path}: {error}"));
                continue;
            }
        }
        // The serializer id, once: the filters and every reader see it whole.
        if let Err(reason) = decode_type(&mut v) {
            refused.push((
                name.clone(),
                format!("recipe {}, {reason}", source_path(&name)),
            ));
            continue;
        }
        if filters.apply(&id, &mut v) {
            parsed.push((name, v));
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("; ").into());
    }
    Ok((parsed, refused))
}

#[cfg(test)]
#[path = "recipe_sources_tests.rs"]
mod tests;
