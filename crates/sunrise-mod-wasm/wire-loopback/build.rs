//! The test target's generated inputs: the host boundary (as sunrise-mod-wasm's build.rs writes it), one module per
//! file of tests/wire_loopback/, and the guest's one Hooks object assembled from those files' `impl <Feature>Hooks`
//! blocks. A package of its own, so that editing a test reruns only this script: sunrise-mod-wasm's rerun would
//! rebuild its library and every crate above it.
#[path = "../../../packaging/executable_link.rs"]
mod executable_link;

fn main() {
    println!("cargo:rerun-if-changed=../../../packaging/executable_link.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    for arg in executable_link::arguments(&os, &env) {
        println!("cargo:rustc-link-arg-tests={arg}");
    }
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../../..");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=../../sunrise-guest-stack/src/guest_stack_constants.rs");
    println!("cargo:rerun-if-changed=../../sunrise-mod-api/src");
    println!("cargo:rerun-if-changed=../../sunrise-mod-vocab/src");
    println!("cargo:rerun-if-changed=../../sunrise-mod-api-gen/src");
    let code = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || sunrise_mod_api_gen::generate_wire_host(&root))
        .expect("starting host generator")
        .join()
        .expect("host generator panicked")
        .expect("generating host boundary");
    sunrise_mod_api_gen::write_if_changed(out.join("wire_host_generated.rs"), code)
        .expect("writing host boundary");
    println!("cargo:rerun-if-changed=tests/wire_loopback");
    let modules = sunrise_game_discovery::folder_modules(&manifest.join("tests/wire_loopback"))
        .unwrap_or_else(|error| panic!("discovering wire_loopback test modules: {error}"));
    sunrise_mod_api_gen::write_if_changed(out.join("wire_loopback_modules.rs"), modules)
        .expect("writing wire_loopback test modules");
    let hooks = sunrise_game_discovery::folder_hooks(&manifest.join("tests/wire_loopback"))
        .unwrap_or_else(|error| panic!("assembling the wire_loopback guest hooks: {error}"));
    sunrise_mod_api_gen::write_if_changed(out.join("wire_loopback_hooks.rs"), hooks)
        .expect("writing wire_loopback guest hooks");
}
