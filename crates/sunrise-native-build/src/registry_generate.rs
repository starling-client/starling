//! Explicit, index-only generation of the registry lock consumed by native staging.
use super::{Bundle, api};
use crate::rust::Toolchain;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use sunrise_mod_package::native_route::PACKAGES;

/// Copies only inert manifests. Source files, build scripts and Cargo config never enter it.
pub fn stage(bundle: &Bundle, output: &Path) -> Result<(), String> {
    let mut todo = vec![PathBuf::new()];
    let mut seen = BTreeSet::new();
    while let Some(relative) = todo.pop() {
        if !seen.insert(relative.clone()) {
            continue;
        }
        let input = bundle.dir.join(&relative);
        let manifest_path = input.join("Cargo.toml");
        if super::crates::is_link_or_junction(
            &fs::symlink_metadata(&manifest_path).map_err(|e| e.to_string())?,
        ) {
            return Err("linked dependency manifest is refused".into());
        }
        let mut manifest: toml::Table = fs::read_to_string(&manifest_path)
            .map_err(|e| e.to_string())?
            .parse()
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        for section in ["patch", "replace", "workspace"] {
            if manifest.contains_key(section) {
                return Err(format!(
                    "{}: [{section}] is refused",
                    manifest_path.display()
                ));
            }
        }
        rewrite_dependencies(bundle, &relative, &mut manifest, &mut todo)?;
        if let Some(targets) = manifest
            .get_mut("target")
            .and_then(toml::Value::as_table_mut)
        {
            for target in targets
                .iter_mut()
                .filter_map(|(_, value)| value.as_table_mut())
            {
                rewrite_dependencies(bundle, &relative, target, &mut todo)?;
            }
        }
        manifest.remove("bin");
        manifest.remove("example");
        manifest.remove("test");
        manifest.remove("bench");
        manifest.remove("lib");
        let package = manifest
            .get_mut("package")
            .and_then(toml::Value::as_table_mut)
            .ok_or("Cargo.toml: missing package")?;
        package.insert("build".into(), false.into());
        for key in ["autobins", "autoexamples", "autotests", "autobenches"] {
            package.insert(key.into(), false.into());
        }
        manifest.insert("lib".into(), toml::toml! { path = "inert.rs" }.into());
        if relative.as_os_str().is_empty() {
            manifest.insert("workspace".into(), toml::Table::new().into());
        }
        let dest = output.join(relative);
        fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        fs::write(dest.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
        fs::write(dest.join("inert.rs"), "").map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn rewrite_dependencies(
    bundle: &Bundle,
    relative: &Path,
    table: &mut toml::Table,
    todo: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for kind in super::crates::DEPENDENCY_KINDS {
        let Some(deps) = table.get_mut(kind).and_then(toml::Value::as_table_mut) else {
            continue;
        };
        let mut sdk = Vec::new();
        for (key, dep) in deps.iter_mut() {
            let get = |field| dep.get(field).and_then(toml::Value::as_str);
            let package = get("package").unwrap_or(key);
            if let Some(&(name, _)) = PACKAGES.iter().find(|(name, _)| *name == package) {
                api::sdk_source(key, name, dep)?;
                sdk.push(key.clone());
                continue;
            }
            for selector in [
                "git",
                "rev",
                "tag",
                "branch",
                "registry",
                "registry-index",
                "workspace",
            ] {
                if dep.get(selector).is_some() {
                    return Err(format!("{key}: {selector} source is refused"));
                }
            }
            if let Some(path) = dep.get("path") {
                let path = path.as_str().ok_or("dependency path must be a string")?;
                let inside = super::crates::local_path(&bundle.dir, relative, path)?;
                todo.push(inside);
            }
        }
        for key in sdk {
            deps.remove(&key);
        }
    }
    Ok(())
}

/// The `Cargo.lock` of the game build `revision`, which a mod's lock starts from: kept in the
/// build's folder, and downloaded from that commit once. A kept file that no longer reads as a
/// lock is downloaded again.
pub fn game_lock(revision: &str) -> Result<String, String> {
    let kept = super::root()
        .join("game-lock")
        .join(format!("{revision}.lock"));
    if let Ok(text) = fs::read_to_string(&kept)
        && super::registry_lock::RegistryLock::parse(&text).is_ok()
    {
        crate::host::mark_used(&kept)?;
        return Ok(text);
    }
    let url = format!(
        "{}/{revision}/Cargo.lock",
        sunrise_addresses::GAME_SOURCE_FILES
    );
    let unavailable = |error: String| {
        crate::host::trf(
            "The mod's lock starts from the Cargo.lock of its game build, {revision}, which could not be downloaded: {error}. To lock without the network, put that file at {path}.",
            &[
                ("revision", revision),
                ("error", &error),
                ("path", &kept.display().to_string()),
            ],
        )
    };
    let mut text = String::new();
    // One byte past the limit shows an answer that is too large, where a cut one could still parse.
    crate::host::get(&url)
        .map_err(unavailable)?
        .take(GAME_LOCK_LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(|e| unavailable(e.to_string()))?;
    if text.len() as u64 > GAME_LOCK_LIMIT {
        return Err(unavailable("the answer is too large to be a lock".into()));
    }
    super::registry_lock::RegistryLock::parse(&text).map_err(unavailable)?;
    let folder = kept.parent().expect("a file in the build's folder");
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let mut pending = tempfile::NamedTempFile::new_in(folder).map_err(|e| e.to_string())?;
    pending
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    pending.persist(&kept).map_err(|e| e.to_string())?;
    crate::host::mark_used(&kept)?;
    Ok(text)
}

/// More than any game lock holds; a larger answer is not one.
const GAME_LOCK_LIMIT: u64 = 16 << 20;

/// The mod's registry pins resolved with the toolchain `rust` gives, once the mod's inputs passed.
/// Resolution starts from `game`, the `Cargo.lock` of the mod's game build: a package the game
/// also locks keeps the game's version, so mods made for one build agree on those packages, and
/// only the mod's own packages take the newest version that fits. `game` is asked for the lock of
/// the build the mod pins.
pub fn generate<T: Toolchain>(
    rust: impl FnOnce() -> Result<T, String>,
    game: impl FnOnce(&str) -> Result<String, String>,
    bundle: &Bundle,
    update: bool,
) -> Result<(), String> {
    let native = bundle.manifest.native.as_ref().ok_or_else(|| {
        crate::host::trf(
            "{path} is not a native mod.",
            &[("path", &bundle.dir.display().to_string())],
        )
    })?;
    // Project owns the original API declaration and source-policy boundary.
    api::Project::read(bundle).map_err(|e| e.to_string())?;
    let destination = bundle.dir.join("Cargo.lock");
    if destination.exists() && !update {
        return Err(
            "Cargo.lock already exists; use --update-lock to replace its pins explicitly".into(),
        );
    }
    fs::create_dir_all(super::root()).map_err(|e| e.to_string())?;
    let work = tempfile::tempdir_in(super::root()).map_err(|e| e.to_string())?;
    stage(bundle, work.path())?;
    let seed = super::registry_lock::RegistryLock::parse(&game(&native.sunrise)?)?;
    let rust = rust()?;
    let text = super::registry_resolve::resolve(work.path(), &seed, &seed, None, |subcommand| {
        super::cargo(&rust, work.path(), subcommand)
    })
    .map_err(|error| {
        crate::host::trf(
            "{mod}: could not write Cargo.lock.\n{error}",
            &[("mod", &bundle.manifest.id), ("error", &error)],
        )
    })?;
    // Refusals and resolution errors above never mutate the existing lock.
    let mut pending = tempfile::NamedTempFile::new_in(&bundle.dir).map_err(|e| e.to_string())?;
    pending
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    pending.as_file().sync_all().map_err(|e| e.to_string())?;
    pending.persist(destination).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> Bundle {
        super::super::tests::write_bundle(
            root,
            "registry_fixture",
            "compile_error!(\"must never compile\");",
        );
        fs::write(root.join("Cargo.toml"), "[package]\nname='fixture'\nversion='0.1.0'\n[dependencies]\nsunrise-mod-api={ path='.sunrise/sdk/api/crates/sunrise-mod-api' }\nitoa='1'\n").unwrap();
        super::super::bundle_in(root).unwrap().unwrap()
    }
    #[test]
    fn the_sdk_is_removed_only_in_its_path_form_and_sources_are_inert() {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path());
        fs::write(input.path().join("build.rs"), "panic!(\"do not run\");").unwrap();
        fs::create_dir_all(input.path().join(".cargo")).unwrap();
        fs::write(
            input.path().join(".cargo/config.toml"),
            "[source.crates-io]\nreplace-with='local'\n",
        )
        .unwrap();
        let before = fs::read(input.path().join("Cargo.toml")).unwrap();
        stage(&bundle, output.path()).unwrap();
        assert!(!output.path().join(".cargo").exists());
        let generated = fs::read_to_string(output.path().join("Cargo.toml")).unwrap();
        assert!(!generated.contains("sunrise-mod-api"));
        assert!(generated.contains("itoa"));
        assert!(!output.path().join("build.rs").exists());
        assert!(!output.path().join("src").exists());
        assert_eq!(fs::read(input.path().join("Cargo.toml")).unwrap(), before);
        let original = String::from_utf8(before).unwrap();
        for (form, refusal) in [
            (
                format!(
                    "{{ git='https://github.com/arismoko/horizon', rev='{}' }}",
                    "a".repeat(40)
                ),
                "E_SDK_SOURCE: sunrise-mod-api: the SDK is no longer named by git",
            ),
            (
                "'0.1'".to_owned(),
                "E_SDK_SOURCE: sunrise-mod-api: a version names no source",
            ),
            (
                "{ path='../sunrise/crates/sunrise-mod-api' }".to_owned(),
                "E_SDK_SOURCE: sunrise-mod-api: the path must be \".sunrise/sdk/api/crates/sunrise-mod-api\"",
            ),
        ] {
            fs::write(
                input.path().join("Cargo.toml"),
                original.replace("{ path='.sunrise/sdk/api/crates/sunrise-mod-api' }", &form),
            )
            .unwrap();
            let error = stage(&bundle, output.path()).unwrap_err();
            assert!(error.starts_with(refusal), "{error}");
        }
    }
    #[test]
    fn source_refusals_leave_existing_lock_untouched() {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path());
        let original = fs::read_to_string(input.path().join("Cargo.toml")).unwrap();
        fs::write(input.path().join("Cargo.lock"), "original pins").unwrap();
        let unreached = || -> Result<crate::rust::Unpacked, String> {
            panic!("no toolchain before the refusal")
        };
        assert!(
            generate(
                unreached,
                |_| panic!("no game lock before the refusal"),
                &bundle,
                false
            )
            .unwrap_err()
            .contains("--update-lock")
        );
        for extra in [
            "[patch.crates-io]\nitoa={path='outside'}\n",
            "[replace]\n'itoa:1.0.18'={path='outside'}\n",
        ] {
            fs::write(
                input.path().join("Cargo.toml"),
                format!("{original}\n{extra}"),
            )
            .unwrap();
            assert!(stage(&bundle, output.path()).is_err());
            assert_eq!(
                fs::read_to_string(input.path().join("Cargo.lock")).unwrap(),
                "original pins"
            );
        }
    }
    #[test]
    fn local_dependencies_are_traversed_but_cannot_escape() {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path());
        let original = fs::read_to_string(input.path().join("Cargo.toml")).unwrap();
        fs::create_dir(input.path().join("local")).unwrap();
        fs::write(
            input.path().join("local/Cargo.toml"),
            "[package]\nname='helper'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(
            input.path().join("Cargo.toml"),
            format!("{original}\nhelper={{path='local'}}\n"),
        )
        .unwrap();
        stage(&bundle, output.path()).unwrap();
        assert!(output.path().join("local/inert.rs").is_file());
        fs::write(
            input.path().join("Cargo.toml"),
            format!("{original}\nhelper={{path='../outside'}}\n"),
        )
        .unwrap();
        assert!(
            stage(&bundle, output.path())
                .unwrap_err()
                .contains("outside the mod")
        );
    }

    struct GameSeedFixture {
        input: tempfile::TempDir,
        bundle: Bundle,
        game: String,
        toolchain: crate::rust::Unpacked,
    }

    fn game_seed_fixture() -> GameSeedFixture {
        let input = tempfile::tempdir().unwrap();
        let vendor = input.path().join("vendor");
        let bundle = fixture(input.path());
        for (name, version, dependency) in [
            ("zipish", "1.0.0", "getrandom='0.4'"),
            ("zipish", "1.0.1", "getrandom='=0.4.4'"),
            ("getrandom", "0.4.3", "libc='0.2'"),
            ("getrandom", "0.4.4", "libc='0.2'"),
            ("libc", "0.2.190", ""),
            ("libc", "0.2.191", "[features]\nnew_api=[]"),
            ("libc", "0.3.0", ""),
            ("unused", "1.0.0", ""),
        ] {
            let folder = vendor.join(format!("{name}-{version}"));
            fs::create_dir_all(folder.join("src")).unwrap();
            fs::write(folder.join("src/lib.rs"), "").unwrap();
            fs::write(
                folder.join("Cargo.toml"),
                format!(
                    "[package]\nname='{name}'\nversion='{version}'\n[dependencies]\n{dependency}\n"
                ),
            )
            .unwrap();
            fs::write(
                folder.join(".cargo-checksum.json"),
                format!(r#"{{"package":"{}","files":{{}}}}"#, "a".repeat(64)),
            )
            .unwrap();
        }
        fs::write(
            input.path().join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\n[dependencies]\nsunrise-mod-api={ path='.sunrise/sdk/api/crates/sunrise-mod-api' }\nzipish='1'\nother-libc={package='libc',version='0.3'}\n",
        )
        .unwrap();
        let package = |name: &str, version: &str, dependencies: &str| {
            format!(
                "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"{}\"\ndependencies = [{dependencies}]\n",
                "a".repeat(64)
            )
        };
        // The game's own crate and a package only the game uses stay out of the mod's lock.
        let game = format!(
            "version = 4\n[[package]]\nname = \"game\"\nversion = \"1.0.0\"\ndependencies = [\"getrandom\", \"unused\"]\n{}{}{}",
            package("getrandom", "0.4.3", "\"libc\""),
            package("libc", "0.2.190", ""),
            package("unused", "1.0.0", "")
        );
        let sysroot = std::process::Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .unwrap()
            .stdout;
        let host = std::process::Command::new("rustc")
            .args(["--print", "host-tuple"])
            .output()
            .unwrap()
            .stdout;
        let toolchain = crate::rust::Unpacked {
            dir: String::from_utf8(sysroot).unwrap().trim().into(),
            target: String::from_utf8(host).unwrap().trim().into(),
            cargo_home: input.path().join("cargo-home"),
            cargo_config: vec![
                "net.offline=true".into(),
                "source.crates-io.replace-with='fixture'".into(),
                format!("source.fixture.directory='{}'", vendor.display()),
            ],
            c_bin: None,
        };
        GameSeedFixture {
            input,
            bundle,
            game,
            toolchain,
        }
    }

    /// A mod's own package needs packages the game locks: the lock gives the game's version of
    /// each, not the newest, and a build's check takes it. A different semver slot is still free.
    #[test]
    #[ignore = "daily"]
    fn a_mods_lock_starts_from_the_games_versions() {
        use crate::registry_lock::RegistryLock;
        let GameSeedFixture {
            input,
            bundle,
            game,
            toolchain,
        } = game_seed_fixture();
        generate(|| Ok(toolchain), |_| Ok(game.clone()), &bundle, false).unwrap();
        let text = fs::read_to_string(input.path().join("Cargo.lock")).unwrap();
        let pins = RegistryLock::parse(&text).unwrap();
        let locked: Vec<_> = pins
            .packages
            .keys()
            .map(|id| format!("{} {}", id.name, id.version))
            .collect();
        assert_eq!(
            locked,
            [
                "getrandom 0.4.3",
                "libc 0.2.190",
                "libc 0.3.0",
                "zipish 1.0.0"
            ],
            "{text}"
        );
        let mut allowed = RegistryLock::parse(&game).unwrap();
        allowed.extend(&pins).unwrap();
        allowed.check_pins(&pins).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_requirement_that_moves_a_game_pin_leaves_the_old_lock_untouched() {
        let GameSeedFixture {
            input,
            bundle,
            game,
            toolchain,
        } = game_seed_fixture();
        let manifest = input.path().join("Cargo.toml");
        let original = fs::read_to_string(&manifest).unwrap();
        let older_parent = original.replace("zipish='1'", "zipish='=1.0.0'");
        let lock = input.path().join("Cargo.lock");
        generate(
            || Ok(toolchain.clone()),
            |_| Ok(game.clone()),
            &bundle,
            false,
        )
        .unwrap();
        let old = fs::read(&lock).unwrap();
        for (cargo, expected) in [
            (
                format!(
                    "{older_parent}\n[target.'cfg(unix)'.build-dependencies]\nnewer={{package='libc',version='=0.2.191'}}\n"
                ),
                ["libc", "0.2.190", "=0.2.191", "fixture v0.1.0"],
            ),
            (
                original.replace("zipish='1'", "zipish='=1.0.1'"),
                ["getrandom", "0.4.3", "=0.4.4", "zipish v1.0.1"],
            ),
            (
                format!("{older_parent}\nlibc={{version='0.2',features=['new_api']}}\n"),
                ["libc", "0.2.190", "0.2.191", "`new_api`"],
            ),
        ] {
            fs::write(&manifest, cargo).unwrap();
            let error = generate(
                || Ok(toolchain.clone()),
                |_| Ok(game.clone()),
                &bundle,
                true,
            )
            .unwrap_err();
            for detail in expected.into_iter().chain(["registry_fixture"]) {
                assert!(error.contains(detail), "{error}");
            }
            assert_eq!(fs::read(&lock).unwrap(), old);
        }
    }
}
