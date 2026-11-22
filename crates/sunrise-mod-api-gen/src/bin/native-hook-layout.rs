//! Catalog-owned layout and generation snapshots for `tools/split_native_hooks.py`.
fn main() -> Result<(), String> {
    use std::io::Read;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if let Some(out) = std::env::args().nth(1) {
        let out = std::path::Path::new(&out);
        std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
        sunrise_mod_api_gen::generate_api_parts(&root, out)?;
        sunrise_mod_api_gen::generate(&root, out, sunrise_mod_api_gen::Target::Client)?;
    } else {
        let mut source = String::new();
        std::io::stdin()
            .read_to_string(&mut source)
            .map_err(|e| e.to_string())?;
        println!(
            "{}",
            sunrise_mod_api_gen::native_hook_layout(&root, &source)?
        );
    }
    Ok(())
}
