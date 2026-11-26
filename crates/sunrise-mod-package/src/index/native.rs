//! Source-only native packages. Reading a package never executes its code.
use super::Facts;
use mod_surface::uses::USES_FILE;
use std::{collections::BTreeMap, io::Read, path::Path};

/// Where the prebuilt game (#589) is released: one `prebuilt-<commit>` prerelease per daily build
/// of this fork's main (Diverges from upstream: horizon-rust/horizon has none).
pub const PREBUILT_REPO: &str = "sunrise-org/sunrise";

/// Whether a valid package is native, irrespective of its install folder.
pub fn holds(path: &Path) -> bool {
    crate::Package::read(path).is_ok_and(|package| package.is_native())
}

/// A flat native mod and its checked, archive-relative files.
pub struct Bundle {
    pub manifest: crate::Manifest,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Bundle {
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let package = crate::Package::read_zip(&mut zip)?;
        if !package.is_native() {
            return Err("not a native package".into());
        }
        // An install is the first open: the runtime files' hashes are checked here, before the
        // download is accepted.
        package.read_runtime(path)?;
        // The game skips what it does not read; the tools that stage the files refuse what is no
        // part of a package, and bound the source before inflating any of it.
        crate::Package::check_source_bounds(&mut zip)?;
        let mut files = BTreeMap::new();
        let mut lowercase = BTreeMap::<String, String>::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            if let Some(other) = lowercase.insert(name.to_lowercase(), name.clone()) {
                return Err(format!(
                    "{other} and {name} differ only by case, which a Windows or macOS extraction cannot keep apart"
                ));
            }
            // TOLERANCE (#5973), the only one: `build` from Nov 25 until #5974 packed the root
            // registry page as source. Only the root `page.toml` and what is under the root
            // `page/` are skipped, after every check above, so no digest changes; any other
            // unexpected path is still refused below. Remove this skip and its tests once no
            // published package carries them, by 26.3-0.1.0 at the latest (#5985).
            if name == "page.toml" || name.starts_with("page/") {
                log::warn!(
                    "{}: skipped {name}, a registry page entry an older build packed as source",
                    package.manifest.id
                );
                continue;
            }
            let source = crate::manifest::source_file(&name);
            let own = name == crate::manifest::MANIFEST
                || name == USES_FILE
                || name == "icon.png"
                || ["assets/", "data/", "runtime/"]
                    .iter()
                    .any(|folder| name.starts_with(folder));
            if !source && !own {
                return Err(format!("unexpected package path: {name}"));
            }
            let size = entry.size();
            let mut bytes = Vec::new();
            // One byte more than declared, so an entry longer than its header says is seen, not cut.
            (&mut entry)
                .take(size.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|e| format!("{name}: {e}"))?;
            if bytes.len() as u64 != size {
                return Err(format!("{name}: invalid expanded size"));
            }
            // The game never opens a source file; the tools that do (the registry's checks, the
            // launcher's build) refuse prebuilt object code here.
            if source
                && let Some(kind) = crate::manifest::binary_kind(&bytes[..bytes.len().min(1024)])
            {
                return Err(crate::manifest::ContentFileError::Binary { name, kind }.to_string());
            }
            files.insert(name, bytes);
        }
        Ok(Self {
            manifest: package.manifest,
            files,
        })
    }

    /// Registry facts from the one package manifest, never Cargo metadata.
    pub fn facts(&self) -> Facts {
        let native = self.manifest.native.as_ref().expect("native bundle");
        Facts {
            id: Some(self.manifest.id.clone()),
            version: Some(self.manifest.version.to_string()),
            // No SDK is at hand here: a package `cargo sunrise-mod build` made carries the
            // revision its SDK named, and one without it lists the pin as written.
            api: match &native.mixins {
                Some(mixins) => format!("native-1/{}/{mixins}", native.sunrise),
                None => format!("native-1/{}", native.sunrise),
            },
            game: Some(native.game.clone()),
            sunrise_pin: Some(native.sunrise.clone()),
            dependencies: super::manifest_dependencies(&self.manifest),
            ..Facts::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn manifest() -> String {
        format!(
            "id = 'veinminer'\nname = 'Veinminer'\nversion = '0.3.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        )
    }

    fn read(name: &str, manifest: &str, extras: &[(&str, &[u8])]) -> Result<Bundle, String> {
        let path =
            std::env::temp_dir().join(format!("native-bundle-{name}-{}.smod", std::process::id()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in [
            (crate::manifest::MANIFEST, manifest.as_bytes()),
            (
                "Cargo.toml",
                b"[package]\nname='other'\nversion='9'\n".as_slice(),
            ),
            ("src/lib.rs", b"fn hook() {}".as_slice()),
        ]
        .into_iter()
        .chain(extras.iter().copied())
        {
            if name.ends_with('/') {
                zip.add_directory(name, options).unwrap();
            } else {
                zip.start_file(name, options).unwrap();
                zip.write_all(bytes).unwrap();
            }
        }
        zip.finish().unwrap();
        let result = Bundle::read(&path);
        std::fs::remove_file(path).unwrap();
        result
    }

    #[test]
    fn an_install_refuses_runtime_bytes_that_do_not_match_the_manifest() {
        let manifest = format!(
            "{}[runtime]\n\"nav.bin\"='{}'\n",
            manifest(),
            crate::manifest::sha512_of(b"tables")
        );
        let bundle = read("runtime-ok", &manifest, &[("runtime/nav.bin", b"tables")]).unwrap();
        assert_eq!(bundle.files["runtime/nav.bin"], b"tables");
        // The same name and size, different bytes.
        let error = read(
            "runtime-edited",
            &manifest,
            &[("runtime/nav.bin", b"tablez")],
        )
        .err()
        .unwrap();
        assert!(error.contains("runtime/nav.bin"), "{error}");
    }

    #[test]
    fn facts_use_manifest_identity_and_exact_build_pins() {
        let manifest = format!("{}[dependencies]\ncreature = '^0.1'\n", manifest());
        let bundle = read("facts", &manifest, &[]).unwrap();
        let facts = bundle.facts();
        assert_eq!(
            serde_json::to_value(&facts.dependencies).unwrap(),
            serde_json::json!([
                {"id": "creature", "version": "^0.1"}
            ])
        );
        assert_eq!(facts.id.as_deref(), Some("veinminer"));
        assert_eq!(facts.version.as_deref(), Some("0.3.0"));
        assert_eq!(
            facts.api,
            format!("native-1/{}/{}", "a".repeat(40), "b".repeat(40))
        );
        assert_eq!(facts.game.as_deref(), Some("26.3"));
    }

    #[test]
    fn package_carries_resources_and_code_together() {
        let bytes = [0, 255, 42];
        let bundle = read(
            "resources",
            &manifest(),
            &[
                ("data/veinminer/table.bin", &bytes),
                ("assets/veinminer/lang/en_us.json", b"{}"),
            ],
        )
        .unwrap();
        assert_eq!(bundle.files["data/veinminer/table.bin"], bytes);
        assert_eq!(bundle.files["src/lib.rs"], b"fn hook() {}");
        assert!(bundle.files.contains_key(crate::manifest::MANIFEST));
    }

    #[test]
    fn an_empty_archive_directory_is_allowed_and_any_other_file_is_carried_as_source() {
        let manifest = manifest();
        let bundle = read("empty-dirs", &manifest, &[("native/", b""), ("rust/", b"")]).unwrap();
        assert_eq!(bundle.manifest.id, "veinminer");
        assert_eq!(bundle.files.len(), 3);
        for (i, name) in [
            "sunrise-native.json",
            "native/rust/src/lib.rs",
            "vendor/zlib/inflate.c",
        ]
        .into_iter()
        .enumerate()
        {
            let bundle = read(&format!("carried-{i}"), &manifest, &[(name, b"int x;")]).unwrap();
            assert_eq!(bundle.files[name], b"int x;");
        }
    }

    #[test]
    fn a_prebuilt_binary_among_the_source_is_refused_by_its_bytes() {
        for (i, bytes) in [&b"\x7fELF\x02\x01"[..], b"!<arch>\nlib.o/"]
            .into_iter()
            .enumerate()
        {
            let error = read(
                &format!("binary-{i}"),
                &manifest(),
                &[("vendor/lib.c", bytes)],
            )
            .err()
            .unwrap();
            assert!(error.starts_with("vendor/lib.c: "), "{error}");
            assert!(error.contains("prebuilt binaries"), "{error}");
        }
    }

    #[test]
    fn the_tools_refuse_entries_outside_the_file_set_that_the_game_skips() {
        for (i, (name, bytes)) in [
            ("target/x.so", &b"\x7fELF\x02\x01"[..]),
            (".cargo/config.toml", b"[build]\nrustflags = []\n"),
        ]
        .into_iter()
        .enumerate()
        {
            let extras = [(name, bytes)];
            let error = read(&format!("outside-{i}"), &manifest(), &extras)
                .err()
                .unwrap();
            assert!(error.contains(name), "{error}");
            // The game's reader skips such an entry without error.
            let path = std::env::temp_dir().join(format!(
                "native-bundle-game-{i}-{}.smod",
                std::process::id()
            ));
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for (name, bytes) in [
                (crate::manifest::MANIFEST, manifest().into_bytes()),
                (
                    "Cargo.toml",
                    b"[package]\nname='other'\nversion='9'\n".to_vec(),
                ),
                ("src/lib.rs", b"fn hook() {}".to_vec()),
                (name, bytes.to_vec()),
            ] {
                zip.start_file(name, options).unwrap();
                zip.write_all(&bytes).unwrap();
            }
            zip.finish().unwrap();
            let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
            assert!(crate::Package::read_zip(&mut zip).is_ok(), "{name}");
            std::fs::remove_file(path).unwrap();
        }
    }

    // The three tests of the #5973 tolerance; remove them with it (#5985).
    #[test]
    fn an_already_built_package_with_root_page_entries_reads_without_them() {
        let bundle = read(
            "page",
            &manifest(),
            &[
                ("page.toml", b"summary = 'x'"),
                ("page/runes.jpg", b"\xff\xd8\xff"),
                ("page/more/shot.png", b"\x89PNG"),
            ],
        )
        .unwrap();
        let names: Vec<_> = bundle.files.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            ["Cargo.toml", "src/lib.rs", crate::manifest::MANIFEST]
        );
    }

    #[test]
    fn nested_page_paths_are_not_touched_by_the_page_tolerance() {
        let nested: [(&str, &[u8]); 4] = [
            ("src/page.rs", b"fn page() {}"),
            ("src/page/mod.rs", b"fn page() {}"),
            ("assets/veinminer/page/info.json", b"{}"),
            ("data/veinminer/page/table.json", b"{}"),
        ];
        let bundle = read("nested-page", &manifest(), &nested).unwrap();
        for (name, bytes) in nested {
            assert_eq!(bundle.files[name], bytes, "{name}");
        }
    }

    #[test]
    fn another_unexpected_root_path_is_still_refused_beside_page_entries() {
        let error = read(
            "page-and-other",
            &manifest(),
            &[
                ("page.toml", b"summary = 'x'"),
                (".cargo/config.toml", b"[build]\n"),
            ],
        )
        .err()
        .unwrap();
        assert_eq!(error, "unexpected package path: .cargo/config.toml");
    }

    #[test]
    fn names_that_differ_only_by_case_are_refused_by_both_names() {
        let error = read(
            "case",
            &manifest(),
            &[("src/Evil.rs", b"a"), ("src/evil.rs", b"b")],
        )
        .err()
        .unwrap();
        assert!(
            error.contains("src/Evil.rs") && error.contains("src/evil.rs"),
            "{error}"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn source_entries_are_counted_before_any_is_inflated() {
        let names: Vec<String> = (0..=crate::manifest::MAX_SOURCE_FILES)
            .map(|i| format!("vendor/f{i}.c"))
            .collect();
        let extras: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
        let error = read("many", &manifest(), &extras).err().unwrap();
        assert_eq!(error, "native package exceeds its entry or size limit");
    }

    #[test]
    fn refuses_unsafe_entries_and_unknown_fields() {
        for (i, name) in ["../escape.rs", "src/../escape.rs"].iter().enumerate() {
            assert!(
                read(&format!("invalid-{i}"), &manifest(), &[(name, b"bad")]).is_err(),
                "{name}"
            );
        }
        assert!(read("unknown", &(manifest() + "unexpected = true\n"), &[]).is_err());
        assert!(read("pin", &manifest().replace(&"a".repeat(40), "main"), &[]).is_err());
        assert!(
            read(
                "magic",
                &manifest(),
                &[("assets/veinminer/file.bin", b"MZ bad")]
            )
            .is_err()
        );
    }

    #[test]
    fn classification_uses_the_validated_package_not_install_folder() {
        let root = std::env::temp_dir().join(format!("native-kind-{}", std::process::id()));
        for (folder, native) in [("content", true), ("native", false)] {
            let dir = root.join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("test.smod");
            let text = manifest();
            let (text, files) = if native {
                (
                    text.as_str(),
                    vec![
                        (
                            "Cargo.toml",
                            b"[package]\nname='example'\nversion='1'".as_slice(),
                        ),
                        ("src/lib.rs", b"fn hook() {}".as_slice()),
                    ],
                )
            } else {
                (text.split("[native]").next().unwrap(), vec![])
            };
            std::fs::write(&path, crate::Package::write_files(text, &files).unwrap()).unwrap();
            assert_eq!(holds(&path), native);
            if native {
                let directory = dir.join("source");
                zip::ZipArchive::new(std::fs::File::open(&path).unwrap())
                    .unwrap()
                    .extract(&directory)
                    .unwrap();
                assert!(holds(&directory));
                std::fs::remove_file(directory.join(crate::manifest::MANIFEST)).unwrap();
                assert!(
                    !holds(&directory),
                    "Cargo alone is not a valid native package"
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
