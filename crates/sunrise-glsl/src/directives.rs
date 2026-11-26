//! The directives of a stage, run as a GLSL preprocessor runs them (GLSL 4.60 §3.3) before naga
//! parses it, naga then reading code with no macro left: `#define` and `#undef` ([`Macros`]),
//! `#if`, `#ifdef`, `#ifndef`, `#elif`, `#else` and `#endif` ([`Conditionals`], each condition
//! by [`test`]), `#line`, and each other line expanded with the macros of its point, `__LINE__`
//! its line and `__VERSION__` the version the stage's file declares ([`Expanded`]'s). Lines
//! ending in `\` continue on the next, comments are blanked, and a macro call's arguments may
//! span lines ([`Macros::join_call`]). `#version`, `#extension`, `#pragma`,
//! `#error` and any other directive are left as they are, for naga.
//!
//! The result keeps the input's line breaks and its source positions; expanded tokens from a
//! macro body point to the call, and tokens copied from arguments keep their own positions.

use crate::condition::{Conditionals, eval_condition, eval_line};
use crate::located::Lines;
use crate::macros::{Macros, Stop, is_ident};
use crate::{Error, Expanded, ImportRefusal, MAX_SCANNED};

/// The macros a stage starts with: `__VERSION__`, the `version` its file declares
/// ([`crate::stage_version`]), then its program's `defines` (`NAME` or `NAME value`, in order),
/// with `work` to expand ([`Macros::checked`]).
pub(crate) fn macros(defines: &[&str], work: usize, version: u32) -> Macros {
    let mut macros = Macros::checked(work);
    macros.define(VERSION, version.to_string());
    for define in defines {
        let (name, value) = define.split_once(' ').unwrap_or((define, ""));
        macros.define(name, value);
    }
    macros
}

/// The macro that reads as the stage's GLSL version (GLSL 4.60 §3.3).
const VERSION: &str = "__VERSION__";

/// Whether the condition of `#kind rest` holds under `macros` (`kind` is `if` for an `#elif`):
/// `#ifdef` and `#ifndef` name one macro, `#if` is [`eval_condition`]'s arithmetic.
pub(crate) fn test(kind: &str, rest: &str, macros: &Macros) -> Result<bool, String> {
    match kind {
        "ifdef" | "ifndef" => {
            if rest.is_empty() || !rest.chars().all(is_ident) {
                return Err(format!("invalid #{kind} {rest}"));
            }
            Ok(macros.is_defined(rest) == (kind == "ifdef"))
        }
        _ => eval_condition(rest, macros)
            .map(|v| v != 0)
            .map_err(|e| format!("`#{kind} {rest}`: {e}")),
    }
}

/// Why a call of `name` is refused: it gives a number of arguments the macro doesn't take.
fn arguments(name: &str) -> String {
    format!("`{name}` called with a number of arguments it doesn't take")
}

/// Why `stop` stopped an expansion, without its source position.
pub(crate) fn stopped(stop: Stop) -> String {
    match stop {
        Stop::Work => ImportRefusal::TooMuchScanned.to_string(),
        Stop::Depth => crate::macros::calls_too_deep(),
        Stop::Arguments { name, .. } => arguments(&name),
    }
}

/// The parameters of a function-like definition (`NAME(a, b) body`) that name one twice, as
/// naga's preprocessor (pp-rs 0.2.1) refused: the first such name.
fn repeated_parameter(rest: &str) -> Option<&str> {
    let name_end = rest.find(|c| !is_ident(c)).unwrap_or(rest.len());
    let params = rest[name_end..].strip_prefix('(')?.split_once(')')?.0;
    let params: Vec<&str> = params.split(',').map(str::trim).collect();
    (params.iter().enumerate()).find_map(|(n, p)| params[..n].contains(p).then_some(*p))
}

/// The directive word of a line and the rest of it, trimmed; none for a line of code.
pub(crate) fn directive(line: &str) -> Option<(&str, &str)> {
    let d = line.trim_start().strip_prefix('#')?.trim_start();
    let i = d.find(|c| !is_ident(c)).unwrap_or(d.len());
    Some((&d[..i], d[i..].trim()))
}

/// The name a `#define`, `#undef`, `#ifdef` or `#ifndef` gives (the rest of its line up to
/// what is not part of a name), when it doesn't start with a letter or `_`: naga's preprocessor
/// (pp-rs 0.2.1) read a name there as an identifier token, and refused anything else.
fn bad_name(rest: &str) -> Option<&str> {
    let name = &rest[..rest.find(|c| !is_ident(c)).unwrap_or(rest.len())];
    (!name.starts_with(crate::macros::is_ident_start)).then_some(name)
}

/// Append a line with each `__LINE__` written as `number` (unsigned), keeping source positions.
/// The comment-blanked `line` starts at `at` in `joined`.
fn with_line(out: &mut Lines, line: &str, mut at: usize, joined: &Lines, number: i64) {
    for token in crate::macros::lex(line) {
        if token == "__LINE__" {
            out.push_replacement(&format!("{number}u"), None, joined, at);
        } else {
            out.push_spanned(token, None, joined, at..at + token.len());
        }
        at += token.len();
    }
}

/// `code` with its directives run under `defines` (`NAME` or `NAME value`, in order, none
/// holding a line break), as naga's preprocessor (pp-rs 0.2.1) ran them before. Its refusals
/// stay: a macro defined again with other tokens, a parameter named twice, a name that doesn't
/// start with a letter or `_`, and a call with the wrong number of arguments. `__LINE__` is the
/// line it is written on, and in a macro's replacement the line of the call (the last line of a
/// call over lines). A `#define __LINE__` is taken and changes nothing, as there.
/// `__VERSION__` is the version the stage's file declares (`stage_version`), in every
/// condition and in the code; a `#define` of it replaces it, as the name was the stage's to define
/// before (naga's preprocessor had no `__VERSION__`), and an `#undef` removes it. The expansion
/// spends at most [`MAX_SCANNED`] tokens of work (what each call reads and what its replacement
/// makes, [`Macros::checked`]), the bound on what the translator reads for a stage; past it the
/// stage is refused. Directive refusals point to the authored `#`, and wrong-arity calls in
/// code point to the authored invocation. A refusal known only by its line has no column.
pub fn run_directives(code: &Expanded, defines: &[&str]) -> Result<Expanded, Error> {
    if let Some(define) = defines.iter().find(|d| d.contains('\n')) {
        // Every line of the result is a line of the code: a replacement can't add one.
        return Err(code.error_at(None, format!("the define `{define}` holds a line break")));
    }
    let mut macros = macros(defines, MAX_SCANNED, code.version);
    // Whether `__VERSION__` is still the stage's version: a `#define` of it replaces it once, as
    // the name was the stage's own to define before it was predefined.
    let mut predefined_version = !(defines.iter()).any(|d| d.split(' ').next() == Some(VERSION));
    let text = code.text();
    // Splicing precedes comment removal: each logical line remembers its first physical one.
    let mut joined = Expanded {
        lines: Lines::default(),
        files: code.files.clone(),
        version: code.version,
    };
    let mut first = vec![0usize];
    let mut start = 0;
    for (n, line) in text.split_inclusive('\n').enumerate() {
        let piece = match line
            .strip_suffix("\\\r\n")
            .or_else(|| line.strip_suffix("\\\n"))
        {
            Some(part) => part,
            None => {
                first.push(n + 1);
                line
            }
        };
        joined.lines.push_spanned(
            piece,
            code.lines.origin(n),
            &code.lines,
            start..start + piece.len(),
        );
        start += line.len();
    }
    let clean = crate::strip_comments(joined.text());
    let logical: Vec<&str> = clean.split('\n').collect();
    let byte_at = |piece: &str| piece.as_ptr() as usize - clean.as_ptr() as usize;
    let physical = text.split('\n').count();
    let mut out = Lines::default();
    let mut out_line = 0;
    let to_line = |out: &mut Lines, out_line: &mut usize, at: usize| {
        out.push(&"\n".repeat(at - *out_line), None);
        *out_line = at;
    };
    let mut branches = Conditionals::default();
    // The `#` byte of each `#if` still open, the innermost last.
    let mut open = Vec::new();
    // A `#define __LINE__`'s rest, which only another one compares with.
    let mut line_macro: Option<&str> = None;
    // As naga's preprocessor numbers lines: the text's, moved by `#line` (the directive's own
    // line takes its number).
    let mut offset = 0i64;
    let number = |k: usize, offset: i64| first[k] as i64 + 1 + offset;
    let mut k = 0;
    while k < logical.len() {
        let line = logical[k].trim_end_matches('\r');
        let at = first[k];
        let here = number(k, offset);
        macros.define("__LINE__", format!("{here}u"));
        let directive =
            directive(line).map(|(word, rest)| (byte_at(line.trim_start()), word, rest));
        let line_error = |message| {
            let (file, line) = code.lines.origin(at).map_or((code.file(), 0), |origin| {
                (code.files[origin.file as usize].as_str(), origin.line)
            });
            Error::new(file, line, message)
        };
        let error = |message| match directive {
            Some((start, _, _)) => joined.error_at(Some(start), message),
            None => line_error(message),
        };
        if let Some((start, word, rest)) = directive {
            if matches!(word, "define" | "undef" | "ifdef" | "ifndef")
                && branches.active()
                && let Some(name) = bad_name(rest)
            {
                return Err(error(format!("`{name}` is not a macro name")));
            }
            match branches.apply_with(word, |kind| test(kind, rest, &macros)) {
                Ok(true) => {
                    match word {
                        "if" | "ifdef" | "ifndef" => open.push(start),
                        "endif" => {
                            open.pop();
                        }
                        _ => {}
                    }
                    k += 1;
                    continue;
                }
                Ok(false) if !branches.active() => {
                    k += 1;
                    continue;
                }
                Ok(false) => {}
                Err(why) => return Err(error(why)),
            }
            match word {
                "define" if rest.split(|c| !is_ident(c)).next() == Some("__LINE__") => {
                    let alike = |a: &str, b: &str| {
                        let tokens = |t: &str| -> Vec<String> {
                            (crate::macros::lex(t).into_iter())
                                .filter(|t| !t.trim().is_empty())
                                .map(str::to_owned)
                                .collect()
                        };
                        tokens(a) == tokens(b)
                    };
                    if line_macro.is_some_and(|before| !alike(before, rest)) {
                        return Err(error(format!("`#define {rest}` redefines a macro")));
                    }
                    line_macro = Some(rest);
                }
                "define" => {
                    if predefined_version && rest.split(|c| !is_ident(c)).next() == Some(VERSION) {
                        macros.undef(VERSION);
                        predefined_version = false;
                    }
                    if let Some(name) = repeated_parameter(rest) {
                        return Err(error(format!("parameter `{name}` named twice")));
                    }
                    // Defined again only the same way, as naga's preprocessor (pp-rs 0.2.1)
                    // took it.
                    if macros.defines_alike(rest) == Some(false) {
                        return Err(error(format!("`#define {rest}` redefines a macro")));
                    }
                    macros.define_line(rest).map_err(error)?;
                }
                "undef" if rest == "__LINE__" => line_macro = None,
                "undef" if !rest.is_empty() && rest.chars().all(is_ident) => {
                    predefined_version &= rest != VERSION;
                    macros.undef(rest);
                }
                "undef" => return Err(error(format!("invalid #undef {rest}"))),
                "line" => {
                    let value = eval_line(rest, &macros).map_err(error)?;
                    if !(0..1 << 32).contains(&value) {
                        return Err(error("line number outside GLSL range".into()));
                    }
                    offset = value - (at as i64 + 1);
                }
                _ => {
                    to_line(&mut out, &mut out_line, at);
                    let start = byte_at(line);
                    out.push_spanned(line, None, &joined.lines, start..start + line.len());
                }
            }
        } else if branches.active() {
            let spliced = first.get(k + 1).map_or(0, |next| next - at - 1);
            let mut probe = line.to_owned();
            let taken = macros.join_call(&mut probe, &logical[k + 1..], spliced);
            // Each line's own `__LINE__` is its number; a replacement's is the call's last line.
            let mut input = Lines::default();
            with_line(&mut input, line, byte_at(line), &joined.lines, here);
            for j in k + 1..=k + taken {
                let piece = logical[j].trim_end_matches('\r');
                input.push(" ", None);
                with_line(
                    &mut input,
                    piece,
                    byte_at(piece),
                    &joined.lines,
                    number(j, offset),
                );
            }
            let last = number(k + taken, offset);
            if !(0..1 << 32).contains(&last) && macros.mentions(&probe, "__LINE__") {
                return Err(error("line number outside GLSL range".into()));
            }
            // The first stop lasts. A probe uses the original spelling of `__LINE__`, so its
            // offsets belong to the probe, not the mapped input of the actual expansion.
            let probe_stop = macros.stopped();
            let from_probe = probe_stop.is_some();
            let stop = probe_stop.or_else(|| {
                macros.define("__LINE__", format!("{last}u"));
                to_line(&mut out, &mut out_line, at);
                macros.expand_located(&input, &mut out);
                macros.stopped()
            });
            if let Some(stop) = stop {
                let Stop::Arguments {
                    byte: Some(byte), ..
                } = stop
                else {
                    return Err(error(stopped(stop)));
                };
                let message = stopped(stop);
                let located = if from_probe {
                    // The probe replaces each LF with a space and only trims CRs from the
                    // first piece. Restore those skipped bytes for tokens on later pieces.
                    let skipped = if byte < line.len() {
                        0
                    } else {
                        logical[k].len() - line.len()
                    };
                    joined.error_at(Some(byte_at(line) + byte + skipped), message)
                } else {
                    Expanded {
                        lines: input,
                        ..joined
                    }
                    .error_at(Some(byte), message)
                };
                return Err(if located.column == 0 {
                    line_error(located.message)
                } else {
                    located
                });
            }
            k += taken + 1;
            continue;
        }
        if let Some(stop) = macros.stopped() {
            return Err(error(stopped(stop)));
        }
        k += 1;
    }
    if let Some(&at) = open.last() {
        return Err(joined.error_at(Some(at), "#if without #endif".into()));
    }
    to_line(&mut out, &mut out_line, physical - 1);
    Ok(Expanded {
        lines: out.with_line_origins(&code.lines),
        ..joined
    })
}
