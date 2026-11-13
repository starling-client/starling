//! NeoForge's load conditions (`neoforge:conditions`): a data file that has
//! the key loads only when every condition in it holds, and is silently
//! absent otherwise (NeoForge's `ICondition`). Mods' packs lean on it
//! for cross-mod recipes — a recipe for another mod's material only makes
//! sense to load when that mod's data is in the root.
use serde_json::Value;

/// Enabled package IDs captured by the engine for one owning data load.
/// Readers share this immutable view; they never recover IDs from namespaces.
/// Resource selection happens before evaluating ordinary JSON files. Stacked
/// tags and data maps instead evaluate each layer before merge or replace.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LoadConditions {
    mods: Vec<String>,
}

impl LoadConditions {
    /// A vanilla data load has the built-in package only.
    pub const fn vanilla() -> Self {
        Self { mods: Vec::new() }
    }

    /// Capture IDs visible to the owning load's side. The engine supplies its
    /// enabled-mod inventory's server view for logical-server data.
    pub fn new<'a>(ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut mods: Vec<_> = ids
            .into_iter()
            .filter(|id| *id != "minecraft")
            .map(str::to_owned)
            .collect();
        mods.sort();
        mods.dedup();
        Self { mods }
    }

    pub fn mod_loaded(&self, id: &str) -> bool {
        id == "minecraft"
            || self
                .mods
                .binary_search_by(|loaded| loaded.as_str().cmp(id))
                .is_ok()
    }

    /// Unknown condition kinds fail closed through the shared evaluator.
    pub fn allows(&self, file: &Value) -> bool {
        conditions_hold(file, &|id| self.mod_loaded(id))
    }
}

/// What a condition asks: whether a mod is enabled for this data-load side.
/// Callers use the owning resource generation's enabled-mod inventory, plus
/// `minecraft`, always. Resource namespaces do not identify loaded mods.
pub type ModLoaded<'a> = dyn Fn(&str) -> bool + 'a;

/// Whether `v`'s `neoforge:conditions` all hold; a file without the key
/// loads, one with an unknown condition type doesn't.
pub fn conditions_hold(v: &Value, loaded: &ModLoaded) -> bool {
    fn hold(c: &Value, loaded: &ModLoaded) -> bool {
        let all = |key: &str| {
            c[key]
                .as_array()
                .into_iter()
                .flatten()
                .all(|c| hold(c, loaded))
        };
        match c["type"].as_str() {
            Some("neoforge:mod_loaded") => c["modid"].as_str().is_some_and(loaded),
            Some("neoforge:true") => true,
            // An unknown child fails the file: `not` of it is not a pass.
            Some("neoforge:not") => known(&c["value"]) && !hold(&c["value"], loaded),
            Some("neoforge:and") => all("values"),
            Some("neoforge:or") => {
                let values: Vec<&Value> = c["values"].as_array().into_iter().flatten().collect();
                values.iter().all(|v| known(v)) && values.iter().any(|v| hold(v, loaded))
            }
            _ => false,
        }
    }
    v["neoforge:conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|c| hold(c, loaded))
}

/// Whether the condition's kind is one the game knows; an unknown kind
/// anywhere in the tree leaves the file out, through `not` and the
/// combinators alike.
fn known(c: &Value) -> bool {
    if !matches!(
        c["type"].as_str(),
        Some("neoforge:mod_loaded")
            | Some("neoforge:true")
            | Some("neoforge:not")
            | Some("neoforge:and")
            | Some("neoforge:or")
    ) {
        return false;
    }
    // The whole tree must be known, not only this node: an unknown kind
    // nested inside `not` or a combinator leaves the file out too.
    match c["type"].as_str() {
        Some("neoforge:not") => known(&c["value"]),
        Some("neoforge:and") | Some("neoforge:or") => {
            c["values"].as_array().into_iter().flatten().all(known)
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn load_conditions_capture_package_ids_and_keep_their_generation() {
        let mut enabled = vec!["code_only".to_owned()];
        let old = LoadConditions::new(enabled.iter().map(String::as_str));
        enabled.clear();
        let next = LoadConditions::new(enabled.iter().map(String::as_str));
        let requires_code = json!({"neoforge:conditions":[{
            "type":"neoforge:mod_loaded", "modid":"code_only"
        }]});
        assert!(old.allows(&requires_code));
        assert!(!next.allows(&requires_code));
        assert!(old.mod_loaded("minecraft") && next.mod_loaded("minecraft"));
        assert!(!old.mod_loaded("resource_namespace"));
        assert!(old.allows(&json!({"values":[]})));
        assert!(!old.allows(&json!({"neoforge:conditions":[{"type":"future:condition"}]})));
    }

    #[test]
    fn load_condition_cache_identity_uses_membership_not_input_order() {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let key = |conditions: &LoadConditions| {
            let mut hasher = DefaultHasher::new();
            conditions.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(
            key(&LoadConditions::new(["b", "a", "a"])),
            key(&LoadConditions::new(["a", "b"]))
        );
        assert_ne!(
            key(&LoadConditions::new(["a"])),
            key(&LoadConditions::vanilla())
        );
        assert_eq!(
            LoadConditions::new(["minecraft"]),
            LoadConditions::vanilla()
        );
    }

    fn loaded(id: &str) -> bool {
        matches!(id, "minecraft" | "create")
    }

    #[test]
    fn a_file_without_conditions_loads() {
        assert!(conditions_hold(
            &json!({"type": "minecraft:crafting"}),
            &loaded
        ));
    }

    #[test]
    fn mod_loaded_follows_the_answer_and_minecraft_always_holds() {
        assert!(conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "create"}]}),
            &loaded
        ));
        assert!(!conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "ic2"}]}),
            &loaded
        ));
        assert!(conditions_hold(
            &json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "minecraft"}]}),
            &loaded
        ));
    }

    #[test]
    fn not_and_or_combine_and_all_must_hold() {
        let not = json!({"type": "neoforge:not", "value": {"type": "neoforge:mod_loaded", "modid": "ic2"}});
        let mut c = serde_json::Map::new();
        c.insert("neoforge:conditions".to_owned(), json!([not]));
        assert!(conditions_hold(&Value::Object(c.clone()), &loaded));
        let both = json!([
            {"type": "neoforge:mod_loaded", "modid": "create"},
            {"type": "neoforge:or", "values": [
                {"type": "neoforge:mod_loaded", "modid": "ic2"},
                {"type": "neoforge:true"}
            ]}
        ]);
        c.insert("neoforge:conditions".to_owned(), both);
        assert!(conditions_hold(&Value::Object(c), &loaded));
        // One failing condition in the list fails the file.
        let failing = json!([
            {"type": "neoforge:true"},
            {"type": "neoforge:mod_loaded", "modid": "ic2"}
        ]);
        let mut f = serde_json::Map::new();
        f.insert("neoforge:conditions".to_owned(), failing);
        assert!(!conditions_hold(&Value::Object(f), &loaded));
    }

    #[test]
    fn an_unknown_kind_nested_in_the_tree_leaves_the_file_out() {
        // `not` of `not` of an unknown: the unknown is two levels down.
        let nested_not = json!({
            "neoforge:conditions": [
                {"type": "neoforge:not", "value":
                    {"type": "neoforge:not", "value":
                        {"type": "example:unknown"}}}
            ]
        });
        assert!(!conditions_hold(&nested_not, &loaded));
        // `or` with a `true` beside a `not` of an unknown: one child is
        // known-true, but the other's subtree holds an unknown.
        let nested_or = json!({
            "neoforge:conditions": [
                {"type": "neoforge:or", "values": [
                    {"type": "neoforge:true"},
                    {"type": "neoforge:not", "value":
                        {"type": "example:unknown"}}
                ]}
            ]
        });
        assert!(!conditions_hold(&nested_or, &loaded));
    }

    #[test]
    fn an_unknown_condition_does_not_load_the_file() {
        let mut c = serde_json::Map::new();
        c.insert(
            "neoforge:conditions".to_owned(),
            json!([{"type": "neoforge:future_thing"}]),
        );
        assert!(!conditions_hold(&Value::Object(c), &loaded));
    }
}
