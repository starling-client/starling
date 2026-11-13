//! Starling: the native mixin bundle 0.8.0 read (`horizon-native.json`, sources under
//! `mixins/<id>/`), which the registry's index publishes as `.hnmod` and Starling's Android build
//! service compiles. Horizon's own native mods are flat packages ([`super::Bundle`]); a phone takes
//! both. Reading a bundle never executes its code.
use super::Facts;
use crate::index::valid_id;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, path::Path};

pub const MANIFEST: &str = "horizon-native.json";

/// Why a native mod may not carry `mod.wasm`: WebAssembly mods don't run any more.
pub const WASM_GONE: &str = "mod.wasm: WebAssembly mods don't run any more, port the code to a native mod (docs/native-mods.md)";

/// Whether the mod file at `path` is such a bundle: it carries [`MANIFEST`].
pub fn holds(path: &Path) -> bool {
    std::fs::File::open(path)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .is_some_and(|mut zip| zip.by_name(MANIFEST).is_ok())
}

/// Whether a bundle's entry `name` is its content's (`horizon-mod.toml`, `assets/`, `data/`),
/// which the native build doesn't take.
fn content(name: &str) -> bool {
    name == crate::manifest::MANIFEST || name.starts_with("assets/") || name.starts_with("data/")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub id: String,
    pub version: String,
    pub game: String,
    /// Exact source revisions tested together, never mutable tags.
    pub horizon: String,
    pub mixins: String,
    /// The version of Horizon's native hook points a mod that hooks nothing else needs: built on
    /// any Horizon with that version or a later one, `horizon` being the revision it was tested on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<u32>,
}

pub fn revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Manifest {
    pub fn check(&self) -> Result<(), String> {
        if self.schema != 1
            || !valid_id(&self.id)
            || self.version.is_empty()
            || self.game.is_empty()
            || !revision(&self.horizon)
            || !revision(&self.mixins)
            || self.hooks == Some(0)
        {
            return Err(
                "invalid native manifest: schema 1, id, version, game and exact revisions required"
                    .into(),
            );
        }
        Ok(())
    }

    /// The exact native compatibility contract, in the index's API field.
    pub fn api(&self) -> String {
        format!("native-1/{}/{}", self.horizon, self.mixins)
    }

    pub fn facts(&self) -> Facts {
        Facts {
            id: Some(self.id.clone()),
            version: Some(self.version.clone()),
            api: self.api(),
            game: Some(self.game.clone()),
            ..Facts::default()
        }
    }
}

pub struct Bundle {
    pub manifest: Manifest,
    /// Validated relative paths: Rust and TOML, plus files under `mixins/<id>/data/` or a `data/`
    /// folder deeper in (a crate's). The content's files are not among them.
    pub files: BTreeMap<String, Vec<u8>>,
}

/// Whether `rest` (a path inside `mixins/<id>/`) is a data file: under the bundle's `data/`, or
/// under the top-level `data/` of a folder of it (a crate's, `engine/data/`).
fn in_data(rest: &str) -> bool {
    let parts: Vec<&str> = rest.split('/').collect();
    parts.len() >= 2 && (parts[0] == "data" || (parts.len() >= 3 && parts[1] == "data"))
}

/// Whether `rest` is one of the text files a crate folder usually carries, which a crate may need
/// to compile (`#![doc = include_str!("../README.md")]`), in the bundle's folder or a top-level
/// folder of it.
fn text(rest: &str) -> bool {
    let parts: Vec<&str> = rest.split('/').collect();
    let name = parts[parts.len() - 1];
    parts.len() <= 2
        && (["README", "LICENSE", "LICENCE", "COPYING", "NOTICE"]
            .iter()
            .any(|p| name.starts_with(p))
            || name == ".gitignore")
}

/// Whether a bundle may carry the file `rest` (its path under `mixins/<id>/`).
pub fn allowed(rest: &str) -> bool {
    rest.ends_with(".rs") || rest.ends_with(".toml") || in_data(rest) || text(rest)
}

impl Bundle {
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut files = BTreeMap::new();
        let mut content_id = None;
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            if entry.is_dir() {
                continue;
            }
            if entry
                .unix_mode()
                .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
                || name.contains('\\')
                || name
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == "..")
                || name.starts_with('/')
                || name.contains(':')
            {
                return Err(format!("invalid native bundle path: {name}"));
            }
            if name == crate::manifest::WASM {
                return Err(WASM_GONE.into());
            }
            if content(&name) {
                if name == crate::manifest::MANIFEST {
                    let mut text = String::new();
                    (&mut entry)
                        .take(1 << 20)
                        .read_to_string(&mut text)
                        .map_err(|e| format!("{name}: {e}"))?;
                    // An older content manifest this build's schema refuses still names its mod.
                    content_id = crate::Manifest::parse(&text).ok().map(|m| m.id);
                }
                continue;
            }
            if files.len() >= 512 {
                return Err("native bundle has too many entries".into());
            }
            let mut bytes = Vec::new();
            (&mut entry)
                .take(crate::index::MAX_FILE + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            total += bytes.len() as u64;
            if total > crate::index::MAX_FILE {
                return Err("native bundle expands beyond 64 MiB".into());
            }
            if files.insert(name, bytes).is_some() {
                return Err("duplicate native bundle path".into());
            }
        }
        let manifest: Manifest =
            serde_json::from_slice(files.get(MANIFEST).ok_or("missing horizon-native.json")?)
                .map_err(|e| e.to_string())?;
        manifest.check()?;
        if let Some(id) = content_id.filter(|id| *id != manifest.id) {
            return Err(format!(
                "its {} is mod {id}, its {MANIFEST} {}: one file is one mod",
                crate::manifest::MANIFEST,
                manifest.id
            ));
        }
        let prefix = format!("mixins/{}/", manifest.id);
        for name in files.keys().filter(|n| n.as_str() != MANIFEST) {
            let Some(rest) = name.strip_prefix(&prefix) else {
                return Err(format!("unexpected native source: {name}"));
            };
            if !allowed(rest) {
                return Err(format!("unexpected native source: {name}"));
            }
        }
        let source = files
            .get(&format!("{prefix}horizon-mixin.toml"))
            .ok_or("missing horizon-mixin.toml")?;
        let source: toml::Value =
            toml::from_str(std::str::from_utf8(source).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let field = |table: &str, key: &str| {
            source
                .get(table)
                .and_then(|v| v.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        };
        if field("mod", "id").as_ref() != Some(&manifest.id)
            || field("mod", "version").as_ref() != Some(&manifest.version)
            || field("compatibility", "horizon_revision").as_ref() != Some(&manifest.horizon)
        {
            return Err("native manifest and mixin identity/revision differ".into());
        }
        Ok(Self { manifest, files })
    }

    /// The bundle as the Android build service needs it: its manifest and sources, without the
    /// content only the game reads. The same files always make the same bytes, which is how the
    /// service knows it built them before.
    pub fn for_build(&self) -> Result<Vec<u8>, String> {
        use std::io::Write as _;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        for (name, bytes) in &self.files {
            zip.start_file(name.as_str(), options)
                .map_err(|e| e.to_string())?;
            zip.write_all(bytes).map_err(|e| e.to_string())?;
        }
        Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn manifest() -> Manifest {
        Manifest {
            schema: 1,
            id: "veinminer".into(),
            version: "0.3.0".into(),
            game: "26.3".into(),
            horizon: "a".repeat(40),
            mixins: "b".repeat(40),
            hooks: None,
        }
    }
    fn write(name: &str, extras: &[(&str, &[u8])], mixin: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("legacy-bundle-{name}-{}.hnmod", std::process::id()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(MANIFEST, options).unwrap();
        zip.write_all(&serde_json::to_vec(&manifest()).unwrap())
            .unwrap();
        zip.start_file("mixins/veinminer/horizon-mixin.toml", options)
            .unwrap();
        zip.write_all(mixin.as_bytes()).unwrap();
        for (name, bytes) in extras {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }
    fn read(name: &str, extras: &[(&str, &[u8])], mixin: &str) -> Result<Bundle, String> {
        let path = write(name, extras, mixin);
        let result = Bundle::read(&path);
        std::fs::remove_file(path).unwrap();
        result
    }
    fn mixin() -> String {
        format!(
            "[mod]\nid = \"veinminer\"\nversion = \"0.3.0\"\n[compatibility]\nhorizon_revision = \"{}\"\n",
            "a".repeat(40)
        )
    }

    #[test]
    fn a_bundle_carries_its_exact_compatibility_into_index_facts() {
        let bundle = read(
            "valid",
            &[("mixins/veinminer/hook.rs", b"fn hook() {}")],
            &mixin(),
        )
        .unwrap();
        assert_eq!(bundle.manifest, manifest());
        let facts = bundle.manifest.facts();
        assert_eq!(
            facts.api,
            format!("native-1/{}/{}", "a".repeat(40), "b".repeat(40))
        );
        assert_eq!(facts.id.as_deref(), Some("veinminer"));
    }

    /// What Starling's build service reads (`mod_build_server.py`'s `read_bundle`): the manifest
    /// and the sources, the content left out, the same bytes each time.
    #[test]
    fn what_goes_to_the_build_service_is_the_manifest_and_sources() {
        let big = [7u8; 4096];
        let bundle = read(
            "for-build",
            &[
                ("mixins/veinminer/hook.rs", b"fn hook() {}"),
                ("assets/veinminer/big.bin", &big),
                ("data/veinminer/loot.json", b"{}"),
            ],
            &mixin(),
        )
        .unwrap();
        let sent = bundle.for_build().unwrap();
        assert_eq!(sent, bundle.for_build().unwrap());
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&sent)).unwrap();
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "horizon-native.json",
                "mixins/veinminer/hook.rs",
                "mixins/veinminer/horizon-mixin.toml"
            ]
        );
    }

    #[test]
    fn it_refuses_escaping_paths_and_payloads_that_are_not_source() {
        for (i, name) in [
            "../escape.rs",
            "/absolute.rs",
            "mixins/other/hook.rs",
            "mixins/veinminer/code.dll",
            "mixins/veinminer/database/file.bin",
            "mixins\\veinminer\\hook.rs",
        ]
        .iter()
        .enumerate()
        {
            assert!(read(&format!("bad-path-{i}"), &[(name, b"bad")], &mixin()).is_err());
        }
        let e = read("wasm", &[("mod.wasm", b"\0asm")], &mixin())
            .err()
            .unwrap();
        assert_eq!(e, WASM_GONE);
        assert!(read("wrong-version", &[], &mixin().replace("0.3.0", "0.2.0")).is_err());
    }

    /// A phone reads either kind ([`super::super::Any`]), and an index check takes this one.
    #[test]
    fn either_kind_reads_and_inspects_as_a_native_mod() {
        let path = write(
            "any",
            &[("mixins/veinminer/hook.rs", b"fn hook() {}")],
            &mixin(),
        );
        assert!(holds(&path) && super::super::holds(&path));
        let any = super::super::Any::read(&path).unwrap();
        assert_eq!((any.id(), any.version()), ("veinminer", "0.3.0"));
        let facts = crate::index::inspect(&crate::index::Kind::NativeMod, &path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(facts.unwrap().id.as_deref(), Some("veinminer"));
    }
}
