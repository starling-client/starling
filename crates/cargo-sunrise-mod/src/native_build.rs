//! `native-build`: native mods compiled against an unpacked SDK (docs/modding/src/reference/sdk/08-native-build.md).
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;
use sunrise_mod_package::{internals::ListedInternal, manifest::Native, sdk::Mixins};
use sunrise_native_build::{check as checks, contract, discover, host, rust::Unpacked};

/// The plan's build folder, the host's root.
static WORK: OnceLock<PathBuf> = OnceLock::new();

/// Reads the plan from standard input and builds it: 0 built, 1 failed (a `failed` event says why), 2 no plan, 3 another contract.
pub fn run(args: &[String]) -> ExitCode {
    if !args.is_empty() {
        eprintln!(
            "usage: cargo sunrise-mod native-build < plan.json (docs/modding/src/reference/sdk/08-native-build.md)"
        );
        return ExitCode::from(2);
    }
    let mut input = std::io::stdin().lock();
    let plan = match contract::read_plan(&mut input) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(match error {
                contract::PlanError::Contract(_) => 3,
                contract::PlanError::Invalid(_) => 2,
            });
        }
    };
    install_host(&plan.work);
    match contract::run(&plan, &mut input, &mut std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// The tool as the native build's host: `work` its folder, English, its own network, used files noted for the reply.
pub(crate) fn install_host(work: &Path) {
    let _ = WORK.set(work.to_owned());
    host::install(host::Host {
        tr: |en| en,
        root: || WORK.get().cloned().unwrap_or_default(),
        get,
        mark_used: contract::note_used,
    });
}

/// Fetches through the proxy named in the environment (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY`).
fn get(url: &str) -> Result<host::Body, String> {
    match sunrise_proxy::get(url, &[], None) {
        Ok(response) => Ok(Some(Box::new(response.into_reader()))),
        Err(error) if error.status() == Some(404) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Reads a check plan from standard input and prints one reply line: 0 checked, 1 an error reply (a file it names didn't read), 2 no plan, 3 another contract.
pub fn check(args: &[String]) -> ExitCode {
    let plan: checks::Plan = match plan(args, "native-check") {
        Ok(plan) => plan,
        Err(code) => return code,
    };
    let (code, line) = checked(&plan);
    println!("{line}");
    ExitCode::from(code)
}

/// The exit code and the reply line for `plan`: a tool from before `native-check` prints no such line, which is how a caller tells.
fn checked(plan: &checks::Plan) -> (u8, serde_json::Value) {
    match checks::check(plan)
        .and_then(|reply| serde_json::to_value(reply).map_err(|e| e.to_string()))
    {
        Ok(reply) => (0, reply),
        Err(message) => (
            1,
            serde_json::json!({
                "contract": contract::CONTRACT,
                "error": {"kind": "unread", "message": message},
            }),
        ),
    }
}

/// Reads one plan line of `T` from standard input, or the exit code that says why it isn't one: 2 no plan, 3 another contract.
fn plan<T: serde::de::DeserializeOwned>(args: &[String], command: &str) -> Result<T, ExitCode> {
    if !args.is_empty() {
        eprintln!(
            "usage: cargo sunrise-mod {command} < plan.json (docs/modding/src/reference/sdk/08-native-build.md)"
        );
        return Err(ExitCode::from(2));
    }
    contract::read_line(&mut std::io::stdin().lock()).map_err(|error| {
        eprintln!("error: {error}");
        ExitCode::from(match error {
            contract::PlanError::Contract(_) => 3,
            contract::PlanError::Invalid(_) => 2,
        })
    })
}

/// Prints `reply`, or the error reply of `kind`, as one line: 0 or 1.
fn answer(reply: Result<serde_json::Value, (&'static str, String)>) -> ExitCode {
    let (code, line) = match reply {
        Ok(mut reply) => {
            reply["contract"] = contract::CONTRACT.into();
            (0, reply)
        }
        Err((kind, message)) => (
            1,
            serde_json::json!({"contract": contract::CONTRACT, "error": {"kind": kind, "message": message}}),
        ),
    };
    println!("{line}");
    ExitCode::from(code)
}

/// `native-discover`: the mod's internals (`{"internals": [...]}`), or its pinned game's build data copied to `catalog` (`{"catalog": path}`).
pub fn discover(args: &[String]) -> ExitCode {
    let plan: contract::DiscoverPlan = match plan(args, "native-discover") {
        Ok(plan) => plan,
        Err(code) => return code,
    };
    install_host(&plan.work);
    answer(discovered(&plan))
}

/// Find the internals a project names, using the Rust and mixins identity in its signed SDK.
pub(crate) fn project_internals(
    dir: &Path,
    native: &Native,
) -> Result<Vec<ListedInternal>, String> {
    discover_project(dir, native, |rust, dir, mixins| {
        discover::internals(rust, dir, Some(mixins))
    })
}

/// Build an independent catalog of the pinned source with this project's modules applied.
pub(crate) fn project_catalog(dir: &Path, native: &Native) -> Result<PathBuf, String> {
    discover_project(dir, native, |rust, dir, mixins| {
        let snapshots = dir.join("target/sunrise-mod/catalog-snapshots");
        fs::create_dir_all(&snapshots).map_err(|e| format!("{}: {e}", snapshots.display()))?;
        let destination = snapshots.join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        ));
        discover::catalog(rust, dir, Some(mixins), &destination)
    })
}

fn discover_project<T>(
    dir: &Path,
    native: &Native,
    run: impl FnOnce(&Unpacked, &Path, &Mixins) -> Result<T, discover::Error>,
) -> Result<T, String> {
    let dir = dunce::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let cache = crate::dev::cache_dir()?;
    let work = cache.join("discovery");
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    // Source staging replaces kept files; a second CLI must not change them mid-discovery.
    let lock = work.join("discovery.lock");
    let _lease = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .and_then(|file| file.lock().map(|()| file))
        .map_err(|e| format!("{}: {e}", lock.display()))?;
    let sdk = crate::dev::native_sdk(&cache, &native.sunrise)?;
    install_host(&work);
    sunrise_native_build::clear_log()?;
    run(&sdk.rust, &dir, &sdk.index.mixins).map_err(|e| {
        format!(
            "{e}\nDiscovery log: {}",
            sunrise_native_build::log_path().display()
        )
    })
}

fn discovered(plan: &contract::DiscoverPlan) -> Result<serde_json::Value, (&'static str, String)> {
    let kind = |error: sunrise_native_build::discover::Error| match error {
        sunrise_native_build::discover::Error::SourceUnavailable { .. } => {
            ("source", error.to_string())
        }
        sunrise_native_build::discover::Error::Check(_) => ("discover", error.to_string()),
    };
    match &plan.catalog {
        Some(output) => {
            sunrise_native_build::discover::catalog(&plan.toolchain, &plan.mods, None, output)
                .map(|path| serde_json::json!({"catalog": path}))
                .map_err(kind)
        }
        None => sunrise_native_build::discover::internals(&plan.toolchain, &plan.mods, None)
            .map(|internals| serde_json::json!({"internals": internals}))
            .map_err(kind),
    }
}

/// `native-lock`: the mod's registry dependencies pinned in its `Cargo.lock` (`{"lock": path}`).
pub fn lock(args: &[String]) -> ExitCode {
    let plan: contract::LockPlan = match plan(args, "native-lock") {
        Ok(plan) => plan,
        Err(code) => return code,
    };
    install_host(&plan.work);
    answer(locked(&plan))
}

fn locked(plan: &contract::LockPlan) -> Result<serde_json::Value, (&'static str, String)> {
    sunrise_native_build::bundle_in(&plan.mods)
        .and_then(|found| {
            found.ok_or_else(|| format!("{} is not a native mod.", plan.mods.display()))
        })
        .and_then(|bundle| {
            sunrise_native_build::registry_generate::generate(
                || Ok(plan.toolchain.clone()),
                sunrise_native_build::registry_generate::game_lock,
                &bundle,
                plan.update,
            )
            .map(|()| bundle.dir.join("Cargo.lock"))
        })
        .map(|path| serde_json::json!({"lock": path}))
        .map_err(|message| ("lock", message))
}

/// `native-layout`: a local prebuilt game checked to be the plan's build, host and side, then laid out as slices (`{"package": {runtime, api, engine}}`).
pub fn layout(args: &[String]) -> ExitCode {
    let plan: contract::LayoutPlan = match plan(args, "native-layout") {
        Ok(plan) => plan,
        Err(code) => return code,
    };
    answer(laid_out(&plan))
}

fn laid_out(plan: &contract::LayoutPlan) -> Result<serde_json::Value, (&'static str, String)> {
    use sunrise_native_build::prebuilt;
    let layout = || -> Result<prebuilt::Package, String> {
        prebuilt::check_identity(&plan.prebuilt, &plan.build, &plan.host, plan.target.into())?;
        let _ = std::fs::remove_dir_all(&plan.out);
        prebuilt::lay_out(&plan.prebuilt, &plan.out, true)
    };
    layout()
        .and_then(|package| serde_json::to_value(package).map_err(|e| e.to_string()))
        .map(|package| serde_json::json!({ "package": package }))
        .map_err(|message| ("layout", message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_that_cant_read_its_mods_replies_with_an_error_line() {
        // A file where the plan names a folder of mods: it doesn't read as one.
        let file = std::env::temp_dir().join(format!("native-check-file-{}", std::process::id()));
        std::fs::write(&file, "not a folder").unwrap();
        let (code, reply) = checked(&checks::Plan {
            contract: contract::CONTRACT,
            mods: file.clone(),
            build: "b".repeat(40),
            surface: None,
            build_data: None,
        });
        assert_eq!(code, 1);
        assert_eq!(reply["contract"], contract::CONTRACT);
        assert_eq!(reply["error"]["kind"], "unread");
        assert!(
            reply["error"]["message"]
                .as_str()
                .is_some_and(|m| !m.is_empty())
        );
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn discover_and_lock_name_their_step_when_the_folder_is_no_native_mod() {
        let empty = std::env::temp_dir().join(format!("native-tools-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let toolchain: sunrise_native_build::rust::Unpacked =
            serde_json::from_value(serde_json::json!({
                "dir": empty, "target": "x86_64-unknown-linux-gnu", "cargo_home": empty
            }))
            .unwrap();
        let (kind, _) = discovered(&contract::DiscoverPlan {
            contract: contract::CONTRACT,
            work: empty.join("work"),
            toolchain: toolchain.clone(),
            mods: empty.clone(),
            catalog: None,
        })
        .unwrap_err();
        assert_eq!(kind, "discover");
        let (kind, message) = locked(&contract::LockPlan {
            contract: contract::CONTRACT,
            work: empty.join("work"),
            toolchain,
            mods: empty.clone(),
            update: false,
        })
        .unwrap_err();
        assert_eq!(kind, "lock");
        assert!(message.contains("is not a native mod"), "{message}");
        std::fs::remove_dir_all(empty).unwrap();
    }

    #[test]
    fn a_folder_that_is_no_prebuilt_game_is_refused_by_layout() {
        let empty =
            std::env::temp_dir().join(format!("native-layout-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let (kind, _) = laid_out(&contract::LayoutPlan {
            contract: contract::CONTRACT,
            prebuilt: empty.clone(),
            build: "b".repeat(40),
            host: "x86_64-unknown-linux-gnu".into(),
            target: contract::TargetName::Client,
            out: empty.join("out"),
        })
        .unwrap_err();
        assert_eq!(kind, "layout");
        assert!(!empty.join("out").exists(), "nothing is laid out");
        std::fs::remove_dir_all(empty).unwrap();
    }
}
