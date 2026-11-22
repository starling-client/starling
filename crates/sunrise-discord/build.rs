use std::fmt::Write as _;
use std::{env, fs, path::PathBuf};

fn main() {
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets/lang");
    println!("cargo:rerun-if-changed={}", source.display());
    let mut paths: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort_unstable();
    let mut list = String::from("&[\n");
    for path in paths {
        let locale = path.file_stem().unwrap().to_str().unwrap();
        writeln!(list, "    ({locale:?}, include_str!({:?})),", path).unwrap();
    }
    list.push_str("]\n");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(out.join("locales.rs"), list).unwrap();
}
