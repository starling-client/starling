//! The Rust toolchain a build compiles with, and the settings every build command shares.
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Rust release the SDK and its mods build with (`tools/ci/rust-version`).
pub const VERSION: &str = include_str!("../../../tools/ci/rust-version").trim_ascii();

/// An unpacked Rust toolchain that commands run with: the launcher's managed one, or a plan's [`Unpacked`].
pub trait Toolchain {
    /// `program` (this toolchain's cargo, or a tool built with it) set up to use only this toolchain.
    fn command(&self, program: &Path) -> Command;
    fn cargo(&self) -> PathBuf;
    fn rustc(&self) -> PathBuf;
    /// Adds the target's own Cargo configuration to a cargo command.
    fn configure_cargo(&self, cmd: &mut Command);
    /// Where Rust's standard library for this computer is.
    fn target_libdir(&self) -> Option<PathBuf>;
    /// The host's target triple.
    fn target(&self) -> &str;
    /// The Cargo home builds download crates into.
    fn cargo_home(&self) -> PathBuf;
    /// The toolchain's C and link programs (MinGW-w64 on Windows), whose runtime the game loads.
    fn mingw_bin(&self) -> PathBuf;
}

/// The Windows x64 host, whose GNU runtime libraries ship beside the game.
pub const WINDOWS_X64_TARGET: &str = "x86_64-pc-windows-gnu";

/// A toolchain named by its folder, as a plan gives it: the caller already set up its C tools.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Unpacked {
    /// The folder holding `bin/cargo` and `bin/rustc`.
    pub dir: PathBuf,
    /// Its host triple.
    pub target: String,
    /// The Cargo home of builds.
    pub cargo_home: PathBuf,
    /// `--config` values every cargo command gets.
    #[serde(default)]
    pub cargo_config: Vec<String>,
    /// The folder of its C and link programs, when it brings them (Windows).
    #[serde(default)]
    pub c_bin: Option<PathBuf>,
}

impl Unpacked {
    fn exe(&self, name: &str) -> PathBuf {
        self.dir
            .join("bin")
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }
}

impl Toolchain for Unpacked {
    fn command(&self, program: &Path) -> Command {
        let mut cmd = Command::new(program);
        let path = std::env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(self.dir.join("bin"))
            .chain(self.c_bin.clone())
            .chain(std::env::split_paths(&path));
        if let Some(root) = crate::host::root()
            .ancestors()
            .last()
            .filter(|root| root.has_root())
        {
            cmd.current_dir(root);
        }
        cmd.env("PATH", std::env::join_paths(paths).unwrap_or(path))
            .env("RUSTC", self.rustc())
            .env("CARGO_HOME", &self.cargo_home)
            // Nix's compiler wrapper, told to enforce purity, drops every `-I` and `-L` outside
            // the Nix store. This build's sources are in `cargo_home`, never there, so a C or C++
            // dependency would lose its own include folders.
            .env("NIX_ENFORCE_PURITY", "0");
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(scrubbed) {
                cmd.env_remove(name);
            }
        }
        cmd
    }

    fn cargo(&self) -> PathBuf {
        self.exe("cargo")
    }

    fn rustc(&self) -> PathBuf {
        self.exe("rustc")
    }

    fn configure_cargo(&self, cmd: &mut Command) {
        for value in &self.cargo_config {
            cmd.arg("--config").arg(value);
        }
    }

    fn target_libdir(&self) -> Option<PathBuf> {
        Some(
            self.dir
                .join("lib")
                .join("rustlib")
                .join(&self.target)
                .join("lib"),
        )
    }

    fn target(&self) -> &str {
        &self.target
    }

    fn cargo_home(&self) -> PathBuf {
        self.cargo_home.clone()
    }

    fn mingw_bin(&self) -> PathBuf {
        self.c_bin
            .clone()
            .unwrap_or_else(|| self.dir.join("mingw64").join("bin"))
    }
}

/// Whether the player's environment variable `name` stays out of a build: it would pick another
/// Rust or wrap it (rustup, `RUSTC_WRAPPER`), or change the compiler's flags (`RUSTFLAGS`, a
/// profile, incremental, the build's target or linker: `CARGO_BUILD_*` but the jobs,
/// `CARGO_TARGET_*`),
/// so the build would differ from the release's and recompile everything after any change.
pub fn scrubbed(name: &str) -> bool {
    const PREFIXES: [&str; 3] = ["CARGO_PROFILE_", "CARGO_BUILD_", "CARGO_TARGET_"];
    // Windows' variable names ignore case.
    let name = name.to_ascii_uppercase();
    // The number of jobs changes no flag: a player may still limit it.
    if name == "CARGO_BUILD_JOBS" {
        return false;
    }
    SCRUBBED.contains(&name.as_str()) || PREFIXES.iter().any(|p| name.starts_with(p))
}

/// `ioprio_set`'s "which" for one process.
#[cfg(target_os = "linux")]
const IOPRIO_WHO_PROCESS: libc::c_long = 1;
/// The best-effort I/O class (2) at its lowest level (7), as `ioprio_set` takes it.
#[cfg(target_os = "linux")]
const LOW_IO: libc::c_long = (2 << 13) | 7;

/// Runs `cmd` (a build) below the player's own programs, so the desktop and a game in play stay
/// smooth while it compiles; with the computer otherwise idle it still gets every core. Unix: nice
/// 19, and on Linux the lowest best-effort I/O level (not the idle class, which gets the disk only
/// when nothing else uses it, so a game streaming chunks could stall a build);
/// Windows: below-normal priority, keeping the no-console flag
/// (`creation_flags` sets the whole value, so this sets both).
pub fn low_priority(cmd: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: between fork and exec the closure only makes two system calls, which are
        // async-signal-safe, on the child itself; a failure (not allowed) leaves the priority.
        unsafe {
            cmd.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, 19);
                #[cfg(target_os = "linux")]
                {
                    libc::syscall(
                        libc::SYS_ioprio_set,
                        IOPRIO_WHO_PROCESS,
                        0 as libc::c_long,
                        LOW_IO,
                    );
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    cmd
}

pub const SCRUBBED: [&str; 8] = [
    "RUSTUP_TOOLCHAIN",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    "CARGO_INCREMENTAL",
];

/// What to do when Windows' Smart App Control refused one of the build's programs (#5911). It
/// lets only signed programs or ones Microsoft knows run, and a build runs programs it has just
/// compiled (every crate's build script, horizon-mixins, the game) and the toolchain's own
/// (`dlltool.exe`). Windows refuses them with os error 4551 ("An Application Control policy has
/// blocked this file"). Nothing is checked before the build: with Smart App Control on, a build
/// whose programs already ran before it was turned on still succeeds.
pub fn smart_app_control() -> String {
    crate::host::tr("Windows' Smart App Control is blocking the build (os error 4551). \
It only lets signed programs run, and a native build runs programs no one has signed: the ones it \
compiles on this computer and some of the compiler's own. To build native mods, turn it off in \
Windows Security > App & browser control > Smart App Control settings, then build again. On some \
Windows versions it can't be turned back on without reinstalling Windows. Content mods (.smod) need \
no build and work with it on.")
    .to_string()
}

/// What to install when the link of a native build cannot find udev or ALSA, which the game
/// links on Linux: the linker only names the missing `-l` flags.
pub fn system_libraries() -> String {
    crate::host::tr(
        "The build could not find udev or ALSA, system libraries the game links on \
Linux. Install their development files, then build again: `libudev-dev` and `libasound2-dev` on \
Debian and Ubuntu, `systemd-devel` and `alsa-lib-devel` on Fedora, `udev` and `alsa-lib` in the \
shell's packages on NixOS.",
    )
    .to_string()
}

/// Whether `text` (a line of a build's output) is a linker not finding udev or ALSA: rust-lld's
/// `unable to find library -ludev`, or GNU ld's `cannot find -ludev: No such file or directory`.
pub fn missing_system_libraries(text: &str) -> bool {
    ["unable to find library ", "cannot find "]
        .iter()
        .any(|said| {
            text.match_indices(said).any(|(at, _)| {
                let flag = text[at + said.len()..].split_whitespace().next();
                // GNU ld ends the flag with one colon; a colon inside it names another library.
                let flag = flag.map(|flag| flag.strip_suffix(':').unwrap_or(flag));
                matches!(flag, Some("-ludev" | "-lasound"))
            })
        })
}

/// Whether `text` (a build's output, or the error starting one of its programs) is Windows
/// refusing a program under an Application Control policy, Smart App Control's.
pub fn blocked_by_app_control(text: &str) -> bool {
    text.contains("os error 4551")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every host the launcher ships a toolchain for: commands are checked for each, on any machine.
    pub const HOSTS: [&str; 3] = [
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        WINDOWS_X64_TARGET,
    ];

    pub fn unpacked(dir: impl Into<PathBuf>, target: &str) -> Unpacked {
        let dir = dir.into();
        Unpacked {
            cargo_home: dir.join("cargo"),
            dir,
            target: target.into(),
            cargo_config: Vec::new(),
            c_bin: None,
        }
    }

    /// The toolchain installed at `dir`, for the host its rustc names.
    pub fn system(dir: PathBuf) -> Unpacked {
        let rustc = dir
            .join("bin")
            .join(format!("rustc{}", std::env::consts::EXE_SUFFIX));
        let out = Command::new(rustc).arg("-vV").output().unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let host = text
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .unwrap();
        unpacked(dir, host)
    }

    /// Under `NIX_ENFORCE_PURITY=1` Nix's compiler wrapper drops the include folders of a C++
    /// dependency in the build's own Cargo home, so every build command turns it off.
    #[test]
    fn build_commands_turn_off_nix_purity_filtering() {
        let cmd = unpacked("toolchain", HOSTS[1]).command(Path::new("cargo"));
        let purity = cmd
            .get_envs()
            .find(|(name, _)| *name == "NIX_ENFORCE_PURITY")
            .and_then(|(_, value)| value);
        assert_eq!(purity, Some(std::ffi::OsStr::new("0")));
    }

    #[test]
    fn only_a_linker_missing_exactly_udev_or_alsa_is_recognised() {
        for line in [
            "  = note: rust-lld: error: unable to find library -ludev",
            "          rust-lld: error: unable to find library -lasound",
            "/usr/bin/ld: cannot find -ludev: No such file or directory",
            "/usr/bin/ld: cannot find -lasound: No such file or directory",
        ] {
            assert!(missing_system_libraries(line), "{line}");
        }
        for line in [
            "rust-lld: error: unable to find library -lasound_extra",
            "/usr/bin/ld: cannot find -ludev2: No such file or directory",
            "rust-lld: error: unable to find library -lasound:plugin",
            "/usr/bin/ld: cannot find -ludev:plugin: No such file or directory",
            "/usr/bin/ld: cannot find -ludev:plugin:: No such file or directory",
            "rust-lld: error: unable to find library -lz",
            "error: linking with `cc` failed: exit status: 1",
        ] {
            assert!(!missing_system_libraries(line), "{line}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "daily"]
    fn builds_run_at_the_lowest_priority() {
        // Read back from the running child itself: its niceness and its I/O priority.
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        let mut child = low_priority(&mut cmd).spawn().unwrap();
        let pid = child.id();
        // The priority is set between fork and exec; wait for the exec.
        std::thread::sleep(std::time::Duration::from_millis(200));
        // SAFETY: plain system calls on a child process of this test.
        let (nice, io) = unsafe {
            let nice = libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t);
            let io = libc::syscall(
                libc::SYS_ioprio_get,
                IOPRIO_WHO_PROCESS,
                pid as libc::c_long,
            );
            (nice, io)
        };
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(nice, 19);
        assert_eq!(io, LOW_IO);
    }
}
