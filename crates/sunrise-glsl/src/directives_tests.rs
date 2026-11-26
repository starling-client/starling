//! Every way `sunrise-glsl`'s directives ([`run_directives`], with [`crate::condition`] and
//! [`crate::macros`]) read a stage differently from what read it before:
//! - a core shader's conditions went through the walk that pastes its includes, which expanded
//!   each condition's macros with naga's preprocessor (`pp-rs`, still naga's own) and evaluated
//!   it there ([`CONDITIONS`]' first column, `old_gating`); the rest of the stage, and every
//!   directive of a post shader, went to naga's preprocessor with the defines ([`old`]);
//! - now both run through [`run_directives`] and naga is given no define ([`new`]).
//!
//! Each row runs through each path, and the test checks the reading listed for every path.
//! [`CONDITIONS`] lists the `#if` arithmetic, [`STAGES`] the other directives; a row where the
//! paths differ says so.

use super::*;
use crate::located::Lines;
use naga::front::glsl::{Frontend, Options};

/// The walk's evaluation of a condition before this change, for core shaders' includes: the
/// condition's macros expanded by naga's preprocessor (`pp-rs`), then its `#if` arithmetic.
/// Copied from `import_conditions.rs` as it was, without its budget.
mod old_gating {
    use crate::condition::resolve_defined;
    use crate::macros::is_ident;
    use pp_rs::{pp::Preprocessor, token::TokenValue};
    use std::collections::{HashMap, HashSet};

    fn replace_ident(text: &str, from: &str, to: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(at) = rest.find(from) {
            let before = rest[..at].chars().next_back();
            let after = rest[at + from.len()..].chars().next();
            out.push_str(&rest[..at]);
            if before.is_some_and(is_ident) || after.is_some_and(is_ident) {
                out.push_str(from);
            } else {
                out.push_str(to);
            }
            rest = &rest[at + from.len()..];
        }
        out.push_str(rest);
        out
    }

    #[derive(Default)]
    pub(super) struct Conditions {
        macros: HashMap<String, String>,
    }

    impl Conditions {
        pub(super) fn new(defines: &[&str]) -> Self {
            let mut state = Self::default();
            for define in defines {
                state.define(define);
            }
            state
        }

        pub(super) fn define(&mut self, definition: &str) {
            let end = definition
                .find(|c| !is_ident(c))
                .unwrap_or(definition.len());
            let name = &definition[..end];
            self.macros.insert(name.to_owned(), definition.to_owned());
        }

        /// Object-like and function-like definitions can refer to later definitions. Resolve
        /// their names at use, leaving actual token expansion to the GLSL frontend.
        fn definitions<'a>(&'a self, text: &'a str) -> Vec<&'a str> {
            let mut pending = vec![text];
            let mut seen = HashSet::new();
            let mut found = Vec::new();
            while let Some(source) = pending.pop() {
                for name in source.split(|c| !is_ident(c)).filter(|s| !s.is_empty()) {
                    if let Some(definition) = self.macros.get(name)
                        && seen.insert(name)
                    {
                        found.push(definition.as_str());
                        pending.push(definition);
                    }
                }
            }
            found
        }

        pub(super) fn test(&mut self, kind: &str, rest: &str, line: u32) -> Result<bool, String> {
            match kind {
                "ifdef" | "ifndef" => {
                    if rest.is_empty() || !rest.chars().all(is_ident) {
                        return Err(format!("invalid #{kind} {rest}"));
                    }
                    let defined = rest == "__LINE__" || self.macros.contains_key(rest);
                    Ok(defined == (kind == "ifdef"))
                }
                _ => self.number(rest, line, false).map(|n| n != 0),
            }
        }

        fn number(
            &mut self,
            expression: &str,
            line: u32,
            line_directive: bool,
        ) -> Result<u32, String> {
            let expression = resolve_defined(expression, |name| {
                name == "__LINE__" || self.macros.contains_key(name)
            })?;
            let mut source = String::new();
            for definition in self.definitions(&expression) {
                source.push_str("#define ");
                source.push_str(definition);
                source.push('\n');
            }
            source.push_str(&expression);
            // Expand macros as ordinary GLSL tokens. In particular, function-like macros take
            // their arguments here before the conditional expression parser evaluates the result.
            let source = replace_ident(&source, "__LINE__", &line.to_string());
            let mut code = String::from(if line_directive { "#line " } else { "#if " });
            for token in Preprocessor::new(&source) {
                let token = token.map_err(|(why, _)| format!("GLSL directive: {why:?}"))?;
                let spelling = &source[token.location.start as usize..token.location.end as usize];
                code.push_str(spelling);
                code.push(' ');
            }
            if line_directive {
                code.push_str("\n__LINE__\n");
            } else {
                code.push_str("\n1\n#else\n0\n#endif\n");
            }
            match Preprocessor::new(&code).next() {
                Some(Ok(token)) => match token.value {
                    TokenValue::Integer(n) => n
                        .value
                        .try_into()
                        .map_err(|_| "line number outside GLSL range".into()),
                    _ => Err("expected an integer from a GLSL directive".into()),
                },
                Some(Err((why, _))) => Err(format!("GLSL directive: {why:?}")),
                None => Err("empty GLSL directive".into()),
            }
        }
    }
}

/// What a stage gives: the value of its constant `v`, or a refusal.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Read {
    V(i64),
    /// It parses with no `v`.
    NoV,
    Refused,
}
use Read::{NoV, Refused, V};

/// The value of the stage's `const int v`.
fn v(module: &naga::Module) -> Read {
    let Some((_, constant)) =
        (module.constants.iter()).find(|(_, c)| c.name.as_deref() == Some("v"))
    else {
        return NoV;
    };
    match module.global_expressions[constant.init] {
        naga::Expression::Literal(naga::Literal::I32(n)) => V(n.into()),
        naga::Expression::Literal(naga::Literal::U32(n)) => V(n.into()),
        ref other => panic!("v is {other:?}"),
    }
}

/// `body` after `#version 450`, with `defines`, through naga's preprocessor alone.
fn old(body: &str, defines: &[&str]) -> Read {
    let mut options = Options::from(naga::ShaderStage::Fragment);
    for define in defines {
        let (name, value) = define.split_once(' ').unwrap_or((define, ""));
        options.defines.insert(name.into(), value.into());
    }
    let text = format!("#version 450\n{body}\nvoid main() {{}}\n");
    Frontend::default()
        .parse(&options, &text)
        .map_or(Refused, |m| v(&m))
}

/// The same through [`run_directives`], then naga with no define.
fn new(body: &str, defines: &[&str]) -> Read {
    let text = format!("#version 450\n{body}\nvoid main() {{}}\n");
    let code = Expanded {
        lines: Lines::from_text(&text),
        files: vec!["t:shaders/t.fsh".into()],
        version: crate::stage_version(&text),
    };
    let Ok(code) = run_directives(&code, defines) else {
        return Refused;
    };
    Frontend::default()
        .parse(
            &glsl_options(naga::ShaderStage::Fragment, InterfaceLimits::WGPU_DEFAULT),
            code.text(),
        )
        .map_or(Refused, |m| v(&m))
}

/// `#if expr` choosing `v`: 1 when it holds, 2 when not.
fn condition(expr: &str) -> String {
    format!("#if {expr}\nconst int v = 1;\n#else\nconst int v = 2;\n#endif")
}

/// `expr` as the walk that gated core shaders' includes read it before (`old_gating`), at line
/// 3 after `prelude`'s definitions, with `defines`.
fn old_core(prelude: &str, expr: &str, defines: &[&str]) -> Read {
    let mut conditions = old_gating::Conditions::new(defines);
    for definition in prelude.lines().filter_map(|l| l.strip_prefix("#define ")) {
        conditions.define(definition);
    }
    match conditions.test("if", expr, 3) {
        Ok(true) => V(1),
        Ok(false) => V(2),
        Err(_) => Refused,
    }
}

/// `#if` conditions on line 3, after `prelude`'s line, with the defines `A 0.7`, `FLAG` and
/// `TWO 2`: how the walk that gates a core shader's includes read it before (core), how naga's
/// preprocessor read it in the code it parsed (a post shader's, and a core shader's code after
/// its includes had been pasted: post), and how `sunrise-glsl` reads it now (both). Rows where
/// the new reading differs from either old one are the listed differences.
const CONDITIONS: &[(&str, &str, [Read; 3])] = &[
    ("", "1", [V(1), V(1), V(1)]),
    ("", "0", [V(2), V(2), V(2)]),
    // Undefined names count as 0 in all three.
    ("", "UNDEFINED", [V(2), V(2), V(2)]),
    ("", "UNDEFINED == 0", [V(1), V(1), V(1)]),
    ("", "defined(FLAG) && defined TWO", [V(1), V(1), V(1)]),
    ("", "!defined NOPE", [V(1), V(1), V(1)]),
    ("", "1 + 2 * 3 == 7 && (1 + 2) * 3 == 9", [V(1), V(1), V(1)]),
    ("", "0x10 == 16 && 010 == 8 && 3u == 3", [V(1), V(1), V(1)]),
    ("", "-7 / 2 == -3 && -7 % 2 == -1", [V(1), V(1), V(1)]),
    (
        "",
        "~0 == -1 && 1 << 4 == 16 && !0 == 1",
        [V(1), V(1), V(1)],
    ),
    ("", "6 & 3 == 3", [V(2), V(2), V(2)]),
    ("", "1 || 0 && 0", [V(1), V(1), V(1)]),
    ("", "TWO", [V(1), V(1), V(1)]),
    ("", "__LINE__ == 3", [V(1), V(1), V(1)]),
    // A macro with more of the condition after it.
    ("", "TWO * TWO == 4", [V(1), Refused, V(1)]),
    (
        "#define SUM TWO + TWO",
        "SUM * 2 == 6",
        [V(1), Refused, V(1)],
    ),
    (
        "#define SQ(x) ((x) * (x))",
        "SQ(3) == 9",
        [V(1), Refused, V(1)],
    ),
    // Float literals.
    ("", "0.5", [Refused, Refused, V(1)]),
    ("", "1.5 > 1.0", [Refused, Refused, V(1)]),
    ("", "A > 0.5", [Refused, Refused, V(1)]),
    ("", "1e3 == 1000", [Refused, Refused, V(1)]),
    ("", "1 == 1.0", [Refused, Refused, V(1)]),
    // Division by zero, unless `&&` or `||` skips it.
    ("", "1 / 0", [Refused, Refused, Refused]),
    ("", "0 && 1 / 0", [Refused, Refused, V(2)]),
    ("", "1 || 1 / 0", [Refused, Refused, V(1)]),
    // Refused by all three.
    ("", "", [Refused, Refused, Refused]),
    ("", "1 2", [Refused, Refused, Refused]),
    ("", "(1", [Refused, Refused, Refused]),
    ("", "1 @ 2", [Refused, Refused, Refused]),
    ("", "defined", [Refused, Refused, Refused]),
    ("", "FLAG", [Refused, Refused, Refused]),
    ("", "1 ? 2 : 3", [Refused, Refused, Refused]),
    // Comparisons and big numbers.
    ("", "-1 < 0u", [V(1), V(1), V(1)]),
    ("", "4294967296 > 0", [V(1), V(1), V(1)]),
    ("#define HAS defined(FLAG)", "HAS", [Refused, V(1), V(1)]),
    // `__LINE__` counts as defined, as the walk read it; naga's preprocessor refused the name.
    ("", "defined(__LINE__)", [V(1), Refused, V(1)]),
];

#[test]
fn conditions_read_as_before_but_the_listed_ones() {
    let defines = ["A 0.7", "FLAG", "TWO 2"];
    let mut wrong = Vec::new();
    for &(prelude, expr, listed) in CONDITIONS {
        let body = format!("{prelude}\n{}", condition(expr));
        let got = [
            old_core(prelude, expr, &defines),
            old(&body, &defines),
            new(&body, &defines),
        ];
        if got != listed {
            wrong.push(format!("#if {expr}: {got:?}, listed as {listed:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Whether a [`STAGES`] row differs between the paths.
const SAME: bool = false;
const DIFFERS: bool = true;

/// Whole stages: naga's preprocessor's reading (a post shader's, and a core shader's code once
/// its includes were pasted), then `sunrise-glsl`'s. The walk that gated a core shader's
/// includes took `#else` and `#endif` with words after them and `#ifdef __LINE__` (defined) as
/// `sunrise-glsl` does now, which this change leaves alone: those two rows differ for post
/// shaders only.
const STAGES: &[(&str, &str, Read, Read, bool)] = &[
    (
        "nested #elif",
        "#ifdef FLAG\n#if TWO > 2\nconst int v = 1;\n#elif TWO == 2\nconst int v = 3;\n#else\nconst int v = 4;\n#endif\n#else\nconst int v = 5;\n#endif",
        V(3),
        V(3),
        SAME,
    ),
    (
        "an #elif after the taken arm isn't evaluated",
        "#if 1\nconst int v = 1;\n#elif 1 / 0\nconst int v = 2;\n#endif",
        V(1),
        V(1),
        SAME,
    ),
    (
        "macros of both kinds, nested",
        "#define ADD(a, b) ((a) + (b))\n#define THREE ADD(1, TWO)\nconst int v = ADD(THREE, TWO);",
        V(5),
        V(5),
        SAME,
    ),
    (
        "a call over lines",
        "#define ADD(a, b) ((a) + (b))\nconst int v = ADD(1,\n    2);",
        V(3),
        V(3),
        SAME,
    ),
    (
        "#undef",
        "#define N 1\n#undef N\n#define N 7\nconst int v = N;",
        V(7),
        V(7),
        SAME,
    ),
    ("__LINE__", "\n\nconst int v = __LINE__;", V(4), V(4), SAME),
    (
        "#line",
        "#line 20\nconst int v = __LINE__;",
        V(21),
        V(21),
        SAME,
    ),
    (
        "#line with a source number",
        "#line 20 3\nconst int v = __LINE__;",
        V(21),
        V(21),
        SAME,
    ),
    (
        "a comment in a definition",
        "#define N 4 // four\nconst int v = N + 1;",
        V(5),
        V(5),
        SAME,
    ),
    (
        "a continued definition",
        "#define N 4 + \\\n  2\nconst int v = N;",
        V(6),
        V(6),
        SAME,
    ),
    (
        "the same definition again",
        "#define N 4\n#define N 4\nconst int v = N;",
        V(4),
        V(4),
        SAME,
    ),
    (
        "another definition of the same name",
        "#define N 4\n#define N 5\nconst int v = N;",
        Refused,
        Refused,
        SAME,
    ),
    (
        "a define given again",
        "#define TWO 3\nconst int v = TWO;",
        Refused,
        Refused,
        SAME,
    ),
    (
        "## pastes",
        "#define CAT(a, b) a ## b\nconst int v = CAT(1, 2);",
        Refused,
        V(12),
        DIFFERS,
    ),
    (
        "a call with too many arguments",
        "#define ONE(a) a\nconst int v = ONE(1, 2);",
        Refused,
        Refused,
        SAME,
    ),
    (
        "a call with too few arguments",
        "#define BOTH(a, b) a + b\nconst int v = BOTH(1);",
        Refused,
        Refused,
        SAME,
    ),
    (
        "a parameter named twice",
        "#define TWICE(a, a) a\nconst int v = 1;",
        Refused,
        Refused,
        SAME,
    ),
    (
        "words after #else and #endif",
        "#if 0\nconst int v = 1;\n#else junk\nconst int v = 2;\n#endif junk",
        Refused,
        V(2),
        DIFFERS,
    ),
    (
        "#ifdef __LINE__",
        "#ifdef __LINE__\nconst int v = 1;\n#else\nconst int v = 2;\n#endif",
        V(2),
        V(1),
        DIFFERS,
    ),
    (
        "__LINE__ in a call's arguments over lines: each its own line",
        "#define PAIR(a, b) a * 100 + b\nconst int v = PAIR(__LINE__,\n__LINE__);",
        V(304),
        V(304),
        SAME,
    ),
    (
        "__LINE__ in a replacement: the call's last line",
        "#define L(x) __LINE__\nconst int v = L(1\n);",
        V(4),
        V(4),
        SAME,
    ),
    (
        "__LINE__ in an object-like replacement before a call over lines",
        "#define L __LINE__\n#define ID(a) a\nconst int v = L + ID(\n0);",
        V(4),
        V(5),
        DIFFERS,
    ),
    (
        "__LINE__ in a replacement of a call closed before a call over lines",
        "#define L(x) __LINE__\n#define ID(a) a\nconst int v = L(0) * 100 + ID(\n0);",
        V(400),
        V(500),
        DIFFERS,
    ),
    (
        "#define __LINE__",
        "#define __LINE__ 5\nconst int v = __LINE__;",
        V(3),
        V(3),
        SAME,
    ),
    (
        "#define __LINE__ again otherwise",
        "#define __LINE__ 5\n#define __LINE__ 6\nconst int v = 1;",
        Refused,
        Refused,
        SAME,
    ),
    (
        "#define with a name starting with a digit",
        "#define 1A 2\nconst int v = 1;",
        Refused,
        Refused,
        SAME,
    ),
    (
        "#undef with a name starting with a digit",
        "#undef 1A\nconst int v = 1;",
        Refused,
        Refused,
        SAME,
    ),
    (
        "#ifdef with a name starting with a digit",
        "#ifdef 1A\n#endif\nconst int v = 1;",
        Refused,
        Refused,
        SAME,
    ),
    // The same refusals.
    (
        "#ifdef with two words",
        "#ifdef FLAG TWO\nconst int v = 1;\n#endif",
        Refused,
        Refused,
        SAME,
    ),
    (
        "#undef with two words",
        "#undef FLAG TWO\nconst int v = 1;",
        Refused,
        Refused,
        SAME,
    ),
    (
        "#if left open",
        "#if 1\nconst int v = 1;",
        Refused,
        Refused,
        SAME,
    ),
    (
        "#endif with no #if",
        "const int v = 1;\n#endif",
        Refused,
        Refused,
        SAME,
    ),
    (
        "#else after #else",
        "#if 0\n#else\nconst int v = 1;\n#else\n#endif",
        Refused,
        Refused,
        SAME,
    ),
    (
        "#error",
        "const int v = 1;\n#error stop",
        Refused,
        Refused,
        SAME,
    ),
    (
        "an unknown directive",
        "const int v = 1;\n#bogus",
        Refused,
        Refused,
        SAME,
    ),
    (
        "#error in a skipped branch",
        "#if 0\n#error stop\n#endif\nconst int v = 1;",
        V(1),
        V(1),
        SAME,
    ),
];

#[test]
fn stages_read_as_naga_did_but_the_listed_ones() {
    let defines = ["A 0.7", "FLAG", "TWO 2"];
    let mut wrong = Vec::new();
    for &(name, body, was, is, _) in STAGES {
        let got = (old(body, &defines), new(body, &defines));
        if got != (was, is) {
            wrong.push(format!("{name}: {got:?}, listed as {:?}", (was, is)));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    for &(name, _, was, is, differs) in STAGES {
        assert_eq!(was != is, differs, "{name}");
    }
}

/// A program whose vertex shader is `vsh`, its includes `files`, through the core translation.
fn program(vsh: &str, files: &[(&str, &str)]) -> Result<Translated, Vec<Error>> {
    const GLUE: Glue = Glue {
        inputs: &[("Position", "vec3")],
        prelude: "layout(location = 0) in vec3 hz_position;",
        setup: "Position = hz_position;",
        finish: "",
        vertex_id: "int(gl_VertexIndex)",
        fragment_finish: None,
        interface: InterfaceLimits::WGPU_DEFAULT,
    };
    let read = |id: &str| -> Result<Option<String>, String> {
        Ok(files
            .iter()
            .find(|(file, _)| *file == id)
            .map(|(_, text)| (*text).to_owned()))
    };
    let stages = Stages {
        vertex_dir: "minecraft:shaders/core/",
        fragment_dir: "minecraft:shaders/core/",
        vertex: vsh,
        fragment: "#version 330\nout vec4 c;\nvoid main() { c = vec4(1.0); }\n",
        vertex_file: tests::VSH_FILE,
        fragment_file: tests::FSH_FILE,
    };
    tests::translate_core_at(&stages, &["FADE 0.5"], &read, &GLUE, &|_| false)
}

const MAIN: &str =
    "in vec3 Position;\nvoid main() { gl_Position = vec4(Position * SCALE, 1.0); }\n";

/// A directive the directives' run refuses inside an include names that file and the line the
/// author wrote it on, past a comment over two lines and a continued line, and is the stage's
/// one error (naga's frontend went on past it); one the walk that pastes the includes refuses
/// names its file and line too.
#[test]
fn a_directive_error_in_an_include_names_that_file_and_line() {
    let inner = "/* over\n   lines */\nfloat spliced = \\\n    1.0;\n#undef SCALE TWICE\n";
    let files = [
        (
            "minecraft:shaders/include/outer.glsl",
            "#define SCALE 2.0\n#include \"inner.glsl\"\n",
        ),
        ("minecraft:shaders/include/inner.glsl", inner),
    ];
    let vsh = format!("#version 330\n#include <minecraft:outer.glsl>\n{MAIN}");
    let errors = program(&vsh, &files).unwrap_err();
    let at: Vec<_> = errors.iter().map(|e| (e.file.as_str(), e.line)).collect();
    assert_eq!(at, [("minecraft:shaders/include/inner.glsl", 5)]);
    let gated = "#define SCALE 2.0\n\n#if SCALE @ 1\n#endif\n";
    let files = [("minecraft:shaders/include/outer.glsl", gated)];
    let errors = program(&vsh, &files).unwrap_err();
    let at: Vec<_> = errors.iter().map(|e| (e.file.as_str(), e.line)).collect();
    assert_eq!(at, [("minecraft:shaders/include/outer.glsl", 3)]);
}

/// An include under a condition with a float, which naga's preprocessor refused, is read when the
/// condition holds and left when not: the one behaviour the move changes for a working stage.
#[test]
fn a_float_condition_gates_an_include() {
    let files = [
        (
            "minecraft:shaders/include/scale.glsl",
            "#define SCALE 2.0\n",
        ),
        ("minecraft:shaders/include/absent.glsl", "not glsl\n"),
    ];
    let vsh = format!(
        "#version 330\n#if FADE > 0.25\n#include <minecraft:scale.glsl>\n#else\n\
         #include <minecraft:absent.glsl>\n#endif\n{MAIN}"
    );
    program(&vsh, &files).unwrap();
}

/// Macros that double at every level are refused once their expansion has spent the work the
/// translator reads for a stage, at the line that expands them: the directives expand each line
/// whole before naga reads it (naga's preprocessor handed naga one token at a time, and naga
/// stopped at the first it couldn't take). The stop has no token position, so its column is unknown.
#[test]
#[ignore = "daily"]
fn an_expansion_past_the_budget_is_refused_where_it_is() {
    let mut defines = String::from("#define M0 x\n");
    for n in 1..40 {
        defines.push_str(&format!("#define M{n} M{} M{}\n", n - 1, n - 1));
    }
    let vsh = format!("#version 330\n{defines}float M39;\n{MAIN}");
    let errors = program(&vsh, &[]).unwrap_err();
    let at: Vec<_> = errors.iter().map(|e| (e.file.as_str(), e.line)).collect();
    assert_eq!(at, [(tests::VSH_FILE, 42)]);
    let refused = Error {
        file: tests::VSH_FILE.into(),
        line: 42,
        column: 0,
        stage: Some(ShaderStage::Vertex),
        unprovided: None,
        message: directives::stopped(crate::macros::Stop::Work),
    };
    assert_eq!(errors, [refused]);
}

/// A post shader's directives are read after pasting its includes. Refusals keep the include's
/// authored line and byte column of the directive, past comments and across splices.
#[test]
fn a_post_shaders_directive_errors_name_the_include_and_position() {
    let run = |include: &str| {
        let read = |id: &str| -> Result<Option<String>, String> {
            Ok((id == "p:shaders/include/lib.glsl").then(|| include.to_owned()))
        };
        let includes = Includes {
            read: &read,
            namespace: "p",
            folder: "shaders/include/",
            once: &[],
        };
        let fsh = "#version 330\n#include <lib.glsl>\nvoid main() {}\n";
        let code = expand_includes(
            fsh,
            "p:shaders/post/a.fsh",
            "p:shaders/post/",
            &includes,
            None,
        )
        .unwrap();
        run_directives(&code, &["DEPTH"]).unwrap_err()
    };
    for (include, line, column) in [
        ("float a;\n\t/*é*/ #if 1\n#ifdef X\n#endif\n", 2, 9),
        ("// first\n/* a\n   b */\n#define DEPTH 1\n", 4, 1),
        ("\t/*é*/ #undef X Y\n", 1, 9),
        ("\t/*é*/ \\\n  #undef X Y\n", 2, 3),
    ] {
        let error = run(include);
        assert_eq!(
            (error.file.as_str(), error.line, error.column),
            ("p:shaders/include/lib.glsl", line, column)
        );
    }
}

/// Wrong arity points to the invocation, whether the join probe or the actual expansion stops.
#[test]
fn wrong_arity_names_the_authored_call() {
    const FILE: &str = "p:shaders/post/a.fsh";
    for (text, line, column) in [
        ("#define F(x) x\n\tfloat x = F(1, 2);\n", 2, 12),
        // No upcoming line to probe.
        ("#define F(x) x\n\tfloat x = F(1, 2);", 2, 12),
        // The probe sees the call only after joining a CRLF and a spliced line. An earlier F
        // is valid; the bad call's name comes from ALIAS and points there.
        (
            concat!(
                "#define F(x) x\n#define OUTER(x) x\n#define ALIAS F\n",
                "\tfloat x = OUTER(\r\n  F(0) + \\\r\n \tALIAS(1, 2));\r\n",
                "float y;\n",
            ),
            6,
            3,
        ),
        // The next directive ends probing before the joined call is checked. The actual
        // expansion's offsets must account for the shorter spelling of __LINE__.
        (
            concat!(
                "#define F(x) x\n#define OUTER(x) x\n",
                "\tfloat x = OUTER(\r\n  __LINE__ + \\\r\n \tF(1, 2));\r\n",
                "#define NEXT 1\n",
            ),
            5,
            3,
        ),
    ] {
        let code = Expanded {
            lines: Lines::from_text(text),
            files: vec![FILE.into()],
            version: 110,
        };
        let error = run_directives(&code, &[]).unwrap_err();
        assert_eq!(
            (error.file.as_str(), error.line, error.column),
            (FILE, line, column),
            "{text:?}"
        );
        assert_eq!(
            error.message,
            "`F` called with a number of arguments it doesn't take"
        );
    }
}

/// Overflow of the logical line counter reports the physical line, not an unrelated token.
#[test]
fn a_line_counter_overflow_does_not_invent_a_column() {
    let code = Expanded {
        lines: Lines::from_text("#define N __LINE__\n#line 4294967295\n\tfloat x = N;\n"),
        files: vec!["p:shaders/post/a.fsh".into()],
        version: 110,
    };
    let error = run_directives(&code, &[]).unwrap_err();
    assert_eq!(
        error,
        Error::new("p:shaders/post/a.fsh", 3, "line number outside GLSL range")
    );
}

/// An error on the line after a call over lines names its own line: the joined lines keep theirs.
#[test]
fn an_error_after_a_call_over_lines_names_its_own_line() {
    let vsh = format!(
        "#version 330\n#define ADD(a, b) ((a) + (b))\nfloat x = ADD(1.0,\n    2.0);\n\
         float y = undefined_name;\n#define SCALE 1.0\n{MAIN}"
    );
    let errors = program(&vsh, &[]).unwrap_err();
    let at: Vec<_> = errors.iter().map(|e| (e.file.as_str(), e.line)).collect();
    assert_eq!(at[0], (tests::VSH_FILE, 5), "{errors:?}");
}

/// A long line of macros and a call over many lines, each far below the expansion's work,
/// translate: the work is what the expansion makes, not the line's length for every macro.
#[test]
fn long_lines_and_calls_over_lines_fit_the_work() {
    let decls = |from: usize, count: usize| -> Vec<String> {
        (from..from + count)
            .map(|n| format!("float s{n} = W;"))
            .collect()
    };
    let line = decls(0, 2050).join(" ");
    let vsh = format!("#version 330\n#define W 1.0\n#define SCALE 1.0\n{line}\n{MAIN}");
    program(&vsh, &[]).unwrap();
    let rows: Vec<String> = (0..60).map(|r| decls(r * 10, 10).join(" ")).collect();
    let vsh = format!(
        "#version 330\n#define W 1.0\n#define SCALE 1.0\n#define KEEP(a) a\n\
         KEEP({})\n{MAIN}",
        rows.join("\n    ")
    );
    program(&vsh, &[]).unwrap();
}

/// Calls nested in calls' arguments to `MAX_ARGUMENT_DEPTH` expand on a thread of half the
/// 2 MiB Rust gives a spawned thread, in this (debug) build; one level more is refused, by
/// name, at its line.
#[test]
fn calls_nested_to_the_bound_fit_a_small_thread() {
    let nested = |depth: usize| {
        format!(
            "#version 330\n#define F(x) x\nconst int v = {}1{};\n",
            "F(".repeat(depth),
            ")".repeat(depth)
        )
    };
    let run = |text: String| {
        std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || {
                let code = Expanded {
                    lines: Lines::from_text(&text),
                    files: vec!["t:shaders/t.vsh".into()],
                    version: crate::stage_version(&text),
                };
                run_directives(&code, &[]).map(|c| c.text().to_owned())
            })
            .unwrap()
            .join()
            .unwrap()
    };
    let deepest = run(nested(crate::macros::MAX_ARGUMENT_DEPTH)).unwrap();
    assert!(deepest.contains("const int v = 1;"), "{deepest}");
    let error = run(nested(crate::macros::MAX_ARGUMENT_DEPTH + 1)).unwrap_err();
    assert_eq!((error.file.as_str(), error.line), ("t:shaders/t.vsh", 3));
    assert_eq!(
        error.message,
        directives::stopped(crate::macros::Stop::Depth)
    );
}

/// `(` and `)` around `value`, `depth` deep.
fn parens(depth: usize, value: &str) -> String {
    format!("{}{value}{}", "(".repeat(depth), ")".repeat(depth))
}

/// `run` on a thread of half the 2 MiB Rust gives a spawned thread.
fn on_small_thread<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap()
}

/// `#if`, `#elif` and `#line` nested to `MAX_CONDITION_DEPTH` run on a thread of half the 2 MiB
/// Rust gives a spawned thread, in this (debug) build; one level more is refused by name, at the
/// directive's file and line.
#[test]
fn conditions_nested_to_the_bound_fit_a_small_thread() {
    use crate::condition::{MAX_CONDITION_DEPTH, too_deep};
    let at = MAX_CONDITION_DEPTH;
    let run = |text: String| {
        on_small_thread(move || {
            let code = Expanded {
                lines: Lines::from_text(&text),
                files: vec!["t:shaders/t.vsh".into()],
                version: crate::stage_version(&text),
            };
            run_directives(&code, &[]).map(|c| c.text().to_owned())
        })
    };
    let deepest = run(format!(
        "#version 330\n#if {}\nfloat kept;\n#endif\n#if 0\n#elif {}\nfloat elif_kept;\n#endif\n\
         #line {}\nconst int l = __LINE__;\n",
        parens(at, "1"),
        parens(at, "2"),
        parens(at, "40"),
    ))
    .unwrap();
    // The directive's own line takes the number, as naga's preprocessor numbered it.
    for kept in ["float kept;", "float elif_kept;", "const int l = 41u;"] {
        assert!(deepest.contains(kept), "{kept}: {deepest}");
    }
    let past = parens(at + 1, "1");
    let condition = format!("`#if {past}`: {}", too_deep());
    let cases = [
        (format!("#version 330\n#if {past}\n#endif\n"), 2, &condition),
        (
            format!("#version 330\n#if 0\n#elif {past}\n#endif\n"),
            3,
            &condition,
        ),
        (format!("#version 330\n\n\n#line {past}\n"), 4, &too_deep()),
    ];
    for (text, line, message) in cases {
        let error = run(text).unwrap_err();
        assert_eq!(
            (error.file.as_str(), error.line, &error.message),
            ("t:shaders/t.vsh", line, message)
        );
    }
}

/// The walk that pastes a core shader's includes tests their conditions inside its own
/// recursion: an `#if` and a `#line` nested to `MAX_CONDITION_DEPTH` in an include sixteen deep
/// (the most the walk takes) run on a thread of half the 2 MiB Rust gives a spawned thread; one
/// level more is refused by name, at that include's file and line.
#[test]
fn an_include_sixteen_deep_tests_a_condition_at_the_bound_on_a_small_thread() {
    use crate::condition::{MAX_CONDITION_DEPTH, too_deep};
    let walk = |deepest: String| {
        on_small_thread(move || {
            let read = move |id: &str| -> Result<Option<String>, String> {
                let n: usize = id
                    .strip_prefix("minecraft:shaders/include/i")
                    .and_then(|r| r.strip_suffix(".glsl"))
                    .and_then(|n| n.parse().ok())
                    .ok_or(format!("{id} not a test file"))?;
                Ok(Some(match n {
                    16 => deepest.clone(),
                    _ => format!("#include <i{}.glsl>\n", n + 1),
                }))
            };
            let includes = crate::Includes {
                read: &read,
                namespace: "minecraft",
                folder: "shaders/include/",
                once: &["moj_import"],
            };
            let vsh = "#version 330\n#include <i1.glsl>\n#ifdef DEEPEST\nfloat deepest;\n#endif\n";
            crate::expand_includes(
                vsh,
                tests::VSH_FILE,
                "minecraft:shaders/core/",
                &includes,
                Some(&[]),
            )
            .map(|e| e.text().to_owned())
        })
    };
    let condition =
        |depth: usize| format!("\n#if {}\n#define DEEPEST 1\n#endif\n", parens(depth, "3"));
    let line = |depth: usize| format!("\n#line {}\nfloat after_line;\n", parens(depth, "70"));
    let text = walk(condition(MAX_CONDITION_DEPTH)).unwrap();
    assert!(text.contains("float deepest;"), "{text}");
    let text = walk(line(MAX_CONDITION_DEPTH)).unwrap();
    assert!(text.contains("float after_line;"), "{text}");
    let refused = |why: String| crate::ImportError {
        file: "minecraft:shaders/include/i16.glsl".into(),
        line: 2,
        refusal: crate::ImportRefusal::Directive(why),
    };
    let past = parens(MAX_CONDITION_DEPTH + 1, "3");
    assert_eq!(
        walk(condition(MAX_CONDITION_DEPTH + 1)).unwrap_err(),
        refused(format!("`#if {past}`: {}", too_deep()))
    );
    assert_eq!(
        walk(line(MAX_CONDITION_DEPTH + 1)).unwrap_err(),
        refused(too_deep())
    );
}

/// A `defined` a macro's replacement makes is read in a condition, a post shader's as naga's
/// preprocessor read it, and now a core shader's too (the walk refused it before).
#[test]
fn a_defined_made_by_a_macro_is_read() {
    let fsh = "#version 330\n#define HAS defined(DEPTH)\n#if HAS\nfloat kept;\n#else\nfloat dropped;\n#endif\n";
    let code = Expanded {
        lines: Lines::from_text(fsh),
        files: vec!["p:shaders/post/a.fsh".into()],
        version: crate::stage_version(fsh),
    };
    let text = run_directives(&code, &["DEPTH"]).unwrap().text().to_owned();
    assert!(
        text.contains("float kept;") && !text.contains("dropped"),
        "{text}"
    );
    let files = [(
        "minecraft:shaders/include/scale.glsl",
        "#define SCALE 2.0\n",
    )];
    let vsh = format!(
        "#version 330\n#define HAS defined(FADE)\n#if HAS\n#include <minecraft:scale.glsl>\n#endif\n{MAIN}"
    );
    program(&vsh, &files).unwrap();
}

/// The value of `module`'s constant `name`.
fn constant(module: &naga::Module, name: &str) -> i64 {
    let (_, c) = (module.constants.iter())
        .find(|(_, c)| c.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no constant {name}"));
    match module.global_expressions[c.init] {
        naga::Expression::Literal(naga::Literal::I32(n)) => n.into(),
        ref other => panic!("{name} is {other:?}"),
    }
}

/// The version a stage file declares, as `__VERSION__` reads it: the first `#version`'s number,
/// past comments and through a continued line, whatever follows the number; 110 without one.
#[test]
fn the_stage_version_is_its_first_version_line() {
    let cases: &[(&str, u32)] = &[
        ("#version 330\n", 330),
        ("#version 150 core\nvoid main() {}\n", 150),
        ("#version 300 es\n", 300),
        ("  #  version 140\n", 140),
        ("// #version 100\n/* #version 120\n */#version 130\n", 130),
        ("#version \\\n 410\n", 410),
        ("#version 430\n#version 440\n", 430),
        ("#if 0\n#version 420\n#endif\n", 420),
        ("void main() {}\n", 110),
        ("#version\n", 110),
        ("#version abc\n", 110),
        ("#version +330\n", 110),
        ("#version 99999999999\n", 110),
    ];
    for &(text, version) in cases {
        assert_eq!(crate::stage_version(text), version, "{text:?}");
    }
}

/// A core shader's `__VERSION__` is its file's `#version` in the condition that gates an include,
/// in another condition, in a macro's replacement and in its code; an include's `#version` is
/// not the stage's.
#[test]
fn a_core_shader_reads_its_version() {
    let files = [(
        "minecraft:shaders/include/scale.glsl",
        "#version 400\n#define SCALE 2.0\n",
    )];
    let vsh = format!(
        "#version 150\n#if __VERSION__ == 150\n#include <minecraft:scale.glsl>\n#endif\n\
         #ifdef __VERSION__\nconst int seen = __VERSION__;\n#endif\n\
         #define NEXT (__VERSION__ + 1)\nconst int next = NEXT;\n\
         #if defined(__VERSION__) && __VERSION__ >= 150\nconst int above = 3;\n#endif\n{MAIN}"
    );
    let program = program(&vsh, &files).unwrap();
    let read: Vec<i64> = ["seen", "next", "above"]
        .map(|name| constant(&program.vertex, name))
        .into();
    assert_eq!(read, [150, 151, 3]);
}

/// A stage without `#version`, or whose `#version` has no number, reads `__VERSION__` as 110
/// (GLSL 4.60 §3.3), on the walk that pastes includes and in its code alike.
#[test]
fn a_stage_without_version_reads_110() {
    let files = [(
        "minecraft:shaders/include/scale.glsl",
        "#define SCALE 2.0\n",
    )];
    for header in ["", "#version abc\n"] {
        let vsh = format!(
            "{header}#if __VERSION__ == 110\n#include <minecraft:scale.glsl>\n#endif\n\
             const int seen = __VERSION__;\n{MAIN}"
        );
        let program = program(&vsh, &files).unwrap();
        assert_eq!(constant(&program.vertex, "seen"), 110, "{header:?}");
    }
}

/// An include the stage pastes, holding no `#version` of its own: its `#if __VERSION__` gates
/// the include that defines what the stage uses, and its `#elif`, `#ifndef` and code read the
/// stage's version (`post_glsl`'s test has the same include).
const VERSION_INCLUDE: &str = "#if __VERSION__ == 150\n#include \"scale.glsl\"\n#endif\n\
     const int a = __VERSION__;\n\
     #if __VERSION__ == 100\nconst int e = 1;\n#elif __VERSION__ == 150\nconst int e = 2;\n#endif\n\
     #ifndef __VERSION__\nconst int n = 1;\n#else\nconst int n = 4;\n#endif\n";

/// A core shader's include reads the stage's `__VERSION__` in a condition that gates another
/// include, in `#elif`, in `#ifndef` and in its code.
#[test]
fn a_core_shaders_include_reads_the_stage_version() {
    let files = [
        ("minecraft:shaders/include/a.glsl", VERSION_INCLUDE),
        (
            "minecraft:shaders/include/scale.glsl",
            "#define SCALE 2.0\n",
        ),
    ];
    let vsh = format!("#version 150\n#include <minecraft:a.glsl>\n{MAIN}");
    let program = program(&vsh, &files).unwrap();
    let read: Vec<i64> = ["a", "e", "n"]
        .map(|name| constant(&program.vertex, name))
        .into();
    assert_eq!(read, [150, 2, 4]);
}

/// A stage's own `#define __VERSION__` replaces the version, as the name was the stage's to define
/// before (naga's preprocessor had none); defining it again otherwise is still refused, and an
/// `#undef` removes it.
#[test]
fn a_stage_may_define_and_undefine_version() {
    let run = |body: &str| new(body, &[]);
    assert_eq!(run("const int v = __VERSION__;"), V(450));
    assert_eq!(
        run("#define __VERSION__ 100\nconst int v = __VERSION__;"),
        V(100)
    );
    assert_eq!(
        run("#define __VERSION__ 100\n#define __VERSION__ 100\nconst int v = __VERSION__;"),
        V(100)
    );
    assert_eq!(
        run("#define __VERSION__ 100\n#define __VERSION__ 120\nconst int v = __VERSION__;"),
        Refused
    );
    assert_eq!(
        run(&format!(
            "#undef __VERSION__\n{}",
            condition("defined(__VERSION__)")
        )),
        V(2)
    );
    assert_eq!(
        run("#undef __VERSION__\nconst int v = __VERSION__;"),
        Refused
    );
}
