use super::*;
use serde_json::json;

#[test]
fn isolated_file_returns_all_unknown_ids_without_poisoning_the_candidate() {
    let bad = json!({"neoforge:conditions":[{"type":"neoforge:or","values":[
        {"type":"neoforge:always"}, {"type":"othermod:unknown"},
        {"type":"neoforge:not","value":{"type":"minecraft:missing"}},
        {"type":"othermod:unknown"}
    ]}]});
    let good = json!({"neoforge:conditions":[{"type":"neoforge:always"}]});
    let vanilla = LoadConditions::vanilla();
    let candidate = LoadConditions::new([]).with_lookup([("test:flag", |_: &Value| Ok(true))]);
    for conditions in [vanilla, candidate] {
        assert_eq!(
            conditions.allows_file_isolated(&bad, "bad.json"),
            Err("unknown load conditions [minecraft:missing, othermod:unknown]".into())
        );
        assert_eq!(
            conditions.allows_file_isolated(&good, "good.json"),
            Ok(true)
        );
        assert_eq!(conditions.check_errors(), Ok(()));
    }
}
