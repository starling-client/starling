//! The index reader isolates malformed optional rows; writers deserialize the strict types.
use super::{Index, LeftOut, NativeBinary, NativeBinaryRefusal, Project, Version, check_binary};
use serde::Deserialize;
use serde_json::{Value, value::RawValue};
use std::collections::HashMap;

#[derive(Deserialize)]
pub(super) struct ReadIndex {
    #[serde(flatten)]
    index: Index,
    #[serde(default)]
    collections: Vec<Value>,
    #[serde(default)]
    projects: Vec<ReadProject>,
}

#[derive(Deserialize)]
struct ReadProject {
    #[serde(flatten)]
    project: Project,
    #[serde(default)]
    versions: Vec<ReadVersion>,
}

#[derive(Deserialize)]
struct ReadVersion {
    #[serde(flatten)]
    version: Version,
    #[serde(default = "empty_rows")]
    native_binaries: Box<RawValue>,
}

fn empty_rows() -> Box<RawValue> {
    RawValue::from_string("[]".into()).expect("empty JSON array")
}

impl ReadIndex {
    pub(super) fn validated(self, local_files: bool) -> Index {
        let Self {
            mut index,
            projects,
            collections,
        } = self;
        for ReadProject {
            mut project,
            versions,
        } in projects
        {
            for version in versions {
                let version = version.keep_binaries(&project, local_files, &mut index.left_out);
                project.versions.push(version);
            }
            index.projects.push(project);
        }
        for (position, row) in collections.into_iter().enumerate() {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("collections[{position}]"));
            match serde_json::from_value(row) {
                Ok(collection) => index.collections.push(collection),
                Err(error) => index.left_out.push(LeftOut {
                    id,
                    version: None,
                    reason: format!("invalid collection: {error}"),
                }),
            }
        }
        index.keep_valid(local_files);
        index.prune_native_binaries();
        index
    }
}

impl ReadVersion {
    fn keep_binaries(
        self,
        project: &Project,
        local_files: bool,
        left_out: &mut Vec<LeftOut>,
    ) -> Version {
        let Self {
            mut version,
            native_binaries,
        } = self;
        let mut rejected = |reason| left_out.push(LeftOut::version(&project.id, &version, reason));
        let mut refused = Vec::new();
        let mut accepted = Vec::new();
        match serde_json::from_str::<Vec<&RawValue>>(native_binaries.get()) {
            Ok(rows) => {
                #[derive(Deserialize, PartialEq, Eq, Hash)]
                struct Key {
                    build: String,
                    target: String,
                }
                // Read just the coordinates even if the rest of the row is malformed. Ambiguous
                // coordinates (including duplicate fields) cannot identify a refusal for a host.
                let keys: Vec<_> = rows
                    .iter()
                    .map(|row| serde_json::from_str::<Key>(row.get()).ok())
                    .collect();
                let mut counts = HashMap::new();
                for key in keys.iter().flatten() {
                    *counts.entry(key).or_insert(0) += 1;
                }
                for (position, (row, key)) in rows.into_iter().zip(&keys).enumerate() {
                    let result = if key.as_ref().is_some_and(|key| counts[key] > 1) {
                        Err("duplicate native build and target".into())
                    } else {
                        serde_json::from_str::<NativeBinary>(row.get())
                            .map_err(|e| e.to_string())
                            .and_then(|binary| {
                                check_binary(project, &binary, local_files)?;
                                Ok(binary)
                            })
                    };
                    match (result, key) {
                        (Ok(binary), _) => accepted.push(binary),
                        (Err(error), Some(Key { build, target })) => {
                            let reason = format!("native binary {build} / {target}: {error}");
                            rejected(reason.clone());
                            refused.push(NativeBinaryRefusal {
                                build: build.clone(),
                                target: target.clone(),
                                reason,
                            });
                        }
                        (Err(error), None) => {
                            rejected(format!("native_binaries[{position}]: {error}"))
                        }
                    }
                }
            }
            Err(_) => rejected("native_binaries must be a list".into()),
        }
        version.native_binaries = accepted;
        version.native_binary_refusals = refused;
        version
    }
}
