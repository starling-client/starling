//! Shared scratch, SDK-invocation and dedicated-installation helpers for `own-sdk` daily tests.
#![allow(dead_code)]

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// One isolated scratch directory under `CARGO_TARGET_TMPDIR` with stub resources for
/// registration-only or `--initSettings` server runs.
pub struct OwnSdkScratch {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl OwnSdkScratch {
    pub fn new() -> Self {
        let target = Path::new(env!("CARGO_TARGET_TMPDIR"));
        fs::create_dir_all(target).unwrap();
        let dir = tempfile::tempdir_in(target).unwrap();
        let root = dir.path().canonicalize().unwrap();
        let resources = root.join("resources");
        fs::create_dir(&resources).unwrap();
        for (name, json) in [
            (
                "sunrise-resources.json",
                r#"{"schema":7,"minecraft":"26.3"}"#,
            ),
            ("sunrise-prepare.json", r#"{"output_version":9}"#),
            ("registries.json", "{}"),
        ] {
            fs::write(resources.join(name), json).unwrap();
        }
        Self { _dir: dir, root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn stub_resources(&self) -> PathBuf {
        self.root.join("resources")
    }

    pub fn tool_output(&self, dir: &Path, args: &[&str], registry: Option<&str>) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"));
        cmd.current_dir(dir)
            .env("SUNRISE_MOD_CACHE", self.root.join("cache"))
            .env("SUNRISE_HOME", self.root.join("home"))
            .env("SUNRISE_LAUNCHER_HOME", self.root.join("launcher"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1")
            .args(args);
        if let Some(registry) = registry {
            cmd.env("SUNRISE_MOD_REGISTRY_TEST_BASE", registry);
        }
        cmd.output().unwrap()
    }

    pub fn tool(&self, dir: &Path, args: &[&str]) -> String {
        passed(self.tool_output(dir, args, None))
    }

    /// Create a starter native mod `id` and write `source` to its `src/lib.rs`.
    pub fn new_native_mod(&self, id: &str, source: &str) -> PathBuf {
        self.tool(&self.root, &["new", id, "--template", "native"]);
        let folder = self.root.join(id);
        fs::write(folder.join("src/lib.rs"), source).unwrap();
        folder
    }

    /// Build `project` (plus `libraries`) for the dedicated server target via `cargo sunrise-mod run`,
    /// keep a copy of its staged `app/` installation under `builds/<name>`, and return
    /// `(kept_installation, stdout)`.
    pub fn stage_dedicated_app(
        &self,
        name: &str,
        project: &Path,
        libraries: &[&Path],
        extra_server_args: &[&str],
    ) -> (PathBuf, String) {
        let run = project.join("target/sunrise-mod/run");
        fs::create_dir_all(run.join("game")).unwrap();
        fs::write(run.join("game/eula.txt"), "eula=true\n").unwrap();
        let resources = self.stub_resources();
        let mut args = vec!["run"];
        for library in libraries {
            args.extend(["--with", library.to_str().unwrap()]);
        }
        args.extend([
            "--resources",
            resources.to_str().unwrap(),
            "--",
            "--dedicated",
        ]);
        args.extend(extra_server_args);
        let stdout = self.tool(project, &args);
        let executable = stdout
            .lines()
            .find_map(|line| line.strip_prefix("Starting "))
            .and_then(|line| line.split(" with ").next())
            .map(PathBuf::from)
            .expect("run names the program it starts");
        let kept = self.root.join("builds").join(name);
        copy_dir(&run.join("app"), &kept);
        fs::write(
            kept.join(".program"),
            executable.file_name().unwrap().as_encoded_bytes(),
        )
        .unwrap();
        (kept, stdout)
    }

    /// Combine separately built server installations into a single installation under `combined/<name>`:
    /// starts from `base` and copies every extra installation's library files, `native/<mod>/`
    /// content folders, and `native-mods.json` rows.
    pub fn combine_installations(&self, name: &str, base: &Path, extras: &[&Path]) -> PathBuf {
        let folder = self.root.join("combined").join(name);
        copy_dir(base, &folder);
        let mut combined = record(&folder);
        let combined_rows = combined["libraries"].as_array_mut().unwrap();
        for extra in extras {
            let extra_record = record(extra);
            for row in extra_record["libraries"].as_array().unwrap() {
                let file = row["file"].as_str().unwrap();
                fs::copy(extra.join(file), folder.join(file)).unwrap();
                combined_rows.push(row.clone());
            }
            let extra_native = extra.join(sunrise_mod_package::mod_list::NATIVE_DIR);
            if extra_native.is_dir() {
                let target_native = folder.join(sunrise_mod_package::mod_list::NATIVE_DIR);
                fs::create_dir_all(&target_native).unwrap();
                for entry in fs::read_dir(&extra_native).unwrap() {
                    let entry = entry.unwrap();
                    let dest = target_native.join(entry.file_name());
                    if entry.file_type().unwrap().is_dir() {
                        copy_dir(&entry.path(), &dest);
                    } else {
                        fs::copy(entry.path(), dest).unwrap();
                    }
                }
            }
        }
        fs::write(folder.join("native-mods.json"), combined.to_string()).unwrap();
        folder
    }

    /// Start the installed dedicated server binary with `game_dir` and `resources`.
    pub fn start_dedicated(
        &self,
        installation: &Path,
        game_dir: &Path,
        resources: &Path,
        extra_args: &[&str],
    ) -> Output {
        let program = fs::read_to_string(installation.join(".program")).unwrap();
        let home = tempfile::tempdir_in(&self.root).unwrap();
        let mut command = Command::new(installation.join(program));
        command
            .arg("--game-dir")
            .arg(game_dir)
            .arg("--resources")
            .arg(resources)
            .arg("--dedicated")
            .args(extra_args)
            .env("SUNRISE_HOME", home.path())
            .env("SUNRISE_NO_AUDIO", "1")
            .env_remove("SUNRISE_SESSION_FILE");
        #[cfg(target_os = "linux")]
        command.env("LD_LIBRARY_PATH", installation);
        command.output().unwrap()
    }
}

pub fn passed(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

pub fn record(installation: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(installation.join("native-mods.json")).unwrap())
        .unwrap()
}

pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
