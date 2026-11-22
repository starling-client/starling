//! Entity subpredicates are a map of registry keys to individual payloads.
use super::{Kind::*, Row, opt, req, row};
const TYPES: &[Row] = &[
    Row {
        name: "entity_type",
        kind: Named("ids"),
    },
    Row {
        name: "location",
        kind: Named("location_predicate"),
    },
    Row {
        name: "stepping_on",
        kind: Named("location_predicate"),
    },
    Row {
        name: "movement_affected_by",
        kind: Named("location_predicate"),
    },
    row(
        "distance",
        &[
            opt("x", Named("bounds_double")),
            opt("y", Named("bounds_double")),
            opt("z", Named("bounds_double")),
            opt("horizontal", Named("bounds_double")),
            opt("absolute", Named("bounds_double")),
        ],
    ),
    row(
        "movement",
        &[
            opt("x", Named("bounds_double")),
            opt("y", Named("bounds_double")),
            opt("z", Named("bounds_double")),
            opt("speed", Named("bounds_double")),
            opt("horizontal_speed", Named("bounds_double")),
            opt("vertical_speed", Named("bounds_double")),
            opt("fall_distance", Named("bounds_double")),
        ],
    ),
    Row {
        name: "effects",
        kind: Named("effects_predicate"),
    },
    Row {
        name: "nbt",
        kind: Snbt,
    },
    row(
        "flags",
        &[
            opt("is_on_ground", Bool),
            opt("is_on_fire", Bool),
            opt("is_sneaking", Bool),
            opt("is_sprinting", Bool),
            opt("is_swimming", Bool),
            opt("is_flying", Bool),
            opt("is_baby", Bool),
            opt("is_in_water", Bool),
            opt("is_fall_flying", Bool),
        ],
    ),
    row(
        "equipment",
        &[
            opt("head", Named("item_predicate")),
            opt("chest", Named("item_predicate")),
            opt("legs", Named("item_predicate")),
            opt("feet", Named("item_predicate")),
            opt("body", Named("item_predicate")),
            opt("mainhand", Named("item_predicate")),
            opt("offhand", Named("item_predicate")),
        ],
    ),
    Row {
        name: "periodic_tick",
        kind: IntRange(1, i32::MAX as i64),
    },
    Row {
        name: "vehicle",
        kind: Named("entity_predicate"),
    },
    Row {
        name: "passenger",
        kind: Named("entity_predicate"),
    },
    Row {
        name: "targeted_entity",
        kind: Named("entity_predicate"),
    },
    Row {
        name: "team",
        kind: String(usize::MAX),
    },
    Row {
        name: "slots",
        kind: SlotMap(&Named("item_predicate")),
    },
    Row {
        name: "components",
        kind: Named("component_values"),
    },
    Row {
        name: "predicates",
        kind: Named("component_predicates"),
    },
    row(
        "entity_tags",
        &[
            opt("any_of", List(&String(usize::MAX), 0, usize::MAX)),
            opt("all_of", List(&String(usize::MAX), 0, usize::MAX)),
            opt("none_of", List(&String(usize::MAX), 0, usize::MAX)),
        ],
    ),
    row(
        "type_specific/lightning",
        &[
            opt("blocks_set_on_fire", Named("bounds_int")),
            opt("entity_struck", Named("entity_predicate")),
        ],
    ),
    row("type_specific/fishing_hook", &[opt("in_open_water", Bool)]),
    Row {
        name: "type_specific/player",
        kind: Named("player_predicate"),
    },
    row(
        "type_specific/cube_mob",
        &[opt("size", Named("bounds_int"))],
    ),
    row(
        "type_specific/raider",
        &[opt("has_raid", Bool), opt("is_captain", Bool)],
    ),
    row("type_specific/sheep", &[opt("sheared", Bool)]),
];
pub(super) const ROWS: &[Row] = &[
    Row {
        name: "entity_predicate",
        kind: RegistryMap(TYPES),
    },
    row(
        "player_predicate",
        &[
            opt("level", Named("bounds_int")),
            opt(
                "food",
                Object(&[
                    opt("level", Named("bounds_int")),
                    opt("saturation", Named("bounds_double")),
                ]),
            ),
            opt(
                "gamemode",
                List(
                    &Enum(&["survival", "creative", "adventure", "spectator"]),
                    0,
                    usize::MAX,
                ),
            ),
            opt(
                "stats",
                List(
                    &Object(&[
                        req(
                            "type",
                            RegistryEnum(&[
                                "mined",
                                "crafted",
                                "used",
                                "broken",
                                "picked_up",
                                "dropped",
                                "killed",
                                "killed_by",
                                "custom",
                            ]),
                        ),
                        req("stat", Id),
                        opt("value", Named("bounds_int")),
                    ]),
                    0,
                    usize::MAX,
                ),
            ),
            opt("recipes", IdMap(&Bool)),
            opt("advancements", IdMap(&Either(&[Bool, Map(&Bool)]))),
            opt("looking_at", Named("entity_predicate")),
            opt(
                "input",
                Object(&[
                    opt("forward", Bool),
                    opt("backward", Bool),
                    opt("left", Bool),
                    opt("right", Bool),
                    opt("jump", Bool),
                    opt("sneak", Bool),
                    opt("sprint", Bool),
                ]),
            ),
        ],
    ),
];
