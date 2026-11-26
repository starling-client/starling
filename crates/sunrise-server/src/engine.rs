//! The engine: the game itself, run headless (`sunrise-client --dedicated`, crates/sunrise-client
//! game/interface/dedicated/mod.rs). It reads the vanilla files the supervisor writes, takes console commands on stdin and
//! logs on stdout.

use std::process::Command;
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpStream},
    path::Path,
    process::{Child, Stdio},
    time::Duration,
};

/// Checks that the game program starts at all: `--help` loads no game files, so it fails only when the
/// system can't load the program, as on a Linux without the ALSA and udev libraries the full game links
/// (the server build doesn't). The reason, with what to install.
pub fn check_starts(program: &Path) -> Result<(), String> {
    let mut command = crate::process::command(program);
    let output = command
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("the game ({}) can't start: {e}", program.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(start_failure(program, output.status.code(), &stderr))
}

/// What to tell the owner when the game program exits at once.
fn start_failure(program: &Path, code: Option<i32>, stderr: &str) -> String {
    let reason = stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut text = format!(
        "the game ({}) can't start{}{}",
        program.display(),
        code.map_or(String::new(), |c| format!(" (exit status {c})")),
        if reason.is_empty() {
            String::new()
        } else {
            format!(": {reason}")
        }
    );
    if reason.contains("error while loading shared libraries")
        && (reason.contains("libasound") || reason.contains("libudev"))
    {
        text.push_str(
            "\nThe full game needs ALSA's and udev's libraries (libasound.so.2, libudev.so.1). Install the \
             server build, which needs neither (packaging/server/install.sh picks it on Linux), or the \
             libraries:\n  Debian, Ubuntu: sudo apt install libasound2 libudev1   (Ubuntu 24.04 and later: \
             libasound2t64)\n  Fedora, RHEL: sudo dnf install alsa-lib systemd-libs\n  Arch: sudo pacman -S \
             alsa-lib systemd-libs\n  NixOS: enable programs.nix-ld with alsa-lib and udev in its libraries",
        );
    }
    text
}

/// Starts the engine in the server folder, with its console and log piped to the supervisor and its status
/// on the control endpoint.
pub fn spawn(
    program: &Path,
    resources: &Path,
    dir: &Path,
    low_memory: bool,
    parallel_tick: bool,
    droppable: (u32, u32),
    transfers: Option<&str>,
) -> std::io::Result<Child> {
    command(
        program,
        resources,
        dir,
        low_memory,
        parallel_tick,
        droppable,
        transfers,
    )?
    .spawn()
}

/// The command `spawn` runs.
fn command(
    program: &Path,
    resources: &Path,
    dir: &Path,
    low_memory: bool,
    parallel_tick: bool,
    droppable: (u32, u32),
    transfers: Option<&str>,
) -> std::io::Result<Command> {
    let mut command = sunrise_server_launch::command_with_transfers(
        program, resources, dir, low_memory, transfers,
    )?;
    // `[engine] parallel_tick`: the game's switch, for a host that cannot set its environment. Off leaves
    // the variable alone, so one the operator set still works.
    if parallel_tick {
        command.env("SUNRISE_PARALLEL_TICK", "1");
    }
    // `[engine] droppable_clientbound` and `droppable_serverbound`: the game's byte budgets for
    // `Droppable` mod payloads, always passed so the file is the one source.
    command
        .env(
            "SUNRISE_DROPPABLE_CLIENTBOUND_BYTES_PER_SECOND",
            droppable.0.to_string(),
        )
        .env(
            "SUNRISE_DROPPABLE_SERVERBOUND_BYTES_PER_SECOND",
            droppable.1.to_string(),
        );
    Ok(command)
}

/// How long a [`Control`] waits for each reply.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// A connection to the game's control endpoint: one JSON line asked, one answered.
pub struct Control {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Control {
    pub fn connect(dir: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(sunrise_server_launch::control_path(dir))?;
        let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not a control endpoint");
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| bad())?;
        let port = v["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(bad)?;
        let token = v["token"].as_str().ok_or_else(bad)?;
        let writer = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
        writer.set_read_timeout(Some(REPLY_TIMEOUT))?;
        let mut control = Self {
            reader: BufReader::new(writer.try_clone()?),
            writer,
        };
        let reply = control.ask(&format!("auth {token}"))?;
        if reply["ok"] != true {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the game refused the token",
            ));
        }
        Ok(control)
    }

    /// How long [`Self::ask`] waits for a reply; [`REPLY_TIMEOUT`] unless changed.
    pub fn set_reply_timeout(&self, wait: Duration) -> std::io::Result<()> {
        self.writer.set_read_timeout(Some(wait))
    }

    pub fn ask(&mut self, line: &str) -> std::io::Result<serde_json::Value> {
        // One request: the endpoint reads a line per request, so a line break would make the
        // rest a request of its own. Refused before anything is sent.
        if line.contains(['\n', '\r']) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a request is one line",
            ));
        }
        writeln!(self.writer, "{line}")?;
        self.writer.flush()?;
        let mut reply = String::new();
        if self.reader.read_line(&mut reply)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        serde_json::from_str(&reply)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    #[ignore = "daily"]
    fn a_request_with_a_line_break_is_refused_unsent() {
        let dir = std::env::temp_dir().join(format!("sunrise-control-line-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".sunrise")).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(
            sunrise_server_launch::control_path(&dir),
            format!("{{\"port\": {port}, \"token\": \"t\"}}"),
        )
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                tx.send(line.trim().to_owned()).unwrap();
                line.clear();
                let _ = writer.write_all(b"{\"ok\":true}\n");
            }
        });
        let mut control = Control::connect(&dir).unwrap();
        for request in [
            "console kick Kim0 x\nconsole op Mallory",
            "console say a\rb",
        ] {
            let refused = control.ask(request).unwrap_err();
            assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);
        }
        control.ask("console list").unwrap();
        drop(control);
        let heard: Vec<String> = rx.iter().collect();
        assert_eq!(
            heard,
            ["auth t", "console list"],
            "nothing of the refused requests was sent"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn low_memory_turns_huge_pages_off_for_the_game() {
        let thp = |low_memory| {
            let dir = std::env::temp_dir();
            let command = command(
                Path::new("sunrise-client"),
                &dir,
                &dir,
                low_memory,
                false,
                (0, 0),
                None,
            )
            .unwrap();
            command
                .get_envs()
                .find(|(k, _)| *k == "MIMALLOC_ALLOW_THP")
                .and_then(|(_, v)| v.map(|v| v.to_owned()))
        };
        assert_eq!(thp(true).as_deref(), Some(std::ffi::OsStr::new("0")));
        assert_eq!(thp(false), None);
    }

    #[test]
    fn parallel_tick_reaches_the_game_only_when_it_is_on() {
        let switch = |parallel_tick| {
            let dir = std::env::temp_dir();
            let command = command(
                Path::new("sunrise-client"),
                &dir,
                &dir,
                false,
                parallel_tick,
                (0, 0),
                None,
            )
            .unwrap();
            command
                .get_envs()
                .find(|(k, _)| *k == "SUNRISE_PARALLEL_TICK")
                .and_then(|(_, v)| v.map(|v| v.to_owned()))
        };
        assert_eq!(switch(true).as_deref(), Some(std::ffi::OsStr::new("1")));
        assert_eq!(switch(false), None);
    }

    #[test]
    fn the_droppable_budgets_always_reach_the_game() {
        let dir = std::env::temp_dir();
        let command = command(
            Path::new("sunrise-client"),
            &dir,
            &dir,
            false,
            false,
            (1000, 0),
            None,
        )
        .unwrap();
        let env = |name: &str| {
            command
                .get_envs()
                .find(|(k, _)| *k == name)
                .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
        };
        assert_eq!(
            env("SUNRISE_DROPPABLE_CLIENTBOUND_BYTES_PER_SECOND").as_deref(),
            Some("1000")
        );
        assert_eq!(
            env("SUNRISE_DROPPABLE_SERVERBOUND_BYTES_PER_SECOND").as_deref(),
            Some("0")
        );
    }

    #[test]
    fn transfers_reach_the_game_only_when_they_are_on() {
        let args = |transfers: Option<&str>| {
            let dir = std::env::temp_dir();
            let command = command(
                Path::new("sunrise-client"),
                &dir,
                &dir,
                false,
                false,
                (0, 0),
                transfers,
            )
            .unwrap();
            command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert!(!args(None).iter().any(|a| a == "--transfers"));
        let on = args(Some("expect_seconds=10"));
        let at = on.iter().position(|a| a == "--transfers").unwrap();
        assert_eq!(on[at + 1], "expect_seconds=10");
        // Before `--dedicated`: what follows it is vanilla's server arguments.
        assert!(at < on.iter().position(|a| a == "--dedicated").unwrap());
    }

    #[test]
    fn a_missing_library_says_what_to_install() {
        let program = Path::new("/opt/sunrise/sunrise-client");
        let text = start_failure(
            program,
            Some(127),
            "/opt/sunrise/sunrise-client: error while loading shared libraries: libasound.so.2: cannot open \
             shared object file: No such file or directory\n",
        );
        assert!(text.starts_with(
            "the game (/opt/sunrise/sunrise-client) can't start (exit status 127): /opt/sunrise/sunrise-client: \
             error while loading shared libraries: libasound.so.2"
        ));
        assert!(text.contains("sudo apt install libasound2 libudev1"));
        // Another library: no ALSA advice (the server build, sunrise-engine, links neither).
        let engine = start_failure(
            Path::new("/opt/sunrise/sunrise-engine"),
            Some(127),
            "/opt/sunrise/sunrise-engine: error while loading shared libraries: libgcc_s.so.1: cannot open \
             shared object file: No such file or directory\n",
        );
        assert!(!engine.contains("libasound"), "{engine}");
        let other = start_failure(program, Some(1), "something else\n");
        assert_eq!(
            other,
            "the game (/opt/sunrise/sunrise-client) can't start (exit status 1): something else"
        );
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn a_relative_folder_reaches_the_game_whole() {
        use std::os::unix::fs::PermissionsExt;
        // Relative to this process's folder (the crate's, under cargo test), as `--dir srv` gives it.
        let root = PathBuf::from(format!("reldir-test-{}", std::process::id()));
        let server = root.join("srv");
        std::fs::create_dir_all(server.join(".sunrise")).unwrap();
        std::fs::create_dir_all(root.join("res")).unwrap();
        // A game that checks, from its own folder, the folders it was given.
        let game = root.join("game");
        std::fs::write(
            &game,
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in --resources) r=$2; shift;; \
             --control-socket) c=$2; shift;; esac; shift; done\n\
             [ -d \"$r\" ] && [ -d \"$(dirname \"$c\")\" ] && [ -f server.toml ]\n",
        )
        .unwrap();
        std::fs::set_permissions(&game, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(server.join("server.toml"), "").unwrap();
        // ETXTBSY: see `check_written`.
        let mut tries = 0;
        let status = loop {
            match spawn(
                &std::path::absolute(&game).unwrap(),
                &root.join("res"),
                &server,
                false,
                false,
                (0, 0),
                None,
            ) {
                Err(e) if e.raw_os_error() == Some(26) && tries < 50 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                child => break child.unwrap().wait(),
            }
        };
        std::fs::remove_dir_all(&root).unwrap();
        let status = status.unwrap();
        assert!(status.success(), "{status}");
    }

    /// `check_starts` on a script this test process just wrote. Another test's spawn can fork while the
    /// script's write handle is still open, and its child keeps that handle until it execs: running the
    /// script meanwhile fails with ETXTBSY (Linux), so that one error is retried.
    #[cfg(unix)]
    fn check_written(program: &Path) -> Result<(), String> {
        for _ in 0..50 {
            match check_starts(program) {
                Err(e) if e.contains("Text file busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                other => return other,
            }
        }
        check_starts(program)
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn the_check_runs_the_program() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("sunrise-server-check-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = |name: &str, body: &str| {
            let path = root.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        assert_eq!(check_written(&script("starts", "exit 0")), Ok(()));
        let fails = script("fails", "echo 'no luck' >&2; exit 3");
        assert_eq!(
            check_written(&fails),
            Err(format!(
                "the game ({}) can't start (exit status 3): no luck",
                fails.display()
            ))
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
