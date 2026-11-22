//! Source-only helpers shared by the generator and the ignored-test inventory.
#![forbid(unsafe_code)]

use std::{
    fs,
    path::{Path, PathBuf},
};

pub mod exports;

/// The hand-written sources, in deterministic order; `fire.rs` includes generated dispatch.
pub fn native_hook_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = root.join("crates/sunrise-client/src/game/native_hooks");
    let mut paths = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|entry| entry.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().is_some_and(|name| name != "fire.rs")
    });
    paths.sort();
    Ok(paths)
}

/// The `*.rs` file stems of `directory` but `mod`, sorted, each checked to be a Rust identifier.
pub fn module_files(directory: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            if name != "mod" {
                syn::parse_str::<syn::Ident>(&name).map_err(|_| {
                    format!("{}: the file name is not a Rust identifier", path.display())
                })?;
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}
