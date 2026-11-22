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

/// The crates a mod may name, by what follows `horizon_` or `horizon-` in their names. Anything
/// else that says "horizon" (a mod's own id, a namespace, a comment) stays as written.
const CRATES: [&str; 14] = [
    "mod", "client", "core", "world", "items", "protocol", "assets", "hooks", "platform", "shader",
    "render", "threads", "look", "optifine",
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
        let name = entry.name().map_err(|e| at(&e))?.into_owned();
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
        } else {
            (name, bytes)
        };
        out.start_file(name.as_str(), options).map_err(|e| at(&e))?;
        out.write_all(&bytes).map_err(|e| at(&e))?;
    }
    let bytes = out.finish().map_err(|e| at(&e))?.into_inner();
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("mod");
    let temporary = path.with_file_name(format!("{file}.adopting"));
    std::fs::write(&temporary, bytes).map_err(|e| at(&e))?;
    std::fs::rename(&temporary, path).map_err(|e| at(&e))
}

#[cfg(test)]
mod tests {
    use super::{manifest, source};

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
