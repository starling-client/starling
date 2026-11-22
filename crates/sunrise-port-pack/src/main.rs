//! `sunrise-port-pack`: port a Java modpack to Sunrise (`docs/modpacks.md`).
//!
//! - `import <pack> <work>`: a `.mrpack`, CurseForge zip or instance folder
//!   to `<work>/pack.json`.
//! - `inventory <work> [--offline]`: Modrinth metadata (cached in
//!   `<work>/modrinth.json`), the jars in the instance or `<work>/jars/`,
//!   classes, tiers and dependency layers in `<work>/inventory.json` and
//!   `inventory.csv`.
//! - `fetch <work> [--from <folder>] [<mod id>...]`: the jars of the
//!   portable mods (content, data tier; or the ones named) into
//!   `<work>/jars/`: copied from the player's folder, or downloaded from the
//!   pack's own links or Modrinth's CDN, hash checked. What only CurseForge
//!   has needs a key and is listed in `<work>/fetch.json`.
//! - `carry <work> [--resources <prepared>] [<mod id>...]`: the content
//!   mods' data and assets into `<work>/mods/<id>/carry/`, a folder mod for
//!   local use. With the NeoForge oracle's `<work>/oracle/facts.json`
//!   (`tools/neoforge_oracle.py`), blocks and items come from its facts;
//!   `--resources` limits templates to the blocks Sunrise's tables have.
//! - `report <work>`: the coverage report, per mod ported, partial, not yet
//!   or not needed with the reason, in `<work>/coverage.json` and
//!   `coverage.md`.
//! - `check <jar> <out>`: one mod jar on its own, as the launcher's mod
//!   browser installs it: carried over into `<out>/carry/`, its classes
//!   read, and the verdict (Works, Partly, Doesn't run) in
//!   `<out>/check.json`.
//! - `events <jar or folder> [--hooks <hook-counterparts.json>]`: the NeoForge
//!   events a mod listens to and the capabilities it uses, each with the
//!   hooks for it (the catalog's `neoforge` names, built in or `--hooks`);
//!   over a folder of jars, ranked by how many mods use each.
//! - `port <pack> <work> [--from <folder>] [--resources <prepared>]`: every
//!   step for the portable mods, as the launcher's "Import modpack" runs it.
//!   A jar that is only a data pack goes to the profile's packs for new worlds
//!   (`<work>/home/new-world-datapacks/`), not through the carry.
//! - `upgrade-datapack <pack> <out> | --world <world>`: a 26.2 data pack
//!   (zip or folder) with its world generation rewritten in 26.3's formats
//!   (`docs/datapack-upgrade.md`), into the folder `<out>`, or into
//!   `<world>/datapacks/<pack name>-26.3`.
//!
//! - `port-work <work> [--from <folder>] [--resources <prepared>]`: the port step alone, on an imported work folder.
//! - `status <work>`: what the work folder holds, as the launcher shows it.
//! - `profile <work>`: the staged resource packs installed into the profile's home, recorded in
//!   `<work>/profile-resourcepacks.json`.
//! - `--json` (not `upgrade-datapack` or `events`): JSON lines on stdout, the reply last (`docs/api/port-pack.md`).
//!
//! The work folder belongs to the user: jars and carried-over files stay
//! there and are never uploaded or committed.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};
use sunrise_port_pack::{events, fetch, inventory::Record, pipeline, status, upgrade};

const USAGE: &str = "usage: sunrise-port-pack import <pack> <work> | inventory <work> [--offline] | fetch <work> [--from <folder>] [<mod id>...] | carry <work> [--resources <prepared>] [<mod id>...] | report <work> | check <jar> <out> | port <pack> <work> [--from <folder>] [--resources <prepared>] | port-work <work> [--from <folder>] [--resources <prepared>] | status <work> | profile <work> | upgrade-datapack <pack> (<out> | --world <world>) | events <jar or folder> [--hooks <hook-counterparts.json>]; --json for JSON lines (not upgrade-datapack or events); --version; --help";

/// This release's version: release builds set `SUNRISE_VERSION` from the tag (docs/versioning.md).
const VERSION: &str = match option_env!("SUNRISE_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--version" || flag == "-V" => {
            println!("sunrise-port-pack {VERSION}");
            return ExitCode::SUCCESS;
        }
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let json = take_flag(&mut args, "--json");
    if json
        && matches!(
            args.first().map(String::as_str),
            Some("upgrade-datapack" | "events")
        )
    {
        eprintln!("error: --json is not for {}\n{USAGE}", args[0]);
        return ExitCode::from(EXIT_USAGE);
    }
    let say = |line: String| match json {
        true => println!("{}", json!({ "message": line })),
        false => println!("{line}"),
    };
    let result = run(&mut args, json, &say);
    let (code, reply) = outcome(result, json);
    if let Some(reply) = reply {
        println!("{reply}");
    }
    ExitCode::from(code)
}

/// The exit code for a command line it doesn't understand: the usage on stderr, no JSON.
const EXIT_USAGE: u8 = 2;

/// The step a command was on when it failed: its `kind` in the error reply (docs/api/port-pack.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Import,
    Inventory,
    Fetch,
    Carry,
    Report,
    Check,
    Port,
    Status,
    Profile,
    UpgradeDatapack,
    Events,
}

impl Step {
    fn kind(self) -> &'static str {
        match self {
            Step::Import => "import",
            Step::Inventory => "inventory",
            Step::Fetch => "fetch",
            Step::Carry => "carry",
            Step::Report => "report",
            Step::Check => "check",
            Step::Port => "port",
            Step::Status => "status",
            Step::Profile => "profile",
            Step::UpgradeDatapack => "upgrade-datapack",
            Step::Events => "events",
        }
    }
}

/// Why a command didn't answer.
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    /// A missing argument, an option without its value or an unknown command.
    Usage,
    /// A step that couldn't finish, with why.
    Step(Step, String),
}

/// Tags a step's error with the step.
fn at(step: Step) -> impl Fn(String) -> Failure {
    move |message| Failure::Step(step, message)
}

/// The exit code and the line to print last on stdout, for what `run` returned.
fn outcome(result: Result<Option<Value>, Failure>, json: bool) -> (u8, Option<Value>) {
    match result {
        Ok(reply) => (0, reply.filter(|_| json)),
        Err(Failure::Usage) => {
            eprintln!("{USAGE}");
            (EXIT_USAGE, None)
        }
        Err(Failure::Step(step, message)) => {
            eprintln!("error: {message}");
            let reply = json!({
                "format": status::FORMAT,
                "error": { "kind": step.kind(), "message": message },
            });
            (1, json.then_some(reply))
        }
    }
}

/// Runs the command in `args`; with `json`, returns the reply its last line carries.
fn run(args: &mut Vec<String>, json: bool, say: &dyn Fn(String)) -> Result<Option<Value>, Failure> {
    let from = option(args, "--from")?;
    let resources = option(args, "--resources")?;
    let world = option(args, "--world")?;
    let hooks = option(args, "--hooks")?;
    let offline = take_flag(args, "--offline");
    let path = |i: usize| args.get(i).map(Path::new).ok_or(Failure::Usage);
    let rest = || args.get(2..).unwrap_or_default().to_vec();
    let log = &|line: String| say(line);
    // The plain output's last lines; JSON has the reply instead.
    let plain = |line: String| {
        if !json {
            println!("{line}");
        }
    };
    let state = |work: &Path| -> Result<Option<Value>, Failure> {
        match json {
            true => status::status(work)
                .and_then(|found| serde_json::to_value(found).map_err(|e| e.to_string()))
                .map(Some)
                .map_err(at(Step::Status)),
            false => Ok(None),
        }
    };
    match args.first().map(String::as_str) {
        Some("import") => {
            let (pack, work) = (path(1)?, path(2)?);
            pipeline::import(pack, work, log).map_err(at(Step::Import))?;
            state(work)
        }
        Some("inventory") => {
            let work = path(1)?;
            pipeline::inventory(work, offline, log).map_err(at(Step::Inventory))?;
            state(work)
        }
        Some("fetch") => {
            let work = path(1)?;
            let records: Vec<Record> =
                pipeline::read_json(&work.join("inventory.json")).map_err(at(Step::Fetch))?;
            let only = rest();
            let want = |r: &Record| {
                if only.is_empty() {
                    pipeline::portable(r)
                } else {
                    only.contains(&r.key)
                }
            };
            fetch::fetch(work, &records, from.as_deref(), &want, log).map_err(at(Step::Fetch))?;
            state(work)
        }
        Some("carry") => {
            let work = path(1)?;
            pipeline::carry(work, resources.as_deref(), &rest(), log).map_err(at(Step::Carry))?;
            state(work)
        }
        Some("report") => {
            let work = path(1)?;
            pipeline::report(work, log).map_err(at(Step::Report))?;
            plain(work.join("coverage.md").display().to_string());
            state(work)
        }
        Some("check") => {
            let (jar, out) = (path(1)?, path(2)?);
            let c = sunrise_port_pack::check::check(jar, out).map_err(at(Step::Check))?;
            if json {
                return Ok(Some(json!({ "format": status::FORMAT, "check": c })));
            }
            println!("{}: {}. {}", c.mod_id, c.verdict.label(), c.summary);
            for p in &c.ports {
                println!("  ports: {p}");
            }
            for s in &c.skipped {
                println!("  skipped: {s}");
            }
            Ok(None)
        }
        Some("port") => {
            let (pack, work) = (path(1)?, path(2)?);
            pipeline::import(pack, work, log).map_err(at(Step::Import))?;
            pipeline::inventory(work, offline, log).map_err(at(Step::Inventory))?;
            pipeline::port(work, resources.as_deref(), from.as_deref(), log)
                .map_err(at(Step::Port))?;
            plain(work.join("coverage.md").display().to_string());
            state(work)
        }
        Some("port-work") => {
            let work = path(1)?;
            pipeline::port(work, resources.as_deref(), from.as_deref(), log)
                .map_err(at(Step::Port))?;
            plain(work.join("coverage.md").display().to_string());
            state(work)
        }
        Some("status") => {
            let work = path(1)?;
            if json {
                return state(work);
            }
            let found = status::status(work).map_err(at(Step::Status))?;
            let s = &found.summary;
            println!(
                "{}: {} files, {} of {} mods carried over ({} partly), {} portable, {} loaded",
                pipeline::title(&found.pack),
                found.rows.len(),
                s.carried,
                s.needing,
                s.partial,
                found.portable,
                found.mods.len()
            );
            for line in &found.left_off {
                println!("  left off: {line}");
            }
            Ok(None)
        }
        Some("profile") => {
            let work = path(1)?;
            let report = pipeline::profile(work).map_err(at(Step::Profile))?;
            for (pack, outcome) in &report.files {
                plain(format!("{pack}: {:?}", outcome.state));
            }
            state(work)
        }
        Some("upgrade-datapack") => {
            let pack = path(1)?;
            let out = match &world {
                Some(world) => {
                    let name = pack.file_stem().unwrap_or_default().to_string_lossy();
                    world.join("datapacks").join(format!("{name}-26.3"))
                }
                None => path(2)?.to_path_buf(),
            };
            let report = upgrade::upgrade_into(pack, &out).map_err(at(Step::UpgradeDatapack))?;
            for line in report.lines() {
                println!("{line}");
            }
            println!("{}", out.display());
            Ok(None)
        }
        Some("events") => {
            let jars = path(1)?;
            let catalog = match &hooks {
                Some(file) => std::fs::read_to_string(file)
                    .map_err(|e| Failure::Step(Step::Events, format!("{}: {e}", file.display())))?,
                None => events::CATALOG.to_owned(),
            };
            print!(
                "{}",
                events::report(jars, &catalog).map_err(at(Step::Events))?
            );
            Ok(None)
        }
        _ => Err(Failure::Usage),
    }
}

/// Removes `--name <value>` from `args`.
fn option(args: &mut Vec<String>, name: &str) -> Result<Option<PathBuf>, Failure> {
    match args.iter().position(|a| a == name) {
        Some(i) if i + 1 < args.len() => {
            let value = args.remove(i + 1);
            args.remove(i);
            Ok(Some(PathBuf::from(value)))
        }
        Some(_) => Err(Failure::Usage),
        None => Ok(None),
    }
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != name);
    args.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ran(args: &[&str], json: bool) -> (u8, Option<Value>) {
        let mut args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        outcome(run(&mut args, json, &|_| {}), json)
    }

    #[test]
    fn a_command_line_it_does_not_understand_exits_2_without_json() {
        for args in [
            &["status"][..],
            &["import", "pack.mrpack"],
            &["fetch", "work", "--from"],
            &["unknown", "work"],
            &[],
        ] {
            assert_eq!(ran(args, true), (2, None), "{args:?}");
        }
    }

    #[test]
    fn a_failed_step_replies_with_its_kind_and_message() {
        let work =
            std::env::temp_dir().join(format!("sunrise-port-pack-nowork-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        let work = work.to_str().unwrap();
        let (code, reply) = ran(&["fetch", work], true);
        let reply = reply.unwrap();
        assert_eq!((code, reply["format"].as_u64()), (1, Some(1)));
        assert_eq!(reply["error"]["kind"], "fetch");
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("inventory.json")
        );
        assert_eq!(ran(&["fetch", work], false), (1, None));
    }

    #[test]
    fn profile_installs_the_staged_packs_into_the_profiles_home_once() {
        let work =
            std::env::temp_dir().join(format!("sunrise-port-pack-profile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        let pack = work.join("resourcepacks/glow");
        std::fs::create_dir_all(pack.join("assets/minecraft/textures")).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack": {"pack_format": 34, "description": "glow"}}"#,
        )
        .unwrap();
        std::fs::write(pack.join("assets/minecraft/textures/a.png"), b"png").unwrap();
        std::fs::write(
            work.join("pack.json"),
            r#"{"name": "Cozy", "format": "mrpack", "minecraft": "1.21.1", "loader": "neoforge",
                "loader_version": "21.1.1", "files": [], "overrides": {}}"#,
        )
        .unwrap();
        let installed = work.join("home/resourcepacks/glow/pack.mcmeta");
        let (code, reply) = ran(&["profile", work.to_str().unwrap()], true);
        let reply = reply.unwrap();
        assert_eq!(code, 0, "{reply}");
        assert!(installed.is_file());
        let profile = &reply["resource_reports"]["profile"]["files"]["resourcepacks/glow"];
        assert!(profile["state"].is_string(), "{reply}");
        std::fs::write(&installed, "the player's own").unwrap();
        assert_eq!(ran(&["profile", work.to_str().unwrap()], true).0, 0);
        assert_eq!(
            std::fs::read_to_string(&installed).unwrap(),
            "the player's own"
        );
        std::fs::remove_dir_all(&work).unwrap();
    }
}
