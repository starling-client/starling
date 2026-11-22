//! `engine-command [--json]`: the game command `run` would start here, without starting it (docs/api/engine-command.md).

use serde_json::{Map, Value, json};
use std::{
    ffi::OsStr,
    io::Write,
    path::Path,
    process::{Command, ExitCode},
};
use sunrise_server_launch::{Engine, EngineCommandError};

/// The JSON's `format`: a new field keeps it, a changed or removed one makes the next.
pub const FORMAT: u32 = 1;

/// The exit code when the prepared game files are missing: preparing them is the fix.
pub const EXIT_RESOURCES: u8 = 3;

/// The exit code for a refused command line: a line on stderr, nothing on stdout.
pub const EXIT_USAGE: u8 = 2;

/// Runs `engine-command` for the folder `dir`, as this program's own supervisor would see it.
pub fn command(dir: &Path, args: &[String]) -> ExitCode {
    ExitCode::from(run(dir, args, &mut std::io::stdout().lock()))
}

/// [`command`], writing its stdout to `out`: the exit code.
fn run(dir: &Path, args: &[String], out: &mut impl Write) -> u8 {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => {
            eprintln!("sunrise-server: engine-command takes only --json");
            return EXIT_USAGE;
        }
    };
    let described = std::env::current_exe()
        .map_err(|source| EngineCommandError::Io {
            path: "sunrise-server".into(),
            source,
        })
        .and_then(|supervisor| sunrise_server_launch::engine(&supervisor, dir));
    let (code, reply) = reply(described);
    let written = if json {
        writeln!(out, "{reply}")
    } else if let Some(error) = reply["error"]["message"].as_str() {
        eprintln!("sunrise-server: {error}");
        Ok(())
    } else {
        write!(out, "{}", human(&reply))
    };
    match written {
        Ok(()) => code,
        Err(_) => 1,
    }
}

/// The exit code and the JSON for a described engine, or for why it couldn't be described.
fn reply(described: Result<Engine, EngineCommandError>) -> (u8, Value) {
    let failed = |kind: &str, message: String| {
        let code = if kind == "resources" {
            EXIT_RESOURCES
        } else {
            1
        };
        let error = json!({ "format": FORMAT, "error": { "kind": kind, "message": message } });
        (code, error)
    };
    let engine = match described {
        Ok(engine) => engine,
        Err(error) => return failed(error.kind(), error.to_string()),
    };
    match describe(&engine) {
        Ok(value) => (0, value),
        Err(message) => failed("io", message),
    }
}

fn describe(engine: &Engine) -> Result<Value, String> {
    let command: &Command = &engine.command;
    let text = |value: &OsStr| {
        value
            .to_str()
            .map(|s| Value::String(s.to_owned()))
            .ok_or_else(|| format!("{} is not valid Unicode", value.display()))
    };
    let args = command
        .get_args()
        .map(text)
        .collect::<Result<Vec<_>, _>>()?;
    let mut env = Map::new();
    for (key, value) in command.get_envs() {
        let key = text(key)?.as_str().unwrap_or_default().to_owned();
        env.insert(key, value.map(text).transpose()?.unwrap_or(Value::Null));
    }
    let cwd = command
        .get_current_dir()
        .ok_or("the engine command has no working folder")?;
    Ok(json!({
        "format": FORMAT,
        "program": text(command.get_program())?,
        "args": args,
        "cwd": text(cwd.as_os_str())?,
        "env": env,
        "resources": text(engine.resources.as_os_str())?,
    }))
}

/// The same, for a person: the command line, its folder and its environment.
fn human(reply: &Value) -> String {
    let words: Vec<&str> = std::iter::once(&reply["program"])
        .chain(reply["args"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    let mut out = format!(
        "{}\nin {}\n",
        words.join(" "),
        reply["cwd"].as_str().unwrap_or_default()
    );
    for (key, value) in reply["env"].as_object().into_iter().flatten() {
        match value.as_str() {
            Some(value) => out.push_str(&format!("{key}={value}\n")),
            None => out.push_str(&format!("{key} unset\n")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn folder(name: &str) -> PathBuf {
        // Canonical like the supervisor's own path: a temporary folder may be a link or a short name.
        let root = dunce::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "sunrise-engine-command-{name}-{}",
                std::process::id()
            ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("server")).unwrap();
        for program in ["sunrise-server", "sunrise-engine"] {
            let path = root
                .join("bin")
                .join(format!("{program}{}", std::env::consts::EXE_SUFFIX));
            std::fs::write(path, "not executable").unwrap();
        }
        root
    }

    #[test]
    fn the_reply_names_the_program_its_arguments_folder_and_environment() {
        let root = folder("ok");
        let server = root.join("server");
        std::fs::write(server.join("server.toml"), "[engine]\nlow_memory = true\n").unwrap();
        std::fs::create_dir_all(server.join("resources")).unwrap();
        std::fs::write(server.join("resources/sunrise-resources.json"), "{}").unwrap();
        let supervisor = root.join(format!(
            "bin/sunrise-server{}",
            std::env::consts::EXE_SUFFIX
        ));
        let (code, reply) = reply(sunrise_server_launch::engine(&supervisor, &server));
        let path = |p: PathBuf| p.to_str().unwrap().to_owned();
        let engine = root.join(format!(
            "bin/sunrise-engine{}",
            std::env::consts::EXE_SUFFIX
        ));
        assert_eq!(code, 0);
        assert_eq!(
            reply,
            json!({
                "format": 1,
                "program": path(engine),
                "args": [
                    "--resources", path(server.join("resources")),
                    "--control-socket", path(server.join(".sunrise/engine.json")),
                    "--dedicated", "--nogui",
                ],
                "cwd": path(server.clone()),
                "env": { "SUNRISE_NO_AUDIO": "1", "MIMALLOC_ALLOW_THP": "0" },
                "resources": path(server.join("resources")),
            })
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_game_files_exit_3_with_the_resources_kind() {
        let root = folder("unprepared");
        let server = root.join("server");
        std::fs::write(server.join("server.toml"), "").unwrap();
        let supervisor = root.join(format!(
            "bin/sunrise-server{}",
            std::env::consts::EXE_SUFFIX
        ));
        let (code, reply) = reply(sunrise_server_launch::engine(&supervisor, &server));
        assert_eq!(code, 3);
        assert_eq!(reply["format"], 1);
        assert_eq!(reply["error"]["kind"], "resources");
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("no prepared game files")
        );
        std::fs::write(server.join("server.toml"), "not TOML").unwrap();
        let (code, reply) = super::reply(sunrise_server_launch::engine(&supervisor, &server));
        assert_eq!(
            (code, reply["error"]["kind"].as_str()),
            (1, Some("configuration"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_game_program_exits_1_with_the_program_kind() {
        let root = folder("no-program");
        let server = root.join("server");
        std::fs::write(
            server.join("server.toml"),
            "[engine]\npath = \"gone/sunrise-engine\"\n",
        )
        .unwrap();
        let supervisor = root.join(format!(
            "bin/sunrise-server{}",
            std::env::consts::EXE_SUFFIX
        ));
        let (code, reply) = reply(sunrise_server_launch::engine(&supervisor, &server));
        assert_eq!((code, reply["format"].as_u64()), (1, Some(1)));
        assert_eq!(reply["error"]["kind"], "program");
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("gone/sunrise-engine")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_refused_command_line_exits_2_and_writes_nothing_to_stdout() {
        let root = folder("usage");
        for args in [vec!["--yaml"], vec!["--json", "--json"]] {
            let args: Vec<String> = args.into_iter().map(String::from).collect();
            let mut out = Vec::new();
            assert_eq!(run(&root.join("server"), &args, &mut out), 2, "{args:?}");
            assert!(out.is_empty(), "{args:?}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
