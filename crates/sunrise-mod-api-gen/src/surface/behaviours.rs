//! Behaviour records: one file `crates/sunrise-mod-vocab/behaviours/<name>.toml` per promise a mod
//! relies on that is no type or signature. The file name is the id (`behaviour:<name>`).
//!
//! ```toml
//! version = 1
//! doc = "What the engine promises, stated so that a test could check it."
//! tests = ["crates/sunrise-client/src/some_tests.rs::the_test_that_pins_it"]
//! ```
//!
//! [`read`] refuses an unreadable folder or file, bad TOML, an unknown key (by name), a missing or
//! mistyped key, a file name that is not an id, or missing or malformed test references, each
//! with a typed [`BehaviourError`] that names the declaration. A separate test validates the
//! references against the source tree; export does not open the proof files.
use mod_surface::{Behaviour, BehaviourContract};
use std::path::{Component, Path};

const DIRECTORY: &str = "crates/sunrise-mod-vocab/behaviours";

#[derive(Debug)]
struct Declaration {
    name: String,
    doc: String,
    contract: BehaviourContract,
    tests: Vec<Proof>,
}

#[derive(Debug)]
struct Proof {
    path: String,
    function: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BehaviourError {
    /// The folder or a file could not be read.
    Read { file: String, error: String },
    /// Not TOML.
    Toml { file: String, error: String },
    /// A key a behaviour does not have.
    UnknownKey { file: String, key: String },
    /// A key it must have.
    Missing { file: String, key: &'static str },
    /// A key whose value is not what it must be.
    WrongType {
        file: String,
        key: &'static str,
        expected: &'static str,
    },
    /// The file name is not a lowercase id (`a-z`, `0-9`, `_`, starting with a letter).
    NotAnId { file: String },
    /// `tests` lists nothing.
    NoTests { file: String },
    /// A test that is not `<repo-relative path>::<Rust function name>`.
    BadTest { file: String, test: String },
}

impl std::fmt::Display for BehaviourError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { file, error } => write!(f, "behaviour {file}: {error}"),
            Self::Toml { file, error } => write!(f, "behaviour {file} is not TOML: {error}"),
            Self::UnknownKey { file, key } => write!(
                f,
                "behaviour {file} has the unknown key `{key}` (a behaviour has `version`, `doc` and `tests`)"
            ),
            Self::Missing { file, key } => write!(f, "behaviour {file} has no `{key}`"),
            Self::WrongType {
                file,
                key,
                expected,
            } => write!(f, "behaviour {file}: `{key}` must be {expected}"),
            Self::NotAnId { file } => write!(
                f,
                "behaviour {file}: the file name must be an id of lowercase letters, digits and `_`, starting with a letter"
            ),
            Self::NoTests { file } => write!(f, "behaviour {file} names no tests"),
            Self::BadTest { file, test } => write!(
                f,
                "behaviour {file}: test `{test}` must be `<repo-relative path>::<Rust function name>`"
            ),
        }
    }
}

impl std::error::Error for BehaviourError {}

/// Reads one declaration: `file` is its file name (`<name>.toml`) and `text` its contents.
fn parse(file: &str, text: &str) -> Result<Declaration, BehaviourError> {
    let name = file.strip_suffix(".toml").unwrap_or(file);
    if !name.starts_with(|c: char| c.is_ascii_lowercase())
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(BehaviourError::NotAnId { file: file.into() });
    }
    let table: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| BehaviourError::Toml {
            file: file.into(),
            error: e.message().to_owned(),
        })?;
    if let Some(key) = table
        .keys()
        .find(|k| !["version", "doc", "tests"].contains(&k.as_str()))
    {
        return Err(BehaviourError::UnknownKey {
            file: file.into(),
            key: key.clone(),
        });
    }
    let wrong = |key, expected| BehaviourError::WrongType {
        file: file.into(),
        key,
        expected,
    };
    let get = |key| {
        table.get(key).ok_or(BehaviourError::Missing {
            file: file.into(),
            key,
        })
    };
    let version = get("version")?
        .as_integer()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v >= 1)
        .ok_or_else(|| wrong("version", "a whole number from 1"))?;
    let doc = get("doc")?
        .as_str()
        .ok_or_else(|| wrong("doc", "text"))?
        .to_owned();
    let references = get("tests")?
        .as_array()
        .ok_or_else(|| wrong("tests", "a list of tests"))?;
    if references.is_empty() {
        return Err(BehaviourError::NoTests { file: file.into() });
    }
    let mut tests = Vec::with_capacity(references.len());
    for reference in references {
        let test = reference
            .as_str()
            .ok_or_else(|| wrong("tests", "a list of tests"))?;
        let (path, function) = test
            .rsplit_once("::")
            .filter(|(path, function)| {
                !path.is_empty()
                    && !path.contains(['\\', ':', '\0'])
                    && Path::new(path)
                        .components()
                        .all(|part| matches!(part, Component::Normal(_)))
                    && syn::parse_str::<syn::Ident>(function).is_ok_and(|ident| ident == *function)
            })
            .ok_or_else(|| BehaviourError::BadTest {
                file: file.into(),
                test: test.into(),
            })?;
        tests.push(Proof {
            path: path.into(),
            function: function.into(),
        });
    }
    Ok(Declaration {
        name: name.into(),
        doc,
        contract: BehaviourContract { version },
        tests,
    })
}

/// Every behaviour of the vocabulary by file name; none when the folder is absent.
pub fn read(root: &Path) -> Result<Vec<Behaviour>, BehaviourError> {
    Ok(declarations(&root.join(DIRECTORY))?
        .into_iter()
        .map(|decl| {
            Behaviour::new(
                format!("behaviour:{}", decl.name),
                decl.doc,
                decl.tests
                    .into_iter()
                    .map(|proof| format!("{}::{}", proof.path, proof.function))
                    .collect(),
                decl.contract,
            )
        })
        .collect())
}

fn declarations(dir: &Path) -> Result<Vec<Declaration>, BehaviourError> {
    let error = |file: &Path, e: std::io::Error| BehaviourError::Read {
        file: file.display().to_string(),
        error: e.to_string(),
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(error(dir, e)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| error(dir, e))?.path();
        if path.extension().is_some_and(|e| e == "toml") {
            paths.push(path);
        }
    }
    paths.sort();
    paths
        .iter()
        .map(|path| {
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = std::fs::read_to_string(path).map_err(|e| error(path, e))?;
            parse(&file, &text)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str =
        "version = 2\ndoc = \"It holds.\"\ntests = [\"a/b.rs::holds\", \"c.rs::also\"]\n";

    /// Whether `items` hold a function called `name`, in an inline module too.
    fn declares(items: &[syn::Item], name: &str) -> bool {
        items.iter().any(|item| match item {
            syn::Item::Fn(function) => function.sig.ident == name,
            syn::Item::Mod(module) => module
                .content
                .as_ref()
                .is_some_and(|(_, items)| declares(items, name)),
            _ => false,
        })
    }

    /// Validate links published in the behaviour records. This intentionally reads engine files.
    fn check_proofs(root: &Path) -> Result<(), String> {
        for decl in declarations(&root.join(DIRECTORY)).map_err(|e| e.to_string())? {
            for proof in decl.tests {
                let file = root.join(&proof.path);
                let context = format!(
                    "behaviour {}.toml: {}::{}",
                    decl.name, proof.path, proof.function
                );
                let text = std::fs::read_to_string(&file)
                    .map_err(|e| format!("{context}: cannot read test file: {e}"))?;
                let parsed = syn::parse_file(&text)
                    .map_err(|e| format!("{context}: test file is not Rust: {e}"))?;
                if !declares(&parsed.items, &proof.function) {
                    return Err(format!("{context}: no `fn` for test"));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn published_behaviour_proofs_exist() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        check_proofs(&root).unwrap();
    }

    #[test]
    fn malformed_behaviours_are_refused_with_a_typed_error_naming_the_file() {
        let file = || "b.toml".to_owned();
        let wrong = |key, expected| BehaviourError::WrongType {
            file: file(),
            key,
            expected,
        };
        for (name, text, expected) in [
            (
                "1b.toml",
                GOOD,
                BehaviourError::NotAnId {
                    file: "1b.toml".into(),
                },
            ),
            (
                "B.toml",
                GOOD,
                BehaviourError::NotAnId {
                    file: "B.toml".into(),
                },
            ),
            (
                "a-b.toml",
                GOOD,
                BehaviourError::NotAnId {
                    file: "a-b.toml".into(),
                },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = [\"a.rs::t\"]\nwhy = \"x\"\n",
                BehaviourError::UnknownKey {
                    file: file(),
                    key: "why".into(),
                },
            ),
            (
                "b.toml",
                "doc = \"d\"\ntests = [\"a.rs::t\"]\n",
                BehaviourError::Missing {
                    file: file(),
                    key: "version",
                },
            ),
            (
                "b.toml",
                "version = 1\ntests = [\"a.rs::t\"]\n",
                BehaviourError::Missing {
                    file: file(),
                    key: "doc",
                },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\n",
                BehaviourError::Missing {
                    file: file(),
                    key: "tests",
                },
            ),
            (
                "b.toml",
                "version = 0\ndoc = \"d\"\ntests = [\"a.rs::t\"]\n",
                wrong("version", "a whole number from 1"),
            ),
            (
                "b.toml",
                "version = 1\ndoc = 3\ntests = [\"a.rs::t\"]\n",
                wrong("doc", "text"),
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = \"a.rs::t\"\n",
                wrong("tests", "a list of tests"),
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = []\n",
                BehaviourError::NoTests { file: file() },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = [\"just_a_name\"]\n",
                BehaviourError::BadTest {
                    file: file(),
                    test: "just_a_name".into(),
                },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = [\"/a.rs::pins\"]\n",
                BehaviourError::BadTest {
                    file: file(),
                    test: "/a.rs::pins".into(),
                },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = [\"dir/../../a.rs::pins\"]\n",
                BehaviourError::BadTest {
                    file: file(),
                    test: "dir/../../a.rs::pins".into(),
                },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = [\"C:/a.rs::pins\"]\n",
                BehaviourError::BadTest {
                    file: file(),
                    test: "C:/a.rs::pins".into(),
                },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = [\"a\\u0000.rs::pins\"]\n",
                BehaviourError::BadTest {
                    file: file(),
                    test: "a\0.rs::pins".into(),
                },
            ),
            (
                "b.toml",
                "version = 1\ndoc = \"d\"\ntests = [\"a.rs::not a function\"]\n",
                BehaviourError::BadTest {
                    file: file(),
                    test: "a.rs::not a function".into(),
                },
            ),
        ] {
            assert_eq!(parse(name, text).unwrap_err(), expected, "{name}: {text}");
        }
        assert!(matches!(
            parse("b.toml", "version = "),
            Err(BehaviourError::Toml { .. })
        ));
    }

    #[test]
    fn a_test_is_found_by_parsing_for_an_fn_item() {
        let text = "// fn commented() {}\nconst S: &str = \"fn quoted\";\n#[test]\nfn\n spaced() {}\nfn holds_for_all() {}\n#[cfg(test)]\nmod tests { mod deeper { fn nested() {} } }\n";
        let items = syn::parse_file(text).unwrap().items;
        for name in ["spaced", "holds_for_all", "nested"] {
            assert!(declares(&items, name), "{name}");
        }
        for name in ["commented", "quoted", "holds_for", "hold", "tests"] {
            assert!(!declares(&items, name), "{name}");
        }
    }

    #[test]
    fn export_keeps_the_proof_links_without_reading_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(read(root).unwrap().is_empty());

        let folder = root.join(DIRECTORY);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("zeta.toml"), GOOD).unwrap();
        std::fs::write(folder.join("alpha.toml"), GOOD).unwrap();
        std::fs::write(folder.join("README.md"), "not a behaviour").unwrap();
        let behaviours = read(root).unwrap();
        let ids: Vec<_> = behaviours.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["behaviour:alpha", "behaviour:zeta"]);
        assert_eq!(behaviours[0].contract.version, 2);
        assert_eq!(behaviours[0].doc, "It holds.");
        assert_eq!(behaviours[0].tests, ["a/b.rs::holds", "c.rs::also"]);

        // Keep one proof valid so each refusal is due to the other one, not a later proof.
        std::fs::write(root.join("c.rs"), "mod tests { fn also() {} }").unwrap();
        assert!(check_proofs(root).is_err());
        std::fs::create_dir(root.join("a")).unwrap();
        std::fs::write(root.join("a/b.rs"), "fn (").unwrap();
        assert!(check_proofs(root).is_err());
        std::fs::write(root.join("a/b.rs"), "fn holds_for_all() {}").unwrap();
        assert!(check_proofs(root).is_err());
        std::fs::write(root.join("a/b.rs"), "fn holds() {}").unwrap();
        check_proofs(root).unwrap();
    }
}
