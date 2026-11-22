use super::*;
use serde::{
    Deserializer,
    de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, value::RawValue};

pub(super) fn load_kind(
    data: &Path,
    kind: &str,
    out: &mut HashMap<String, Value>,
) -> io::Result<()> {
    for ns in crate::json_folder::namespaces(data) {
        load_dir(
            &data.join(&ns).join(kind),
            &ns,
            "",
            out,
            kind == "enchantment",
        )?;
    }
    Ok(())
}

pub(super) fn load_ordered_kind(
    root: &Path,
    overlay: &PackOverlay,
    kind: &str,
) -> io::Result<HashMap<String, Value>> {
    let registry: Value = match fs::read(root.join("registries.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Value::Null,
        Err(error) => return Err(error),
    };
    let reader = LootJson(registry["minecraft:data_component_type"]["entries"].as_object());
    types::load_files(root, overlay, kind, |bytes| reader.read(bytes))
}

/// Read loot JSON normally except for the component patch of `set_components`.
/// That map must resolve spellings in document order before entering JSON Value.
#[derive(Clone, Copy)]
struct LootJson<'a>(Option<&'a Map<String, Value>>);
impl LootJson<'_> {
    fn read(self, bytes: &[u8]) -> serde_json::Result<Value> {
        let mut input = serde_json::Deserializer::from_slice(bytes);
        let value = self.deserialize(&mut input)?;
        input.end()?;
        Ok(value)
    }
}
impl<'de> DeserializeSeed<'de> for LootJson<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, input: D) -> Result<Value, D::Error> {
        input.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for LootJson<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("loot JSON")
    }
    fn visit_bool<E: Error>(self, value: bool) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_i64<E: Error>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_u64<E: Error>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_f64<E: Error>(self, value: f64) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_str<E: Error>(self, value: &str) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_string<E: Error>(self, value: String) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_unit<E: Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut output = Vec::new();
        while let Some(value) = seq.next_element_seed(self)? {
            output.push(value);
        }
        Ok(Value::Array(output))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut output = Map::new();
        let mut components: Option<&RawValue> = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "components" {
                components = Some(map.next_value()?);
            } else if matches!(
                key.as_str(),
                "pools" | "entries" | "children" | "modifier" | "functions" | "on_pass" | "on_fail"
            ) {
                output.insert(key, map.next_value_seed(self)?);
            } else {
                output.insert(key, map.next_value()?);
            }
        }
        let kind = output
            .get("type")
            .or_else(|| output.get("function"))
            .and_then(Value::as_str);
        let patch = matches!(kind, Some("minecraft:set_components" | "set_components"));
        if let Some(components) = components {
            let value = if patch {
                serde_json::Deserializer::from_str(components.get())
                    .deserialize_map(ComponentPatch(self.0))
            } else {
                serde_json::from_str(components.get())
            }
            .map_err(A::Error::custom)?;
            output.insert("components".into(), value);
        } else if patch {
            return Err(A::Error::custom(
                "set_components requires a components object",
            ));
        }
        Ok(Value::Object(output))
    }
}

struct ComponentPatch<'a>(Option<&'a Map<String, Value>>);
impl<'de> Visitor<'de> for ComponentPatch<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a component patch object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut output = Map::new();
        while let Some((key, value)) = map.next_entry::<String, Value>()? {
            let removal = key.strip_prefix('!');
            let id = Identifier::parse(removal.unwrap_or(&key))
                .map_err(|_| A::Error::custom(format!("invalid component ID {key:?}")))?
                .to_string();
            if self.0.is_some_and(|known| !known.contains_key(&id)) {
                return Err(A::Error::custom(format!("unknown component ID {id}")));
            }
            let marker = format!("!{id}");
            if removal.is_some() {
                if !value.is_object() {
                    return Err(A::Error::custom(format!(
                        "removal of {id} requires an object"
                    )));
                }
                output.remove(&id);
                output.insert(marker, Value::Object(Map::new()));
            } else {
                output.remove(&marker);
                output.insert(id, value);
            }
        }
        Ok(Value::Object(output))
    }
}

/// Every file's path, size and modification time under `dir`, in name order.
pub(super) fn hash_files(dir: &Path, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            hash_files(&path, hasher);
        } else {
            path.hash(hasher);
            meta.len().hash(hasher);
            meta.modified().ok().hash(hasher);
        }
    }
}

pub(super) fn load_dir(
    dir: &Path,
    namespace: &str,
    prefix: &str,
    out: &mut HashMap<String, Value>,
    enchantment: bool,
) -> io::Result<()> {
    // Through `json_folder`: a load holding the folder (the recipes, read by
    // the item tables, the recipe book and the server's sync too) shares one
    // read of it.
    let Ok(folder) = crate::json_folder::read(dir) else {
        return Ok(());
    };
    for file in &folder.files {
        let bytes = file.bytes()?;
        let value = if enchantment {
            let Some(value) = crate::json_folder::enchantment_definition(
                bytes,
                &dir.join(format!("{}.json", file.name)),
            ) else {
                continue;
            };
            value
        } else {
            serde_json::from_slice(bytes)?
        };
        out.insert(format!("{namespace}:{prefix}{}", file.name), value);
    }
    Ok(())
}
