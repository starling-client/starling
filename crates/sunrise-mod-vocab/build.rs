fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    sunrise_mod_api_gen::generate(&root, &out_dir, sunrise_mod_api_gen::Target::Vocab)
        .expect("generating the mod API");
}
