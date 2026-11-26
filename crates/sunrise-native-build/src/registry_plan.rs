//! One dependency plan shared by source staging and immutable prebuilt SDK builds.
use super::{api::Project, crates::Wanted, registry_lock::RegistryLock, registry_resolve};
use crate::host::trf;
use std::{collections::BTreeSet, path::Path};

pub struct Plan {
    pub game: RegistryLock,
    seed: RegistryLock,
    allowed: RegistryLock,
}

impl Plan {
    pub fn read(game: &str, projects: &[Project<'_>], wanted: &[Wanted]) -> Result<Self, String> {
        let game = RegistryLock::parse(game)?;
        let mut mods = RegistryLock::default();
        for project in projects {
            let bundle = project.bundle;
            let requested: Vec<_> = wanted
                .iter()
                .filter(|w| w.mod_id == bundle.manifest.id)
                .collect();
            if requested.is_empty() {
                continue;
            }
            let pins = super::read_lock(&bundle.dir)
                .and_then(|text| RegistryLock::parse(&text))
                .map_err(|error| format!("{}: {error}", bundle.manifest.id))?;
            for request in requested {
                if !pins
                    .packages
                    .keys()
                    .any(|id| id.name == request.name && request.requirement.matches(&id.version))
                {
                    return Err(format!(
                        "{mod_id}: Cargo.lock does not pin {} {}, which its Cargo.toml asks for. Run native-lock with update on {mod_id}.",
                        request.name,
                        request.version,
                        mod_id = request.mod_id
                    ));
                }
            }
            mods.extend(&pins)
                .map_err(|error| format!("{}: {error}", bundle.manifest.id))?;
        }
        let mut seed = game.clone();
        seed.extend(&mods)?;
        // Cargo prefers a compatible pin already in the lock. Leave it only the game's record
        // in shared slots, including the game's dependencies and checksums for those records.
        for (_, id) in game.moved_pins(&mods) {
            seed.packages.remove(id);
        }
        Ok(Self {
            game,
            allowed: seed.clone(),
            seed,
        })
    }

    /// Patches come only from the validated SDK identity, never from a mod manifest.
    pub fn authorize_game_patches(&mut self, patches: &toml::Table) {
        let names = patches
            .values()
            .flat_map(|registry| {
                registry
                    .as_table()
                    .expect("validated SDK patch registry")
                    .keys()
                    .cloned()
            })
            .collect();
        self.allowed.withhold_sdk_patched(&self.game, &names);
    }

    pub(super) fn check_resolved(&self, resolved: &RegistryLock) -> Result<(), String> {
        registry_resolve::check_game_pins(&self.game, resolved)?;
        self.allowed.check_pins(resolved)
    }

    pub fn fetch_with_patches(
        &self,
        rust: &dyn crate::rust::Toolchain,
        directory: &Path,
        patches: &toml::Table,
    ) -> Result<(), String> {
        // Resolve the validated mod workspace with its actual aliases, features
        // and defaults, rather than reconstructing package-only fetch roots.
        apply_patches(directory, patches)?;
        self.resolve_and_fetch(directory, |subcommand| {
            super::cargo(rust, directory, subcommand)
        })
    }

    fn resolve_and_fetch(
        &self,
        directory: &Path,
        mut cargo: impl FnMut(&str) -> std::process::Command,
    ) -> Result<(), String> {
        registry_resolve::resolve(
            directory,
            &self.game,
            &self.seed,
            Some(&self.allowed),
            &mut cargo,
        )
        .map_err(|error| {
            // Cargo names the requiring package and its local path, not its Sunrise mod ID.
            // Map only package paths in its dependency report, never the mods' lock contents.
            let mods = directory.join(super::crates::DIR);
            let canonical = mods.canonicalize().ok();
            let ids: BTreeSet<_> = error
                .lines()
                .filter(|line| line.contains(" package `"))
                .filter_map(|line| {
                    let (_, path) = line.trim_end().strip_suffix(")`")?.split_once(" (")?;
                    let path = Path::new(path);
                    let relative = path
                        .strip_prefix(&mods)
                        .ok()
                        .or_else(|| path.strip_prefix(canonical.as_ref()?).ok())?;
                    relative.components().next()?.as_os_str().to_str()
                })
                .collect();
            if ids.is_empty() {
                error
            } else {
                let mods = ids.into_iter().collect::<Vec<_>>().join(", ");
                let sources = trf(
                    "Mods in Cargo's dependency report: {mods}.",
                    &[("mods", &mods)],
                );
                format!("{sources}\n\n{error}")
            }
        })?;
        let mut fetch = cargo("fetch");
        fetch.arg("--locked");
        super::run(fetch, "Downloading the mods' pinned crates")
    }
}

/// Only the SDK identity supplies patches; mod manifests cannot inject overrides.
pub fn apply_patches(directory: &Path, patches: &toml::Table) -> Result<(), String> {
    let path = directory.join("Cargo.toml");
    let mut manifest: toml::Table = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|e| format!("Cargo.toml: {e}"))?;
    manifest.insert("patch".into(), patches.clone().into());
    std::fs::write(path, manifest.to_string()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn production_plan_preserves_isolated_game_patch_identity() {
        let original = "version=4\n[[package]]\nname='allocator-sys'\nversion='0.1.49'\n";
        let plan = Plan::read(original, &[], &[]).unwrap();
        plan.allowed
            .check_pins(&RegistryLock::parse(original).unwrap())
            .unwrap();
        let moved = RegistryLock::parse(&original.replace("0.1.49", "0.1.50")).unwrap();
        let error = plan.allowed.check_pins(&moved).unwrap_err();
        assert!(error.contains("allocator-sys"), "{error}");
    }

    #[test]
    #[ignore = "daily"]
    fn cargo_resolves_registry_parent_through_sdk_local_patch_without_compiling() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for path in [
            "vendor/allocator/src",
            "sdk/allocator-sys/src",
            "src",
            ".cargo",
        ] {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        for (path, text) in [
            (
                "Cargo.toml",
                "[package]\nname='fixture'\nversion='1.0.0'\n[workspace]\n[dependencies]\nallocator='1'\n",
            ),
            ("src/lib.rs", ""),
            (
                "vendor/allocator/Cargo.toml",
                "[package]\nname='allocator'\nversion='1.0.0'\n[dependencies]\nallocator-sys='0.1'\n",
            ),
            ("vendor/allocator/src/lib.rs", ""),
            (
                "sdk/allocator-sys/Cargo.toml",
                "[package]\nname='allocator-sys'\nversion='0.1.49'\n",
            ),
            ("sdk/allocator-sys/src/lib.rs", ""),
            (
                ".cargo/config.toml",
                "[source.crates-io]\nreplace-with='fixture'\n[source.fixture]\ndirectory='vendor'\n",
            ),
        ] {
            std::fs::write(root.join(path), text).unwrap();
        }
        std::fs::write(
            root.join("vendor/allocator/.cargo-checksum.json"),
            format!(r#"{{"package":"{}","files":{{}}}}"#, "a".repeat(64)),
        )
        .unwrap();
        let patches: toml::Table = format!(
            "[crates-io]\nallocator-sys={{path={:?}}}\n",
            root.join("sdk/allocator-sys").to_string_lossy()
        )
        .parse()
        .unwrap();
        apply_patches(root, &patches).unwrap();
        let output = std::process::Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        let game = format!(
            "version=4\n[[package]]\nname='allocator'\nversion='1.0.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='{}'\ndependencies=['allocator-sys']\n[[package]]\nname='allocator-sys'\nversion='0.1.49'\n",
            "a".repeat(64)
        );
        RegistryLock::parse(&game)
            .unwrap()
            .check_pins(&RegistryLock::parse(&text).unwrap())
            .unwrap();
        assert!(!root.join("target").exists());
    }

    #[test]
    #[ignore = "daily"]
    fn a_mods_registry_package_builds_on_the_sdks_naga_and_never_on_the_registrys() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for path in ["vendor/mod-parent/src", "sdk/naga/src", "src", ".cargo"] {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        for (path, text) in [
            (
                "Cargo.toml",
                "[package]\nname='fixture'\nversion='1.0.0'\n[workspace]\n[dependencies]\nmod-parent='1'\n",
            ),
            ("src/lib.rs", ""),
            (
                "vendor/mod-parent/Cargo.toml",
                "[package]\nname='mod-parent'\nversion='1.0.0'\n[dependencies]\nnaga='30'\n",
            ),
            ("vendor/mod-parent/src/lib.rs", ""),
            (
                "sdk/naga/Cargo.toml",
                "[package]\nname='naga'\nversion='30.0.1'\n",
            ),
            ("sdk/naga/src/lib.rs", ""),
            (
                ".cargo/config.toml",
                "[source.crates-io]\nreplace-with='fixture'\n[source.fixture]\ndirectory='vendor'\n",
            ),
        ] {
            std::fs::write(root.join(path), text).unwrap();
        }
        std::fs::write(
            root.join("vendor/mod-parent/.cargo-checksum.json"),
            format!(r#"{{"package":"{}","files":{{}}}}"#, "a".repeat(64)),
        )
        .unwrap();
        let patches: toml::Table = format!(
            "[crates-io]\nnaga={{path={:?}}}\n",
            root.join("sdk/naga").to_string_lossy()
        )
        .parse()
        .unwrap();
        // First obtain the mod's independent registry lock, without SDK patches.
        std::fs::create_dir_all(root.join("vendor/naga/src")).unwrap();
        std::fs::write(
            root.join("vendor/naga/Cargo.toml"),
            "[package]\nname='naga'\nversion='30.0.1'\n",
        )
        .unwrap();
        std::fs::write(root.join("vendor/naga/src/lib.rs"), "").unwrap();
        std::fs::write(
            root.join("vendor/naga/.cargo-checksum.json"),
            format!(r#"{{"package":"{}","files":{{}}}}"#, "b".repeat(64)),
        )
        .unwrap();
        let original = std::process::Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            original.status.success(),
            "{}",
            String::from_utf8_lossy(&original.stderr)
        );
        let mod_lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        let pins = RegistryLock::parse(&mod_lock).unwrap();
        apply_patches(root, &patches).unwrap();
        let output = std::process::Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        let resolved = RegistryLock::parse(&text).unwrap();
        let game_lock = "version=4\n[[package]]\nname='naga'\nversion='30.0.1'\n";
        let mut plan = Plan::read(game_lock, &[], &[]).unwrap();
        plan.allowed.extend(&pins).unwrap();
        plan.authorize_game_patches(&patches);
        plan.allowed.check_pins(&resolved).unwrap();
        // The SDK's package is used in the SDK's form only: neither the registry's copy of it,
        // which the mod's own lock pins, nor another version in its place.
        for bad in [&mod_lock, &text.replace("30.0.1", "30.0.2")] {
            let error = plan
                .allowed
                .check_pins(&RegistryLock::parse(bad).unwrap())
                .unwrap_err();
            assert!(error.contains("the SDK supplies naga 30.0.1"), "{error}");
        }
        assert!(!root.join("target").exists());
    }

    #[test]
    #[ignore = "daily"]
    fn fetch_retains_actual_feature_closure_and_excludes_unreachable_pins() {
        const ACTOR: &str = "SUNRISE_REGISTRY_FETCH_TEST_ACTOR";
        if std::env::var_os(ACTOR).is_none() {
            let home = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "registry_plan::tests::fetch_retains_actual_feature_closure_and_excludes_unreachable_pins", "--include-ignored", "--nocapture"])
                .env(ACTOR, "1")
                .env("SUNRISE_LAUNCHER_HOME", home.path())
                .output().unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{stdout}\n{stderr}");
            assert!(
                stdout.contains("test result: ok. 1 passed;"),
                "exact-name child did not run one test:\n{stdout}\n{stderr}"
            );
            return;
        }
        assert!(
            !super::super::log_path().exists(),
            "the isolated actor must start with a fresh home"
        );
        // Production build entries call logged()/clear_log(); native-lock also
        // creates root() before resolution. This direct unit call bypasses them.
        super::super::clear_log().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["feature-parent", "feature-only"] {
            let folder = root.join("vendor").join(name);
            std::fs::create_dir_all(folder.join("src")).unwrap();
            let extra = if name == "feature-parent" {
                "[dependencies]\nfeature-only={version='1',optional=true}\n[features]\ndefault=['feature-only']\n"
            } else {
                ""
            };
            std::fs::write(
                folder.join("Cargo.toml"),
                format!("[package]\nname='{name}'\nversion='1.0.0'\n{extra}"),
            )
            .unwrap();
            std::fs::write(folder.join("src/lib.rs"), "").unwrap();
            std::fs::write(
                folder.join(".cargo-checksum.json"),
                format!(r#"{{"package":"{}","files":{{}}}}"#, "a".repeat(64)),
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join(".cargo")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(
            root.join(".cargo/config.toml"),
            "[source.crates-io]\nreplace-with='fixture'\n[source.fixture]\ndirectory='vendor'\n",
        )
        .unwrap();
        // Alias, explicit feature and default-feature semantics are left intact.
        let manifest = "[package]\nname='fixture'\nversion='1.0.0'\n[workspace]\n[dependencies]\nalias={package='feature-parent',version='1',default-features=false,features=['feature-only']}\n";
        std::fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let cargo = |subcommand: &str| {
            let mut command = std::process::Command::new("cargo");
            command
                .arg(subcommand)
                .arg("--offline")
                .current_dir(root)
                .env("CARGO_HOME", root.join("cold-cargo-home"));
            command
        };
        let generated = cargo("generate-lockfile").output().unwrap();
        assert!(
            generated.status.success(),
            "{}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let game = format!(
            "{}\n[[package]]\nname='outside-closure'\nversion='1.0.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='{}'\n",
            std::fs::read_to_string(root.join("Cargo.lock")).unwrap(),
            "b".repeat(64)
        );
        let plan = Plan::read(&game, &[], &[]).unwrap();
        plan.resolve_and_fetch(root, cargo).unwrap();
        let lock = RegistryLock::parse(&std::fs::read_to_string(root.join("Cargo.lock")).unwrap())
            .unwrap();
        assert!(lock.packages.keys().any(|id| id.name == "feature-only"));
        assert!(!lock.packages.keys().any(|id| id.name == "outside-closure"));
        // No source for outside-closure exists: eagerly fetching the union would fail.
        assert!(!root.join("vendor/outside-closure").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("Cargo.toml")).unwrap(),
            manifest
        );
        let metadata = cargo("metadata")
            .args(["--locked", "--format-version", "1"])
            .output()
            .unwrap();
        assert!(
            metadata.status.success(),
            "{}",
            String::from_utf8_lossy(&metadata.stderr)
        );
        assert!(!root.join("target").exists());
    }

    fn shared_registry_fixture(
        ahead_requirement: &str,
        build: bool,
        transitive: bool,
    ) -> Result<RegistryLock, String> {
        use std::{fs, process::Command};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(super::super::root()).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nresolver='3'\nmembers=['native-api/loose_mod','native-api/ahead_mod']\n",
        )
        .unwrap();
        let package = |name: &str, version: &str, digit: &str| {
            format!(
                "[[package]]\nname='{name}'\nversion='{version}'\nsource='{}'\nchecksum='{}'\n",
                sunrise_addresses::CARGO_CRATES_IO_SOURCE,
                digit.repeat(64)
            )
        };
        let vendor_crate = |name: &str, version: &str, digit: &str, extra: &str| {
            let vendor = root.join("vendor").join(format!("{name}-{version}"));
            fs::create_dir_all(vendor.join("src")).unwrap();
            fs::write(vendor.join("src/lib.rs"), "").unwrap();
            fs::write(
                vendor.join("Cargo.toml"),
                format!("[package]\nname='{name}'\nversion='{version}'\nedition='2024'\n{extra}"),
            )
            .unwrap();
            fs::write(
                vendor.join(".cargo-checksum.json"),
                format!(r#"{{"package":"{}","files":{{}}}}"#, digit.repeat(64)),
            )
            .unwrap();
        };
        for (version, digit) in [("3.0.6", "a"), ("3.0.10", "b")] {
            vendor_crate("syn", version, digit, "");
        }
        if transitive {
            // A later registry release is compatible with the game but neither mod pins it.
            vendor_crate("parent", "1.0.2", "c", "[dependencies]\nsyn='3'\n");
        }
        let game = format!("version=4\n{}", package("syn", "3.0.6", "a"));
        let mut bundles = Vec::new();
        let mut wanted = Vec::new();
        for (id, requirement, version) in [
            (
                "loose_mod",
                "^3.0",
                if transitive { "3.0.6" } else { "3.0.10" },
            ),
            ("ahead_mod", ahead_requirement, "3.0.10"),
        ] {
            let path = root.join(crate::crates::DIR).join(id);
            let crate_name = if id == "loose_mod" {
                "loose_crate"
            } else {
                "ahead_crate"
            };
            let digit = if version == "3.0.6" { "a" } else { "b" };
            super::super::tests::write_bundle(&path, id, "");
            let (dependency, requirement) = if transitive {
                ("parent", "1")
            } else {
                ("syn", requirement)
            };
            let manifest = format!(
                "[package]\nname='{crate_name}'\nversion='1.0.0'\nedition='2024'\n[dependencies]\n{dependency}='{requirement}'\n"
            );
            fs::write(
                path.join("Cargo.toml"),
                format!(
                    "{manifest}sunrise-mod-api={{path='.sunrise/sdk/api/crates/sunrise-mod-api'}}\n"
                ),
            )
            .unwrap();
            let mut lock = format!("version=4\n{}", package("syn", version, digit));
            if transitive {
                // With two seeded parents Cargo prefers the first pin, even though its real
                // dependency would move the game. The other mod's parent is also admissible.
                let (parent, requirement) = if id == "loose_mod" {
                    ("1.0.1", "3")
                } else {
                    ("1.0.0", "=3.0.10")
                };
                vendor_crate(
                    "parent",
                    parent,
                    digit,
                    &format!("[dependencies]\nsyn='{requirement}'\n"),
                );
                lock.push_str(&package("parent", parent, digit));
                lock.push_str("dependencies=['syn']\n");
            }
            fs::write(path.join("Cargo.lock"), lock).unwrap();
            bundles.push((super::super::bundle_in(&path).unwrap().unwrap(), manifest));
            wanted.push(Wanted {
                mod_id: id.into(),
                name: dependency.into(),
                version: requirement.into(),
                requirement: semver::VersionReq::parse(requirement).unwrap(),
            });
        }
        let projects: Vec<_> = bundles
            .iter()
            .map(|(bundle, manifest)| {
                let project = Project::read(bundle).unwrap();
                // Staging removes the SDK dependency before Cargo resolves registry packages.
                fs::write(bundle.dir.join("Cargo.toml"), manifest).unwrap();
                project
            })
            .collect();
        let cargo = |subcommand: &str| {
            let mut command = Command::new("cargo");
            command
                .args([
                    subcommand,
                    "--offline",
                    "--config",
                    "source.crates-io.replace-with='fixture'",
                ])
                .arg("--config")
                .arg(format!(
                    "source.fixture.directory={:?}",
                    root.join("vendor").to_string_lossy()
                ))
                .current_dir(root)
                .env("CARGO_HOME", root.join("cargo-home"));
            command
        };
        let plan = Plan::read(&game, &projects, &wanted).unwrap();
        plan.resolve_and_fetch(root, cargo)?;
        if build {
            let output = cargo("build")
                .args(["--locked", "--workspace", "--target-dir"])
                .arg(root.join("target"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        RegistryLock::parse(&super::super::read_lock(root).unwrap())
    }

    #[test]
    #[ignore = "daily"]
    fn two_mods_build_on_the_games_pin_when_both_requirements_allow_it() {
        let lock = shared_registry_fixture("^3.0", true, false).unwrap();
        let pins: Vec<_> = lock
            .packages
            .keys()
            .map(|id| (&*id.name, id.version.to_string()))
            .collect();
        assert_eq!(pins, [("syn", "3.0.6".to_owned())]);
    }

    #[test]
    #[ignore = "daily"]
    fn joint_build_chooses_a_pinned_parent_that_keeps_the_games_version() {
        let lock = shared_registry_fixture("^3.0", true, true).unwrap();
        let pins: Vec<_> = lock
            .packages
            .keys()
            .map(|id| (&*id.name, id.version.to_string()))
            .collect();
        assert_eq!(
            pins,
            [("parent", "1.0.1".to_owned()), ("syn", "3.0.6".to_owned())]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn joint_build_can_drop_the_games_line_of_a_linked_package() {
        use std::{fs, process::Command};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(super::super::root()).unwrap();
        let mut pins = BTreeMap::new();
        for (name, version, digit, build_dep) in [
            ("native-sys", "3.0.6", "a", ""),
            ("native-sys", "3.0.10", "b", ""),
            ("native-sys", "4.0.0", "c", ""),
            ("parent", "1.0.0", "d", "=3.0.10"),
            ("parent", "1.0.1", "e", "4"),
            ("parent", "1.0.2", "f", "4"), // Published but not pinned by a mod.
        ] {
            let vendor = root.join("vendor").join(format!("{name}-{version}"));
            fs::create_dir_all(vendor.join("src")).unwrap();
            fs::write(vendor.join("src/lib.rs"), "").unwrap();
            let extra = if build_dep.is_empty() {
                fs::write(vendor.join("build.rs"), "fn main() {}").unwrap();
                "links='fixture-native'\n".to_owned()
            } else {
                format!("[build-dependencies]\nnative-sys='{build_dep}'\n")
            };
            fs::write(
                vendor.join("Cargo.toml"),
                format!("[package]\nname='{name}'\nversion='{version}'\nedition='2024'\n{extra}"),
            )
            .unwrap();
            let checksum = digit.repeat(64);
            fs::write(
                vendor.join(".cargo-checksum.json"),
                format!(r#"{{"package":"{checksum}","files":{{}}}}"#),
            )
            .unwrap();
            pins.insert((name, version), format!(
                "[[package]]\nname='{name}'\nversion='{version}'\nsource='{}'\nchecksum='{checksum}'\n",
                sunrise_addresses::CARGO_CRATES_IO_SOURCE,
            ));
        }
        let game =
            RegistryLock::parse(&format!("version=4\n{}", pins[&("native-sys", "3.0.6")])).unwrap();
        let expected = RegistryLock::parse(&format!(
            "version=4\n{}{}",
            pins[&("native-sys", "4.0.0")],
            pins[&("parent", "1.0.1")],
        ))
        .unwrap();
        let mut seed = game.clone();
        seed.extend(&expected).unwrap();
        seed.extend(
            &RegistryLock::parse(&format!("version=4\n{}", pins[&("parent", "1.0.0")])).unwrap(),
        )
        .unwrap();
        fs::create_dir(root.join(".cargo")).unwrap();
        fs::write(
            root.join(".cargo/config.toml"),
            "[source.crates-io]\nreplace-with='fixture'\n[source.fixture]\ndirectory='vendor'\n",
        )
        .unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        let cargo = |subcommand: &str| {
            let mut command = Command::new("cargo");
            command
                .args([subcommand, "--offline"])
                .current_dir(root)
                .env("CARGO_HOME", root.join("cargo-home"));
            command
        };
        for (game, extra) in [
            // The first seeded parent moves the game's pin, conflicts on `links`, or
            // selects an unpinned release when there is no game pin, respectively.
            (game.clone(), ""),
            (game, "[build-dependencies]\nnative-sys='4'\n"),
            (RegistryLock::default(), ""),
        ] {
            let plan = Plan {
                game,
                allowed: seed.clone(),
                seed: seed.clone(),
            };
            let manifest = format!(
                "[workspace]\n[package]\nname='fixture'\nversion='1.0.0'\nedition='2024'\n[dependencies]\nparent='1'\n{extra}"
            );
            fs::write(root.join("Cargo.toml"), &manifest).unwrap();
            plan.resolve_and_fetch(root, cargo).unwrap();
            let resolved = RegistryLock::parse(&super::super::read_lock(root).unwrap()).unwrap();
            assert_eq!(
                resolved.packages.keys().collect::<Vec<_>>(),
                expected.packages.keys().collect::<Vec<_>>()
            );
            expected.check_pins(&resolved).unwrap();
            assert_eq!(
                fs::read_to_string(root.join("Cargo.toml")).unwrap(),
                manifest
            );
        }
        let output = cargo("build")
            .args(["--locked", "--target-dir"])
            .arg(root.join("target"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn refusal_names_the_requiring_mod_not_the_other_mod_with_the_same_lock_pin() {
        let error = shared_registry_fixture("=3.0.10", false, false).unwrap_err();
        for detail in ["syn", "3.0.6", "=3.0.10", "ahead_mod", "ahead_crate"] {
            assert!(error.contains(detail), "{error}");
        }
        assert!(!error.contains("loose_mod"), "{error}");
        // The stand-ins that leave releases out stay out of what the author reads.
        assert!(!error.contains("sunrise-registry"), "{error}");
    }

    #[test]
    fn missing_malformed_and_bad_checksum_locks_refuse_before_fetch() {
        let dir = tempfile::tempdir().unwrap();
        super::super::tests::write_bundle(dir.path(), "example", "");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='example'\nversion='1.0.0'\n[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\nitoa='1'\n").unwrap();
        let bundle = super::super::bundle_in(dir.path()).unwrap().unwrap();
        let projects = [Project::read(&bundle).unwrap()];
        let wanted = [Wanted {
            mod_id: "example".into(),
            name: "itoa".into(),
            version: "1".into(),
            requirement: semver::VersionReq::parse("1").unwrap(),
        }];
        let game = "version=4\npackage=[]\n";
        let missing = Plan::read(game, &projects, &wanted).err().unwrap();
        assert!(missing.contains("Cargo.lock"), "{missing}");
        for (text, expected) in [
            ("version=4", "missing packages"),
            (
                "version=4\n[[package]]\nname='itoa'\nversion='1.0.18'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='bad'\n",
                "checksum",
            ),
        ] {
            std::fs::write(dir.path().join("Cargo.lock"), text).unwrap();
            let error = Plan::read(game, &projects, &wanted).err().unwrap();
            assert!(error.contains(expected), "{error}");
        }
        assert!(!dir.path().join("target").exists());
    }

    #[test]
    #[ignore = "daily"]
    fn a_line_the_mod_requires_itself_is_refused_without_trying_each_release() {
        use std::{cell::Cell, fs, process::Command};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(super::super::root()).unwrap();
        let mut pins = BTreeMap::new();
        // The game locks 3.0.6; the mod asks for 3.0.10 or newer on the same line.
        for (index, patch) in std::iter::once(6).chain(10..=40).enumerate() {
            let version = format!("3.0.{patch}");
            let vendor = root.join("vendor").join(format!("syn-{version}"));
            fs::create_dir_all(vendor.join("src")).unwrap();
            fs::write(vendor.join("src/lib.rs"), "").unwrap();
            fs::write(
                vendor.join("Cargo.toml"),
                format!("[package]\nname='syn'\nversion='{version}'\nedition='2024'\n"),
            )
            .unwrap();
            let checksum = format!("{:064x}", index + 1);
            fs::write(
                vendor.join(".cargo-checksum.json"),
                format!(r#"{{"package":"{checksum}","files":{{}}}}"#),
            )
            .unwrap();
            pins.insert(patch, format!(
                "[[package]]\nname='syn'\nversion='{version}'\nsource='{}'\nchecksum='{checksum}'\n",
                sunrise_addresses::CARGO_CRATES_IO_SOURCE,
            ));
        }
        let game = RegistryLock::parse(&format!("version=4\n{}", pins[&6])).unwrap();
        let mut seed = game.clone();
        seed.extend(&RegistryLock::parse(&format!("version=4\n{}", pins[&10])).unwrap())
            .unwrap();
        fs::create_dir(root.join(".cargo")).unwrap();
        fs::write(
            root.join(".cargo/config.toml"),
            "[source.crates-io]\nreplace-with='fixture'\n[source.fixture]\ndirectory='vendor'\n",
        )
        .unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\n[package]\nname='fixture'\nversion='1.0.0'\nedition='2024'\n[dependencies]\nsyn='3.0.10'\n",
        )
        .unwrap();
        let runs = Cell::new(0);
        let cargo = |subcommand: &str| {
            runs.set(runs.get() + 1);
            let mut command = Command::new("cargo");
            command
                .args([subcommand, "--offline"])
                .current_dir(root)
                .env("CARGO_HOME", root.join("cargo-home"));
            command
        };
        let plan = Plan {
            game,
            allowed: seed.clone(),
            seed,
        };
        let error = plan.resolve_and_fetch(root, cargo).unwrap_err();
        for detail in [
            "syn",
            "3.0.6",
            "^3.0.10",
            "fixture v1.0.0",
            "the game's lock",
        ] {
            assert!(error.contains(detail), "{error}");
        }
        assert!(!error.contains("sunrise-registry"), "{error}");
        // One resolution, then one that keeps the game's release: none per release of the line.
        assert_eq!(runs.get(), 2, "{error}");
    }
}
