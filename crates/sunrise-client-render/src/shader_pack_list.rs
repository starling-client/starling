//! The shader packs in the `shaderpacks` folder beside `options.txt`, as Options > Sunrise
//! Settings > Shaders lists them: those in the `shaders/` format. Sunrise's own WGSL packs
//! (a folder with `sunrise-shaders.toml`, or a `.sshader`) are no longer run: they are left out
//! of the list with a note (`UNSUPPORTED_NOTE`).

use std::path::Path;

pub const DIR: &str = "shaderpacks";

/// The translation key of the note for a pack in the removed WGSL format; `%s` is its name.
pub const UNSUPPORTED_NOTE: &str = "sunrise.shaders.unsupported";

/// The manifest of the removed WGSL format's pack folder.
const WGSL_MANIFEST: &str = "sunrise-shaders.toml";
/// The extension of the removed WGSL format's package.
const WGSL_EXTENSION: &str = "sshader";

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The folder or file name in `shaderpacks`: a folder with a `shaders` folder, or a `.zip`.
    pub file: String,
}

/// The packs in `dir`, by name. A missing folder has none. Each pack in the removed WGSL
/// format gets its note in the log and is left out.
pub fn scan(dir: &Path) -> Vec<Entry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    let mut packs = Vec::new();
    for path in &paths {
        let Some(file) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if file.starts_with('.') {
            continue;
        }
        if is_wgsl_pack(path) {
            log::warn!(
                "shader pack {file}: {}",
                crate::lang::tr(UNSUPPORTED_NOTE, &[&file])
            );
            continue;
        }
        let zip = path.is_file()
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        if zip || path.join("shaders").is_dir() {
            packs.push(Entry {
                file: file.to_owned(),
            });
        }
    }
    packs.sort_by_key(|e| e.file.to_lowercase());
    packs
}

/// Whether the pick `picked` saved in the options names a pack in the removed WGSL format:
/// its old `sunrise:<id>` identity, a `.sshader` name (installed or not), or a pack folder.
/// Such a pick falls back to no pack, with the note.
pub fn unsupported(dir: &Path, picked: &str) -> bool {
    !picked.is_empty()
        && (picked.starts_with("sunrise:")
            || picked.ends_with(&format!(".{WGSL_EXTENSION}"))
            || is_wgsl_pack(&dir.join(picked)))
}

/// A folder holding `sunrise-shaders.toml`, or a `.sshader` file.
fn is_wgsl_pack(path: &Path) -> bool {
    path.join(WGSL_MANIFEST).is_file()
        || (path.is_file() && path.extension().is_some_and(|e| e == WGSL_EXTENSION))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sunrise-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_the_shaders_packs_by_name_and_skips_the_rest() {
        let dir = scratch("shader-list");
        std::fs::create_dir(dir.join("warm")).unwrap();
        std::fs::write(dir.join("warm").join(WGSL_MANIFEST), "schema = 1\n").unwrap();
        std::fs::write(dir.join("glow-1.0.0.sshader"), b"").unwrap();
        std::fs::create_dir_all(dir.join("Classic Folder/shaders")).unwrap();
        std::fs::write(dir.join("archive.zip"), b"").unwrap();
        std::fs::write(dir.join("notes.txt"), b"").unwrap();
        std::fs::create_dir(dir.join("empty")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden/shaders")).unwrap();
        let listed: Vec<String> = scan(&dir).into_iter().map(|e| e.file).collect();
        assert_eq!(listed, ["archive.zip", "Classic Folder"]);
        assert!(scan(&dir.join("missing")).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A saved pick of a removed-format pack, however it was saved, falls back; one of a
    /// `shaders/` pack, or none, stays.
    #[test]
    fn a_saved_pick_of_a_wgsl_pack_is_unsupported() {
        let dir = scratch("shader-unsupported");
        std::fs::create_dir(dir.join("warm")).unwrap();
        std::fs::write(dir.join("warm").join(WGSL_MANIFEST), "schema = 1\n").unwrap();
        std::fs::write(dir.join("glow-1.0.0.sshader"), b"").unwrap();
        std::fs::create_dir_all(dir.join("Classic/shaders")).unwrap();
        std::fs::create_dir_all(dir.join("warm-classic/shaders")).unwrap();
        // The identity of the old settings, a package name, an installed package, a folder.
        for picked in [
            "sunrise:warm",
            "gone-2.sshader",
            "glow-1.0.0.sshader",
            "warm",
        ] {
            assert!(unsupported(&dir, picked), "{picked}");
        }
        for picked in ["", "Classic", "warm-classic", "missing.zip"] {
            assert!(!unsupported(&dir, picked), "{picked}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
