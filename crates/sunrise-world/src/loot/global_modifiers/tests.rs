use super::*;
use serde_json::json;
struct Identity;
impl ModifierApply for Identity {
    fn apply(
        &self,
        _: &mut dyn ModifierRoll,
        generated: &[LootStack],
    ) -> Result<Vec<LootStack>, String> {
        Ok(generated.to_vec())
    }
}
fn file(id: &str, pack: &str, data: Value) -> ModifierFile {
    let bytes = serde_json::to_vec(&data).unwrap();
    ModifierFile {
        id: id.into(),
        pack: pack.into(),
        path: format!("data/test/loot_modifiers/{id}.json"),
        read: Box::new(move || Ok(bytes)),
    }
}
#[test]
fn winning_files_mask_lower_readers_and_fail_independently() {
    let mut files = Vec::new();
    for id in ["test:shadow", "test:broken", "test:unreadable"] {
        files.push(ModifierFile {
            id: id.into(),
            pack: "low".into(),
            path: "loser".into(),
            read: Box::new(|| panic!("a losing resource must never be read")),
        });
    }
    files.push(file(
        "test:shadow",
        "high",
        json!({"neoforge:conditions":[{"type":"neoforge:not","value":{"type":"neoforge:always"}}]}),
    ));
    files.push(ModifierFile {
        id: "test:broken".into(),
        pack: "high".into(),
        path: "broken.json".into(),
        read: Box::new(|| Ok(b"not JSON".to_vec())),
    });
    files.push(ModifierFile {
        id: "test:unreadable".into(),
        pack: "high".into(),
        path: "unreadable.json".into(),
        read: Box::new(|| Err("permission denied".into())),
    });
    files.push(file(
        "test:bad_condition",
        "high",
        json!({"type":"test:identity","neoforge:conditions":[{"type":"unknown:condition"}]}),
    ));
    files.push(file("test:missing_type", "high", json!({})));
    files.push(file(
        "test:decode_failure",
        "high",
        json!({"type":"test:unknown"}),
    ));
    files.push(file(
        "test:high",
        "high",
        json!({"type":"test:identity","priority":1002}),
    ));
    files.push(file(
        "test:default",
        "high",
        json!({"type":"test:identity"}),
    ));
    files.push(file(
        "test:low",
        "high",
        json!({"type":"test:identity","priority":900}),
    ));
    let conditions = LoadConditions::vanilla();
    let mut diagnostics = Vec::new();
    let generation = GlobalLootModifiers::load(
        files,
        &conditions,
        |_, kind, value| {
            if kind != "test:identity" {
                return Err("type: unknown serializer".into());
            }
            Ok(DecodedModifier {
                priority: value
                    .get("priority")
                    .map_or(1000, |value| value.as_i64().unwrap() as i32),
                condition: None,
                apply: Arc::new(Identity),
            })
        },
        |diagnostic| diagnostics.push(diagnostic),
    );
    assert_eq!(
        generation.ids().collect::<Vec<_>>(),
        ["test:high", "test:default", "test:low"]
    );
    assert_eq!(diagnostics.len(), 5);
    for diagnostic in diagnostics {
        assert_eq!(diagnostic.pack, "high");
        assert!(!diagnostic.path.is_empty());
        assert!(!diagnostic.reason.is_empty());
        assert_ne!(diagnostic.id, "test:shadow");
    }
    // One bad file's condition callback cannot poison the whole candidate.
    assert!(conditions.check_errors().is_ok());
}
