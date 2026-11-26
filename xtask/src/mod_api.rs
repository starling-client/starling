//! Fill new native hooks' placeholder bodies in the hand-written source.
use std::path::{Path, PathBuf};

fn checkout() -> Result<PathBuf, String> {
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    root.ancestors()
        .find(|path| path.join("crates/sunrise-mod-vocab/src/hooks").is_dir())
        .map(Path::to_path_buf)
        .ok_or("run xtask from within the Sunrise checkout".into())
}

pub fn run() -> Result<(), String> {
    let root = checkout()?;
    sunrise_mod_api_gen::fill_hooks(&root)
}

/// Prints the hook-notes chapter for the book preprocessor, without writing a source file.
pub fn notes() -> Result<(), String> {
    sunrise_mod_api_gen::generated_on_stdout();
    print!(
        "{}",
        sunrise_mod_api_gen::generate_hook_notes(&checkout()?)?
    );
    Ok(())
}

/// Prints the generated API reference chapter for the book preprocessor, without writing a source
/// file.
pub fn reference() -> Result<(), String> {
    sunrise_mod_api_gen::generated_on_stdout();
    print!(
        "{}",
        sunrise_mod_api_gen::generate_api_reference(&checkout()?)?
    );
    Ok(())
}

/// Prints the NeoForge counterparts chapter for the book preprocessor, without writing a source
/// file.
pub fn counterparts() -> Result<(), String> {
    sunrise_mod_api_gen::generated_on_stdout();
    print!(
        "{}",
        sunrise_mod_api_gen::generate_counterparts(&checkout()?)?
    );
    Ok(())
}
