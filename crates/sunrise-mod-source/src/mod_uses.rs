//! The Rust sources scanned for a mod's uses table.
use crate::package;
use mod_surface::scan::{Token, tokens};
use std::path::{Path, PathBuf};

/// The largest Rust source file read, in bytes; a larger one is refused, never truncated.
pub const MAX_SOURCE_BYTES: u64 = 8 << 20;
/// The most Rust source read from one mod, in bytes.
pub const MAX_TOTAL_BYTES: u64 = 64 << 20;
/// The most Rust source files read from one mod.
pub const MAX_FILES: usize = 4096;
/// The largest `Cargo.toml` read to find the mod's library root, in bytes.
const MAX_CARGO_TOML_BYTES: u64 = 64 << 10;

/// Why a mod folder cannot be judged.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    /// A file or folder could not be read.
    Io { path: PathBuf, error: String },
    /// A symbolic link in the folder: what it points to is not the mod's own.
    Symlink(PathBuf),
    /// A `#[path]` or `include!` in `file` brings in source from outside the folder, or from a
    /// place the scan cannot know.
    Escapes { file: String, target: String },
    /// A `#[path]` or `include!` in `file` names a file of the folder that is not in the scanned
    /// Rust set: not a `.rs` file the scan selects, or missing.
    NotScanned { file: String, target: String },
    /// The mod's library root (`[lib].path` in its `Cargo.toml`, or `src/lib.rs`) is one of the
    /// files the scan was told to leave out, so the root's own uses would go unread.
    RootSkipped(String),
    /// More source than is read: a file, or the whole mod.
    TooLarge { what: String, size: u64, most: u64 },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Symlink(path) => write!(
                f,
                "{} is a symbolic link; the mod's source cannot be judged",
                path.display()
            ),
            Self::Escapes { file, target } => write!(
                f,
                "{file} brings in `{target}`, outside the mod's folder; the mod's source cannot be judged"
            ),
            Self::NotScanned { file, target } => write!(
                f,
                "{file} brings in `{target}`, which the scan does not read; the mod's source cannot be judged"
            ),
            Self::RootSkipped(file) => write!(
                f,
                "{file} is the mod's library root and also a file it hands to the game; move one of them to its own file so the mod's source can be judged"
            ),
            Self::TooLarge { what, size, most } => {
                write!(f, "{what} is {size} bytes; the most read is {most}")
            }
        }
    }
}

impl std::error::Error for ReadError {}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ReadError + '_ {
    move |error| ReadError::Io {
        path: path.to_path_buf(),
        error: error.to_string(),
    }
}

/// `target` joined to `base` (both relative to the mod's folder), `/`-separated, if it stays
/// inside the folder.
fn inside(base: &[String], target: &str) -> Option<String> {
    if target.starts_with('/') || target.starts_with('\\') || target.contains(':') {
        return None;
    }
    let mut parts: Vec<&str> = base.iter().map(String::as_str).collect();
    for component in target.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    Some(parts.join("/"))
}

/// The text of a string literal token as Rust reads it: `"…"` with its escapes, or a raw string
/// `r"…"` / `r#"…"#`. `None` for any other literal or an escape this does not know.
fn decode(literal: &str) -> Option<String> {
    if let Some(raw) = literal.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let body = raw.get(hashes..raw.len().checked_sub(hashes)?)?;
        return body.strip_prefix('"')?.strip_suffix('"').map(str::to_owned);
    }
    let body = literal.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            'x' => {
                let code =
                    u8::from_str_radix(&chars.by_ref().take(2).collect::<String>(), 16).ok()?;
                out.push(char::from(code).is_ascii().then_some(char::from(code))?);
            }
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let digits: String = chars.by_ref().take_while(|c| *c != '}').collect();
                out.push(char::from_u32(
                    u32::from_str_radix(&digits.replace('_', ""), 16).ok()?,
                )?);
            }
            '\n' => {
                // A line continuation: the newline and the whitespace after it are skipped.
                let rest: String = chars.collect();
                out.push_str(&decode(&format!("\"{}\"", rest.trim_start()))?);
                return Some(out);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// The token at `at`. Every read of the token list in [`in_attributes`] and [`references`] goes
/// through here, so a test can count them: the scan reads each token a bounded number of times.
fn token<'a>(list: &'a [Token<'a>], at: usize) -> Option<&'a Token<'a>> {
    #[cfg(test)]
    tests::READS.with(|reads| reads.set(reads.get() + 1));
    list.get(at)
}

/// For each token, whether it sits inside an attribute's brackets, `#[…]` or `#![…]`, at any
/// depth. One pass.
fn in_attributes(list: &[Token<'_>]) -> Vec<bool> {
    let mut inside = Vec::with_capacity(list.len());
    // Each open bracket, and whether it is or lies inside an attribute's.
    let mut open: Vec<bool> = Vec::new();
    for at in 0..list.len() {
        let outer = open.last().copied().unwrap_or(false);
        match token(list, at) {
            Some(Token::Punct(bracket @ (b'(' | b'[' | b'{'))) => {
                let attribute = *bracket == b'['
                    && at >= 1
                    && (token(list, at - 1) == Some(&Token::Punct(b'#'))
                        || (at >= 2
                            && token(list, at - 1) == Some(&Token::Punct(b'!'))
                            && token(list, at - 2) == Some(&Token::Punct(b'#'))));
                inside.push(outer);
                open.push(outer || attribute);
            }
            Some(Token::Punct(b')' | b']' | b'}')) => {
                open.pop();
                inside.push(outer);
            }
            _ => inside.push(outer),
        }
    }
    inside
}

/// Every file `text` brings in with `#[path = "…"]` or `include!(…)` (any delimiter), each as a
/// path relative to the mod's folder, given the file's folder. `Err` names the first one that
/// leaves the folder or that the scan cannot read: a computed or undecodable path, an `include!`
/// renamed by a `use` (`use std::include as inc;`), or a
/// `#[path]` inside braces (an inline module, whose files nest by its name).
fn references(text: &str, folder: &[String]) -> Result<Vec<String>, String> {
    let list: Vec<Token<'_>> = tokens(text).map(|(t, _)| t).collect();
    let in_attribute = in_attributes(&list);
    let mut found = Vec::new();
    let mut depth = 0usize;
    let resolve = |at: usize, found: &mut Vec<String>| match token(&list, at) {
        Some(Token::Literal(literal)) => match decode(literal) {
            Some(target) => match inside(folder, &target) {
                Some(path) => {
                    found.push(path);
                    Ok(())
                }
                None => Err(target),
            },
            None => Err(format!("{literal}, a path the scan cannot read")),
        },
        _ => Err("a path the scan cannot read".to_owned()),
    };
    for at in 0..list.len() {
        match token(&list, at) {
            Some(Token::Punct(b'{')) => depth += 1,
            Some(Token::Punct(b'}')) => depth = depth.saturating_sub(1),
            // `path = "…"` anywhere inside an attribute, `cfg_attr(…, path = …)` included.
            Some(Token::Ident("path"))
                if token(&list, at + 1) == Some(&Token::Punct(b'='))
                    && token(&list, at + 2) != Some(&Token::Punct(b'='))
                    && in_attribute[at] =>
            {
                if depth > 0 {
                    return Err("a #[path] inside braces".to_owned());
                }
                resolve(at + 2, &mut found)?;
            }
            // `use std::include as inc;`: an `include!` the scan would not see by its name.
            Some(Token::Ident("include")) if token(&list, at + 1) == Some(&Token::Ident("as")) => {
                return Err("`include!` under another name".to_owned());
            }
            Some(Token::Ident("include")) if token(&list, at + 1) == Some(&Token::Punct(b'!')) => {
                let close = match token(&list, at + 2) {
                    Some(Token::Punct(b'(')) => b')',
                    Some(Token::Punct(b'[')) => b']',
                    Some(Token::Punct(b'{')) => b'}',
                    _ => continue,
                };
                if token(&list, at + 4) != Some(&Token::Punct(close)) {
                    return Err("a computed path".to_owned());
                }
                resolve(at + 3, &mut found)?;
            }
            _ => {}
        }
    }
    Ok(found)
}

/// The file Cargo compiles as the mod's library: `[lib].path` of the `Cargo.toml` in `dir`,
/// relative and `/`-separated without `.` parts, or `src/lib.rs` when the manifest names none
/// (or there is no manifest to read: the build then says so).
fn library_root(dir: &Path) -> Result<String, ReadError> {
    let manifest = dir.join("Cargo.toml");
    let named = if manifest.is_file() {
        let text = read_bounded(&manifest, "Cargo.toml", MAX_CARGO_TOML_BYTES)?;
        toml::from_str::<toml::Table>(&text)
            .ok()
            .and_then(|table| Some(table.get("lib")?.get("path")?.as_str()?.to_owned()))
    } else {
        None
    };
    Ok(named.map_or_else(
        || "src/lib.rs".to_owned(),
        |path| {
            path.split(['/', '\\'])
                .filter(|part| !part.is_empty() && *part != ".")
                .collect::<Vec<_>>()
                .join("/")
        },
    ))
}

/// The first of `skip` that a `mod name;` in `text`, a file in `folder`, may load as the mod's
/// own source: a file below that folder named for the module (`name.rs`, `name/mod.rs`). Where
/// the module's file lies is not resolved, so one of that name anywhere below counts.
fn declares_skipped<'s>(text: &str, folder: &[String], skip: &[&'s str]) -> Option<&'s str> {
    let list: Vec<Token<'_>> = tokens(text).map(|(t, _)| t).collect();
    let below = folder
        .iter()
        .map(|part| format!("{part}/"))
        .collect::<String>();
    list.windows(3).find_map(|window| {
        let [Token::Ident("mod"), Token::Ident(name), Token::Punct(b';')] = window else {
            return None;
        };
        skip.iter().copied().find(|path| {
            let Some(rest) = path.strip_prefix(below.as_str()) else {
                return false;
            };
            let mut parts = rest.rsplit('/');
            let module = match parts.next() {
                Some("mod.rs") => parts.next(),
                file => file.and_then(|file| file.strip_suffix(".rs")),
            };
            module == Some(*name)
        })
    })
}

/// Reads at most `most` bytes of `path`, refusing a larger file by its size.
fn read_bounded(path: &Path, shown: &str, most: u64) -> Result<String, ReadError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(most + 1).read_to_end(&mut bytes))
        .map_err(io(path))?;
    if bytes.len() as u64 > most {
        return Err(ReadError::TooLarge {
            what: shown.to_owned(),
            size: bytes.len() as u64,
            most,
        });
    }
    String::from_utf8(bytes).map_err(|e| ReadError::Io {
        path: path.to_path_buf(),
        error: e.to_string(),
    })
}

/// Reads `dir`: the `.rs` files its package carries ([`package::source_files`], the selection
/// packing makes), but not the files `skip` names
/// (relative and `/`-separated), which are not the mod's own source. A symbolic link among them,
/// or a linked `src/`, is refused; any other entry that is not a regular file is left unread.
/// A `#[path]` or `include!` of a file the scan does not read, a skipped one included, is
/// refused, and so is a `mod` declaration that may load a skipped file, and so is a skipped file
/// that is the mod's library root.
pub fn read_sources(dir: &Path, skip: &[&str]) -> Result<Vec<(String, String)>, ReadError> {
    let root = library_root(dir)?;
    if skip.contains(&root.as_str()) {
        return Err(ReadError::RootSkipped(root));
    }
    let mut sources = Vec::new();
    let mut total = 0u64;
    let mut brought: Vec<(String, String)> = Vec::new();
    // A linked `src/` is not walked by the selection, so it is refused here, not read as empty.
    let src = dir.join("src");
    if std::fs::symlink_metadata(&src).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(ReadError::Symlink(src));
    }
    for shown in package::source_files(dir).map_err(io(dir))? {
        if !shown.ends_with(".rs") || skip.contains(&shown.as_str()) {
            continue;
        }
        let path = dir.join(&shown);
        let kind = std::fs::symlink_metadata(&path)
            .map_err(io(&path))?
            .file_type();
        if kind.is_symlink() {
            return Err(ReadError::Symlink(path));
        }
        if !kind.is_file() {
            continue;
        }
        let relative: Vec<String> = shown.split('/').map(str::to_owned).collect();
        let text = read_bounded(&path, &shown, MAX_SOURCE_BYTES)?;
        total += text.len() as u64;
        if total > MAX_TOTAL_BYTES || sources.len() >= MAX_FILES {
            return Err(ReadError::TooLarge {
                what: format!("the mod's source ({} files)", sources.len() + 1),
                size: total,
                most: MAX_TOTAL_BYTES,
            });
        }
        if let Some(target) = declares_skipped(&text, &relative[..relative.len() - 1], skip) {
            return Err(ReadError::NotScanned {
                file: shown,
                target: target.to_owned(),
            });
        }
        match references(&text, &relative[..relative.len() - 1]) {
            Ok(paths) => brought.extend(paths.into_iter().map(|p| (shown.clone(), p))),
            Err(target) => {
                return Err(ReadError::Escapes {
                    file: shown,
                    target,
                });
            }
        }
        sources.push((shown, text));
    }
    sources.sort();
    for (file, target) in brought {
        if sources
            .binary_search_by(|(path, _)| path.as_str().cmp(&target))
            .is_err()
        {
            return Err(ReadError::NotScanned { file, target });
        }
    }
    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    std::thread_local! {
        /// Token reads by [`token`] on this thread.
        pub(super) static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn reads_the_packaged_sources() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/lib.rs", "a");
        write(root, "src/deep/b.rs", "b");
        write(root, "build.rs", "c");
        write(root, "target/debug/build/x.rs", "no");
        write(root, "src/target/aim.rs", "no");
        write(root, "tools/generate.rs", "no");
        write(root, ".sunrise/sdk/api/lib.rs", "no");
        write(root, "src/notes.txt", "no");
        let input = read_sources(root, &[]).unwrap();
        let sources: Vec<(&str, &str)> = input
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        assert_eq!(
            sources,
            [
                ("build.rs", "c"),
                ("src/deep/b.rs", "b"),
                ("src/lib.rs", "a")
            ]
        );
    }

    #[test]
    fn cargo_toml_and_each_source_are_read_with_a_cap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let big = format!("[lib]\n# {}\n", "x".repeat(MAX_CARGO_TOML_BYTES as usize));
        write(root, "Cargo.toml", &big);
        assert!(matches!(
            read_sources(root, &[]).unwrap_err(),
            ReadError::TooLarge { what, most: MAX_CARGO_TOML_BYTES, .. } if what == "Cargo.toml"
        ));
        write(root, "Cargo.toml", "[lib]\n");
        write(
            root,
            "src/lib.rs",
            &"/".repeat(MAX_SOURCE_BYTES as usize + 1),
        );
        assert!(matches!(
            read_sources(root, &[]).unwrap_err(),
            ReadError::TooLarge { what, most: MAX_SOURCE_BYTES, .. } if what == "src/lib.rs"
        ));
    }

    #[test]
    fn a_skipped_file_that_is_the_library_root_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/lib.rs", "");
        write(root, "src/entry.rs", "");
        // The default root, with and without a manifest that names none.
        let refused = |skip: &str| read_sources(root, &[skip]).unwrap_err();
        assert_eq!(
            refused("src/lib.rs"),
            ReadError::RootSkipped("src/lib.rs".into())
        );
        write(root, "Cargo.toml", "[package]\nname = \"m\"\n");
        assert_eq!(
            refused("src/lib.rs"),
            ReadError::RootSkipped("src/lib.rs".into())
        );
        // A named root, however its path is written; the default is then an ordinary file.
        write(root, "Cargo.toml", "[lib]\npath = \"./src//entry.rs\"\n");
        assert_eq!(
            refused("src/entry.rs"),
            ReadError::RootSkipped("src/entry.rs".into())
        );
        let scanned = read_sources(root, &["src/lib.rs"]).unwrap();
        let files: Vec<_> = scanned.iter().map(|(file, _)| file.as_str()).collect();
        assert_eq!(files, ["src/entry.rs"]);
    }

    #[test]
    fn source_from_outside_the_folder_cannot_be_judged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Inside the folder and scanned: fine, with any delimiter and a decoded or raw literal.
        write(root, "src/gen/x.rs", "");
        write(root, "src/y.rs", "");
        write(
            root,
            "src/lib.rs",
            "#[path = \"gen/x.rs\"] mod x;\ninclude!(\"../src/y.rs\");\ninclude! { r#\"y.rs\"# }\ninclude![\"\\x79.rs\"];\ninclude!(\"\\u{79}.rs\");\n// #[path = \"/etc\"]",
        );
        assert!(read_sources(root, &[]).is_ok());
        let escapes = |text: &str| {
            write(root, "src/lib.rs", text);
            read_sources(root, &[]).unwrap_err()
        };
        // Escapes decoded before the check, with every delimiter.
        for text in [
            "include!(\"./\\x2e\\x2e/\\x2e\\x2e/shared.rs\");",
            "include!(\"\\u{2e}\\u{2e}/../shared.rs\");",
            "include! { \"../../shared.rs\" }",
            "include![\"../../shared.rs\"];",
            "#[path = r\"../../shared.rs\"] mod x;",
        ] {
            assert!(matches!(escapes(text), ReadError::Escapes { .. }), "{text}");
        }
        // An escape the scan cannot decode cannot be judged.
        assert!(matches!(
            escapes("include!(\"\\q.rs\");"),
            ReadError::Escapes { .. }
        ));
        // A file of the folder the scan does not read cannot be judged.
        write(root, "src/part.inc", "");
        write(root, "target/gen.rs", "");
        write(root, ".hidden/gen.rs", "");
        for (text, target) in [
            ("include!(\"part.inc\");", "src/part.inc"),
            ("include!(\"../target/gen.rs\");", "target/gen.rs"),
            ("#[path = \"../.hidden/gen.rs\"] mod gen;", ".hidden/gen.rs"),
            ("include!(\"missing.rs\");", "src/missing.rs"),
        ] {
            assert_eq!(
                escapes(text),
                ReadError::NotScanned {
                    file: "src/lib.rs".into(),
                    target: target.into()
                }
            );
        }
        // `include!` under another name is not seen by its name: not judged.
        for text in [
            "use std::include as inc;\ninc!(\"y.rs\");",
            "use core::{include as inc};\ninc!(\"../../shared/gen.rs\");",
        ] {
            assert!(matches!(escapes(text), ReadError::Escapes { .. }), "{text}");
        }
        // Many `path = …` statements outside attributes are read in one pass: each token is read
        // a bounded number of times, not once per earlier token.
        let many = format!("fn f() {{ {} }}", "path = x; ".repeat(10_000));
        let count = tokens(&many).count();
        write(root, "src/lib.rs", &many);
        READS.with(|reads| reads.set(0));
        assert!(read_sources(root, &[]).is_ok());
        let reads = READS.with(std::cell::Cell::get);
        assert!(reads <= 3 * count, "{reads} token reads for {count} tokens");
        // A `#[path]` inside an inline module nests by its name: not judged.
        assert!(matches!(
            escapes("mod outer { #[path = \"y.rs\"] mod inner; }"),
            ReadError::Escapes { .. }
        ));
        assert_eq!(
            escapes("#[path = \"../../other/x.rs\"] mod x;"),
            ReadError::Escapes {
                file: "src/lib.rs".into(),
                target: "../../other/x.rs".into()
            }
        );
        assert!(matches!(
            escapes("#[path = \"/abs/x.rs\"] mod x;"),
            ReadError::Escapes { .. }
        ));
        assert!(matches!(
            escapes("#[path = \"C:\\\\x.rs\"] mod x;"),
            ReadError::Escapes { .. }
        ));
        assert_eq!(
            escapes("#[cfg_attr(all(), path = \"../../elsewhere/x.rs\")] mod x;"),
            ReadError::Escapes {
                file: "src/lib.rs".into(),
                target: "../../elsewhere/x.rs".into()
            }
        );
        assert_eq!(
            escapes("include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"));"),
            ReadError::Escapes {
                file: "src/lib.rs".into(),
                target: "a computed path".into()
            }
        );
    }

    #[test]
    fn a_deep_folder_tree_is_read_without_recursion() {
        let dir = tempfile::tempdir().unwrap();
        let deep = "m/".repeat(200);
        write(dir.path(), &format!("src/{deep}x.rs"), "x");
        let input = read_sources(dir.path(), &[]).unwrap();
        assert_eq!(input, [(format!("src/{deep}x.rs"), "x".to_owned())]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_cannot_be_judged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/lib.rs", "a");
        std::os::unix::fs::symlink(root.join("src/lib.rs"), root.join("src/linked.rs")).unwrap();
        assert_eq!(
            read_sources(root, &[]).unwrap_err(),
            ReadError::Symlink(root.join("src/linked.rs"))
        );
        // A linked `src/` itself is refused, not scanned as empty.
        let linked = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(root.join("src"), linked.path().join("src")).unwrap();
        assert_eq!(
            read_sources(linked.path(), &[]).unwrap_err(),
            ReadError::Symlink(linked.path().join("src"))
        );
    }

    /// A file packing leaves out is not read, so its contents cannot stop the scan.
    #[cfg(unix)]
    #[test]
    fn files_packing_leaves_out_are_not_read() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/lib.rs", "a");
        // Packed, and Cargo's library root: read.
        write(root, "README.rs", "r");
        write(
            root,
            "Cargo.toml",
            "[package]\nname = \"m\"\n[lib]\npath = \"README.rs\"\n",
        );
        let not_text = [0xff_u8, 0xfe];
        for name in [
            &b"src/a\\b.rs"[..],
            b"src/\xff.rs",
            b"node_modules/x.rs",
            b"gen.rs",
            b"build.rs/x.rs",
        ] {
            let path = root.join(std::ffi::OsStr::from_bytes(name));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, not_text).unwrap();
        }
        std::os::unix::fs::symlink(root.join("src"), root.join("linked")).unwrap();
        // A selected name that is not a regular file is not opened, so it cannot block the scan.
        let _socket = std::os::unix::net::UnixListener::bind(root.join("src/sock.rs")).unwrap();
        let input = read_sources(root, &[]).unwrap();
        assert_eq!(
            input,
            [
                ("README.rs".to_owned(), "r".to_owned()),
                ("src/lib.rs".to_owned(), "a".to_owned())
            ]
        );
    }
}
