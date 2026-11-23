//! Distributor wrappers must survive disabling and re-enabling menu integration.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::Command;

fn desktop_entry(home: &Path, wrapper: Option<&str>, args: &[&str]) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"));
    command
        .arg("desktop-entry")
        .args(args)
        .env("SUNRISE_LAUNCHER_HOME", home.join("launcher"))
        .env("SUNRISE_HOME", home.join("game"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("SUNRISE_LAUNCHER_TRASH", home.join("trash"))
        .env("SUNRISE_LAUNCHER_CLIPBOARD", home.join("clipboard"))
        .env("SUNRISE_LAUNCHER_DESKTOP", home.join("desktop"))
        .env("SUNRISE_NO_AUDIO", "1")
        .env("XDG_DATA_HOME", home.join("data"))
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env_remove("SUNRISE_LAUNCHER_DESKTOP_EXEC");
    if let Some(wrapper) = wrapper {
        command.env("SUNRISE_LAUNCHER_DESKTOP_EXEC", wrapper);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn menu_registration_preserves_the_package_wrapper() {
    let home = std::env::temp_dir().join(format!("sunrise-desktop-wrapper-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let entry = home
        .join("data/applications")
        .join(if cfg!(sunrise_fork_launcher) {
            "io.github.arismoko.sunrisefork.launcher.desktop"
        } else {
            "sunrise-launcher.desktop"
        });
    let other = home
        .join("data/applications")
        .join(if cfg!(sunrise_fork_launcher) {
            "sunrise-launcher.desktop"
        } else {
            "io.github.arismoko.sunrisefork.launcher.desktop"
        });
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::write(&other, "other launcher registration").unwrap();

    // Migrate a registration that pointed at the private executable.
    desktop_entry(&home, None, &[]);
    let original = std::fs::read_to_string(&entry).unwrap();
    assert!(original.contains(&format!(
        "Exec=\"{}\"\n",
        env!("CARGO_BIN_EXE_sunrise-launcher")
    )));

    let wrapper = "/usr/bin/sunrise-launcher";
    for _ in 0..2 {
        desktop_entry(&home, Some(wrapper), &[]);
        let text = std::fs::read_to_string(&entry).unwrap();
        assert!(text.contains(&format!("Exec=\"{wrapper}\"\n")));
        assert!(text.contains(&format!("TryExec={wrapper}\n")));
        desktop_entry(&home, Some(wrapper), &["--remove"]);
        assert!(!entry.exists());
    }

    // An unset or empty override keeps standalone installations working.
    desktop_entry(&home, Some(""), &[]);
    assert_eq!(std::fs::read_to_string(&entry).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(&other).unwrap(),
        "other launcher registration"
    );
    std::fs::remove_dir_all(&home).unwrap();
}
