//! The launcher's generated executable for trait mods.
use super::{Bundle, internals};
use std::fs;
use std::path::Path;
use sunrise_mod_package::native_route::{API_PACKAGE, INTERNALS_PACKAGE, PACKAGES};

pub fn read(path: &Path) -> Result<toml::Table, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.parse().map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, value: &toml::Table) -> Result<(), String> {
    let text = toml::to_string(value).map_err(|e| format!("{}: {e}", path.display()))?;
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn path_dep(path: &str) -> toml::Table {
    toml::Table::from_iter([("path".into(), path.into())])
}

pub fn template(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).ok()
        .filter(|template| ["/* MOD_ENTRIES */", "/* BUILT_MODS */"].iter()
            .all(|marker| template.matches(marker).count() == 1))
        .ok_or_else(|| crate::host::tr("This game source has no native inventory entry template. Update Sunrise and rebuild the mods against this game.").to_string())
}

pub fn main_rs(template: &str, mods: &[&sunrise_mod_package::Manifest]) -> String {
    // Own the bytes even when the staged SDK template references a static in a dylib.
    let template = template.replace(
        "std::hint::black_box(&sunrise_mod_package::holder::CAPABILITY);",
        "#[used] static HOLDER_CAPABILITY: [u8; sunrise_mod_package::holder::CAPABILITY.len()] = sunrise_mod_package::holder::CAPABILITY; std::hint::black_box(&HOLDER_CAPABILITY);",
    );
    let entries = mods
        .iter()
        .enumerate()
        .map(|(i, m)| format!("        {:?} => mod_{i}::hooks,\n", m.id))
        .collect::<String>();
    let inventory = mods.iter().map(|m| format!(
        "    mod_api::__private::native::BuiltMod {{ id: {:?}, version: {:?}, side: mod_api::Side::{:?} }},\n",
        m.id, m.version.to_string(), m.side,
    )).collect::<String>();
    let markers = [
        ("/* MOD_ENTRIES */", &entries),
        ("/* BUILT_MODS */", &inventory),
    ];
    template
        .split_inclusive("*/")
        .map(|chunk| {
            markers
                .iter()
                .find_map(|(marker, value)| {
                    chunk
                        .strip_suffix(*marker)
                        .map(|prefix| prefix.trim_end_matches([' ', '\t']).to_owned() + value)
                })
                .unwrap_or_else(|| chunk.to_owned())
        })
        .collect::<String>()
}

/// Parsed once from the pipeline's immutable incoming copy, then reused for staging.
pub struct Project<'a> {
    pub bundle: &'a Bundle,
    pub cargo: toml::Table,
    pub package: String,
    api_deps: Vec<ApiDependency>,
}

/// Where a mod's `Cargo.toml` names the SDK package `package`: the copy of its pinned build that
/// `cargo sunrise-mod` fetches into the project (`.sunrise/sdk/`).
pub fn sdk_path(package: &str) -> String {
    format!(".sunrise/sdk/api/crates/{package}")
}

/// Whether `value`, a mod's dependency `key` on the SDK package `package`, names the SDK the one
/// way the launcher takes: by the path to the mod tool's copy of the pinned SDK ([`sdk_path`]).
/// The launcher builds against its own copy of the same build. Every other form is refused under
/// one code, each with its own reason: the git form the path form replaced, a version alone (which
/// Cargo would resolve from crates.io), any other source selector, and a path elsewhere.
pub fn sdk_source(key: &str, package: &str, value: &toml::Value) -> Result<(), String> {
    let path = sdk_path(package);
    let why = if value.get("git").is_some() {
        "the SDK is no longer named by git".to_owned()
    } else if let Some(selector) = [
        "registry",
        "registry-index",
        "branch",
        "tag",
        "rev",
        "workspace",
    ]
    .into_iter()
    .find(|selector| value.get(*selector).is_some())
    {
        format!("{selector} does not name the SDK")
    } else {
        match value.get("path") {
            None => format!(
                "a version names no source, so Cargo would take {package} from crates.io, where anyone can publish that name"
            ),
            Some(written) if written.as_str() != Some(path.as_str()) => {
                format!("the path must be \"{path}\", the pinned SDK the mod tool fetches")
            }
            Some(_) => return Ok(()),
        }
    };
    Err(format!(
        "E_SDK_SOURCE: {key}: {why}; write {key} = {{ path = \"{path}\" }}, then any cargo sunrise-mod command fetches the pinned SDK there"
    ))
}

/// A dependency on one of [`PACKAGES`], under `[dependencies]` or a `[target.*]` table.
struct ApiDependency {
    target: Option<String>,
    key: String,
    package: &'static str,
    library: &'static str,
}

impl ApiDependency {
    fn table_mut<'a>(&self, cargo: &'a mut toml::Table) -> &'a mut toml::Table {
        match &self.target {
            Some(target) => cargo["target"][target.as_str()]["dependencies"]
                .as_table_mut()
                .expect("parsed dependencies"),
            None => cargo["dependencies"]
                .as_table_mut()
                .expect("parsed dependencies"),
        }
    }

    fn value_mut<'a>(&self, cargo: &'a mut toml::Table) -> &'a mut toml::Value {
        self.table_mut(cargo)
            .get_mut(&self.key)
            .expect("parsed dependency")
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ProjectError {
    Cargo { mod_id: String, reason: String },
    PackageName { mod_id: String },
    ApiDependency { mod_id: String },
    GameDependency { mod_id: String, key: String },
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::host::trf;
        let message = match self {
            Self::Cargo { mod_id, reason } => {
                trf("{mod}: {error}", &[("mod", mod_id), ("error", reason)])
            }
            Self::PackageName { mod_id } => {
                trf("{mod}: Cargo.toml has no package name.", &[("mod", mod_id)])
            }
            Self::ApiDependency { mod_id } => trf(
                "{mod}: Cargo.toml has no sunrise-mod-api dependency.",
                &[("mod", mod_id)],
            ),
            Self::GameDependency { mod_id, key } => trf(
                "E_GAME_DEPENDENCY: {mod}: Cargo.toml depends on the game ({key}). Declare game internals through sunrise-internals instead.",
                &[("mod", mod_id), ("key", key)],
            ),
        };
        f.write_str(&message)
    }
}
impl std::error::Error for ProjectError {}

impl<'a> Project<'a> {
    /// The mod's crate name: its `[lib] name`, or its package name with `_` for `-`.
    pub fn crate_name(&self) -> String {
        self.cargo
            .get("lib")
            .and_then(|l| l.get("name"))
            .and_then(toml::Value::as_str)
            .map_or_else(|| self.package.replace('-', "_"), str::to_owned)
    }

    pub fn read(bundle: &'a Bundle) -> Result<Self, ProjectError> {
        let path = bundle.dir.join("Cargo.toml");
        let text = fs::read_to_string(&path).map_err(|e| ProjectError::Cargo {
            mod_id: bundle.manifest.id.clone(),
            reason: format!("{}: {e}", path.display()),
        })?;
        Self::parse(bundle, &text)
    }

    /// Parses the staged Cargo manifest, or one read by a caller with its own input bounds.
    pub fn parse(bundle: &'a Bundle, text: &str) -> Result<Self, ProjectError> {
        let mod_id = || bundle.manifest.id.clone();
        let cargo: toml::Table = text.parse().map_err(|e| ProjectError::Cargo {
            mod_id: mod_id(),
            reason: format!("{}: {e}", bundle.dir.join("Cargo.toml").display()),
        })?;
        for section in ["patch", "replace"] {
            if cargo.contains_key(section) {
                return Err(ProjectError::Cargo {
                    mod_id: mod_id(),
                    reason: format!("[{section}] is not allowed in a native mod"),
                });
            }
        }
        let package = cargo
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| ProjectError::PackageName { mod_id: mod_id() })?
            .to_owned();
        let cargo_error = |reason| ProjectError::Cargo {
            mod_id: mod_id(),
            reason,
        };
        let targets = cargo
            .get("target")
            .map(|targets| {
                targets
                    .as_table()
                    .ok_or_else(|| cargo_error("[target] must be a table".into()))
            })
            .transpose()?;
        let configs = std::iter::once((None, Ok(&cargo))).chain(targets.into_iter().flatten().map(
            |(target, config)| {
                let table = config
                    .as_table()
                    .ok_or_else(|| cargo_error(format!("[target.{target}] must be a table")));
                (Some(target.as_str()), table)
            },
        ));
        let mut api_deps = Vec::new();
        for (target, table) in configs {
            let table = table?;
            for kind in [
                "dependencies",
                "dev-dependencies",
                "build-dependencies",
                "dev_dependencies",
                "build_dependencies",
            ] {
                let Some(dependencies) = table.get(kind) else {
                    continue;
                };
                let dependencies = dependencies
                    .as_table()
                    .ok_or_else(|| cargo_error(format!("[{kind}] must be a table")))?;
                for (key, value) in dependencies {
                    if !(value.is_str() || value.is_table()) {
                        return Err(cargo_error(format!(
                            "{key} in [{kind}] must be a version string or a table"
                        )));
                    }
                    let package = match value.get("package") {
                        Some(package) => package.as_str().ok_or_else(|| {
                            cargo_error(format!("{key} in [{kind}]: package must be a string"))
                        })?,
                        None => key,
                    };
                    if let Some(&(package, library)) =
                        PACKAGES.iter().find(|(sdk, _)| *sdk == package)
                    {
                        sdk_source(key, package, value).map_err(cargo_error)?;
                        if kind == "dependencies" {
                            api_deps.push(ApiDependency {
                                target: target.map(str::to_owned),
                                key: key.clone(),
                                package,
                                library,
                            });
                        }
                    } else {
                        for selector in [
                            "git",
                            "registry",
                            "registry-index",
                            "branch",
                            "rev",
                            "tag",
                            "workspace",
                        ] {
                            if value.get(selector).is_some() {
                                return Err(ProjectError::Cargo {
                                    mod_id: mod_id(),
                                    reason: format!(
                                        "{key}: {selector} source is refused; use crates.io with Cargo.lock"
                                    ),
                                });
                            }
                        }
                    }
                    if package == "sunrise-client" {
                        return Err(ProjectError::GameDependency {
                            mod_id: mod_id(),
                            key: key.clone(),
                        });
                    }
                }
            }
        }
        if !api_deps
            .iter()
            .any(|api| api.target.is_none() && api.package == API_PACKAGE)
        {
            return Err(ProjectError::ApiDependency { mod_id: mod_id() });
        }
        Ok(Self {
            bundle,
            cargo,
            package,
            api_deps,
        })
    }

    /// Whether the mod links its game internals facade, under any dependency alias.
    pub fn uses_internals(&self) -> bool {
        self.api_deps
            .iter()
            .any(|api| api.package == INTERNALS_PACKAGE)
    }

    /// The names the mod's code calls the game's packages by (`mod_api` or a
    /// renamed dependency's key), each with its package's library name.
    pub fn api_dependencies(&self) -> std::collections::BTreeSet<(String, &'static str)> {
        self.api_deps
            .iter()
            .map(|api| {
                let name = if api.key == api.package {
                    api.library.to_owned()
                } else {
                    api.key.replace('-', "_")
                };
                (name, api.library)
            })
            .collect()
    }
}

/// Add the mod libraries and their generated internals facades to the discovery workspace.
/// Discovery checks the library targets, so it does not stage a game executable.
pub fn stage(
    stage: &Path,
    projects: &[Project<'_>],
    declared: &[Vec<super::dependencies::Dependency>],
    resolution: &super::dependencies::Resolution,
    catalog: &internals::Catalog,
) -> Result<bool, String> {
    let mods: Vec<_> = resolution
        .order
        .iter()
        .map(|&i| (i, &projects[i]))
        .collect();
    if mods.is_empty() {
        return Ok(false);
    }
    let facades = catalog
        .prepare(mods.iter().map(|(_, project)| &project.bundle.manifest))
        .map_err(|e| e.to_string())?;
    let packages: std::collections::HashMap<_, _> = mods
        .iter()
        .map(|(_, project)| (project.bundle.manifest.id.as_str(), project.package.clone()))
        .collect();
    let mut members = Vec::new();
    for (index, project) in mods {
        let bundle = project.bundle;
        let relative = format!("native-api/{}", bundle.manifest.id);
        let dest = stage.join(&relative);
        super::copy_bundle(bundle, &dest)?;

        let path = dest.join("Cargo.toml");
        let mut manifest = project.cargo.clone();
        for api in &project.api_deps {
            let dependency = api.value_mut(&mut manifest);
            let mut rewritten = path_dep(&if api.package == INTERNALS_PACKAGE {
                format!("../../{}", internals::relative(&bundle.manifest.id))
            } else {
                format!("../../crates/{}", api.package)
            });
            if let Some(old) = dependency.as_table() {
                // A renamed dependency also needs its package name.
                for key in ["package", "features", "default-features"] {
                    if let Some(value) = old.get(key) {
                        rewritten.insert(key.into(), value.clone());
                    }
                }
            }
            if api.package == INTERNALS_PACKAGE {
                rewritten.insert(
                    "package".into(),
                    internals::package(&bundle.manifest.id).into(),
                );
            }
            *dependency = toml::Value::Table(rewritten);
            // Cargo treats a key differing from `package` as the crate's alias. Preserve the
            // facade's library name when giving its per-mod package a unique name.
            if api.package == INTERNALS_PACKAGE && api.key == INTERNALS_PACKAGE {
                let table = api.table_mut(&mut manifest);
                let value = table.remove(&api.key).expect("rewritten dependency");
                table.insert("internals".into(), value);
            }
        }
        let targets: Vec<_> = declared[index]
            .iter()
            .map(|dependency| super::dependencies::Package {
                dependency,
                package: packages.get(dependency.id.as_str()).map(String::as_str),
            })
            .collect();
        super::dependencies::rewrite(&bundle.manifest.id, &mut manifest, &targets)
            .map_err(|e| e.to_string())?;
        write(&path, &manifest)?;
        if internals::needed(&bundle.manifest) || project.uses_internals() {
            internals::stage(stage, project, &facades[bundle.manifest.id.as_str()])?;
            members.push(internals::relative(&bundle.manifest.id));
        }
        members.push(relative);
    }
    let root = stage.join("Cargo.toml");
    let mut workspace = read(&root)?;
    let list = workspace
        .get_mut("workspace")
        .and_then(|w| w.get_mut("members"))
        .and_then(toml::Value::as_array_mut)
        .ok_or("workspace has no members")?;
    list.extend(members.into_iter().map(toml::Value::String));
    write(&root, &workspace)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        let name = std::thread::current().name().unwrap().replace(':', "-");
        let dir = super::super::tests::dir(&name);
        super::super::tests::write_bundle(&dir, "example", "");
        fs::write(dir.join("Cargo.toml"), "[package]\nname='example-mod'\nversion='9.0.0'\nedition='2024'\n[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\n").unwrap();
        dir
    }

    #[test]
    #[ignore = "daily: starts rustc"]
    fn wrapper_owns_holder_marker() {
        let dir = fixture();
        let source = dir.join("main.rs");
        let output = dir.join(std::env::consts::EXE_SUFFIX.to_owned() + "wrapper");
        let library_source = dir.join("library.rs");
        fs::write(&library_source, "pub mod holder { pub static CAPABILITY: [u8; 26] = *b\"sunrise-private-holder-v2;\"; }").unwrap();
        let library = dir.join(format!(
            "{}sunrise_mod_package{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ));
        assert!(
            std::process::Command::new("rustc")
                .args([
                    "--crate-type=dylib",
                    "--crate-name=sunrise_mod_package",
                    "-Cprefer-dynamic"
                ])
                .arg(&library_source)
                .arg("-o")
                .arg(&library)
                .status()
                .unwrap()
                .success()
        );
        let template =
            "fn main() { std::hint::black_box(&sunrise_mod_package::holder::CAPABILITY); }";
        fs::write(&source, main_rs(template, &[])).unwrap();
        assert!(
            std::process::Command::new("rustc")
                .args(["--edition=2021", "-O", "-Cprefer-dynamic", "--extern"])
                .arg(format!("sunrise_mod_package={}", library.display()))
                .arg(&source)
                .arg("-o")
                .arg(&output)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            fs::read(&output)
                .unwrap()
                .windows(26)
                .any(|w| w == b"sunrise-private-holder-v2;")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn projects_refuse_a_game_dependency_and_need_the_mod_api() {
        let dir = fixture();
        for (dependencies, game) in [
            (
                "[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\nsunrise-client='0.1'",
                Some("sunrise-client"),
            ),
            (
                "[dependencies.api]\npackage='sunrise-mod-api'\npath='.sunrise/sdk/api/crates/sunrise-mod-api'\n[target.'cfg(unix)'.dependencies.game]\npackage='sunrise-client'\nversion='0.1'",
                Some("game"),
            ),
            ("[dependencies]\nserde='1'", None),
            (
                "[target.'cfg(unix)'.dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}",
                None,
            ),
        ] {
            let text = format!("[package]\nname='example-mod'\nversion='1.0.0'\n{dependencies}\n");
            fs::write(dir.join("Cargo.toml"), text).unwrap();
            let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
            let error = Project::read(&bundle).err().unwrap();
            match game {
                Some(game) => assert!(
                    matches!(&error, ProjectError::GameDependency { mod_id, key } if mod_id == "example" && key == game),
                    "{error}"
                ),
                None => assert!(
                    matches!(&error, ProjectError::ApiDependency { .. }),
                    "{error}"
                ),
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_sdk_is_taken_only_by_its_path_form_and_every_other_form_is_refused_by_name() {
        let dir = fixture();
        for (form, refusal) in [
            ("{ path = '.sunrise/sdk/api/crates/sunrise-mod-api' }", None),
            (
                "{ path = '.sunrise/sdk/api/crates/sunrise-mod-api', features = ['testing'] }",
                None,
            ),
            (
                "{ git = 'https://github.com/arismoko/horizon', rev = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' }",
                Some(
                    "example: E_SDK_SOURCE: sunrise-mod-api: the SDK is no longer named by git; write sunrise-mod-api = { path = \".sunrise/sdk/api/crates/sunrise-mod-api\" }",
                ),
            ),
            (
                "'0.1'",
                Some(
                    "example: E_SDK_SOURCE: sunrise-mod-api: a version names no source, so Cargo would take sunrise-mod-api from crates.io",
                ),
            ),
            (
                "{ version = '0.1', features = ['testing'] }",
                Some("example: E_SDK_SOURCE: sunrise-mod-api: a version names no source"),
            ),
            (
                "{ workspace = true }",
                Some("example: E_SDK_SOURCE: sunrise-mod-api: workspace does not name the SDK"),
            ),
            (
                "{ path = '../sunrise/crates/sunrise-mod-api' }",
                Some(
                    "example: E_SDK_SOURCE: sunrise-mod-api: the path must be \".sunrise/sdk/api/crates/sunrise-mod-api\"",
                ),
            ),
        ] {
            let text = format!(
                "[package]\nname='example-mod'\nversion='1.0.0'\n[dependencies]\nsunrise-mod-api = {form}\n"
            );
            fs::write(dir.join("Cargo.toml"), text).unwrap();
            let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
            match (Project::read(&bundle), refusal) {
                (Ok(_), None) => {}
                (Err(error), Some(refusal)) => {
                    assert!(error.to_string().starts_with(refusal), "{error}")
                }
                (result, _) => panic!("{form}: {:?}", result.err()),
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generates_only_tool_contract_tables_without_mutating_package() {
        let dir = fixture();
        let path = dir.join(sunrise_mod_package::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap()
            + "[[native.module]]\nid='test'\nfile='crates/sunrise-client/src/game.rs'\nname='support'\nsource='src/lib.rs'\n";
        fs::write(&path, &text).unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        assert!(bundle.needs_mixins());
        super::super::package::tool_manifest(&bundle).unwrap();
        let value = read(&dir.join(super::super::MANIFEST)).unwrap();
        assert!(!value.contains_key("api"));
        assert!(!value.contains_key("native"));
        assert!(value.contains_key("module"));
        assert!(!value.contains_key("injection"));
        assert_eq!(value["mod"]["id"].as_str(), Some("example"));
        assert_eq!(fs::read_to_string(path).unwrap(), text);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generated_main_keeps_mod_order() {
        let template =
            include_str!("../../../crates/sunrise-client/src/native_mod_entry.rs.template");
        let dir = fixture();
        let mut base = sunrise_mod_package::Package::read(&dir).unwrap().manifest;
        base.id = "base".into();
        base.version = semver::Version::new(1, 2, 3);
        base.side = mod_vocab::Side::Both;
        let mut extra = base.clone();
        extra.id = "extra".into();
        extra.version = semver::Version::new(2, 0, 0);
        extra.side = mod_vocab::Side::Client;
        let mods = [&base, &extra];
        for count in 0..=2 {
            let text = main_rs(template, &mods[..count]);
            let padded = ["/* MOD_ENTRIES */", "/* BUILT_MODS */"]
                .into_iter()
                .fold(template.to_owned(), |text, marker| {
                    text.replace(marker, &format!("    {marker}"))
                });
            assert_eq!(main_rs(&padded, &mods[..count]), text);
            let facts: Vec<_> = text
                .lines()
                .filter(|line| line.contains(" => mod_"))
                .collect();
            assert_eq!(
                facts,
                &[
                    "        \"base\" => mod_0::hooks,",
                    "        \"extra\" => mod_1::hooks,",
                ][..count]
            );
            let inventory: Vec<_> = text
                .lines()
                .filter(|line| line.contains("BuiltMod {"))
                .collect();
            assert_eq!(
                inventory,
                &[
                    "    mod_api::__private::native::BuiltMod { id: \"base\", version: \"1.2.3\", side: mod_api::Side::Both },",
                    "    mod_api::__private::native::BuiltMod { id: \"extra\", version: \"2.0.0\", side: mod_api::Side::Client },",
                ][..count]
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn discovery_stages_a_library_and_its_facade_without_a_game_entry_template() {
        let dir = fixture();
        let workspace = tempfile::tempdir_in(dir.parent().unwrap()).unwrap();
        let stage_dir = workspace.path();
        fs::write(stage_dir.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        let cargo = dir.join("Cargo.toml");
        fs::write(
            &cargo,
            format!(
                "{}sunrise-internals={{path='.sunrise/sdk/api/crates/sunrise-internals'}}\n",
                fs::read_to_string(&cargo).unwrap()
            ),
        )
        .unwrap();
        let manifest = dir.join(sunrise_mod_package::manifest::MANIFEST);
        let source = fs::read_to_string(&manifest).unwrap();
        fs::write(
            &manifest,
            format!("{source}\n[dependencies]\ncontent='^1'\n"),
        )
        .unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        let project = Project::read(&bundle).unwrap();
        let catalog = internals::Catalog {
            discovery: true,
            ..Default::default()
        };
        let side = sunrise_mod_package::manifest::Side::Both;
        let declared =
            super::super::dependencies::build_dependencies(&bundle.manifest, side, |_| false);
        let resolution =
            super::super::dependencies::resolve_links(&[super::super::dependencies::Mod {
                id: &bundle.manifest.id,
                version: &bundle.manifest.version,
                has_api: true,
                code: true,
                dependencies: &declared,
            }])
            .unwrap();
        assert!(stage(stage_dir, &[project], &[declared], &resolution, &catalog).unwrap());
        let root = read(&stage_dir.join("Cargo.toml")).unwrap();
        assert_eq!(
            root["workspace"]["members"].as_array().unwrap(),
            &[
                "native-internals/example".into(),
                "native-api/example".into()
            ]
        );
        let lib = read(&stage_dir.join("native-api/example/Cargo.toml")).unwrap();
        assert!(lib["dependencies"].get("content").is_none());
        assert_eq!(
            lib["dependencies"]["internals"]["path"].as_str(),
            Some("../../native-internals/example")
        );
        assert!(
            stage_dir
                .join("native-internals/example/src/lib.rs")
                .is_file()
        );
        assert!(!stage_dir.join("crates/native-game").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    /// Project-local Cargo and toolchain files are left out of the staged copy, not refused.
    #[test]
    fn local_cargo_and_toolchain_files_stay_out_of_the_staged_mod() {
        let dir = fixture();
        fs::create_dir_all(dir.join(".cargo")).unwrap();
        fs::write(
            dir.join(".cargo/config.toml"),
            "[source.crates-io]\nreplace-with='local'\n",
        )
        .unwrap();
        fs::write(
            dir.join("rust-toolchain.toml"),
            "[toolchain]\nchannel='nightly'\n",
        )
        .unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        Project::read(&bundle).unwrap();
        let staged = tempfile::tempdir().unwrap();
        super::super::copy_bundle(&bundle, staged.path()).unwrap();
        assert!(staged.path().join("src/lib.rs").is_file());
        assert!(!staged.path().join(".cargo").exists());
        assert!(!staged.path().join("rust-toolchain.toml").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn api_only_skips_mixins() {
        let dir = fixture();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        assert!(!bundle.needs_mixins());
        fs::remove_dir_all(dir).unwrap();
    }
}
