//! The SDK's build-pinned data (`build-data.tar.xz`): `internals.json`,
//! `loops.json` and `functions.json`, written by the staging pass and read by the launcher and the
//! mod tools. Each file opens with the same header, `schema` and `build.commit`, so a file names
//! the game build it describes and a reader refuses a schema it does not know, by name, before
//! reading the rest.
//! No compatibility promise beyond that: the data is exact to its build.
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// The build-data schema this source writes and reads. A new field does not bump it; a removed
/// or redefined one does.
pub const SCHEMA: u32 = 1;
/// The staged internals' signatures.
pub const INTERNALS: &str = "internals.json";
/// The published loops.
pub const LOOPS: &str = "loops.json";
/// The function index: where each function a mixin may target is, and whether it can take a
/// replacement.
pub const FUNCTIONS: &str = "functions.json";
/// The files, in the order the archive holds them.
pub const FILES: [&str; 3] = [FUNCTIONS, INTERNALS, LOOPS];
/// The release asset holding them.
pub const ARCHIVE: &str = "build-data.tar.xz";

/// Token-stream SHA-256 digests of a game function, independent of its source span.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionDigests {
    /// The function's declaration, including its enclosing impl when present.
    pub declaration: String,
    /// The function block.
    pub body: String,
}

/// The build a file describes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    /// The game's full commit sha.
    pub commit: String,
}

/// A build-data file: the header, then the file's own fields.
#[derive(Serialize)]
pub struct Document<'a, T: Serialize> {
    pub schema: u32,
    pub build: &'a Build,
    #[serde(flatten)]
    pub body: T,
}

impl<'a, T: Serialize> Document<'a, T> {
    /// The file for `build` holding `body`, at this source's schema.
    pub fn new(build: &'a Build, body: T) -> Self {
        Self {
            schema: SCHEMA,
            build,
            body,
        }
    }
}

#[derive(Debug)]
pub enum ReadError {
    /// An unknown numbered schema with a readable build identity.
    Schema {
        file: String,
        schema: u64,
        build: Build,
    },
    Invalid(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schema { file, schema, .. } => write!(
                f,
                "{file} has build-data schema {schema}; this program reads schema {SCHEMA}. Use the tool and launcher made for that game build"
            ),
            Self::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for ReadError {}

/// Read the build-data file `file` from `text`. The build identity is readable even on unknown
/// schemas; the body is read only on the supported schema. `T` names the file's own fields.
pub fn read<T: DeserializeOwned>(file: &str, text: &str) -> Result<(Build, T), ReadError> {
    #[derive(Deserialize)]
    struct Header {
        schema: Option<u64>,
    }
    #[derive(Deserialize)]
    struct WithBuild<T> {
        build: Build,
        #[serde(flatten)]
        body: T,
    }
    let invalid = |e| ReadError::Invalid(format!("{file}: {e}"));
    let header: Header = serde_json::from_str(text).map_err(invalid)?;
    match header.schema {
        Some(schema) if schema == u64::from(SCHEMA) => {}
        Some(schema) => {
            #[derive(Deserialize)]
            struct Named {
                build: Build,
            }
            let Named { build } = serde_json::from_str(text).map_err(invalid)?;
            return Err(ReadError::Schema {
                file: file.into(),
                schema,
                build,
            });
        }
        None => {
            return Err(ReadError::Invalid(format!(
                "{file} has no build-data schema; it was written by an older game build. Fetch this build's SDK again"
            )));
        }
    }
    let document: WithBuild<T> = serde_json::from_str(text).map_err(invalid)?;
    Ok((document.build, document.body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Loops {
        loops: Vec<String>,
    }

    #[test]
    fn a_written_file_reads_back_with_its_build() {
        let build = Build {
            commit: "c".repeat(40),
        };
        let text = serde_json::to_string(&Document::new(
            &build,
            Loops {
                loops: vec!["tick".into()],
            },
        ))
        .unwrap();
        assert!(
            text.starts_with(r#"{"schema":1,"build":{"commit":"#),
            "{text}"
        );
        let (read_build, body) = read::<Loops>(LOOPS, &text).unwrap();
        assert_eq!(read_build, build);
        assert_eq!(body.loops, ["tick"]);
    }

    #[test]
    fn an_unknown_or_missing_schema_is_refused_by_name() {
        let commit = "c".repeat(40);
        let later = format!(r#"{{"schema":2,"build":{{"commit":"{commit}"}},"loops":[]}}"#);
        let error = read::<Loops>(LOOPS, &later).unwrap_err();
        assert!(
            matches!(&error, ReadError::Schema { file, schema: 2, build }
            if file == LOOPS && build.commit == commit),
            "{error}"
        );
        let error = read::<Loops>(LOOPS, r#"{"loops":[]}"#).unwrap_err();
        assert!(
            matches!(&error, ReadError::Invalid(reason)
            if reason.contains("loops.json has no build-data schema")),
            "{error}"
        );
        let error = read::<Loops>(LOOPS, "[]").unwrap_err();
        assert!(
            matches!(&error, ReadError::Invalid(reason)
            if reason.starts_with("loops.json: ")),
            "{error}"
        );
        for schema in [SCHEMA, SCHEMA + 1] {
            let headless = format!(r#"{{"schema":{schema},"loops":[]}}"#);
            let error = read::<Loops>(LOOPS, &headless).unwrap_err();
            assert!(
                matches!(&error, ReadError::Invalid(reason)
                if reason.contains("build")),
                "{error}"
            );
        }
    }
}
