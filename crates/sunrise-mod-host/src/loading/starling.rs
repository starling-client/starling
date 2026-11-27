//! Starling's part of loading.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// What a game that loads mods installs before the engine starts: the title screen's Mods button,
/// the mods' instrumentation and the engine platform socket (packages, registration data, parts,
/// profiler). [`main_with`] does it for the desktop; Starling's Android start calls it itself
/// (android_lib.rs), since it starts the engine without `main_with`, which loads native libraries
/// next to an executable the phone does not have.
pub fn install_platform() -> Result<(), String> {
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
    mod_api::__private::initialize().map_err(|error| {
        format!("refusing startup because mod instrumentation is unavailable: {error}")
    })?;
    mod_api::__private::platform::install_platform(mod_api::__private::platform::Platform {
        install_packages: mod_api::__private::install_packages,
        install_registration_data: mod_api::__private::install_registration_data,
        install_parts: mod_api::__private::install_parts,
        with_registration_open: mod_api::__private::with_registration_open,
        profiler_is_enabled: mod_api::__private::profiler::is_enabled,
        profiler_start: mod_api::__private::profiler::start,
        profiler_stop: mod_api::__private::profiler::stop,
        profiler_summary: mod_api::__private::profiler::summary,
        profiler_firefox_profile: mod_api::__private::profiler::firefox_profile,
    })
    .map_err(|error| error.to_string())
}

/// The installed code in the resolved package order, installed for dispatch: `mods` are the native entries
/// built in, or those of the native libraries Starling's Android start opened (android_lib.rs). What
/// `entry::start` loads, from [`main_with`] and from that start.
pub fn load_code(
    mods: Vec<mod_api::ModEntry>,
    native: &sunrise_mod_package::runtime::Runtime,
    launch: &sunrise_mod_package::selection::LaunchArguments,
) {
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
    let order =
        resolve_startup(&manifests, side, &launch.context.game_dir).unwrap_or_else(|error| {
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
        .map(|i| {
            let entry = entries.remove(manifests[i].id.as_str()).unwrap();
            mod_api::__private::with_mod_side(entry, manifests[i].side)
        })
        .collect();
    mod_api::__private::install_dependencies(order.effective);
    mod_api::__private::install(mods.into());
}
