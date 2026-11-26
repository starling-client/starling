//! `.properties` files: `shaders.properties` (spec 3.1) and the id files `block.properties`,
//! `item.properties`, `entity.properties` (4.1). `key=value` lines, `#` comments, a trailing `\`
//! continuing a value on the next line, and conditional directives evaluated with the
//! standard and option macros (1.5 item 2). `#define` and `#undef` work in every one of them:
//! a public pack defines a macro in its `shaders.properties` and tests it further down. The
//! lines of custom uniforms and variables (`uniform.*`, `variable.*`) have those macros expanded,
//! and so do the values of buffer and image sizes (`size.buffer.*`, `image.*`).
//! The file's expansions spend at most [`MAX_EXPANSION_WORK`](crate::macros::MAX_EXPANSION_WORK):
//! an `#if` that would spend more reads as false, an entry is dropped, each with a note.
//! No other escape is read: values are names, numbers and expressions. A later duplicate key
//! replaces the earlier one, with a note.

use crate::Macros;
use crate::condition::{Conditionals, split_directive};

/// One `key=value`, with the line it starts on (from 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub value: String,
    pub line: u32,
}

/// Something wrong with a line, for the pack's log.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Default, PartialEq)]
pub struct Properties {
    /// In file order; a key appears once (a later duplicate replaces the earlier one).
    pub entries: Vec<Entry>,
    pub notes: Vec<Note>,
}

impl Properties {
    /// Reads `text`; `#define`/`#undef` change the macros for the lines after them only.
    pub fn parse(text: &str, macros: &Macros) -> Self {
        let mut out = Self::default();
        let mut macros = macros.for_file();
        let mut conditionals = Conditionals::default();
        let mut lines = text.lines().zip(1u32..);
        while let Some((raw, number)) = lines.next() {
            let line = raw.trim();
            let on = conditionals.active();
            if let Some(directive) = line.strip_prefix('#') {
                if let Some((word, rest)) = split_directive(directive) {
                    out.directive(word, rest, number, &mut conditionals, &mut macros);
                }
                continue;
            }
            if !on {
                continue;
            }
            // A directive line ends a continued entry, as the empty line the preprocessor leaves
            // for it would.
            let mut logical = line.to_owned();
            while ends_with_continuation(&logical) {
                logical.pop();
                let Some((next, n)) = lines.next() else {
                    break;
                };
                let next = next.trim();
                if let Some(directive) = next.strip_prefix('#') {
                    if let Some((word, rest)) = split_directive(directive) {
                        out.directive(word, rest, n, &mut conditionals, &mut macros);
                    }
                    break;
                }
                logical.push_str(next);
            }
            if logical.is_empty() {
                continue;
            }
            match logical.split_once('=') {
                Some((key, value)) if !key.trim().is_empty() => {
                    let expanded =
                        |text: &str| macros.try_expand(text).map(|t| t.trim().to_owned());
                    let read = || {
                        let (mut key, mut value) = (key.trim().to_owned(), value.trim().to_owned());
                        // A custom uniform's or variable's line is read with the macros expanded,
                        // as Iris 1.11.7 reads it (probe pack): options, standard macros and this
                        // file's own `#define`s, in the name too.
                        if key.starts_with("uniform.") || key.starts_with("variable.") {
                            key = expanded(&key)?;
                            value = expanded(&value)?;
                        }
                        // A buffer's or an image's sizes may be macros the file defines above it
                        // (spec 1.5 item 2, 6.1.2, 6.5): its value is expanded with the macros of
                        // its line. The name stays as the pack wrote it.
                        if key.starts_with("image.") || key.starts_with("size.buffer.") {
                            value = expanded(&value)?;
                        }
                        Ok::<_, String>((key, value))
                    };
                    // A call nested past `MAX_ARGUMENT_DEPTH`, or an expansion past the file's
                    // `MAX_EXPANSION_WORK`, drops the entry, with a note.
                    let (key, value) = match read() {
                        Ok(read) => read,
                        Err(why) => {
                            out.note(number, &why);
                            continue;
                        }
                    };
                    out.insert(Entry {
                        key,
                        value,
                        line: number,
                    })
                }
                _ => out.note(number, "not a key=value line"),
            }
        }
        if conditionals.open() {
            out.note(text.lines().count() as u32, "`#if` without `#endif`");
        }
        out
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entry(key).map(|e| e.value.as_str())
    }

    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// `separateAo = true`: terrain vertices give their ambient occlusion in the colour's
    /// alpha instead of multiplying it into the colour. Anything else, or no key, is `false`.
    pub fn separate_ao(&self) -> bool {
        self.get("separateAo").is_some_and(|v| v.trim() == "true")
    }

    /// Sunrise's HDR extension, asked for by `sunrise.hdr = true`, and who owns the tone curve
    /// (`sunrise.hdr.tonemap`, `pack` when absent). Not a key of the format: Sunrise's own,
    /// which another loader does not read. A value of either key outside its set turns the
    /// request off, with a note at its line naming the key and the value. `parse` checks
    /// neither key: only what decides the frame reads them, so a pack read without Sunrise's
    /// HDR logs nothing new.
    pub fn sunrise_hdr(&self) -> (Option<Tonemap>, Vec<Note>) {
        let mut notes = Vec::new();
        let mut value = |key: &str, takes: &str, known: &dyn Fn(&str) -> bool| {
            let entry = self.entry(key)?;
            if !known(&entry.value) {
                notes.push(Note {
                    line: entry.line,
                    message: format!(
                        "`{key}` is `{}`, not {takes}: Sunrise's HDR stays off",
                        entry.value
                    ),
                });
            }
            Some(entry.value.as_str())
        };
        let asked = value(HDR_KEY, "`true` or `false`", &|v| {
            matches!(v, "true" | "false")
        });
        let tonemap = value(TONEMAP_KEY, "`pack` or `sunrise`", &|v| {
            Tonemap::named(v).is_some()
        });
        let hdr = match (asked, tonemap) {
            (Some("true"), None) => Some(Tonemap::Pack),
            (Some("true"), Some(tonemap)) => Tonemap::named(tonemap),
            _ => None,
        };
        (hdr, notes)
    }

    fn insert(&mut self, entry: Entry) {
        if let Some(at) = self.entries.iter().position(|e| e.key == entry.key) {
            let first = self.entries.remove(at).line;
            self.note(
                entry.line,
                &format!(
                    "`{}` is set again (first on line {first}); this one counts",
                    entry.key
                ),
            );
        }
        self.entries.push(entry);
    }

    fn note(&mut self, line: u32, message: &str) {
        self.notes.push(Note {
            line,
            message: message.to_owned(),
        });
    }

    /// A line starting with `#` whose first word may be a directive; any other word makes it a
    /// comment.
    fn directive(
        &mut self,
        word: &str,
        rest: &str,
        number: u32,
        conditionals: &mut Conditionals,
        macros: &mut Macros,
    ) {
        match crate::condition::apply(conditionals, word, rest, macros) {
            Err(e) => self.note(number, &e),
            Ok(true) => {}
            Ok(false) if conditionals.active() => match word {
                "define" => {
                    if let Err(e) = macros.define_line(rest) {
                        self.note(number, &e);
                    }
                }
                "undef" => macros.undef(rest),
                _ => {}
            },
            Ok(false) => {}
        }
    }
}

const HDR_KEY: &str = "sunrise.hdr";
const TONEMAP_KEY: &str = "sunrise.hdr.tonemap";

/// Who applies the tone curve to the light an HDR pack's `final` writes (`sunrise.hdr.tonemap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tonemap {
    /// `pack`: the pack's `final` writes display-referred light; Sunrise adds no curve.
    Pack,
    /// `sunrise`: the pack writes scene-referred light and Sunrise's curve maps it.
    Sunrise,
}

impl Tonemap {
    fn named(value: &str) -> Option<Self> {
        match value {
            "pack" => Some(Self::Pack),
            "sunrise" => Some(Self::Sunrise),
            _ => None,
        }
    }
}

/// An odd number of trailing backslashes: the last one joins the next line (`\\` is a literal).
fn ends_with_continuation(line: &str) -> bool {
    line.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Properties {
        let mut m = Macros::standard();
        m.define("SHADOWS", "");
        m.define("QUALITY", "2");
        Properties::parse(text, &m)
    }

    fn pairs(p: &Properties) -> Vec<(&str, &str)> {
        p.entries
            .iter()
            .map(|e| (e.key.as_str(), e.value.as_str()))
            .collect()
    }

    /// `sunrise.hdr = true` asks; the tone curve is the pack's unless `sunrise.hdr.tonemap =
    /// sunrise`. Absent, `false` or a tone curve without the request: nothing asked, no note.
    #[test]
    fn sunrise_hdr_is_asked_by_its_key() {
        let asked = |text: &str| {
            let (hdr, notes) = read(text).sunrise_hdr();
            assert!(notes.is_empty(), "{text}: {notes:?}");
            hdr
        };
        assert_eq!(asked("sunrise.hdr = true"), Some(Tonemap::Pack));
        assert_eq!(
            asked("sunrise.hdr.tonemap=pack\nsunrise.hdr=true"),
            Some(Tonemap::Pack)
        );
        assert_eq!(
            asked("sunrise.hdr = true\nsunrise.hdr.tonemap = sunrise"),
            Some(Tonemap::Sunrise)
        );
        assert_eq!(asked("separateAo=true"), None);
        assert_eq!(
            asked("sunrise.hdr = false\nsunrise.hdr.tonemap = sunrise"),
            None
        );
        assert_eq!(asked("sunrise.hdr.tonemap = sunrise"), None);
        // The last of a duplicated key counts, as for any key.
        assert_eq!(asked("sunrise.hdr=true\nsunrise.hdr=false"), None);
    }

    /// The request may sit behind the pack's own option, or a macro the file defines.
    #[test]
    fn sunrise_hdr_follows_the_conditionals_around_it() {
        let text = "#ifdef HDR_OUTPUT\nsunrise.hdr=true\n#endif\n\
                    #if QUALITY == 2\nsunrise.hdr.tonemap=sunrise\n#endif";
        assert_eq!(read(text).sunrise_hdr().0, None);
        let mut m = Macros::standard();
        m.define("HDR_OUTPUT", "");
        m.define("QUALITY", "2");
        assert_eq!(
            Properties::parse(text, &m).sunrise_hdr().0,
            Some(Tonemap::Sunrise)
        );
        m.define("QUALITY", "1");
        assert_eq!(
            Properties::parse(text, &m).sunrise_hdr().0,
            Some(Tonemap::Pack)
        );
        let defined = "#define WANT_HDR\n#ifdef WANT_HDR\nsunrise.hdr=true\n#endif";
        assert_eq!(read(defined).sunrise_hdr().0, Some(Tonemap::Pack));
    }

    /// A value outside a key's set turns the request off, with a note at its line naming the
    /// key and the value; a tone curve's unknown value is noted even without the request.
    /// Reading the file notes neither: only the request's read does.
    #[test]
    fn an_unknown_sunrise_hdr_value_is_noted_by_the_request_only() {
        let noted = |text: &str| {
            let p = read(text);
            assert!(p.notes.is_empty(), "{text}: {:?}", p.notes);
            let (hdr, notes) = p.sunrise_hdr();
            let notes: Vec<(u32, bool)> = notes
                .iter()
                .map(|n| {
                    let entry = p.entries.iter().find(|e| e.line == n.line).unwrap();
                    let named = n.message.contains(&format!("`{}`", entry.key))
                        && n.message.contains(&format!("`{}`", entry.value));
                    (n.line, named)
                })
                .collect();
            (hdr, notes)
        };
        assert_eq!(noted("a=1\nsunrise.hdr = yes"), (None, vec![(2, true)]));
        assert_eq!(noted("sunrise.hdr = TRUE"), (None, vec![(1, true)]));
        assert_eq!(noted("sunrise.hdr ="), (None, vec![(1, true)]));
        assert_eq!(
            noted("sunrise.hdr = true\n\nsunrise.hdr.tonemap = aces"),
            (None, vec![(3, true)])
        );
        assert_eq!(noted("sunrise.hdr.tonemap = Pack"), (None, vec![(1, true)]));
        // Both unknown: one note each, at its own line.
        assert_eq!(
            noted("sunrise.hdr = 1\nsunrise.hdr.tonemap = 2").1,
            [(1, true), (2, true)]
        );
    }

    #[test]
    fn keys_values_and_comments() {
        let p =
            read("# a comment\n\n  sun = false\n#commented.key=1\nscreen=A B [C]\r\nbad line\n");
        assert_eq!(pairs(&p), [("sun", "false"), ("screen", "A B [C]")]);
        assert_eq!(p.entry("screen").unwrap().line, 5);
        assert_eq!(
            p.notes,
            [Note {
                line: 6,
                message: "not a key=value line".into()
            }]
        );
    }

    #[test]
    fn continuation_lines_join() {
        let p = read("screen=A B \\\n    C D\\\n  E\nsliders=X\nliteral=a\\\\\nnext=1");
        assert_eq!(
            pairs(&p),
            [
                ("screen", "A B C DE"),
                ("sliders", "X"),
                ("literal", "a\\\\"),
                ("next", "1")
            ]
        );
        assert_eq!(p.entry("sliders").unwrap().line, 4);
    }

    /// A continued value ending before `#else` stops there, and the line after `#endif` is an
    /// entry of its own.
    #[test]
    fn continuations_stop_at_directives() {
        let text = "#define ON\n#ifdef ON\nu = if( \\\n  a, 1.0, \\\n  0.5) \\\n#else\nu = 1.0\n#endif\n\n\
                    #ifdef OFF\nv = 1 \\\n  2\n#endif\nw = 3";
        assert_eq!(pairs(&read(text)), [("u", "if( a, 1.0, 0.5)"), ("w", "3")]);
        let text = "#define ON\n#ifdef ON\nu = 1 \\\n#else\nu = 2\n#endif\nw = 3";
        assert_eq!(pairs(&read(text)), [("u", "1"), ("w", "3")]);
    }

    #[test]
    fn conditionals_pick_one_branch() {
        let p = read(
            "#if MC_VERSION >= 11300\nnew=1\n#else\nold=1\n#endif\n\
             #ifdef SHADOWS\n#if QUALITY == 1\nq=1\n#elif QUALITY == 2\nq=2\n#elif QUALITY >= 2\nq=3\n#endif\n#endif\n\
             #ifndef SHADOWS\nnone=1\n#elif 0\nnever=1\n#else\nfallback=1\n#endif\n#   ifdef SHADOWS // trailing comment\nspaced=1\n#endif",
        );
        assert_eq!(
            pairs(&p),
            [("new", "1"), ("q", "2"), ("fallback", "1"), ("spaced", "1")]
        );
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    #[test]
    fn comment_lines_are_not_directives() {
        let p = read("#ifdefault=1\n#iffy\n#endiff\n#else-where\nkey=1");
        assert_eq!(pairs(&p), [("key", "1")]);
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    #[test]
    fn defines_apply_to_the_lines_after_them() {
        let text = "#ifdef MC_OS_NONE\n#define WATER 0\n#endif\n#define WATER 8\n\
                    #if WATER == 8\nblock.8=minecraft:water\n#endif\n#undef WATER\n#ifdef WATER\nno=1\n#endif";
        let p = Properties::parse(text, &Macros::default());
        assert_eq!(pairs(&p), [("block.8", "minecraft:water")]);
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    #[test]
    fn duplicate_keys_keep_the_last() {
        let p = read("a=1\nb=2\na=3");
        assert_eq!(pairs(&p), [("b", "2"), ("a", "3")]);
        assert_eq!(p.notes[0].line, 3);
    }

    #[test]
    fn broken_conditionals_are_noted() {
        let p = read("#endif\n#else\n#if 1 +\nx=1\n#endif\n#if 1\n#else\n#else\n");
        let lines: Vec<u32> = p.notes.iter().map(|n| n.line).collect();
        assert_eq!(lines, [1, 2, 3, 8, 8]);
        assert!(p.entries.is_empty());
    }

    /// `#if` and `#elif` nested to `MAX_CONDITION_DEPTH` read on a thread of half the 2 MiB Rust
    /// gives a spawned thread, such as the one a pack loads on, in this (debug) build; one level
    /// more is noted by name at its line and reads as false.
    #[test]
    fn a_condition_nested_past_the_bound_is_noted_and_read_as_false() {
        use sunrise_glsl::condition::{MAX_CONDITION_DEPTH, too_deep};
        let parens = |depth: usize| format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
        let (at, past) = (parens(MAX_CONDITION_DEPTH), parens(MAX_CONDITION_DEPTH + 1));
        let text = format!(
            "#if {at}\ndeepest=1\n#endif\n#if {past}\npast=1\n#else\nother=1\n#endif\n\
             #if 0\n#elif {at}\nelif_deepest=1\n#endif\n#if 0\n#elif {past}\nelif_past=1\n#endif\n"
        );
        let p = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || read(&text))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(
            pairs(&p),
            [("deepest", "1"), ("other", "1"), ("elif_deepest", "1")]
        );
        let note = |line: u32| Note {
            line,
            message: format!("`#if {past}`: {}; read as false", too_deep()),
        };
        assert_eq!(p.notes, [note(4), note(14)]);
    }

    /// Macro calls nested to `MAX_ARGUMENT_DEPTH` expand on a thread of half the 2 MiB Rust gives
    /// a spawned thread, in this (debug) build. 400 levels, which overflowed it, stop at the
    /// bound, by name at the line: an `#if` or `#elif` reads as false, and a custom uniform or a
    /// buffer size expanded with the macros is dropped. The lines after expand.
    #[test]
    fn macro_calls_nested_past_the_depth_are_noted_where_they_are() {
        use sunrise_glsl::macros::{MAX_ARGUMENT_DEPTH, calls_too_deep};
        let nested = |depth: usize, value: &str| {
            format!("{}{value}{}", "F(".repeat(depth), ")".repeat(depth))
        };
        let past = nested(400, "1");
        let text = format!(
            "#define F(x) x\n#if {past}\npast=1\n#else\nother=1\n#endif\n\
             #if 0\n#elif {past}\nelif_past=1\n#endif\n\
             uniform.float.deepest = {}\nuniform.float.past = {past}\n\
             size.buffer.colortex4 = {} 4\nsize.buffer.colortex5 = {past} 4\n\
             uniform.float.after = F(3)\n",
            nested(MAX_ARGUMENT_DEPTH, "1"),
            nested(MAX_ARGUMENT_DEPTH, "4"),
        );
        let p = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || read(&text))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(
            pairs(&p),
            [
                ("other", "1"),
                ("uniform.float.deepest", "1"),
                ("size.buffer.colortex4", "4 4"),
                ("uniform.float.after", "3")
            ]
        );
        let condition = format!("`#if {past}`: {}; read as false", calls_too_deep());
        let note = |line: u32, message: &str| Note {
            line,
            message: message.to_owned(),
        };
        assert_eq!(
            p.notes,
            [
                note(2, &condition),
                note(8, &condition),
                note(12, &calls_too_deep()),
                note(14, &calls_too_deep())
            ]
        );
    }

    /// A `.properties` file spends one file's `MAX_EXPANSION_WORK`, as a stage file does (#4135):
    /// the issue's object-like doubling in an `#if` is a note at its line and reads as false; a
    /// doubling call in a custom uniform or a buffer size drops the entry, with a note at its
    /// line. The entries before, and those without macros, read.
    #[test]
    fn doubling_macros_stop_at_the_files_work() {
        let mut text = "#define Q(x) x\nuniform.float.before = Q(2)\n#define FZ_E0 1\n".to_owned();
        for n in 1..=39 {
            text += &format!("#define FZ_E{n} (FZ_E{m} + FZ_E{m})\n", m = n - 1);
        }
        for n in 4..=23 {
            text += &format!("#define FZ_D{n}(x) FZ_D{m}(x) FZ_D{m}(x)\n", m = n - 1);
        }
        // Line 63.
        text += "#if FZ_E39 > 0\ntaken=1\n#else\nother=1\n#endif\n\
                 uniform.float.bomb = FZ_D23(1)\nsize.buffer.colortex5 = FZ_D23(1) 4\nafter=1\n";
        let p = read(&text);
        assert_eq!(
            pairs(&p),
            [
                ("uniform.float.before", "2"),
                ("other", "1"),
                ("after", "1")
            ]
        );
        let why = sunrise_glsl::macros::work_ran_out(crate::macros::MAX_EXPANSION_WORK);
        let note = |line: u32, message: &str| Note {
            line,
            message: message.to_owned(),
        };
        assert_eq!(
            p.notes,
            [
                note(63, &format!("`#if FZ_E39 > 0`: {why}; read as false")),
                note(68, &why),
                note(69, &why)
            ]
        );
    }
}
