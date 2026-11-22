//! Context-number provider schemas. Related kinds share data, not validator code.
use super::{Field, Kind, Kind::*, Row, opt, req, row};
const I: Kind = Named("int");
const F: Kind = Named("float");
const IU: &[Field] = &[req("input", I)];
const FU: &[Field] = &[req("input", F)];
const IB: &[Field] = &[req("left", I), req("right", I)];
const FB: &[Field] = &[req("left", F), req("right", F)];
const IA: &[Field] = &[req("inputs", Named("ints"))];
const FA: &[Field] = &[req("inputs", Named("floats"))];
const IC: &[Field] = &[
    req("condition", Condition),
    req("on_true", I),
    opt("on_false", I),
];
const FC: &[Field] = &[
    req("condition", Condition),
    req("on_true", F),
    opt("on_false", F),
];
const ID: &[Field] = &[
    req(
        "cases",
        List(
            &Object(&[req("condition", Condition), req("value", I)]),
            0,
            usize::MAX,
        ),
    ),
    opt("default", I),
];
const FD: &[Field] = &[
    req(
        "cases",
        List(
            &Object(&[req("condition", Condition), req("value", F)]),
            0,
            usize::MAX,
        ),
    ),
    opt("default", F),
];
const IW: Kind = All(&[
    Object(&[req(
        "distribution",
        List(
            &Object(&[req("data", I), req("weight", IntRange(0, i32::MAX as i64))]),
            1,
            usize::MAX,
        ),
    )]),
    Object(&[req("distribution", PositiveWeights)]),
]);
const FW: Kind = All(&[
    Object(&[req(
        "distribution",
        List(
            &Object(&[req("data", F), req("weight", IntRange(0, i32::MAX as i64))]),
            1,
            usize::MAX,
        ),
    )]),
    Object(&[req("distribution", PositiveWeights)]),
]);
const INTS: &[Row] = &[
    row("abs", IU),
    row("avg", IA),
    row("binomial", &[req("n", I), req("p", F)]),
    row("conditional", IC),
    row("constant", &[req("value", Number)]),
    row("sub", IB),
    row(
        "environment_attribute",
        &[req("attribute", Named("int_attribute"))],
    ),
    row("from_float", FU),
    row("max", IA),
    row("min", IA),
    row("floor_mod", IB),
    row("floor_div", IB),
    row("mod", IB),
    row("div", IB),
    row("negate", IU),
    row("number_dispatcher", ID),
    row("pow", &[req("base", I), req("exponent", I)]),
    row("mul", IA),
    row(
        "score",
        &[
            req("target", Named("score_target")),
            req("score", String(usize::MAX)),
            opt("fallback", I),
        ],
    ),
    row(
        "storage",
        &[req("storage", Id), req("path", NbtPath), opt("fallback", I)],
    ),
    row("add", IA),
    row("uniform", &[req("min", I), req("max", I)]),
    Row {
        name: "weighted_list",
        kind: IW,
    },
];
const FLOATS: &[Row] = &[
    row("abs", FU),
    row("avg", FA),
    row("ceil", FU),
    row("conditional", FC),
    row("constant", &[req("value", Number)]),
    row("cos", FU),
    row("sub", FB),
    row("enchantment_level", &[req("amount", Named("level_value"))]),
    row(
        "environment_attribute",
        &[req("attribute", Named("float_attribute"))],
    ),
    row("floor", FU),
    row("from_int", IU),
    row("length", FA),
    row("max", FA),
    row("min", FA),
    row("mod", FB),
    row("negate", FU),
    row("number_dispatcher", FD),
    row("pow", &[req("base", F), req("exponent", F)]),
    row("mul", FA),
    row("div", FB),
    row("round", FU),
    row("sin", FU),
    row("sqrt", FU),
    row(
        "storage",
        &[req("storage", Id), req("path", NbtPath), opt("fallback", F)],
    ),
    row("add", FA),
    row("truncate", FU),
    row("uniform", &[req("min", F), req("max", F)]),
    Row {
        name: "weighted_list",
        kind: FW,
    },
];
const LEVELS: &[Row] = &[
    Row {
        name: "clamped",
        kind: All(&[
            Object(&[
                req("value", Named("level_value")),
                req("min", Number),
                req("max", Number),
            ]),
            OrderedFloat("min", "max", true),
        ]),
    },
    row(
        "fraction",
        &[
            req("numerator", Named("level_value")),
            req("denominator", Named("level_value")),
        ],
    ),
    row("levels_squared", &[req("added", Number)]),
    row(
        "linear",
        &[req("base", Number), req("per_level_above_first", Number)],
    ),
    row(
        "exponent",
        &[
            req("base", Named("level_value")),
            req("power", Named("level_value")),
        ],
    ),
    row(
        "lookup",
        &[
            req("values", List(&Number, 0, usize::MAX)),
            req("fallback", Named("level_value")),
        ],
    ),
];
pub(super) const ROWS: &[Row] = &[
    Row {
        name: "int",
        kind: Either(&[Number, Id, Switch("type", true, INTS)]),
    },
    Row {
        name: "float",
        kind: Either(&[Number, Id, Switch("type", true, FLOATS)]),
    },
    Row {
        name: "ints",
        kind: Either(&[List(&I, 0, usize::MAX), I, Tag]),
    },
    Row {
        name: "floats",
        kind: Either(&[List(&F, 0, usize::MAX), F, Tag]),
    },
    Row {
        name: "int_range",
        kind: Either(&[I, Object(&[opt("min", I), opt("max", I)])]),
    },
    Row {
        name: "float_range",
        kind: Either(&[F, Object(&[opt("min", F), opt("max", F)])]),
    },
    row("int_limit", &[opt("min", I), opt("max", I)]),
    Row {
        name: "level_value",
        kind: Either(&[Number, Switch("type", true, LEVELS)]),
    },
    Row {
        name: "score_target",
        kind: Either(&[
            Named("entity_target"),
            Switch(
                "type",
                true,
                &[
                    row("fixed", &[req("name", String(usize::MAX))]),
                    row("context", &[req("target", Named("entity_target"))]),
                ],
            ),
        ]),
    },
    Row {
        name: "nbt_provider",
        kind: Either(&[
            Named("entity_or_block"),
            Switch(
                "type",
                true,
                &[
                    row("context", &[req("target", Named("entity_or_block"))]),
                    row("storage", &[req("source", Id)]),
                ],
            ),
        ]),
    },
];
