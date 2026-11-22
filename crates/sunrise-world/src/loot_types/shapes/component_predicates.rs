//! Partial component predicates have their own schemas, not component-value limits.
use super::{Kind::*, Row, opt, req, row};
macro_rules! collection {
    ($name:literal, $element:expr) => {
        row(
            $name,
            &[
                opt("contains", List(&$element, 0, usize::MAX)),
                opt(
                    "count",
                    List(
                        &Object(&[req("test", $element), req("count", Named("bounds_int"))]),
                        0,
                        usize::MAX,
                    ),
                ),
                opt("size", Named("bounds_int")),
            ],
        )
    };
}
pub(super) const TYPES: &[Row] = &[
    row(
        "damage",
        &[
            opt("durability", Named("bounds_int")),
            opt("damage", Named("bounds_int")),
        ],
    ),
    Row {
        name: "enchantments",
        kind: Named("enchantment_predicates"),
    },
    Row {
        name: "stored_enchantments",
        kind: Named("enchantment_predicates"),
    },
    row(
        "potion_contents",
        &[
            opt("potions", Named("ids")),
            opt("effects", Named("effect_collection")),
        ],
    ),
    Row {
        name: "custom_data",
        kind: Snbt,
    },
    row("container", &[opt("items", Named("item_collection"))]),
    row("bundle_contents", &[opt("items", Named("item_collection"))]),
    Row {
        name: "firework_explosion",
        kind: Named("firework_predicate"),
    },
    row(
        "fireworks",
        &[
            opt("explosions", Named("firework_collection")),
            opt("flight_duration", Named("bounds_int")),
        ],
    ),
    row(
        "writable_book_content",
        &[opt("pages", Named("string_collection"))],
    ),
    row(
        "written_book_content",
        &[
            opt("pages", Named("text_collection")),
            opt("author", String(usize::MAX)),
            opt("title", String(usize::MAX)),
            opt("generation", Named("bounds_int")),
            opt("resolved", Bool),
        ],
    ),
    row(
        "attribute_modifiers",
        &[opt("modifiers", Named("attribute_collection"))],
    ),
    row(
        "trim",
        &[opt("material", Named("ids")), opt("pattern", Named("ids"))],
    ),
    row("jukebox_playable", &[opt("song", Named("ids"))]),
    Row {
        name: "villager/variant",
        kind: Named("ids"),
    },
];
pub(super) const ROWS: &[Row] = &[
    collection!("item_collection", Named("item_predicate")),
    collection!("effect_collection", Named("effects_predicate")),
    collection!("firework_collection", Named("firework_predicate")),
    collection!("string_collection", String(usize::MAX)),
    collection!("text_collection", Named("text")),
    collection!(
        "attribute_collection",
        Object(&[
            opt("attribute", Named("ids")),
            opt("id", Id),
            opt("amount", Named("bounds_double")),
            opt("operation", Named("attribute_operation")),
            opt("slot", Named("slot_group"))
        ])
    ),
    Row {
        name: "enchantment_predicates",
        kind: List(
            &Object(&[
                opt("enchantments", Named("ids")),
                opt("levels", Named("bounds_int")),
            ]),
            0,
            usize::MAX,
        ),
    },
    row(
        "firework_predicate",
        &[
            opt("shape", Named("firework_shape")),
            opt("has_twinkle", Bool),
            opt("has_trail", Bool),
        ],
    ),
];
