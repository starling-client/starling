//! Text is accepted in compact string/list and typed or fuzzy object forms.
use super::{Kind::*, Row, opt, req, row};
const SOURCES: &[Row] = &[
    row("entity", &[req("entity", String(usize::MAX))]),
    row("block", &[req("block", String(usize::MAX))]),
    row("storage", &[req("storage", Id)]),
];
const OBJECTS: &[Row] = &[
    row("atlas", &[opt("atlas", Id), req("sprite", Id)]),
    row(
        "player",
        &[req("player", Named("component_profile")), opt("hat", Bool)],
    ),
];
const CONTENTS: &[Row] = &[
    row("text", &[req("text", String(usize::MAX))]),
    row(
        "translatable",
        &[
            req("translate", String(usize::MAX)),
            opt("fallback", Any),
            opt(
                "with",
                List(&Either(&[Number, Bool, Named("text")]), 0, usize::MAX),
            ),
        ],
    ),
    row("keybind", &[req("keybind", String(usize::MAX))]),
    row(
        "score",
        &[req(
            "score",
            Object(&[
                req("name", String(usize::MAX)),
                req("objective", String(usize::MAX)),
            ]),
        )],
    ),
    row(
        "selector",
        &[
            req("selector", String(usize::MAX)),
            opt("separator", Named("text")),
        ],
    ),
    Row {
        name: "nbt",
        kind: All(&[
            Object(&[
                req("nbt", NbtPath),
                opt("interpret", Any),
                opt("plain", Any),
                opt("separator", Any),
            ]),
            NotBoth("interpret", "plain"),
            FuzzySwitch("source", SOURCES),
        ]),
    },
    Row {
        name: "object",
        kind: All(&[
            FuzzySwitch("object", OBJECTS),
            Object(&[opt("fallback", Named("text"))]),
        ]),
    },
];
const CLICKS: &[Row] = &[
    row("open_url", &[req("url", HttpScheme)]),
    row("run_command", &[req("command", ChatString)]),
    row("suggest_command", &[req("command", ChatString)]),
    row("show_dialog", &[req("dialog", Named("dialog"))]),
    row("change_page", &[req("page", IntRange(1, i32::MAX as i64))]),
    row("copy_to_clipboard", &[req("value", String(usize::MAX))]),
    row("custom", &[req("id", Id), opt("payload", Any)]),
];
const HOVERS: &[Row] = &[
    row("show_text", &[req("value", Named("text"))]),
    row(
        "show_item",
        &[
            req("id", NonAirItemId),
            opt("count", IntRange(1, 99)),
            opt("components", Named("component_patch")),
        ],
    ),
    row(
        "show_entity",
        &[
            req("id", Id),
            req("uuid", Either(&[Named("component_uuid"), Uuid])),
            opt("name", Named("text")),
        ],
    ),
];
pub(super) const ROWS: &[Row] = &[
    Row {
        name: "text",
        kind: Text,
    },
    Row {
        name: "text_fields",
        kind: Either(&[
            String(usize::MAX),
            List(&Named("text"), 1, usize::MAX),
            All(&[
                FuzzySwitch("type", CONTENTS),
                Named("style"),
                Object(&[opt("extra", List(&Named("text"), 1, usize::MAX))]),
            ]),
        ]),
    },
    row(
        "style",
        &[
            opt("color", TextColor),
            opt("shadow_color", Named("argb")),
            opt("bold", Bool),
            opt("italic", Bool),
            opt("underlined", Bool),
            opt("strikethrough", Bool),
            opt("obfuscated", Bool),
            opt("click_event", Switch("action", false, CLICKS)),
            opt("hover_event", Switch("action", false, HOVERS)),
            opt("insertion", String(usize::MAX)),
            opt("font", Id),
        ],
    ),
    Row {
        name: "rgb",
        kind: Either(&[Number, List(&Number, 3, 3)]),
    },
    Row {
        name: "argb",
        kind: Either(&[Number, List(&Number, 4, 4)]),
    },
    Row {
        name: "rgb_string",
        kind: Either(&[HexColor(6), Named("rgb")]),
    },
    Row {
        name: "argb_string",
        kind: Either(&[HexColor(8), Named("argb")]),
    },
    Row {
        name: "sound",
        kind: Either(&[Id, Object(&[req("sound_id", Id), opt("range", Any)])]),
    },
];
