//! Holder sets as data files write them, read by one decoder for every field
//! vanilla types as a `HolderSet`: an id, a `#tag`, a list of ids
//! (`HolderSetCodec`), and NeoForge's `neoforge:any`, `neoforge:and` /
//! `neoforge:or` (`values`) and `neoforge:not` (`value`) objects, nested freely
//! (`AnyHolderSet`, `AndHolderSet`, `OrHolderSet`, `NotHolderSet`).
//! Mod-defined holder-set types wait on the typed codec foundation (#811).
use serde_json::Value;
use std::collections::BTreeSet;

/// One registry, as a reader binds it: what an id and a tag resolve to.
pub trait HolderLookup<E = String> {
    type Id: Ord + Clone;
    /// The registry's entry noun in errors (`biome`, `block`, `item`...).
    fn what(&self) -> &str;
    /// The members one entry id stands for (a block stands for all its states).
    fn entry(&self, id: &str) -> Result<Vec<Self::Id>, E>;
    /// A tag's members; `tag` has no `#`.
    fn tag(&self, tag: &str) -> Result<Vec<Self::Id>, E>;
    /// Every entry: what `any` holds and what `not` takes its value from.
    fn all(&self) -> Result<Vec<Self::Id>, E>;
}

/// The members `value` names in `lookup`'s registry, in holder-set order
/// without repeats: `or` concatenates its values, `and` keeps its first
/// value's order, `any` and `not` follow `all`. Lookup failures keep their error type.
pub fn resolve<L: HolderLookup<E> + ?Sized, E: From<String>>(
    value: &Value,
    lookup: &L,
) -> Result<Vec<L::Id>, E> {
    decode(value, lookup, false, None)
}

/// `HolderSetCodec.directCodec` with the registry element's inline codec.
/// Vanilla direct lists keep holder order and multiplicity; NeoForge's
/// composite sets still use the shared set operations.
pub(crate) fn resolve_with_inline<L: HolderLookup<E>, E: From<String>>(
    value: &Value,
    lookup: &L,
    always_list: bool,
    inline: &dyn Fn(&Value) -> Result<L::Id, E>,
) -> Result<Vec<L::Id>, E> {
    decode(value, lookup, always_list, Some(inline))
}

type InlineHolder<'a, Id, E> = &'a dyn Fn(&Value) -> Result<Id, E>;

fn decode<L: HolderLookup<E> + ?Sized, E: From<String>>(
    value: &Value,
    lookup: &L,
    always_list: bool,
    inline: Option<InlineHolder<'_, L::Id, E>>,
) -> Result<Vec<L::Id>, E> {
    let element = |value: &Value| match value {
        Value::String(name) if !name.starts_with('#') => lookup.entry(name),
        Value::Object(_) => {
            inline.ok_or_else(|| format!("not a {} holder", lookup.what()))?(value).map(|v| vec![v])
        }
        _ => Err(format!("not a {} holder", lookup.what()).into()),
    };
    let nested = |value| decode(value, lookup, always_list, inline);
    let direct = match value {
        Value::String(name) if name.starts_with('#') => return lookup.tag(&name[1..]).map(unique),
        Value::Array(list) => {
            let mut out = Vec::new();
            for v in list {
                out.extend(element(v)?);
            }
            out
        }
        Value::Object(object) => {
            let kind = object.get("type").and_then(Value::as_str).unwrap_or("");
            match kind.strip_prefix("neoforge:").unwrap_or(kind) {
                "any" => return lookup.all().map(unique),
                short @ ("or" | "and") => {
                    let values = object
                        .get("values")
                        .and_then(Value::as_array)
                        .ok_or_else(|| format!("{} holder set {kind}: no values", lookup.what()))?;
                    let mut sets = values.iter().map(nested);
                    let mut out = unique(sets.next().transpose()?.unwrap_or_default());
                    if short == "or" {
                        for set in sets {
                            out.extend(set?);
                        }
                        return Ok(unique(out));
                    }
                    for set in sets {
                        let set: BTreeSet<_> = set?.into_iter().collect();
                        out.retain(|id| set.contains(id));
                    }
                    return Ok(out);
                }
                "not" => {
                    let value = object
                        .get("value")
                        .ok_or_else(|| format!("{} holder set {kind}: no value", lookup.what()))?;
                    let inner: BTreeSet<_> = nested(value)?.into_iter().collect();
                    return Ok(unique(lookup.all()?)
                        .into_iter()
                        .filter(|id| !inner.contains(id))
                        .collect());
                }
                _ if !always_list && inline.is_some() => element(value)?,
                _ => return Err(format!("{} set type {kind:?} is not read", lookup.what()).into()),
            }
        }
        _ if !always_list => element(value)?,
        _ => {
            return Err(format!(
                "{} holder set must be a list, tag or custom set",
                lookup.what()
            )
            .into());
        }
    };
    Ok(if inline.is_some() {
        direct
    } else {
        unique(direct)
    })
}

/// `ids` in order, each first occurrence kept.
fn unique<T: Ord + Clone>(ids: Vec<T>) -> Vec<T> {
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// The ids a holder set names in a registry of string ids: the shape loot,
/// enchantment and biome-modifier readers keep.
pub struct StringLookup<'a> {
    pub what: &'a str,
    /// The registry's ids; `None` when the reader can't list them, so `any`
    /// and `not` fail with an error naming the registry.
    pub ids: Option<&'a dyn Fn() -> Vec<String>>,
    pub tag: &'a dyn Fn(&str) -> Result<Vec<String>, String>,
    /// What one id stands for: itself, or an error for an id the reader refuses.
    pub entry: &'a dyn Fn(&str) -> Result<Vec<String>, String>,
}
impl HolderLookup for StringLookup<'_> {
    type Id = String;
    fn what(&self) -> &str {
        self.what
    }
    fn entry(&self, id: &str) -> Result<Vec<String>, String> {
        (self.entry)(id)
    }
    fn tag(&self, tag: &str) -> Result<Vec<String>, String> {
        (self.tag)(tag)
    }
    fn all(&self) -> Result<Vec<String>, String> {
        self.ids
            .map(|ids| ids())
            .ok_or_else(|| format!("{} universe is unavailable", self.what))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Biomes;
    impl HolderLookup for Biomes {
        type Id = &'static str;
        fn what(&self) -> &str {
            "biome"
        }
        fn entry(&self, id: &str) -> Result<Vec<&'static str>, String> {
            ["minecraft:plains", "minecraft:ocean", "minecraft:desert"]
                .into_iter()
                .find(|b| *b == id)
                .map(|b| vec![b])
                .ok_or_else(|| format!("unknown biome {id}"))
        }
        fn tag(&self, tag: &str) -> Result<Vec<&'static str>, String> {
            match tag {
                "minecraft:is_ocean" => Ok(vec!["minecraft:ocean"]),
                "minecraft:is_dry" => Ok(vec!["minecraft:desert"]),
                _ => Err(format!("unknown biome tag {tag}")),
            }
        }
        fn all(&self) -> Result<Vec<&'static str>, String> {
            Ok(vec![
                "minecraft:plains",
                "minecraft:ocean",
                "minecraft:desert",
            ])
        }
    }

    fn set(value: Value) -> Result<Vec<&'static str>, String> {
        resolve(&value, &Biomes)
    }

    #[test]
    fn unavailable_universes_and_missing_object_fields_name_the_registry() {
        let lookup = StringLookup {
            what: "instrument",
            ids: None,
            tag: &|_| Ok(Vec::new()),
            entry: &|id| Ok(vec![id.to_owned()]),
        };
        for value in [
            json!({"type":"neoforge:any"}),
            json!({"type":"neoforge:not", "value":[]}),
        ] {
            let error = resolve(&value, &lookup).unwrap_err();
            assert!(
                error.contains("instrument") && error.contains("unavailable"),
                "{error}"
            );
        }
        for kind in ["neoforge:and", "neoforge:or", "neoforge:not"] {
            let error = resolve(&json!({"type":kind}), &lookup).unwrap_err();
            assert!(
                error.contains("instrument") && error.contains(kind),
                "{error}"
            );
        }
    }

    #[test]
    fn plain_forms_resolve_as_vanilla_reads_them() {
        assert_eq!(set(json!("minecraft:plains")), Ok(vec!["minecraft:plains"]));
        assert_eq!(
            set(json!("#minecraft:is_ocean")),
            Ok(vec!["minecraft:ocean"])
        );
        assert_eq!(
            set(json!([
                "minecraft:plains",
                "minecraft:desert",
                "minecraft:plains"
            ])),
            Ok(vec!["minecraft:plains", "minecraft:desert"])
        );
        assert!(set(json!(["#minecraft:is_ocean"])).is_err());
        assert!(set(json!(3)).is_err());
    }

    #[test]
    fn neoforge_objects_nest() {
        assert_eq!(
            set(json!({"type": "neoforge:not", "value": "#minecraft:is_ocean"})),
            Ok(vec!["minecraft:plains", "minecraft:desert"])
        );
        assert_eq!(
            set(json!({"type": "neoforge:and", "values": [
                {"type": "neoforge:any"},
                {"type": "neoforge:not", "value": {"type": "neoforge:or", "values": [
                    "#minecraft:is_ocean", "minecraft:plains"]}}
            ]})),
            Ok(vec!["minecraft:desert"])
        );
        assert_eq!(set(json!({"type": "neoforge:any"})).unwrap().len(), 3);
        assert!(set(json!({"type": "neoforge:or"})).is_err());
        assert!(set(json!({"type": "example:custom"})).is_err());
    }
}
