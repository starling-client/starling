//! A failed build's compiler diagnostics as the mod's author reads them.
//!
//! A build compiles a copy of each mod, staged under its own work folder, so the compiler names
//! files the author never edits. Rewriting its text, or telling it to name other paths, would
//! also change what it prints for crates that are not the author's and what it compiles into
//! the library (`file!()`, debug information). So this reads the compiler's structured
//! diagnostics instead: each source location (`file_name`) that lies in a mod's staged copy and
//! names a file the author has is shown at the author's file, in the rendered text's location
//! lines and nowhere else.
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

/// A compiler diagnostic: one `--error-format=json` line of rustc, or the `message` of a Cargo
/// `compiler-message` event.
#[derive(Deserialize)]
#[serde(transparent)]
pub struct Diagnostic(Value);

impl Diagnostic {
    /// The diagnostic in `line`, when it is one: any other line (a panic's text, a linker's, a
    /// JSON line that is no diagnostic) stays text.
    pub fn parse(line: &str) -> Option<Self> {
        let diagnostic = Self(serde_json::from_str(line).ok()?);
        diagnostic.rendered().is_some().then_some(diagnostic)
    }

    /// The text rustc prints for it.
    pub fn rendered(&self) -> Option<&str> {
        self.0.get("rendered")?.as_str()
    }

    /// Whether rustc itself says this type or trait mismatch (E0277, E0308) is about one crate
    /// being in the dependency graph in several versions: it adds a note to the diagnostic, in
    /// these words, when the types it compares come from two copies of one crate. Nothing the
    /// diagnostic quotes or names, nor any other diagnostic, counts.
    pub fn crate_present_twice(&self) -> bool {
        let code = self.0.pointer("/code/code").and_then(Value::as_str);
        matches!(code, Some("E0277" | "E0308"))
            && self
                .0
                .get("children")
                .and_then(Value::as_array)
                .is_some_and(|children| {
                    children.iter().any(|child| {
                        child.get("level").and_then(Value::as_str) == Some("note")
                            && child
                                .get("message")
                                .and_then(Value::as_str)
                                .is_some_and(|message| {
                                    message.starts_with(
                                        "there are multiple different versions of crate `",
                                    ) && message.ends_with("` in the dependency graph")
                                })
                    })
                })
    }

    /// Every file the diagnostic's spans name, those of its children and macro expansions too.
    fn file_names(&self) -> BTreeSet<&str> {
        fn walk<'a>(value: &'a Value, names: &mut BTreeSet<&'a str>) {
            match value {
                Value::Object(fields) => {
                    if let Some(name) = fields.get("file_name").and_then(Value::as_str) {
                        names.insert(name);
                    }
                    fields.values().for_each(|field| walk(field, names));
                }
                Value::Array(items) => items.iter().for_each(|item| walk(item, names)),
                _ => {}
            }
        }
        let mut names = BTreeSet::new();
        if let Some(fields) = self.0.as_object() {
            for key in ["spans", "children"] {
                if let Some(field) = fields.get(key) {
                    walk(field, &mut names);
                }
            }
        }
        names
    }
}

/// A mod's source as a build staged it.
pub struct Staged {
    /// The folder the build copied the mod's files into, and compiles there.
    pub root: PathBuf,
    /// The folder the author keeps the mod in.
    pub author: PathBuf,
}

/// Where the compiler's file names are found, and what the build staged.
pub struct Sources {
    /// The folder the compiler ran in, which its relative file names are relative to.
    base: PathBuf,
    staged: Vec<Staged>,
}

/// A file name the compiler gave, as the author has it.
struct Own<'a> {
    name: &'a str,
    path: PathBuf,
    /// Its lines: the staged copy of a mod's crate root has more, appended by the build.
    lines: usize,
}

impl Sources {
    pub fn new(base: PathBuf, staged: Vec<Staged>) -> Self {
        Self { base, staged }
    }

    /// `name`, a file name of the compiler's, as the author's own file: only a path in a staged
    /// copy that the author's folder has too. A registry crate, an SDK file, a file the build
    /// generated and a path leaving the staged folder (`..`) are no author's file.
    fn own<'a>(&self, name: &'a str) -> Option<Own<'a>> {
        let named = Path::new(name);
        if named.components().any(|part| part == Component::ParentDir) {
            return None;
        }
        let full = self.base.join(named);
        self.staged.iter().find_map(|staged| {
            let inside = full.strip_prefix(&staged.root).ok()?;
            let path = staged.author.join(inside);
            let bytes = std::fs::read(&path).ok()?;
            let ended = bytes.last().is_none_or(|last| *last == b'\n');
            Some(Own {
                name,
                path,
                lines: bytes.iter().filter(|byte| **byte == b'\n').count() + usize::from(!ended),
            })
        })
    }

    /// The lines of `diagnostic`'s rendered text, each with the line as shown to the author:
    /// only a location line (`-->`, `:::`) whose path is exactly a file name of the diagnostic's
    /// spans, and that is the author's file, names the author's folder, and only where the
    /// author's file has that line (the build appends to a crate root). Each path is looked up
    /// once, so a result is never looked up again.
    pub fn lines<'a>(&self, diagnostic: &'a Diagnostic) -> Vec<(&'a str, String)> {
        let own: Vec<_> = diagnostic
            .file_names()
            .into_iter()
            .filter_map(|name| self.own(name))
            .collect();
        diagnostic
            .rendered()
            .unwrap_or_default()
            .lines()
            .map(|line| (line, shown(line, &own)))
            .collect()
    }

    /// `diagnostic` as the author reads it.
    pub fn text(&self, diagnostic: &Diagnostic) -> String {
        let lines = self.lines(diagnostic);
        lines
            .iter()
            .map(|(_, shown)| shown.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// `line` with its location (`--> path:line:column`) at the author's file, when it names one.
fn shown(line: &str, own: &[Own<'_>]) -> String {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    for marker in ["--> ", "::: "] {
        let Some(location) = body.strip_prefix(marker) else {
            continue;
        };
        for file in own {
            let Some(at) = location
                .strip_prefix(file.name)
                .and_then(|rest| rest.strip_prefix(':'))
            else {
                continue;
            };
            let row = at
                .split(':')
                .next()
                .and_then(|row| row.parse::<usize>().ok());
            if row.is_some_and(|row| row <= file.lines) {
                return format!("{indent}{marker}{}:{at}", file.path.display());
            }
        }
    }
    line.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A rustc diagnostic as `--error-format=json` prints it (rustc 1.99.1), its `explanation`
    /// and the source text of its spans left out, naming `file` for the error.
    fn mismatch(file: &str, also: &str) -> String {
        let rendered = format!(
            "error[E0308]: mismatched types\\n --> {file}:1:21\\n  |\\n1 | pub fn f() -> u32 {{ \\\"x\\\" }}\\n  |               ---   ^^^ expected `u32`, found `&str`\\n  |               |\\n  |               expected `u32` because of return type\\n{also}\\n\\n"
        );
        format!(
            r#"{{"$message_type":"diagnostic","message":"mismatched types","code":{{"code":"E0308","explanation":"…"}},"level":"error","spans":[{{"file_name":"{file}","byte_start":20,"byte_end":23,"line_start":1,"line_end":1,"column_start":21,"column_end":24,"is_primary":true,"text":[],"label":"expected `u32`, found `&str`","suggested_replacement":null,"suggestion_applicability":null,"expansion":null}}],"children":[],"rendered":"{rendered}"}}"#
        )
    }

    /// The scratch folders of one test: a build's work folder with `native-api/<id>` staged
    /// copies of the mods, and the folders their authors keep them in.
    struct Build {
        _scratch: tempfile::TempDir,
        out: PathBuf,
    }

    impl Build {
        fn new() -> Self {
            let scratch = tempfile::tempdir().unwrap();
            let out = scratch.path().join("cache").join("work");
            Self {
                _scratch: scratch,
                out,
            }
        }

        /// A mod `id` whose author keeps `files` in `folder` (below the scratch folder), staged
        /// the way the registry route does.
        fn stage(&self, id: &str, folder: &str, files: &[&str]) -> Staged {
            let author = self._scratch.path().join(folder);
            let root = self.out.join("native-api").join(id);
            for file in files {
                for base in [&author, &root] {
                    fs::create_dir_all(base.join(file).parent().unwrap()).unwrap();
                    fs::write(base.join(file), "pub fn f() -> u32 { \"x\" }\n").unwrap();
                }
            }
            Staged { root, author }
        }

        fn sources(&self, staged: Vec<Staged>) -> Sources {
            Sources::new(self.out.clone(), staged)
        }
    }

    fn text(sources: &Sources, json: &str) -> String {
        sources.text(&Diagnostic::parse(json).unwrap())
    }

    #[test]
    fn a_relative_staged_span_names_the_authors_file() {
        let build = Build::new();
        let one = build.stage("one", "mods/one", &["src/lib.rs"]);
        let author = one.author.join("src/lib.rs");
        let shown = text(
            &build.sources(vec![one]),
            &mismatch("native-api/one/src/lib.rs", ""),
        );
        assert!(
            shown.contains(&format!(" --> {}:1:21", author.display())),
            "{shown}"
        );
        assert!(!shown.contains("native-api"), "{shown}");
        // The snippet below the location is the compiler's, untouched.
        assert!(shown.contains("1 | pub fn f() -> u32 { \"x\" }"), "{shown}");
    }

    #[test]
    fn an_absolute_staged_span_names_the_authors_file_without_the_cache_prefix() {
        let build = Build::new();
        let one = build.stage("one", "mods/one", &["src/lib.rs"]);
        let author = one.author.join("src/lib.rs");
        let staged = build.out.join("native-api/one/src/lib.rs");
        let shown = text(
            &build.sources(vec![one]),
            &mismatch(&staged.display().to_string(), ""),
        );
        assert!(
            shown.contains(&format!(" --> {}:1:21", author.display())),
            "{shown}"
        );
        assert!(!shown.contains("cache"), "{shown}");
    }

    #[test]
    fn a_dependencys_own_path_is_left_as_it_is() {
        let build = Build::new();
        let one = build.stage("one", "mods/one", &["src/lib.rs"]);
        // A registry crate's source, a file of the SDK, and a relative name that is the
        // dependency's own (rustc's note about two versions of a crate names `src/lib.rs`
        // of the crate's folder, which the author's folder also has).
        let registry = "/home/player/.cargo/registry/src/index.crates.io-1/glam-0.30/src/lib.rs";
        let sdk = "/sdk/native-api/one/src/lib.rs";
        for file in [registry, sdk, "src/lib.rs"] {
            let json = mismatch(file, "");
            let shown = text(
                &build.sources(vec![Staged {
                    root: one.root.clone(),
                    author: one.author.clone(),
                }]),
                &json,
            );
            assert!(shown.contains(&format!(" --> {file}:1:21")), "{shown}");
        }
    }

    #[test]
    fn a_path_quoted_in_a_message_or_snippet_is_not_a_location() {
        let build = Build::new();
        let one = build.stage("one", "mods/one", &["src/lib.rs"]);
        let author = one.author.join("src/lib.rs");
        // The staged path quoted in a note's text.
        let quoted = "  = note: native-api/one/src/lib.rs:1:21 is the staged copy";
        let shown = text(
            &build.sources(vec![one]),
            &mismatch("native-api/one/src/lib.rs", quoted),
        );
        assert!(shown.contains(&format!(" --> {}:1:21", author.display())));
        assert!(
            shown.contains("  = note: native-api/one/src/lib.rs:1:21 is the staged copy"),
            "{shown}"
        );
    }

    #[test]
    fn a_result_is_never_looked_up_again() {
        let build = Build::new();
        // The author of `one` keeps it in a folder with a staged-looking component, the staged
        // copy of `two`: its result must stay there, not move to `two`'s own folder.
        let one = build.stage("one", "work/native-api/two", &["src/lib.rs"]);
        let two = build.stage("two", "mods/two", &["src/lib.rs"]);
        let at = one.author.join("src/lib.rs");
        let shown = text(
            &build.sources(vec![one, two]),
            &mismatch("native-api/one/src/lib.rs", ""),
        );
        assert!(
            shown.contains(&format!(" --> {}:1:21", at.display())),
            "{shown}"
        );
        assert!(!shown.contains("mods/two"), "{shown}");
    }

    #[test]
    fn each_mod_is_shown_at_its_own_folder() {
        let build = Build::new();
        let one = build.stage("one", "mods/one", &["src/lib.rs"]);
        let longer = build.stage("one_more", "mods/more", &["src/lib.rs"]);
        let at = longer.author.join("src/lib.rs");
        let shown = text(
            &build.sources(vec![one, longer]),
            &mismatch("native-api/one_more/src/lib.rs", ""),
        );
        assert!(
            shown.contains(&format!(" --> {}:1:21", at.display())),
            "{shown}"
        );
    }

    #[test]
    fn a_generated_file_is_no_authors_file() {
        let build = Build::new();
        let one = build.stage("one", "mods/one", &["src/lib.rs"]);
        // The entry the build writes beside the crate root exists only in the staged copy.
        fs::write(one.root.join("src/__sunrise_entry.rs"), "// generated\n").unwrap();
        let shown = text(
            &build.sources(vec![one]),
            &mismatch("native-api/one/src/__sunrise_entry.rs", ""),
        );
        assert!(
            shown.contains(" --> native-api/one/src/__sunrise_entry.rs:1:21"),
            "{shown}"
        );
    }

    #[test]
    fn the_end_the_build_appended_to_a_crate_root_is_not_the_authors() {
        let build = Build::new();
        let one = build.stage("one", "mods/one", &["src/lib.rs"]);
        // The staged root is longer than the author's file (one line): line 4 is the build's.
        let json = mismatch("native-api/one/src/lib.rs", "").replace(":1:21", ":4:1");
        let shown = text(&build.sources(vec![one]), &json);
        assert!(
            shown.contains(" --> native-api/one/src/lib.rs:4:1"),
            "{shown}"
        );
    }

    #[test]
    fn a_path_leaving_the_staged_folder_is_left_as_it_is() {
        let build = Build::new();
        let one = build.stage("one", "mods/one", &["src/lib.rs"]);
        fs::create_dir_all(build.out.join("native-api/two/src")).unwrap();
        fs::write(build.out.join("native-api/two/src/lib.rs"), "\n").unwrap();
        for file in [
            "native-api/one/../two/src/lib.rs",
            "native-api/one/src/../../../../outside.rs",
        ] {
            let shown = text(
                &build.sources(vec![Staged {
                    root: one.root.clone(),
                    author: one.author.clone(),
                }]),
                &mismatch(file, ""),
            );
            assert!(shown.contains(&format!(" --> {file}:1:21")), "{shown}");
        }
    }

    #[test]
    fn an_authors_folder_with_an_equals_sign_is_named_whole() {
        let build = Build::new();
        let one = build.stage("one", "mods/a=b/c=d", &["src/lib.rs"]);
        let at = one.author.join("src/lib.rs");
        let shown = text(
            &build.sources(vec![one]),
            &mismatch("native-api/one/src/lib.rs", ""),
        );
        assert!(
            shown.contains(&format!(" --> {}:1:21", at.display())),
            "{shown}"
        );
    }

    #[test]
    fn only_a_diagnostic_is_parsed() {
        for line in [
            "thread 'rustc' panicked at compiler/rustc_middle/src/ty/context.rs:1:1:",
            "{\"$message_type\":\"artifact\",\"artifact\":\"/x/libm.rlib\",\"emit\":\"link\"}",
            "{\"rendered\":",
            "[]",
            "",
        ] {
            assert!(Diagnostic::parse(line).is_none(), "{line}");
        }
    }

    /// rustc 1.99.1's diagnostics for a type from two versions of one crate (E0308), a trait
    /// from two (E0277), an E0308 without the note, and an unrelated error saying "different
    /// versions of" in a message, a quoted line and a path.
    const TWO_VERSIONS: &str = r#"{"$message_type":"diagnostic","message":"mismatched types","code":{"code":"E0308","explanation":"…"},"level":"error","spans":[{"file_name":"src.rs","line_start":2,"line_end":2,"column_start":21,"column_end":32,"is_primary":true,"text":[],"label":"expected `W`, found `dep::W`","expansion":null}],"children":[{"message":"there are multiple different versions of crate `dep` in the dependency graph","code":null,"level":"note","spans":[{"file_name":"src/lib.rs","line_start":1,"line_end":1,"column_start":1,"column_end":13,"is_primary":true,"text":[],"label":"this is the expected type","expansion":null}],"children":[],"rendered":null},{"message":"you can use `cargo tree` to explore your dependency tree","code":null,"level":"help","spans":[],"children":[],"rendered":null}],"rendered":"error[E0308]: mismatched types\n"}"#;

    #[test]
    fn a_mismatch_with_rustcs_note_about_versions_of_a_crate_is_the_two_copies_case() {
        assert!(
            Diagnostic::parse(TWO_VERSIONS)
                .unwrap()
                .crate_present_twice()
        );
        let e0277 = TWO_VERSIONS.replace("E0308", "E0277").replace(
            "mismatched types",
            "the trait bound `dep::W: T` is not satisfied",
        );
        assert!(Diagnostic::parse(&e0277).unwrap().crate_present_twice());
    }

    #[test]
    fn nothing_but_the_compilers_note_makes_the_two_copies_case() {
        let ordinary = |message: &str, code: &str, spans: &str, rendered: &str| {
            Diagnostic::parse(&format!(
                r#"{{"message":"{message}","code":{{"code":"{code}"}},"level":"error","spans":[{spans}],"children":[],"rendered":"{rendered}"}}"#
            ))
            .unwrap()
        };
        // An E0308 without the note, as the unrelated diagnostics of the same build.
        assert!(!ordinary("mismatched types", "E0308", "", "error[E0308]").crate_present_twice());
        // The phrase in an unrelated message, a quoted source line and a path.
        let phrase = "there are multiple different versions of crate `dep` in the dependency graph";
        assert!(!ordinary(phrase, "E0308", "", "error[E0308]").crate_present_twice());
        assert!(
            !ordinary(
                "mismatched types",
                "E0308",
                "",
                &format!("error[E0308]\\n1 | // {phrase}")
            )
            .crate_present_twice()
        );
        let in_path =
            format!(r#"{{"file_name":"mods/{phrase}/src/lib.rs","line_start":1,"line_end":1}}"#);
        assert!(
            !ordinary("mismatched types", "E0308", &in_path, "error[E0308]").crate_present_twice()
        );
        // The note on another error, and a different note on the right one.
        let mut other = TWO_VERSIONS.replace("E0308", "E0599");
        assert!(!Diagnostic::parse(&other).unwrap().crate_present_twice());
        other = TWO_VERSIONS.replace(
            "there are multiple different versions of crate",
            "two different versions of crate",
        );
        assert!(!Diagnostic::parse(&other).unwrap().crate_present_twice());
    }
}
