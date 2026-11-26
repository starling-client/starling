//! Resolve the staged graph on the game's registry pins without downloading archives.
use super::registry_lock::RegistryLock;
use crate::host::{tr, trf};
use std::{collections::BTreeSet, fs, path::Path, process::Command};

/// The crate an excluded release depends on, which no version satisfies.
const EXCLUDED: &str = "sunrise-registry-pin-excluded";

/// Prefer the supplied locks, but let Cargo backtrack through their versions if it needs to.
/// A game pin constrains the version on its line; it does not require that line to be present.
///
/// After the seeded pass, one pass requires each moved game pin exactly, as most graphs that
/// can keep the game's version resolve that way. Only when that fails, as when the line can be
/// left out of the graph instead (a `links` conflict), are the rejected releases excluded one at
/// a time until Cargo finds a graph without them.
pub(super) fn resolve(
    directory: &Path,
    game: &RegistryLock,
    seed: &RegistryLock,
    allowed: Option<&RegistryLock>,
    mut cargo: impl FnMut(&str) -> Command,
) -> Result<String, String> {
    let lock_path = directory.join("Cargo.lock");
    let original_seed = seed.seed("version=4\npackage=[]\n")?;
    let mut update = |package: Option<&str>| {
        let mut command = cargo("update");
        command.args(["--color", "never"]);
        if let Some(package) = package {
            command.args(["-p", package]);
        } else {
            command.arg("--workspace");
        }
        super::run(command, "Resolving registry dependency pins")
    };
    let read = || {
        let text = super::read_lock(directory)?;
        let resolved = RegistryLock::parse(&text)?;
        Ok::<_, String>((text, resolved))
    };
    let check = |resolved: &RegistryLock| {
        check_game_pins(game, resolved)?;
        if let Some(allowed) = allowed {
            allowed.check_pins(resolved)?;
        }
        Ok(())
    };
    fs::write(&lock_path, &original_seed).map_err(|e| e.to_string())?;
    let mut rejected = match update(None) {
        Ok(()) => {
            let (text, resolved) = read()?;
            match check(&resolved) {
                Ok(()) => return Ok(text),
                Err(error) => Some((resolved, error)),
            }
        }
        Err(error) if seed.packages.is_empty() => return Err(error),
        Err(_) => None,
    };

    let work = tempfile::Builder::new()
        .prefix("sunrise-registry-unlock-")
        .tempdir_in(directory)
        .map_err(|e| e.to_string())?;
    let name = work
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid temporary registry pin directory")?;
    // `cargo update -p` unlocks the named package and stops enforcing its old transitive
    // edges, but still prefers those versions (including yanked pins). This synthetic
    // record is only in the seed, not in any manifest, so it cannot pull in dependencies.
    let pins: Vec<_> = seed
        .packages
        .keys()
        .map(|pin| format!("{} {} ({})", pin.name, pin.version, pin.source))
        .collect();
    let registry = sunrise_addresses::CARGO_CRATES_IO_SOURCE;
    let checksum = "0".repeat(64);
    let unlock = (!pins.is_empty()).then_some(name);
    let unlocked_seed = if unlock.is_some() {
        format!(
            "{original_seed}\n[[package]]\nname='{name}'\nversion='0.0.0'\nsource='{registry}'\nchecksum='{checksum}'\ndependencies={pins:?}\n"
        )
    } else {
        original_seed
    };

    let manifest_path = directory.join("Cargo.toml");
    let original = fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
    let mut manifest: toml::Table = original.parse().map_err(|e| format!("Cargo.toml: {e}"))?;
    let mut excluded = BTreeSet::new();
    let mut patched = false;
    let result = (|| {
        fs::write(work.path().join("inert.rs"), "").map_err(|e| e.to_string())?;
        if let Some((resolved, refusal)) = &rejected {
            let moved: BTreeSet<_> = game.moved_pins(resolved).map(|(game, _)| game).collect();
            if !moved.is_empty() {
                // A temporary member requires each moved pin exactly: an edge from the root,
                // so this pass only shows a graph that keeps the game's line.
                let pins = work.path().join("pins");
                fs::create_dir(&pins).map_err(|e| e.to_string())?;
                let dependencies: toml::Table = moved
                    .iter()
                    .enumerate()
                    .map(|(index, pin)| {
                        let spec = toml::Table::from_iter([
                            ("package".into(), pin.name.clone().into()),
                            ("version".into(), format!("={}", pin.version).into()),
                            ("default-features".into(), false.into()),
                        ]);
                        (format!("pin_{index}"), spec.into())
                    })
                    .collect();
                let member = toml::Table::from_iter([
                    (
                        "package".into(),
                        toml::Table::from_iter([
                            ("name".into(), format!("{name}-pins").into()),
                            ("version".into(), "0.0.0".into()),
                            ("edition".into(), "2024".into()),
                        ])
                        .into(),
                    ),
                    (
                        "lib".into(),
                        toml::Table::from_iter([("path".into(), "../inert.rs".into())]).into(),
                    ),
                    ("dependencies".into(), dependencies.into()),
                ]);
                fs::write(pins.join("Cargo.toml"), member.to_string())
                    .map_err(|e| e.to_string())?;
                let mut constrained = manifest.clone();
                constrained
                    .get_mut("workspace")
                    .and_then(toml::Value::as_table_mut)
                    .ok_or("Missing staged Cargo workspace")?
                    .entry("members")
                    .or_insert_with(|| toml::Value::Array(Vec::new()))
                    .as_array_mut()
                    .ok_or("Invalid staged Cargo workspace members")?
                    .push(format!("{name}/pins").into());
                patched = true;
                fs::write(&manifest_path, constrained.to_string()).map_err(|e| e.to_string())?;
                fs::write(&lock_path, &unlocked_seed).map_err(|e| e.to_string())?;
                match update(unlock) {
                    Ok(()) => {
                        let (text, constrained) = read()?;
                        if check(&constrained).is_ok() {
                            return Ok(text);
                        }
                    }
                    // A line the mods' own packages require directly is in every graph, and
                    // no graph holds the game's release of each such line: leaving releases
                    // out one by one would only walk the lines down to this refusal.
                    Err(conflict)
                        if game
                            .moved_pins(resolved)
                            .all(|(_, pin)| resolved.required_locally(game, pin)) =>
                    {
                        let pins = format!("{name}-pins");
                        let report = conflict
                            .lines()
                            .map(|line| {
                                if !line.contains(&pins) {
                                    line
                                } else if line.contains("required by package") {
                                    tr("    ... required by the game's lock")
                                } else {
                                    tr("    ... which the game's lock requires")
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        return Err(format!("{refusal}\n\n{report}"));
                    }
                    Err(_) => {}
                }
                fs::write(&manifest_path, manifest.to_string()).map_err(|e| e.to_string())?;
            }
        }
        let mut refusal = None;
        loop {
            if let Some((resolved, error)) = rejected {
                let before = excluded.len();
                let unpinned = resolved.packages.keys().filter(|id| {
                    allowed.is_some_and(|allowed| !allowed.packages.contains_key(*id))
                });
                let patches = manifest
                    .entry("patch")
                    .or_insert_with(|| toml::Table::new().into())
                    .as_table_mut()
                    .ok_or("Invalid staged Cargo patches")?
                    .entry("crates-io")
                    .or_insert_with(|| toml::Table::new().into())
                    .as_table_mut()
                    .ok_or("Invalid staged crates.io patches")?;
                for pin in game
                    .moved_pins(&resolved)
                    .map(|(_, selected)| selected)
                    .chain(unpinned)
                {
                    // The SDK already patches the release it builds from source, and Cargo
                    // takes one patch per release: refused below, not excluded.
                    if game.supplies(pin) || !excluded.insert(pin.clone()) {
                        continue;
                    }
                    // A patch shadows just this exact release. Give it an impossible local
                    // dependency, so Cargo must backtrack instead of selecting that release.
                    // Unlike an exact requirement on another version, this adds no root edge.
                    let key = format!("{name}-{}", excluded.len());
                    let path = work.path().join(&key);
                    fs::create_dir(&path).map_err(|e| e.to_string())?;
                    fs::write(
                        path.join("Cargo.toml"),
                        format!(
                            "[package]\nname={:?}\nversion='{}'\nedition='2024'\n[lib]\npath='../inert.rs'\n[dependencies]\n{EXCLUDED}={{path='..',version='=0.0.1'}}\n",
                            pin.name, pin.version,
                        ),
                    ).map_err(|e| e.to_string())?;
                    patches.insert(
                        key.clone(),
                        toml::Table::from_iter([
                            ("package".into(), pin.name.clone().into()),
                            ("path".into(), format!("{name}/{key}").into()),
                        ])
                        .into(),
                    );
                }
                if excluded.len() == before {
                    // A checksum change or SDK-local conflict is a trust-boundary refusal,
                    // not a registry version choice that can be retried.
                    return Err(error);
                }
                refusal.get_or_insert(error);
                if before == 0 {
                    fs::write(work.path().join("Cargo.toml"), format!("[package]\nname='{EXCLUDED}'\nversion='0.0.0'\nedition='2024'\n[lib]\npath='inert.rs'\n")).map_err(|e| e.to_string())?;
                }
                patched = true;
                fs::write(&manifest_path, manifest.to_string()).map_err(|e| e.to_string())?;
            }
            fs::write(&lock_path, &unlocked_seed).map_err(|e| e.to_string())?;
            if let Err(error) = update(unlock) {
                return Err(match refusal {
                    // Cargo's report names the stand-in that leaves a release out, not the
                    // release: say which, and keep the chain of what still requires it.
                    Some(refusal) if error.contains(EXCLUDED) => {
                        let report = error
                            .lines()
                            .filter_map(|line| {
                                if let Some(package) = line
                                    .strip_prefix("required by package `")
                                    .filter(|_| line.contains(name))
                                {
                                    let package = package.split(" (").next().unwrap_or(package);
                                    Some(trf(
                                        "{package} was left out, and is still required:",
                                        &[("package", package)],
                                    ))
                                } else if line.contains(EXCLUDED)
                                    || line.contains(name)
                                    || line.starts_with("candidate versions found")
                                {
                                    None
                                } else {
                                    Some(line.to_owned())
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        format!("{refusal}\n\n{report}")
                    }
                    Some(refusal) => format!("{refusal}\n\n{error}"),
                    None => error,
                });
            }
            let (text, resolved) = read()?;
            match check(&resolved) {
                Ok(()) => return Ok(text),
                Err(error) => rejected = Some((resolved, error)),
            }
        }
    })();
    if !patched {
        return result;
    }
    fs::write(&manifest_path, original).map_err(|e| e.to_string())?;
    result?;

    // Drop unused exclusion patches from the lock with the real manifest restored.
    // This pass has a complete, valid lock to preserve, not a union of mod locks.
    update(None)?;
    let (text, resolved) = read()?;
    check(&resolved)?;
    Ok(text)
}

pub(super) fn check_game_pins(game: &RegistryLock, resolved: &RegistryLock) -> Result<(), String> {
    let mut moved = game.moved_pins(resolved).peekable();
    if moved.peek().is_some() {
        Err(moved.map(|(game, selected)| {
            trf(
                "{crate}: the game locks {game}, but resolution without the game's constraint selected {selected}.",
                &[
                    ("crate", &game.name),
                    ("game", &game.version.to_string()),
                    ("selected", &selected.version.to_string()),
                ],
            )
        }).collect::<Vec<_>>().join("\n"))
    } else {
        Ok(())
    }
}
