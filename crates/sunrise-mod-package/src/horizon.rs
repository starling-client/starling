//! Starling: Horizon's packages on this game. Horizon's registry lists `.hmod` files whose
//! manifest is `horizon-mod.toml` and whose native code names Horizon's crates; Sunrise, which
//! this game is built on, reads `.smod` files with `sunrise-mod.toml` and has the same crates under
//! its own names. A Horizon package becomes a Sunrise one here, once, as it is installed: the
//! manifest takes its new name and its `[native]` pin its new key, and the Rust and Cargo files
//! name the crates as this game has them. Assets and data are copied as they are.
use std::io::{Read, Write};
use std::path::Path;

/// Horizon's manifest file.
pub const MANIFEST: &str = "horizon-mod.toml";

/// In an adopted package: the SHA-256 of the file Horizon's registry lists, which Starling's mod
/// store names its library of the mod by (the adopted file's own bytes are this phone's alone).
pub const ORIGIN: &str = "starling-origin.sha256";

/// The SHA-256 of the registry's file the package at `path` was adopted from, if it was.
pub fn origin(path: &Path) -> Option<String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let mut text = String::new();
    zip.by_name(ORIGIN).ok()?.read_to_string(&mut text).ok()?;
    let text = text.trim();
    (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())).then(|| text.to_owned())
}

/// The crates a mod may name, by what follows `horizon_` or `horizon-` in their names. Anything
/// else that says "horizon" (a mod's own id, a namespace, a comment) stays as written.
/// `part` is no crate: `#[horizon_part]`, the attribute of a part.
const CRATES: [&str; 15] = [
    "mod", "client", "core", "world", "items", "protocol", "assets", "hooks", "platform", "shader",
    "render", "threads", "look", "optifine", "part",
];

/// Whether the file at `path` is a Horizon package: a ZIP with Horizon's manifest and not
/// Sunrise's.
pub fn holds(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(mut zip) = zip::ZipArchive::new(file) else {
        return false;
    };
    zip.by_name(MANIFEST).is_ok() && zip.by_name(crate::manifest::MANIFEST).is_err()
}

/// The manifest as Sunrise reads it: the `[native]` table's `horizon` pin is `sunrise`.
fn manifest(text: &str) -> String {
    let mut native = false;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            native = trimmed.starts_with("[native]");
        }
        let pin = native
            && trimmed
                .strip_prefix("horizon")
                .is_some_and(|rest| rest.trim_start().starts_with('='));
        if pin {
            let indent = &line[..line.len() - trimmed.len()];
            out.push_str(indent);
            out.push_str("sunrise");
            out.push_str(&trimmed["horizon".len()..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

/// A Rust or Cargo file naming the crates as this game has them.
fn source(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("horizon") {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        let after = &from["horizon".len()..];
        // `.horizon/` (the SDK's folder beside the mod), or a crate's name.
        let folder = out.ends_with('.') && after.starts_with('/');
        let named = after
            .strip_prefix(['_', '-'])
            .is_some_and(|stem| CRATES.iter().any(|c| stem.starts_with(c)));
        // Not the middle of a longer word (`myhorizon_mod`).
        let starts = !out
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        out.push_str(if folder || (named && starts) {
            "sunrise"
        } else {
            "horizon"
        });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// A file's path as this game reads it. Horizon keeps a mod's declarations in
/// `data/<namespace>/horizon/` (blocks, items, menus...) and its engine's own data in the
/// `horizon` namespace (`data/horizon/tags/...`); Sunrise reads `data/<namespace>/sunrise/` and
/// `sunrise`. Without this every block and item of a Horizon mod was left out.
fn path_in_package(name: &str) -> String {
    let mut parts: Vec<&str> = name.splitn(4, '/').collect();
    if matches!(parts.first(), Some(&"data" | &"assets")) {
        if parts.get(1) == Some(&"horizon") {
            parts[1] = "sunrise";
        }
        if parts[0] == "data" && parts.len() == 4 && parts[2] == "horizon" {
            parts[2] = "sunrise";
        }
    }
    parts.join("/")
}

/// Whether `name` is a data or asset file that names ids: the engine's (`"horizon:pick"`,
/// `"#horizon:..."`) are `sunrise:` here.
fn names_ids(name: &str) -> bool {
    (name.starts_with("data/") || name.starts_with("assets/"))
        && (name.ends_with(".json") || name.ends_with(".mcmeta"))
}

/// A data or asset file naming the engine's ids by this game's namespace.
fn ids(text: &str) -> String {
    text.replace("\"horizon:", "\"sunrise:")
        .replace("\"#horizon:", "\"#sunrise:")
}

/// Whether `name` is one of the package's text files that name crates.
fn names_crates(name: &str) -> bool {
    name == "Cargo.toml" || (name.starts_with("src/") && name.ends_with(".rs"))
}

/// The Horizon package at `path` rewritten in place as a Sunrise package. Nothing changes when
/// it fails.
pub fn adopt(path: &Path) -> Result<(), String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::open(path).map_err(|e| at(&e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| at(&e))?;
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| at(&e))?;
        let name = path_in_package(&entry.name().map_err(|e| at(&e))?);
        if entry.is_dir() {
            out.add_directory(name.as_str(), options)
                .map_err(|e| at(&e))?;
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| at(&e))?;
        let (name, bytes) = if name == MANIFEST {
            let text = String::from_utf8(bytes).map_err(|e| at(&e))?;
            (
                crate::manifest::MANIFEST.to_owned(),
                manifest(&text).into_bytes(),
            )
        } else if names_crates(&name) {
            match String::from_utf8(bytes) {
                Ok(text) => (name, source(&text).into_bytes()),
                Err(e) => (name, e.into_bytes()),
            }
        } else if names_ids(&name) {
            match String::from_utf8(bytes) {
                Ok(text) => (name, ids(&text).into_bytes()),
                Err(e) => (name, e.into_bytes()),
            }
        } else {
            (name, bytes)
        };
        out.start_file(name.as_str(), options).map_err(|e| at(&e))?;
        out.write_all(&bytes).map_err(|e| at(&e))?;
    }
    use sha2::Digest;
    let listed = std::fs::read(path).map_err(|e| at(&e))?;
    let digest: String = sha2::Sha256::digest(&listed)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    out.start_file(ORIGIN, options).map_err(|e| at(&e))?;
    out.write_all(digest.as_bytes()).map_err(|e| at(&e))?;
    let bytes = out.finish().map_err(|e| at(&e))?.into_inner();
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("mod");
    let temporary = path.with_file_name(format!("{file}.adopting"));
    std::fs::write(&temporary, bytes).map_err(|e| at(&e))?;
    std::fs::rename(&temporary, path).map_err(|e| at(&e))
}

#[cfg(test)]
mod tests {
    use super::{ids, manifest, path_in_package, source};

    #[test]
    fn horizons_data_folders_and_ids_take_sunrises_names() {
        assert_eq!(
            path_in_package("data/evilcraft/horizon/menu/a.json"),
            "data/evilcraft/sunrise/menu/a.json"
        );
        assert_eq!(
            path_in_package("data/horizon/tags/block/class/crop.json"),
            "data/sunrise/tags/block/class/crop.json"
        );
        assert_eq!(
            path_in_package("assets/horizon/lang/en_us.json"),
            "assets/sunrise/lang/en_us.json"
        );
        // A mod's own folder named horizon elsewhere, and source, stay.
        assert_eq!(
            path_in_package("assets/evilcraft/horizon/x.png"),
            "assets/evilcraft/horizon/x.png"
        );
        assert_eq!(path_in_package("src/horizon.rs"), "src/horizon.rs");
        assert_eq!(
            ids(
                r##"{"processor_type": "horizon:pick", "tag": "#horizon:x", "id": "myhorizon:y"}"##
            ),
            r##"{"processor_type": "sunrise:pick", "tag": "#sunrise:x", "id": "myhorizon:y"}"##
        );
    }

    #[test]
    fn the_native_pin_takes_sunrises_key() {
        let text =
            "id = \"x\"\nhorizon = \"kept\"\n\n[native]\ngame = \"26.3\"\nhorizon = \"abc\"\n";
        assert_eq!(
            manifest(text),
            "id = \"x\"\nhorizon = \"kept\"\n\n[native]\ngame = \"26.3\"\nsunrise = \"abc\"\n"
        );
    }

    #[test]
    fn only_crates_and_the_sdk_folder_are_renamed() {
        assert_eq!(
            source("use horizon_mod_api::Host; // horizon_tools:ruby\n"),
            "use sunrise_mod_api::Host; // horizon_tools:ruby\n"
        );
        assert_eq!(
            source("horizon-mod-api = { path = \".horizon/sdk/api/crates/horizon-mod-api\" }"),
            "sunrise-mod-api = { path = \".sunrise/sdk/api/crates/sunrise-mod-api\" }"
        );
        assert_eq!(source("myhorizon_mod"), "myhorizon_mod");
    }
}
