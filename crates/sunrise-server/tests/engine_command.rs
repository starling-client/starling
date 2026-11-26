#![cfg(unix)]

use serde_json::Value;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::{Command, Stdio},
};

#[test]
#[ignore = "daily"]
fn init_prepares_the_pending_slot_without_changing_existing_settings() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("engine-command-init-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let install = root.join("install");
    let release = install.join("releases/test-release");
    let server = root.join("server");
    for dir in [&release, &server] {
        fs::create_dir_all(dir).unwrap();
    }
    symlink(&release, install.join("current")).unwrap();
    let supervisor = release.join("sunrise-server");
    fs::copy(env!("CARGO_BIN_EXE_sunrise-server"), &supervisor).unwrap();
    fs::write(
        release.join("sunrise-engine"),
        "This engine must not be started",
    )
    .unwrap();
    let preparer = release.join("sunrise-prepare");
    // Real supervisor and CLI, with a packaged preparer stub that verifies the server-mode
    // invocation and writes a current server profile without Mojang's files or a download.
    fs::write(
        &preparer,
        r#"#!/bin/sh
set -eu
[ "$1" = --server ]
[ "$2" = --output ]
case $# in
    3) ;;
    5) [ "$4" = --root ] && [ -f "$5/research-lock.json" ] ;;
    *) exit 1 ;;
esac
out=$3
mkdir -p "$out"
for name in facts.json block-tags.json commands.json registries.json; do
    printf '{}\n' > "$out/$name"
done
printf '%s\n' '{"schema":7,"minecraft":"26.3","profile":"server","server_files":["facts.json","block-tags.json","commands.json","registries.json"]}' > "$out/sunrise-resources.json"
printf '%s\n' '{"output_version":1}' > "$out/sunrise-prepare.json"
"#,
    )
    .unwrap();
    fs::set_permissions(&preparer, fs::Permissions::from_mode(0o755)).unwrap();
    let settings = "# Operator settings\n[world]\nname = 'reserved'\n";
    fs::write(server.join("server.toml"), settings).unwrap();
    let env_dirs = [
        ("SUNRISE_HOME", root.join("home")),
        ("SUNRISE_LAUNCHER_HOME", root.join("launcher")),
        ("XDG_DATA_HOME", root.join("data")),
        ("XDG_CONFIG_HOME", root.join("config")),
    ];
    let cli = |args: &[&str]| {
        Command::new(&supervisor)
            .current_dir(&root)
            .envs(env_dirs.iter().map(|(key, path)| (*key, path)))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .arg("--dir")
            .arg(&server)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let probe = |pending: bool| {
        let result = cli(&["engine-command", "--json"]);
        assert!(result.status.success(), "{result:?}");
        let reply: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(reply["resources_pending"], pending, "{reply}");
        assert_eq!(reply["resources"], reply["args"][1], "{reply}");
        reply["resources"].as_str().unwrap().to_owned()
    };
    let path = probe(true);
    assert_eq!(
        Path::new(&path),
        install.join("game-files/sunrise-client/26.3-server")
    );
    assert!(!install.join("game-files").exists());
    assert_eq!(fs::read_dir(&server).unwrap().count(), 1);
    let eula_path = server.join("eula.txt");
    let refused_eula = "# Not accepted by the owner\neula=false\n";
    fs::write(&eula_path, refused_eula).unwrap();
    let denied = cli(&["init", "--yes"]);
    assert!(!denied.status.success(), "{denied:?}");
    assert!(!install.join("game-files").exists());
    assert_eq!(fs::read_to_string(&eula_path).unwrap(), refused_eula);

    let eula = "# Accepted by the owner\neula=true\n";
    fs::write(&eula_path, eula).unwrap();
    let prepared = cli(&["init", "--yes"]);
    assert!(prepared.status.success(), "{prepared:?}");
    assert_eq!(probe(false), path);
    assert!(Path::new(&path).join("sunrise-prepare.json").is_file());
    assert_eq!(
        fs::read_to_string(server.join("server.toml")).unwrap(),
        settings
    );
    assert_eq!(fs::read_to_string(eula_path).unwrap(), eula);
    assert_eq!(fs::read_dir(server).unwrap().count(), 2);
    fs::remove_dir_all(root).unwrap();
}
