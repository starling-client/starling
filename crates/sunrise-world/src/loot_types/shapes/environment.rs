//! Environment values reuse the owning attribute registry's names, types and ranges.
use super::{Kind, Kind::*, Row, opt, req, row};
use crate::environment_attributes::{self as attributes, Kind as AttributeKind, Range};

const NN: Kind = IntRange(0, i32::MAX as i64);
const COUNT: Kind = Named("environment_count");
const WEIGHTED_COUNT: Kind = All(&[
    Object(&[req(
        "distribution",
        List(
            &Object(&[req("data", COUNT), req("weight", NN)]),
            1,
            usize::MAX,
        ),
    )]),
    Object(&[req("distribution", PositiveWeights)]),
]);
const COUNTS: &[Row] = &[
    row("constant", &[req("value", Number)]),
    Row {
        name: "uniform",
        kind: Named("environment_count_bounds"),
    },
    Row {
        name: "biased_to_bottom",
        kind: Named("environment_count_bounds"),
    },
    Row {
        name: "very_biased_to_bottom",
        kind: Named("environment_count_bounds"),
    },
    Row {
        name: "clamped",
        kind: All(&[
            Named("environment_count_bounds"),
            Object(&[req("source", COUNT)]),
        ]),
    },
    Row {
        name: "clamped_normal",
        kind: All(&[
            Named("environment_count_bounds"),
            Object(&[req("mean", Number), req("deviation", Number)]),
        ]),
    },
    Row {
        name: "weighted_list",
        kind: WEIGHTED_COUNT,
    },
    Row {
        name: "trapezoid",
        kind: All(&[
            Object(&[
                req("min", Number),
                req("max", Number),
                req("plateau", Number),
            ]),
            OrderedInt("min", "max"),
            DifferenceLimit("min", "max", "plateau"),
        ]),
    },
];
const CATEGORIES: &[&str] = &[
    "monster",
    "creature",
    "ambient",
    "axolotls",
    "underground_water_creature",
    "water_creature",
    "water_ambient",
    "misc",
];
const OTHER: &[Row] = &[
    Row {
        name: "visual/moon_phase",
        kind: Enum(&[
            "full_moon",
            "waning_gibbous",
            "third_quarter",
            "waning_crescent",
            "new_moon",
            "waxing_crescent",
            "first_quarter",
            "waxing_gibbous",
        ]),
    },
    Row {
        name: "visual/default_dripstone_particle",
        kind: Named("particle"),
    },
    Row {
        name: "visual/ambient_particles",
        kind: List(
            &Object(&[
                req("particle", Named("particle")),
                req("probability", FloatRange(0.0, 1.0)),
            ]),
            0,
            usize::MAX,
        ),
    },
    Row {
        name: "audio/background_music",
        kind: Named("environment_music"),
    },
    Row {
        name: "audio/ambient_sounds",
        kind: Named("environment_sounds"),
    },
    Row {
        name: "gameplay/bed_rule",
        kind: Named("environment_bed"),
    },
    Row {
        name: "gameplay/straw_bed_rule",
        kind: Named("environment_bed"),
    },
    Row {
        name: "gameplay/eyeblossom_open",
        kind: Either(&[Bool, Enum(&["true", "false", "default"])]),
    },
    Row {
        name: "gameplay/natural_mob_spawns",
        kind: Named("environment_spawns"),
    },
    Row {
        name: "gameplay/villager_activity",
        kind: Named("environment_activity"),
    },
    Row {
        name: "gameplay/baby_villager_activity",
        kind: Named("environment_activity"),
    },
];
pub(super) const ROWS: &[Row] = &[
    Row {
        name: "environment_check",
        kind: EnvironmentCheck,
    },
    Row {
        name: "int_attribute",
        kind: AttributeNumber(false),
    },
    Row {
        name: "float_attribute",
        kind: AttributeNumber(true),
    },
    row(
        "environment_bed",
        &[
            req("can_sleep", Enum(&["always", "when_dark", "never"])),
            req("can_set_spawn", Enum(&["always", "when_dark", "never"])),
            opt("destroy_on_use", Bool),
            opt("destroy_on_leave", Bool),
            opt("error_message", Named("text")),
        ],
    ),
    row(
        "environment_music_entry",
        &[
            req("sound", Named("sound")),
            req("min_delay", NN),
            req("max_delay", NN),
            opt("replace_current_music", Bool),
        ],
    ),
    row(
        "environment_music",
        &[
            opt("default", Named("environment_music_entry")),
            opt("creative", Named("environment_music_entry")),
            opt("underwater", Named("environment_music_entry")),
        ],
    ),
    row(
        "environment_mood",
        &[
            req("sound", Named("sound")),
            req("tick_delay", Number),
            req("block_search_extent", Number),
            req("offset", Number),
        ],
    ),
    row(
        "environment_additions",
        &[req("sound", Named("sound")), req("tick_chance", Number)],
    ),
    row(
        "environment_sounds",
        &[
            opt("loop", Named("sound")),
            opt("mood", Named("environment_mood")),
            opt(
                "additions",
                Either(&[
                    List(&Named("environment_additions"), 0, usize::MAX),
                    Named("environment_additions"),
                ]),
            ),
        ],
    ),
    Row {
        name: "environment_activity",
        kind: RegistryEnum(&[
            "core",
            "idle",
            "work",
            "play",
            "rest",
            "meet",
            "panic",
            "raid",
            "pre_raid",
            "hide",
            "fight",
            "celebrate",
            "admire_item",
            "avoid",
            "ride",
            "play_dead",
            "long_jump",
            "ram",
            "tongue",
            "swim",
            "lay_spawn",
            "sniff",
            "investigate",
            "roar",
            "emerge",
            "dig",
        ]),
    },
    Row {
        name: "environment_count",
        kind: Either(&[Number, Switch("type", true, COUNTS)]),
    },
    Row {
        name: "environment_count_bounds",
        kind: All(&[
            Object(&[req("min_inclusive", Number), req("max_inclusive", Number)]),
            OrderedInt("min_inclusive", "max_inclusive"),
        ]),
    },
    row(
        "environment_spawn_entry",
        &[req("type", Id), req("count", COUNT), req("weight", NN)],
    ),
    row(
        "environment_spawns",
        &[
            // Vanilla promotes partial list decoding here; the kernel checks the
            // surviving entries' weight total without refusing recoverable entries.
            req(
                "spawns_by_category",
                EnumMap(
                    CATEGORIES,
                    &PartialWeightedList(&Named("environment_spawn_entry")),
                ),
            ),
            req(
                "spawn_costs",
                IdMap(&Object(&[
                    req("energy_budget", Number),
                    req("charge", Number),
                ])),
            ),
        ],
    ),
];

pub(super) fn value(attribute: &attributes::Attribute) -> Option<Kind> {
    Some(match attribute.kind {
        AttributeKind::Boolean => Bool,
        AttributeKind::Float | AttributeKind::AngleDegrees => match attribute.range {
            Range::Any => Number,
            Range::Float(min, max) => FloatRange(f64::from(min), f64::from(max)),
        },
        AttributeKind::RgbColor => Named("rgb_string"),
        AttributeKind::ArgbColor => Named("argb_string"),
        AttributeKind::AmbientParticles | AttributeKind::Other => {
            OTHER.iter().find(|row| row.name == attribute.id)?.kind
        }
    })
}
