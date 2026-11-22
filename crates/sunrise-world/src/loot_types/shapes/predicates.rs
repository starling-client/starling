//! Shared records and scalar vocabularies referenced by loot codec rows.
use super::{Kind::*, Row, opt, req, row};
macro_rules! operation {
    ($name:literal, $max:expr) => {
        Row {
            name: $name,
            kind: Switch(
                "mode",
                false,
                &[
                    row("replace_all", &[]),
                    row("append", &[]),
                    row("insert", &[opt("offset", IntRange(0, i32::MAX as i64))]),
                    row(
                        "replace_section",
                        &[
                            opt("offset", IntRange(0, i32::MAX as i64)),
                            opt("size", IntRange(0, $max)),
                        ],
                    ),
                ],
            ),
        }
    };
}
macro_rules! edit {
    ($name:literal, $element:expr, $max:expr, $operation:literal) => {
        Row {
            name: $name,
            kind: All(&[
                Object(&[req("values", List(&$element, 0, $max))]),
                Named($operation),
            ]),
        }
    };
}
pub(super) const ROWS: &[Row] = &[
    Row {
        name: "entity_target",
        kind: Enum(&[
            "this",
            "attacker",
            "direct_attacker",
            "attacking_player",
            "target_entity",
            "interacting_entity",
        ]),
    },
    Row {
        name: "entity_or_block",
        kind: Enum(&[
            "this",
            "attacker",
            "direct_attacker",
            "attacking_player",
            "target_entity",
            "interacting_entity",
            "block_entity",
        ]),
    },
    Row {
        name: "component_source",
        kind: Enum(&[
            "this",
            "attacker",
            "direct_attacker",
            "attacking_player",
            "target_entity",
            "interacting_entity",
            "block_entity",
            "tool",
        ]),
    },
    Row {
        name: "ids",
        kind: Either(&[Id, Tag, List(&Id, 0, usize::MAX)]),
    },
    Row {
        name: "functions",
        kind: HolderSet(true),
    },
    Row {
        name: "conditions",
        kind: HolderSet(false),
    },
    Row {
        name: "slot_group",
        kind: Enum(&[
            "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor", "body",
            "saddle",
        ]),
    },
    Row {
        name: "attribute_operation",
        kind: Enum(&["add_value", "add_multiplied_base", "add_multiplied_total"]),
    },
    row(
        "attribute_modifier",
        &[
            req("id", Id),
            req("attribute", Id),
            req("operation", Named("attribute_operation")),
            req("amount", Named("float")),
            req(
                "slot",
                Either(&[
                    Named("slot_group"),
                    List(&Named("slot_group"), 1, usize::MAX),
                ]),
            ),
        ],
    ),
    Row {
        name: "container_component",
        kind: RegistryEnum(&["container", "bundle_contents", "charged_projectiles"]),
    },
    Row {
        name: "dye_color",
        kind: Enum(&[
            "white",
            "orange",
            "magenta",
            "light_blue",
            "yellow",
            "lime",
            "pink",
            "gray",
            "light_gray",
            "cyan",
            "purple",
            "blue",
            "brown",
            "green",
            "red",
            "black",
        ]),
    },
    Row {
        name: "firework_shape",
        kind: Enum(&["small_ball", "large_ball", "star", "creeper", "burst"]),
    },
    Row {
        name: "firework_colors",
        kind: List(&Number, 0, usize::MAX),
    },
    row(
        "firework_explosion",
        &[
            req("shape", Named("firework_shape")),
            opt("colors", Named("firework_colors")),
            opt("fade_colors", Named("firework_colors")),
            opt("has_trail", Bool),
            opt("has_twinkle", Bool),
        ],
    ),
    Row {
        name: "banner_pattern",
        kind: Either(&[
            Id,
            Object(&[
                req("asset_id", Id),
                req("translation_key", String(usize::MAX)),
            ]),
        ]),
    },
    Row {
        name: "instruments",
        kind: Either(&[
            Named("component_instrument"),
            Tag,
            List(&Named("component_instrument"), 0, usize::MAX),
        ]),
    },
    operation!("operation_100", 100),
    operation!("operation_256", 256),
    operation!("operation_max", i32::MAX as i64),
    edit!(
        "explosion_list_operation",
        Named("firework_explosion"),
        256,
        "operation_256"
    ),
    edit!(
        "float_list_operation",
        Named("float"),
        i32::MAX as usize,
        "operation_max"
    ),
    edit!(
        "bool_list_operation",
        Bool,
        i32::MAX as usize,
        "operation_max"
    ),
    edit!(
        "string_list_operation",
        String(usize::MAX),
        i32::MAX as usize,
        "operation_max"
    ),
    edit!(
        "color_list_operation",
        Either(&[Named("int"), Named("rgb")]),
        i32::MAX as usize,
        "operation_max"
    ),
    Row {
        name: "bonus_function",
        kind: All(&[
            Object(&[req("enchantment", Id)]),
            Switch(
                "formula",
                true,
                &[
                    row("ore_drops", &[]),
                    row(
                        "uniform_bonus_count",
                        &[req("parameters", Object(&[req("bonusMultiplier", Number)]))],
                    ),
                    row(
                        "binomial_with_bonus_count",
                        &[req(
                            "parameters",
                            Object(&[req("extra", Number), req("probability", Number)]),
                        )],
                    ),
                ],
            ),
        ]),
    },
    Row {
        name: "book_title",
        kind: Either(&[
            String(32),
            Object(&[req("raw", String(32)), opt("filtered", String(32))]),
        ]),
    },
    Row {
        name: "writable_page",
        kind: Either(&[
            String(1024),
            Object(&[req("raw", String(1024)), opt("filtered", String(1024))]),
        ]),
    },
    Row {
        name: "written_page",
        kind: Either(&[
            Named("text"),
            Object(&[req("raw", Named("text")), opt("filtered", Named("text"))]),
        ]),
    },
    Row {
        name: "writable_pages_function",
        kind: All(&[
            Object(&[req("pages", List(&Named("writable_page"), 0, 100))]),
            Named("operation_100"),
        ]),
    },
    Row {
        name: "written_pages_function",
        kind: All(&[
            Object(&[req("pages", List(&Named("written_page"), 0, usize::MAX))]),
            Named("operation_max"),
        ]),
    },
    Row {
        name: "lore_function",
        kind: All(&[
            Object(&[
                req("lore", List(&Named("text"), 0, 256)),
                opt("entity", Named("entity_target")),
            ]),
            Named("operation_256"),
        ]),
    },
    row("tooltip_function", &[req("toggles", IdMap(&Bool))]),
    Row {
        name: "bounds_int",
        kind: Either(&[
            Number,
            All(&[
                Object(&[opt("min", Number), opt("max", Number)]),
                OrderedInt("min", "max"),
            ]),
        ]),
    },
    Row {
        name: "bounds_double",
        kind: Either(&[
            Number,
            All(&[
                Object(&[opt("min", Number), opt("max", Number)]),
                Ordered("min", "max", false),
            ]),
        ]),
    },
    Row {
        name: "state_properties",
        kind: Map(&Either(&[
            String(usize::MAX),
            Object(&[
                opt("min", String(usize::MAX)),
                opt("max", String(usize::MAX)),
            ]),
        ])),
    },
    row(
        "component_matchers",
        &[
            opt("components", Named("component_values")),
            opt("predicates", Named("component_predicates")),
        ],
    ),
    Row {
        name: "item_predicate",
        kind: All(&[
            Object(&[
                opt("items", Named("ids")),
                opt("count", Named("bounds_int")),
            ]),
            Named("component_matchers"),
        ]),
    },
    Row {
        name: "block_predicate",
        kind: All(&[
            Object(&[
                opt("blocks", Named("ids")),
                opt("state", Named("state_properties")),
                opt("nbt", Snbt),
            ]),
            Named("component_matchers"),
        ]),
    },
    row(
        "fluid_predicate",
        &[
            opt("fluids", Named("ids")),
            opt("state", Named("state_properties")),
        ],
    ),
    row(
        "position_predicate",
        &[
            opt("x", Named("bounds_double")),
            opt("y", Named("bounds_double")),
            opt("z", Named("bounds_double")),
        ],
    ),
    row(
        "location_predicate",
        &[
            opt("position", Named("position_predicate")),
            opt("biomes", Named("ids")),
            opt("structures", Named("ids")),
            opt("dimension", Id),
            opt("smokey", Bool),
            opt("light", Object(&[opt("light", Named("bounds_int"))])),
            opt("block", Named("block_predicate")),
            opt("fluid", Named("fluid_predicate")),
            opt("can_see_sky", Bool),
        ],
    ),
    row(
        "damage_predicate",
        &[
            opt(
                "tags",
                List(
                    &Object(&[req("id", Named("ids")), req("expected", Bool)]),
                    0,
                    usize::MAX,
                ),
            ),
            opt("direct_entity", Named("entity_predicate")),
            opt("source_entity", Named("entity_predicate")),
            opt("is_direct", Bool),
        ],
    ),
    Row {
        name: "effects_predicate",
        kind: IdMap(&Object(&[
            opt("amplifier", Named("bounds_int")),
            opt("duration", Named("bounds_int")),
            opt("ambient", Bool),
            opt("visible", Bool),
        ])),
    },
];
