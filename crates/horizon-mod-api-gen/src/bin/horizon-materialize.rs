//! `horizon-materialize ROOT`: a disposable copy of the game's source made ready for horizon-mixins,
//! as Horizon's launcher does before it reads a copy (launcher/src/native_mods.rs). Its generated
//! module lists (the `include!`s of the build script's output) are written out as plain
//! `#[path] mod` lines (`materialize_game_modules`), and the hook catalog as the
//! `native-hooks.json` horizon-mixins resolves a mod's named hooks (`hook = "frame"`) through:
//! Horizon declares it in Rust now (horizon-mod-vocab/src/hooks), and its generator refuses that
//! file, so a build must leave it out. Starling's native_build.sh runs it. Never on a working tree.
use std::path::Path;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let (Some(root), None) = (args.next(), args.next()) else {
        eprintln!("usage: horizon-materialize ROOT");
        std::process::exit(2)
    };
    if let Err(error) = materialize(Path::new(&root)) {
        eprintln!("horizon-materialize: {error}");
        std::process::exit(1)
    }
}

fn materialize(root: &Path) -> Result<(), String> {
    let hooks: Vec<_> = horizon_mod_api_gen::hook_points(root)?
        .into_iter()
        .map(|hook| {
            serde_json::json!({ "name": hook.name, "version": hook.version, "target": hook.target })
        })
        .collect();
    horizon_mod_api_gen::materialize_game_modules(root)?;
    let catalog = serde_json::to_vec_pretty(&serde_json::json!({ "hooks": hooks }))
        .map_err(|e| e.to_string())?;
    std::fs::write(root.join("native-hooks.json"), catalog).map_err(|e| e.to_string())
}
