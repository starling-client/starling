//! One row per built-in function. Reusable payload records live in the other families.
use super::{Kind::*, Row, opt, req, row};
pub(super) const ROWS: &[Row] = &[
    row("set_count", &[req("count", Named("int")), opt("add", Bool)]),
    row("set_item", &[req("item", NonAirItemId)]),
    row(
        "enchant_with_levels",
        &[
            req("levels", Named("int")),
            opt("options", Named("ids")),
            opt("include_additional_cost_component", Bool),
        ],
    ),
    row(
        "enchant_randomly",
        &[
            opt("options", Named("ids")),
            opt("only_compatible", Bool),
            opt("include_additional_cost_component", Bool),
        ],
    ),
    row(
        "set_enchantments",
        &[opt("enchantments", IdMap(&Named("int"))), opt("add", Bool)],
    ),
    row("set_custom_data", &[req("tag", Snbt)]),
    row(
        "set_components",
        &[req("components", Named("component_patch"))],
    ),
    row("furnace_smelt", &[opt("use_input_count", Bool)]),
    row(
        "enchanted_count_increase",
        &[
            req("enchantment", Id),
            req("count", Named("float")),
            opt("limit", Number),
        ],
    ),
    row(
        "set_damage",
        &[req("damage", Named("float")), opt("add", Bool)],
    ),
    row(
        "set_attributes",
        &[
            req(
                "modifiers",
                List(&Named("attribute_modifier"), 0, usize::MAX),
            ),
            opt("replace", Bool),
        ],
    ),
    row(
        "set_name",
        &[
            opt("name", Named("text")),
            opt("entity", Named("entity_target")),
            opt("target", Enum(&["custom_name", "item_name"])),
        ],
    ),
    row(
        "exploration_map",
        &[
            req("destination", Named("ids")),
            opt("decoration", Id),
            opt("zoom", Number),
            opt("search_radius", Number),
            opt("skip_existing_chunks", Bool),
        ],
    ),
    row(
        "set_stew_effect",
        &[opt(
            "effects",
            List(
                &Object(&[req("type", Id), req("duration", Named("int"))]),
                0,
                usize::MAX,
            ),
        )],
    ),
    row("copy_name", &[req("source", Named("entity_or_block"))]),
    row(
        "set_contents",
        &[
            req("component", Named("container_component")),
            req("entries", List(&Entry, 0, usize::MAX)),
        ],
    ),
    row(
        "modify_contents",
        &[
            req("component", Named("container_component")),
            req("modifier", Function),
        ],
    ),
    row(
        "filtered",
        &[
            req("item_filter", Named("item_predicate")),
            opt("on_pass", Function),
            opt("on_fail", Function),
        ],
    ),
    row("limit_count", &[req("limit", Named("int_limit"))]),
    Row {
        name: "apply_bonus",
        kind: Named("bonus_function"),
    },
    row(
        "set_loot_table",
        &[req("loot_table_id", Id), opt("seed", Number)],
    ),
    row("explosion_decay", &[]),
    Row {
        name: "set_lore",
        kind: Named("lore_function"),
    },
    row("fill_player_head", &[req("entity", Named("entity_target"))]),
    row(
        "copy_custom_data",
        &[
            req("source", Named("nbt_provider")),
            req(
                "ops",
                List(
                    &Object(&[
                        req("source", NbtPath),
                        req("target", NbtPath),
                        req("op", Enum(&["replace", "append", "merge"])),
                    ]),
                    0,
                    usize::MAX,
                ),
            ),
        ],
    ),
    row(
        "copy_state",
        &[
            req("block", Id),
            req("properties", List(&String(usize::MAX), 0, usize::MAX)),
        ],
    ),
    row(
        "set_banner_pattern",
        &[
            req(
                "patterns",
                List(
                    &Object(&[
                        req("pattern", Named("banner_pattern")),
                        req("color", Named("dye_color")),
                    ]),
                    0,
                    usize::MAX,
                ),
            ),
            req("append", Bool),
        ],
    ),
    row("set_potion", &[req("id", Id)]),
    row("set_random_dyes", &[req("number_of_dyes", Named("int"))]),
    row("set_random_potion", &[opt("options", Named("ids"))]),
    row("set_instrument", &[req("options", Named("instruments"))]),
    row("sequence", &[req("functions", Named("functions"))]),
    row(
        "copy_components",
        &[
            req("source", Named("component_source")),
            opt("include", List(&Id, 0, usize::MAX)),
            opt("exclude", List(&Id, 0, usize::MAX)),
        ],
    ),
    row(
        "set_fireworks",
        &[
            opt("explosions", Named("explosion_list_operation")),
            opt("flight_duration", Number),
        ],
    ),
    row(
        "set_firework_explosion",
        &[
            opt("shape", Named("firework_shape")),
            opt("colors", Named("firework_colors")),
            opt("fade_colors", Named("firework_colors")),
            opt("trail", Bool),
            opt("twinkle", Bool),
        ],
    ),
    row(
        "set_book_cover",
        &[
            opt("title", Named("book_title")),
            opt("author", String(usize::MAX)),
            opt("generation", IntRange(0, 3)),
        ],
    ),
    Row {
        name: "set_written_book_pages",
        kind: Named("written_pages_function"),
    },
    Row {
        name: "set_writable_book_pages",
        kind: Named("writable_pages_function"),
    },
    Row {
        name: "toggle_tooltips",
        kind: Named("tooltip_function"),
    },
    row(
        "set_ominous_bottle_amplifier",
        &[req("amplifier", Named("int"))],
    ),
    row(
        "set_custom_model_data",
        &[
            opt("floats", Named("float_list_operation")),
            opt("flags", Named("bool_list_operation")),
            opt("strings", Named("string_list_operation")),
            opt("colors", Named("color_list_operation")),
        ],
    ),
    row("discard", &[]),
];
