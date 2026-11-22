//! Which files of a mod folder are its package's source: the one selection that packing
//! (`sunrise_mod_package::Package::directory_files`) and the uses scan ([`crate::mod_uses`]) share.
use std::path::Path;

/// Folders of a mod's project that are never part of its package: Git's, Cargo's build output
/// and the pinned SDK the mod tool fetches into `.sunrise/`.
pub const NOT_PACKAGED: [&str; 3] = [".git", "target", ".sunrise"];

/// A source file that belongs to the flat Cargo project, not its resource packs.
pub fn source_file(name: &str) -> bool {
    if name.split('/').any(|part| NOT_PACKAGED.contains(&part)) {
        return false;
    }
    name == "Cargo.toml"
        || name == "Cargo.lock"
        || name == "build.rs"
        || (name.starts_with("src/") && (name.ends_with(".rs") || name.ends_with(".toml")))
        || (!name.contains('/')
            && ["README", "LICENSE", "LICENCE", "COPYING", "NOTICE"]
                .iter()
                .any(|prefix| name.starts_with(prefix)))
}

/// The source files a package of the folder at `root` carries, `/`-separated and sorted: the
/// root files and the files under `src/` that [`source_file`] names, outside [`NOT_PACKAGED`]
/// folders. Nothing else is read: no folder at the top but `src/`, and no entry whose name a
/// package path cannot hold (not UTF-8, or with a `\` in it). The walk keeps its own stack, so
/// a deep tree needs no recursion.
pub fn source_files(root: &Path) -> std::io::Result<Vec<String>> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
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
                if (prefix.is_empty() && file_name != "src") || NOT_PACKAGED.contains(&file_name) {
                    continue;
                }
                pending.push((entry.path(), name));
            } else if source_file(&name) {
                found.push(name);
            }
        }
    }
    found.sort();
    Ok(found)
}
