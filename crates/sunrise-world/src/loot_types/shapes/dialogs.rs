//! The recursively referenced dialog records inside text click events.
use super::{Kind::*, Row, opt, req, row};
const WIDTH: super::Kind = IntRange(1, 1024);
const POS: super::Kind = IntRange(1, i32::MAX as i64);
const POS_FLOAT: super::Kind = FloatRange(f32::from_bits(1) as f64, f32::MAX as f64);
const BODY: &[Row] = &[
    row(
        "plain_message",
        &[req("contents", Named("text")), opt("width", WIDTH)],
    ),
    row(
        "item",
        &[
            req("item", Named("component_template")),
            opt("description", Named("dialog_plain")),
            opt("show_decorations", Bool),
            opt("show_tooltip", Bool),
            opt("width", IntRange(1, 256)),
            opt("height", IntRange(1, 256)),
        ],
    ),
];
const TYPES: &[Row] = &[
    row("notice", &[opt("action", Named("dialog_button"))]),
    row(
        "server_links",
        &[
            opt("exit_action", Named("dialog_button")),
            opt("columns", POS),
            opt("button_width", WIDTH),
        ],
    ),
    row(
        "dialog_list",
        &[
            req("dialogs", Named("dialog_list")),
            opt("exit_action", Named("dialog_button")),
            opt("columns", POS),
            opt("button_width", WIDTH),
        ],
    ),
    row(
        "multi_action",
        &[
            req("actions", List(&Named("dialog_button"), 1, usize::MAX)),
            opt("exit_action", Named("dialog_button")),
            opt("columns", POS),
        ],
    ),
    row(
        "confirmation",
        &[
            req("yes", Named("dialog_button")),
            req("no", Named("dialog_button")),
        ],
    ),
];
const INPUTS: &[Row] = &[
    row(
        "boolean",
        &[
            req("label", Named("text")),
            opt("initial", Bool),
            opt("on_true", String(usize::MAX)),
            opt("on_false", String(usize::MAX)),
        ],
    ),
    Row {
        name: "number_range",
        kind: All(&[
            Object(&[
                opt("width", WIDTH),
                req("label", Named("text")),
                opt("label_format", String(usize::MAX)),
                req("start", Number),
                req("end", Number),
                opt("initial", Number),
                opt("step", POS_FLOAT),
            ]),
            WithinEndpoints("initial", "start", "end"),
        ]),
    },
    Row {
        name: "single_option",
        kind: All(&[
            Object(&[
                opt("width", WIDTH),
                req("options", List(&Named("dialog_option"), 1, usize::MAX)),
                req("label", Named("text")),
                opt("label_visible", Bool),
            ]),
            AtMostOne("options", "initial"),
        ]),
    },
    Row {
        name: "text",
        kind: All(&[
            Object(&[
                opt("width", WIDTH),
                req("label", Named("text")),
                opt("label_visible", Bool),
                opt("initial", String(usize::MAX)),
                opt("max_length", POS),
                opt(
                    "multiline",
                    Object(&[opt("max_lines", POS), opt("height", IntRange(1, 512))]),
                ),
            ]),
            StringLimit("initial", "max_length", 32),
        ]),
    },
];
const ACTIONS: &[Row] = &[
    row("open_url", &[req("url", HttpScheme)]),
    row("run_command", &[req("command", ChatString)]),
    row("suggest_command", &[req("command", ChatString)]),
    row("show_dialog", &[req("dialog", Named("dialog"))]),
    row("change_page", &[req("page", POS)]),
    row("copy_to_clipboard", &[req("value", String(usize::MAX))]),
    row("custom", &[req("id", Id), opt("payload", Any)]),
    row("dynamic/run_command", &[req("template", MacroTemplate)]),
    // CompoundTag is object-only here, unlike CustomData's flattened SNBT alternative.
    row(
        "dynamic/custom",
        &[req("id", Id), opt("additions", Object(&[]))],
    ),
];
pub(super) const ROWS: &[Row] = &[
    Row {
        name: "dialog",
        kind: Either(&[
            Id,
            All(&[Named("dialog_common"), Switch("type", true, TYPES)]),
        ]),
    },
    Row {
        name: "dialog_list",
        kind: Either(&[List(&Named("dialog"), 0, usize::MAX), Named("dialog"), Tag]),
    },
    Row {
        name: "dialog_common",
        kind: All(&[
            Object(&[
                req("title", Named("text")),
                opt("external_title", Named("text")),
                opt("can_close_with_escape", Bool),
                opt("pause", Bool),
                opt(
                    "after_action",
                    Enum(&["close", "none", "wait_for_response"]),
                ),
                opt(
                    "body",
                    Either(&[
                        List(&Switch("type", true, BODY), 0, usize::MAX),
                        Switch("type", true, BODY),
                    ]),
                ),
                opt("inputs", List(&Named("dialog_input"), 0, usize::MAX)),
            ]),
            NotBoolEnum("pause", true, "after_action", "none"),
        ]),
    },
    row(
        "dialog_button",
        &[
            req("label", Named("text")),
            opt("tooltip", Named("text")),
            opt("width", WIDTH),
            opt("action", Switch("type", true, ACTIONS)),
        ],
    ),
    Row {
        name: "dialog_plain",
        kind: Either(&[
            Object(&[req("contents", Named("text")), opt("width", WIDTH)]),
            Named("text"),
        ]),
    },
    Row {
        name: "dialog_input",
        kind: All(&[
            Object(&[req("key", VariableName)]),
            Switch("type", true, INPUTS),
        ]),
    },
    Row {
        name: "dialog_option",
        kind: Either(&[
            String(usize::MAX),
            Object(&[
                req("id", String(usize::MAX)),
                opt("display", Named("text")),
                opt("initial", Bool),
            ]),
        ]),
    },
];
