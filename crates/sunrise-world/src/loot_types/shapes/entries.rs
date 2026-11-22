//! Entry, table and slot-source arguments nested inside set_contents.
//! These describe nested arguments, not a new whole-resource loader contract.
use super::{Kind::*, Row, opt, req, row};
const TYPES: &[Row] = &[
    Row {
        name: "empty",
        kind: Named("uniform_entry"),
    },
    Row {
        name: "item",
        kind: All(&[Named("uniform_entry"), Object(&[req("name", NonAirItemId)])]),
    },
    Row {
        name: "dynamic",
        kind: All(&[Named("uniform_entry"), Object(&[req("name", Id)])]),
    },
    Row {
        name: "tag",
        kind: All(&[
            Named("uniform_entry"),
            Object(&[req("items", Named("ids")), opt("expand", Bool)]),
        ]),
    },
    Row {
        name: "loot_table",
        kind: All(&[
            Named("uniform_entry"),
            Object(&[req("value", Named("loot_tables")), opt("expand", Bool)]),
        ]),
    },
    Row {
        name: "slots",
        kind: All(&[
            Named("uniform_entry"),
            Object(&[req("slot_source", Named("slot_source"))]),
        ]),
    },
    row(
        "alternatives",
        &[opt("children", List(&Entry, 0, usize::MAX))],
    ),
    row("sequence", &[opt("children", List(&Entry, 0, usize::MAX))]),
    row("group", &[opt("children", List(&Entry, 0, usize::MAX))]),
];
const SLOT_TYPES: &[Row] = &[
    row("group", &[req("terms", Named("slot_sources"))]),
    row(
        "filtered",
        &[
            req("slot_source", Named("slot_source")),
            req("item_filter", Named("item_predicate")),
        ],
    ),
    row(
        "limit_slots",
        &[
            req("slot_source", Named("slot_source")),
            req("limit", IntRange(1, i32::MAX as i64)),
        ],
    ),
    row(
        "slot_range",
        &[
            opt(
                "source",
                Either(&[Named("entity_target"), Enum(&["block_entity", "container"])]),
            ),
            req("slots", SlotRange),
        ],
    ),
    row(
        "contents",
        &[
            req("slot_source", Named("slot_source")),
            req("component", Named("container_component")),
        ],
    ),
    row("empty", &[]),
];
pub(super) const ROWS: &[Row] = &[
    Row {
        name: "loot_entry",
        kind: All(&[
            Object(&[opt("condition", Condition), opt("modifier", Function)]),
            Switch("type", true, TYPES),
        ]),
    },
    row(
        "uniform_entry",
        &[
            opt("weight", IntRange(i32::MIN as i64, i32::MAX as i64)),
            opt("quality", IntRange(i32::MIN as i64, i32::MAX as i64)),
        ],
    ),
    Row {
        name: "loot_tables",
        kind: Either(&[List(&Table, 0, usize::MAX), Table, Tag]),
    },
    row(
        "loot_table_direct",
        &[
            // LootTable uses lenientOptionalFieldOf: even a malformed context
            // set falls back to its default rather than refusing the table.
            opt("type", Any),
            opt("random_sequence", Id),
            opt("pools", List(&Named("loot_pool"), 0, usize::MAX)),
            opt("modifier", Function),
        ],
    ),
    row(
        "loot_pool",
        &[
            req("entries", List(&Entry, 0, usize::MAX)),
            opt("condition", Condition),
            opt("modifier", Function),
            req("rolls", Named("int")),
            opt("bonus_rolls", Named("float")),
        ],
    ),
    Row {
        name: "slot_source",
        kind: Either(&[Id, Switch("type", true, SLOT_TYPES), Named("slot_sources")]),
    },
    Row {
        name: "slot_sources",
        kind: Either(&[
            List(&Named("slot_source_element"), 0, usize::MAX),
            Named("slot_source_element"),
            Tag,
        ]),
    },
    Row {
        name: "slot_source_element",
        kind: Either(&[Id, Switch("type", true, SLOT_TYPES)]),
    },
];
