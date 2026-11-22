//! Extension tag records come from the same explicit inputs as the package and book catalogs.
use mod_surface::ExtensionTag;
use serde::{Deserialize, de::DeserializeOwned};
use std::{collections::BTreeSet, fs, path::Path};

const DIRECTORY: &str = "crates/sunrise-mod-vocab/extension-tags";

#[derive(Deserialize)]
#[serde(untagged)]
enum Family {
    Tags(Vec<ExtensionTag>),
    Source(String),
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read(root: &Path) -> Result<Vec<ExtensionTag>, String> {
    let dir = root.join(DIRECTORY);
    let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut tags = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.extension().is_some_and(|ext| ext == "json") && path.is_file() {
            tags.extend(match read_json(&path)? {
                Family::Tags(rows) => rows,
                Family::Source(source) => read_json(&root.join(source))?,
            });
        }
    }
    let mut seen = BTreeSet::new();
    for tag in &tags {
        if !tag.id.starts_with("sunrise:")
            || tag.id.starts_with("sunrise:class/")
            || tag.registry.is_empty()
            || tag.doc.is_empty()
            || !seen.insert((&tag.registry, &tag.id))
        {
            return Err(format!(
                "invalid or duplicate extension tag {}/{}",
                tag.registry, tag.id
            ));
        }
    }
    tags.sort_by(|a, b| (&a.registry, &a.id).cmp(&(&b.registry, &b.id)));
    Ok(tags)
}
