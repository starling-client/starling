//! Generates the native mod API.

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let root = std::path::Path::new(&dir).join("../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    sunrise_mod_api_gen::generate(&root, &out_dir, sunrise_mod_api_gen::Target::Client)
        .expect("generating native mod API");
    // Set only where crates/sunrise-parity-fixtures compiles these sources as its program.
    println!("cargo::rustc-check-cfg=cfg(sunrise_parity_fixtures)");
}
