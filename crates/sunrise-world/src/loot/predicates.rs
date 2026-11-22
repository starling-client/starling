//! `Registries.PREDICATE`: the reloadable registry a loot condition's string
//! form names (`LootItemCondition.CODEC`, a `RegistryFileCodec` over the
//! predicate registry), read from `data/<ns>/predicate/` of the resource root
//! and the packs over it. A pack replaces a built-in predicate
//! (`minecraft:tool/can_silk_touch`) or adds its own, and block tables, item
//! modifiers and entity tables all reference those ids.
use super::PackOverlay;
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::path::Path;

/// Every predicate id (`minecraft:tool/can_silk_touch`, `mypack:my_silk`) of
/// every namespace in `root`'s `data/`, with `overlay`'s files over them: the
/// top pack's file for an id replaces the rest, as
/// `SimpleJsonResourceReloadListener` sees one resource per id.
pub fn load_predicates(root: &Path, overlay: &PackOverlay) -> io::Result<HashMap<String, Value>> {
    super::types::load_files(root, overlay, "predicate", |bytes| {
        serde_json::from_slice(bytes)
    })
}
