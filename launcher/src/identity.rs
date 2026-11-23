//! Repository-selected identity. An upstream build retains its original namespaces.
macro_rules! identity {
    ($($name:ident: $upstream:literal => $fork:literal),* $(,)?) => {$ (
        pub const $name: &str = if cfg!(sunrise_fork_launcher) { $fork } else { $upstream };
    )*};
}
identity! {
    VERSION_PREFIX: "sunrise-launcher " => "sunrise-fork-launcher ",
    DESKTOP_ENTRY: "sunrise-launcher.desktop" => "io.github.arismoko.sunrisefork.launcher.desktop",
    WINDOW_TITLE: "Sunrise" => "Sunrise Fork Launcher (arismoko)",
    QUIT_LABEL: "Quit Sunrise Launcher" => "Quit Sunrise Fork Launcher (arismoko)",
    DISPLAY_NAME: "Sunrise Launcher" => "Sunrise Fork Launcher (arismoko)",
    EXECUTABLE: "sunrise-launcher" => "sunrise-fork-launcher",
    WINDOWS_EXECUTABLE: "sunrise-launcher.exe" => "sunrise-fork-launcher.exe",
    WINDOWS_EXECUTABLE_PREFIX: "sunrise-launcher.exe." => "sunrise-fork-launcher.exe.",
    APP_ID: "sunrise-launcher" => "io.github.arismoko.sunrisefork.launcher",
    KEYCHAIN_SERVICE: "dev.playhorizon.launcher" => "io.github.arismoko.sunrisefork.launcher",
    LOCATION_FOLDER: "sunrise-launcher-location" => "horizon-fork-arismoko-location",
    LAUNCHER_FOLDER: "sunrise-launcher" => "horizon-fork-arismoko/launcher",
    GAME_FOLDER: "sunrise" => "horizon-fork-arismoko/game",
    PUBLISHER: "Sunrise" => "arismoko",
    UNINSTALL_KEY: "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\SunriseLauncher" => "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\ArismokoSunriseForkLauncher",
    INSTALL_KEY: "Software\\Sunrise\\Launcher" => "Software\\ArismokoSunriseFork\\Launcher",
    REGISTRY_PARENT: "Software\\Sunrise" => "Software\\ArismokoSunriseFork",
    UNINSTALL_PREFIX: "sunrise-launcher-uninstall-" => "sunrise-fork-launcher-uninstall-",
    SETUP_PREFIX: "sunrise-launcher-setup" => "sunrise-fork-launcher-setup",
    PROCESS_FILTER: "IMAGENAME eq sunrise-launcher.exe" => "IMAGENAME eq sunrise-fork-launcher.exe",
}

/// Product home used by the Windows installer's About link.
pub const ABOUT_URL: &str = if cfg!(sunrise_fork_launcher) {
    sunrise_addresses::FORK_LAUNCHER_ABOUT
} else {
    sunrise_addresses::LAUNCHER_ABOUT
};

/// The complete launcher-owned suffix, not just a generic leaf selected by an override.
/// Uninstallation keeps the existing named-folder guard for relocated data as well.
pub fn owns_launcher_data_dir(path: &std::path::Path) -> bool {
    let mut actual = path.components().rev();
    std::path::Path::new(LAUNCHER_FOLDER)
        .components()
        .rev()
        .all(|expected| {
            actual.next().is_some_and(|component| {
                component
                    .as_os_str()
                    .eq_ignore_ascii_case(expected.as_os_str())
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn data_removal_requires_the_complete_owned_namespace() {
        let profile = Path::new("profile");
        assert!(owns_launcher_data_dir(&profile.join(LAUNCHER_FOLDER)));
        assert!(owns_launcher_data_dir(
            &profile.join(LAUNCHER_FOLDER.to_ascii_uppercase())
        ));
        let other = if cfg!(sunrise_fork_launcher) {
            "sunrise-launcher"
        } else {
            "horizon-fork-arismoko/launcher"
        };
        for unowned in [
            other,
            "launcher",
            "unrelated/launcher",
            "not-horizon-fork-arismoko/launcher",
        ] {
            assert!(!owns_launcher_data_dir(&profile.join(unowned)), "{unowned}");
        }
    }
}
