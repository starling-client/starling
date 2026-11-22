//! A package's runtime data files: the `runtime/` folder and the `[runtime]` table that lists
//! each file with its SHA-512.
//!
//! Runtime files are the bytes a mod's code opens beside itself (a compiled script bundle, a baked
//! lookup table). They are not pack data: no datapack or resource pack can replace them and no
//! reload touches them.
use super::hash::{sha512_hex, sha512_of};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The folder of a package that holds its runtime files.
pub const RUNTIME_DIR: &str = "runtime";

/// A `[runtime]` table: each file's name under `runtime/` and its SHA-512.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(try_from = "BTreeMap<String, String>")]
pub struct RuntimeTable(BTreeMap<String, String>);

impl TryFrom<BTreeMap<String, String>> for RuntimeTable {
    type Error = String;

    fn try_from(files: BTreeMap<String, String>) -> Result<Self, Self::Error> {
        if files.is_empty() {
            return Err("[runtime] lists no file".into());
        }
        for (name, hash) in &files {
            if !super::plain_path(name) {
                return Err(format!("[runtime] {name}: not a plain relative path"));
            }
            sha512_hex(hash).map_err(|e| format!("[runtime] {name} {e}"))?;
        }
        Ok(Self(files))
    }
}

impl RuntimeTable {
    /// Each listed file's name under `runtime/` and its SHA-512, in name order.
    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, hash)| (name.as_str(), hash.as_str()))
    }

    /// Whether the table lists `name`.
    pub fn lists(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// What two installations compare: a SHA-256 over every listed name and SHA-512, as 64 hex
    /// digits. Equal digests mean equal files, since each hash was checked against its bytes
    /// when the package opened.
    pub fn digest(&self) -> String {
        let mut sha = Sha256::new();
        for (name, hash) in &self.0 {
            sha.update(name.as_bytes());
            sha.update([0]);
            sha.update(hash.as_bytes());
            sha.update(b"\n");
        }
        sha.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Refuse a package whose `runtime/` holds a file this table does not list or lacks one it
    /// lists. `found` is each file under `runtime/` as `(name below it, bytes)`.
    pub fn check_layout(table: Option<&Self>, found: &[(String, u64)]) -> Result<(), String> {
        for (name, _) in found {
            if !table.is_some_and(|table| table.lists(name)) {
                return Err(format!("{RUNTIME_DIR}/{name}: not listed in [runtime]"));
            }
        }
        if let Some(table) = table
            && let Some(missing) = table
                .0
                .keys()
                .find(|name| !found.iter().any(|(have, _)| have == *name))
        {
            return Err(format!(
                "[runtime] lists {missing}, but the package has no {RUNTIME_DIR}/{missing}"
            ));
        }
        Ok(())
    }

    /// Refuse `bytes` that are not what the table lists for `name`.
    pub fn check(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let Some(hash) = self.0.get(name) else {
            return Err(format!("{RUNTIME_DIR}/{name}: not listed in [runtime]"));
        };
        if sha512_of(bytes) != *hash {
            return Err(format!(
                "{RUNTIME_DIR}/{name} doesn't match its [runtime] sha512"
            ));
        }
        Ok(())
    }
}

/// A package's runtime files, each checked against its hash when the package was opened. Reads
/// borrow the retained bytes and never hash again. Clones share the verified data.
#[derive(Clone, Debug)]
pub struct RuntimeData(Arc<RuntimeContents>);

#[derive(Debug)]
struct RuntimeContents {
    // Stored separately to lend all the sorted names as a slice. Indices match `files`.
    names: Vec<String>,
    files: Vec<RuntimeFile>,
    digest: String,
}

#[derive(Debug)]
pub(super) struct RuntimeFile {
    pub(super) bytes: Box<[u8]>,
    pub(super) path: Option<PathBuf>,
}

impl RuntimeData {
    /// `files` arrives in name order from the checked [`RuntimeTable`].
    pub(super) fn new(files: Vec<(String, RuntimeFile)>, digest: String) -> Self {
        let (names, files) = files.into_iter().unzip();
        Self(Arc::new(RuntimeContents {
            names,
            files,
            digest,
        }))
    }

    /// The verified bytes of `name` below `runtime/` and, for a package folder, its live disk
    /// path. `None` for a name the manifest does not list; an archive's path is always `None`.
    pub fn file(&self, name: &str) -> Option<(&[u8], Option<&Path>)> {
        let at = self
            .0
            .names
            .binary_search_by(|listed| listed.as_str().cmp(name))
            .ok()?;
        let file = &self.0.files[at];
        Some((&file.bytes, file.path.as_deref()))
    }

    /// The names of the files, in order.
    pub fn names(&self) -> &[String] {
        &self.0.names
    }

    /// [`RuntimeTable::digest`] of the table these files were checked against.
    pub fn digest(&self) -> &str {
        &self.0.digest
    }
}
