//! `sunrise-materialize ROOT`: a disposable copy of the game's source made ready for horizon-mixins,
//! as Sunrise's launcher does before it reads a copy. The build script's generated host forwarders
//! (the game's `include!` of OUT_DIR) are written into the copy (`materialize_generated_host`), and
//! the hook catalog as the `native-hooks.json` horizon-mixins resolves a mod's named hooks
//! (`hook = "frame"`) through: Sunrise declares it in Rust now (sunrise-mod-vocab/src/hooks), and
//! its generator refuses that file, so a build must leave it out. Starling's native_build.sh runs
//! it. Never on a working tree.
#[cfg(not(test))]
use std::path::Path;

// The catalog's loader, read from its source because the library keeps it private. Left out of a
// test build: its own tests (`#[cfg(test)]`) name the library's items. rustfmt formats it where the
// library declares it.
#[cfg(not(test))]
#[allow(dead_code, unused)]
#[rustfmt::skip]
#[path = "../catalog.rs"]
mod catalog;

#[cfg(not(test))]
fn main() {
    let mut args = std::env::args_os().skip(1);
    let (Some(root), None) = (args.next(), args.next()) else {
        eprintln!("usage: sunrise-materialize ROOT");
        std::process::exit(2)
    };
    if let Err(error) = materialize(Path::new(&root)) {
        eprintln!("sunrise-materialize: {error}");
        std::process::exit(1)
    }
}

/// Sunrise's hooks carry no contract version; every bundle written for Starling names version 1,
/// the only one its catalogs ever had (packaging/android/native-hooks-0.8.json).
#[cfg(not(test))]
const HOOK_VERSION: u64 = 1;

#[cfg(not(test))]
fn materialize(root: &Path) -> Result<(), String> {
    let hooks: Vec<_> = catalog::Catalog::load(root)?
        .hooks
        .into_iter()
        .map(|hook| {
            serde_json::json!({ "name": hook.name, "version": HOOK_VERSION, "target": hook.target })
        })
        .collect();
    sunrise_mod_api_gen::materialize_generated_host(root)?;
    let catalog = serde_json::to_vec_pretty(&serde_json::json!({ "hooks": hooks }))
        .map_err(|e| e.to_string())?;
    std::fs::write(root.join("native-hooks.json"), catalog).map_err(|e| e.to_string())
}
