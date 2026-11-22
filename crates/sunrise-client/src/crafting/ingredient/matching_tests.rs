use super::*;
use std::cell::Cell;
use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
thread_local! { static CALLS: Cell<usize> = const { Cell::new(0) }; }

fn empty_ingredient_recipes_retain_predicates_but_not_placement() {
    use crate::crafting::{Recipes, ResolvedSources};
    use serde_json::json;
    use sunrise_world::loot::PackOverlay;
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let context = load.loop_context(LoopSide::Server, []);
    let mut items = Items::for_tests(Default::default());
    items
        .item_tag_order
        .insert("empty_fixture:empty".into(), vec![]);
    let inputs = [
        ("direct", json!([]), false, false),
        ("missing", json!("#empty_fixture:missing"), false, false),
        ("tag", json!("#empty_fixture:empty"), true, false),
        (
            "nested_tag",
            json!({"type":"neoforge:compound","children":["#empty_fixture:empty"]}),
            true,
            false,
        ),
        (
            "custom",
            json!({"type":"empty_fixture:predicate"}),
            true,
            true,
        ),
        (
            "nested",
            json!({"type":"neoforge:compound",
            "children":[{"type":"empty_fixture:predicate"}]}),
            true,
            true,
        ),
    ];
    let mut sources: Vec<_> = inputs
        .iter()
        .flat_map(|(id, ingredient, _, _)| {
            [
                (
                    format!("empty_fixture:{id}"),
                    json!({
                        "type":"minecraft:crafting_shaped", "pattern":["A"],
                        "key":{"A":ingredient}, "result":{"id":"minecraft:stick"}
                    }),
                ),
                (
                    format!("empty_fixture:{id}_cook"),
                    json!({
                        "type":"minecraft:smelting", "ingredient":ingredient,
                        "result":{"id":"minecraft:stick"}
                    }),
                ),
                (
                    format!("empty_fixture:{id}_cut"),
                    json!({
                        "type":"minecraft:stonecutting", "ingredient":ingredient,
                        "result":{"id":"minecraft:stick"}
                    }),
                ),
                (
                    format!("empty_fixture:{id}_smith"),
                    json!({
                        "type":"minecraft:smithing_transform",
                        "base":"minecraft:stone", "addition":ingredient,
                        "result":{"id":"minecraft:stick"}
                    }),
                ),
            ]
        })
        .collect();
    sources.push((
        "empty_fixture:visible_neighbor".into(),
        json!({
            "type":"minecraft:crafting_shaped","pattern":["A"],
            "key":{"A":"minecraft:stone"},"result":{"id":"minecraft:stick"}
        }),
    ));
    for (name, file) in sources.iter().filter(|(name, _)| {
        name.starts_with("empty_fixture:direct") || name.starts_with("empty_fixture:missing")
    }) {
        let error = Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &PackOverlay::NONE,
            vec![
                (name.clone(), file.clone()),
                sources.last().unwrap().clone(),
            ],
            &load,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(
            error.contains(name.strip_prefix("empty_fixture:").unwrap()),
            "{error}"
        );
        assert!(error.contains("invalid ingredient"), "{error}");
    }
    sources.retain(|(name, _)| {
        !name.starts_with("empty_fixture:direct") && !name.starts_with("empty_fixture:missing")
    });
    let resolved = ResolvedSources::resolve(sources, &items, &load).unwrap();
    let root = std::env::temp_dir();
    let recipes = Recipes::from_resolved(&root, &items, &PackOverlay::NONE, &resolved).unwrap();
    let stations = crate::stations::StationRecipes::from_sources(&root, &items, &resolved);
    let mut accepted = ItemStack::new("minecraft:stone", 1);
    accepted.damage = 7;
    let book = crate::recipe_book::Book::build(&recipes, &items);
    let index = crate::recipe_book::RecipeIndex::from_recipes(&recipes);
    let mut learned = crate::recipe_book::Settings::default();
    assert_eq!(book.entries.len(), 1);
    assert_eq!(book.entries[0].id, "empty_fixture:visible_neighbor");
    assert_eq!(
        index.first_display("empty_fixture:visible_neighbor"),
        Some(0)
    );
    for (id, _, retained, matches) in inputs {
        let id = format!("empty_fixture:{id}");
        let entry = recipes.by_type["minecraft:crafting"]
            .iter()
            .find(|r| r.id == id);
        assert_eq!(entry.is_some(), retained, "{id}");
        assert!(!index.contains(&id));
        assert!(book.entry(&id).is_none());
        assert_eq!(
            learned.add_recipes(&index, std::slice::from_ref(&id)),
            (0, vec![])
        );
        assert!(!learned.knows(&id));
        assert_eq!(
            recipes.recipe_matches(&context, &id, &[Some(accepted.clone())], 1),
            matches
        );
        assert!(!recipes.recipe_matches(
            &context,
            &id,
            &[Some(ItemStack::new("minecraft:stone", 1))],
            1
        ));
        assert!(recipes.placement(&id).is_none_or(|p| p.impossible()));
        assert!(
            recipes
                .placement(&format!("{id}_cook"))
                .is_none_or(|p| p.impossible())
        );
        for (kind, suffix) in [
            ("minecraft:smelting", "_cook"),
            ("minecraft:stonecutting", "_cut"),
            ("minecraft:smithing", "_smith"),
        ] {
            assert_eq!(
                recipes.by_type[kind]
                    .iter()
                    .any(|r| r.id == format!("{id}{suffix}")),
                retained
            );
        }
        if let Some(entry) = entry {
            assert!(entry.ingredients[0].items.is_empty());
            if matches {
                assert!(
                    entry.ingredients[0].custom.is_some()
                        || entry.ingredients[0].children.is_some()
                );
            } else {
                assert!(entry.ingredients[0].custom.is_none());
            }
        }
    }
    assert_eq!(
        recipes
            .cook(&context, "minecraft:smelting", &accepted)
            .unwrap()
            .id,
        "empty_fixture:custom_cook"
    );
    assert!(stations.stonecutting.is_empty());
    assert!(stations.stonecutter(&context, &accepted).is_empty());
    assert!(!stations.smithing_accepts(&context, 1, &accepted));
    assert!(!recipes.cooking_accepts(&context, "minecraft:smelting", &accepted));
}

#[test]
fn empty_ingredient_load_diagnostics_distinguish_errors_and_warnings() {
    crate::test_log::install();
    use crate::crafting::Recipes;
    use serde_json::json;
    use sunrise_world::loot::PackOverlay;
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let mut items = Items::for_tests(Default::default());
    items.item_tag_order.insert("fixture:empty".into(), vec![]);
    for (ingredient, expected) in [
        (json!([]), log::Level::Error),
        (json!("#fixture:missing"), log::Level::Error),
        (json!("#fixture:empty"), log::Level::Warn),
        (json!({"type":"fixture:predicate"}), log::Level::Error),
    ] {
        crate::test_log::clear();
        let recipes = Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &PackOverlay::NONE,
            vec![(
                "fixture:recipe".into(),
                json!({"type":"minecraft:crafting_shaped",
                "pattern":["A"], "key":{"A":ingredient}, "result":{"id":"minecraft:stick"}}),
            )],
            &load,
        );
        let levels = crate::test_log::records();
        if expected == log::Level::Error {
            let error = recipes.err().unwrap().to_string();
            assert!(error.contains("data/fixture/recipe/recipe.json"), "{error}");
            assert!(
                error.contains("invalid ingredient")
                    || error.contains("ingredient kind fixture:predicate"),
                "{error}"
            );
            // One Error record, the refusal naming the file.
            let errors: Vec<_> = levels
                .iter()
                .filter(|(level, _)| *level == log::Level::Error)
                .collect();
            assert_eq!(errors.len(), 1, "{levels:?}");
            assert!(
                errors[0].1.contains("data/fixture/recipe/recipe.json"),
                "{levels:?}"
            );
        } else {
            assert_eq!(recipes.unwrap().crafting.len(), 1);
            assert!(levels.iter().any(|(level, _)| *level == log::Level::Warn));
            assert!(!levels.iter().any(|(level, _)| *level == log::Level::Error));
        }
    }
}

struct ListenerIngredient(&'static str, &'static [&'static str]);
#[mod_api::__subscriptions]
impl mod_api::Hooks for ListenerIngredient {
    fn ingredient(
        &self,
        _: &dyn mod_api::DataLoadContext,
        event: &mut mod_api::IngredientEvent<'_>,
    ) -> Option<mod_api::IngredientDefinition> {
        let kind = event.kind;
        if kind != self.0 {
            return None;
        }
        Some(mod_api::IngredientDefinition::new(
            self.1.iter().map(|id| id.to_string()).collect(),
            |_, stack| {
                CALLS.set(CALLS.get() + 1);
                stack.item_id() == "minecraft:stone" && stack.damage() == Some(7)
            },
        ))
    }
}

#[test]
fn panicking_ingredient_predicate_disables_only_its_definition_across_recipe_loads() {
    use crate::crafting::Recipes;
    use serde_json::json;
    use sunrise_world::loot::PackOverlay;

    struct Predicates;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Predicates {
        fn ingredient(
            &self,
            _: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::IngredientEvent<'_>,
        ) -> Option<mod_api::IngredientDefinition> {
            if event.kind != "fixture:predicate" {
                return None;
            }
            let panics = matches!(
                event.data.get(&mod_api::JavaString::from("panics")),
                Some(mod_api::Tag::Byte(1))
            );
            Some(mod_api::IngredientDefinition::new(
                vec!["minecraft:stone".into()],
                move |_, stack| {
                    if panics && stack.count() == 3 {
                        CALLS.set(CALLS.get() + 1);
                        panic!("ingredient regression panic");
                    }
                    stack.count() == 2
                },
            ))
        }
    }

    crate::test_log::install();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "predicate_owner",
            Box::new(Predicates),
        )]
        .into(),
        || {
            let data = crate::test_resources::server_load();
            let context = data.loop_context(LoopSide::Server, []);
            let items = Items::for_tests(Default::default());
            let load = |panics| {
                Recipes::from_sources(
                    &std::env::temp_dir(),
                    &items,
                    &PackOverlay::NONE,
                    [("fixture:panicking", panics), ("fixture:healthy", true)]
                        .map(|(recipe, panics)| {
                            (
                                recipe.into(),
                                json!({"type":"minecraft:crafting_shapeless",
                                "ingredients":[{"type":"fixture:predicate", "panics":panics}],
                                "result":{"id":"minecraft:stick"}}),
                            )
                        })
                        .into(),
                    &data,
                )
                .unwrap()
            };
            let one = [Some(ItemStack::new("minecraft:stone", 1))];
            let two = [Some(ItemStack::new("minecraft:stone", 2))];
            let three = [Some(ItemStack::new("minecraft:stone", 3))];
            let recipes = load(true);
            crate::test_log::clear();
            CALLS.set(0);
            assert!(!recipes.recipe_matches(&context, "fixture:panicking", &three, 1));
            assert!(!recipes.recipe_matches(&context, "fixture:panicking", &two, 1));
            // The second recipe decoded exactly the same ingredient JSON into its own callback.
            assert!(recipes.recipe_matches(&context, "fixture:healthy", &two, 1));
            let recipes = load(true);
            assert!(!recipes.recipe_matches(&context, "fixture:panicking", &three, 1));
            assert!(!recipes.recipe_matches(&context, "fixture:panicking", &two, 1));
            assert!(!recipes.recipe_matches(&context, "fixture:healthy", &one, 1));
            assert!(recipes.recipe_matches(&context, "fixture:healthy", &two, 1));
            let recipes = load(false);
            assert!(recipes.recipe_matches(&context, "fixture:panicking", &two, 1));
            assert_eq!(CALLS.get(), 1);
            let logs = crate::test_log::records();
            assert_eq!(
                logs.iter()
                    .filter(|(level, text)| *level == log::Level::Error
                        && text.contains("predicate_owner")
                        && text.contains("ingredient regression panic"))
                    .count(),
                1
            );
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "predicate_owner",
                    hook: "ingredient_predicate",
                }]
            );
        },
    );
}

#[test]
fn unclaimed_custom_kind_refuses_load_but_an_empty_answer_only_warns() {
    crate::test_log::install();
    use crate::crafting::Recipes;
    use serde_json::json;
    use sunrise_world::loot::PackOverlay;
    let items = Items::for_tests(Default::default());
    let recipe = || {
        vec![(
            "fixture:needs_mod".to_string(),
            json!({"type":"minecraft:crafting_shapeless",
            "ingredients":[{"type":"absent:kind"}], "result":{"id":"minecraft:stick"}}),
        )]
    };
    let load = || {
        LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
            [],
        )
    };
    let run = |mods: Vec<mod_api::ModEntry>| {
        crate::test_log::clear();
        let mut loaded = None;
        mod_platform::testing::with_mods(mods.into(), || {
            loaded = Some(Recipes::from_sources(
                &std::env::temp_dir(),
                &items,
                &PackOverlay::NONE,
                recipe(),
                &load(),
            ));
        });
        let logs = crate::test_log::records();
        (loaded.unwrap(), logs)
    };
    // No listener: refuse the entire load, naming the file and kind once.
    let (recipes, logs) = run(vec![]);
    let error = recipes.err().unwrap().to_string();
    assert!(
        error.contains("data/fixture/recipe/needs_mod.json"),
        "{error}"
    );
    assert!(error.contains("ingredient kind absent:kind"), "{error}");
    // The refusal is logged once at Error, naming the file.
    assert_eq!(logs.len(), 1, "{logs:?}");
    assert_eq!(logs[0].0, log::Level::Error);
    assert!(
        logs[0].1.contains("data/fixture/recipe/needs_mod.json"),
        "{logs:?}"
    );
    // An unrelated subscriber does not claim this kind.
    let (recipes, logs) = run(vec![mod_api::ModEntry::new(
        "unrelated",
        Box::new(ListenerIngredient("other:kind", &[])),
    )]);
    let unrelated = recipes.err().unwrap().to_string();
    assert_eq!(unrelated, error);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].0, log::Level::Error);

    // A listener with an empty answer: kept, with a warning naming both.
    let (recipes, logs) = run(vec![mod_api::ModEntry::new(
        "empty",
        Box::new(ListenerIngredient("absent:kind", &[])),
    )]);
    assert_eq!(recipes.unwrap().crafting.len(), 1);
    assert!(logs.iter().any(|(level, text)| *level == log::Level::Warn
        && text.contains("fixture:needs_mod")
        && text.contains("absent:kind")));
    assert!(!logs.iter().any(|(level, _)| *level == log::Level::Error));
    // A listener with items: loads as before, silently.
    let (recipes, logs) = run(vec![mod_api::ModEntry::new(
        "items",
        Box::new(ListenerIngredient("absent:kind", &["minecraft:stick"])),
    )]);
    assert_eq!(recipes.unwrap().crafting.len(), 1);
    assert!(logs.is_empty());
}

fn cooking_indexes_empty_predicate_descendants_once_in_registration_order() {
    use crate::crafting::Recipes;
    use serde_json::json;
    use sunrise_world::loot::PackOverlay;
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let context = load.loop_context(LoopSide::Server, []);
    let items = Items::for_tests(Default::default());
    for (ingredient, damage, expected) in [
        (
            json!({"type":"neoforge:compound","children":[
            {"type":"empty_fixture:predicate"},"minecraft:dirt"]}),
            7,
            "fixture:a",
        ),
        (
            json!({"type":"neoforge:difference","base":"minecraft:stone",
            "subtracted":{"type":"empty_fixture:predicate"}}),
            7,
            "fixture:b",
        ),
    ] {
        let sources = vec![
            (
                "fixture:a".into(),
                json!({"type":"minecraft:smelting","ingredient":ingredient,
                "result":{"id":"minecraft:stick"}}),
            ),
            (
                "fixture:b".into(),
                json!({"type":"minecraft:smelting","ingredient":"minecraft:stone",
                "result":{"id":"minecraft:diamond"}}),
            ),
        ];
        let recipes = Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &PackOverlay::NONE,
            sources,
            &load,
        )
        .unwrap();
        let mut stack = ItemStack::new("minecraft:stone", 1);
        stack.damage = damage;
        CALLS.set(0);
        assert_eq!(
            recipes
                .cook(&context, "minecraft:smelting", &stack)
                .unwrap()
                .id,
            expected
        );
        assert_eq!(CALLS.get(), 1);
    }
}

// A mod listens to the `ingredient` hook and answers with no items.
#[test]
fn empty_ingredient_recipes_retain_predicates_but_not_placement_with_a_listener() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "empty",
            Box::new(ListenerIngredient("empty_fixture:predicate", &[])),
        )]
        .into(),
        empty_ingredient_recipes_retain_predicates_but_not_placement,
    );
}

// A mod listens to the `ingredient` hook and answers with no items.
#[test]
fn cooking_indexes_empty_predicate_descendants_once_in_registration_order_with_a_listener() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "empty",
            Box::new(ListenerIngredient("empty_fixture:predicate", &[])),
        )]
        .into(),
        cooking_indexes_empty_predicate_descendants_once_in_registration_order,
    );
}
