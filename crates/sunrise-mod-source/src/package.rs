//! Which files of a package are its source, and which of them are refused: the one file list and
//! the one binary rule, shared by the packer (`sunrise_mod_package::Package::write_files`), the
//! uses scan ([`crate::mod_uses`]) and the registry bot. The game's loader never calls them. Walking
//! a mod folder for them (`source_files`) is the `folder` feature, which only the tools enable.
#[cfg(feature = "folder")]
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use mod_surface::uses::USES_FILE;
#[cfg(feature = "folder")]
use std::{io, path::Path};

/// The manifest's file name in a `.smod` or a mod folder.
pub const MANIFEST: &str = "sunrise-mod.toml";

/// The folder of a package's runtime files, which `[runtime]` lists.
pub const RUNTIME_DIR: &str = "runtime";

/// Folders of a mod's project that are never part of its package: Git's, Cargo's build output
/// (only the root `target`, as Cargo skips only that one), Cargo's `.cargo/config.toml` folder,
/// which the build would apply, and the pinned SDK the mod tool fetches into `.sunrise/`.
pub const NOT_PACKAGED: [&str; 4] = [".git", "target", ".sunrise", ".cargo"];

/// The registry page beside the manifest, and the folder of its images: `build` writes them to
/// `<id>.page.tar`, never into the `.smod`. Older builds' readers allow only a short file list
/// and refuse a package that carries them.
pub const PAGE_FILE: &str = "page.toml";
pub const PAGE_DIR: &str = "page";

/// Whether `name` is the root page file or under the root page folder. Only the root ones: a
/// crate's own `src/page/` is source.
pub fn page_file(name: &str) -> bool {
    name == PAGE_FILE
        || name
            .strip_prefix(PAGE_DIR)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether `name` (`/`-separated, relative to the package) is a source file: any path outside
/// [`NOT_PACKAGED`] folders that is not one of the package's own names, which are the manifest,
/// the uses table, the icon, and what is under `runtime/`, `assets/` and `data/`, nor the
/// registry page ([`page_file`]). A name is never judged by its extension; a trailing `/` reads
/// as a file below the folder.
pub fn source_file(name: &str) -> bool {
    let mut parts = name.split('/');
    let first = parts.next().unwrap_or_default();
    !NOT_PACKAGED.contains(&first)
        && !parts.any(|part| part != "target" && NOT_PACKAGED.contains(&part))
        && name != MANIFEST
        && name != USES_FILE
        && name != "icon.png"
        && !page_file(name)
        && ![RUNTIME_DIR, "assets", "data"].iter().any(|folder| {
            name.strip_prefix(folder)
                .is_some_and(|rest| rest.starts_with('/'))
        })
}

/// The kind of prebuilt object code a file is, from its header alone, whatever its name: ELF,
/// Mach-O (32- and 64-bit in both byte orders, and both universal headers), PE, WebAssembly, or
/// an `ar` archive (which covers `.a` and `.lib`). `head` is the file's first 1024 bytes, or the
/// whole file when it is shorter: a PE is told from a text file starting with `MZ` by the `PE\0\0`
/// signature its header points to. This catches a prebuilt binary by its header; it is not a
/// security boundary, since `build.rs` runs code. The tools refuse a source file this names.
pub fn binary_kind(head: &[u8]) -> Option<&'static str> {
    let pe = head.starts_with(b"MZ")
        && head.get(0x3c..0x40).is_some_and(|at| {
            let at = u32::from_le_bytes([at[0], at[1], at[2], at[3]]) as usize;
            head.get(at..at.saturating_add(4)) == Some(&b"PE\0\0"[..])
        });
    match head {
        [0x7f, b'E', b'L', b'F', ..] => Some("ELF"),
        [0xfe, 0xed, 0xfa, 0xce | 0xcf, ..] | [0xce | 0xcf, 0xfa, 0xed, 0xfe, ..] => Some("Mach-O"),
        [0xca, 0xfe, 0xba, 0xbe, ..] | [0xbe, 0xba, 0xfe, 0xca, ..] => {
            Some("Mach-O universal binary or Java class file")
        }
        [0xca, 0xfe, 0xba, 0xbf, ..] | [0xbf, 0xba, 0xfe, 0xca, ..] => {
            Some("Mach-O 64-bit universal binary")
        }
        [0, b'a', b's', b'm', ..] => Some("WebAssembly"),
        _ if pe => Some("PE"),
        _ if head.starts_with(b"!<arch>\n") => Some("ar archive"),
        _ => None,
    }
}

/// The source files a package of the folder at `root` carries, `/`-separated and sorted: the files
/// `cargo package` would put in the crate. The whole folder is walked, but not [`NOT_PACKAGED`]
/// folders, nested packages (a folder holding its own `Cargo.toml`) that no packaged manifest names
/// as a `path` dependency, `.`-named files and folders (unless `include` is set), the names [`source_file`] refuses, or what the root `.gitignore` ignores. A `path` dependency of a
/// packaged manifest that leaves the package, or names a folder the walk leaves out, is refused by
/// the manifest and path (one under `.sunrise/`, the SDK the native build replaces, is not). Two
/// names that differ only by case are refused, since a Windows or macOS extraction keeps one. Then
/// `[package] include` of `Cargo.toml` keeps only the files it matches, or else `[package]
/// exclude` drops the files it matches, both as gitignore-style patterns. `Cargo.toml` and
/// `Cargo.lock` at the root are always kept. No entry whose name a package path cannot hold (not
/// UTF-8, or with a `\` in it) is read. The walk keeps its own stack, so a deep tree needs no
/// recursion.
#[cfg(feature = "folder")]
pub fn source_files(root: &Path) -> io::Result<Vec<String>> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
    let manifest = match std::fs::read_to_string(root.join("Cargo.toml")) {
        Ok(text) => text
            .parse::<toml::Table>()
            .map_err(|e| invalid(format!("Cargo.toml: {e}")))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => toml::Table::new(),
        Err(e) => return Err(e),
    };
    let patterns = |key: &str| -> io::Result<Option<Gitignore>> {
        let Some(value) = manifest.get("package").and_then(|package| package.get(key)) else {
            return Ok(None);
        };
        let list = value
            .as_array()
            .and_then(|list| {
                list.iter()
                    .map(toml::Value::as_str)
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| {
                invalid(format!(
                    "Cargo.toml: package.{key} must be a list of strings"
                ))
            })?;
        // Cargo reads an empty `include` as none.
        if list.is_empty() {
            return Ok(None);
        }
        // Every name matched here is relative to the folder: with an empty root the matcher
        // strips nothing from it, as a root that is itself a relative path would.
        let mut builder = GitignoreBuilder::new("");
        for pattern in list {
            builder.add_line(None, pattern).map_err(|e| {
                invalid(format!(
                    "Cargo.toml: package.{key}: pattern `{pattern}`: {e}"
                ))
            })?;
        }
        builder
            .build()
            .map(Some)
            .map_err(|e| invalid(format!("Cargo.toml: package.{key}: {e}")))
    };
    let include = patterns("include")?;
    let exclude = patterns("exclude")?;
    let mut ignored = GitignoreBuilder::new("");
    if root.join(".gitignore").is_file()
        && let Some(error) = ignored.add(root.join(".gitignore"))
    {
        return Err(invalid(format!(".gitignore: {error}")));
    }
    let ignored = ignored
        .build()
        .map_err(|e| invalid(format!(".gitignore: {e}")))?;
    let packaged = |name: &str, is_dir: bool| {
        if !is_dir && (name == "Cargo.toml" || name == "Cargo.lock") {
            return true;
        }
        if ignored
            .matched_path_or_any_parents(name, is_dir)
            .is_ignore()
        {
            return false;
        }
        match (&include, &exclude) {
            // Cargo checks no folder against `include`, which names files.
            (Some(include), _) => {
                is_dir || include.matched_path_or_any_parents(name, false).is_ignore()
            }
            (None, Some(exclude)) => !exclude
                .matched_path_or_any_parents(name, is_dir)
                .is_ignore(),
            (None, None) => true,
        }
    };

    // Cargo skips `.*` unless `include` is set, which alone then decides.
    let hidden =
        |name: &str| include.is_none() && name.split('/').any(|part| part.starts_with('.'));
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    // Folders a packaged manifest names as a `path` dependency, and the nested packages seen so far.
    let mut wanted = std::collections::HashMap::new();
    let mut nested = Vec::new();
    wanted_folders(&manifest, "", &mut wanted)?;
    loop {
        while let Some((dir, prefix)) = pending.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                let file_name = entry.file_name();
                let Some(file_name) = file_name.to_str().filter(|name| !name.contains('\\')) else {
                    continue;
                };
                let name = if prefix.is_empty() {
                    file_name.to_owned()
                } else {
                    format!("{prefix}/{file_name}")
                };
                if kind.is_dir() {
                    if hidden(&name) || !source_file(&format!("{name}/")) || !packaged(&name, true)
                    {
                        continue;
                    }
                    if entry.path().join("Cargo.toml").exists() {
                        nested.push((entry.path(), name));
                    } else {
                        pending.push((entry.path(), name));
                    }
                } else if source_file(&name) && !hidden(&name) && packaged(&name, false) {
                    found.push(name);
                }
            }
        }
        // A nested package is walked once a packaged manifest names its folder, which can be one
        // found after it, so the list is read again until none is newly named.
        let (named, others): (Vec<_>, Vec<_>) = std::mem::take(&mut nested)
            .into_iter()
            .partition(|(_, name)| wanted.contains_key(name));
        nested = others;
        if named.is_empty() {
            break;
        }
        for (dir, name) in named {
            let text = std::fs::read_to_string(dir.join("Cargo.toml"))?;
            let table = text
                .parse::<toml::Table>()
                .map_err(|e| invalid(format!("{name}/Cargo.toml: {e}")))?;
            wanted_folders(&table, &name, &mut wanted)?;
            pending.push((dir, name));
        }
    }
    refuse_case_twins(&found)?;
    for (folder, dependency) in &wanted {
        if !found.contains(&format!("{folder}/Cargo.toml")) {
            return Err(invalid(format!(
                "{dependency}: the folder `{folder}` is not in the package (a dot-folder, `include`, `exclude`, `.gitignore` or a missing Cargo.toml leaves it out)"
            )));
        }
    }
    found.sort();
    Ok(found)
}

/// Refuses two of `names` that differ only by case: a Windows or macOS extraction keeps one.
#[cfg(feature = "folder")]
fn refuse_case_twins(names: &[String]) -> io::Result<()> {
    let mut lowercase = std::collections::HashMap::new();
    for name in names {
        if let Some(other) = lowercase.insert(name.to_lowercase(), name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{other} and {name} differ only by case, which a Windows or macOS extraction cannot keep apart"
                ),
            ));
        }
    }
    Ok(())
}

/// Adds to `wanted` the folders (relative to the package, `/`-separated) that `manifest`, the
/// `Cargo.toml` of the folder `base`, names as `path` dependencies in `[dependencies]`,
/// `[build-dependencies]`, `[dev-dependencies]` and their `[target.*.*]` forms, each with the
/// manifest and path that name it. A path that leaves the package is refused; one under
/// `.sunrise/` is the SDK, which the native build removes, and is not added.
#[cfg(feature = "folder")]
fn wanted_folders(
    manifest: &toml::Table,
    base: &str,
    wanted: &mut std::collections::HashMap<String, String>,
) -> io::Result<()> {
    const TABLES: [&str; 3] = ["dependencies", "build-dependencies", "dev-dependencies"];
    let mut tables: Vec<&toml::Value> =
        TABLES.iter().filter_map(|key| manifest.get(*key)).collect();
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            tables.extend(TABLES.iter().filter_map(|key| target.get(*key)));
        }
    }
    let label = if base.is_empty() {
        "Cargo.toml".to_owned()
    } else {
        format!("{base}/Cargo.toml")
    };
    for table in tables.into_iter().filter_map(toml::Value::as_table) {
        for (key, dependency) in table {
            let Some(path) = dependency.get("path").and_then(toml::Value::as_str) else {
                continue;
            };
            let mut parts: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
            // An absolute path or a drive leaves the package as `..` does.
            let mut inside =
                !path.starts_with(['/', '\\']) && path.as_bytes().get(1) != Some(&b':');
            for part in path.split(['/', '\\']) {
                match part {
                    "" | "." => {}
                    ".." => inside &= parts.pop().is_some(),
                    part => parts.push(part),
                }
            }
            let named = format!("{label}: dependency `{key}` has path `{path}`");
            if !inside {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{named}, which leaves the package"),
                ));
            }
            // The SDK is named as `.sunrise/sdk/...` from any manifest, and the native build puts it there.
            let sdk = parts.first() == Some(&".sunrise") || path.starts_with(".sunrise/");
            if !sdk && !parts.is_empty() {
                wanted.entry(parts.join("/")).or_insert(named);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{binary_kind, source_file};

    #[test]
    fn a_source_file_is_any_path_but_the_packages_own_and_not_packaged_folders() {
        for name in [
            "Cargo.toml",
            "build.rs",
            "src/ffi/add.c",
            "vendor/zlib/inflate.h",
            "configure",
            "Makefile",
            "tools/gen.sh",
            "assetsx/a.c",
            // Cargo skips only the root `target`.
            "src/target/x.c",
            "vendor/foo/target/x.c",
            // Only the root page is the registry's.
            "src/page/mod.rs",
            "src/page.toml",
            "pages/a.rs",
        ] {
            assert!(source_file(name), "{name}");
        }
        for name in [
            "sunrise-mod.toml",
            "uses.json",
            "icon.png",
            "runtime/table.bin",
            "assets/ruby/lang.json",
            "data/ruby/tags.json",
            "assets/",
            ".git/config",
            "target/debug/x.c",
            ".sunrise/sdk/api/lib.rs",
            ".cargo/config.toml",
            "vendor/.git/config",
            "page.toml",
            "page/runes.jpg",
            "page/",
        ] {
            assert!(!source_file(name), "{name}");
        }
    }

    #[test]
    fn binary_kind_reads_the_first_bytes_whatever_the_name() {
        // A DOS header whose offset 0x3c points at `PE\0\0`.
        let mut pe = vec![0u8; 0x48];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c] = 0x40;
        pe[0x40..0x44].copy_from_slice(b"PE\0\0");
        for (head, kind) in [
            (&b"\x7fELF\x02\x01\x01\0"[..], Some("ELF")),
            (b"\xcf\xfa\xed\xfe\x07\0\0\x01", Some("Mach-O")),
            (b"\xfe\xed\xfa\xce", Some("Mach-O")),
            (
                b"\xca\xfe\xba\xbe\0\0\0\x02",
                Some("Mach-O universal binary or Java class file"),
            ),
            (
                b"\xca\xfe\xba\xbf\0\0\0\x02",
                Some("Mach-O 64-bit universal binary"),
            ),
            (
                b"\xbf\xba\xfe\xca\0\0\0\x02",
                Some("Mach-O 64-bit universal binary"),
            ),
            (b"\0asm\x01\0\0\0", Some("WebAssembly")),
            (&pe, Some("PE")),
            // `MZ` alone, or a pointer that leads nowhere, is text.
            (b"MZ\x90\0", None),
            (b"MZ is a prefix of this C comment", None),
            (&pe[..0x42], None),
            (b"!<arch>\nfoo.o/", Some("ar archive")),
            (b"#include <stdio.h>", None),
            (b"!<arch>", None),
            (b"", None),
        ] {
            assert_eq!(binary_kind(head), kind, "{head:?}");
        }
    }
}

#[cfg(all(test, feature = "folder"))]
mod folder_tests {
    use super::{refuse_case_twins, source_files};
    use std::path::Path;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (name, text) in files {
            let path = root.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        root
    }

    fn names(root: &Path) -> Vec<String> {
        source_files(root).unwrap()
    }

    #[test]
    fn c_sources_and_headers_under_src_and_other_project_files_are_packaged() {
        let root = project(&[
            ("Cargo.toml", "[package]\nname = 'ruby'\n"),
            ("src/lib.rs", ""),
            ("src/ffi/add.c", "int add(int a, int b) { return a + b; }"),
            ("src/ffi/add.h", "int add(int, int);"),
            // A data file the code embeds with `include_str!`, judged by no extension.
            ("src/catalog.json", "{}"),
            ("build.rs", ""),
            ("vendor/zlib/inflate.c", ""),
            // The package's own names are never source.
            ("sunrise-mod.toml", ""),
            ("assets/ruby/lang.json", ""),
            ("data/ruby/tags.json", ""),
            ("runtime/table.bin", ""),
            ("icon.png", ""),
        ]);
        assert_eq!(
            names(root.path()),
            [
                "Cargo.toml",
                "build.rs",
                "src/catalog.json",
                "src/ffi/add.c",
                "src/ffi/add.h",
                "src/lib.rs",
                "vendor/zlib/inflate.c"
            ]
        );
    }

    #[test]
    fn include_names_the_only_files_a_package_carries() {
        let root = project(&[
            (
                "Cargo.toml",
                "[package]\nname = 'ruby'\ninclude = ['src/**', 'shim.c']\n",
            ),
            ("Cargo.lock", ""),
            ("src/lib.rs", ""),
            ("shim.c", ""),
            ("notes.txt", ""),
            ("vendor/zlib/inflate.c", ""),
        ]);
        assert_eq!(
            names(root.path()),
            ["Cargo.lock", "Cargo.toml", "shim.c", "src/lib.rs"]
        );
    }

    #[test]
    fn exclude_keeps_a_file_out() {
        let root = project(&[
            (
                "Cargo.toml",
                "[package]\nname = 'ruby'\nexclude = ['src/bench.c', 'docs/']\n",
            ),
            ("src/lib.rs", ""),
            ("src/bench.c", ""),
            ("src/add.c", ""),
            ("docs/guide.md", ""),
        ]);
        assert_eq!(
            names(root.path()),
            ["Cargo.toml", "src/add.c", "src/lib.rs"]
        );
    }

    #[test]
    fn only_a_nested_package_a_manifest_names_is_packaged_and_gitignored_files_are_left_out() {
        let root = project(&[
            (
                "Cargo.toml",
                "[package]\nname = 'ruby'\n[dependencies]\nhelper = { path = 'src/helper' }\n",
            ),
            (".gitignore", "out/\n*.o\n"),
            ("src/lib.rs", ""),
            (
                "src/helper/Cargo.toml",
                "[package]\nname = 'helper'\n[dependencies]\ndeep = { path = '../deep' }\n",
            ),
            ("src/helper/src/lib.rs", ""),
            ("src/deep/Cargo.toml", "[package]\nname = 'deep'\n"),
            ("src/deep/lib.rs", ""),
            ("src/cache.o", ""),
            ("out/generated.c", ""),
            ("vendor/dep/Cargo.toml", "[package]\nname = 'dep'\n"),
            ("vendor/dep/src/lib.rs", ""),
            ("vendor/plain.c", ""),
            (".cargo/config.toml", ""),
        ]);
        assert_eq!(
            names(root.path()),
            [
                "Cargo.toml",
                "src/deep/Cargo.toml",
                "src/deep/lib.rs",
                "src/helper/Cargo.toml",
                "src/helper/src/lib.rs",
                "src/lib.rs",
                "vendor/plain.c"
            ]
        );
    }

    #[test]
    fn a_path_dependency_that_leaves_the_package_or_is_left_out_is_refused_by_name() {
        for (manifest, files, wanted) in [
            (
                "[dependencies]\nhelper = { path = '../helper' }\n",
                &[][..],
                "Cargo.toml: dependency `helper` has path `../helper`, which leaves the package",
            ),
            (
                "[target.'cfg(unix)'.build-dependencies]\nhelper = { path = '/opt/helper' }\n",
                &[][..],
                "which leaves the package",
            ),
            (
                "[dependencies]\nhelper = { path = 'src/helper' }\n",
                &[(
                    "src/helper/Cargo.toml",
                    "[package]\nname = 'helper'\n[dependencies]\nup = { path = '../../../up' }\n",
                )][..],
                "src/helper/Cargo.toml: dependency `up`",
            ),
            (
                "[dependencies]\nhelper = { path = '.vendor/helper' }\n",
                &[(".vendor/helper/Cargo.toml", "[package]\nname = 'helper'\n")][..],
                "the folder `.vendor/helper` is not in the package",
            ),
        ] {
            let mut all = vec![(
                "Cargo.toml",
                format!("[package]\nname = 'ruby'\n{manifest}"),
            )];
            all.extend(files.iter().map(|(name, text)| (*name, (*text).to_owned())));
            let all: Vec<(&str, &str)> = all.iter().map(|(n, t)| (*n, t.as_str())).collect();
            let error = source_files(project(&all).path()).unwrap_err().to_string();
            assert!(error.contains(wanted), "{error}");
        }
    }

    #[test]
    fn the_sdks_own_path_dependency_is_allowed() {
        let root = project(&[
            (
                "Cargo.toml",
                "[package]\nname = 'ruby'\n[dependencies]\nsunrise-mod-api = { path = '.sunrise/sdk/api/crates/sunrise-mod-api' }\n",
            ),
            ("src/lib.rs", ""),
            (".sunrise/sdk/api/crates/sunrise-mod-api/Cargo.toml", ""),
        ]);
        assert_eq!(names(root.path()), ["Cargo.toml", "src/lib.rs"]);
    }

    #[test]
    fn only_the_root_target_and_never_cargos_own_folder_are_left_out() {
        let root = project(&[
            ("Cargo.toml", "[package]\nname = 'ruby'\ninclude = ['**']\n"),
            ("target/debug/x.c", ""),
            ("src/target/x.c", ""),
            (".cargo/config.toml", ""),
        ]);
        assert_eq!(names(root.path()), ["Cargo.toml", "src/target/x.c"]);
    }

    #[test]
    fn names_that_differ_only_by_case_are_refused() {
        // A list, not a folder: Windows' and macOS's file systems cannot hold both names.
        let names = ["Cargo.toml", "src/Evil.rs", "src/evil.rs", "src/lib.rs"].map(String::from);
        let error = refuse_case_twins(&names).unwrap_err().to_string();
        assert!(
            error.contains("src/Evil.rs") && error.contains("src/evil.rs"),
            "{error}"
        );
    }
}
