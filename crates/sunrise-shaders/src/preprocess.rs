//! The GLSL preprocessor a program's stage file goes through before translation (spec 1.4,
//! 1.5, 1.6): comments removed, `#include` expanded, conditionals evaluated, macros expanded.
//! The macros and the `#if` arithmetic are `sunrise-glsl`'s ([`sunrise_glsl::macros`],
//! [`sunrise_glsl::condition`]); the pack's files, its includes and how Iris treats a bad
//! directive are this module's.
//! Every output line remembers the pack file and line it came from, so errors found later point
//! at the pack's own code.
//!
//! `#include` is expanded where it stands in live code only: an include inside a false `#if`
//! is not read, so it can't fail (OQ-14). Ten levels deep at most, which also stops a file that
//! includes itself. `#version` and `#extension` are taken out of the code and kept apart: the
//! translation writes its own header.
//!
//! The macros of a stage file, its includes and all, spend at most
//! [`MAX_EXPANSION_WORK`](crate::macros::MAX_EXPANSION_WORK) (#4135): a line that would spend more
//! refuses the program, an `#if` that would reads as false, both noted at their file and line.

use std::collections::HashMap;

use crate::Macros;
use crate::Pack;
use crate::condition::{self, Conditionals, split_directive};
use crate::pack::normalize;

const MAX_DEPTH: usize = 10;

/// Where a line came from: an index into [`Preprocessed::files`] and a line number from 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Origin {
    pub file: u16,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub origin: Origin,
    pub message: String,
}

/// A stage's code with where each of its lines came from.
///
/// `code`, `lines` and `files` are private and stay parallel: `lines` has one [`Origin`] per line
/// of `code` (`code.split('\n')`) and every origin's file indexes `files`. [`Preprocessed::new`]
/// and [`Preprocessed::from_source`] are how a stage is built by hand, [`preprocess`] builds the
/// pack's, and [`Preprocessed::with_code`] and [`Preprocessed::with_declaration`] derive one from
/// another. The one exception is the stage of a file that could not be read, the
/// [`Default`]: no code, no lines, no files, and its diagnostic.
#[derive(Clone, Debug, Default)]
pub struct Preprocessed {
    code: String,
    lines: Vec<Origin>,
    files: Vec<String>,
    /// The stage's `#version`, the first one read (an included file's too): the number and the
    /// profile word, if any.
    pub version: Option<(u32, Option<String>)>,
    /// `#extension` lines of live code, as written after `#extension`.
    pub extensions: Vec<String>,
    /// The text of every block comment in live code (`DRAWBUFFERS:`, buffer formats).
    pub comments: Vec<(Origin, String)>,
    pub diagnostics: Vec<Diagnostic>,
    /// Conditions that could not be evaluated and read as false: noted, the program still runs.
    pub warnings: Vec<Diagnostic>,
}

impl Preprocessed {
    /// A stage of `code` whose line `n` (from 0) came from `lines[n]`, a line of `files`' file.
    /// Refuses a `lines` that is not one origin per line of `code`, or an origin whose file is
    /// not in `files`.
    pub fn new(code: String, lines: Vec<Origin>, files: Vec<String>) -> Result<Self, String> {
        let expected = code.split('\n').count();
        if lines.len() != expected {
            return Err(format!(
                "{} line origins for {expected} lines of code",
                lines.len()
            ));
        }
        if let Some(origin) = lines.iter().find(|o| o.file as usize >= files.len()) {
            return Err(format!(
                "a line of file {} when there are {} files",
                origin.file,
                files.len()
            ));
        }
        Ok(Self {
            code,
            lines,
            files,
            ..Self::default()
        })
    }

    /// A stage of `code` read whole from the one file `file`: line `n` of the code is line `n` of
    /// the file.
    pub fn from_source(file: &str, code: &str) -> Self {
        Self {
            code: code.to_owned(),
            lines: code
                .split('\n')
                .zip(1u32..)
                .map(|(_, line)| Origin { file: 0, line })
                .collect(),
            files: vec![file.to_owned()],
            ..Self::default()
        }
    }

    /// The code, one line per entry of [`Preprocessed::lines`].
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Where each line of the code came from.
    pub fn lines(&self) -> &[Origin] {
        &self.lines
    }

    /// The pack paths of the files read, the stage file first.
    pub fn files(&self) -> &[String] {
        &self.files
    }

    /// This stage with `code` in place of its own, for a rewrite that keeps its lines where they
    /// are. A line the rewrite added at the end gets [`Origin::default`], and one it removed
    /// loses its origin.
    pub(crate) fn with_code(&self, code: String) -> Self {
        let mut out = self.clone();
        out.lines
            .resize(code.split('\n').count(), Origin::default());
        out.code = code;
        out
    }

    /// This stage with `declaration` after its leading directives (`#version`, `#extension`...).
    /// The inserted lines are Sunrise's own, so they get [`Origin::default`] and every later line
    /// keeps the pack file and line it came from.
    pub(crate) fn with_declaration(&self, declaration: &str) -> Self {
        let mut at = 0;
        for line in self.code.split_inclusive('\n') {
            let t = line.trim_start();
            if !(t.is_empty() || t.starts_with('#')) {
                break;
            }
            at += line.len();
        }
        let first = self.code[..at].matches('\n').count();
        let added = declaration.matches('\n').count() + 1;
        let mut out = self.clone();
        out.code = format!("{}{declaration}\n{}", &self.code[..at], &self.code[at..]);
        out.lines
            .splice(first..first, std::iter::repeat_n(Origin::default(), added));
        out
    }

    /// `file:line` of an output line (from 0), for messages.
    pub fn locate(&self, line: usize) -> String {
        let none = (self.file_of(Origin::default()), 0);
        let (file, line) = self.origin_at(line).unwrap_or(none);
        format!("{file}:{line}")
    }

    /// The pack file an origin names: `"?"` for an index outside [`Preprocessed::files`], which
    /// only an origin of the [`Default`] stage (no files) has.
    pub(crate) fn file_of(&self, origin: Origin) -> &str {
        self.files
            .get(origin.file as usize)
            .map_or("?", String::as_str)
    }

    /// The pack file and line output line `line` (from 0) came from; `None` past the last line.
    /// The one lookup from the code's lines to the pack that [`Preprocessed::locate`],
    /// [`Preprocessed::error_at_line`] and the compile step share.
    pub(crate) fn origin_at(&self, line: usize) -> Option<(&str, u32)> {
        self.lines.get(line).map(|o| (self.file_of(*o), o.line))
    }

    /// The stage's own file: the first of [`Preprocessed::files`]. [`preprocess`] always reads it
    /// first; only a stage file that could not be read (refused by its diagnostic before any
    /// translation) has none, and then the name is empty.
    pub(crate) fn file(&self) -> &str {
        self.files.first().map_or("", String::as_str)
    }

    /// An error at output line `line` (from 0), at the pack's file and line it came from. A line
    /// past the code's last has no origin: it names the stage file with no line, as
    /// [`Origin::default`] does for the lines Sunrise writes.
    pub(crate) fn error_at_line(&self, line: usize, message: impl Into<String>) -> crate::Error {
        match self.origin_at(line) {
            Some((file, line)) => crate::Error::new(file, line, message),
            None => crate::Error::new(self.file(), 0, message),
        }
    }
}

/// Runs the preprocessor on the stage file `path` of `pack` with `macros` defined.
pub fn preprocess(pack: &Pack, path: &str, macros: &Macros) -> Preprocessed {
    let mut run = Run {
        pack,
        macros: macros.for_file(),
        conditionals: Conditionals::default(),
        out: Preprocessed::default(),
        code: Vec::new(),
        texts: HashMap::new(),
    };
    run.file(path, 0, Origin::default());
    // Iris 1.11.7 draws a program whose `#if` stays open to its end: a warning, as there.
    if run.conditionals.open() {
        run.out.warnings.push(Diagnostic {
            origin: Origin::default(),
            message: "`#if` without `#endif`: read to the end of the file".into(),
        });
    }
    run.out.code = run.code.join("\n");
    run.out
}

struct Run<'p> {
    pack: &'p Pack,
    macros: Macros,
    conditionals: Conditionals,
    out: Preprocessed,
    code: Vec<String>,
    /// Files already read: a guarded header is included many times.
    texts: HashMap<String, Option<String>>,
}

impl Run<'_> {
    fn file(&mut self, path: &str, depth: usize, from: Origin) {
        let cached = self
            .texts
            .entry(path.to_owned())
            .or_insert_with(|| self.pack.read_text(path));
        let Some(text) = cached.clone() else {
            return self.diagnose(from, &format!("can't find `{path}`"));
        };
        let file = match self.out.files.iter().position(|f| f == path) {
            Some(i) => i,
            None => {
                self.out.files.push(path.to_owned());
                self.out.files.len() - 1
            }
        } as u16;
        let (text, comments) = strip_comments(&text);
        let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        let mut comments = comments.into_iter().peekable();
        let mut n = 0;
        while n < lines.len() {
            let first = n;
            let mut line = lines[n].to_owned();
            while line.ends_with('\\') && n + 1 < lines.len() {
                line.pop();
                n += 1;
                line.push_str(lines[n]);
            }
            n += 1;
            // A macro call's arguments may span lines: they join the line the call starts on.
            // Telling whether a call goes on expands the line so far, from the file's work too,
            // so a joined line spends its work up to twice.
            if !line.trim_start().starts_with('#') && self.conditionals.active() {
                n += self.macros.join_call(&mut line, &lines[n..], n - first - 1);
            }
            let origin = Origin {
                file,
                line: first as u32 + 1,
            };
            self.line(&line, origin, path, depth);
            for k in first + 1..n {
                self.emit(
                    String::new(),
                    Origin {
                        file,
                        line: k as u32 + 1,
                    },
                );
            }
            while let Some((_, comment)) = comments.next_if(|(at, _)| *at < n) {
                if self.conditionals.active() {
                    self.out.comments.push((origin, comment));
                }
            }
        }
    }

    fn line(&mut self, line: &str, origin: Origin, path: &str, depth: usize) {
        let Some(after) = line.trim_start().strip_prefix('#') else {
            let code = if self.conditionals.active() {
                self.macros.define("__LINE__", origin.line.to_string());
                // A call nested past `MAX_ARGUMENT_DEPTH`, or an expansion past the file's
                // `MAX_EXPANSION_WORK`, refuses the program, at its line.
                self.macros.try_expand(line).unwrap_or_else(|why| {
                    self.diagnose(origin, &why);
                    String::new()
                })
            } else {
                String::new()
            };
            return self.emit(code, origin);
        };
        self.emit(String::new(), origin);
        let Some((word, rest)) = split_directive(after) else {
            return self.diagnose(origin, &format!("unknown directive `#{}`", after.trim()));
        };
        match condition::apply(&mut self.conditionals, word, rest, &self.macros) {
            // As Iris loads such a pack: the branch is skipped, the program kept.
            Err(e) if e.ends_with("read as false") => {
                return self.out.warnings.push(Diagnostic { origin, message: e });
            }
            // Iris 1.11.7 logs a stray `#endif` and draws the program: it is ignored.
            Err(e) if word == "endif" => {
                let message = format!("{e}: ignored");
                return self.out.warnings.push(Diagnostic { origin, message });
            }
            Err(e) => return self.diagnose(origin, &e),
            Ok(true) => return,
            Ok(false) if !self.conditionals.active() => return,
            Ok(false) => {}
        }
        match word {
            "define" => self.define(rest, origin),
            "undef" => self.macros.undef(rest),
            "include" => {
                let target = rest.trim_matches(|c| c == '"' || c == '<' || c == '>');
                match resolve(path, target) {
                    _ if depth + 1 >= MAX_DEPTH => self.diagnose(
                        origin,
                        &format!("`{target}`: includes nest more than {MAX_DEPTH} deep"),
                    ),
                    Some(file) => {
                        // The include line's own (empty) output line stays; the file follows.
                        self.file(&file, depth + 1, origin);
                    }
                    None => self.diagnose(origin, &format!("`{target}` is outside the pack")),
                }
            }
            // The first `#version`, also from an included file (a pack's common header).
            "version" if self.out.version.is_none() => {
                let mut words = rest.split_whitespace();
                let number = words.next().and_then(|v| v.parse().ok()).unwrap_or(110);
                self.out.version = Some((number, words.next().map(str::to_owned)));
                self.macros.define("__VERSION__", number.to_string());
            }
            "extension" => self.out.extensions.push(rest.to_owned()),
            "error" => self.diagnose(origin, &format!("#error {rest}")),
            "version" | "pragma" | "line" | "" => {}
            _ => self.diagnose(origin, &format!("unknown directive `#{word}`")),
        }
    }

    fn define(&mut self, rest: &str, origin: Origin) {
        if let Err(e) = self.macros.define_line(rest) {
            self.diagnose(origin, &e);
        }
    }

    fn emit(&mut self, code: String, origin: Origin) {
        self.code.push(code);
        self.out.lines.push(origin);
    }

    fn diagnose(&mut self, origin: Origin, message: &str) {
        self.out.diagnostics.push(Diagnostic {
            origin,
            message: message.to_owned(),
        });
    }
}

/// An `#include` path: from the `shaders` folder with a leading `/`, else from the including
/// file's folder (spec 1.4). `None` when it climbs out of the pack.
fn resolve(from_file: &str, target: &str) -> Option<String> {
    match target.strip_prefix('/') {
        Some(absolute) => normalize(absolute),
        None => {
            let dir = from_file.rsplit_once('/').map_or("", |(dir, _)| dir);
            normalize(&format!("{dir}/{target}"))
        }
    }
}

/// `text` with comments replaced by spaces (new lines kept, so line numbers hold), and the body of
/// each block comment with the index of the line it starts on.
fn strip_comments(text: &str) -> (String, Vec<(usize, String)>) {
    let mut out = String::with_capacity(text.len());
    let mut comments = Vec::new();
    let (mut rest, mut line) = (text, 0);
    while let Some(at) = rest.find('/') {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        line += before.matches('\n').count();
        if let Some(body) = from.strip_prefix("//") {
            // A line comment ending in a backslash includes the next line (GLSL 4.60, 3.4; Iris
            // does so too): such a comment runs on. Its line breaks stay.
            let mut end = body.len();
            for (at, _) in body.match_indices('\n') {
                if !body[..at].trim_end_matches('\r').ends_with('\\') {
                    end = at;
                    break;
                }
            }
            let joined = body[..end].matches('\n').count();
            line += joined;
            out.extend(std::iter::repeat_n('\n', joined));
            rest = &body[end..];
        } else if let Some(body) = from.strip_prefix("/*") {
            let end = body.find("*/").unwrap_or(body.len());
            let comment = &body[..end];
            comments.push((line, comment.to_owned()));
            let newlines = comment.matches('\n').count();
            line += newlines;
            out.push(' ');
            out.extend(std::iter::repeat_n('\n', newlines));
            rest = body.get(end + 2..).unwrap_or("");
        } else {
            out.push('/');
            rest = &from[1..];
        }
    }
    out.push_str(rest);
    (out, comments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;

    fn run(files: &[(&str, &str)]) -> Preprocessed {
        let files: Vec<(String, &str)> = files
            .iter()
            .map(|(p, t)| (format!("shaders/{p}"), *t))
            .collect();
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
        preprocess(&pack, "program/final.fsh", &Macros::standard())
    }

    fn code_lines(p: &Preprocessed) -> Vec<(&str, String)> {
        p.code
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, l)| (l.trim(), p.locate(i)))
            .collect()
    }

    #[test]
    fn includes_resolve_and_lines_map_back() {
        let p = run(&[
            (
                "program/final.fsh",
                "#version 330 compatibility\n#include \"/lib/a.glsl\"\nvoid main() {}\n",
            ),
            (
                "lib/a.glsl",
                "#ifndef A\n#define A\n#include \"b.glsl\"\nfloat a;\n#endif\n",
            ),
            ("lib/b.glsl", "// b\nfloat b;\n#include \"a.glsl\"\n"),
        ]);
        assert_eq!(p.version, Some((330, Some("compatibility".into()))));
        assert_eq!(
            code_lines(&p),
            [
                ("float b;", "lib/b.glsl:2".into()),
                ("float a;", "lib/a.glsl:4".into()),
                ("void main() {}", "program/final.fsh:3".into()),
            ]
        );
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
    }

    #[test]
    fn missing_and_runaway_includes_are_diagnosed_where_they_stand() {
        let p = run(&[(
            "program/final.fsh",
            "#ifdef NOPE\n#include \"gone.glsl\"\n#endif\n#include \"gone.glsl\"\n#include \"final.fsh\"\n",
        )]);
        let messages: Vec<String> = p
            .diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.origin.line, d.message))
            .collect();
        assert_eq!(messages[0], "4: can't find `program/gone.glsl`");
        assert!(
            messages
                .last()
                .unwrap()
                .ends_with("includes nest more than 10 deep")
        );
        assert!(
            run(&[("program/final.fsh", "#include \"../../x\"")]).diagnostics[0]
                .message
                .contains("outside")
        );
    }

    #[test]
    fn macros_expand_like_the_glsl_preprocessor() {
        let p = run(&[(
            "program/final.fsh",
            "#define SQ(x) ((x) * (x))\n#define CAT(a, b) a ## b\n#define N 4\n#define \\\n  LONG 1\n\
             float v = SQ(N + 1) + CAT(N, 2) + LONG;\nint l = __LINE__;\n#if MC_VERSION > 12100 && defined MC_RENDER_STAGE_SKY\nint ok;\n#else\nint bad;\n#endif\n",
        )]);
        let lines: Vec<&str> = code_lines(&p).into_iter().map(|(l, _)| l).collect();
        assert_eq!(
            lines,
            [
                "float v = ((4 + 1) * (4 + 1)) + N2 + 1;",
                "int l = 7;",
                "int ok;"
            ]
        );
        assert_eq!(p.lines.len(), p.code.split('\n').count());
    }

    #[test]
    fn a_paste_with_a_parenthesis_leaves_a_name_that_calls_a_macro() {
        // Alpha Piscium: `colors2_colorspaces_ ## a ## _to_ ## b ##(x)` makes the name of a
        // function-like macro and its `(`, which is then rescanned as a call.
        let p = run(&[(
            "program/final.fsh",
            "#define conv_6_to_8(x) aces_to_mc(x)\n\
             #define _convert(a, b, x) conv_ ## a ## _to_ ## b ##(x)\n\
             #define convert(a, b, x) _convert(a, b, x)\n\
             vec3 v = convert(6, 8, c);\n",
        )]);
        let lines: Vec<&str> = code_lines(&p).into_iter().map(|(l, _)| l).collect();
        assert_eq!(lines, ["vec3 v = aces_to_mc(c);"]);
    }

    #[test]
    fn macro_arguments_span_lines() {
        let p = run(&[(
            "program/final.fsh",
            "#define CLAMP01(x) clamp(x, 0.0, 1.0)\nfloat v = CLAMP01(\n    a * (b\n    + c)\n);\nfloat w;\n",
        )]);
        assert_eq!(
            code_lines(&p),
            [
                (
                    "float v = clamp(a * (b     + c), 0.0, 1.0);",
                    "program/final.fsh:2".into()
                ),
                ("float w;", "program/final.fsh:6".into())
            ]
        );
        assert_eq!(p.lines.len(), p.code.split('\n').count());
    }

    #[test]
    fn comments_extensions_and_errors() {
        let p = run(&[(
            "program/final.fsh",
            "#version 120\n#extension GL_ARB_shader_texture_lod : enable\n/* DRAWBUFFERS:04 */\n#if 0\n/* RENDERTARGETS: 1 */\n#error dead\n#endif\n\
             /* multi\nline */ float x; // gone\n#error live\n#bogus\n",
        )]);
        assert_eq!(p.extensions, ["GL_ARB_shader_texture_lod : enable"]);
        let comments: Vec<&str> = p.comments.iter().map(|(_, c)| c.trim()).collect();
        assert_eq!(comments, ["DRAWBUFFERS:04", "multi\nline"]);
        assert_eq!(code_lines(&p), [("float x;", "program/final.fsh:9".into())]);
        let messages: Vec<&str> = p.diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages, ["#error live", "unknown directive `#bogus`"]);
    }

    /// A line comment ending in a backslash runs on to the next line, as in Iris: the line it
    /// takes is not code and not a directive, and later lines keep their numbers.
    #[test]
    fn a_line_comment_runs_on_after_a_backslash() {
        let p = run(&[(
            "program/final.fsh",
            "#version 330\n#if 1\nfloat a; // ends in a backslash \\\n#endif\n\
             // alone \\\r\n#define HIDDEN\n#ifdef HIDDEN\nfloat b;\n#endif\nfloat c;\n#endif\n",
        )]);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        assert_eq!(
            code_lines(&p),
            [
                ("float a;", "program/final.fsh:3".into()),
                ("float c;", "program/final.fsh:10".into()),
            ]
        );
    }

    /// A stray `#endif` and an `#if` left open are warnings, as in Iris 1.11.7 (which draws such
    /// a program): the `#endif` is ignored, the open `#if` reads on to the end of the file.
    #[test]
    fn unbalanced_conditionals_warn_and_the_code_goes_on() {
        let p = run(&[(
            "program/final.fsh",
            "#version 330\n#ifdef NEVER\nfloat a;\n#else\nfloat b;\n#endif\n#endif\n\
             #if 1\nfloat c;\n",
        )]);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        let warnings: Vec<&str> = p.warnings.iter().map(|w| w.message.as_str()).collect();
        assert_eq!(
            warnings,
            [
                "`#endif` without `#if`: ignored",
                "`#if` without `#endif`: read to the end of the file"
            ]
        );
        let code: Vec<&str> = code_lines(&p).iter().map(|(c, _)| *c).collect();
        assert_eq!(code, ["float b;", "float c;"]);
    }

    /// A condition that can't be evaluated is a warning and reads as false; the code goes on.
    #[test]
    fn bad_conditions_warn_and_read_false() {
        let p = run(&[(
            "program/final.fsh",
            "#if (in(biome, BIOME_SOUL_SAND_VALLEY)\nfloat a;\n#else\nfloat b;\n#endif\nfloat c;\n",
        )]);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        assert_eq!(p.warnings.len(), 1);
        let code: Vec<&str> = code_lines(&p).iter().map(|(c, _)| *c).collect();
        assert_eq!(code, ["float b;", "float c;"]);
    }

    /// `#if` and `#elif` nested to `MAX_CONDITION_DEPTH` in an include nine deep (the most a
    /// pack's includes nest) run on a thread of half the 2 MiB Rust gives a spawned thread, such
    /// as the one a pack loads on, in this (debug) build; one level more is a warning naming
    /// the depth at its file and line, and reads as false, as every condition that can't be
    /// evaluated does.
    #[test]
    fn a_condition_nested_past_the_bound_is_read_as_false() {
        use sunrise_glsl::condition::{MAX_CONDITION_DEPTH, too_deep};
        let parens = |depth: usize, value: &str| {
            format!("{}{value}{}", "(".repeat(depth), ")".repeat(depth))
        };
        let (at, past) = (
            parens(MAX_CONDITION_DEPTH, "1"),
            parens(MAX_CONDITION_DEPTH + 1, "1"),
        );
        let deepest = format!(
            "#if {at}\nfloat deepest;\n#endif\n#if {past}\nfloat past;\n#else\nfloat other;\n#endif\n\
             #if 0\n#elif {at}\nfloat elif_deepest;\n#endif\n#if 0\n#elif {past}\nfloat elif_past;\n#endif\n"
        );
        let p = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || {
                let mut files = vec![(
                    "program/final.fsh".to_owned(),
                    "#version 330\n#include \"/lib/i1.glsl\"\n".to_owned(),
                )];
                for n in 1..9 {
                    files.push((
                        format!("lib/i{n}.glsl"),
                        format!("#include \"i{}.glsl\"\n", n + 1),
                    ));
                }
                files.push(("lib/i9.glsl".into(), deepest));
                let files: Vec<(&str, &str)> = files
                    .iter()
                    .map(|(p, t)| (p.as_str(), t.as_str()))
                    .collect();
                run(&files)
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        let code: Vec<&str> = code_lines(&p).iter().map(|(c, _)| *c).collect();
        assert_eq!(
            code,
            ["float deepest;", "float other;", "float elif_deepest;"]
        );
        let deepest = p.files.iter().position(|f| f == "lib/i9.glsl").unwrap() as u16;
        let warning = |line: u32| Diagnostic {
            origin: Origin {
                file: deepest,
                line,
            },
            message: format!("`#if {past}`: {}; read as false", too_deep()),
        };
        assert_eq!(p.warnings, [warning(4), warning(14)]);
    }

    /// Macro calls nested to `MAX_ARGUMENT_DEPTH` in an include nine deep expand on a thread of
    /// half the 2 MiB Rust gives a spawned thread, in this (debug) build. 400 levels, which
    /// overflowed it, stop at the bound: in an `#if` or `#elif` it is a warning at its file and
    /// line and reads as false; in code, a diagnostic at its line, which refuses the program.
    /// Each expansion past the bound stops alone: the lines after it expand.
    #[test]
    fn macro_calls_nested_past_the_depth_are_refused_where_they_are() {
        use sunrise_glsl::macros::MAX_ARGUMENT_DEPTH;
        let nested = |depth: usize, value: &str| {
            format!("{}{value}{}", "F(".repeat(depth), ")".repeat(depth))
        };
        let (at, past) = (nested(MAX_ARGUMENT_DEPTH, "1"), nested(400, "1"));
        let deepest = format!(
            "#define F(x) x\n#if {at}\nfloat deepest;\n#endif\n#if {past}\nfloat past;\n#else\n\
             float other;\n#endif\n#if 0\n#elif {past}\nfloat elif_past;\n#endif\n\
             float code = {};\nfloat code_past = {};\nfloat after = F(3.0);\n",
            nested(MAX_ARGUMENT_DEPTH, "2.0"),
            nested(400, "2.0"),
        );
        let p = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || {
                let mut files = vec![(
                    "program/final.fsh".to_owned(),
                    "#version 330\n#include \"/lib/i1.glsl\"\n".to_owned(),
                )];
                for n in 1..9 {
                    files.push((
                        format!("lib/i{n}.glsl"),
                        format!("#include \"i{}.glsl\"\n", n + 1),
                    ));
                }
                files.push(("lib/i9.glsl".into(), deepest));
                let files: Vec<(&str, &str)> = files
                    .iter()
                    .map(|(p, t)| (p.as_str(), t.as_str()))
                    .collect();
                run(&files)
            })
            .unwrap()
            .join()
            .unwrap();
        let code: Vec<&str> = code_lines(&p).iter().map(|(c, _)| *c).collect();
        assert_eq!(
            code,
            [
                "float deepest;",
                "float other;",
                "float code = 2.0;",
                "float after = 3.0;"
            ]
        );
        let why = sunrise_glsl::macros::calls_too_deep();
        let file = p.files.iter().position(|f| f == "lib/i9.glsl").unwrap() as u16;
        let at = |line: u32| Origin { file, line };
        let warning = |line: u32| Diagnostic {
            origin: at(line),
            message: format!("`#if {past}`: {why}; read as false"),
        };
        assert_eq!(p.warnings, [warning(5), warning(11)]);
        let refused = Diagnostic {
            origin: at(15),
            message: why.clone(),
        };
        assert_eq!(p.diagnostics, [refused]);
    }

    /// The issue's doubling macros (#4135): `FZ_D23(+1.0)` would make 2^20 copies of
    /// `FZ_D3(+1.0)`, past the memory of a pack's load.
    fn doubling(top: usize) -> String {
        (4..=top)
            .map(|n| format!("#define FZ_D{n}(x) FZ_D{m}(x) FZ_D{m}(x)\n", m = n - 1))
            .collect()
    }

    /// The issue's object-like macros, each the sum of two of the one before, tested by an `#if`.
    fn doubling_objects(top: usize) -> String {
        let mut text = "#define FZ_E0 1\n".to_owned();
        for n in 1..=top {
            text += &format!("#define FZ_E{n} (FZ_E{m} + FZ_E{m})\n", m = n - 1);
        }
        text
    }

    /// The stage files that a doubling macro reaches stop at the file's `MAX_EXPANSION_WORK`, at
    /// the file and line of the expansion: in code (an included file's line) it refuses the
    /// program; in an `#if`, it is a warning and reads as false, as every condition that can't be
    /// evaluated does. Another stage read with the same macros has its whole work again.
    #[test]
    fn a_doubling_macro_stops_at_the_files_work() {
        let code = format!(
            "{}float before = 1.0;\nfloat fz_bomb = 0.0 FZ_D23(+1.0);\n",
            doubling(23)
        );
        let condition = format!(
            "#version 330\n{}#if FZ_E39 > 0\nfloat taken;\n#else\nfloat other;\n#endif\n",
            doubling_objects(39)
        );
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                (
                    "shaders/final.fsh",
                    "#version 330\n#include \"/lib/bomb.glsl\"\nvoid main() {}\n",
                ),
                ("shaders/lib/bomb.glsl", code.as_str()),
                ("shaders/final.vsh", condition.as_str()),
                (
                    "shaders/composite.fsh",
                    "#define F(x) x\nfloat c = F(2.0);\n",
                ),
            ]),
        )
        .unwrap();
        let macros = Macros::standard();
        let why = sunrise_glsl::macros::work_ran_out(crate::macros::MAX_EXPANSION_WORK);
        let p = preprocess(&pack, "final.fsh", &macros);
        let file = p.files.iter().position(|f| f == "lib/bomb.glsl").unwrap() as u16;
        let refused = Diagnostic {
            origin: Origin { file, line: 22 },
            message: why.clone(),
        };
        assert_eq!(p.diagnostics, std::slice::from_ref(&refused));
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        let at = p.lines.iter().position(|o| *o == refused.origin).unwrap();
        assert_eq!(p.locate(at), "lib/bomb.glsl:22");
        let code: Vec<&str> = code_lines(&p).iter().map(|(c, _)| *c).collect();
        assert_eq!(code, ["float before = 1.0;", "void main() {}"]);

        let p = preprocess(&pack, "final.vsh", &macros);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        let warning = Diagnostic {
            origin: Origin { file: 0, line: 42 },
            message: format!("`#if FZ_E39 > 0`: {why}; read as false"),
        };
        assert_eq!(p.warnings, [warning]);
        let code: Vec<&str> = code_lines(&p).iter().map(|(c, _)| *c).collect();
        assert_eq!(code, ["float other;"]);

        let p = preprocess(&pack, "composite.fsh", &macros);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        let code: Vec<&str> = code_lines(&p).iter().map(|(c, _)| *c).collect();
        assert_eq!(code, ["float c = 2.0;"]);
    }

    /// A call over lines joins its lines as before the bound: telling whether it goes on expands
    /// the line so far, from the file's work. A doubling call whose `(` closes on the next line
    /// is refused at its first line, the lines after it kept: neither the expansions that tell
    /// whether it goes on nor the line's own get past the work.
    #[test]
    fn a_call_over_lines_past_the_work_is_refused() {
        let text = format!(
            "{}float x = 0.0 FZ_D23(\n+1.0);\nfloat after;\n",
            doubling(23)
        );
        let p = run(&[("program/final.fsh", &text)]);
        let why = sunrise_glsl::macros::work_ran_out(crate::macros::MAX_EXPANSION_WORK);
        let refused = Diagnostic {
            origin: Origin { file: 0, line: 21 },
            message: why,
        };
        assert_eq!(p.diagnostics, std::slice::from_ref(&refused));
        assert_eq!(
            code_lines(&p),
            [("float after;", "program/final.fsh:23".into())]
        );
    }

    /// Which lines a call joins is what it was before the bound. A function-like name followed by
    /// a macro that expands to nothing, with `(` on the next line, joins that line and is not
    /// called; a name pasted with `__LINE__` on a file's first line of code is pasted with the
    /// name `__LINE__` while the joining is told (`__LINE__` is defined for the line after), so
    /// it doesn't join the next line, and is `F4` in the line's code.
    #[test]
    fn calls_over_lines_join_as_before() {
        let p = run(&[(
            "program/final.fsh",
            "#define F(x) x x\n#define EMPTY\nint a = F EMPTY\n(1);\nint b;\n",
        )]);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        assert_eq!(
            code_lines(&p),
            [
                ("int a = F  (1);", "program/final.fsh:3".into()),
                ("int b;", "program/final.fsh:5".into())
            ]
        );
        let p = run(&[(
            "program/final.fsh",
            "#define CAT(a, b) a ## b\n#define XCAT(a, b) CAT(a, b)\n#define F4(x) x + x\n\
             int a = XCAT(F, __LINE__)\n(7);\n",
        )]);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        assert_eq!(
            code_lines(&p),
            [
                ("int a = F4", "program/final.fsh:4".into()),
                ("(7);", "program/final.fsh:5".into())
            ]
        );
    }

    /// Each name a token hides is work (16 bytes held as long as it): a chain of 1000 macros that
    /// ends in a replacement of 400,000 tokens, or in a call that copies an argument of 101
    /// tokens 10,000 times, would hold GBs of names in fewer tokens than the file's work. Both
    /// stop at the work, at their lines, before the names are made.
    #[test]
    fn the_names_a_token_hides_are_work() {
        let chain = |top: &str| {
            let mut text = format!("#define C0 {top}\n");
            for n in 1..1000 {
                text += &format!("#define C{n} C{}\n", n - 1);
            }
            text
        };
        let text = format!("{}float a = C999;\n", chain(&"a,".repeat(200_000)));
        let p = run(&[("program/final.fsh", &text)]);
        let why = sunrise_glsl::macros::work_ran_out(crate::macros::MAX_EXPANSION_WORK);
        let refused = |line: u32| Diagnostic {
            origin: Origin { file: 0, line },
            message: why.clone(),
        };
        assert_eq!(p.diagnostics, [refused(1001)]);
        let text = format!(
            "#define F(x) {}\n{}float b = C999;\n",
            "x,".repeat(10_000),
            chain(&format!("F({})", "b+".repeat(50) + "b"))
        );
        let p = run(&[("program/final.fsh", &text)]);
        assert_eq!(p.diagnostics, [refused(1002)]);
    }

    #[test]
    fn a_stage_is_built_only_with_one_origin_per_line_inside_its_files() {
        let at = |file, line| Origin { file, line };
        let files = || vec!["a.fsh".to_owned()];
        let built = Preprocessed::new("x\ny".into(), vec![at(0, 1), at(0, 2)], files()).unwrap();
        assert_eq!((built.code(), built.lines().len()), ("x\ny", 2));
        assert!(Preprocessed::new("x\ny".into(), vec![at(0, 1)], files()).is_err());
        assert!(Preprocessed::new("x".into(), vec![at(0, 1), at(0, 2)], files()).is_err());
        assert!(Preprocessed::new("x".into(), vec![at(1, 1)], files()).is_err());
        let read = Preprocessed::from_source("b.fsh", "x\ny\n");
        assert_eq!(read.lines(), [at(0, 1), at(0, 2), at(0, 3)]);
        assert_eq!(read.files(), ["b.fsh"]);
    }
}
