//! The built-in condition map codecs, including flattened predicate payloads.
use super::{Kind::*, Row, opt, req, row};
pub(super) const ROWS: &[Row] = &[
    row("inverted", &[req("term", Condition)]),
    row("any_of", &[req("terms", Named("conditions"))]),
    row("all_of", &[req("terms", Named("conditions"))]),
    row("random_chance", &[req("chance", Named("float"))]),
    row(
        "random_chance_with_enchanted_bonus",
        &[
            req("unenchanted_chance", FloatRange(0.0, 1.0)),
            req("enchanted_chance", Named("level_value")),
            req("enchantment", Id),
        ],
    ),
    row(
        "entity_properties",
        &[
            opt("predicate", Named("entity_predicate")),
            req("entity", Named("entity_target")),
        ],
    ),
    row("killed_by_player", &[]),
    row(
        "entity_scores",
        &[
            req("scores", Map(&Named("int_range"))),
            req("entity", Named("entity_target")),
        ],
    ),
    Row {
        name: "match_block",
        kind: Named("block_predicate"),
    },
    row("match_tool", &[opt("predicate", Named("item_predicate"))]),
    row(
        "table_bonus",
        &[
            req("enchantment", Id),
            req("chances", List(&Number, 1, usize::MAX)),
        ],
    ),
    row("survives_explosion", &[]),
    row(
        "damage_source_properties",
        &[opt("predicate", Named("damage_predicate"))],
    ),
    row(
        "location_check",
        &[
            opt("predicate", Named("location_predicate")),
            opt("offsetX", Number),
            opt("offsetY", Number),
            opt("offsetZ", Number),
        ],
    ),
    row(
        "weather_check",
        &[opt("raining", Bool), opt("thundering", Bool)],
    ),
    row(
        "time_check",
        &[
            req("clock", Id),
            opt("period", Number),
            req("value", Named("int_range")),
        ],
    ),
    row(
        "int_value_check",
        &[req("value", Named("int")), req("test", Named("int_range"))],
    ),
    row(
        "float_value_check",
        &[
            req("value", Named("float")),
            req("test", Named("float_range")),
        ],
    ),
    row("enchantment_active_check", &[req("active", Bool)]),
    Row {
        name: "environment_attribute_check",
        kind: Named("environment_check"),
    },
];
