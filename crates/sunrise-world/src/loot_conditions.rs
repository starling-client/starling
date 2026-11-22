use super::*;
/// A kind's overlay files over what the resource root provided: the top
/// pack's file for each id replaces the rest (`SimpleJsonResourceReloadListener`
/// sees one resource per id).
pub(super) fn overlay_map(overlay: &PackOverlay, kind: &str, out: &mut HashMap<String, Value>) {
    for (id, files) in overlay.kind(kind) {
        let Ok(v) = serde_json::from_slice::<Value>(PackOverlay::top(files)) else {
            continue;
        };
        out.insert(id.to_owned(), v);
    }
    // Only a loot table's pools carry their own conditions (#1381).
    out.retain(|id, value| match kind {
        "loot_table" => overlay.conditions.admit_loot_table(value, id),
        _ => overlay.conditions.allows_id(value, kind, id),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conditions::LoadConditions;
    use serde_json::json;

    fn file(kind: &str, value: Value) -> (String, Vec<Vec<u8>>) {
        (
            format!("{kind}/demo:file"),
            vec![value.to_string().into_bytes()],
        )
    }

    fn read(kind: &str, value: Value) -> Value {
        let files = [file(kind, value)];
        let conditions = LoadConditions::new(["create"]);
        let overlay = PackOverlay {
            files: &files,
            conditions: &conditions,
            ..PackOverlay::default()
        };
        let mut out = HashMap::new();
        overlay_map(&overlay, kind, &mut out);
        // A nested key outside pool entries is never evaluated: its unknown
        // condition leaves no diagnostic.
        assert_eq!(conditions.check_errors(), Ok(()), "{kind}");
        out.remove("demo:file").expect("the file loads")
    }

    /// #1381: a loot table's pools carry their own conditions: a failing pool
    /// is dropped and a passing one loses the key. A pool's `entries` and
    /// `functions` and an entry's `children` are plain lists: their key stays as
    /// data, never evaluated, even naming an unknown condition.
    #[test]
    fn a_loot_tables_pools_are_filtered_and_nothing_else() {
        let unknown = json!([{"type": "absent:condition"}]);
        let create = json!([{"type": "neoforge:mod_loaded", "modid": "create"}]);
        let ic2 = json!([{"type": "neoforge:mod_loaded", "modid": "ic2"}]);
        let inner = json!({
            "entries": [
                {"type": "minecraft:item", "name": "ic2:tin", "neoforge:conditions": unknown},
                {"type": "minecraft:alternatives", "children": [
                    {"type": "minecraft:item", "name": "minecraft:stick", "neoforge:conditions": unknown}
                ]}
            ],
            "functions": [{"function": "minecraft:set_count", "count": 2, "neoforge:conditions": unknown}]
        });
        let pool = |rolls: i32, conditions: Option<&Value>| {
            let mut pool = inner.clone();
            pool["rolls"] = json!(rolls);
            if let Some(conditions) = conditions {
                pool["neoforge:conditions"] = conditions.clone();
            }
            pool
        };
        let table = read(
            "loot_table",
            json!({"pools": [pool(1, Some(&create)), pool(2, Some(&ic2)), pool(3, None)]}),
        );
        assert_eq!(table, json!({"pools": [pool(1, None), pool(3, None)]}));
    }

    /// #1381: in a predicate, an item modifier, an enchantment or a recipe, a
    /// list element's `neoforge:conditions` is ordinary data: never evaluated,
    /// so even an unknown condition keeps the element, and the key stays.
    #[test]
    fn other_kinds_keep_a_list_elements_conditions_as_data() {
        let element =
            json!({"item": "ic2:tin", "neoforge:conditions": [{"type": "absent:condition"}]});
        for kind in ["predicate", "item_modifier", "enchantment", "recipe"] {
            let value = json!({"terms": [element.clone()], "extra": {"list": [element.clone()]}});
            assert_eq!(read(kind, value.clone()), value, "{kind}");
        }
    }

    #[test]
    fn a_pool_with_an_unknown_condition_refuses_its_whole_table() {
        let files = [file(
            "loot_table",
            json!({"pools":[
                {"rolls":1},
                {"rolls":1,"neoforge:conditions":[{"type":"unknown:pool"}]}
            ]}),
        )];
        let conditions = LoadConditions::new([]);
        let mut out = HashMap::new();
        overlay_map(
            &PackOverlay {
                files: &files,
                conditions: &conditions,
                ..PackOverlay::default()
            },
            "loot_table",
            &mut out,
        );
        assert!(out.is_empty());
        let error = conditions.check_errors().unwrap_err();
        assert!(
            error
                .contains("data/demo/loot_table/file.json: unknown load conditions [unknown:pool]"),
            "{error}"
        );
    }

    /// The whole-file check still holds for every kind.
    #[test]
    fn a_files_own_failing_conditions_still_omit_it() {
        for kind in ["loot_table", "predicate", "recipe"] {
            let files = [file(
                kind,
                json!({"neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": "ic2"}]}),
            )];
            let conditions = LoadConditions::new(["create"]);
            let overlay = PackOverlay {
                files: &files,
                conditions: &conditions,
                ..PackOverlay::default()
            };
            let mut out = HashMap::new();
            overlay_map(&overlay, kind, &mut out);
            assert!(out.is_empty(), "{kind}");
        }
    }
}
