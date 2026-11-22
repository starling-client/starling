use super::*;
use serde_json::json;

#[test]
fn identifier_adds_default_namespace() {
    assert_eq!(identifier("stone").unwrap(), "minecraft:stone");
    assert_eq!(identifier("a:b").unwrap(), "a:b");
}

#[test]
fn positive_modulo_wraps_negatives() {
    assert!((positive_modulo(-0.25, 1.0) - 0.75).abs() < 1e-6);
    assert!((positive_modulo(1.5, 1.0) - 0.5).abs() < 1e-6);
}

#[test]
fn display_context_names_round_trip() {
    for name in ["gui", "ground", "fixed", "head"] {
        assert_eq!(DisplayContext::by_name(name).unwrap().name(), name);
    }
}

#[test]
fn custom_condition_is_evaluated_per_stack_at_branch_selection() {
    struct Env;
    impl ItemEnv for Env {
        fn item_model_property(&self, property: &str, stack: &StackView) -> bool {
            assert_eq!(property, "example:charged");
            stack.count > 1 && stack.patch.get("example:charged") == Some(&Some(Value::Bool(true)))
        }
    }
    let definition = ClientItem::from_json(
        r#"{"model":{
        "type":"minecraft:condition", "property":"example:charged",
        "on_true":{"type":"minecraft:model","model":"example:item/charged"},
        "on_false":{"type":"minecraft:model","model":"example:item/idle"}
    }}"#,
    )
    .unwrap();
    let mut models = ItemModels::default();
    models.definitions.insert("example:tool".into(), definition);
    Arc::make_mut(&mut models.prototypes).insert(
        "example:tool".into(),
        BTreeMap::from([(
            "minecraft:item_model".into(),
            Value::String("example:tool".into()),
        )]),
    );
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    let mut stack = StackView::new("example:tool", 1);
    let chosen = |stack: &StackView, env: &dyn ItemEnv| {
        let resolved = models.resolve(stack, &context, env);
        assert_eq!(resolved.layers.len(), 1);
        let LayerContent::Model { model } = &resolved.layers[0].content else {
            panic!("expected model leaf")
        };
        model.clone()
    };
    assert_eq!(chosen(&stack, &NoEnv), "example:item/idle");
    assert_eq!(chosen(&stack, &Env), "example:item/idle");
    stack
        .patch
        .insert("example:charged".into(), Some(Value::Bool(true)));
    assert_eq!(chosen(&stack, &Env), "example:item/idle");
    stack.count = 2;
    assert_eq!(chosen(&stack, &Env), "example:item/charged");
    assert_eq!(chosen(&stack, &NoEnv), "example:item/idle");
    stack.patch.insert("example:charged".into(), None);
    assert_eq!(chosen(&stack, &Env), "example:item/idle");
}

#[test]
fn a_layer_refusal_stops_inside_a_bundle_and_skips_its_later_siblings() {
    struct Env(std::cell::Cell<usize>);
    impl ItemEnv for Env {
        fn item_model_property(&self, _: &str, _: &StackView) -> bool {
            self.0.set(self.0.get() + 1);
            false
        }
    }
    let tail = r#"{"type":"condition","property":"test:late",
        "on_true":{"type":"empty"},"on_false":{"type":"empty"}}"#;
    let mut models = ItemModels::default();
    for (item, first) in [
        ("test:bundle", r#"{"type":"bundle/selected_item"}"#),
        (
            "test:selected",
            r#"{"type":"model","model":"test:item/first"}"#,
        ),
    ] {
        let definition = format!(r#"{{"model":{{"type":"composite","models":[{first},{tail}]}}}}"#);
        models
            .definitions
            .insert(item.into(), ClientItem::from_json(&definition).unwrap());
    }
    let mut stack = StackView::new("test:bundle", 1);
    stack.bundle_selected = 0;
    stack
        .patch
        .insert("minecraft:item_model".into(), Some(json!("test:bundle")));
    stack.patch.insert(
        "minecraft:bundle_contents".into(),
        Some(json!([{
            "id": "test:selected", "components": { "minecraft:item_model": "test:selected" }
        }])),
    );
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    let env = Env(std::cell::Cell::new(0));
    let mut visited = Vec::new();
    let outcome = models.visit_layers(
        &stack,
        &context,
        &env,
        LayerTransform::Model,
        &mut |layer, source| {
            assert_eq!(source.item, "test:selected");
            visited.push(layer.content);
            ControlFlow::Break(())
        },
    );
    assert_eq!(outcome, ControlFlow::Break(()));
    assert_eq!(
        visited,
        [LayerContent::Model {
            model: "test:item/first".into()
        }]
    );
    assert_eq!(env.0.get(), 0);
}

#[test]
fn custom_condition_validates_its_name_and_both_branches() {
    let node = |property: &str, on_true: Value| {
        ItemModelNode::decode(&serde_json::json!({
            "type": "minecraft:condition", "property": property,
            "on_true": on_true, "on_false": {"type": "minecraft:empty"}
        }))
    };
    let empty = serde_json::json!({"type": "minecraft:empty"});
    for name in [
        "minecraft:unknown",
        "unknown",
        "example:Bad",
        "bad name:charged",
    ] {
        assert!(node(name, empty.clone()).is_err(), "{name}");
    }
    assert!(node("example:charged", Value::Null).is_err());
    assert!(matches!(
        node("minecraft:damaged", empty.clone()).unwrap(),
        ItemModelNode::Condition {
            property: ConditionProperty::Damaged,
            ..
        }
    ));
    assert!(matches!(node("example:charged", empty).unwrap(),
        ItemModelNode::Condition { property: ConditionProperty::Custom(name), .. } if name == "example:charged"));
}

#[test]
fn a_mods_tint_source_is_untinted_and_an_unknown_vanilla_one_fails() {
    let tint = |json: &str| {
        TintSource::decode_with(
            &serde_json::from_str(json).unwrap(),
            &ItemModelDecoders::default(),
        )
    };
    assert_eq!(
        tint(r#"{"type":"ae2:storage_cell_state"}"#).unwrap(),
        TintSource::Constant(-1)
    );
    assert!(tint(r#"{"type":"minecraft:nonsense"}"#).is_err());
}

#[test]
fn registered_range_property_thresholds_on_the_mods_number_per_stack() {
    struct Env;
    impl ItemEnv for Env {
        fn item_model_range_property(
            &self,
            property: &RegisteredRangeProperty,
            stack: &StackView,
            _ctx: &Context,
        ) -> f32 {
            let divisor = *property
                .0
                .downcast_ref::<f32>()
                .expect("bound range divisor");
            stack.count as f32 / divisor
        }
    }
    let load_range = |kind: &str, fields: &Map<String, Value>| -> std::result::Result<_, String> {
        if kind != "rangetest:fill" {
            return Ok(None);
        }
        let divisor = fields
            .get("divisor")
            .and_then(Value::as_f64)
            .map_or(10.0, |v| v as f32);
        Ok(Some(RegisteredRangeProperty(Arc::new(divisor))))
    };
    let decoders = ItemModelDecoders {
        range_property: Some(&load_range),
        ..Default::default()
    };
    let json = r#"{"model":{
        "type":"minecraft:range_dispatch", "property":"rangetest:fill", "divisor":10.0, "scale":2.0,
        "entries":[
            {"threshold":1.0,"model":{"type":"minecraft:model","model":"rangetest:item/full"}},
            {"threshold":0.5,"model":{"type":"minecraft:model","model":"rangetest:item/half"}}
        ],
        "fallback":{"type":"minecraft:model","model":"rangetest:item/empty"}
    }}"#;
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    // A resource reload decodes the definition again; the property stays bound to the decoder.
    for _reload in 0..2 {
        let mut models = ItemModels::default();
        models.definitions.insert(
            "rangetest:cell".into(),
            ClientItem::from_json_with_decoders(json, &decoders).unwrap(),
        );
        Arc::make_mut(&mut models.prototypes).insert(
            "rangetest:cell".into(),
            BTreeMap::from([(
                "minecraft:item_model".into(),
                Value::String("rangetest:cell".into()),
            )]),
        );
        let chosen = |count: i32, env: &dyn ItemEnv| {
            let resolved = models.resolve(&StackView::new("rangetest:cell", count), &context, env);
            let LayerContent::Model { model } = &resolved.layers[0].content else {
                panic!("expected model leaf")
            };
            model.clone()
        };
        // Scaled by 2: 0.2 -> 0.4, 0.3 -> 0.6, 0.5 -> 1.0.
        assert_eq!(chosen(2, &Env), "rangetest:item/empty");
        assert_eq!(chosen(3, &Env), "rangetest:item/half");
        assert_eq!(chosen(5, &Env), "rangetest:item/full");
        assert_eq!(chosen(5, &NoEnv), "rangetest:item/empty");
    }
}

#[test]
fn registered_tint_source_colours_each_stack_from_its_entry() {
    struct Env;
    impl ItemEnv for Env {
        fn item_tint_source(&self, source: &RegisteredTintSource, stack: &StackView) -> i32 {
            let default = *source.0.downcast_ref::<i32>().expect("bound tint default");
            match stack.count {
                1 => default,
                _ => 0xFF11_2233_u32 as i32,
            }
        }
    }
    let load_tint = |kind: &str, fields: &Map<String, Value>| -> std::result::Result<_, String> {
        if kind != "tinttest:charge" {
            return Ok(None);
        }
        let Some(default) = fields.get("default").and_then(Value::as_i64) else {
            return Err("missing integer default".into());
        };
        Ok(Some(RegisteredTintSource(Arc::new(default as i32))))
    };
    let decoders = ItemModelDecoders {
        tint: Some(&load_tint),
        ..Default::default()
    };
    assert!(
        ClientItem::from_json_with_decoders(
            r#"{"model":{"type":"minecraft:model","model":"tinttest:item/grey","tints":[{"type":"tinttest:charge","bad":null}]}}"#,
            &decoders,
        )
        .is_err()
    );
    let json = r#"{"model":{
        "type":"minecraft:model", "model":"tinttest:item/grey",
        "tints":[
            {"type":"tinttest:charge","default":-5614336},
            {"type":"tinttest:unregistered"},
            {"type":"minecraft:constant","value":255}
        ]
    }}"#;
    let mut models = ItemModels::default();
    models.definitions.insert(
        "tinttest:grey".into(),
        ClientItem::from_json_with_decoders(json, &decoders).unwrap(),
    );
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    let tints = |count: i32, env: &dyn ItemEnv| {
        let mut stack = StackView::new("tinttest:grey", count);
        stack.patch.insert(
            "minecraft:item_model".into(),
            Some(serde_json::json!("tinttest:grey")),
        );
        models.resolve(&stack, &context, env).layers[0]
            .tints
            .clone()
    };
    let blue = 0xFF00_00FF_u32 as i32;
    assert_eq!(tints(1, &Env), [-5614336, -1, blue]);
    assert_eq!(tints(2, &Env), [0xFF11_2233_u32 as i32, -1, blue]);
    // With no mod answering, the entry draws white.
    assert_eq!(tints(1, &NoEnv), [-1, -1, blue]);
}

#[test]
fn registered_select_property_picks_the_case_the_mod_answers_per_stack() {
    struct Env;
    impl ItemEnv for Env {
        fn item_model_select_property(
            &self,
            property: &RegisteredSelectProperty,
            stack: &StackView,
            _ctx: &Context,
            keys: &mut dyn Iterator<Item = (usize, &RegisteredSelectKey)>,
        ) -> Option<usize> {
            assert_eq!(property.payload.downcast_ref::<&str>(), Some(&"mode"));
            let chosen = match stack.count {
                1 => "dig",
                2 => "build",
                3 => "unlisted",
                _ => return None,
            };
            for (i, key) in keys {
                if key.downcast_ref::<String>().unwrap() == chosen {
                    return Some(i);
                }
            }
            None
        }
    }
    let load_select = |kind: &str,
                       _fields: &Map<String, Value>|
     -> std::result::Result<_, String> {
        if kind != "selecttest:mode" {
            return Ok(None);
        }
        Ok(Some(RegisteredSelectProperty::new(
            Arc::new("mode"),
            |value| {
                value
                    .as_str()
                    .map(|s| RegisteredSelectKey::new(s.to_owned()))
                    .ok_or_else(|| SelectCaseDecodeError::Mismatch("expected string".to_owned()))
            },
            |a, b| Ok(a.downcast_ref::<String>() == b.downcast_ref::<String>()),
        )))
    };
    let decoders = ItemModelDecoders {
        select_property: Some(&load_select),
        ..Default::default()
    };
    // A when value of a mod's property is a string; another value is refused, as vanilla's codec does.
    assert!(
        ItemModelNode::decode_with(
            &serde_json::json!({
                "type": "minecraft:select", "property": "selecttest:mode",
                "cases": [{"when": 1, "model": {"type": "minecraft:empty"}}]
            }),
            &decoders,
        )
        .is_err()
    );
    let json = r#"{"model":{
        "type":"minecraft:select", "property":"selecttest:mode",
        "cases":[
            {"when":"dig","model":{"type":"minecraft:model","model":"selecttest:item/dig"}},
            {"when":["build","place"],"model":{"type":"minecraft:model","model":"selecttest:item/build"}}
        ],
        "fallback":{"type":"minecraft:model","model":"selecttest:item/idle"}
    }}"#;
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    // A resource reload decodes the definition again; the property stays bound to the decoder.
    for _reload in 0..2 {
        let mut models = ItemModels::default();
        models.definitions.insert(
            "selecttest:tool".into(),
            ClientItem::from_json_with_decoders(json, &decoders).unwrap(),
        );
        Arc::make_mut(&mut models.prototypes).insert(
            "selecttest:tool".into(),
            BTreeMap::from([(
                "minecraft:item_model".into(),
                Value::String("selecttest:tool".into()),
            )]),
        );
        let chosen = |count: i32, env: &dyn ItemEnv| {
            let resolved = models.resolve(&StackView::new("selecttest:tool", count), &context, env);
            let LayerContent::Model { model } = &resolved.layers[0].content else {
                panic!("expected model leaf")
            };
            model.clone()
        };
        assert_eq!(chosen(1, &Env), "selecttest:item/dig");
        assert_eq!(chosen(2, &Env), "selecttest:item/build");
        assert_eq!(chosen(3, &Env), "selecttest:item/idle");
        assert_eq!(chosen(4, &Env), "selecttest:item/idle");
        assert_eq!(chosen(1, &NoEnv), "selecttest:item/idle");
    }
}

#[test]
fn a_patched_stack_reads_its_patch_over_its_items_defaults() {
    let mut models = ItemModels::default();
    Arc::make_mut(&mut models.prototypes).insert(
        "minecraft:diamond_sword".into(),
        Components::from_iter([
            (
                "minecraft:item_model".into(),
                json!("minecraft:diamond_sword"),
            ),
            ("minecraft:max_damage".into(), json!(1561)),
            ("minecraft:rarity".into(), json!("common")),
        ]),
    );
    let mut stack = StackView::new("minecraft:diamond_sword", 1);
    stack.patch.insert(
        "minecraft:enchantments".into(),
        Some(json!({"minecraft:sharpness": 5})),
    );
    stack
        .patch
        .insert("minecraft:rarity".into(), Some(json!("rare")));
    stack.patch.insert("minecraft:max_damage".into(), None);
    let view = models.components_view(&stack);
    assert_eq!(view.get("minecraft:rarity"), Some(&json!("rare")));
    assert_eq!(view.get("minecraft:max_damage"), None, "a removed default");
    assert_eq!(
        view.get("minecraft:item_model"),
        Some(&json!("minecraft:diamond_sword"))
    );
    assert!(view.contains_key("minecraft:enchantments"));
    assert!(!view.contains_key("minecraft:custom_name"));
    // Every key reads as the merged map has it.
    let merged = models.components(&stack);
    for key in [
        "minecraft:rarity",
        "minecraft:max_damage",
        "minecraft:item_model",
        "minecraft:enchantments",
        "minecraft:custom_name",
    ] {
        assert_eq!(view.get(key), merged.get(key), "{key}");
    }
    assert_eq!(view.to_map(), *merged);
}

/// A node of a registered type draws, per stack, the named `models` its bound provider answers,
/// in order, under the node's transformation; a name the node doesn't have draws nothing, and
/// with no mod answering the node draws nothing.
#[test]
fn registered_node_draws_the_models_the_mod_answers_per_stack() {
    fn choice(name: &str) -> ModelChoice {
        ModelChoice {
            model: name.to_owned(),
            transform: LayerTransform::Model,
        }
    }
    struct Env;
    impl ItemEnv for Env {
        fn item_model_update(
            &self,
            model: &RegisteredItemModel,
            stack: &StackView,
            ctx: &Context,
            models: &mut Vec<ModelChoice>,
        ) {
            let glow = *model.0.downcast_ref::<bool>().expect("bound glow flag");
            assert!(glow);
            if ctx.display == DisplayContext::Ground {
                models.push(choice("full"));
                return;
            }
            let names: &[&str] = match stack.count {
                1 => &["empty"],
                2 => &["full", "empty"],
                _ => &["absent"],
            };
            models.extend(names.iter().map(|name| choice(name)));
        }
    }
    let load_model = |kind: &str,
                      fields: &Map<String, Value>,
                      decode_child: &dyn Fn(&Value) -> Result<ItemModelNode>|
     -> std::result::Result<_, String> {
        if kind != "nodetest:charge" {
            return Ok(None);
        }
        let glow = fields.get("glow").and_then(Value::as_bool).unwrap_or(false);
        let models = match fields.get("models") {
            Some(Value::Object(entries)) => entries
                .iter()
                .map(|(name, node)| {
                    Ok((name.clone(), decode_child(node).map_err(|e| e.to_string())?))
                })
                .collect::<std::result::Result<Vec<_>, String>>()?,
            _ => Vec::new(),
        };
        Ok(Some(DecodedItemModel {
            transformation: optional_transformation(fields).map_err(|err| err.to_string())?,
            model: RegisteredItemModel(Arc::new(glow)),
            models,
        }))
    };
    let decoders = ItemModelDecoders {
        item_model: Some(&load_model),
        ..Default::default()
    };
    let json = r#"{"model":{"type":"nodetest:charge","glow":true,
        "transformation":{"translation":[1,0,0],"left_rotation":[0,0,0,1],"scale":[1,1,1],"right_rotation":[0,0,0,1]},
        "models":{
            "empty":{"type":"minecraft:model","model":"nodetest:item/empty"},
            "full":{"type":"minecraft:model","model":"nodetest:item/full"}
        }}}"#;
    let definition = ClientItem::from_json_with_decoders(json, &decoders).unwrap();
    let mut dependencies = Vec::new();
    definition.model.dependencies(&mut dependencies);
    dependencies.sort();
    assert_eq!(dependencies, ["nodetest:item/empty", "nodetest:item/full"]);
    let mut models = ItemModels::default();
    models
        .definitions
        .insert("nodetest:cell".into(), definition);
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    let drawn = |count: i32, env: &dyn ItemEnv| {
        let mut stack = StackView::new("nodetest:cell", count);
        stack.patch.insert(
            "minecraft:item_model".into(),
            Some(serde_json::json!("nodetest:cell")),
        );
        models
            .resolve(&stack, &context, env)
            .layers
            .iter()
            .map(|layer| match &layer.content {
                LayerContent::Model { model } => (model.clone(), layer.transform.0[3][0]),
                other => panic!("not a model layer: {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(drawn(1, &Env), [("nodetest:item/empty".to_owned(), 1.0)]);
    assert_eq!(
        drawn(2, &Env),
        [
            ("nodetest:item/full".to_owned(), 1.0),
            ("nodetest:item/empty".to_owned(), 1.0)
        ]
    );
    assert!(drawn(3, &Env).is_empty(), "a name the node lacks");
    // The surroundings reach the mod: on the ground the same stack draws `full`.
    let ground = Context {
        display: DisplayContext::Ground,
        ..context.clone()
    };
    let mut stack = StackView::new("nodetest:cell", 1);
    stack.patch.insert(
        "minecraft:item_model".into(),
        Some(serde_json::json!("nodetest:cell")),
    );
    let layers = models.resolve(&stack, &ground, &Env).layers;
    assert!(matches!(&layers[..], [layer] if matches!(&layer.content,
        LayerContent::Model { model } if model == "nodetest:item/full")));
    assert!(drawn(1, &NoEnv).is_empty(), "no mod answers");
}

/// A choice with no display transform marks every layer its model draws, however deep, and a
/// choice that keeps the model's transform leaves the layers' own marks alone.
#[test]
fn a_choice_without_a_display_transform_owns_the_layers_below_it() {
    struct Env;
    impl ItemEnv for Env {
        fn item_model_update(
            &self,
            _: &RegisteredItemModel,
            _: &StackView,
            _: &Context,
            layers: &mut Vec<ModelChoice>,
        ) {
            layers.push(ModelChoice {
                model: "bare".into(),
                transform: LayerTransform::None,
            });
            layers.push(ModelChoice {
                model: "plain".into(),
                transform: LayerTransform::Model,
            });
        }
    }
    let model = |json: serde_json::Value| ItemModelNode::decode(&json).unwrap();
    let definition = ClientItem {
        model: ItemModelNode::Registered {
            model: RegisteredItemModel(Arc::new(())),
            transformation: None,
            models: vec![
                (
                    "bare".into(),
                    model(serde_json::json!({"type":"minecraft:composite","models":[
                        {"type":"minecraft:model","model":"layertest:item/a"},
                        {"type":"minecraft:model","model":"layertest:item/b"}]})),
                ),
                (
                    "plain".into(),
                    model(serde_json::json!({"type":"minecraft:model","model":"layertest:item/c"})),
                ),
            ],
        },
        hand_animation_on_swap: true,
        oversized_in_gui: false,
        swap_animation_scale: 1.0,
    };
    let mut models = ItemModels::default();
    models
        .definitions
        .insert("layertest:wrap".into(), definition);
    let mut stack = StackView::new("layertest:wrap", 1);
    stack.patch.insert(
        "minecraft:item_model".into(),
        Some(serde_json::json!("layertest:wrap")),
    );
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    let marks: Vec<_> = models
        .resolve(&stack, &context, &Env)
        .layers
        .iter()
        .map(|layer| layer.display_transform)
        .collect();
    assert_eq!(
        marks,
        [
            LayerTransform::None,
            LayerTransform::None,
            LayerTransform::Model
        ]
    );
}

#[test]
fn a_head_model_names_a_skull_kind_only_once_a_mod_registers_it() {
    let head = |kind: &str| SpecialModel::decode(&json!({"type": "minecraft:head", "kind": kind}));
    // Before registration custom kinds are refused, as an unknown vanilla kind is.
    assert!(head("skulltest:goblin").is_err());
    assert!(head("minecraft:custom_skull").is_err());
    assert!(head("goblin").is_err());
    let kept = register_skull_kinds(vec![
        "skulltest:goblin".into(),
        "minecraft:custom_skull".into(),
        "minecraft:skeleton".into(),
        "minecraft:wither_skeleton".into(),
        "minecraft:player".into(),
        "minecraft:zombie".into(),
        "minecraft:creeper".into(),
        "minecraft:piglin".into(),
        "minecraft:dragon".into(),
        "zombie".into(),
        "skulltest:Bad".into(),
        "skulltest:goblin".into(),
        "minecraft:custom_skull".into(),
    ]);
    // Any valid non-vanilla canonical Identifier is kept (including `minecraft:` non-vanilla
    // paths); the 7 built-in vanilla skull types, invalid syntax, and duplicates are rejected.
    assert_eq!(kept, ["skulltest:goblin", "minecraft:custom_skull"]);
    assert!(
        register_skull_kinds(vec![
            "skulltest:goblin".into(),
            "minecraft:custom_skull".into(),
        ])
        .is_empty()
    );
    assert!(is_skull_kind("skulltest:goblin"));
    assert!(is_skull_kind("minecraft:custom_skull"));
    for vanilla in [
        "minecraft:skeleton",
        "minecraft:wither_skeleton",
        "minecraft:player",
        "minecraft:zombie",
        "minecraft:creeper",
        "minecraft:piglin",
        "minecraft:dragon",
    ] {
        assert!(!is_skull_kind(vanilla), "{vanilla}");
    }
    let goblin = head("skulltest:goblin").unwrap();
    assert!(matches!(&goblin.kind, SpecialKind::Head { kind, .. } if kind == "skulltest:goblin"));
    assert_eq!(goblin.to_json()["kind"], "skulltest:goblin");
    let custom_mc = head("minecraft:custom_skull").unwrap();
    assert!(
        matches!(&custom_mc.kind, SpecialKind::Head { kind, .. } if kind == "minecraft:custom_skull")
    );
    assert_eq!(custom_mc.to_json()["kind"], "minecraft:custom_skull");
    assert!(head("skulltest:orc").is_err());
    assert!(head("minecraft:orc").is_err());
    // Vanilla's kinds are unchanged.
    assert!(head("zombie").is_ok());
}

#[test]
fn last_index_less_or_equal_linear_and_binary() {
    use super::last_index_less_or_equal;
    let make = |thresholds: &[f32]| -> Vec<(f32, ItemModelNode)> {
        thresholds
            .iter()
            .map(|&t| (t, ItemModelNode::Empty))
            .collect()
    };

    // Empty
    assert_eq!(last_index_less_or_equal(&[], 0.0), None);

    // Short (< 16): linear search
    let short = make(&[-1.0, 0.0, 1.0, 2.5, 5.0]);
    assert_eq!(last_index_less_or_equal(&short, -2.0), None);
    assert_eq!(last_index_less_or_equal(&short, -1.0), Some(0));
    assert_eq!(last_index_less_or_equal(&short, -0.5), Some(0));
    assert_eq!(last_index_less_or_equal(&short, 0.0), Some(1));
    assert_eq!(last_index_less_or_equal(&short, 2.5), Some(3));
    assert_eq!(last_index_less_or_equal(&short, 3.0), Some(3));
    assert_eq!(last_index_less_or_equal(&short, 5.0), Some(4));
    assert_eq!(last_index_less_or_equal(&short, 10.0), Some(4));

    // Long (>= 16): binary search
    let thresholds: Vec<f32> = (0..32).map(|i| i as f32 * 10.0).collect();
    let long = make(&thresholds);
    assert_eq!(last_index_less_or_equal(&long, -5.0), None);
    assert_eq!(last_index_less_or_equal(&long, 0.0), Some(0));
    assert_eq!(last_index_less_or_equal(&long, 5.0), Some(0));
    assert_eq!(last_index_less_or_equal(&long, 10.0), Some(1));
    assert_eq!(last_index_less_or_equal(&long, 150.0), Some(15));
    assert_eq!(last_index_less_or_equal(&long, 155.0), Some(15));
    assert_eq!(last_index_less_or_equal(&long, 310.0), Some(31));
    assert_eq!(last_index_less_or_equal(&long, 1000.0), Some(31));

    // Short lists use upper-bound tie behavior, including signed zero.
    let duplicates = make(&[1.0, 1.0, 2.0]);
    assert_eq!(last_index_less_or_equal(&duplicates, 1.0), Some(1));

    let zeroes = make(&[-0.0, 0.0]);
    assert_eq!(last_index_less_or_equal(&zeroes, -0.0), Some(1));
    assert_eq!(last_index_less_or_equal(&zeroes, 0.0), Some(1));

    // At 16 entries, vanilla returns Arrays.binarySearch's hit, not the last tie.
    // Expected indices were checked on the JVM.
    let duplicates = make(&[
        0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 7.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0,
    ]);
    assert_eq!(last_index_less_or_equal(&duplicates, 7.0), Some(7));

    let zeroes = make(&[
        -8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0, -0.0, -0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0,
    ]);
    assert_eq!(last_index_less_or_equal(&zeroes, -0.0), Some(7));
    assert_eq!(last_index_less_or_equal(&zeroes, 0.0), Some(9));
}
