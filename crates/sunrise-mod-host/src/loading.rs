//! Installed native code loading.
use mod_api::__private::game::Fields;
use std::path::Path;
#[path = "loading/starling.rs"]
mod starling;
#[allow(unused_imports)]
pub use self::starling::*;
use sunrise_mod_package::{
    Manifest,
    dependencies::{Overrides, Resolution, resolve_manifests},
    manifest::Side,
    mod_list,
};

fn resolve_startup(
    manifests: &[&Manifest],
    side: Side,
    game_dir: &Path,
) -> Result<Resolution, String> {
    // The native dynamic loader and compiled package inventory already enforce
    // native physical requirements. Only logical requirements reach this resolver.
    let overrides = Overrides::read(&game_dir.join(mod_list::DIR))?;
    resolve_manifests(manifests, side, &overrides).map_err(|error| error.to_string())
}

/// Load installed native code, register its dependency-ordered entries,
/// and start the engine. Keep the validated startup lease alive for the whole run.
pub fn main_with(
    mods: Vec<mod_api::ModEntry>,
    built: &'static [mod_api::__private::native::BuiltMod],
    startup: sunrise_client::entry::Startup,
) {
    if let Err(error) = install_platform() {
        eprintln!("Sunrise: {error}");
        std::process::exit(1);
    }
    let exe = std::env::current_exe();
    let native: Result<_, Box<dyn std::error::Error + Send + Sync>> = exe
        .as_ref()
        .map_err(|e| e.to_string().into())
        .and_then(|exe| {
            sunrise_mod_package::runtime::Runtime::load(exe, built).map_err(Into::into)
        });
    let native = match native {
        Ok(native) => native,
        Err(error) => {
            eprintln!("Sunrise: {error}");
            std::process::exit(1);
        }
    };
    mod_render::render::install();
    sunrise_client::entry::start(
        native,
        |native, launch| load_code(mods, native, launch),
        startup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::__private::native::BuiltMod;
    use std::{fs, path::PathBuf};
    use sunrise_mod_package::{dependencies::OVERRIDES, runtime::Runtime};

    #[test]
    fn native_startup_requires_manifest_only_data_unless_dropped() {
        const BUILT: &[BuiltMod] = &[
            BuiltMod {
                id: "addon",
                version: "1.0.0",
                side: Side::Both,
            },
            BuiltMod {
                id: "data",
                version: "1.0.0",
                side: Side::Both,
            },
        ];
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let root = Scratch(std::env::temp_dir().join(format!(
            "sunrise-native-startup-deps-{}",
            std::process::id()
        )));
        let package = root.0.join(mod_list::NATIVE_DIR).join("addon");
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(
            package.join("Cargo.toml"),
            "[package]\nname='addon'\nversion='1.0.0'\n",
        )
        .unwrap();
        fs::write(package.join("src/lib.rs"), "").unwrap();
        fs::write(package.join("sunrise-mod.toml"), format!(
            "id='addon'\nname='Addon'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[dependencies]\ndata='^1'\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
            "a".repeat(40), "b".repeat(40),
        )).unwrap();
        let executable = root.0.join("sunrise-client");
        let native = Runtime::load(&executable, &BUILT[..1]).unwrap();
        let manifests = [&native.packages()[0].manifest];
        assert_eq!(
            resolve_startup(&manifests, Side::Client, &root.0).unwrap_err(),
            "addon requires missing mod data"
        );

        let mods_dir = root.0.join(mod_list::DIR);
        fs::create_dir(&mods_dir).unwrap();
        fs::write(mods_dir.join(OVERRIDES), "[addon]\ndata='drop'\n").unwrap();
        let resolved = resolve_startup(&manifests, Side::Client, &root.0).unwrap();
        assert_eq!(resolved.order, [0]);
        assert!(!resolved.effective.permits("addon", "data"));

        // If the executable actually includes the dependency's code, its package
        // is required before logical policy can be applied, even with the drop.
        let error = Runtime::load(&executable, BUILT).unwrap_err();
        assert_eq!(error.refused.len(), 1);
        assert!(error.to_string().contains("data 1.0.0:"), "{error}");
    }
}
