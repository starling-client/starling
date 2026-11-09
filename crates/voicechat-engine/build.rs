// Compiles the bundled libopus 1.5.2 (BSD-3-Clause, see data/opus/COPYING): the
// float build, generic C only, no deep-learning extensions.
fn main() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("data")
        .join("opus");
    let list = std::fs::read_to_string(root.join("sources.txt")).expect("data/opus/sources.txt");
    let mut build = cc::Build::new();
    for dir in ["include", "celt", "silk", "silk/float", "src", ""] {
        build.include(root.join(dir));
    }
    build
        .define("OPUS_BUILD", None)
        .define("VAR_ARRAYS", None)
        .define("HAVE_LRINT", "1")
        .define("HAVE_LRINTF", "1")
        .warnings(false)
        .opt_level(2);
    for file in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
        build.file(root.join(file));
    }
    build.compile("opus");
    println!("cargo:rerun-if-changed=data/opus");
    println!("cargo:rerun-if-changed=build.rs");
}
