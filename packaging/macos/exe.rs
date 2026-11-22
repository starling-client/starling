//! macOS Mach-O resources shared by the `sunrise-game` and `sunrise-official` build scripts.
//! Embeds `packaging/macos/game-info.plist` into `__TEXT,__info_plist` so standalone macOS
//! executables carry `CFBundleIdentifier`, `CFBundleName`, and `NSMicrophoneUsageDescription`
//! required by TCC before opening microphone capture.

/// Embed the executable's Info.plist section on macOS; do nothing for other targets.
pub fn resources(bin_name: &str) {
    let manifest_dir = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let plist = manifest_dir
        .join("../..")
        .join("packaging/macos/game-info.plist");
    let target_os =
        std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo must provide the build target OS");
    println!("cargo:rerun-if-changed={}", plist.display());
    if target_os == "macos" {
        let plist = plist.canonicalize().unwrap_or_else(|error| {
            panic!("resolving macOS Info.plist {}: {error}", plist.display())
        });
        println!("cargo:rustc-link-arg-bin={bin_name}=-Xlinker");
        println!("cargo:rustc-link-arg-bin={bin_name}=-sectcreate");
        println!("cargo:rustc-link-arg-bin={bin_name}=-Xlinker");
        println!("cargo:rustc-link-arg-bin={bin_name}=__TEXT");
        println!("cargo:rustc-link-arg-bin={bin_name}=-Xlinker");
        println!("cargo:rustc-link-arg-bin={bin_name}=__info_plist");
        println!("cargo:rustc-link-arg-bin={bin_name}=-Xlinker");
        println!("cargo:rustc-link-arg-bin={bin_name}={}", plist.display());
    }
}
