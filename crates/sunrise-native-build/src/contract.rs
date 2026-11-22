//! The JSON contract of `cargo sunrise-mod native-build`: one plan line in, one event per line out.
use crate::prebuilt::{self, Package, Route};
use crate::rust::Unpacked;
use crate::{Target, api, dependencies, library_record, scan_bundles};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The contract this tool speaks: a plan naming another is refused, so a caller never misreads a reply.
pub const CONTRACT: u32 = 2;

/// What to build, the first line of standard input.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// Must be [`CONTRACT`].
    pub contract: u32,
    /// The build's own folder: `build.log`, the horizon-mixins tool, sources and kept compiles.
    pub work: PathBuf,
    /// The Rust toolchain the SDK was built with.
    pub toolchain: Unpacked,
    /// The SDK's three compiled slices for this computer and target, unpacked and verified by the caller.
    pub package: Package,
    /// The mods to compile: a mod's folder, or a folder of mod folders.
    pub mods: PathBuf,
    /// `client` (the game) or `server` (the headless engine).
    pub target: TargetName,
    /// Whether registry crates may be added (their build scripts and proc macros run); absent asks.
    #[serde(default)]
    pub registry: Option<bool>,
    /// Build horizon-mixins from its pinned source instead of downloading its prebuilt release.
    #[serde(default)]
    pub mixins_from_source: bool,
}

/// What `native-discover` reads: one mod's internals, or its pinned game's build data with its modules applied.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoverPlan {
    /// Must be [`CONTRACT`].
    pub contract: u32,
    /// The tool's own folder: the pinned game's source and the discovery caches.
    pub work: PathBuf,
    /// The Rust toolchain the mod's SDK was built with.
    pub toolchain: Unpacked,
    /// The mod's folder.
    pub mods: PathBuf,
    /// Where to copy `internals.json`, a folder that doesn't exist yet; absent, the reply lists the mod's internals.
    #[serde(default)]
    pub catalog: Option<PathBuf>,
}

/// What `native-lock` reads: the mod whose registry dependencies to pin in its `Cargo.lock`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LockPlan {
    /// Must be [`CONTRACT`].
    pub contract: u32,
    /// The tool's own folder.
    pub work: PathBuf,
    /// The Rust toolchain the mod's SDK was built with.
    pub toolchain: Unpacked,
    /// The mod's folder.
    pub mods: PathBuf,
    /// Resolve again even when the mod has a lock.
    #[serde(default)]
    pub update: bool,
}

/// What `native-layout` reads: a local prebuilt game (`tools/prebuilt_game.py build`) to check and lay out as an SDK's slices.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPlan {
    /// Must be [`CONTRACT`].
    pub contract: u32,
    /// The local prebuilt game's folder; it is never changed.
    pub prebuilt: PathBuf,
    /// The build id (the game's full commit) it must be.
    pub build: String,
    /// The host triple it must be built for.
    pub host: String,
    /// `client` or `server`: the side it must be.
    pub target: TargetName,
    /// Where to lay out its slices, by links: replaced when it exists.
    pub out: PathBuf,
}

/// [`Target`] as a plan names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetName {
    Client,
    Server,
}

impl From<TargetName> for Target {
    fn from(name: TargetName) -> Self {
        match name {
            TargetName::Client => Target::Client,
            TargetName::Server => Target::Server,
        }
    }
}

/// One line of standard output.
#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// One mod's library starts compiling: which, and how far through the mods.
    Mod {
        id: String,
        done: usize,
        total: usize,
    },
    /// Registry crates need a yes: the caller answers with one line, `true` or `false`.
    Consent {
        added_crates: usize,
        build_scripts: Vec<String>,
        proc_macros: Vec<String>,
    },
    /// It built: the executable and the libraries to install beside it, in load order.
    Built {
        /// The route, as the build record (`native-mods.json`) names it.
        route: String,
        binary: PathBuf,
        libraries: Vec<PathBuf>,
        /// Each library's identity, the build record's `libraries`.
        records: Vec<library_record::Library>,
        /// The mods compiled, in dependency order.
        mods: Vec<String>,
        /// Each compiled mod's `[[internals]]`, every absent hash taken from the SDK's catalog: the build record's form.
        internals: BTreeMap<String, serde_json::Value>,
        /// The horizon-mixins revision the build ran.
        mixins: String,
        /// The kept folders and files the build used, for the caller's cache to count as used.
        used: Vec<PathBuf>,
    },
    /// It didn't: the error as the build log ends with it.
    Failed {
        error: String,
        /// The mod the error is that mod's own, and how: the one a caller leaves out next time.
        blame: Option<Blame>,
        /// The kept folders and files the build used before it stopped.
        used: Vec<PathBuf>,
    },
}

/// The mod a failed build stops at.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Blame {
    #[serde(rename = "mod")]
    pub mod_id: String,
    pub kind: BlameKind,
}

/// How a mod stopped a build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlameKind {
    /// Its own library didn't compile.
    Compile,
    /// It has a build script, which a prebuilt build refuses.
    BuildScript,
    /// It asks for Cargo features, which a prebuilt build refuses.
    Features,
}

/// What the build used of its kept folders, for the events.
static USED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Notes that the build used `path`: the tool's [`crate::host::Host::mark_used`].
pub fn note_used(path: &Path) -> Result<(), String> {
    USED.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(path.to_owned());
    Ok(())
}

fn take_used() -> Vec<PathBuf> {
    let mut used = std::mem::take(
        &mut *USED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    used.sort();
    used.dedup();
    used
}

/// The mod among `ids` that `error` blames by name, as the build log says it.
fn blame(error: &str, ids: &[String]) -> Option<Blame> {
    ids.iter().find_map(|id| {
        let kind = if error.contains(&format!("{} failed (", prebuilt::compiling(id))) {
            BlameKind::Compile
        } else if error.contains(&prebuilt::Refusal::BuildScript { mod_id: id.clone() }.to_string())
        {
            BlameKind::BuildScript
        } else if error.contains(&prebuilt::Refusal::Features { mod_id: id.clone() }.to_string()) {
            BlameKind::Features
        } else {
            return None;
        };
        Some(Blame {
            mod_id: id.clone(),
            kind,
        })
    })
}

/// Why the first line of standard input isn't a plan this tool builds.
#[derive(Debug, PartialEq, Eq)]
pub enum PlanError {
    /// It names another contract (or none a number can say), read before any other field.
    Contract(serde_json::Value),
    /// It isn't a plan of this contract: not one JSON object, a field missing, wrong or unknown.
    Invalid(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contract(contract) => write!(
                f,
                "the plan speaks contract {contract}; this tool speaks contract {CONTRACT}: use the tool of the SDK the plan was written for"
            ),
            Self::Invalid(error) => write!(f, "the plan: {error}"),
        }
    }
}

/// Reads the plan from the first line of `input`. The contract is checked first, so a plan of another contract is
/// refused for that and never for a field this contract doesn't have.
pub fn read_plan(input: &mut dyn BufRead) -> Result<Plan, PlanError> {
    read_line(input)
}

/// Reads one line of `input` as a `T` of this contract: [`read_plan`] for `native-build`, and `native-check`'s plan.
pub fn read_line<T: serde::de::DeserializeOwned>(input: &mut dyn BufRead) -> Result<T, PlanError> {
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|e| PlanError::Invalid(e.to_string()))?;
    let value: serde_json::Value =
        serde_json::from_str(&line).map_err(|e| PlanError::Invalid(e.to_string()))?;
    let contract = value
        .as_object()
        .ok_or_else(|| PlanError::Invalid("not a JSON object".into()))?
        .get("contract")
        .ok_or_else(|| PlanError::Invalid("missing field `contract`".into()))?;
    if contract.as_u64() != Some(u64::from(CONTRACT)) {
        return Err(PlanError::Contract(contract.clone()));
    }
    serde_json::from_value(value).map_err(|e| PlanError::Invalid(e.to_string()))
}

/// Writes `event` as one line of `out`.
pub fn emit(out: &mut dyn Write, event: &Event) -> Result<(), String> {
    let line = serde_json::to_string(event).map_err(|e| e.to_string())?;
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())
}

/// Builds `plan`, saying each step on `out` and reading consent answers from `input`; ends with [`Event::Built`] or [`Event::Failed`].
pub fn run(plan: &Plan, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<(), String> {
    crate::mixins_from_source(plan.mixins_from_source);
    let events = std::cell::RefCell::new(out);
    let ids = std::cell::RefCell::new(Vec::new());
    let built = compile(
        plan,
        &|event| emit(&mut **events.borrow_mut(), &event),
        input,
        &ids,
    );
    let out = events.into_inner();
    match built {
        Ok(event) => emit(out, &event),
        Err(error) => {
            emit(
                out,
                &Event::Failed {
                    blame: blame(&error, &ids.borrow()),
                    error: error.clone(),
                    used: take_used(),
                },
            )?;
            Err(error)
        }
    }
}

fn compile(
    plan: &Plan,
    say: &dyn Fn(Event) -> Result<(), String>,
    input: &mut dyn BufRead,
    ids: &std::cell::RefCell<Vec<String>>,
) -> Result<Event, String> {
    let work = crate::root();
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let side = match Target::from(plan.target) {
        Target::Client => sunrise_mod_package::manifest::Side::Client,
        Target::Server => sunrise_mod_package::manifest::Side::Server,
    };
    let mods: Vec<_> = scan_bundles(&plan.mods)?
        .into_iter()
        .filter(|bundle| bundle.manifest.native.is_some())
        .collect();
    if mods.is_empty() {
        return Err(format!("{} holds no native mod.", plan.mods.display()));
    }
    *ids.borrow_mut() = mods
        .iter()
        .map(|bundle| bundle.manifest.id.clone())
        .collect();
    let declared: Vec<Vec<dependencies::Dependency>> = mods
        .iter()
        .map(|bundle| {
            dependencies::build_dependencies(&bundle.manifest, side, |id| {
                mods.iter().any(|project| project.manifest.id == id)
            })
        })
        .collect();
    let inputs: Vec<_> = mods
        .iter()
        .zip(&declared)
        .map(|(bundle, deps)| dependencies::Mod {
            id: &bundle.manifest.id,
            version: &bundle.manifest.version,
            has_api: true,
            code: true,
            dependencies: deps,
        })
        .collect();
    let resolution = dependencies::resolve_links(&inputs)
        .map_err(|error| format!("code link requirements refused: {error}"))?;
    let projects: Vec<_> = mods
        .iter()
        .map(api::Project::read)
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    if let Some(reason) = prebuilt::refusal(&projects) {
        return Err(reason.to_string());
    }
    let route = Route::of(&projects);
    let asked = std::cell::RefCell::new(input);
    let consent = |inventory: &crate::RegistryInventory| {
        plan.registry.unwrap_or_else(|| {
            let question = Event::Consent {
                added_crates: inventory.added_crates,
                build_scripts: inventory.build_scripts.clone(),
                proc_macros: inventory.proc_macros.clone(),
            };
            say(question).is_ok() && answer(&mut **asked.borrow_mut())
        })
    };
    let (binary, libraries) = prebuilt::compile(
        &plan.toolchain,
        &plan.package,
        &crate::root().join("prebuilt-build"),
        &plan.mods,
        route,
        &projects,
        &declared,
        &resolution,
        Some(&consent),
        &|id, done, total| {
            let _ = say(Event::Mod {
                id: id.to_owned(),
                done,
                total,
            });
        },
    )?;
    let catalog = crate::internals::Catalog::read(&plan.package.api)?;
    let mut internals = BTreeMap::new();
    for &index in &resolution.order {
        let manifest = &mods[index].manifest;
        let pinned =
            serde_json::to_value(catalog.pinned(manifest).internals).map_err(|e| e.to_string())?;
        internals.insert(manifest.id.clone(), pinned);
    }
    Ok(Event::Built {
        route: route.name().into(),
        records: library_record::collect(&libraries)?,
        binary,
        libraries,
        mods: resolution
            .order
            .iter()
            .map(|&index| mods[index].manifest.id.clone())
            .collect(),
        internals,
        mixins: crate::MIXINS_REV.into(),
        used: take_used(),
    })
}

/// The caller's answer to a [`Event::Consent`]: only a line saying `true` agrees.
fn answer(input: &mut dyn BufRead) -> bool {
    let mut line = String::new();
    input.read_line(&mut line).is_ok() && line.trim() == "true"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn plan(mods: &Path) -> Plan {
        Plan {
            contract: CONTRACT,
            work: mods.join("work"),
            toolchain: crate::rust::tests::unpacked(mods.join("rust"), "x86_64-unknown-linux-gnu"),
            package: Package::at(&mods.join("sdk")),
            mods: mods.to_owned(),
            target: TargetName::Client,
            registry: Some(false),
            mixins_from_source: false,
        }
    }

    #[test]
    fn a_plan_is_one_line_of_the_contract_this_tool_speaks() {
        let dir = crate::tests::dir("contract-plan");
        let line = serde_json::to_string(&plan(&dir)).unwrap();
        let read = read_plan(&mut format!("{line}\ntrue\n").as_bytes()).unwrap();
        assert_eq!(read, plan(&dir));
        let mut newer = serde_json::to_value(plan(&dir)).unwrap();
        newer["contract"] = CONTRACT.into();
        newer["profile"] = "debug".into();
        let error = read_plan(&mut newer.to_string().as_bytes()).unwrap_err();
        assert!(
            matches!(&error, PlanError::Invalid(e) if e.contains("unknown field `profile`")),
            "{error}"
        );
        for not_a_plan in ["", "[]\n", "{\"work\": \"w\"}\n"] {
            let error = read_plan(&mut not_a_plan.as_bytes()).unwrap_err();
            assert!(
                matches!(error, PlanError::Invalid(_)),
                "{not_a_plan:?}: {error}"
            );
        }
    }

    #[test]
    fn another_contract_is_refused_for_its_number_before_any_other_field() {
        let error = read_plan(&mut "{\"contract\":1}\n".as_bytes()).unwrap_err();
        assert_eq!(error, PlanError::Contract(1.into()));
        assert!(
            error
                .to_string()
                .starts_with("the plan speaks contract 1; this tool speaks contract 2"),
            "{error}"
        );
        let error = read_plan(&mut "{\"contract\":\"1\",\"later\":true}\n".as_bytes()).unwrap_err();
        assert_eq!(error, PlanError::Contract("1".into()));
    }

    #[test]
    fn events_are_tagged_lines() {
        let mut out = Vec::new();
        emit(
            &mut out,
            &Event::Mod {
                id: "greeter".into(),
                done: 1,
                total: 2,
            },
        )
        .unwrap();
        emit(
            &mut out,
            &Event::Failed {
                error: "no".into(),
                blame: Some(Blame {
                    mod_id: "greeter".into(),
                    kind: BlameKind::Compile,
                }),
                used: vec!["kept".into()],
            },
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "{\"event\":\"mod\",\"id\":\"greeter\",\"done\":1,\"total\":2}\n{\"event\":\"failed\",\"error\":\"no\",\"blame\":{\"mod\":\"greeter\",\"kind\":\"compile\"},\"used\":[\"kept\"]}\n"
        );
    }

    #[test]
    fn a_refused_mod_ends_in_a_failed_event_before_anything_compiles() {
        let dir = crate::tests::dir("contract-refused");
        let project = dir.join("scripted");
        crate::tests::write_bundle(&project, "scripted", "");
        std::fs::write(
            project.join("Cargo.toml"),
            "[package]\nname='scripted'\nversion='1.0.0'\nedition='2024'\n[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\n",
        )
        .unwrap();
        std::fs::write(project.join("build.rs"), "fn main() {}").unwrap();
        let mut out = Vec::new();
        let error = run(&plan(&dir), &mut "".as_bytes(), &mut out).unwrap_err();
        assert!(
            error.starts_with("E_NATIVE_BUILD_SCRIPT: scripted"),
            "{error}"
        );
        let lines: Vec<Event> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            lines,
            [Event::Failed {
                error,
                blame: Some(Blame {
                    mod_id: "scripted".into(),
                    kind: BlameKind::BuildScript,
                }),
                used: Vec::new(),
            }]
        );
    }

    #[test]
    fn a_failure_blames_the_mod_its_error_names() {
        let ids = ["alpha".to_owned(), "beta".to_owned()];
        let compile = format!("{} failed (exit status: 101)", prebuilt::compiling("beta"));
        assert_eq!(
            blame(&compile, &ids),
            Some(Blame {
                mod_id: "beta".into(),
                kind: BlameKind::Compile
            })
        );
        let features = prebuilt::Refusal::Features {
            mod_id: "alpha".into(),
        }
        .to_string();
        assert_eq!(
            blame(&features, &ids).map(|b| b.kind),
            Some(BlameKind::Features)
        );
        assert_eq!(blame("the linker ran out of memory", &ids), None);
    }

    #[test]
    fn only_a_true_line_agrees_to_registry_crates() {
        assert!(answer(&mut "true\n".as_bytes()));
        for no in ["false\n", "yes\n", ""] {
            assert!(!answer(&mut no.as_bytes()), "{no:?}");
        }
    }
}
