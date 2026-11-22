<<<<<<< sync-sunrise-1
//! Installed native code loading. Starling runs no WebAssembly mods (2026-11-20, as Sunrise): a
//! package with wasm code is left out, and the Mods page says why (`code_runtime`).
use mod_api::__private::game::Fields;
/// The packages left out because their code is WebAssembly, by id (`load_code`).
static WASM_REFUSED: std::sync::OnceLock<std::collections::BTreeSet<String>> =
    std::sync::OnceLock::new();
=======
//! Installed native code loading.
use mod_api::__private::game::Fields;
use std::path::Path;
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

>>>>>>> sunrise/main
/// Load installed native code, register its dependency-ordered entries,
/// and start the engine. Keep the validated startup lease alive for the whole run.
pub fn main_with(
    mods: Vec<mod_api::ModEntry>,
    built: &'static [mod_api::__private::native::BuiltMod],
    startup: sunrise_client::entry::Startup,
) {
    // The title screen's Mods button: every game that loads mods has it, before the game seals
    // the title screen's buttons.
    if let Err(error) = sunrise_hooks::title::register(&crate::mods_button::MODS) {
        log::warn!("Sunrise: the title screen's Mods button did not register: {error}");
    }
    // Its label travels with it: the crate that supplies the row supplies its text.
    if let Err(error) = sunrise_hooks::lang::register(crate::mods_button::LANG) {
        log::warn!("Sunrise: the title screen's Mods label did not register: {error}");
    }
    // Platform instrumentation must precede registration and the first mod guard.
    if let Err(error) = mod_api::__private::initialize() {
        eprintln!("Sunrise: refusing startup because mod instrumentation is unavailable: {error}");
        std::process::exit(1);
    }
    if let Err(error) =
        mod_api::__private::platform::install_platform(mod_api::__private::platform::Platform {
            install_packages: mod_api::__private::install_packages,
            install_registration_data: mod_api::__private::install_registration_data,
            install_parts: mod_api::__private::install_parts,
            profiler_is_enabled: mod_api::__private::profiler::is_enabled,
            profiler_start: mod_api::__private::profiler::start,
            profiler_stop: mod_api::__private::profiler::stop,
            profiler_summary: mod_api::__private::profiler::summary,
            profiler_firefox_profile: mod_api::__private::profiler::firefox_profile,
        })
    {
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
<<<<<<< sync-sunrise-1
        |native, launch| load_code(mods, slots, native, launch),
        code_runtime(),
=======
        |native, launch| {
            // Installed code dispatch follows the resolved package order.
            let code_manifests: Vec<_> = native
                .packages()
                .iter()
                .map(|p| &p.manifest)
                .filter(|m| mods.iter().any(|entry| entry.fields().id == m.id))
                .collect();
            assert_eq!(
                code_manifests.len(),
                mods.len(),
                "loaded code has checked manifests"
            );
            let side = launch.installation_side();
            let code_ids: std::collections::HashSet<_> =
                mods.iter().map(|entry| entry.fields().id).collect();
            let settings = mod_list::ModList::load(&launch.context.game_dir);
            let content: Vec<_> = sunrise_mod_package::selection::content_packages(
                &launch.context.game_dir,
                &settings.enabled,
                &launch.context.dev_mods,
            )
            .collect();
            let manifests: Vec<_> = code_manifests
                .iter()
                .copied()
                .chain(content.iter().map(|source| &source.package.manifest))
                .collect();
            let order = resolve_startup(&manifests, side, &launch.context.game_dir).unwrap_or_else(
                |error| {
                    eprintln!("Sunrise: mod load refused: {error}");
                    sunrise_client::logging::flush();
                    std::process::exit(1);
                },
            );
            for warning in &order.warnings {
                log::warn!("Sunrise: {warning}");
            }
            // What every level.dat this game saves records (`fml.LoadingModList`).
            sunrise_client::save::level_mods::set_loaded(Some(
                order
                    .order
                    .iter()
                    .filter(|&&i| code_ids.contains(manifests[i].id.as_str()))
                    .map(|&i| sunrise_client::save::level_mods::ModRecord {
                        id: manifests[i].id.clone(),
                        version: manifests[i].version.to_string(),
                    })
                    .collect(),
            ));
            let mut entries: std::collections::BTreeMap<_, _> = mods
                .into_iter()
                .map(|entry| (entry.fields().id, entry))
                .collect();
            let mods: Vec<_> = order
                .order
                .into_iter()
                .filter(|&i| code_ids.contains(manifests[i].id.as_str()))
                .map(|i| entries.remove(manifests[i].id.as_str()).unwrap())
                .collect();
            mod_api::__private::install_dependencies(order.effective);
            mod_api::__private::install(mods.into());
        },
>>>>>>> sunrise/main
        startup,
    );
}

<<<<<<< sync-sunrise-1
/// The installed code in one dependency order, installed for dispatch: `mods` (the native entries
/// built in, or the native libraries Starling's Android start opened, android_lib.rs) and
/// `slots`, their typed listeners. What `entry::start` loads, from `main_with` and from that start.
pub fn load_code(
    mods: Vec<mod_api::ModEntry>,
    slots: &[crate::SlotListener],
    native: &sunrise_mod_package::runtime::Runtime,
    launch: &sunrise_mod_package::selection::LaunchArguments,
) -> (Vec<&'static str>, ()) {
    // One installed dispatch order; ties retain bundle order.
    let code_manifests: Vec<_> = native
        .packages()
        .iter()
        .map(|p| &p.manifest)
        .filter(|m| mods.iter().any(|entry| entry.fields().id == m.id))
        .collect();
    assert_eq!(
        code_manifests.len(),
        mods.len(),
        "loaded code has checked manifests"
    );
    let side = launch.installation_side();
    let declared: Vec<_> = code_manifests
        .iter()
        .map(|m| sunrise_mod_package::dependencies::from_manifest(&m.dependencies, side))
        .collect();
    let inputs: Vec<_> = code_manifests
        .iter()
        .zip(&declared)
        .map(|(m, dependencies)| sunrise_mod_package::dependencies::Mod {
            id: &m.id,
            version: &m.version,
            has_api: true,
            code: true,
            dependencies,
        })
        .collect();
    let code_ids: std::collections::HashSet<_> =
        mods.iter().map(|entry| entry.fields().id).collect();
    let settings = sunrise_mod_package::mod_list::ModList::load(&launch.context.game_dir);
    let content: Vec<_> = sunrise_mod_package::selection::content_packages(
        &launch.context.game_dir,
        &settings.enabled,
        &launch.context.dev_mods,
    )
    .collect();
    // A package whose code is WebAssembly loads neither its code nor its content: its content
    // leans on that code (its recipe types, its block entities).
    let wasm: std::collections::BTreeSet<String> = content
        .iter()
        .map(|source| &source.package.manifest)
        .filter(|manifest| manifest.wasm.is_some() && manifest.native.is_none())
        .map(|manifest| manifest.id.clone())
        .collect();
    for id in &wasm {
        log::warn!("Sunrise: {id} is a WebAssembly mod, which Starling no longer runs; left out");
    }
    let _ = WASM_REFUSED.set(wasm);
    let manifests: Vec<_> = code_manifests
        .iter()
        .copied()
        .chain(
            content
                .iter()
                .filter(|source| {
                    let manifest = &source.package.manifest;
                    let code = manifest.native.is_some() || manifest.wasm.is_some();
                    (!code || !code_ids.contains(manifest.id.as_str()))
                        && !wasm_refused(&manifest.id)
                })
                .map(|source| &source.package.manifest),
        )
        .collect();
    let order = sunrise_mod_package::dependencies::Overrides::read(
        &launch
            .context
            .game_dir
            .join(sunrise_mod_package::mod_list::DIR),
    )
    .and_then(|overrides| {
        let resolution =
            sunrise_mod_package::dependencies::resolve_manifests(&manifests, side, &overrides)
                .map_err(|error| error.to_string())?;
        sunrise_mod_package::dependencies::resolve_links(&inputs)
            .map_err(|error| format!("code link requirements refused: {error}"))?;
        Ok(resolution)
    })
    .unwrap_or_else(|error| {
        eprintln!("Sunrise: mod load refused: {error}");
        sunrise_client::logging::flush();
        std::process::exit(1);
    });
    for warning in &order.warnings {
        log::warn!("Sunrise: {warning}");
    }
    // What every level.dat this game saves records (`fml.LoadingModList`).
    sunrise_client::save::level_mods::set_loaded(Some(
        order
            .order
            .iter()
            .filter(|&&i| code_ids.contains(manifests[i].id.as_str()))
            .map(|&i| sunrise_client::save::level_mods::ModRecord {
                id: manifests[i].id.clone(),
                version: manifests[i].version.clone(),
            })
            .collect(),
    ));
    let mut entries: std::collections::BTreeMap<_, _> = mods
        .into_iter()
        .map(|entry| (entry.fields().id, entry))
        .collect();
    let mods: Vec<_> = order
        .order
        .into_iter()
        .filter(|&i| code_ids.contains(manifests[i].id.as_str()))
        .map(|i| entries.remove(manifests[i].id.as_str()).unwrap())
        .collect();
    let ids: Vec<_> = mods.iter().map(|entry| entry.fields().id).collect();
    mod_api::__private::install(mods.into());
    for &id in &ids {
        for slot in slots.iter().filter(|slot| slot.id() == id) {
            if let Err(error) = mod_points::register(slot) {
                log::error!("Sunrise: {id}: {error}");
                sunrise_client::logging::flush();
                std::process::exit(1);
            }
        }
    }
    (ids, ())
}

/// The code runtime's services for `entry::start`: no WebAssembly runtime to start, and the Mods
/// page's reason for a package `load_code` left out for its wasm code.
pub fn code_runtime() -> sunrise_client::entry::CodeRuntime {
    sunrise_client::entry::CodeRuntime {
        initialize: |_| {},
        refusal: |id| {
            wasm_refused(id).then(|| {
                sunrise_client::text::Text::translate_with_fallback(
                    "starling.mods.wasm_removed",
                    "WebAssembly mods no longer run in Starling: this one needs a native or content-only version.",
                    &[],
                )
                .plain()
            })
        },
        startup_refused: wasm_refused,
    }
}

fn wasm_refused(id: &str) -> bool {
    WASM_REFUSED.get().is_some_and(|ids| ids.contains(id))
}
=======
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
>>>>>>> sunrise/main
