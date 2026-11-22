use super::*;

fn node(json: &str) -> Result<ItemModelNode> {
    ItemModelNode::decode(&serde_json::from_str(json).unwrap())
}
fn model_id(n: &ItemModelNode) -> &str {
    match n {
        ItemModelNode::Model(leaf) => &leaf.model,
        other => panic!("not a model: {other:?}"),
    }
}

#[test]
fn unknown_vanilla_names_still_fail() {
    assert!(node(r#"{"type":"minecraft:nope"}"#).is_err());
    assert!(node(r#"{"type":"nope","model":"a:b"}"#).is_err());
    let leaf = r#"{"type":"minecraft:model","model":"a:b"}"#;
    for json in [
        format!(
            r#"{{"type":"minecraft:condition","property":"minecraft:nope","on_true":{leaf},"on_false":{leaf}}}"#
        ),
        format!(
            r#"{{"type":"minecraft:select","property":"minecraft:nope","cases":[],"fallback":{leaf}}}"#
        ),
        format!(
            r#"{{"type":"minecraft:range_dispatch","property":"minecraft:nope","entries":[],"fallback":{leaf}}}"#
        ),
    ] {
        assert!(node(&json).is_err(), "{json}");
    }
    assert!(
        node(r#"{"type":"minecraft:special","base":"a:b","model":{"type":"minecraft:nope"}}"#)
            .is_err()
    );
    assert!(ClientItem::from_json(r#"{"model":{"type":"minecraft:nope"}}"#).is_err());
}

#[test]
fn condition_property_defaults_to_false_at_draw_time() {
    let definition = ClientItem::from_json(
        r#"{"model":{"type":"minecraft:condition","property":"mod:flag",
            "on_true":{"type":"minecraft:model","model":"a:yes"},
            "on_false":{"type":"minecraft:model","model":"a:no"}}}"#,
    )
    .unwrap();
    let mut models = ItemModels::default();
    models.definitions.insert("mod:tool".into(), definition);
    Arc::make_mut(&mut models.prototypes).insert(
        "mod:tool".into(),
        BTreeMap::from([(
            "minecraft:item_model".into(),
            Value::String("mod:tool".into()),
        )]),
    );
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    let resolved = models.resolve(&StackView::new("mod:tool", 1), &context, &NoEnv);
    assert_eq!(resolved.layers.len(), 1);
    assert_eq!(
        resolved.layers[0].content,
        LayerContent::Model {
            model: "a:no".into()
        }
    );
}

#[test]
fn select_property_takes_the_fallback_or_nothing() {
    let n = node(
        r#"{"type":"minecraft:select","property":"mod:level",
            "cases":[{"when":"low","model":{"type":"minecraft:model","model":"a:low"}}],
            "fallback":{"type":"minecraft:model","model":"a:other"}}"#,
    )
    .unwrap();
    assert_eq!(model_id(&n), "a:other");
    let n = node(r#"{"type":"minecraft:select","property":"mod:level","cases":[]}"#).unwrap();
    assert!(matches!(n, ItemModelNode::Empty));
}

#[test]
fn range_property_reads_zero() {
    let entry = |t: f32, m: &str| {
        format!(r#"{{"threshold":{t},"model":{{"type":"minecraft:model","model":"{m}"}}}}"#)
    };
    let fallback = r#","fallback":{"type":"minecraft:model","model":"a:fb"}"#;
    let make = |entries: &[String], fallback: &str| {
        node(&format!(
            r#"{{"type":"minecraft:range_dispatch","property":"mod:fill","entries":[{}]{fallback}}}"#,
            entries.join(",")
        ))
        .unwrap()
    };
    let n = make(&[entry(0.2, "a:one"), entry(0.4, "a:two")], fallback);
    assert_eq!(model_id(&n), "a:fb");
    let n = make(
        &[
            entry(0.0, "a:zero"),
            entry(-1.0, "a:neg"),
            entry(0.5, "a:half"),
        ],
        fallback,
    );
    assert_eq!(model_id(&n), "a:zero");
    let n = make(&[entry(0.2, "a:one")], "");
    assert!(matches!(n, ItemModelNode::Empty));
}

/// A special model type no mod registered is an error naming it, so the item definition holding
/// it isn't loaded and the item draws the missing model; its `base` no longer stands in. A type a
/// mod registered, vanilla's included, decodes with its model object for the mod's renderer, and
/// vanilla's types no mod registered keep theirs.
#[test]
fn a_special_type_draws_by_its_renderer_or_fails_by_name() {
    let error = node(
        r#"{"type":"minecraft:special","base":"mod:item/cube","model":{"type":"mod:energy_cube"}}"#,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("unknown special model type mod:energy_cube"),
        "{error}"
    );
    // A mod may replace a vanilla type: its models then go to the mod's renderer.
    let skillet = RegisteredSpecialModel(Arc::new(2));
    let bell = RegisteredSpecialModel(Arc::new(()));
    let empty_path = RegisteredSpecialModel(Arc::new(3));
    let load =
        |kind: &sunrise_core::identifier::Identifier, base: &str, json: &Map<String, Value>| {
            Ok(match kind.to_string().as_str() {
                "foreigntest:skillet" => {
                    assert_eq!(base, "minecraft:item/pan");
                    assert_eq!(json["handle"], serde_json::json!(2));
                    Some(skillet.clone())
                }
                "minecraft:bell" => Some(bell.clone()),
                "example:" => Some(empty_path.clone()),
                _ => None,
            })
        };
    let decoders = ItemModelDecoders {
        special: Some(&load),
        ..Default::default()
    };
    let decode = |json| {
        ClientItem::from_json_with_decoders(json, &decoders)
            .unwrap()
            .model
    };
    let pan = decode(
        r#"{"model":{"type":"special","base":"item/pan","model":{"type":"foreigntest:skillet","handle":2}}}"#,
    );
    let mut dependencies = Vec::new();
    pan.dependencies(&mut dependencies);
    assert_eq!(dependencies, ["minecraft:item/pan"]);
    assert!(matches!(pan, ItemModelNode::RegisteredSpecial { model, .. } if model == skillet));
    let shield = decode(r#"{"model":{"type":"special","base":"a:b","model":{"type":"shield"}}}"#);
    assert!(matches!(shield, ItemModelNode::Special { .. }));
    let overridden = decode(r#"{"model":{"type":"special","base":"a:b","model":{"type":"bell"}}}"#);
    assert!(matches!(overridden, ItemModelNode::RegisteredSpecial { model, .. } if model == bell));
    let empty = decode(r#"{"model":{"type":"special","base":"a:b","model":{"type":"example:"}}}"#);
    assert!(matches!(empty, ItemModelNode::RegisteredSpecial { model, .. } if model == empty_path));
}

#[test]
fn a_registered_vanilla_override_error_rejects_only_its_enclosing_item() {
    let load =
        |kind: &sunrise_core::identifier::Identifier, _: &str, _: &Map<String, Value>| match kind
            .to_string()
            .as_str()
        {
            "minecraft:bell" => Err("the bell override rejected its model".into()),
            _ => Ok(None),
        };
    let composite = r#"{"model":{"type":"composite","models":[
        {"type":"model","model":"a:plain"},
        {"type":"select","property":"display_context","cases":[{"when":"gui","model":
            {"type":"special","base":"a:bell","model":{"type":"bell"}}
        }]}
    ]}}"#;
    let decoders = ItemModelDecoders {
        special: Some(&load),
        ..Default::default()
    };
    // The same tree decodes with no registrations, so a builtin fallback would hide the error.
    assert!(ClientItem::from_json(composite).is_ok());
    assert_eq!(
        ClientItem::from_json_with_decoders(composite, &decoders)
            .unwrap_err()
            .to_string(),
        "the bell override rejected its model"
    );
    let other = ClientItem::from_json_with_decoders(
        r#"{"model":{"type":"special","base":"a:shield","model":{"type":"shield"}}}"#,
        &decoders,
    )
    .unwrap();
    assert!(matches!(
        other.model,
        ItemModelNode::Special {
            model: SpecialModel {
                kind: SpecialKind::Shield
            },
            ..
        }
    ));
}

/// A node type no mod registered is an error naming it, so the item definition holding it isn't
/// loaded and the item draws the missing model, whatever keys the node has. NeoForge's own
/// types are no exception, except `neoforge:fluid_container`, which the engine decodes (#2606).
#[test]
fn an_unregistered_node_type_fails_by_name() {
    for json in [
        r#"{"type":"mod:thing","model":"mod:item/thing"}"#,
        r#"{"type":"neoforge:trimmed_armor","base_model":{"type":"minecraft:model","model":"mod:item/boots"}}"#,
        r#"{"type":"mod:facade"}"#,
    ] {
        let kind = serde_json::from_str::<Value>(json).unwrap()["type"]
            .as_str()
            .unwrap()
            .to_owned();
        let error = node(json).unwrap_err().to_string();
        assert!(
            error.contains(&format!("unknown item model type {kind}")),
            "{error}"
        );
    }
    assert!(ClientItem::from_json(r#"{"model":{"type":"ae2:facade"}}"#).is_err());
}

/// Ownership comes from the decoder, not from the minecraft namespace or registrations.
#[test]
fn special_model_builtin_ownership_uses_the_decoder_identity() {
    assert!(SpecialModel::is_builtin_type("minecraft:bell"));
    assert!(SpecialModel::is_builtin_type("minecraft:chest"));
    assert!(!SpecialModel::is_builtin_type("minecraft:foo"));
    assert!(!SpecialModel::is_builtin_type("example:bell"));
    assert!(SpecialModel::decode(&serde_json::json!({"type": "minecraft:bell"})).is_ok());
    assert!(SpecialModel::decode(&serde_json::json!({"type": "minecraft:foo"})).is_err());
}

#[test]
fn registered_tint_model_range_and_select_decoders_override_builtins() {
    let custom_tint = RegisteredTintSource(Arc::new("tint"));
    let custom_model = RegisteredItemModel(Arc::new("model"));
    let custom_fluid = RegisteredItemModel(Arc::new("fluid"));
    let custom_range = RegisteredRangeProperty(Arc::new("range"));
    let custom_select = RegisteredSelectProperty::new(
        Arc::new("select"),
        |value| {
            value
                .as_str()
                .map(|s| RegisteredSelectKey::new(s.to_owned()))
                .ok_or_else(|| SelectCaseDecodeError::Mismatch("expected string".to_owned()))
        },
        |a, b| Ok(a.downcast_ref::<String>() == b.downcast_ref::<String>()),
    );

    let load_tint = |kind: &str, _: &Map<String, Value>| -> std::result::Result<_, String> {
        Ok((kind == "minecraft:constant").then(|| custom_tint.clone()))
    };
    let load_model = |kind: &str,
                      _: &Map<String, Value>,
                      _: &dyn Fn(&Value) -> Result<ItemModelNode>|
     -> std::result::Result<_, String> {
        match kind {
            "minecraft:model" => Ok(Some(DecodedItemModel {
                transformation: None,
                model: custom_model.clone(),
                models: Vec::new(),
            })),
            "neoforge:fluid_container" => Ok(Some(DecodedItemModel {
                transformation: None,
                model: custom_fluid.clone(),
                models: Vec::new(),
            })),
            _ => Ok(None),
        }
    };
    let load_range = |kind: &str, _: &Map<String, Value>| -> std::result::Result<_, String> {
        Ok((kind == "minecraft:damage").then(|| custom_range.clone()))
    };
    let load_select = |kind: &str, _: &Map<String, Value>| -> std::result::Result<_, String> {
        Ok((kind == "minecraft:main_hand").then(|| custom_select.clone()))
    };

    // Tint override takes effect ahead of `minecraft:constant`, while `minecraft:dye` stays built-in.
    let tint_decoders = ItemModelDecoders {
        tint: Some(&load_tint),
        ..Default::default()
    };
    let tinted = ClientItem::from_json_with_decoders(
        r#"{"model":{"type":"model","model":"a:b","tints":[{"type":"constant"},{"type":"dye","default":1}]}}"#,
        &tint_decoders,
    )
    .unwrap();
    let ItemModelNode::Model(leaf) = tinted.model else {
        panic!("expected model leaf");
    };
    assert_eq!(
        leaf.tints,
        vec![TintSource::Registered(custom_tint), TintSource::Dye(1)]
    );

    // Item model override takes effect ahead of both vanilla arms and `neoforge:fluid_container`.
    let model_decoders = ItemModelDecoders {
        item_model: Some(&load_model),
        ..Default::default()
    };
    let overridden_model = ClientItem::from_json_with_decoders(
        r#"{"model":{"type":"model","model":"a:b"}}"#,
        &model_decoders,
    )
    .unwrap();
    assert!(matches!(
        overridden_model.model,
        ItemModelNode::Registered { model, .. } if model == custom_model
    ));
    let overridden_fluid = ClientItem::from_json_with_decoders(
        r#"{"model":{"type":"neoforge:fluid_container","fluid":"minecraft:water"}}"#,
        &model_decoders,
    )
    .unwrap();
    assert!(matches!(
        overridden_fluid.model,
        ItemModelNode::Registered { model, .. } if model == custom_fluid
    ));
    let plain_empty = ClientItem::from_json_with_decoders(
        r#"{"model":{"type":"minecraft:empty"}}"#,
        &model_decoders,
    )
    .unwrap();
    assert!(matches!(plain_empty.model, ItemModelNode::Empty));

    // Range property override takes effect ahead of `minecraft:damage`, while `minecraft:count` stays built-in.
    let range_decoders = ItemModelDecoders {
        range_property: Some(&load_range),
        ..Default::default()
    };
    let overridden_range = ClientItem::from_json_with_decoders(
        r#"{"model":{"type":"range_dispatch","property":"damage","entries":[{"threshold":0.5,"model":{"type":"empty"}}]}}"#,
        &range_decoders,
    )
    .unwrap();
    assert!(matches!(
        overridden_range.model,
        ItemModelNode::RangeDispatch {
            property: NumericProperty::Registered(prop),
            ..
        } if prop == custom_range
    ));
    let builtin_range = ClientItem::from_json_with_decoders(
        r#"{"model":{"type":"range_dispatch","property":"count","entries":[{"threshold":0.5,"model":{"type":"empty"}}]}}"#,
        &range_decoders,
    )
    .unwrap();
    assert!(matches!(
        builtin_range.model,
        ItemModelNode::RangeDispatch {
            property: NumericProperty::Count { normalize: true },
            ..
        }
    ));

    // Select property override takes effect ahead of `minecraft:main_hand`, while `minecraft:charge_type` stays built-in.
    let select_decoders = ItemModelDecoders {
        select_property: Some(&load_select),
        ..Default::default()
    };
    let overridden_select = ClientItem::from_json_with_decoders(
        r#"{"model":{"type":"select","property":"main_hand","cases":[{"when":"custom_arm","model":{"type":"empty"}}]}}"#,
        &select_decoders,
    )
    .unwrap();
    assert!(matches!(
        overridden_select.model,
        ItemModelNode::Select {
            property: SelectProperty::Registered(prop),
            ..
        } if prop == custom_select
    ));
    let builtin_select = ClientItem::from_json_with_decoders(
        r#"{"model":{"type":"select","property":"charge_type","cases":[{"when":"arrow","model":{"type":"empty"}}]}}"#,
        &select_decoders,
    )
    .unwrap();
    assert!(matches!(
        builtin_select.model,
        ItemModelNode::Select {
            property: SelectProperty::ChargeType,
            ..
        }
    ));
}

#[test]
fn registered_override_errors_reject_enclosing_item_without_builtin_fallback() {
    let fail_tint = |kind: &str, _: &Map<String, Value>| -> std::result::Result<_, String> {
        match kind {
            "minecraft:constant" => Err("tint override rejected".into()),
            _ => Ok(None),
        }
    };
    let tint_json =
        r#"{"model":{"type":"model","model":"a:b","tints":[{"type":"constant","value":255}]}}"#;
    assert!(ClientItem::from_json(tint_json).is_ok());
    assert_eq!(
        ClientItem::from_json_with_decoders(
            tint_json,
            &ItemModelDecoders {
                tint: Some(&fail_tint),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string(),
        "tint override rejected"
    );

    let fail_model = |kind: &str,
                      _: &Map<String, Value>,
                      _: &dyn Fn(&Value) -> Result<ItemModelNode>|
     -> std::result::Result<_, String> {
        match kind {
            "minecraft:model" => Err("model override rejected".into()),
            "neoforge:fluid_container" => Err("fluid container override rejected".into()),
            _ => Ok(None),
        }
    };
    let model_decoders = ItemModelDecoders {
        item_model: Some(&fail_model),
        ..Default::default()
    };
    let model_json = r#"{"model":{"type":"model","model":"a:b"}}"#;
    assert!(ClientItem::from_json(model_json).is_ok());
    assert_eq!(
        ClientItem::from_json_with_decoders(model_json, &model_decoders)
            .unwrap_err()
            .to_string(),
        "model override rejected"
    );
    let fluid_json = r#"{"model":{"type":"neoforge:fluid_container","fluid":"minecraft:water","textures":{"base":"a:base"}}}"#;
    assert!(ClientItem::from_json(fluid_json).is_ok());
    assert_eq!(
        ClientItem::from_json_with_decoders(fluid_json, &model_decoders)
            .unwrap_err()
            .to_string(),
        "fluid container override rejected"
    );

    let fail_range = |kind: &str, _: &Map<String, Value>| -> std::result::Result<_, String> {
        match kind {
            "minecraft:damage" => Err("range override rejected".into()),
            _ => Ok(None),
        }
    };
    let range_json = r#"{"model":{"type":"range_dispatch","property":"damage","entries":[{"threshold":0.5,"model":{"type":"empty"}}]}}"#;
    assert!(ClientItem::from_json(range_json).is_ok());
    assert_eq!(
        ClientItem::from_json_with_decoders(
            range_json,
            &ItemModelDecoders {
                range_property: Some(&fail_range),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string(),
        "range override rejected"
    );

    let fail_select = |kind: &str, _: &Map<String, Value>| -> std::result::Result<_, String> {
        match kind {
            "minecraft:main_hand" => Err("select override rejected".into()),
            _ => Ok(None),
        }
    };
    let select_json = r#"{"model":{"type":"select","property":"main_hand","cases":[{"when":"left","model":{"type":"empty"}}]}}"#;
    assert!(ClientItem::from_json(select_json).is_ok());
    assert_eq!(
        ClientItem::from_json_with_decoders(
            select_json,
            &ItemModelDecoders {
                select_property: Some(&fail_select),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string(),
        "select override rejected"
    );
}
