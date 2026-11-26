//! Library identities are produced by compilation, then hashed at publication.
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};
use sunrise_mod_package::index::registry::builds::library_identity;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Library {
    pub file: String,
    pub sha256: String,
    pub kind: String,
    #[serde(rename = "mod")]
    pub mod_id: String,
    pub identity: String,
}

/// The recorded identity of each library built so far, by mod id: what its dependents are built
/// against.
#[derive(Default)]
pub struct Identities(HashMap<String, String>);

impl Identities {
    /// Mod `id`'s library identity as its entry exports it, without the panic strategy the library
    /// appends: built for `build` from the files `source` digests, against the native mods among
    /// `linked` built before it.
    pub fn next<'a>(
        &mut self,
        build: &str,
        id: &str,
        version: &str,
        source: &str,
        linked: impl IntoIterator<Item = &'a str>,
    ) -> String {
        let exported = library_identity(
            [build, id, version, crate::rust::VERSION],
            source,
            linked.into_iter().filter_map(|dependency| {
                self.0
                    .get(dependency)
                    .map(|identity| (dependency, identity.as_str()))
            }),
        );
        // Every library, the patch library too, is built with `unwind`: the game unwinds a
        // refused Host call through the mod's frames, and the loader refuses `abort` by name.
        self.0.insert(id.to_owned(), format!("{exported} unwind"));
        exported
    }

    /// The identity mod `id`'s library records, as the build record keeps it.
    pub fn recorded(&self, id: &str) -> &str {
        &self.0[id]
    }
}

pub fn stamp(path: &Path, id: &str, identity: &str, patch: bool) -> Result<(), String> {
    let row = Library {
        file: path
            .file_name()
            .ok_or("invalid library path")?
            .to_string_lossy()
            .into_owned(),
        sha256: String::new(),
        kind: if patch { "patch" } else { "mod" }.into(),
        mod_id: id.into(),
        identity: identity.into(),
    };
    fs::write(
        path.with_extension("identity.json"),
        serde_json::to_vec(&row).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn validate_exports(path: &Path, id: &str) -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    let parse = || -> Result<usize, String> {
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let pe = object::read::pe::PeFile64::parse(bytes.as_slice()).map_err(|e| e.to_string())?;
        Ok(pe
            .export_table()
            .map_err(|e| e.to_string())?
            .map_or(0, |table| table.addresses().len()))
    };
    let count = parse().map_err(|e| {
        crate::host::trf(
            "Could not inspect native mod {mod}'s DLL exports: {error}",
            &[("mod", id), ("error", &e)],
        )
    })?;
    refuse_exports(id, count)
}

fn refuse_exports(id: &str, count: usize) -> Result<(), String> {
    if count > 65_535 {
        return Err(crate::host::trf(
            "Native mod {mod} has {count} DLL exports; Windows permits at most 65,535.",
            &[("mod", id), ("count", &count.to_string())],
        ));
    }
    Ok(())
}

pub fn collect(paths: &[std::path::PathBuf]) -> Result<Vec<Library>, String> {
    use sha2::{Digest, Sha256};
    let mut rows = Vec::new();
    for path in paths {
        let stamp = path.with_extension("identity.json");
        if !stamp.exists() {
            continue;
        }
        let mut row: Library = serde_json::from_slice(&fs::read(stamp).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        row.sha256 = format!(
            "{:x}",
            Sha256::digest(fs::read(path).map_err(|e| e.to_string())?)
        );
        rows.push(row);
    }
    // Stable partition keeps dependency order within the mod libraries.
    rows.sort_by_key(|row| row.kind == "patch");
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_limit_names_the_mod_and_count() {
        assert!(refuse_exports("heavy", 65_535).is_ok());
        let error = refuse_exports("heavy", 65_536).unwrap_err();
        assert!(error.contains("heavy"), "{error}");
        assert!(error.contains("65536"), "{error}");
    }

    /// A library installed with no stamp, like the one the mods' registry crates are linked from,
    /// gets no row: it is loaded as the mods' dependency, not registered as a mod.
    #[test]
    fn an_unstamped_library_is_not_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let (mod_library, shared) = (tmp.path().join("one.dll"), tmp.path().join("shared.dll"));
        for path in [&mod_library, &shared] {
            fs::write(path, b"abc").unwrap();
        }
        stamp(&mod_library, "one", "build one 1", false).unwrap();
        let rows = collect(&[mod_library, shared]).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.file.as_str()).collect::<Vec<_>>(),
            ["one.dll"]
        );
    }

    #[test]
    fn records_hashes_and_dependency_order_with_patch_last() {
        let tmp = tempfile::tempdir().unwrap();
        let paths: Vec<_> = ["dependency", "patch", "dependent"]
            .iter()
            .map(|n| tmp.path().join(n))
            .collect();
        for (i, p) in paths.iter().enumerate() {
            fs::write(p, b"abc").unwrap();
            let id = ["dependency", "sunrise_patch", "dependent"][i];
            stamp(p, id, &format!("identity of {id}"), i == 1).unwrap();
        }
        let rows = collect(&paths).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.mod_id.as_str()).collect::<Vec<_>>(),
            ["dependency", "dependent", "sunrise_patch"]
        );
        assert_eq!(
            rows[0].sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let json = serde_json::to_value(&rows).unwrap();
        assert_eq!(json[0]["mod"], "dependency");
        assert_eq!(json[2]["kind"], "patch");
        assert_eq!(rows[2].identity, "identity of sunrise_patch");
    }

    /// A library's identity names the identity of each native mod it links, so a rebuilt
    /// dependency, or a rebuilt dependency of a dependency, gives its dependents another identity.
    #[test]
    fn an_identity_follows_its_dependencies_transitively() {
        let identities = |base_source: &str| {
            let mut built = Identities::default();
            let mut exported = Vec::new();
            for (id, source, linked) in [
                ("base", base_source, &[][..]),
                ("library", "s2", &["base"][..]),
                // `absent` was not built: an optional dependency left out links nothing.
                ("dependent", "s3", &["library", "absent"][..]),
            ] {
                exported.push(built.next("build", id, "1", source, linked.iter().copied()));
            }
            (exported, built)
        };
        let (exported, built) = identities("s1");
        let rust = crate::rust::VERSION;
        assert_eq!(exported[0], format!("build base 1 {rust} s1 none"));
        assert_eq!(built.recorded("base"), format!("{} unwind", exported[0]));
        let hash = |text: &str| sunrise_mod_package::index::sha256(text.as_bytes());
        assert_eq!(
            exported[2],
            format!(
                "build dependent 1 {rust} s3 library={}",
                hash(built.recorded("library"))
            )
        );
        let (rebuilt, _) = identities("s1b");
        assert!(exported.iter().zip(&rebuilt).all(|(a, b)| a != b));
    }
}
