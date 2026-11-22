//! #1273: every entry of an ingredient list is an id; a directly named id is
//! never air, while a tag's members are taken as the tag holds them.
use super::*;
use serde_json::json;

fn walk(
    v: Value,
    tags: &std::collections::HashMap<String, Vec<String>>,
) -> Option<WalkedIngredient<()>> {
    WalkedIngredient::walk(&v, tags, &|_| None, &|_, _| None, &|_| None)
}

#[test]
fn a_list_entry_that_is_not_an_id_fails_the_walk() {
    let tags = Default::default();
    for v in [
        json!(["a:b", 5]),
        json!(["a:b", {}]),
        json!(["a:b", null]),
        json!(["a:b", ["c:d"]]),
    ] {
        assert!(walk(v.clone(), &tags).is_none(), "{v}");
    }
    let walked = walk(json!(["a:b", "c:d"]), &tags).expect("a list of ids walks");
    assert_eq!(walked.order, ["a:b", "c:d"]);
}

#[test]
fn a_directly_named_air_fails_the_walk_but_a_tag_holding_it_does_not() {
    let tags = std::collections::HashMap::from([(
        "fixture:with_air".to_owned(),
        vec!["a:b".to_owned(), "minecraft:air".to_owned()],
    )]);
    for v in [
        json!("minecraft:air"),
        json!("air"),
        json!(["a:b", "minecraft:air"]),
    ] {
        assert!(walk(v.clone(), &tags).is_none(), "{v}");
    }
    let walked = walk(json!("#fixture:with_air"), &tags).expect("the tag walks");
    assert_eq!(walked.order, ["a:b", "minecraft:air"]);
}
