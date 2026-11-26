//! The dedicated engine's configuration, install layout and command description.
//! Shared by the supervisor and launcher without starting either or loading supervisor services.

pub mod config;
mod install;
pub mod prune;
pub mod resources;

use config::Config;
pub use install::Install;
use std::{
    fmt, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
/// Prepared-file contract for tools that launch the game.
pub use sunrise_core::prepared_profile;

/// The game's program names, most specific first: the server build (the game without sound or
/// controllers, so it needs neither ALSA nor udev on Linux; `sunrise-server-linux-x86_64.tar.gz` holds it),
/// else the full game.
const PROGRAMS: [&str; 2] = ["sunrise-engine", "sunrise-client"];

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// The game program: `engine.path`, else one installed beside `sunrise-server` (in the release in use when
/// it was installed by the installers), else one on the PATH.
pub fn program(config: &Config, dir: &Path) -> Result<PathBuf, String> {
    program_in(config, dir, Install::find().as_ref())
}

/// [`program`], for the install `install`.
pub fn program_in(
    config: &Config,
    dir: &Path,
    install: Option<&Install>,
) -> Result<PathBuf, String> {
    let executable = std::env::current_exe().ok();
    program_near(config, dir, install, executable.as_deref())
}

fn program_near(
    config: &Config,
    dir: &Path,
    install: Option<&Install>,
    executable: Option<&Path>,
) -> Result<PathBuf, String> {
    if !config.engine.path.is_empty() {
        let path = dir.join(&config.engine.path);
        return path
            .is_file()
            .then_some(path)
            .ok_or_else(|| format!("engine.path {} is not a file", config.engine.path));
    }
    // Installed: the release `current` points at now, so a switch applies at the next start.
    let beside = install
        .map(Install::programs)
        .or_else(|| executable.and_then(|e| e.parent().map(Path::to_path_buf)));
    let path_dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    PROGRAMS
        .iter()
        .flat_map(|name| {
            beside
                .iter()
                .chain(&path_dirs)
                .map(move |d| d.join(exe(name)))
        })
        .find(|p| p.is_file())
        .ok_or_else(|| {
            "the game program (sunrise-client) is neither beside sunrise-server nor on the PATH: \
             install it there, or set engine.path in server.toml"
                .into()
        })
}

/// A failure while describing a configured dedicated-engine command, without starting it.
#[derive(Debug)]
#[non_exhaustive]
pub enum EngineCommandError {
    Configuration(String),
    Program(String),
    Resources(String),
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for EngineCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(error) | Self::Program(error) | Self::Resources(error) => {
                f.write_str(error)
            }
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}
impl EngineCommandError {
    /// The error's `kind` in `engine-command --json` (docs/api/engine-command.md): one per variant.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Configuration(_) => "configuration",
            Self::Program(_) => "program",
            Self::Resources(_) => "resources",
            Self::Io { .. } => "io",
        }
    }
}

impl std::error::Error for EngineCommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The engine a supervisor would run: its command and the prepared game files that command names.
#[derive(Debug)]
pub struct Engine {
    pub command: Command,
    pub resources: PathBuf,
    /// The supervisor must prepare these game files before starting this command.
    pub resources_pending: bool,
}

/// Describe the engine the supervisor at `supervisor` would run in `dir`.
/// Uses the same config, install/current, PATH, resources and command builder as the supervisor.
/// If `resources_pending` is true, the command describes the selected folder but cannot be run
/// until the supervisor prepares it. This function starts nothing, writes nothing and downloads nothing.
pub fn engine(supervisor: &Path, dir: &Path) -> Result<Engine, EngineCommandError> {
    // Not `fs::canonicalize`: its `\\?\` prefix on Windows would reach the game's program and resources
    // paths, where a later `/` in a string no longer means a separator.
    let supervisor = dunce::canonicalize(supervisor).map_err(|source| EngineCommandError::Io {
        path: supervisor.to_owned(),
        source,
    })?;
    let config = Config::load(dir).map_err(EngineCommandError::Configuration)?;
    if let Ok(text) = std::fs::read_to_string(dir.join(config::FILE)) {
        for key in config::retired_in(&text) {
            eprintln!("server.toml: {key} is no longer read and can be removed");
        }
    }
    let install = Install::around(&supervisor);
    let program = program_near(&config, dir, install.as_ref(), Some(&supervisor))
        .map_err(EngineCommandError::Program)?;
    let (resources, resources_pending) = match resources::select(
        &config,
        dir,
        Some(&program),
        install.as_ref().map(|install| install.root.as_path()),
        false,
    )
    .map_err(EngineCommandError::Resources)?
    {
        resources::Selection::Ready(path) => (path, false),
        resources::Selection::NeedsPreparation(path) => (path, true),
    };
    let command =
        command(&program, &resources, dir, config.engine.low_memory).map_err(|source| {
            EngineCommandError::Io {
                path: dir.to_owned(),
                source,
            }
        })?;
    let resources = std::path::absolute(&resources).map_err(|source| EngineCommandError::Io {
        path: resources.clone(),
        source,
    })?;
    Ok(Engine {
        command,
        resources,
        resources_pending,
    })
}

/// The Minecraft version this server is pinned to; prepared game files must be stamped with it.
pub const GAME_VERSION: &str = sunrise_core::version::VERSION_ID;

/// The command the supervisor runs, without starting it.
pub fn command(
    program: &Path,
    resources: &Path,
    dir: &Path,
    low_memory: bool,
) -> std::io::Result<Command> {
    command_with_transfers(program, resources, dir, low_memory, None)
}

/// The supervisor command with optional transfer settings, placed before vanilla's server arguments.
pub fn command_with_transfers(
    program: &Path,
    resources: &Path,
    dir: &Path,
    low_memory: bool,
    transfers: Option<&str>,
) -> std::io::Result<Command> {
    // The game runs in the server folder, so a folder given relative to ours (`--dir srv`) would put its
    // control endpoint in `srv/srv/.sunrise`, which doesn't exist.
    let dir = std::path::absolute(dir)?;
    let resources = std::path::absolute(resources)?;
    let mut command = Command::new(program);
    command
        .arg("--resources")
        .arg(resources)
        .arg("--control-socket")
        .arg(control_path(&dir));
    if let Some(transfers) = transfers {
        command.arg("--transfers").arg(transfers);
    }
    command
        .args(["--dedicated", "--nogui"])
        .current_dir(&dir)
        .env("SUNRISE_NO_AUDIO", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The terminal's Ctrl-C reaches the supervisor alone, which then stops the game cleanly.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut command,
        windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP
            | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
    );
    // `[engine] low_memory`: mimalloc then keeps transparent huge pages off. Mostly empty huge pages cost
    // 60 MB idle and 190 MB at 20 players, and save ~1 % of tick time (docs/hosting.md).
    if low_memory {
        command.env("MIMALLOC_ALLOW_THP", "0");
    }
    Ok(command)
}

/// The game's control endpoint in a server folder (`--control-socket`): TCP on 127.0.0.1 with a token, on every OS.
pub fn control_path(dir: &Path) -> PathBuf {
    dir.join(".sunrise/engine.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use prepared_profile::{MANIFEST, Profile, SCHEMA, STAMP};

    fn prepared_server(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        for name in [
            "facts.json",
            "block-tags.json",
            "commands.json",
            "registries.json",
        ] {
            std::fs::write(dir.join(name), "{}").unwrap();
        }
        std::fs::write(
            dir.join(MANIFEST),
            serde_json::json!({
                "schema": SCHEMA,
                "minecraft": GAME_VERSION,
                "profile": "server",
                "server_files": ["facts.json"],
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join(STAMP),
            serde_json::json!({"output_version": Profile::Server.output_version()}).to_string(),
        )
        .unwrap();
    }

    #[test]
    fn library_command_uses_the_given_supervisor_and_config_without_starting() {
        // Canonical like the supervisor's own path: a temp folder can be a link or a short name.
        let root = dunce::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("sunrise-server-command-{}", std::process::id()));
        prepared_server(&root);
        let bin = root.join("bin");
        let server = root.join("server");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&server).unwrap();
        let supervisor = bin.join(format!("sunrise-server{}", std::env::consts::EXE_SUFFIX));
        let game = bin.join(format!("sunrise-engine{}", std::env::consts::EXE_SUFFIX));
        let client = bin.join(format!("sunrise-client{}", std::env::consts::EXE_SUFFIX));
        // Invalid executables prove description does not spawn or preflight any program.
        for path in [&supervisor, &game, &client] {
            std::fs::write(path, "not executable").unwrap();
        }
        let mut config = Config::default();
        config.engine.resources = root.to_str().unwrap().into();
        config.engine.low_memory = true;
        std::fs::write(server.join("server.toml"), crate::config::render(&config)).unwrap();
        let described = engine(&supervisor, &server).unwrap();
        assert!(!described.resources_pending);
        let command = described.command;
        assert_eq!(command.get_program(), game);
        assert_eq!(command.get_current_dir(), Some(server.as_path()));
        let args: Vec<_> = command.get_args().map(std::ffi::OsStr::to_owned).collect();
        assert_eq!(
            args,
            vec![
                "--resources".into(),
                root.clone().into_os_string(),
                "--control-socket".into(),
                server.join(".sunrise/engine.json").into_os_string(),
                "--dedicated".into(),
                "--nogui".into(),
            ]
        );
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "MIMALLOC_ALLOW_THP"
                    && value == Some(std::ffi::OsStr::new("0")))
        );
        assert!(!server.join("server.properties").exists());
        assert!(!server.join(".sunrise").exists());

        let external = root.join("external-engine");
        std::fs::write(&external, "not executable either").unwrap();
        config.engine.path = "../external-engine".into();
        std::fs::write(server.join("server.toml"), crate::config::render(&config)).unwrap();
        let described = engine(&supervisor, &server).unwrap();
        assert!(!described.resources_pending);
        assert_eq!(
            described.command.get_program(),
            server.join("../external-engine")
        );
        config.backups.every = "6 hours".into();
        std::fs::write(server.join("server.toml"), crate::config::render(&config)).unwrap();
        assert!(matches!(
            engine(&supervisor, &server),
            Err(EngineCommandError::Configuration(_))
        ));
        std::fs::write(server.join("server.toml"), "not TOML").unwrap();
        assert!(matches!(
            engine(&supervisor, &server),
            Err(EngineCommandError::Configuration(_))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_supervisor_describes_the_current_release_not_its_own() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-server-command-current-{}",
            std::process::id()
        ));
        let old = root.join("releases/old");
        let current = root.join("releases/new");
        let server = root.join("server");
        for dir in [&old, &current, &server] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let supervisor = old.join("sunrise-server");
        for path in [
            supervisor.clone(),
            old.join("sunrise-engine"),
            current.join("sunrise-engine"),
        ] {
            std::fs::write(path, "not executable").unwrap();
        }
        std::os::unix::fs::symlink(&current, root.join("current")).unwrap();
        std::fs::write(server.join("server.toml"), "").unwrap();
        prepared_server(&server.join("resources"));
        let described = engine(&supervisor, &server).unwrap();
        assert!(!described.resources_pending);
        let command = described.command;
        assert_eq!(command.get_program(), root.join("current/sunrise-engine"));
        assert_eq!(command.get_current_dir(), Some(server.as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn low_memory_turns_huge_pages_off_for_the_game() {
        let thp = |low_memory| {
            let dir = std::env::temp_dir();
            let command = command(Path::new("sunrise-client"), &dir, &dir, low_memory).unwrap();
            command
                .get_envs()
                .find(|(k, _)| *k == "MIMALLOC_ALLOW_THP")
                .and_then(|(_, v)| v.map(|v| v.to_owned()))
        };
        assert_eq!(thp(true).as_deref(), Some(std::ffi::OsStr::new("0")));
        assert_eq!(thp(false), None);
    }
}
